use std::sync::mpsc;
use std::sync::Arc;

use eqtui::{
    app::App,
    config::Config,
    event::EventHandler,
    handler,
    pipeline::Pipeline,
    state::{PwCommand, PwEvent},
    tui::Tui,
    AppResult,
};
use ratatui::backend::CrosstermBackend;

fn main() -> AppResult<()> {
    color_eyre::install()?;

    let config = Arc::new(Config::new(None));
    let pipeline = Arc::new(Pipeline::new(48000.0));

    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = pipewire::channel::channel::<PwCommand>();

    let pipeline_pw = pipeline.clone();
    let pw_handle = std::thread::spawn(move || {
        eqtui::pw::run(to_tui, from_tui, pipeline_pw);
    });

    let backend = CrosstermBackend::new(std::io::stdout());
    let terminal = ratatui::Terminal::new(backend)?;
    let events = EventHandler::new();
    let mut tui = Tui::new(terminal, events);

    tui.init()?;

    let mut app = App::new(config, pipeline);

    while app.running {
        while let Ok(event) = from_pw.try_recv() {
            app.handle_pw_event(event);
        }

        match tui.events.next()? {
            eqtui::event::Event::Tick => app.tick(),
            eqtui::event::Event::Key(key) => handler::dispatch(key, &mut app),
            eqtui::event::Event::Resize(_, _) => {}
        }

        tui.draw(|frame| eqtui::tui::render(&app, frame))?;
    }

    tui.exit()?;

    let _ = to_pw.send(PwCommand::Terminate);
    pw_handle.join().ok();

    Ok(())
}
