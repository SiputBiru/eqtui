// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Daemon process — owns the `PipeWire` audio pipeline and serves
//! TUI/CLI clients over a Unix-domain socket.

use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use pipewire::channel;
use tracing::{debug, error, info, warn};
use uds::UnixStreamExt;

use crate::paths::{lock_path, socket_path, validate_runtime_dir};
use crate::pipeline::{Pipeline, SAMPLE_RATE};
use crate::protocol::{DaemonStatus, PushEvent, Request, Response};
use crate::state::{
    EqBand, FilterState, MAX_ABS_PREAMP_DB, MAX_BANDS, NodeInfo, NullSinkState, PwCommand, PwEvent,
};
use crate::{AppResult, pw};

/// One logical daemon status, protected by a single mutex so a snapshot is
/// internally consistent by construction.
#[derive(Default)]
struct StatusSnapshot {
    bands: Vec<EqBand>,
    bypass: bool,
    preamp: f32,
    nodes: Vec<NodeInfo>,
    pw_connected: bool,
    filter_state: FilterState,
    null_sink: NullSinkState,
    filter_node_id: Option<u32>,
    connected_devices: Vec<u32>, // CONFIRMED links only
    pending_devices: Vec<u32>,   // ops in flight (connect or disconnect)
}

pub struct DaemonState {
    pub pipeline: Arc<Pipeline>,

    // One lock for the whole status snapshot — atomic reads and writes.
    status: Mutex<StatusSnapshot>,

    // Not part of the status snapshot; operational only. Lock order rule:
    // `status` → `clients`, never the reverse. In practice: push_event
    // (locks `clients`) must only ever be called with no lock held; mutation
    // code locks `status`, drops it, THEN pushes.
    clients: Mutex<Vec<ClientHandle>>,
    shutting_down: Arc<AtomicBool>,
}

struct ClientHandle {
    id: u64,
    tx: mpsc::Sender<String>,
    /// Clone kept only to unblock the handler's reader during shutdown
    /// (`shutdown(Both)` makes the blocked `read_until` error out).
    stream: UnixStream,
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

// ── Entry Point ─────────────────────────────────────────────────
//
// Sets up the lock file, starts the PipeWire pipeline, and listens
// on a Unix socket for TUI/CLI client connections.

/// Returns `true` if the connected peer belongs to the expected uid.
///
/// Uses `SO_PEERCRED` (via `uds`) so a swapped-out or misconfigured runtime
/// directory cannot open the command socket to other users. On failure to
/// read credentials the connection is rejected (fail closed).
fn peer_is_self(stream: &UnixStream, euid: u32) -> bool {
    match stream.initial_peer_credentials() {
        Ok(cred) if cred.euid() == euid => true,
        Ok(cred) => {
            warn!(
                euid = cred.euid(),
                "Rejected IPC connection from foreign uid"
            );
            false
        }
        Err(e) => {
            warn!(%e, "Could not read peer credentials; rejecting");
            false
        }
    }
}

/// Acquires the single-instance lock and records this process's PID.
///
/// Ordering matters: open WITHOUT truncating, take the flock, and only then
/// replace the file contents. A second instance must never erase the running
/// daemon's PID before it discovers it failed to get the lock.
///
/// Returns the locked, PID-written file handle. The caller must keep it alive
/// for as long as the daemon runs — dropping it releases the flock and a
/// second daemon could start.
fn acquire_lock(path: &Path) -> AppResult<fs::File> {
    // Open WITHOUT truncating — a second instance must not destroy
    // the running daemon's metadata before it owns the lock.
    let mut lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;

    // SAFETY: flock is async-signal-safe. LOCK_NB → fail fast instead of blocking.
    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == -1 {
        // Best-effort: tell the user which PID holds the lock.
        let mut pid = String::new();
        let _ = lock_file.read_to_string(&mut pid);
        eprintln!(
            "Daemon already running (pid {}). Use `eqtui stop` to stop it first.",
            pid.trim()
        );
        std::process::exit(1);
    }

    // We hold the lock — NOW it's safe to replace the metadata.
    lock_file.set_len(0)?; // truncate while holding the lock
    lock_file.seek(std::io::SeekFrom::Start(0))?;
    writeln!(&lock_file, "{}", std::process::id())?;
    lock_file.sync_all()?;

    Ok(lock_file)
}

pub fn run_daemon() -> AppResult<()> {
    tracing::info!("Daemon starting up");

    // Fail closed: refuse to run in an unsafe runtime directory.
    let run_dir = validate_runtime_dir()?;

    // Private 0700 subdirectory for the socket + lock. The subdir matters
    // because bind→chmod has a small window where the socket node exists with
    // umask-derived perms; inside a 0700 dir that window is unreachable.
    let eqtui_dir = run_dir.join("eqtui");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&eqtui_dir)?;
    // create() doesn't tighten perms on an existing dir — enforce:
    fs::set_permissions(&eqtui_dir, fs::Permissions::from_mode(0o700))?;

    // Acquire exclusive lock so only one daemon instance runs.
    let lock_path = lock_path()?;
    // NOTE: `lock_file` must stay alive for the whole function — dropping it
    // releases the flock and a second daemon could start.
    let _lock_file = acquire_lock(&lock_path)?;

    let socket_path = socket_path()?;
    let pipeline = Arc::new(Pipeline::new(SAMPLE_RATE));
    let state = Arc::new(DaemonState::new(pipeline.clone()));

    let (pw_tx, pw_rx) = mpsc::channel::<PwEvent>();
    let (cmd_tx, cmd_rx) = channel::channel::<PwCommand>();

    // PipeWire mainloop thread — audio processing and graph management.
    let pw_pipeline = pipeline.clone();
    let pw_shutdown = state.shutting_down.clone(); // Arc<AtomicBool>, shared
    let pw_thread = thread::Builder::new().name("pw".into()).spawn(move || {
        pw::run(pw_tx, cmd_rx, pw_pipeline, pw_shutdown);
    })?;

    // Bridge thread — forwards PwEvents from PipeWire to the shared state.
    let bridge_state = state.clone();
    let bridge_socket = socket_path.clone();
    let bridge = thread::Builder::new()
        .name("pw-bridge".into())
        .spawn(move || {
            while let Ok(event) = pw_rx.recv() {
                bridge_state.handle_pw_event(event);
            }
            if !bridge_state.shutting_down.load(Ordering::Acquire) {
                error!("PW event channel closed unexpectedly — shutting down daemon");
                bridge_state.shutting_down.store(true, Ordering::Release);
                if let Err(e) = std::os::unix::net::UnixStream::connect(&bridge_socket) {
                    debug!(%e, "Failed to connect to socket to unblock accept loop");
                }
            }
        })?;

    // Peak broadcast thread — pushes peak meter updates at ~15 fps.
    let peak_state = state.clone();
    let peak = thread::Builder::new()
        .name("peak-broadcast".into())
        .spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(66));
                if peak_state.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                let (l, r) = peak_state.pipeline.peaks();
                peak_state.push_event(PushEvent::PeakUpdate { l, r });
            }
        })?;

    // Remove a stale socket from a previous crashed run — but ONLY if it
    // really is a socket. Never unlink a regular file or symlink planted at
    // the socket path.
    match fs::symlink_metadata(&socket_path) {
        Ok(md) if md.file_type().is_socket() => fs::remove_file(&socket_path)?,
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "{} exists and is not a socket; refusing to remove",
                    socket_path.display()
                ),
            )
            .into());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    let listener = UnixListener::bind(&socket_path)?;
    // Explicit 0600 regardless of umask — the daemon is the enforcer.
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    info!("Daemon listening on {}", socket_path.display());

    let mut client_id_counter: u64 = 0;
    // Kept so shutdown can join handlers; finished ones are reaped on each
    // accept so the Vec doesn't grow forever.
    let mut client_handles: Vec<thread::JoinHandle<()>> = Vec::new();

    for stream in listener.incoming() {
        if state.shutting_down.load(Ordering::Acquire) {
            break;
        }

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                error!(%e, "Accept error");
                continue;
            }
        };

        // Verify the peer's UID before serving it. Even with a 0600 socket in
        // a 0700 directory, SO_PEERCRED defends against a swapped-out runtime
        // dir and gives us the peer PID for logs.
        // SAFETY: geteuid() never fails and takes no arguments.
        let euid = unsafe { libc::geteuid() };
        if !peer_is_self(&stream, euid) {
            continue; // stream dropped = connection closed
        }

        client_handles.retain(|h| !h.is_finished());
        let handler_state = state.clone();
        let handler_cmd_tx = cmd_tx.clone();
        let h = thread::Builder::new()
            .name(format!("client-{client_id_counter}"))
            .spawn(move || {
                handle_client(stream, handler_state, handler_cmd_tx, client_id_counter);
            })?;
        client_handles.push(h);

        client_id_counter += 1;
    }

    info!("Daemon shutting down");

    // 1. Tell PipeWire to quit: mainloop returns, pw::run cancels + joins its
    //    checker, joins the link worker, then returns — dropping the last
    //    PwEvent senders. Then join the pw thread.
    let _ = cmd_tx.send(PwCommand::Terminate);
    if let Err(e) = pw_thread.join() {
        error!("pw thread panicked: {e:?}");
    }

    // 2. Bridge: its pw_rx.recv() errors once all senders are gone; it sees
    //    shutting_down and returns quietly. Cannot hang — every sender drops
    //    before pw_thread.join() returns (see load-bearing destroy() frees).
    if let Err(e) = bridge.join() {
        error!("pw-bridge panicked: {e:?}");
    }

    // 3. Peak broadcaster: exits within 66 ms on the shutdown flag.
    if let Err(e) = peak.join() {
        error!("peak-broadcast panicked: {e:?}");
    }

    // 4. Clients: close every stream so blocked read_until calls error out,
    //    then join the handlers. Drain first — holding the clients lock across
    //    shutdown() would block handlers trying to unregister.
    let drained: Vec<ClientHandle> = state.clients.lock().unwrap().drain(..).collect();
    for c in drained {
        let _ = c.stream.shutdown(std::net::Shutdown::Both);
    }
    for h in client_handles {
        let _ = h.join();
    }
    // Writer threads stay detached: they die when their channel closes (the
    // ClientHandle senders were dropped by the drain above) at the latest.

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
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            error!(%e, client_id, "Failed to clone client stream for reading");
            return;
        }
    };

    state.register_client(&stream, client_id);

    // 64 KiB covers ~31 bands with generous headroom; anything larger is hostile.
    const MAX_REQUEST_LINE: u64 = 64 * 1024;

    let mut reader = BufReader::new(read_stream);
    loop {
        let mut buf = Vec::new();
        // `take` caps bytes read from the stream, so a missing newline can't
        // grow `buf` past the limit.
        let n = match reader
            .by_ref()
            .take(MAX_REQUEST_LINE + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(n) => n,
            Err(e) => {
                debug!(%e, client_id, "Read error; closing connection");
                break;
            }
        };
        if n == 0 {
            break; // client closed
        }
        if buf.len() as u64 > MAX_REQUEST_LINE || !buf.ends_with(b"\n") {
            let _ = send_resp(
                &stream,
                Response {
                    ok: false,
                    error: Some(format!("request line exceeds {MAX_REQUEST_LINE} bytes")),
                    status: None,
                },
            );
            warn!(client_id, "Oversized request line; disconnecting client");
            break;
        }

        let trimmed = String::from_utf8_lossy(&buf);
        let trimmed = trimmed.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Request = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_resp(
                    &stream,
                    Response {
                        ok: false,
                        error: Some(format!("Invalid JSON: {e}")),
                        status: None,
                    },
                );
                continue;
            }
        };

        let resp = dispatch_request(req, &state, &cmd_tx);
        let _ = send_resp(&stream, resp);
    }

    state.unregister_client(client_id);
}

fn send_resp(mut stream: &UnixStream, resp: Response) -> std::io::Result<()> {
    let json = serde_json::to_string(&resp)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn dispatch_request(
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
                // pw thread is gone — roll back and say so.
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
            state.shutting_down.store(true, Ordering::Release);
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

    /// A same-process socket pair reports the caller's own uid on Linux.
    #[test]
    fn peer_credentials_report_own_uid() {
        // Arrange
        let (a, b) = UnixStream::pair().expect("socketpair should succeed");
        let euid = unsafe { libc::geteuid() };

        // Act
        let cred_a = a.initial_peer_credentials().expect("peer creds readable");
        let cred_b = b.initial_peer_credentials().expect("peer creds readable");

        // Assert
        assert_eq!(cred_a.euid(), euid);
        assert_eq!(cred_b.euid(), euid);
    }

    /// `peer_is_self` accepts a same-uid peer.
    #[test]
    fn peer_is_self_accepts_same_user() {
        // Arrange
        let (a, _b) = UnixStream::pair().expect("socketpair should succeed");
        let euid = unsafe { libc::geteuid() };

        // Act
        let accepted = peer_is_self(&a, euid);

        // Assert
        assert!(accepted, "same-euid peer must be accepted");
    }

    /// `peer_is_self` rejects a peer whose uid differs from the expected one.
    #[test]
    fn peer_is_self_rejects_foreign_uid() {
        // Arrange
        let (a, _b) = UnixStream::pair().expect("socketpair should succeed");
        let euid = unsafe { libc::geteuid() };
        // Flip the lowest bit: always a different uid from our own.
        let foreign = euid ^ 1;

        // Act
        let accepted = peer_is_self(&a, foreign);

        // Assert
        assert!(!accepted, "peer with foreign uid must be rejected");
    }

    /// A second instance opening the lock file must not be able to corrupt
    /// the running daemon's PID before it fails to acquire the flock.
    #[test]
    fn second_instance_cannot_corrupt_lock() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqtui.lock");

        // Act — first instance holds the lock and has written its PID.
        let _first = acquire_lock(&path).unwrap();
        let first_pid = std::fs::read_to_string(&path).unwrap();
        // Sanity: the PID recorded is our own process.
        assert_eq!(first_pid.trim(), std::process::id().to_string());

        // Simulate instance B: open WITHOUT truncate (the fix), flock must fail.
        let second = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let rc = unsafe { libc::flock(second.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, -1);

        // Assert — the first instance's PID survived B's attempt.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first_pid);
    }

    /// A profile apply (bands + preamp changed under ONE lock) is observed
    /// atomically by any reader — there is no interleaving that yields the
    /// new bands with the old preamp.
    #[test]
    fn status_snapshot_is_internally_consistent() {
        use crate::state::FilterType;
        let state = DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE)));

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
        let state = Arc::new(DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE))));
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

    /// A connect is pending until the `LinkResult` confirms it — state never
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

        state.handle_pw_event(PwEvent::LinkResult {
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

        state.handle_pw_event(PwEvent::LinkResult {
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
    /// error — no pointless pw-link -d.
    #[test]
    fn disconnect_requires_connection() {
        let (cmd_tx, cmd_rx) = pipewire::channel::channel::<PwCommand>();
        let state = DaemonState::new(Arc::new(Pipeline::new(SAMPLE_RATE)));
        state.status.lock().unwrap().filter_node_id = Some(42);

        let resp = dispatch_request(Request::DisconnectDevice { node_id: 7 }, &state, &cmd_tx);
        assert!(!resp.ok); // not connected → error

        // Connect + confirm, then disconnect must be accepted.
        let _ = dispatch_request(Request::ConnectDevice { node_id: 7 }, &state, &cmd_tx);
        state.handle_pw_event(PwEvent::LinkResult {
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
        state.handle_pw_event(PwEvent::NodeList(vec![]));

        assert!(state.get_status().pending_devices.is_empty());
        drop(cmd_rx);
    }
}
