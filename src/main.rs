use std::io::{self, Write};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use pipewire::channel;

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
    let mut log = std::fs::File::create("/tmp/eqtui.log")
        .expect("failed to create /tmp/eqtui.log — check /tmp is writable");
    writeln!(log, "Starting eqtui...")
        .expect("log write failed");

    color_eyre::install()?;

    let config = Arc::new(Config::new(None));
    let pipeline = Arc::new(Pipeline::new(48000.0));

    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = channel::channel::<PwCommand>();

    writeln!(log, "Spawning PW thread...")
        .expect("log write failed");
    let pipeline_pw = pipeline.clone();
    let pw_handle = thread::spawn(move || {
        pw::run(to_tui, from_tui, pipeline_pw);
    });

    writeln!(log, "Creating Backend...")
        .expect("log write failed");
    let backend = CrosstermBackend::new(io::stdout());
    writeln!(log, "Creating Terminal...")
        .expect("log write failed");
    let terminal = ratatui::Terminal::new(backend)?;
    writeln!(log, "Creating EventHandler...")
        .expect("log write failed");
    let events = EventHandler::new();
    writeln!(log, "Creating Tui struct...")
        .expect("log write failed");
    let mut tui = Tui::new(terminal, events);

    writeln!(log, "Calling tui.init()...")
        .expect("log write failed");
    tui.init()?;
    writeln!(log, "TUI initialized.")
        .expect("log write failed");

    let mut app = App::new(config, pipeline);
    writeln!(log, "App created. Entering main loop...")
        .expect("log write failed");

    while app.running {
        while let Ok(event) = from_pw.try_recv() {
            app.handle_pw_event(event);
        }

        match tui.events.next()? {
            eqtui::event::Event::Tick => app.tick(),
            eqtui::event::Event::Key(key) => {
                if let Some(cmd) = handler::dispatch(key, &mut app) {
                    let _ = to_pw.send(cmd);
                }
            }
            eqtui::event::Event::Resize(_, _) => {}
        }

        tui.draw(|frame| tui::render(&app, frame))?;
    }

    tui.exit()?;

    let _ = to_pw.send(PwCommand::Terminate);
    pw_handle.join().ok();

    Ok(())
}
