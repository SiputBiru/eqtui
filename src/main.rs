// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use tracing_subscriber::prelude::*;

use eqtui::AppResult;

fn main() -> AppResult<()> {
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("eqtui");
    std::fs::create_dir_all(&log_dir)?;
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("attach", std::string::String::as_str);
    let log_file = if mode == "daemon" {
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(log_dir.join("eqtui.log"))?
    } else {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("eqtui.log"))?
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(log_file)),
        )
        .with(tracing_error::ErrorLayer::default())
        .init();
    color_eyre::install()?;
    eqtui::cli::dispatch(mode, &args, &log_dir)
}
