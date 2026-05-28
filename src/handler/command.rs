// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use crossterm::event::{Event, KeyCode, KeyEvent};
use tui_input::backend::crossterm::EventHandler;

use crate::app::{App, Mode};
use crate::state::{EqBand, FilterType};

pub fn handle(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.command_input.reset();
        }
        KeyCode::Enter => {
            let cmd = app.command_input.value().to_string();
            app.command_input.reset();
            app.mode = Mode::Normal;
            exec(&cmd, app);
        }
        _ => {
            app.command_input.handle_event(&Event::Key(key));
        }
    }
}

fn exec(cmd: &str, app: &mut App) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    match parts.first().copied() {
        Some("q") => app.quit(),
        Some("w") => {
            if let Err(e) = app.sync_bands() {
                app.notify(format!("Save failed: {e}"));
            }
        }
        Some("flat") => {
            for b in &mut app.eq.bands {
                b.gain = 0.0;
            }
        }
        Some("load") => {
            if let Some(path) = parts.get(1) {
                match app.load_peq(path) {
                    Ok(()) => {}
                    Err(e) => app.notify(format!("Error: {e}")),
                }
            }
        }
        Some("bypass") => {
            app.eq.bypass = !app.eq.bypass;
            let _ = app.sync_bypass();
        }
        Some("preamp") => {
            if let Some(gain_str) = parts.get(1)
                && let Ok(gain) = gain_str.parse::<f32>()
            {
                app.preamp = gain;
                let _ = app.sync_bands();
            }
        }
        Some("add") => {
            let freq = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000.0);
            app.eq.bands.push(EqBand {
                frequency: freq,
                gain: 0.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;

    fn setup_app() -> App {
        let config = Arc::new(Config::default());
        App::new_test(config)
    }

    #[test]
    fn test_handle_command_typing() {
        let mut app = setup_app();
        app.mode = Mode::Command;

        handle(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.command_input.value(), "q");
    }

    #[test]
    fn test_handle_command_backspace() {
        let mut app = setup_app();
        app.mode = Mode::Command;
        handle(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut app,
        );
        handle(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.command_input.value(), "");
    }

    #[test]
    fn test_handle_command_enter() {
        let mut app = setup_app();
        app.mode = Mode::Command;
        handle(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut app,
        );
        handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut app);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.command_input.value(), "");
        assert!(!app.running); // :q sets running to false
    }

    #[test]
    fn test_handle_command_esc() {
        let mut app = setup_app();
        app.mode = Mode::Command;
        handle(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut app,
        );
        handle(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut app);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.command_input.value(), "");
        assert!(app.running);
    }
}
