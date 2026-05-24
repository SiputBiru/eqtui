// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Fire-and-forget CLI subcommands that connect to the daemon,
//! send a single request, and exit.

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::AppResult;
use crate::autoeq::parse_peq;
use crate::client::DaemonClient;

/// Stop the running daemon.
pub fn run_stop() -> AppResult<()> {
    let mut client = DaemonClient::connect()?;
    client.shutdown()?;
    println!("Daemon stopped.");
    Ok(())
}

/// Stop the running daemon and start a fresh one.
pub fn run_restart() -> AppResult<()> {
    let mut client = DaemonClient::connect()?;
    client.shutdown()?;
    // Wait for the old daemon to release the lock file.
    std::thread::sleep(Duration::from_millis(500));
    match std::env::current_exe() {
        Ok(exe) => {
            Command::new(exe)
                .arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        Err(e) => {
            eprintln!("Cannot determine binary path: {e}");
            std::process::exit(1);
        }
    }
    println!("Daemon restarted.");
    Ok(())
}

/// Load a PEQ preset file into the daemon.
pub fn run_load(args: &[String]) -> AppResult<()> {
    let path = args.get(2).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Usage: eqtui load <peq_file>",
        )
    })?;

    let preset = parse_peq(std::path::Path::new(path))?;
    let mut client = DaemonClient::connect()?;
    client.set_preamp(preset.preamp)?;
    client.set_bands(&preset.bands)?;

    println!(
        "Loaded: {} bands, preamp {:.1} dB",
        preset.bands.len(),
        preset.preamp
    );
    Ok(())
}
