mod pw;
mod pw_filter_ffi;
mod state;
mod tui;

use std::sync::mpsc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode};
use state::{AppState, PwCommand, PwEvent};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = pipewire::channel::channel::<PwCommand>();

    let pw_handle = std::thread::spawn(move || {
        pw::run(to_tui, from_tui);
    });

    let result = ratatui::run(|terminal| {
        let mut state = AppState::new();

        loop {
            while let Ok(event) = from_pw.try_recv() {
                state.handle_event(event);
            }

            terminal.draw(|frame| tui::render(frame, &state))?;

            if crossterm::event::poll(Duration::from_millis(16))? {
                match crossterm::event::read()? {
                    Event::Key(key) => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                        KeyCode::Up | KeyCode::Char('k') => state.select_prev(),
                        _ => {}
                    },
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
        }

        Ok(())
    });

    let _ = to_pw.send(PwCommand::Terminate);
    pw_handle.join().ok();

    result.map_err(|e: std::io::Error| color_eyre::eyre::eyre!("{e}"))
}
