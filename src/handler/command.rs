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
            if let Some(gain_str) = parts.get(1) {
                if let Ok(gain) = gain_str.parse::<f32>() {
                    app.preamp = gain;
                    let _ = app.sync_bands();
                }
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
