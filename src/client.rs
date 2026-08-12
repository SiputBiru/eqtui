// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::{info, warn};

use crate::paths::socket_path;
use crate::protocol::{DaemonStatus, PushEvent, Request, Response};
use crate::state::EqBand;

/// Classification of a failed client↔daemon exchange. Returned as the
/// concrete error so callers can distinguish "reconnect" from "keep polling".
#[derive(Debug)]
pub enum ClientError {
    /// Daemon closed the connection (clean shutdown, crash, or teardown).
    Disconnected,
    /// The 5 s read/write deadline elapsed — daemon alive but unresponsive.
    Timeout,
    /// A line arrived that is neither `Response` nor `PushEvent`. Carries the
    /// offending text (truncated) for debugging.
    Malformed(String),
    Io(std::io::Error),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "daemon closed the connection"),
            Self::Timeout => write!(f, "daemon did not respond in time"),
            Self::Malformed(line) => write!(f, "malformed data from daemon: {line}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => Self::Timeout,
            std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe => Self::Disconnected,
            _ => Self::Io(e),
        }
    }
}

/// Cuts hostile/garbage megabyte-lines down for logs and notifications.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

pub struct DaemonClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    /// Push events that arrived during a synchronous `request()` call
    /// are buffered here and drained by `try_read_event()`.
    pending_events: VecDeque<PushEvent>,
    /// An auto-launched daemon, kept so it can be reaped via `try_wait()`
    /// (no zombie) and so startup failure is detected fast. `None` when
    /// connected to a pre-existing daemon.
    daemon_child: Option<std::process::Child>,
}

impl DaemonClient {
    /// Connect to the daemon, auto-launching if none is running.
    pub fn connect() -> crate::AppResult<Self> {
        Self::connect_with_exe(&socket_path()?, None, &[])
    }

    /// Full connect logic with an explicit socket path, optional daemon
    /// executable override, and extra env vars for the auto-launched child.
    ///
    /// Exists so tests can drive the auto-launch / fail-fast paths without
    /// mutating the process environment (which is `unsafe` in edition 2024
    /// and races parallel tests). Production callers pass `None` and `&[]` —
    /// `exe = None` resolves to `current_exe()`.
    fn connect_with_exe(
        path: &Path,
        exe: Option<&Path>,
        spawn_env: &[(&str, &str)],
    ) -> crate::AppResult<Self> {
        if let Ok(client) = Self::try_connect(path) {
            info!("Connected to existing daemon");
            return Ok(client);
        }

        info!("No daemon found — auto-launching");
        let mut daemon_child = spawn_daemon(exe, spawn_env);

        let timeout_ms = std::env::var("EQTUI_DAEMON_START_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3000);
        let attempts = (timeout_ms / 100).max(1);
        for _ in 0..attempts {
            std::thread::sleep(Duration::from_millis(100));

            // Fail fast if the daemon died during startup (insecure runtime
            // dir, lock conflict, ...). No zombie: try_wait reaps.
            if let Some(child) = &mut daemon_child
                && let Some(status) = child.try_wait().ok().flatten()
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("daemon exited immediately with {status}"),
                )
                .into());
            }

            if let Ok(mut client) = Self::try_connect(path) {
                info!("Connected to auto-launched daemon");
                client.daemon_child = daemon_child.take(); // keep for reaping
                return Ok(client);
            }
        }

        if let Some(mut child) = daemon_child {
            warn!(pid = child.id(), "Daemon start timed out — killing");
            let _ = child.kill(); // SIGKILL via the handle — NO pid-reuse race
            let _ = child.wait(); // reap — no zombie
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("Daemon failed to start within {timeout_ms}ms"),
        )
        .into())
    }

    fn try_connect(path: &Path) -> std::io::Result<Self> {
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
            daemon_child: None,
        })
    }

    pub fn request(&mut self, req: Request) -> Result<Response, ClientError> {
        let json = serde_json::to_string(&req).map_err(|e| {
            ClientError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        self.stream.write_all(json.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;

        // Loop until a Response arrives.  Push events that arrive before
        // the response are buffered and returned by try_read_event().
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line)?; // From<io::Error> classifies
            if n == 0 {
                return Err(ClientError::Disconnected); // EOF — was "Unexpected data"
            }
            let trimmed = line.trim();

            // Try Response first — it has { ok, error, status }.
            if let Ok(resp) = serde_json::from_str::<Response>(trimmed) {
                return Ok(resp);
            }

            // PushEvents use #[serde(tag = "event")] → { "event": "...", ... }.
            if let Ok(event) = serde_json::from_str::<PushEvent>(trimmed) {
                self.pending_events.push_back(event);
                continue;
            }

            // Neither variant matched — likely a protocol error or corrupted data.
            return Err(ClientError::Malformed(truncate(trimmed, 200)));
        }
    }

    /// Returns `None` when no push events are available.
    pub fn try_read_event(&mut self) -> Result<Option<PushEvent>, ClientError> {
        // Reap an auto-launched daemon that has exited (cheap, non-blocking) —
        // avoids a <defunct> child lingering until the TUI exits.
        if let Some(child) = &mut self.daemon_child
            && let Ok(Some(_status)) = child.try_wait()
        {
            self.daemon_child = None;
        }

        // Drain events that were buffered during a synchronous request()
        // before hitting the socket.  This ensures they are processed in
        // order on the next drain_events() cycle.
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }

        self.reader.get_mut().set_nonblocking(true)?;
        let mut line = String::new();
        let result = match self.reader.read_line(&mut line) {
            Ok(0) => Err(ClientError::Disconnected), // EOF — daemon hung up
            Ok(_) => match serde_json::from_str::<PushEvent>(line.trim()) {
                Ok(event) => Ok(Some(event)),
                // A stray Response or other non-PushEvent data arrived.
                // This shouldn't happen in normal operation (request()
                // always consumes the expected response), but if it does,
                // name it rather than silently dropping it.
                Err(_) => Err(ClientError::Malformed(truncate(line.trim(), 200))),
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
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

    /// Test-only constructor over an arbitrary stream (e.g. one end of a
    /// `UnixStream::pair()`) so the protocol code is testable without a
    /// daemon. Same timeouts and reader setup as `try_connect`.
    #[cfg(test)]
    pub(crate) fn from_stream(stream: UnixStream) -> std::io::Result<Self> {
        let timeout = Some(Duration::from_secs(5));
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            pending_events: VecDeque::new(),
            daemon_child: None,
        })
    }
}

fn check_ok(resp: Response) -> crate::AppResult<()> {
    if resp.ok {
        Ok(())
    } else {
        Err(std::io::Error::other(resp.error.unwrap_or_else(|| "Unknown error".into())).into())
    }
}

fn spawn_daemon(exe: Option<&Path>, spawn_env: &[(&str, &str)]) -> Option<std::process::Child> {
    let exe = if let Some(e) = exe {
        e.to_path_buf()
    } else if let Ok(e) = std::env::current_exe() {
        e
    } else {
        warn!("Cannot determine own binary path — daemon auto-launch disabled");
        return None;
    };

    match Command::new(exe)
        .arg("daemon")
        .envs(spawn_env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            info!(pid = child.id(), "Spawned daemon");
            Some(child)
        }
        Err(e) => {
            warn!(%e, "Failed to spawn daemon");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fail-fast path: when the spawned daemon dies during startup,
    /// `connect_with_exe` must return quickly naming the exit status — not
    /// after the full blind timeout.
    ///
    /// Uses `/bin/true` as a stand-in daemon: it exits instantly with status
    /// 0. This avoids relying on `current_exe()` (which under a lib unit test
    /// points at the test harness, not the real binary) and needs no
    /// `PipeWire` session — so it runs deterministically in CI.
    #[test]
    fn connect_fails_fast_when_daemon_exits_immediately() {
        let dir = tempfile::tempdir().unwrap();
        // No daemon will ever bind here; the connect must fail via try_wait.
        let socket = dir.path().join("eqtui").join("eqtui.sock");

        let start = std::time::Instant::now();
        let Err(err) = DaemonClient::connect_with_exe(&socket, Some(Path::new("/bin/true")), &[])
        else {
            panic!("connect must fail when the daemon dies on startup");
        };

        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "fail-fast must not wait the full blind timeout"
        );
        assert!(
            err.to_string().contains("exited immediately"),
            "error should name the early exit, got: {err}"
        );
    }

    /// EOF in `request()` is a disconnect, not "unexpected data".
    #[test]
    fn eof_in_request_is_disconnected() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut client = DaemonClient::from_stream(a).unwrap();
        drop(b); // daemon hung up
        let err = client.request(Request::GetStatus).unwrap_err();
        assert!(matches!(err, ClientError::Disconnected));
    }

    /// EOF in `try_read_event()` is a disconnect, not "no event".
    #[test]
    fn eof_in_try_read_event_is_disconnected() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut client = DaemonClient::from_stream(a).unwrap();
        drop(b);
        let err = client.try_read_event().unwrap_err();
        assert!(matches!(err, ClientError::Disconnected));
    }

    /// Malformed frames are named, not silently dropped.
    #[test]
    fn malformed_frame_is_named_not_silent() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut client = DaemonClient::from_stream(a).unwrap();
        use std::io::Write;
        (&b).write_all(b"this is not json\n").unwrap();
        let err = client.request(Request::GetStatus).unwrap_err();
        assert!(matches!(err, ClientError::Malformed(_)));
    }

    /// Push events interleaved with a response are buffered, not lost.
    #[test]
    fn events_interleaved_with_response_are_buffered() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut client = DaemonClient::from_stream(a).unwrap();
        use std::io::Write;
        writeln!(
            &b,
            "{}",
            serde_json::json!({"event":"PeakUpdate","l":0.5,"r":0.5})
        )
        .unwrap();
        writeln!(&b, "{}", serde_json::json!({"ok":true})).unwrap();
        assert!(client.request(Request::GetStatus).unwrap().ok);
        // the interleaved event surfaces on the next drain:
        assert!(matches!(
            client.try_read_event().unwrap(),
            Some(PushEvent::PeakUpdate { .. })
        ));
    }

    /// A mute peer is a timeout, not a disconnect.
    #[test]
    fn silence_is_timeout_not_disconnect() {
        let (a, _b) = UnixStream::pair().unwrap(); // peer open but mute
        let mut client = DaemonClient::from_stream(a).unwrap();
        client
            .stream
            .set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .unwrap();
        let err = client.request(Request::GetStatus).unwrap_err();
        assert!(matches!(err, ClientError::Timeout));
    }
}
