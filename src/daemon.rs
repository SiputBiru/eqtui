//! Daemon mode — runs headless, owns PipeWire + EQ engine, serves
//! TUI / CLI clients over a Unix-domain socket.
//!
//! ## Architecture
//!
//! ```text
//!   equti daemon
//!     ├─ PW thread    (pw::run, unchanged)   ──mpsc──► bridge thread
//!     │                                                  │
//!     │  ◄── pipewire::channel ── cmd_tx ◄── DaemonState │
//!     │                                                  │
//!     ├─ bridge thread  (PwEvent → DaemonState → broadcast)
//!     ├─ peak thread    (reads pipeline atomics, pushes PeakUpdate)
//!     ├─ accept loop    (spawns handle_client per connection)
//!     └─ signal handler (SIGTERM/SIGINT → graceful teardown)
//! ```

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

use crate::pipeline::{Pipeline, SAMPLE_RATE};
use crate::protocol::{DaemonStatus, PushEvent, Request, Response};
use crate::pw;
use crate::state::{EqBand, FilterState, NodeInfo, NullSinkState, PwCommand, PwEvent};

// ── DaemonState ─────────────────────────────────────────────────────────────

/// Central daemon state shared between the PW thread, bridge thread,
/// client handlers, and peak broadcaster.
pub struct DaemonState {
    // ── DSP ──
    pub pipeline: Arc<Pipeline>,

    // ── Audio-graph state (mirrored from PW events) ──
    nodes: Mutex<Vec<NodeInfo>>,
    pw_connected: Mutex<bool>,
    filter_node_id: Mutex<Option<u32>>,
    filter_state: Mutex<FilterState>,
    null_sink: Mutex<NullSinkState>,
    connected_devices: Mutex<Vec<u32>>,

    // ── EQ parameters (the source of truth) ──
    eq_bands: Mutex<Vec<EqBand>>,
    bypass: Mutex<bool>,
    preamp: Mutex<f32>,

    // ── Connected clients (for push-event broadcast) ──
    clients: Mutex<Vec<ClientHandle>>,

    // ── Shutdown coordination ──
    shutting_down: AtomicBool,
}

/// Handle to one connected client — we only keep the sending side.
struct ClientHandle {
    id: u64,
    tx: mpsc::Sender<String>, // each message is a JSON line (with trailing \n)
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

    // ── State queries ────────────────────────────────────────────────────

    /// Snapshot all daemon state into a serializable struct for a client
    /// that just connected or issued `GetStatus`.
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

    // ── PW event ingestion (called by bridge thread) ────────────────────

    /// Consume a raw PipeWire event, update local state, and push
    /// the corresponding client-visible event to all connected clients.
    pub fn handle_pw_event(&self, event: PwEvent) {
        match &event {
            PwEvent::NodeList(nodes) => {
                *self.nodes.lock().unwrap() = nodes.clone();
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
                *self.null_sink.lock().unwrap() =
                    NullSinkState::Loaded {
                        module_id: *module_id,
                        has_source: false,
                    };
                self.push_event(PushEvent::NullSinkCreated {
                    module_id: *module_id,
                });
            }
            PwEvent::NullSinkInputState { has_source } => {
                // Update the has_source flag while preserving the module_id.
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
                // These variants exist for future hotplug — not wired yet.
                // The periodic NodeList snapshot covers the same ground.
            }
        }
    }

    // ── Client management ────────────────────────────────────────────────

    /// Register a new client so it receives push events (peaks, node
    /// lists, state changes).  The caller is responsible for spawning
    /// a reader thread that processes incoming `Request`s.
    pub fn register_client(&self, stream: &UnixStream, client_id: u64) {
        let (tx, rx) = mpsc::channel::<String>();

        // Clone the stream so the writer thread has its own fd.
        let write_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, "Failed to clone client stream for writer");
                return;
            }
        };

        // Spawn a dedicated writer thread that pumps mpsc → socket.
        thread::Builder::new()
            .name(format!("client-{client_id}-writer"))
            .spawn(move || {
                let mut w = write_stream;
                for msg in rx {
                    if w.write_all(msg.as_bytes()).is_err() {
                        break; // client disconnected
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

    /// Remove a client from the broadcast list.  Called when the
    /// client handler thread exits.
    pub fn unregister_client(&self, client_id: u64) {
        self.clients.lock().unwrap().retain(|c| c.id != client_id);
        info!(client_id, "Client disconnected");
    }

    // ── Broadcast ────────────────────────────────────────────────────────

    /// Push a JSON-line event to every connected client.  Dead
    /// clients (send error) are removed lazily.
    pub fn push_event(&self, event: PushEvent) {
        let json = match serde_json::to_string(&event) {
            Ok(j) => j + "\n",
            Err(e) => {
                error!(%e, "Failed to serialize push event");
                return;
            }
        };
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|c| c.tx.send(json.clone()).is_ok());
    }
}

// ── Entry point ────────────────────────────────────────────────────────────

pub fn run() -> crate::AppResult<()> {
    let socket_path = socket_path();
    let lock_path = lock_path();

    // ── Lock file (prevent duplicate daemons) ──────────────────────────
    check_lock_file(&lock_path)?;

    // ── Create the DSP pipeline ────────────────────────────────────────
    let pipeline = Arc::new(Pipeline::new(SAMPLE_RATE));

    // ── Shared daemon state ────────────────────────────────────────────
    let state = Arc::new(DaemonState::new(pipeline.clone()));

    // ── Channels for PW thread bridge ──────────────────────────────────
    //
    //   PW thread  ──pw_tx──→  bridge thread  ──→ DaemonState
    //   PW thread  ◄──cmd_rx── bridge thread  ◄── client commands
    //
    // We keep pw::run completely untouched; the bridge adapts its
    // mpsc / pipewire::channel interface to DaemonState + Unix socket.

    let (pw_tx, pw_rx) = mpsc::channel::<PwEvent>();           // PW → bridge
    let (cmd_tx, cmd_rx) = channel::channel::<PwCommand>();    // bridge → PW

    // ── Spawn the PipeWire thread (existing code, zero changes) ────────
    let pw_pipeline = pipeline.clone();
    let pw_thread = thread::Builder::new()
        .name("pw".into())
        .spawn(move || {
            pw::run(pw_tx, cmd_rx, pw_pipeline);
        })?;

    // ── Bridge thread: PW events → DaemonState ─────────────────────────
    let bridge_state = state.clone();
    thread::Builder::new()
        .name("pw-bridge".into())
        .spawn(move || {
            while let Ok(event) = pw_rx.recv() {
                bridge_state.handle_pw_event(event);
            }
            info!("PW event channel closed — bridge thread exiting");
        })?;

    // ── Peak broadcast thread ──────────────────────────────────────────
    let peak_state = state.clone();
    thread::Builder::new()
        .name("peak-broadcast".into())
        .spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(66)); // ~15 fps
                if peak_state.shutting_down.load(Ordering::Relaxed) {
                    break;
                }
                let (l, r) = peak_state.pipeline.peaks();
                peak_state.push_event(PushEvent::PeakUpdate { l, r });
            }
        })?;

    // ── Signal handler (SIGTERM / SIGINT → graceful shutdown) ──────────
    //
    // We use a separate thread because signal_hook would add a
    // dependency.  For now we rely on the TUI's quit key or `eqtui
    // stop` to trigger shutdown; SIGTERM handling can be added
    // later with the `signal-hook` crate if desired.
    //
    // When a client sends Shutdown, the handler thread sets
    // `shutting_down` and sends Terminate via cmd_tx.

    // ── Unix socket ────────────────────────────────────────────────────
    //
    // Remove stale socket from a previous crashed run.
    let _ = fs::remove_file(&socket_path);

    // Ensure parent directory exists.
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    info!("Daemon listening on {}", socket_path.display());

    // ── Accept loop ────────────────────────────────────────────────────
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

        let handler_state = state.clone();
        let handler_cmd_tx = cmd_tx.clone();

        thread::Builder::new()
            .name(format!("client-{client_id_counter}"))
            .spawn(move || {
                handle_client(
                    stream,
                    handler_state,
                    handler_cmd_tx,
                    client_id_counter,
                );
            })?;

        client_id_counter += 1;
    }

    // ── Shutdown ───────────────────────────────────────────────────────
    info!("Daemon shutting down");

    // Tell PW thread to tear down the filter and null sink.
    let _ = cmd_tx.send(PwCommand::Terminate);
    let _ = pw_thread.join();

    // Clean up socket and lock files.
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&lock_path);

    info!("Daemon exited cleanly");
    Ok(())
}

// ── Per-client handler ─────────────────────────────────────────────────────

fn handle_client(
    stream: UnixStream,
    state: Arc<DaemonState>,
    cmd_tx: channel::Sender<PwCommand>,
    client_id: u64,
) {
    // Clone the stream for reading (the writer half is spawned by
    // register_client so we keep the original for send_resp).
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            error!(%e, "Failed to clone client stream for reading");
            return;
        }
    };

    // Register this client for push-event broadcast.
    // A writer thread is spawned internally that holds the cloned write fd.
    state.register_client(&stream, client_id);

    let reader = BufReader::new(read_stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // client disconnected / I/O error
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue; // skip blank lines (keep-alive friendly)
        }

        let req: Request = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_resp(
                    &stream,
                    Response::error(&format!("Invalid JSON: {e}")),
                );
                continue;
            }
        };

        let resp = dispatch_request(req, &state, &cmd_tx);
        let _ = send_resp(&stream, resp);
    }

    state.unregister_client(client_id);
}

// ── Request dispatch ───────────────────────────────────────────────────────

fn dispatch_request(
    req: Request,
    state: &DaemonState,
    cmd_tx: &channel::Sender<PwCommand>,
) -> Response {
    match req {
        Request::GetStatus => Response::ok_with_status(state.get_status()),

        Request::SetBands { bands } => {
            let count = bands.len();
            *state.eq_bands.lock().unwrap() = bands.clone();
            let _ = state.pipeline.set_bands(bands, SAMPLE_RATE);
            info!(count, "Bands updated");
            Response::ok()
        }

        Request::SetPreamp { gain } => {
            *state.preamp.lock().unwrap() = gain;
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
            // Need the filter node ID to create links.
            let filter_id = match *state.filter_node_id.lock().unwrap() {
                Some(id) => id,
                None => return Response::error("Filter not ready yet"),
            };
            state
                .connected_devices
                .lock()
                .unwrap()
                .push(node_id);
            let _ = cmd_tx.send(PwCommand::ConnectDevice {
                filter_id,
                node_id,
            });
            info!(node_id, "Device connected");
            Response::ok()
        }

        Request::DisconnectDevice { node_id } => {
            let filter_id = match *state.filter_node_id.lock().unwrap() {
                Some(id) => id,
                None => return Response::error("Filter not ready yet"),
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

        Request::LoadPeq { path } => {
            // Delegate to the PEQ parser (Phase 2C — placeholder for now).
            info!(%path, "LoadPeq requested (not yet implemented)");
            Response::error("PEQ import not yet implemented (Phase 2C)")
        }

        Request::Shutdown => {
            info!("Shutdown requested by client");
            state.shutting_down.store(true, Ordering::Relaxed);
            let _ = cmd_tx.send(PwCommand::Terminate);
            Response::ok()
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

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

/// Write a JSON-line response to the client socket (best-effort —
/// if the client has disconnected we silently drop it).
fn send_resp(mut stream: &UnixStream, resp: Response) -> std::io::Result<()> {
    let json = serde_json::to_string(&resp).unwrap();
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;
    Ok(())
}

// ── Platform paths ─────────────────────────────────────────────────────────

fn socket_path() -> PathBuf {
    runtime_dir().join("eqtui.sock")
}

fn lock_path() -> PathBuf {
    runtime_dir().join("eqtui.lock")
}

fn runtime_dir() -> PathBuf {
    // $XDG_RUNTIME_DIR is user-specific tmpfs, perfect for sockets
    // and lock files — auto-cleaned on logout.
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/tmp")
    }
}

fn check_lock_file(path: &PathBuf) -> crate::AppResult<()> {
    if let Ok(contents) = fs::read_to_string(path) {
        let pid: i32 = contents.trim().parse().unwrap_or(0);
        if pid > 0 && pid_alive(pid) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Daemon already running (PID {pid}). Use `eqtui stop` to stop it first."),
            )
            .into());
        }
        // PID exists in lock file but process is dead — stale lock.
        warn!("Removing stale lock file (PID {pid} is dead)");
        let _ = fs::remove_file(path);
    }

    // Write our PID.
    fs::write(path, std::process::id().to_string())?;
    Ok(())
}

/// Check whether a process with the given PID exists on Linux.
fn pid_alive(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}
