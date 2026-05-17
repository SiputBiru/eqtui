use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, FocusedBlock, Mode};
use crate::state::{EqBand, FilterType};

pub fn handle(key: KeyEvent, app: &mut App) {
    match app.focused_block {
        FocusedBlock::Devices => handle_devices(key, app),
        FocusedBlock::Pipeline => handle_pipeline(key, app),
        FocusedBlock::CommandBar => handle_command_bar(key, app),
    }
}

fn handle_devices(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Tab | KeyCode::Char('l') => {
            app.focused_block = FocusedBlock::Pipeline;
        }
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

fn handle_pipeline(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Tab | KeyCode::Char('l') => {
            app.focused_block = FocusedBlock::Devices;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !app.eq_bands.is_empty() {
                app.eq_band_selected = (app.eq_band_selected + 1) % app.eq_bands.len();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if !app.eq_bands.is_empty() {
                app.eq_band_selected = app
                    .eq_band_selected
                    .checked_sub(1)
                    .unwrap_or(app.eq_bands.len() - 1);
            }
        }
        KeyCode::Char('a') => {
            let freq = 1000.0;
            let gain = 0.0;
            let q = 1.0;
            let ftype = FilterType::Peak;
            let insert_at = (app.eq_band_selected + 1).min(app.eq_bands.len());
            app.eq_bands.insert(
                insert_at,
                EqBand { frequency: freq, gain, q, filter_type: ftype },
            );
            app.eq_band_selected = insert_at;
            app.sync_bands();
        }
        KeyCode::Char('d') => {
            if app.last_key == Some('d') && !app.eq_bands.is_empty() {
                app.eq_bands.remove(app.eq_band_selected);
                if app.eq_band_selected >= app.eq_bands.len() {
                    app.eq_band_selected = app.eq_bands.len().saturating_sub(1);
                }
                app.last_key = None;
                app.sync_bands();
                return;
            }
        }
        KeyCode::Char('i') => app.mode = Mode::Insert,
        KeyCode::Char('v') => app.mode = Mode::Visual,
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.command_input.clear();
        }
        KeyCode::Char('b') => {
            app.eq_bypass = !app.eq_bypass;
            if app.eq_bypass {
                app.pipeline.set_bypass(true);
            } else {
                app.pipeline.set_bypass(false);
            }
        }
        KeyCode::Char('g') => {
            if app.last_key == Some('g') && !app.eq_bands.is_empty() {
                app.eq_band_selected = 0;
                app.last_key = None;
                return;
            }
        }
        KeyCode::Char('r') => {
            if !app.eq_bands.is_empty() {
                let b = &mut app.eq_bands[app.eq_band_selected];
                b.gain = 0.0;
                b.q = 1.0;
                app.sync_bands();
            }
        }
        KeyCode::Char('R') => {
            for b in &mut app.eq_bands {
                b.gain = 0.0;
                b.q = 1.0;
            }
            app.sync_bands();
        }
        _ => {}
    }

    app.last_key = match key.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    };
}

fn handle_command_bar(_key: KeyEvent, app: &mut App) {
    let _ = app;
}
