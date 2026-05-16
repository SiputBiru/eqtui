mod pw;
mod state;
mod tui;

use std::sync::mpsc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode};
use state::{AppState, PwCommand, PwEvent};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    // Channels: PW thread ↔ TUI main thread
    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = pipewire::channel::channel::<PwCommand>();

    // Spawn PipeWire background thread
    let pw_handle = std::thread::spawn(move || {
        pw::run(to_tui, from_tui);
    });

    // Run the TUI
    let result = ratatui::run(|terminal| {
        let mut state = AppState::new();

        loop {
            // Drain any pending PipeWire events
            while let Ok(event) = from_pw.try_recv() {
                state.handle_event(event);
            }

            // Render
            terminal.draw(|frame| tui::render(frame, &state))?;

            // Non-blocking keyboard poll (~60fps)
            if crossterm::event::poll(Duration::from_millis(16))? {
                match crossterm::event::read()? {
                    Event::Key(key) => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                        KeyCode::Up | KeyCode::Char('k') => state.select_prev(),
                        _ => {}
                    },
                    Event::Resize(_, _) => {
                        // ratatui handles resize automatically on next draw
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    });

    // Clean shutdown: tell PW thread to quit, then wait
    let _ = to_pw.send(PwCommand::Terminate);
    pw_handle.join().ok();

    result.map_err(|e: std::io::Error| color_eyre::eyre::eyre!("{e}"))
}
