// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Shared daemon state: one atomic status snapshot + the client registry.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

use tracing::{error, info, warn};

use crate::paths::socket_path;
use crate::pipeline::Pipeline;
use crate::protocol::{DaemonStatus, PushEvent};
use crate::state::{EqBand, FilterState, NodeInfo, NullSinkState, PwEvent};

/// One logical daemon status, protected by a single mutex so a snapshot is
/// internally consistent by construction.
#[derive(Default)]
pub(super) struct StatusSnapshot {
    pub(super) bands: Vec<EqBand>,
    pub(super) bypass: bool,
    pub(super) preamp: f32,
    pub(super) nodes: Vec<NodeInfo>,
    pub(super) pw_connected: bool,
    pub(super) filter_state: FilterState,
    pub(super) null_sink: NullSinkState,
    pub(super) filter_node_id: Option<u32>,
    pub(super) connected_devices: Vec<u32>, // CONFIRMED links only
    pub(super) pending_devices: Vec<u32>,   // ops in flight (connect or disconnect)
}

pub struct DaemonState {
    pub pipeline: Arc<Pipeline>,

    // One lock for the whole status snapshot — atomic reads and writes.
    pub(super) status: Mutex<StatusSnapshot>,

    // Not part of the status snapshot; operational only. Lock order rule:
    // `status` → `clients`, never the reverse. In practice: push_event
    // (locks `clients`) must only ever be called with no lock held; mutation
    // code locks `status`, drops it, THEN pushes.
    pub(super) clients: Mutex<Vec<ClientHandle>>,
    pub(super) shutting_down: Arc<AtomicBool>,
}

pub(super) struct ClientHandle {
    pub(super) id: u64,
    pub(super) tx: mpsc::Sender<String>,
    /// Clone kept only to unblock the handler's reader during shutdown
    /// (`shutdown(Both)` makes the blocked `read_until` error out).
    pub(super) stream: UnixStream,
}

impl DaemonState {
    pub fn new(pipeline: Arc<Pipeline>) -> Self {
        Self {
            pipeline,
            status: Mutex::new(StatusSnapshot::default()),
            clients: Mutex::new(Vec::new()),
            shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get_status(&self) -> DaemonStatus {
        let s = self.status.lock().unwrap();
        DaemonStatus {
            bands: s.bands.clone(),
            bypass: s.bypass,
            preamp: s.preamp,
            nodes: s.nodes.clone(),
            pw_connected: s.pw_connected,
            filter_state: s.filter_state.clone(),
            null_sink: s.null_sink.clone(),
            filter_node_id: s.filter_node_id,
            connected_devices: s.connected_devices.clone(),
            pending_devices: s.pending_devices.clone(),
        }
    }

    pub fn handle_pw_event(&self, event: PwEvent) {
        match &event {
            PwEvent::NodeList(nodes) => {
                let nodes = nodes.clone();
                {
                    let mut s = self.status.lock().unwrap();
                    s.nodes.clone_from(&nodes);
                    // Reconcile routing state against reality: prune devices
                    // that vanished (covers a connect still in flight).
                    let before = s.connected_devices.len() + s.pending_devices.len();
                    s.connected_devices
                        .retain(|id| nodes.iter().any(|n| n.id == *id));
                    s.pending_devices
                        .retain(|id| nodes.iter().any(|n| n.id == *id));
                    let pruned = before - (s.connected_devices.len() + s.pending_devices.len());
                    if pruned > 0 {
                        info!(pruned, "Pruned vanished devices from routing state");
                    }
                }
                self.push_event(PushEvent::NodeList { nodes });
            }
            PwEvent::Connected => {
                self.status.lock().unwrap().pw_connected = true;
                self.push_event(PushEvent::StateChange {
                    state: "connected".into(),
                });
            }
            PwEvent::FilterStateChanged(state) => {
                {
                    let mut s = self.status.lock().unwrap();
                    s.filter_state = state.clone();
                } // lock released BEFORE push_event (which locks `clients`)

                self.push_event(PushEvent::FilterStateChanged {
                    state: state.clone(),
                });
                self.push_event(PushEvent::StateChange {
                    state: format!("filter:{state:?}"),
                });

                if matches!(state, FilterState::Error(_)) {
                    warn!("PipeWire connection lost — shutting down for restart");
                    self.shutting_down.store(true, Ordering::Release);
                    if let Ok(path) = socket_path() {
                        let _ = UnixStream::connect(&path);
                    }
                }
            }
            PwEvent::FilterReady { node_id } => {
                self.status.lock().unwrap().filter_node_id = Some(*node_id);
                self.push_event(PushEvent::FilterReady { node_id: *node_id });
            }
            PwEvent::NullSinkCreated { module_id } => {
                self.status.lock().unwrap().null_sink = NullSinkState::Loaded {
                    module_id: *module_id,
                    has_source: false,
                };
                self.push_event(PushEvent::NullSinkCreated {
                    module_id: *module_id,
                });
            }
            PwEvent::NullSinkInputState { has_source } => {
                {
                    let mut s = self.status.lock().unwrap();
                    if let NullSinkState::Loaded { has_source: hs, .. } = &mut s.null_sink {
                        *hs = *has_source;
                    }
                }
                self.push_event(PushEvent::SourceActive {
                    active: *has_source,
                });
            }
            PwEvent::NullSinkInputUnknown => {
                self.push_event(PushEvent::SourceUnknown);
            }
            PwEvent::NullSinkError(msg) => {
                error!(%msg, "Null sink creation failed — filter will process silence");
                self.push_event(PushEvent::NullSinkMissing);
                self.push_event(PushEvent::Error {
                    message: msg.clone(),
                });
            }
            PwEvent::Error(msg) => {
                error!(%msg, "PW error forwarded to clients");
                self.push_event(PushEvent::Error {
                    message: msg.clone(),
                });
            }
            PwEvent::LinkResult {
                device_id,
                connect,
                ok,
            } => {
                let mut s = self.status.lock().unwrap();
                s.pending_devices.retain(|id| *id != *device_id);
                match (connect, ok) {
                    (true, true) => {
                        if !s.connected_devices.contains(device_id) {
                            s.connected_devices.push(*device_id);
                        }
                    }
                    (true, false) => {
                        drop(s);
                        self.push_event(PushEvent::Error {
                            message: format!("Failed to link device {device_id}"),
                        });
                    }
                    (false, true) => {
                        s.connected_devices.retain(|id| *id != *device_id);
                    }
                    (false, false) => {
                        // Link may still exist — keep the state truthful.
                        drop(s);
                        self.push_event(PushEvent::Error {
                            message: format!("Failed to unlink device {device_id}"),
                        });
                    }
                }
            }
            PwEvent::NodeAdded(_) | PwEvent::NodeRemoved(_) => {}
        }
    }

    pub fn register_client(&self, stream: &UnixStream, client_id: u64) {
        let (tx, rx) = mpsc::channel::<String>();

        let write_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "Failed to clone client stream for writer");
                return;
            }
        };

        // Second clone used solely to unblock the reader on shutdown.
        let shutdown_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "Failed to clone client stream for shutdown");
                return;
            }
        };

        thread::Builder::new()
            .name(format!("client-{client_id}-writer"))
            .spawn(move || {
                let mut w = write_stream;
                for msg in rx {
                    if w.write_all(msg.as_bytes()).is_err() {
                        break;
                    }
                }
            })
            .ok();

        self.clients.lock().unwrap().push(ClientHandle {
            id: client_id,
            tx,
            stream: shutdown_stream,
        });
        info!(client_id, "Client connected");
    }

    pub fn unregister_client(&self, client_id: u64) {
        self.clients.lock().unwrap().retain(|c| c.id != client_id);
        info!(client_id, "Client disconnected");
    }

    pub fn push_event(&self, event: PushEvent) {
        let mut clients = self.clients.lock().unwrap();
        if clients.is_empty() {
            return;
        }

        let json = match serde_json::to_string(&event) {
            Ok(j) => j + "\n",
            Err(e) => {
                error!(%e, "Failed to serialize push event");
                return;
            }
        };
        clients.retain(|c| c.tx.send(json.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile apply (bands + preamp changed under ONE lock) is observed
    /// atomically by any reader — there is no interleaving that yields the
    /// new bands with the old preamp.
    #[test]
    fn status_snapshot_is_internally_consistent() {
        use crate::state::FilterType;
        let state = DaemonState::new(Arc::new(Pipeline::new(crate::pipeline::SAMPLE_RATE)));

        // Simulate a profile apply: bands + preamp change under ONE lock.
        {
            let mut s = state.status.lock().unwrap();
            s.bands = vec![EqBand {
                frequency: 100.0,
                gain: 3.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            }];
            s.preamp = -6.0;
        }

        // Any reader now sees the pair atomically.
        let status = state.get_status();
        assert_eq!(status.bands.len(), 1);
        assert!((status.preamp - (-6.0)).abs() < f32::EPSILON);
    }

    /// Concurrent readers never observe a mixed snapshot: while a writer
    /// toggles `filter_state` + `filter_node_id` together under one lock,
    /// every observed snapshot satisfies the cross-field invariant.
    #[test]
    fn concurrent_get_status_never_sees_mixed_state() {
        let state = Arc::new(DaemonState::new(Arc::new(Pipeline::new(
            crate::pipeline::SAMPLE_RATE,
        ))));
        let mut handles = Vec::new();

        // Writer: toggles filter_state + filter_node_id together.
        let w = state.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..200 {
                let mut s = w.status.lock().unwrap();
                s.filter_state = FilterState::Streaming;
                s.filter_node_id = Some(1);
                std::thread::yield_now();
                s.filter_state = FilterState::Unconnected;
                s.filter_node_id = None;
                std::thread::yield_now();
            }
        }));

        // Readers: Streaming must imply filter_node_id is set.
        for _ in 0..4 {
            let r = state.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..200 {
                    let s = r.get_status();
                    if s.filter_state == FilterState::Streaming {
                        assert_eq!(s.filter_node_id, Some(1));
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }
}
