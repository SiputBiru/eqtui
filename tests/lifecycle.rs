// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Integration tests for the daemon shutdown lifecycle.
//!
//! Spawns the real daemon binary against a tempdir `XDG_RUNTIME_DIR`, sends a
//! `Shutdown` request, and verifies the orderly teardown: the daemon exits
//! with status 0 and removes its socket.
//!
//! The daemon connects to the *real* `PipeWire` session via
//! `PIPEWIRE_RUNTIME_DIR` (passed through from the test process). If no
//! `PipeWire` daemon is reachable the spawned daemon exits during startup, so
//! these tests skip loudly instead of failing: CI runners typically have no
//! `PipeWire` service.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(5);

struct TestDaemon {
    child: Child,
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawns the daemon with an isolated `XDG_RUNTIME_DIR` (tempdir, 0700) while
/// letting it reach the ambient `PipeWire` session through `PIPEWIRE_RUNTIME_DIR`.
///
/// Returns `None` (with an explanation on stderr) if the daemon exits before
/// its socket appears: i.e. no `PipeWire` session available.
fn spawn_daemon() -> Option<TestDaemon> {
    let dir = tempfile::tempdir().expect("tempdir creation should succeed");
    // tempfile makes 0700 dirs, but enforce to be robust against umask changes:
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("setting tempdir mode should succeed");

    let socket = dir.path().join("eqtui").join("eqtui.sock");

    let mut command = Command::new(env!("CARGO_BIN_EXE_eqtui"));
    command
        .arg("daemon")
        .env("XDG_RUNTIME_DIR", dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The real PipeWire socket lives in the ambient runtime dir; keep the
    // daemon's PipeWire connection out of the isolated tempdir.
    if let Ok(ambient) = std::env::var("XDG_RUNTIME_DIR") {
        command.env("PIPEWIRE_RUNTIME_DIR", ambient);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP: could not spawn daemon binary: {e}");
            return None;
        }
    };

    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if socket.exists() {
            return Some(TestDaemon {
                child,
                socket,
                _dir: dir,
            });
        }
        // Fail fast if the daemon died (e.g. no PipeWire session).
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!(
                "SKIP: daemon exited during startup with {status} \
                 (no PipeWire session?): skipping integration test"
            );
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();
    eprintln!("SKIP: daemon socket did not appear within 5s: skipping integration test");
    None
}

/// True if the line carries a `PushEvent` (which `request()` must skip over).
fn is_push_event(line: &str) -> bool {
    matches!(
        serde_json::from_str::<Value>(line),
        Ok(v) if v.get("event").is_some()
    )
}

/// Sends one request and returns the *response* line, skipping any
/// interleaved push events (the daemon broadcasts ~15 PeakUpdate/s).
fn send_raw(socket: &Path, body: &str) -> String {
    let stream = UnixStream::connect(socket).expect("daemon socket should accept connections");
    stream
        .set_read_timeout(Some(TIMEOUT))
        .expect("setting read timeout should succeed");
    stream
        .set_write_timeout(Some(TIMEOUT))
        .expect("setting write timeout should succeed");
    let reader = BufReader::new(
        stream
            .try_clone()
            .expect("cloning socket for reading should succeed"),
    );
    let (mut stream, mut reader) = (stream, reader);
    stream
        .write_all(body.as_bytes())
        .expect("writing request should succeed");
    stream
        .write_all(b"\n")
        .expect("writing newline should succeed");
    stream.flush().expect("flushing request should succeed");

    let deadline = Instant::now() + TIMEOUT;
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a response"
        );
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .expect("reading response should succeed");
        assert_ne!(n, 0, "daemon closed the connection before responding");
        if !is_push_event(&line) {
            return line;
        }
    }
}

fn ok_of(line: &str) -> bool {
    let v: Value = serde_json::from_str(line).expect("response should be valid JSON");
    v["ok"].as_bool().expect("response should have an ok field")
}

#[test]
fn clean_shutdown_exits_zero_and_removes_socket() {
    let Some(mut d) = spawn_daemon() else { return };

    // Send a Shutdown request over the raw protocol.
    let line = send_raw(&d.socket, r#"{"cmd":"Shutdown"}"#);
    assert!(
        ok_of(&line),
        "Shutdown request must be accepted, got: {line}"
    );

    // The daemon exits within 2 s with status 0.
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = d.child.try_wait().expect("try_wait should succeed") {
            break status;
        }
        assert!(Instant::now() < deadline, "daemon did not exit within 2 s");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "daemon must exit 0, got: {status}");

    // Socket removed by the daemon's shutdown path.
    assert!(
        !d.socket.exists(),
        "socket file must be removed on clean exit"
    );
}
