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
                let nodes = nodes.clone();
                (*self.nodes.lock().unwrap()).clone_from(&nodes);
                self.push_event(PushEvent::NodeList { nodes });
            }
            PwEvent::Connected => {
                *self.pw_connected.lock().unwrap() = true;
                self.push_event(PushEvent::StateChange {
                    state: "connected".into(),
                });
            }
            PwEvent::FilterStateChanged(state) => {
                *self.filter_state.lock().unwrap() = state.clone();
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
                *self.filter_node_id.lock().unwrap() = Some(*node_id);
                self.push_event(PushEvent::FilterReady { node_id: *node_id });
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
    let pw_thread = thread::Builder::new().name("pw".into()).spawn(move || {
        pw::run(pw_tx, cmd_rx, pw_pipeline);
    })?;

    // Bridge thread — forwards PwEvents from PipeWire to the shared state.
    let bridge_state = state.clone();
    let bridge_socket = socket_path.clone();
    thread::Builder::new()
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
    thread::Builder::new()
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
            (*state.eq_bands.lock().unwrap()).clone_from(&bands);
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
            *state.preamp.lock().unwrap() = gain;
            state.pipeline.set_preamp(gain);
            info!(gain, "Preamp updated");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::SetBypass { bypass } => {
            *state.bypass.lock().unwrap() = bypass;
            state.pipeline.set_bypass(bypass);
            info!(bypass, "Bypass toggled");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::ConnectDevice { node_id } => {
            let Some(filter_id) = *state.filter_node_id.lock().unwrap() else {
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
            if let Some(ns_id) = state.null_sink.lock().unwrap().module_id()
                && node_id == ns_id
            {
                return Response {
                    ok: false,
                    error: Some(
                        "Cannot connect to the null sink (would create a feedback loop)".into(),
                    ),
                    status: None,
                };
            }
            {
                let devices = state.connected_devices.lock().unwrap();
                if devices.contains(&node_id) {
                    return Response {
                        ok: true,
                        error: None,
                        status: None,
                    };
                }
            }
            state.connected_devices.lock().unwrap().push(node_id);
            let _ = cmd_tx.send(PwCommand::ConnectDevice { filter_id, node_id });
            info!(node_id, "Device connected");
            Response {
                ok: true,
                error: None,
                status: None,
            }
        }

        Request::DisconnectDevice { node_id } => {
            let Some(filter_id) = *state.filter_node_id.lock().unwrap() else {
                return Response {
                    ok: false,
                    error: Some("Filter not ready yet".into()),
                    status: None,
                };
            };
            state
                .connected_devices
                .lock()
                .unwrap()
                .retain(|id| *id != node_id);
            let _ = cmd_tx.send(PwCommand::DisconnectDevice { filter_id, node_id });
            info!(node_id, "Device disconnected");
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
}
