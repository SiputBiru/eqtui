use std::sync::Arc;

use tui_input::Input;

use crate::config::Config;
use crate::pipeline::Pipeline;
use crate::state::{EqBand, NodeInfo, PwEvent};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusedBlock {
    Devices,
    Pipeline,
    CommandBar,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    Command,
}

pub struct App {
    pub running: bool,
    pub config: Arc<Config>,
    pub focused_block: FocusedBlock,
    pub mode: Mode,
    pub nodes: Vec<NodeInfo>,
    pub nodes_selected: usize,
    pub pw_connected: bool,
    pub command_input: String,
    pub eq_bands: Vec<EqBand>,
    pub eq_band_selected: usize,
    pub eq_column_selected: usize,
    pub cell_input: Input,
    pub eq_bypass: bool,
    pub last_key: Option<char>,
    pub pipeline: Arc<Pipeline>,
}

impl App {
    pub fn new(config: Arc<Config>, pipeline: Arc<Pipeline>) -> Self {
        Self {
            running: true,
            config,
            focused_block: FocusedBlock::Devices,
            mode: Mode::Normal,
            nodes: Vec::new(),
            nodes_selected: 0,
            pw_connected: false,
            command_input: String::new(),
            eq_bands: Vec::new(),
            eq_band_selected: 0,
            eq_column_selected: 1,
            cell_input: Input::default(),
            eq_bypass: false,
            last_key: None,
            pipeline,
        }
    }

    pub fn tick(&mut self) {}

    pub fn handle_pw_event(&mut self, event: PwEvent) {
        match event {
            PwEvent::NodeList(list) => {
                self.nodes = list;
                if self.nodes_selected >= self.nodes.len() {
                    self.nodes_selected = self.nodes.len().saturating_sub(1);
                }
            }
            PwEvent::NodeAdded(node) => {
                self.nodes.push(node);
            }
            PwEvent::NodeRemoved(id) => {
                self.nodes.retain(|n| n.id != id);
                if self.nodes_selected >= self.nodes.len() {
                    self.nodes_selected = self.nodes.len().saturating_sub(1);
                }
            }
            PwEvent::Connected => {
                self.pw_connected = true;
            }
            PwEvent::Error(e) => {
                eprintln!("PW error: {e}");
            }
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn sync_bands(&self) {
        self.pipeline
            .set_bands(self.eq_bands.clone(), 48000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_app_initialization() {
        let config = Arc::new(Config::default());
        let pipeline = Arc::new(Pipeline::new(48000.0));
        let app = App::new(config, pipeline);

        assert!(app.running);
        assert_eq!(app.eq_band_selected, 0);
        assert_eq!(app.eq_column_selected, 1);
        assert_eq!(app.cell_input.value(), "");
        assert!(!app.eq_bypass);
    }
}
