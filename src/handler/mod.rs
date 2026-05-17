pub mod command;
pub mod insert;
pub mod normal;
pub mod visual;

use crossterm::event::KeyEvent;

use crate::app::{App, FocusedBlock, Mode};

pub fn dispatch(key: KeyEvent, app: &mut App) {
    match app.focused_block {
        FocusedBlock::Devices => match app.mode {
            Mode::Normal => normal::handle(key, app),
            Mode::Insert => insert::handle(key, app),
            Mode::Visual => visual::handle(key, app),
            Mode::Command => command::handle(key, app),
        },
        FocusedBlock::Pipeline => {}
        FocusedBlock::CommandBar => {}
    }
}
