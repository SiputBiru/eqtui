// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Integration tests for IPC request validation.
//!
//! These spawn the real daemon binary against a tempdir `XDG_RUNTIME_DIR`
//! and exercise the negative paths added by the validation work:
//!   - oversized request lines → error response + disconnect, daemon alive
//!   - ±inf preamp (JSON overflow) → rejected
//!   - too many bands → rejected, state unchanged
//!
//! The daemon connects to the *real* `PipeWire` session via `PIPEWIRE_RUNTIME_DIR`
//! (passed through from the test process). If no `PipeWire` daemon is reachable
//! the spawned daemon exits during startup, so these tests skip loudly instead
//! of failing — CI runners typically have no `PipeWire` service.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Bound used by the daemon's read loop (must match `MAX_REQUEST_LINE`).
const MAX_REQUEST_LINE: usize = 64 * 1024;

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
/// its socket appears — i.e. no `PipeWire` session available.
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
                 (no PipeWire session?) — skipping integration test"
            );
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();
    eprintln!("SKIP: daemon socket did not appear within 5s — skipping integration test");
    None
}

/// Opens a raw connection with sane timeouts and returns a (write, read) pair.
fn connect(socket: &Path) -> (UnixStream, BufReader<UnixStream>) {
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
    (stream, reader)
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
    let (mut stream, mut reader) = connect(socket);
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

/// Reads the status payload's `bands` length (defaults to 0 if absent).
fn band_count(socket: &Path) -> usize {
    let line = send_raw(socket, r#"{"cmd":"GetStatus"}"#);
    let v: Value = serde_json::from_str(&line).expect("status response should be valid JSON");
    v["status"]["bands"].as_array().map_or(0, Vec::len)
}

#[test]
fn oversized_line_is_rejected_and_connection_closed() {
    let Some(d) = spawn_daemon() else { return };

    let (mut stream, mut reader) = connect(&d.socket);
    // 128 KiB of spaces + newline — beyond the 64 KiB limit.
    let huge = format!("{} \n", " ".repeat(MAX_REQUEST_LINE * 2));
    stream
        .write_all(huge.as_bytes())
        .expect("writing oversized line should succeed");
    stream
        .flush()
        .expect("flushing oversized line should succeed");

    // The daemon answers with an error response (skipping event frames)...
    let deadline = Instant::now() + TIMEOUT;
    let error_line = loop {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the rejection"
        );
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .expect("reading rejection response should succeed");
        assert_ne!(n, 0, "daemon closed before sending the rejection");
        if is_push_event(&line) {
            continue;
        }
        break line;
    };
    assert!(!ok_of(&error_line), "oversized request must be rejected");
    assert!(
        error_line.contains("request line exceeds"),
        "error should name the limit, got: {error_line}"
    );

    // ...then closes the connection (newline framing cannot be resynced).
    // Depending on kernel timing the close shows up as clean EOF (Ok(0)) or
    // ConnectionReset (104) — either way the connection is gone.
    let mut rest = String::new();
    let n = reader.read_to_string(&mut rest);
    match n {
        Ok(0) => {} // clean orderly close
        Err(e) => assert_eq!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset,
            "daemon must close the connection after an oversized line, got: {e}"
        ),
        Ok(n) => panic!("expected no data after the rejection, got {n} bytes: {rest}"),
    }

    // A fresh client still works — the daemon is alive.
    assert!(ok_of(&send_raw(&d.socket, r#"{"cmd":"GetStatus"}"#)));
}

#[test]
fn non_finite_preamp_is_rejected() {
    let Some(d) = spawn_daemon() else { return };

    // JSON has no NaN literal; 1e999 overflows to ±inf in serde_json, which
    // the daemon's range check must reject.
    let line = send_raw(&d.socket, r#"{"cmd":"SetPreamp","gain":1e999}"#);
    assert!(
        !ok_of(&line),
        "non-finite preamp must be rejected, got: {line}"
    );
    assert!(
        line.contains("out of range"),
        "error should name the range, got: {line}"
    );
}

#[test]
fn too_many_bands_is_rejected_and_state_unchanged() {
    let Some(d) = spawn_daemon() else { return };

    // Establish a baseline: 0 bands.
    assert_eq!(band_count(&d.socket), 0);

    // 32 bands (> MAX_BANDS = 31).
    let mut body = String::from(r#"{"cmd":"SetBands","bands":["#);
    for i in 0..32 {
        if i > 0 {
            body.push(',');
        }
        body.push_str(r#"{"frequency":1000.0,"gain":0.0,"q":1.0,"filter_type":"Peak"}"#);
    }
    body.push(']');
    body.push('}');

    let line = send_raw(&d.socket, &body);
    assert!(!ok_of(&line), "32 bands must be rejected, got: {line}");
    assert!(
        line.contains("too many bands"),
        "error should name the band limit, got: {line}"
    );

    // State was not touched.
    assert_eq!(
        band_count(&d.socket),
        0,
        "rejected SetBands must not mutate state"
    );
}

#[test]
fn valid_request_still_roundtrips() {
    let Some(d) = spawn_daemon() else { return };

    // Sanity: a legitimate single-band request is accepted.
    let line = send_raw(
        &d.socket,
        r#"{"cmd":"SetBands","bands":[{"frequency":1000.0,"gain":3.0,"q":1.0,"filter_type":"Peak"}]}"#,
    );
    assert!(ok_of(&line), "valid SetBands must be accepted, got: {line}");
    assert_eq!(band_count(&d.socket), 1);

    // And a preamp within ±40 dB is accepted.
    let line = send_raw(&d.socket, r#"{"cmd":"SetPreamp","gain":-6.0}"#);
    assert!(
        ok_of(&line),
        "valid SetPreamp must be accepted, got: {line}"
    );
}
