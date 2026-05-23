// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;

use ratatui::backend::CrosstermBackend;
use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use eqtui::{
    AppResult,
    app::App,
    autoeq::parse_peq,
    client::DaemonClient,
    config::Config,
    daemon,
    event::EventHandler,
    handler,
    tui::{self, Tui},
};

fn main() -> AppResult<()> {
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("eqtui");

    std::fs::create_dir_all(&log_dir)?;
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("eqtui.log"))?;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(log_file)),
        )
        .with(ErrorLayer::default())
        .init();

    color_eyre::install()?;

    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("attach", std::string::String::as_str);

    match mode {
        "daemon" => {
            let stdout = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join("eqtui.out"))?;

            let stderr = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join("eqtui.err"))?;

            let lock_path = daemon::lock_path()?;
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(&lock_path)?;

            // Acquire exclusive advisory lock (kernel-level).
            // This is robust against crashes and prevents multiple instances
            // from running simultaneously even if a stale PID file exists.
            if let Err(e) = lock_file.try_lock() {
                eprintln!("Daemon already running. Use `eqtui stop` to stop it first.");
                tracing::error!(%e, "Failed to acquire daemon lock; another instance might be running");
                std::process::exit(1);
            }

            if let Err(e) = daemon::init(stdout, stderr, lock_file.try_clone()?) {
                eprintln!("Error starting daemon: {e}");
                std::process::exit(1);
            }
            tracing::info!("Daemonized successfully");
            daemon::run(lock_file)
        }
        "stop" => run_cli_stop(),
        "load" => run_cli_load(&args),
        _ => run_tui_attach(),
    }
}

fn run_cli_stop() -> AppResult<()> {
    let mut client = DaemonClient::connect()?;
    client.shutdown()?;
    println!("Daemon stopped.");
    Ok(())
}

fn run_cli_load(args: &[String]) -> AppResult<()> {
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

fn run_tui_attach() -> AppResult<()> {
    tracing::info!("Connecting to daemon...");
    let client = DaemonClient::connect()?;

    let config = Arc::new(Config::new(None));
    let mut app = App::new(config, client);

    if let Err(e) = app.full_sync() {
        tracing::warn!(%e, "Initial full_sync failed — starting with defaults");
    }

    let backend = CrosstermBackend::new(io::stdout());
    let terminal = ratatui::Terminal::new(backend)?;
    let events = EventHandler::new();
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    tracing::info!("Entering TUI main loop");

    while app.running {
        if let Err(e) = app.drain_events() {
            tracing::error!(%e, "Daemon connection lost");
            app.running = false;
            break;
        }

        match tui.events.next()? {
            eqtui::event::Event::Tick => app.tick(),
            eqtui::event::Event::Key(key) => handler::dispatch(key, &mut app),
            eqtui::event::Event::Resize(_, _) => {}
        }

        tui.draw(|frame| tui::render(&app, frame))?;
    }

    tui.exit()?;
    tracing::info!("TUI exited — daemon keeps running");
    Ok(())
}
