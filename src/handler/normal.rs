// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

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
        KeyCode::Down | KeyCode::Char('j') if !app.nodes.is_empty() => {
            app.nodes_selected = (app.nodes_selected + 1) % app.nodes.len();
        }
        KeyCode::Up | KeyCode::Char('k') if !app.nodes.is_empty() => {
            app.nodes_selected = app
                .nodes_selected
                .checked_sub(1)
                .unwrap_or(app.nodes.len() - 1);
        }
        KeyCode::Char('c' | 'C') if !app.nodes.is_empty() => {
            let target_id = app.nodes[app.nodes_selected].id;
            // Reject connecting the null sink (or filter) to itself —
            // would create an audio feedback loop.
            if app.null_sink.module_id() == Some(target_id) || app.filter_node_id == Some(target_id)
            {
                app.notify("Rejected: connecting null-sink or filter to itself would create a feedback loop");
                return;
            }
            if let Err(e) = app.toggle_device_connection(target_id) {
                tracing::error!(%e, "Failed to toggle device connection");
            }
        }
        _ => {}
    }
}

fn handle_pipeline(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Tab => {
            app.focused_block = FocusedBlock::Devices;
        }
        KeyCode::Left | KeyCode::Char('h') if app.eq.column_selected > 1 => {
            app.eq.column_selected -= 1;
        }
        KeyCode::Right | KeyCode::Char('l') if app.eq.column_selected < 4 => {
            // Freq(1), Gain(2), Q(3), Type(4)
            app.eq.column_selected += 1;
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
        KeyCode::Char('{') => app.switch_profile(-1),
        KeyCode::Char('}') => app.switch_profile(1),
        KeyCode::Char('a') => {
            let freq = 1000.0;
            let gain = 0.0;
            let q = 1.0;
            let ftype = FilterType::Peak;
            let insert_at = (app.eq.band_selected + 1).min(app.eq.bands.len());
            app.eq.bands.insert(
                insert_at,
                EqBand {
                    frequency: freq,
                    gain,
                    q,
                    filter_type: ftype,
                },
            );
            app.eq.band_selected = insert_at;
        }
        KeyCode::Char('d') if app.last_key == Some('d') && !app.eq.bands.is_empty() => {
            app.eq.bands.remove(app.eq.band_selected);
            if app.eq.band_selected >= app.eq.bands.len() {
                app.eq.band_selected = app.eq.bands.len().saturating_sub(1);
            }
            app.last_key = None;

            return;
        }
        KeyCode::Char('i') => {
            app.mode = Mode::Insert;
            if !app.eq.bands.is_empty() {
                let b = &app.eq.bands[app.eq.band_selected];
                let val_str = match app.eq.column_selected {
                    1 => format!("{:.1}", b.frequency),
                    2 => format!("{:.1}", b.gain),
                    3 => format!("{:.2}", b.q),
                    4 => match b.filter_type {
                        FilterType::Peak => "PK".to_string(),
                        FilterType::LowShelf => "LS".to_string(),
                        FilterType::HighShelf => "HS".to_string(),
                    },
                    _ => String::new(),
                };
                let start_empty = match app.eq.column_selected {
                    1 => b.frequency == 0.0,
                    2 => b.gain == 0.0,
                    3 => b.q == 0.0,
                    _ => false,
                };
                app.eq.cell_input =
                    tui_input::Input::new(if start_empty { String::new() } else { val_str });
            }
        }
        KeyCode::Char('v') => app.mode = Mode::Visual,
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.command_input.reset();
        }
        KeyCode::Char('b') => {
            app.eq.bypass = !app.eq.bypass;
            let _ = app.sync_bypass();
        }
        KeyCode::Char('g') if app.last_key == Some('g') && !app.eq.bands.is_empty() => {
            app.eq.band_selected = 0;
            app.last_key = None;
            return;
        }
        KeyCode::Char('r') if !app.eq.bands.is_empty() => {
            let b = &mut app.eq.bands[app.eq.band_selected];
            b.gain = 0.0;
            b.q = 1.0;
        }
        KeyCode::Char('R') => {
            for b in &mut app.eq.bands {
                b.gain = 0.0;
                b.q = 1.0;
            }
        }
        KeyCode::Char('+' | '=') => {
            if app.eq.bands.is_empty() {
                return;
            }
            let b = &mut app.eq.bands[app.eq.band_selected];
            bump_band(b, app.eq.column_selected, 1);
        }
        KeyCode::Char('-') => {
            if app.eq.bands.is_empty() {
                return;
            }
            let b = &mut app.eq.bands[app.eq.band_selected];
            bump_band(b, app.eq.column_selected, -1);
        }
        _ => {}
    }

    app.last_key = match key.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    };
}

/// Adjusts a band parameter by a delta direction (`dir`: +1 or -1).
/// Column 1 = frequency, 2 = gain, 3 = Q, 4 = filter type cycle.
fn bump_band(band: &mut EqBand, col: usize, dir: i8) {
    match col {
        1 => band.frequency = (band.frequency + 50.0 * f32::from(dir)).clamp(20.0, 20000.0),
        2 => band.gain += 0.5 * f32::from(dir),
        3 => band.q = (band.q + 0.1 * f32::from(dir)).clamp(0.1, 10.0),
        4 => {
            const CYCLE: [FilterType; 3] = [
                FilterType::Peak,
                FilterType::LowShelf,
                FilterType::HighShelf,
            ];
            let idx = CYCLE
                .iter()
                .position(|t| *t == band.filter_type)
                .unwrap_or(0);
            // +1 forward, -1 backward (+2 ≡ -1 mod 3)
            band.filter_type = CYCLE[match dir {
                1 => (idx + 1) % 3,
                -1 => (idx + 2) % 3,
                _ => idx,
            }];
        }
        _ => {}
    }
}

fn handle_command_bar(_key: KeyEvent, app: &mut App) {
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::sync::Arc;

    #[test]
    fn test_handle_pipeline_horizontal_navigation() {
        let config = Arc::new(Config::default());
        let mut app = App::new_test(config);
        app.focused_block = FocusedBlock::Pipeline;

        assert_eq!(app.eq.column_selected, 1);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.column_selected, 2);

        handle_pipeline(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq.column_selected, 3);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.column_selected, 4);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.column_selected, 4);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.column_selected, 3);

        handle_pipeline(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq.column_selected, 2);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.column_selected, 1);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.column_selected, 1);
    }

    #[test]
    fn test_handle_pipeline_bumping() {
        let config = Arc::new(Config::default());
        let mut app = App::new_test(config);
        app.focused_block = FocusedBlock::Pipeline;

        app.eq.bands.push(EqBand {
            frequency: 1000.0,
            gain: 0.0,
            q: 1.0,
            filter_type: FilterType::Peak,
        });
        app.eq.band_selected = 0;
        app.eq.column_selected = 1; // Frequency

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
            &mut app,
        );
        assert!((app.eq.bands[0].frequency - 1050.0).abs() < f32::EPSILON);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE),
            &mut app,
        );
        assert!((app.eq.bands[0].frequency - 1000.0).abs() < f32::EPSILON);

        app.eq.column_selected = 2;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE),
            &mut app,
        );
        assert!((app.eq.bands[0].gain - 0.5).abs() < f32::EPSILON);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE),
            &mut app,
        );
        assert!((app.eq.bands[0].gain - 0.0).abs() < f32::EPSILON);

        app.eq.column_selected = 3;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
            &mut app,
        );
        assert!((app.eq.bands[0].q - 1.1).abs() < f32::EPSILON);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE),
            &mut app,
        );
        assert!((app.eq.bands[0].q - 1.0).abs() < f32::EPSILON);

        app.eq.column_selected = 4;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.bands[0].filter_type, FilterType::LowShelf);

        handle_pipeline(
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.bands[0].filter_type, FilterType::Peak);
    }

    #[test]
    fn test_handle_pipeline_insert_mode_initialization() {
        let config = Arc::new(Config::default());
        let mut app = App::new_test(config);
        app.focused_block = FocusedBlock::Pipeline;

        app.eq.bands.push(EqBand {
            frequency: 1000.0,
            gain: 5.5,
            q: 1.0,
            filter_type: FilterType::Peak,
        });
        app.eq.band_selected = 0;

        app.eq.column_selected = 1;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.eq.cell_input.value(), "1000.0");

        app.mode = Mode::Normal;
        app.eq.column_selected = 2;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.cell_input.value(), "5.5");

        app.mode = Mode::Normal;
        app.eq.column_selected = 3;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.cell_input.value(), "1.00");

        app.mode = Mode::Normal;
        app.eq.column_selected = 4;
        handle_pipeline(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &mut app,
        );
        assert_eq!(app.eq.cell_input.value(), "PK");
    }

    #[test]
    fn test_handle_devices_connect_toggle() {
        let config = Arc::new(Config::default());
        let mut app = App::new_test(config);
        app.focused_block = FocusedBlock::Devices;
        app.filter_node_id = Some(42); // simulate filter ready
        app.nodes.push(crate::state::NodeInfo {
            id: 123,
            name: "Test Device".to_string(),
            description: "Test Description".to_string(),
            class: crate::state::DeviceClass::Speaker,
        });
        app.nodes_selected = 0;

        handle_devices(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut app,
        );
        assert!(app.is_device_connected(123));
        assert_eq!(app.connected_devices, vec![123]);

        handle_devices(
            KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE),
            &mut app,
        );
        assert!(!app.is_device_connected(123));
        assert!(app.connected_devices.is_empty());
    }

    #[test]
    fn test_handle_devices_connect_skips_null_sink() {
        let config = Arc::new(Config::default());
        let mut app = App::new_test(config);
        app.focused_block = FocusedBlock::Devices;
        app.filter_node_id = Some(42);
        // Simulate the null sink with a known PipeWire node ID.
        app.null_sink = crate::state::NullSinkState::Loaded {
            module_id: 99,
            has_source: false,
        };
        app.nodes.push(crate::state::NodeInfo {
            id: 99,
            name: "eqtui".to_string(),
            description: "eqtui (Virtual Sink)".to_string(),
            class: crate::state::DeviceClass::Speaker,
        });
        app.nodes_selected = 0;

        handle_devices(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut app,
        );
        // Should skip because the target node matches the null sink ID.
        assert!(app.connected_devices.is_empty());
    }

    #[test]
    fn test_handle_devices_connect_no_filter_ready() {
        let config = Arc::new(Config::default());
        let mut app = App::new_test(config);
        app.focused_block = FocusedBlock::Devices;
        // filter_node_id is None — not ready yet
        app.nodes.push(crate::state::NodeInfo {
            id: 123,
            name: "Test Device".to_string(),
            description: "Test Description".to_string(),
            class: crate::state::DeviceClass::Speaker,
        });
        app.nodes_selected = 0;

        handle_devices(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut app,
        );
        // Should do nothing because filter isn't ready
        assert!(app.connected_devices.is_empty());
    }
}
