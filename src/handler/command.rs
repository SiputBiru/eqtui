use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Mode};
use crate::state::{EqBand, FilterType};

pub fn handle(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.command_input.clear();
        }
        KeyCode::Enter => {
            let cmd = app.command_input.clone();
            app.command_input.clear();
            app.mode = Mode::Normal;
            exec(&cmd, app);
        }
        KeyCode::Char(c) => {
            app.command_input.push(c);
        }
        KeyCode::Backspace => {
            app.command_input.pop();
        }
        _ => {}
    }
}

fn exec(cmd: &str, app: &mut App) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    match parts.first().copied() {
        Some("q") => app.quit(),
        Some("w") => {
            // Save preset (placeholder)
        }
        Some("flat") => {
            for b in &mut app.eq_bands {
                b.gain = 0.0;
            }
            app.sync_bands();
        }
        Some("bypass") => {
            app.eq_bypass = !app.eq_bypass;
        }
        Some("add") => {
            let freq = parts
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000.0);
            app.eq_bands.push(EqBand {
                frequency: freq,
                gain: 0.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            });
            app.sync_bands();
        }
        _ => {}
    }
}
