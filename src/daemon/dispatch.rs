// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Request dispatch: validate and apply a `Request` against daemon state.

use pipewire::channel;
use tracing::info;

use crate::paths::socket_path;
use crate::protocol::{Request, Response};
use crate::state::{EqBand, MAX_ABS_PREAMP_DB, MAX_BANDS, PwCommand};

use super::state::DaemonState;

/// Validates and applies one client request. Returns the response to send.
pub(super) fn dispatch_request(
    req: Request,
    state: &DaemonState,
    cmd_tx: &channel::Sender<PwCommand>,
) -> Response {
    match req {
        Request::GetStatus => Response {
            ok: true,
            error: None,
            status: Some(state.get_status()),
        },

        Request::SetBands { bands } => {
            if bands.len() > MAX_BANDS {
                return Response {
                    ok: false,
                    error: Some(format!("too many bands: {} (max {MAX_BANDS})", bands.len())),
                    status: None,
                };
            }
            if let Err(reason) = bands.iter().try_for_each(EqBand::validate) {
                return Response {
                    ok: false,
                    error: Some(reason),
                    status: None,
                };
            }
            let count = bands.len();
            state.status.lock().unwrap().bands.clone_from(&bands);
            let _ = cmd_tx.send(PwCommand::UpdateEq { bands });
            info!(count, "Bands queued for EQ update");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::SetPreamp { gain } => {
            if !gain.is_finite() || gain.abs() > MAX_ABS_PREAMP_DB {
                return Response {
                    ok: false,
                    error: Some(format!(
                        "preamp {gain} dB out of range ±{MAX_ABS_PREAMP_DB}"
                    )),
                    status: None,
                };
            }
            // WHY the double write: Pipeline's atomics are read by the
            // real-time audio callback, which must never block on a mutex.
            // The status copy is what GetStatus reports. Keep both.
            state.status.lock().unwrap().preamp = gain; // authoritative for UI
            state.pipeline.set_preamp(gain); // authoritative for AUDIO
            info!(gain, "Preamp updated");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::SetBypass { bypass } => {
            // Same dual-write rationale as SetPreamp.
            state.status.lock().unwrap().bypass = bypass;
            state.pipeline.set_bypass(bypass);
            info!(bypass, "Bypass toggled");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::ConnectDevice { node_id } => {
            let mut s = state.status.lock().unwrap();
            let Some(filter_id) = s.filter_node_id else {
                return Response {
                    ok: false,
                    error: Some("Filter not ready yet".into()),
                    status: None,
                };
            };
            if node_id == filter_id {
                return Response {
                    ok: false,
                    error: Some("Cannot connect filter to itself".into()),
                    status: None,
                };
            }
            if s.null_sink.module_id() == Some(node_id) {
                return Response {
                    ok: false,
                    error: Some(
                        "Cannot connect to the null sink (would create a feedback loop)".into(),
                    ),
                    status: None,
                };
            }
            // Idempotent: already connected or an op is in flight.
            if s.connected_devices.contains(&node_id) || s.pending_devices.contains(&node_id) {
                return Response {
                    ok: true,
                    error: None,
                    status: None,
                };
            }
            s.pending_devices.push(node_id);
            drop(s); // don't hold the lock across the channel send

            if cmd_tx
                .send(PwCommand::ConnectDevice { filter_id, node_id })
                .is_err()
            {
                // pw thread is gone: roll back and say so.
                state
                    .status
                    .lock()
                    .unwrap()
                    .pending_devices
                    .retain(|id| *id != node_id);
                return Response {
                    ok: false,
                    error: Some("PipeWire thread unavailable".into()),
                    status: None,
                };
            }
            info!(node_id, "Device connect queued");
            Response {
                ok: true,
                error: None,
                status: None,
            } // ok = "accepted", confirmed later
        }

        Request::DisconnectDevice { node_id } => {
            let mut s = state.status.lock().unwrap();
            let Some(filter_id) = s.filter_node_id else {
                return Response {
                    ok: false,
                    error: Some("Filter not ready yet".into()),
                    status: None,
                };
            };
            let is_connected = s.connected_devices.contains(&node_id);
            let is_pending = s.pending_devices.contains(&node_id);
            if !is_connected && !is_pending {
                return Response {
                    ok: false,
                    error: Some(format!("Device {node_id} is not connected")),
                    status: None,
                };
            }
            if !is_pending {
                s.pending_devices.push(node_id);
            }
            drop(s);

            if cmd_tx
                .send(PwCommand::DisconnectDevice { filter_id, node_id })
                .is_err()
            {
                state
                    .status
                    .lock()
                    .unwrap()
                    .pending_devices
                    .retain(|id| *id != node_id);
                return Response {
                    ok: false,
                    error: Some("PipeWire thread unavailable".into()),
                    status: None,
                };
            }
            info!(node_id, "Device disconnect queued");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::Shutdown => {
            info!("Shutdown requested by client");
            state
                .shutting_down
                .store(true, std::sync::atomic::Ordering::Release);
            let _ = cmd_tx.send(PwCommand::Terminate);
            if let Ok(path) = socket_path() {
                let _ = std::os::unix::net::UnixStream::connect(path);
            }
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::pipeline::{Pipeline, SAMPLE_RATE};

    /// A connect is pending until the `LinkResult` confirms it: state never
    /// claims "connected" before a link actually exists.
    #[test]
    fn connect_is_pending_until_confirmed() {
        let (cmd_tx, cmd_rx) = pipewire::channel::channel::<PwCommand>();
        let state = DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE)));
        state.status.lock().unwrap().filter_node_id = Some(42);

        let resp = dispatch_request(Request::ConnectDevice { node_id: 7 }, &state, &cmd_tx);
        assert!(resp.ok);
        let s = state.get_status();
        assert!(s.connected_devices.is_empty()); // NOT yet connected
        assert!(s.pending_devices.contains(&7)); // in flight

        state.handle_pw_event(crate::state::PwEvent::LinkResult {
            device_id: 7,
            connect: true,
            ok: true,
        });
        let s = state.get_status();
        assert!(s.connected_devices.contains(&7)); // now confirmed
        assert!(!s.pending_devices.contains(&7)); // cleared
        drop(cmd_rx);
    }

    /// A failed link never shows as connected, and the pending entry is
    /// cleared.
    #[test]
    fn failed_link_never_shows_connected() {
        let (cmd_tx, cmd_rx) = pipewire::channel::channel::<PwCommand>();
        let state = DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE)));
        state.status.lock().unwrap().filter_node_id = Some(42);

        let resp = dispatch_request(Request::ConnectDevice { node_id: 7 }, &state, &cmd_tx);
        assert!(resp.ok);

        state.handle_pw_event(crate::state::PwEvent::LinkResult {
            device_id: 7,
            connect: true,
            ok: false,
        });
        let s = state.get_status();
        assert!(s.connected_devices.is_empty());
        assert!(!s.pending_devices.contains(&7)); // cleared
        drop(cmd_rx);
    }

    /// Disconnect of a device that is neither connected nor pending is an
    /// error: no pointless pw-link -d.
    #[test]
    fn disconnect_requires_connection() {
        let (cmd_tx, cmd_rx) = pipewire::channel::channel::<PwCommand>();
        let state = DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE)));
        state.status.lock().unwrap().filter_node_id = Some(42);

        let resp = dispatch_request(Request::DisconnectDevice { node_id: 7 }, &state, &cmd_tx);
        assert!(!resp.ok); // not connected → error

        // Connect + confirm, then disconnect must be accepted.
        let _ = dispatch_request(Request::ConnectDevice { node_id: 7 }, &state, &cmd_tx);
        state.handle_pw_event(crate::state::PwEvent::LinkResult {
            device_id: 7,
            connect: true,
            ok: true,
        });
        let resp = dispatch_request(Request::DisconnectDevice { node_id: 7 }, &state, &cmd_tx);
        assert!(resp.ok);
        drop(cmd_rx);
    }

    /// A device that vanished from the node list is pruned from routing
    /// state, even while its connect is still pending.
    #[test]
    fn node_list_prunes_vanished_devices() {
        let (cmd_tx, cmd_rx) = pipewire::channel::channel::<PwCommand>();
        let state = DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE)));
        state.status.lock().unwrap().filter_node_id = Some(42);
        let _ = dispatch_request(Request::ConnectDevice { node_id: 7 }, &state, &cmd_tx);
        assert!(state.get_status().pending_devices.contains(&7));

        // The node list no longer contains device 7 (it vanished mid-op).
        state.handle_pw_event(crate::state::PwEvent::NodeList(vec![]));

        assert!(state.get_status().pending_devices.is_empty());
        drop(cmd_rx);
    }
}
