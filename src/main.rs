use std::io;
use std::sync::Arc;

use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use eqtui::{
    AppResult,
    app::App,
    client::DaemonClient,
    config::Config,
    daemon,
    event::EventHandler,
    handler,
    tui::{self, Tui},
};
use ratatui::backend::CrosstermBackend;

fn main() -> AppResult<()> {
    // ── Logging ────────────────────────────────────────────────────
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

    // ── Subcommand dispatch ────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("attach");

    match mode {
        "daemon" => daemon::run(),
        "stop" => run_cli_stop(),
        "attach" | _ => run_tui_attach(),
    }
}

/// Send a shutdown request to the running daemon.
fn run_cli_stop() -> AppResult<()> {
    let mut client = DaemonClient::connect()?;
    client.shutdown()?;
    println!("Daemon stopped.");
    Ok(())
}

/// Connect to the daemon (auto-launch if needed) and start the TUI.
fn run_tui_attach() -> AppResult<()> {
    tracing::info!("Connecting to daemon...");
    let client = DaemonClient::connect()?;

    let config = Arc::new(Config::new(None));

    tracing::info!("Creating App and pulling initial state...");
    let mut app = App::new(config, client);

    // Pull the daemon's current state so the TUI starts with real data.
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
        // Drain push events from daemon (peaks, node lists, etc.).
        if let Err(e) = app.drain_events() {
            tracing::error!(%e, "Daemon connection lost");
            app.running = false;
            break;
        }

        // Block on next UI event.
        match tui.events.next()? {
            eqtui::event::Event::Tick => app.tick(),
            eqtui::event::Event::Key(key) => {
                handler::dispatch(key, &mut app);
            }
            eqtui::event::Event::Resize(_, _) => {}
        }

        tui.draw(|frame| tui::render(&app, frame))?;
    }

    tui.exit()?;
    tracing::info!("TUI exited — daemon keeps running");

    Ok(())
}
