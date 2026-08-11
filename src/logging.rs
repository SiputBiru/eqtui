// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Logging initialisation — routes tracing output based on the run mode.
//!
//! - **Daemon / CLI** (`daemon`, `stop`, `restart`, `load`): \
//!   Writes to **stderr** with ANSI colours.  When the daemon is spawned by the
//!   TUI (`spawn_daemon` in `client.rs`) its stderr is already nulled, so no
//!   stray output reaches the terminal.  When run manually or via systemd the
//!   logs go wherever stderr points (terminal / journald).
//!
//! - **TUI** (`attach`): \
//!   Writes to `<data-dir>/eqtui/eqtui-tui.log` (append).  The alternate screen
//!   must not be polluted by stray stderr output, so we use a file instead.

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;

/// Single-generation log rotation cap: 5 MiB per generation, so at most ~2x
/// that on disk, ever.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Single-generation rotation: if the log exceeds the cap, the current file
/// becomes `<name>.old` (overwriting any previous generation) and a fresh log
/// starts on next append.
fn rotate_if_oversized(path: &std::path::Path) {
    if let Ok(md) = std::fs::metadata(path)
        && md.len() > MAX_LOG_BYTES
    {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
}

/// Initialise `tracing` once at process start.
///
/// Must be called **before** any `tracing::info!` / `warn!` / etc. macros.
///
/// Both modes respect the `RUST_LOG` environment variable (falls back to
/// `eqtui=info` when unset).
pub fn init(mode: &str) -> crate::AppResult<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("eqtui=info"));

    let is_tui = matches!(mode, "attach" | "");

    let subscriber = tracing_subscriber::registry().with(filter);

    if is_tui {
        // TUI: log to a file so stderr output doesn't corrupt
        // the alternate screen.
        // WHY no /tmp fallback: a shared, world-readable directory contradicts
        // the same security rationale applied to the IPC socket. No data
        // dir = fail with a clear message.
        let log_dir = dirs::data_dir()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "cannot determine user data directory (XDG_DATA_HOME)",
                )
            })?
            .join("eqtui");
        std::fs::create_dir_all(&log_dir)?;

        let log_path = log_dir.join("eqtui-tui.log");
        // Single-generation rotation: at most ~10 MiB on disk, ever.
        rotate_if_oversized(&log_path);

        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        subscriber
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(log_file)),
            )
            .with(tracing_error::ErrorLayer::default())
            .init();
    } else {
        // Daemon / CLI: stderr (terminal or journald).
        subscriber
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(tracing_error::ErrorLayer::default())
            .init();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_log_is_rotated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqtui-tui.log");
        std::fs::write(&path, vec![b'x'; (MAX_LOG_BYTES as usize) + 1]).unwrap();

        rotate_if_oversized(&path);

        let old = path.with_extension("log.old");
        assert!(old.exists(), "oversized log must be rotated to .old");
        assert!(!path.exists(), "original renamed away");
    }

    #[test]
    fn small_log_is_not_rotated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqtui-tui.log");
        std::fs::write(&path, vec![b'x'; 1024]).unwrap();

        rotate_if_oversized(&path);

        let old = path.with_extension("log.old");
        assert!(path.exists(), "small log must stay in place");
        assert!(!old.exists(), "no .old should be created for a small log");
    }
}
