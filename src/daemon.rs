// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Daemon process — owns the `PipeWire` audio pipeline and serves
//! TUI/CLI clients over a Unix-domain socket.
//!
//! The daemon keeps `pw::run` untouched; a bridge thread translates
//! between its mpsc/pipewire-channel interface and the shared
//! `DaemonState` that socket handlers read and mutate.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use pipewire::channel;
use tracing::{error, info, warn};
use uds::UnixStreamExt;

use crate::pipeline::{Pipeline, SAMPLE_RATE};
use crate::protocol::{DaemonStatus, PushEvent, Request, Response};
use crate::pw;
use crate::state::{EqBand, FilterState, NodeInfo, NullSinkState, PwCommand, PwEvent};

/// Maximum number of concurrent client connections allowed.
/// This protects against resource exhaustion `DoS` (thread/memory leakage).
const MAX_CLIENTS: usize = 10;

pub struct DaemonState {
    pub pipeline: Arc<Pipeline>,

    nodes: Mutex<Vec<NodeInfo>>,
    pw_connected: Mutex<bool>,
    filter_node_id: Mutex<Option<u32>>,
    filter_state: Mutex<FilterState>,
    null_sink: Mutex<NullSinkState>,
    connected_devices: Mutex<Vec<u32>>,

    eq_bands: Mutex<Vec<EqBand>>,
    bypass: Mutex<bool>,
    preamp: Mutex<f32>,

    clients: Mutex<Vec<ClientHandle>>,
    shutting_down: AtomicBool,
}

struct ClientHandle {
    id: u64,
    tx: mpsc::Sender<String>,
}

impl DaemonState {
    pub fn new(pipeline: Arc<Pipeline>) -> Self {
        Self {
            pipeline,
            nodes: Mutex::new(Vec::new()),
            pw_connected: Mutex::new(false),
            filter_node_id: Mutex::new(None),
            filter_state: Mutex::new(FilterState::Unconnected),
            null_sink: Mutex::new(NullSinkState::NotLoaded),
            connected_devices: Mutex::new(Vec::new()),
            eq_bands: Mutex::new(Vec::new()),
            bypass: Mutex::new(false),
            preamp: Mutex::new(0.0),
            clients: Mutex::new(Vec::new()),
            shutting_down: AtomicBool::new(false),
        }
    }

    pub fn get_status(&self) -> DaemonStatus {
        DaemonStatus {
            bands: self.eq_bands.lock().unwrap().clone(),
            bypass: *self.bypass.lock().unwrap(),
            preamp: *self.preamp.lock().unwrap(),
            nodes: self.nodes.lock().unwrap().clone(),
            pw_connected: *self.pw_connected.lock().unwrap(),
            filter_state: self.filter_state.lock().unwrap().clone(),
            null_sink: self.null_sink.lock().unwrap().clone(),
            filter_node_id: *self.filter_node_id.lock().unwrap(),
            connected_devices: self.connected_devices.lock().unwrap().clone(),
        }
    }

    pub fn handle_pw_event(&self, event: PwEvent) {
        match &event {
            PwEvent::NodeList(nodes) => {
                (*self.nodes.lock().unwrap()).clone_from(nodes);
                self.push_event(PushEvent::NodeList {
                    nodes: nodes.clone(),
                });
            }
            PwEvent::Connected => {
                *self.pw_connected.lock().unwrap() = true;
                self.push_event(PushEvent::StateChange {
                    state: "connected".into(),
                });
            }
            PwEvent::FilterStateChanged(state) => {
                *self.filter_state.lock().unwrap() = state.clone();
                self.push_event(PushEvent::StateChange {
                    state: format!("filter:{state:?}"),
                });
            }
            PwEvent::FilterReady { node_id } => {
                *self.filter_node_id.lock().unwrap() = Some(*node_id);
                self.push_event(PushEvent::FilterReady {
                    node_id: *node_id,
                });
            }
            PwEvent::NullSinkCreated { module_id } => {
                *self.null_sink.lock().unwrap() = NullSinkState::Loaded {
                    module_id: *module_id,
                    has_source: false,
                };
                self.push_event(PushEvent::NullSinkCreated {
                    module_id: *module_id,
                });
            }
            PwEvent::NullSinkInputState { has_source } => {
                let mut ns = self.null_sink.lock().unwrap();
                if let NullSinkState::Loaded { has_source: hs, .. } = &mut *ns {
                    *hs = *has_source;
                }
                self.push_event(PushEvent::SourceActive {
                    active: *has_source,
                });
            }
            PwEvent::NullSinkError(msg) | PwEvent::Error(msg) => {
                error!(%msg, "PW error forwarded to clients");
                self.push_event(PushEvent::Error {
                    message: msg.clone(),
                });
            }
            PwEvent::NodeAdded(_) | PwEvent::NodeRemoved(_) => {
                // Not wired yet — periodic NodeList covers the same ground.
            }
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

        self.clients
            .lock()
            .unwrap()
            .push(ClientHandle { id: client_id, tx });
        info!(client_id, "Client connected");
    }

    pub fn unregister_client(&self, client_id: u64) {
        self.clients.lock().unwrap().retain(|c| c.id != client_id);
        info!(client_id, "Client disconnected");
    }

    pub fn push_event(&self, event: PushEvent) {
        let mut clients = self.clients.lock().unwrap();
        // Avoid CPU waste when daemon is running in background without connected TUI.
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

pub fn run(mut lock_file: std::fs::File) -> crate::AppResult<()> {
    // Write process PID to the lock file so that CLI commands (like stop/load)
    // can find the daemon. The exclusive lock is already held by the parent and
    // inherited by the child.
    use std::io::Write;
    lock_file.set_len(0)?; // Clear any previous content
    lock_file.write_all(std::process::id().to_string().as_bytes())?;
    lock_file.flush()?;

    let socket_path = socket_path()?;

    let pipeline = Arc::new(Pipeline::new(SAMPLE_RATE));
    let state = Arc::new(DaemonState::new(pipeline.clone()));

    let (pw_tx, pw_rx) = mpsc::channel::<PwEvent>();
    let (cmd_tx, cmd_rx) = channel::channel::<PwCommand>();

    let pw_pipeline = pipeline.clone();
    let pw_thread = thread::Builder::new()
        .name("pw".into())
        .spawn(move || {
            pw::run(pw_tx, cmd_rx, pw_pipeline);
        })?;

    let bridge_state = state.clone();
    let bridge_socket = socket_path.clone();
    thread::Builder::new()
        .name("pw-bridge".into())
        .spawn(move || {
            while let Ok(event) = pw_rx.recv() {
                bridge_state.handle_pw_event(event);
            }
            if bridge_state.shutting_down.load(Ordering::Relaxed) {
                info!("PW thread terminated cleanly");
            } else {
                error!("PW event channel closed unexpectedly — shutting down daemon");
                bridge_state.shutting_down.store(true, Ordering::Relaxed);
                // Unblock the accept loop so the daemon can exit on unexpected PW death.
                let _ = std::os::unix::net::UnixStream::connect(&bridge_socket);
            }
        })?;

    let peak_state = state.clone();
    thread::Builder::new()
        .name("peak-broadcast".into())
        .spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(66));
                if peak_state.shutting_down.load(Ordering::Relaxed) {
                    break;
                }
                let (l, r) = peak_state.pipeline.peaks();
                peak_state.push_event(PushEvent::PeakUpdate { l, r });
            }
        })?;

    // Remove stale socket from a previous crashed run.
    let _ = fs::remove_file(&socket_path);
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    info!("Daemon listening on {}", socket_path.display());

    let mut client_id_counter: u64 = 0;

    for stream in listener.incoming() {
        if state.shutting_down.load(Ordering::Relaxed) {
            break;
        }

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                error!(%e, "Accept error");
                continue;
            }
        };

        // Enforce concurrent connection limit to prevent resource exhaustion (DoS).
        // The length of the clients list is checked before spawning a new handler thread.
        {
            let clients = state.clients.lock().unwrap();
            if clients.len() >= MAX_CLIENTS {
                warn!(
                    limit = MAX_CLIENTS,
                    "Maximum concurrent clients reached; dropping connection"
                );
                let _ = send_resp(&stream, Response::error("Maximum concurrent clients reached"));
                continue;
            }
        }

        let handler_state = state.clone();
        let handler_cmd_tx = cmd_tx.clone();

        thread::Builder::new()
            .name(format!("client-{client_id_counter}"))
            .spawn(move || {
                handle_client(stream, handler_state, handler_cmd_tx, client_id_counter);
            })?;

        client_id_counter += 1;
    }

    info!("Daemon shutting down");
    let _ = cmd_tx.send(PwCommand::Terminate);
    let _ = pw_thread.join();
    let _ = fs::remove_file(&socket_path);
    info!("Daemon exited cleanly");
    Ok(())
}

fn handle_client(
    stream: UnixStream,
    state: Arc<DaemonState>,
    cmd_tx: channel::Sender<PwCommand>,
    client_id: u64,
) {
    // Verify that the connecting client is the same user as the daemon.
    // This prevents unauthorized local users from sending commands even if
    // they have filesystem access to the socket.
    let creds = match stream.initial_peer_credentials() {
        Ok(c) => c,
        Err(e) => {
            error!(%e, client_id, "Failed to get peer credentials; rejecting connection");
            return;
        }
    };

    let peer_uid = creds.euid();
    let my_uid = unsafe { libc::getuid() };

    if peer_uid != my_uid {
        warn!(
            client_id,
            peer_uid,
            my_uid,
            "Unauthorized connection attempt from different UID; rejecting"
        );
        return;
    }

    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            error!(%e, client_id, "Failed to clone client stream for reading");
            return;
        }
    };

    state.register_client(&stream, client_id);

    let reader = BufReader::new(read_stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Request = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_resp(&stream, Response::error(&format!("Invalid JSON: {e}")));
                continue;
            }
        };

        let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dispatch_request(req, &state, &cmd_tx)
        }))
        .unwrap_or_else(|_| Response::error("Internal daemon error"));

        let _ = send_resp(&stream, resp);
    }

    state.unregister_client(client_id);
}

fn dispatch_request(
    req: Request,
    state: &DaemonState,
    cmd_tx: &channel::Sender<PwCommand>,
) -> Response {
    match req {
        Request::GetStatus => Response::ok_with_status(state.get_status()),

        Request::SetBands { bands } => {
            let count = bands.len();
            (*state.eq_bands.lock().unwrap()).clone_from(&bands);
            let _ = state.pipeline.set_bands(bands, SAMPLE_RATE);
            info!(count, "Bands updated");
            Response::ok()
        }

        Request::SetPreamp { gain } => {
            *state.preamp.lock().unwrap() = gain;
            state.pipeline.set_preamp(gain);
            info!(gain, "Preamp updated");
            Response::ok()
        }

        Request::SetBypass { bypass } => {
            *state.bypass.lock().unwrap() = bypass;
            state.pipeline.set_bypass(bypass);
            info!(bypass, "Bypass toggled");
            Response::ok()
        }

        Request::ConnectDevice { node_id } => {
            let Some(filter_id) = *state.filter_node_id.lock().unwrap() else {
                return Response::error("Filter not ready yet");
            };
            state.connected_devices.lock().unwrap().push(node_id);
            let _ = cmd_tx.send(PwCommand::ConnectDevice {
                filter_id,
                node_id,
            });
            info!(node_id, "Device connected");
            Response::ok()
        }

        Request::DisconnectDevice { node_id } => {
            let Some(filter_id) = *state.filter_node_id.lock().unwrap() else {
                return Response::error("Filter not ready yet");
            };
            state
                .connected_devices
                .lock()
                .unwrap()
                .retain(|id| *id != node_id);
            let _ = cmd_tx.send(PwCommand::DisconnectDevice {
                filter_id,
                node_id,
            });
            info!(node_id, "Device disconnected");
            Response::ok()
        }

        Request::Shutdown => {
            info!("Shutdown requested by client");
            state.shutting_down.store(true, Ordering::Relaxed);
            let _ = cmd_tx.send(PwCommand::Terminate);
            // Connect to the socket to wake up the blocked incoming() loop so main can exit.
            if let Ok(path) = socket_path() {
                let _ = std::os::unix::net::UnixStream::connect(path);
            }
            Response::ok()
        }
    }
}

impl Response {
    fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            status: None,
        }
    }

    fn ok_with_status(status: DaemonStatus) -> Self {
        Self {
            ok: true,
            error: None,
            status: Some(status),
        }
    }

    fn error(msg: &str) -> Self {
        Self {
            ok: false,
            error: Some(msg.into()),
            status: None,
        }
    }
}

fn send_resp(mut stream: &UnixStream, resp: Response) -> std::io::Result<()> {
    let json = serde_json::to_string(&resp).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn socket_path() -> crate::AppResult<PathBuf> {
    Ok(runtime_dir()?.join("eqtui.sock"))
}

pub fn lock_path() -> crate::AppResult<PathBuf> {
    Ok(runtime_dir()?.join("eqtui.lock"))
}

/// Returns the XDG runtime directory for the current user.
///
/// This directory is used for the Unix socket and lock file.
/// Strict requirement for `XDG_RUNTIME_DIR` to be set for security;
/// falling back to /tmp would allow other local users to intercept
/// or control the daemon.
pub fn runtime_dir() -> crate::AppResult<PathBuf> {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "XDG_RUNTIME_DIR environment variable is not set or is empty. \
            This is required for secure operation.",
        )
        .into()),
    }
}
