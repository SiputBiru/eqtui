use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Mode};

pub fn handle(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        _ => {}
    }
}
