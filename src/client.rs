//! Unix-socket client for communicating with the eqtui daemon.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::{info, warn};

use crate::protocol::{DaemonStatus, PushEvent, Request, Response};
use crate::state::EqBand;

pub struct DaemonClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl DaemonClient {
    /// Connect to the daemon.  If no daemon is running, spawn one
    /// automatically (`fork` + `exec $0 daemon`) and retry for up
    /// to 3 seconds.
    pub fn connect() -> crate::AppResult<Self> {
        let path = socket_path();

        // First attempt — daemon might already be running.
        if let Ok(client) = Self::try_connect(&path) {
            info!("Connected to existing daemon");
            return Ok(client);
        }

        // Auto-launch the daemon.
        info!("No daemon found — auto-launching");
        spawn_daemon();

        // Retry every 100ms for up to 3 seconds.
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(client) = Self::try_connect(&path) {
                info!("Connected to auto-launched daemon");
                return Ok(client);
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Daemon failed to start within 3 seconds",
        )
        .into())
    }

    /// Raw connect attempt — no auto-launch.
    fn try_connect(path: &PathBuf) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_nonblocking(false)?; // blocking reads for request/response
        let reader = BufReader::new(
            stream
                .try_clone()
                .expect("BUG: UnixStream::try_clone failed"),
        );
        Ok(Self { stream, reader })
    }

    // ── Request / response ──────────────────────────────────────────

    /// Send a request and wait for the response.
    pub fn request(&mut self, req: Request) -> crate::AppResult<Response> {
        let json = serde_json::to_string(&req)?;
        self.stream.write_all(json.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;

        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let resp: Response = serde_json::from_str(line.trim())?;
        Ok(resp)
    }

    /// Non-blocking read for a pushed event (peaks, node lists, etc.).
    /// Returns `None` if no data is available.
    pub fn try_read_event(&mut self) -> std::io::Result<Option<PushEvent>> {
        // Switch to non-blocking temporarily.
        self.reader.get_mut().set_nonblocking(true)?;
        let mut line = String::new();
        let result = match self.reader.read_line(&mut line) {
            Ok(0) => Ok(None),           // EOF — daemon disconnected
            Ok(_) => {
                let event: PushEvent = serde_json::from_str(line.trim())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Some(event))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        };
        // Restore blocking mode for request/response.
        self.reader.get_mut().set_nonblocking(false)?;
        result
    }

    // ── Convenience methods ─────────────────────────────────────────

    pub fn get_status(&mut self) -> crate::AppResult<DaemonStatus> {
        let resp = self.request(Request::GetStatus)?;
        resp.status
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    resp.error.unwrap_or_else(|| "No status in response".into()),
                )
                .into()
            })
    }

    pub fn set_bands(&mut self, bands: &[EqBand]) -> crate::AppResult<()> {
        let resp = self.request(Request::SetBands {
            bands: bands.to_vec(),
        })?;
        check_ok(resp)
    }

    pub fn set_bypass(&mut self, bypass: bool) -> crate::AppResult<()> {
        let resp = self.request(Request::SetBypass { bypass })?;
        check_ok(resp)
    }

    pub fn connect_device(&mut self, node_id: u32) -> crate::AppResult<()> {
        let resp = self.request(Request::ConnectDevice { node_id })?;
        check_ok(resp)
    }

    pub fn disconnect_device(&mut self, node_id: u32) -> crate::AppResult<()> {
        let resp = self.request(Request::DisconnectDevice { node_id })?;
        check_ok(resp)
    }

    pub fn shutdown(&mut self) -> crate::AppResult<()> {
        let _resp = self.request(Request::Shutdown)?;
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn check_ok(resp: Response) -> crate::AppResult<()> {
    if resp.ok {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            resp.error.unwrap_or_else(|| "Unknown error".into()),
        )
        .into())
    }
}

fn socket_path() -> PathBuf {
    runtime_dir().join("eqtui.sock")
}

fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/tmp")
    }
}

/// Fork + exec the same binary with `daemon` as the first argument.
fn spawn_daemon() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => {
            warn!("Cannot determine own binary path — daemon auto-launch disabled");
            return;
        }
    };

    match Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            info!(pid = child.id(), "Spawned daemon");
        }
        Err(e) => {
            warn!(%e, "Failed to spawn daemon");
        }
    }
}
