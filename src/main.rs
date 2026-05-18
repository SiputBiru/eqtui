use std::sync::Arc;
use std::sync::mpsc;

use eqtui::{
    AppResult,
    app::App,
    config::Config,
    event::EventHandler,
    handler,
    pipeline::Pipeline,
    state::{PwCommand, PwEvent},
    tui::Tui,
};
use ratatui::backend::CrosstermBackend;

fn main() -> AppResult<()> {
    use std::io::Write;
    let mut log = std::fs::File::create("/tmp/eqtui.log").unwrap();
    writeln!(log, "Starting eqtui...").unwrap();

    color_eyre::install()?;

    let config = Arc::new(Config::new(None));
    let pipeline = Arc::new(Pipeline::new(48000.0));

    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = pipewire::channel::channel::<PwCommand>();

    writeln!(log, "Spawning PW thread...").unwrap();
    let pipeline_pw = pipeline.clone();
    let pw_handle = std::thread::spawn(move || {
        eqtui::pw::run(to_tui, from_tui, pipeline_pw);
    });

    writeln!(log, "Creating Backend...").unwrap();
    let backend = CrosstermBackend::new(std::io::stdout());
    writeln!(log, "Creating Terminal...").unwrap();
    let terminal = ratatui::Terminal::new(backend)?;
    writeln!(log, "Creating EventHandler...").unwrap();
    let events = EventHandler::new();
    writeln!(log, "Creating Tui struct...").unwrap();
    let mut tui = Tui::new(terminal, events);

    writeln!(log, "Calling tui.init()...").unwrap();
    tui.init()?;
    writeln!(log, "TUI initialized.").unwrap();

    let mut app = App::new(config, pipeline);
    writeln!(log, "App created. Entering main loop...").unwrap();

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

        tui.draw(|frame| eqtui::tui::render(&app, frame))?;
    }

    tui.exit()?;

    let _ = to_pw.send(PwCommand::Terminate);
    pw_handle.join().ok();

    Ok(())
}
