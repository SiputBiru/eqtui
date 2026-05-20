use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Mode};

pub fn handle(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Down | KeyCode::Char('j') if !app.eq_bands.is_empty() => {
            app.eq_band_selected = (app.eq_band_selected + 1) % app.eq_bands.len();
        }
        KeyCode::Up | KeyCode::Char('k') if !app.eq_bands.is_empty() => {
            app.eq_band_selected = app
                .eq_band_selected
                .checked_sub(1)
                .unwrap_or(app.eq_bands.len() - 1);
        }
        KeyCode::Char('d') if !app.eq_bands.is_empty() => {
            app.eq_bands.remove(app.eq_band_selected);
            if app.eq_band_selected >= app.eq_bands.len() {
                app.eq_band_selected = app.eq_bands.len().saturating_sub(1);
            }
            if let Err(e) = app.sync_bands() {
                tracing::error!(%e, "Failed to sync EQ bands");
            }
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}
