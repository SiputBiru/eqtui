pub mod command;
pub mod insert;
pub mod normal;
pub mod visual;

use crossterm::event::KeyEvent;

use crate::app::{App, Mode};
use crate::state::PwCommand;

pub fn dispatch(key: KeyEvent, app: &mut App) -> Option<PwCommand> {
    match app.mode {
        Mode::Normal => normal::handle(key, app),
        Mode::Insert => {
            insert::handle(key, app);
            None
        }
        Mode::Visual => {
            visual::handle(key, app);
            None
        }
        Mode::Command => {
            command::handle(key, app);
            None
        }
    }
}
