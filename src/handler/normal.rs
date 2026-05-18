use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, FocusedBlock, Mode};
use crate::state::{EqBand, FilterType, PwCommand};

pub fn handle(key: KeyEvent, app: &mut App) -> Option<PwCommand> {
    match app.focused_block {
        FocusedBlock::Devices => handle_devices(key, app),
        FocusedBlock::Pipeline => {
            handle_pipeline(key, app);
            None
        }
        FocusedBlock::CommandBar => {
            handle_command_bar(key, app);
            None
        }
    }
}

fn handle_devices(key: KeyEvent, app: &mut App) -> Option<PwCommand> {
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
        KeyCode::Enter => {
            if !app.nodes.is_empty() {
                let target_node = &app.nodes[app.nodes_selected];
                // Skip if it's the eqtui null sink itself (can't route eqtui → eqtui)
                if target_node.description.contains("eqtui") {
                    return None;
                }
                let target_id = target_node.id;
                app.bound_output_id = Some(target_id);
                return Some(PwCommand::SetTarget { node_id: target_id });
            }
        }
        _ => {}
    }
    None
}

fn handle_pipeline(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Tab => {
            app.focused_block = FocusedBlock::Devices;
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if app.eq_column_selected > 1 {
                app.eq_column_selected -= 1;
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if app.eq_column_selected < 4 { // Freq(1), Gain(2), Q(3), Type(4)
                app.eq_column_selected += 1;
            }
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
            if let Err(e) = app.sync_bands() {
                tracing::error!(%e, "Failed to sync EQ bands");
            }
        }
        KeyCode::Char('d') => {
            if app.last_key == Some('d') && !app.eq_bands.is_empty() {
                app.eq_bands.remove(app.eq_band_selected);
                if app.eq_band_selected >= app.eq_bands.len() {
                    app.eq_band_selected = app.eq_bands.len().saturating_sub(1);
                }
                app.last_key = None;
                if let Err(e) = app.sync_bands() {
                    tracing::error!(%e, "Failed to sync EQ bands");
                }
                return;
            }
        }
        KeyCode::Char('i') => {
            app.mode = Mode::Insert;
            if !app.eq_bands.is_empty() {
                let b = &app.eq_bands[app.eq_band_selected];
                let val_str = match app.eq_column_selected {
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
                app.cell_input = tui_input::Input::new(val_str);
            }
        }
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
                if let Err(e) = app.sync_bands() {
                    tracing::error!(%e, "Failed to sync EQ bands");
                }
            }
        }
        KeyCode::Char('R') => {
            for b in &mut app.eq_bands {
                b.gain = 0.0;
                b.q = 1.0;
            }
            if let Err(e) = app.sync_bands() {
                tracing::error!(%e, "Failed to sync EQ bands");
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.eq_bands.is_empty() { return; }
            let b = &mut app.eq_bands[app.eq_band_selected];
            match app.eq_column_selected {
                1 => b.frequency = (b.frequency + 50.0).min(20000.0),
                2 => b.gain += 0.5,
                3 => b.q = (b.q + 0.1).min(10.0),
                4 => {
                    b.filter_type = match b.filter_type {
                        FilterType::Peak => FilterType::LowShelf,
                        FilterType::LowShelf => FilterType::HighShelf,
                        FilterType::HighShelf => FilterType::Peak,
                    };
                }
                _ => {}
            }
            if let Err(e) = app.sync_bands() {
                tracing::error!(%e, "Failed to sync EQ bands");
            }
        }
        KeyCode::Char('-') => {
            if app.eq_bands.is_empty() { return; }
            let b = &mut app.eq_bands[app.eq_band_selected];
            match app.eq_column_selected {
                1 => b.frequency = (b.frequency - 50.0).max(20.0),
                2 => b.gain -= 0.5,
                3 => b.q = (b.q - 0.1).max(0.1),
                4 => {
                    b.filter_type = match b.filter_type {
                        FilterType::Peak => FilterType::HighShelf,
                        FilterType::LowShelf => FilterType::Peak,
                        FilterType::HighShelf => FilterType::LowShelf,
                    };
                }
                _ => {}
            }
            if let Err(e) = app.sync_bands() {
                tracing::error!(%e, "Failed to sync EQ bands");
            }
        }
        _ => {}
    }

    app.last_key = match key.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use crate::config::Config;
    use crate::pipeline::Pipeline;
    use std::sync::Arc;

    #[test]
    fn test_handle_pipeline_horizontal_navigation() {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let mut app = App::new(config, pipeline);
        app.focused_block = FocusedBlock::Pipeline;

        // Initial state
        assert_eq!(app.eq_column_selected, 1);

        // Move right with 'l'
        handle_pipeline(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 2);

        // Move right with Right arrow
        handle_pipeline(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 3);

        // Move right to boundary
        handle_pipeline(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 4);

        // Move right again (should clamp)
        handle_pipeline(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 4);

        // Move left with 'h'
        handle_pipeline(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 3);

        // Move left with Left arrow
        handle_pipeline(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 2);

        // Move left to boundary
        handle_pipeline(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 1);

        // Move left again (should clamp)
        handle_pipeline(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_column_selected, 1);
    }

    #[test]
    fn test_handle_pipeline_bumping() {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let mut app = App::new(config, pipeline);
        app.focused_block = FocusedBlock::Pipeline;

        // Add a band to test with
        app.eq_bands.push(EqBand {
            frequency: 1000.0,
            gain: 0.0,
            q: 1.0,
            filter_type: FilterType::Peak,
        });
        app.eq_band_selected = 0;
        app.eq_column_selected = 1; // Frequency

        // Bump frequency up
        handle_pipeline(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].frequency, 1050.0);

        // Bump frequency down
        handle_pipeline(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].frequency, 1000.0);

        // Switch to gain
        app.eq_column_selected = 2;
        handle_pipeline(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].gain, 0.5);

        handle_pipeline(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].gain, 0.0);

        // Switch to Q
        app.eq_column_selected = 3;
        handle_pipeline(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].q, 1.1);

        handle_pipeline(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].q, 1.0);

        // Switch to filter type
        app.eq_column_selected = 4;
        handle_pipeline(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].filter_type, FilterType::LowShelf);

        handle_pipeline(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.eq_bands[0].filter_type, FilterType::Peak);
    }

    #[test]
    fn test_handle_pipeline_insert_mode_initialization() {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let mut app = App::new(config, pipeline);
        app.focused_block = FocusedBlock::Pipeline;

        // Add a band
        app.eq_bands.push(EqBand {
            frequency: 1000.0,
            gain: 5.5,
            q: 1.0,
            filter_type: FilterType::Peak,
        });
        app.eq_band_selected = 0;

        // Test frequency column (1)
        app.eq_column_selected = 1;
        handle_pipeline(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.cell_input.value(), "1000.0");

        // Test gain column (2)
        app.mode = Mode::Normal;
        app.eq_column_selected = 2;
        handle_pipeline(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.cell_input.value(), "5.5");

        // Test Q column (3)
        app.mode = Mode::Normal;
        app.eq_column_selected = 3;
        handle_pipeline(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.cell_input.value(), "1.00");

        // Test filter type column (4)
        app.mode = Mode::Normal;
        app.eq_column_selected = 4;
        handle_pipeline(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &mut app);
        assert_eq!(app.cell_input.value(), "PK");
    }

    #[test]
    fn test_handle_devices_enter() {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let mut app = App::new(config, pipeline);
        app.focused_block = FocusedBlock::Devices;
        app.nodes.push(crate::state::NodeInfo {
            id: 123,
            name: "Test Node".to_string(),
            description: "Test Description".to_string(),
            class: crate::state::DeviceClass::Speaker,
        });
        app.nodes_selected = 0;

        let result = handle_devices(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
        );

        assert!(result.is_some());
        assert_eq!(app.bound_output_id, Some(123));
    }

    #[test]
    fn test_handle_devices_enter_skips_eqtui() {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let mut app = App::new(config, pipeline);
        app.focused_block = FocusedBlock::Devices;
        app.nodes.push(crate::state::NodeInfo {
            id: 99,
            name: "eqtui Equalizer".to_string(),
            description: "eqtui Equalizer".to_string(),
            class: crate::state::DeviceClass::Speaker,
        });
        app.nodes_selected = 0;

        let result = handle_devices(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
        );

        assert!(result.is_none());
        assert_eq!(app.bound_output_id, None);
    }
}

fn handle_command_bar(_key: KeyEvent, app: &mut App) {
    let _ = app;
}
