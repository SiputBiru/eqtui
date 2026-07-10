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

use std::path::PathBuf;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::*;

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
        let log_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("eqtui");
        std::fs::create_dir_all(&log_dir)?;
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("eqtui-tui.log"))?;

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
