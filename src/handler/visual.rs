use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Mode};

pub fn handle(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Down | KeyCode::Char('j') if !app.eq.bands.is_empty() => {
            app.eq.band_selected = (app.eq.band_selected + 1) % app.eq.bands.len();
        }
        KeyCode::Up | KeyCode::Char('k') if !app.eq.bands.is_empty() => {
            app.eq.band_selected = app
                .eq
                .band_selected
                .checked_sub(1)
                .unwrap_or(app.eq.bands.len() - 1);
        }
        KeyCode::Char('d') if !app.eq.bands.is_empty() => {
            app.eq.bands.remove(app.eq.band_selected);
            if app.eq.band_selected >= app.eq.bands.len() {
                app.eq.band_selected = app.eq.bands.len().saturating_sub(1);
            }
            if let Err(e) = app.sync_bands() {
                tracing::error!(%e, "Failed to sync EQ bands");
            }
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}
