use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;

pub fn handle(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Down | KeyCode::Char('j') => {
            if !app.nodes.is_empty() {
                app.nodes_selected = (app.nodes_selected + 1) % app.nodes.len();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if !app.nodes.is_empty() {
                app.nodes_selected = app
                    .nodes_selected
                    .checked_sub(1)
                    .unwrap_or(app.nodes.len() - 1);
            }
        }
        _ => {}
    }
}
