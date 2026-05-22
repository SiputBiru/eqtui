// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

pub mod command;
pub mod insert;
pub mod normal;
pub mod visual;

use crossterm::event::KeyEvent;

use crate::app::{App, Mode};

/// Route a key event through the current mode's handler.
/// Side effects (band changes, device connections, etc.) are
/// sent to the daemon directly via `App`'s client.
pub fn dispatch(key: KeyEvent, app: &mut App) {
    match app.mode {
        Mode::Normal => normal::handle(key, app),
        Mode::Insert => insert::handle(key, app),
        Mode::Visual => visual::handle(key, app),
        Mode::Command => command::handle(key, app),
    }
}
