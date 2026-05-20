use crossterm::event::{Event, KeyCode, KeyEvent};
use tui_input::backend::crossterm::EventHandler;

use crate::app::{App, FocusedBlock, Mode};
use crate::state::FilterType;

pub fn handle(key: KeyEvent, app: &mut App) {
    if app.focused_block != FocusedBlock::Pipeline {
        app.mode = Mode::Normal;
        return;
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => {
            if !app.eq.bands.is_empty() {
                let val_str = app.eq.cell_input.value();
                let b = &mut app.eq.bands[app.eq.band_selected];

                match app.eq.column_selected {
                    1 => {
                        if let Ok(v) = val_str.parse::<f32>() {
                            b.frequency = v.clamp(20.0, 20000.0);
                        }
                    }
                    2 => {
                        if let Ok(v) = val_str.parse::<f32>() {
                            b.gain = v.clamp(-30.0, 30.0);
                        }
                    }
                    3 => {
                        if let Ok(v) = val_str.parse::<f32>() {
                            b.q = v.clamp(0.1, 10.0);
                        }
                    }
                    4 => {
                        let upper = val_str.to_uppercase();
                        if upper == "PK" || upper == "PEAK" {
                            b.filter_type = FilterType::Peak;
                        } else if upper == "LS" || upper == "LOWSHELF" {
                            b.filter_type = FilterType::LowShelf;
                        } else if upper == "HS" || upper == "HIGHSHELF" {
                            b.filter_type = FilterType::HighShelf;
                        }
                    }
                    _ => {}
                }
                if let Err(e) = app.sync_bands() {
                    tracing::error!(%e, "Failed to sync EQ bands");
                }
            }
            app.mode = Mode::Normal;
        }
        _ => {
            app.eq.cell_input.handle_event(&Event::Key(key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::FocusedBlock;
    use crate::config::Config;
    use crate::pipeline::Pipeline;
    use crate::state::EqBand;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::sync::Arc;
    use tui_input::Input;

    fn setup_app() -> App {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let mut app = App::new(config, pipeline);
        app.focused_block = FocusedBlock::Pipeline;
        app.mode = Mode::Insert;
        app.eq.bands.push(EqBand {
            frequency: 1000.0,
            gain: 0.0,
            q: 1.0,
            filter_type: FilterType::Peak,
        });
        app
    }

    #[test]
    fn test_handle_insert_mode_typing() {
        let mut app = setup_app();
        app.eq.column_selected = 1;
        app.eq.cell_input = Input::new("123".to_string());

        let key = KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE);
        handle(key, &mut app);
        assert_eq!(app.eq.cell_input.value(), "1234");
    }

    #[test]
    fn test_handle_commit_frequency() {
        let mut app = setup_app();
        app.eq.column_selected = 1;
        app.eq.cell_input = Input::new("500".to_string());

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle(key, &mut app);
        assert_eq!(app.eq.bands[0].frequency, 500.0);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_handle_commit_gain() {
        let mut app = setup_app();
        app.eq.column_selected = 2;
        app.eq.cell_input = Input::new("-10.5".to_string());

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle(key, &mut app);
        assert_eq!(app.eq.bands[0].gain, -10.5);
    }

    #[test]
    fn test_handle_commit_q() {
        let mut app = setup_app();
        app.eq.column_selected = 3;
        app.eq.cell_input = Input::new("2.5".to_string());

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle(key, &mut app);
        assert_eq!(app.eq.bands[0].q, 2.5);
    }

    #[test]
    fn test_handle_commit_filter_type() {
        let mut app = setup_app();
        app.eq.column_selected = 4;

        app.eq.cell_input = Input::new("ls".to_string());
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle(key, &mut app);
        assert!(matches!(app.eq.bands[0].filter_type, FilterType::LowShelf));

        app.mode = Mode::Insert;
        app.eq.cell_input = Input::new("HS".to_string());
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle(key, &mut app);
        assert!(matches!(app.eq.bands[0].filter_type, FilterType::HighShelf));

        app.mode = Mode::Insert;
        app.eq.cell_input = Input::new("peak".to_string());
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle(key, &mut app);
        assert!(matches!(app.eq.bands[0].filter_type, FilterType::Peak));
    }

    #[test]
    fn test_handle_esc() {
        let mut app = setup_app();
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle(key, &mut app);
        assert_eq!(app.mode, Mode::Normal);
    }
}
