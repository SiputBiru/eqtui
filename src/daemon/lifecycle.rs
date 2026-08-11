// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Daemon lifecycle: startup (lock, threads, accept loop) and the orderly
//! shutdown sequence.

use std::fs;
use std::io::{Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, atomic::Ordering, mpsc};
use std::thread;
use std::time::Duration;

use pipewire::channel;
use tracing::{debug, error, info};

use crate::paths::{lock_path, socket_path, validate_runtime_dir};
use crate::pipeline::{Pipeline, SAMPLE_RATE};
use crate::protocol::PushEvent;
use crate::state::{PwCommand, PwEvent};
use crate::{AppResult, pw};

use super::auth::peer_is_self;
use super::state::{ClientHandle, DaemonState};
use super::transport::handle_client;

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

/// Start the daemon: set up the runtime dir, lock, `PipeWire` threads, and the
/// accept loop, then run until shutdown and tear down in dependency order.
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

#[cfg(test)]
mod tests {
    use super::*;

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
