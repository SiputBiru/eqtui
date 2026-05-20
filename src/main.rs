use std::io;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use pipewire::channel;
use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use eqtui::{
    AppResult,
    app::App,
    config::Config,
    event::EventHandler,
    handler,
    pipeline::Pipeline,
    pw,
    state::{PwCommand, PwEvent},
    tui::{self, Tui},
};
use ratatui::backend::CrosstermBackend;

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

    tracing::info!("Starting eqtui...");

    let config = Arc::new(Config::new(None));
    let pipeline = Arc::new(Pipeline::new(48000.0));

    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = channel::channel::<PwCommand>();

    tracing::info!("Spawning PW thread...");
    let pipeline_pw = pipeline.clone();
    let pw_handle = thread::spawn(move || {
        pw::run(to_tui, from_tui, pipeline_pw);
    });

    tracing::info!("Creating Backend...");
    let backend = CrosstermBackend::new(io::stdout());

    tracing::info!("Creating Terminal...");
    let terminal = ratatui::Terminal::new(backend)?;

    tracing::info!("Creating EventHandler...");
    let events = EventHandler::new();

    tracing::info!("Creating Tui struct...");
    let mut tui = Tui::new(terminal, events);

    tracing::info!("Calling tui.init()...");
    tui.init()?;
    tracing::info!("TUI initialized.");

    let mut app = App::new(config, pipeline);
    tracing::info!("App created. Entering main loop...");

    while app.running {
        while let Ok(event) = from_pw.try_recv() {
            app.handle_pw_event(event);
        }
        match tui.events.next()? {
            eqtui::event::Event::Tick => app.tick(),
            eqtui::event::Event::Key(key) => {
                if let Some(cmd) = handler::dispatch(key, &mut app)
                    && to_pw.send(cmd).is_err()
                {
                    tracing::error!("PipeWire thread disconnected; shutting down");
                    app.running = false;
                }
            }
            eqtui::event::Event::Resize(_, _) => {}
        }
        tui.draw(|frame| tui::render(&app, frame))?;
    }

    tui.exit()?;

    if to_pw.send(PwCommand::Terminate).is_err() {
        tracing::warn!("Pipewire thread already terminated before shutdown signal");
    }

    if let Err(panic) = pw_handle.join() {
        tracing::error!("PipeWire thread panicked");
        std::panic::resume_unwind(panic);
    }

    Ok(())
}
