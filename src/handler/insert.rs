use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, FocusedBlock, Mode};

pub fn handle(key: KeyEvent, app: &mut App) {
    if app.focused_block != FocusedBlock::Pipeline {
        app.mode = Mode::Normal;
        return;
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.eq_bands.is_empty() {
                return;
            }
            let b = &mut app.eq_bands[app.eq_band_selected];
            b.gain += 0.5;
            app.sync_bands();
        }
        KeyCode::Char('-') => {
            if app.eq_bands.is_empty() {
                return;
            }
            let b = &mut app.eq_bands[app.eq_band_selected];
            b.gain -= 0.5;
            app.sync_bands();
        }
        _ => {}
    }
}
