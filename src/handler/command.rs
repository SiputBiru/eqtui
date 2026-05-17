use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Mode};

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
            dispatch_command(&cmd, app);
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

fn dispatch_command(cmd: &str, app: &mut App) {
    match cmd {
        "q" => app.quit(),
        _ => {}
    }
}
