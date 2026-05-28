// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::collections::VecDeque;
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
    /// Push events that arrived during a synchronous `request()` call
    /// are buffered here and drained by `try_read_event()`.
    pending_events: VecDeque<PushEvent>,
}

impl DaemonClient {
    /// Connect to the daemon, auto-launching if none is running.
    pub fn connect() -> crate::AppResult<Self> {
        let path = socket_path()?;

        if let Ok(client) = Self::try_connect(&path) {
            info!("Connected to existing daemon");
            return Ok(client);
        }

        info!("No daemon found — auto-launching");
        let daemon_pid = spawn_daemon();

        let timeout_ms = std::env::var("EQTUI_DAEMON_START_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3000);
        let attempts = (timeout_ms / 100).max(1);
        for _ in 0..attempts {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(client) = Self::try_connect(&path) {
                info!("Connected to auto-launched daemon");
                return Ok(client);
            }
        }

        if let Some(pid) = daemon_pid {
            warn!(pid, "Daemon start timed out — sending SIGTERM to orphan");
            #[allow(clippy::cast_possible_wrap)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("Daemon failed to start within {timeout_ms}ms"),
        )
        .into())
    }

    fn try_connect(path: &PathBuf) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path)?;

        // Set 5s timeouts to prevent TUI/CLI hangs if the daemon is unresponsive.
        let timeout = Some(Duration::from_secs(5));
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;

        let reader = BufReader::new(stream.try_clone().map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("Failed to clone daemon socket for reading: {e}"),
            )
        })?);
        Ok(Self {
            stream,
            reader,
            pending_events: VecDeque::new(),
        })
    }

    pub fn request(&mut self, req: Request) -> crate::AppResult<Response> {
        let json = serde_json::to_string(&req)?;
        self.stream.write_all(json.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;

        // Loop until a Response arrives.  Push events that arrive before
        // the response are buffered and returned by try_read_event().
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line)?;

            // Try Response first — it has { ok, error, status }.
            if let Ok(resp) = serde_json::from_str::<Response>(line.trim()) {
                return Ok(resp);
            }

            // PushEvents use #[serde(tag = "event")] → { "event": "...", ... }.
            if let Ok(event) = serde_json::from_str::<PushEvent>(line.trim()) {
                self.pending_events.push_back(event);
                continue;
            }

            // Neither variant matched — likely a protocol error or corrupted data.
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unexpected data from daemon: {}", line.trim()),
            )
            .into());
        }
    }

    /// Returns `None` when no push events are available.
    pub fn try_read_event(&mut self) -> std::io::Result<Option<PushEvent>> {
        // Drain events that were buffered during a synchronous request()
        // before hitting the socket.  This ensures they are processed in
        // order on the next drain_events() cycle.
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }

        self.reader.get_mut().set_nonblocking(true)?;
        let mut line = String::new();
        let result = match self.reader.read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => match serde_json::from_str::<PushEvent>(line.trim()) {
                Ok(event) => Ok(Some(event)),
                // A stray Response or other non-PushEvent data arrived.
                // This shouldn't happen in normal operation (request()
                // always consumes the expected response), but if it does,
                // silently discard it rather than crashing the TUI.
                Err(_) => Ok(None),
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        };
        self.reader.get_mut().set_nonblocking(false)?;
        result
    }

    pub fn get_status(&mut self) -> crate::AppResult<DaemonStatus> {
        let resp = self.request(Request::GetStatus)?;
        resp.status.ok_or_else(|| {
            std::io::Error::other(resp.error.unwrap_or_else(|| "No status in response".into()))
                .into()
        })
    }

    pub fn set_bands(&mut self, bands: &[EqBand]) -> crate::AppResult<()> {
        let resp = self.request(Request::SetBands {
            bands: bands.to_vec(),
        })?;
        check_ok(resp)
    }

    pub fn set_preamp(&mut self, gain: f32) -> crate::AppResult<()> {
        let resp = self.request(Request::SetPreamp { gain })?;
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
        let _ = self.request(Request::Shutdown)?;
        Ok(())
    }
}

fn check_ok(resp: Response) -> crate::AppResult<()> {
    if resp.ok {
        Ok(())
    } else {
        Err(std::io::Error::other(resp.error.unwrap_or_else(|| "Unknown error".into())).into())
    }
}

fn socket_path() -> crate::AppResult<PathBuf> {
    Ok(runtime_dir()?.join("eqtui.sock"))
}

/// Returns the XDG runtime directory for the current user.
///
/// This directory is used for the Unix socket.
/// Strict requirement for `XDG_RUNTIME_DIR` to be set for security;
/// falling back to /tmp would allow other local users to intercept
/// or control the daemon.
fn runtime_dir() -> crate::AppResult<PathBuf> {
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

fn spawn_daemon() -> Option<u32> {
    let Ok(exe) = std::env::current_exe() else {
        warn!("Cannot determine own binary path — daemon auto-launch disabled");
        return None;
    };

    match Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            let pid = child.id();
            info!(pid, "Spawned daemon");
            Some(pid)
        }
        Err(e) => {
            warn!(%e, "Failed to spawn daemon");
            None
        }
    }
}
