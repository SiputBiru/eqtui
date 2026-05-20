use std::sync::Arc;

use tui_input::Input;

use crate::AppResult;
use crate::config::Config;
use crate::pipeline::Pipeline;
use crate::state::{EqBand, NodeInfo, PwCommand, PwEvent};

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
    pub peak_l: f32,
    pub peak_r: f32,
    pub null_sink_loaded: bool,
    pub null_sink_module_id: Option<u32>,
    pub connected_devices: Vec<u32>,
    pub filter_node_id: Option<u32>,
    pub null_sink_has_source: bool,
    pub filter_state: String,
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
            peak_l: -60.0,
            peak_r: -60.0,
            null_sink_loaded: false,
            null_sink_module_id: None,
            connected_devices: Vec::new(),
            filter_node_id: None,
            null_sink_has_source: false,
            filter_state: "UNCONNECTED".to_string(),
        }
    }

    pub fn tick(&mut self) {
        let (mut new_l, mut new_r) = self.pipeline.peaks();

        // Prevent log10(0) by adding a tiny epsilon
        new_l = 20.0 * (new_l + 1e-7).log10();
        new_r = 20.0 * (new_r + 1e-7).log10();

        // Clamp to a reasonable range (e.g., -60dB to 0dB)
        new_l = new_l.clamp(-60.0, 0.0);
        new_r = new_r.clamp(-60.0, 0.0);

        // decay speed (approx 24dB/sec at 30fps)
        let decay_speed = 0.8;

        if new_l < self.peak_l {
            self.peak_l -= decay_speed;
            if self.peak_l < -60.0 {
                self.peak_l = -60.0;
            }
        } else {
            // Instant attack (snap to higher peak)
            self.peak_l = new_l;
        }

        if new_r < self.peak_r {
            self.peak_r -= decay_speed;
            if self.peak_r < -60.0 {
                self.peak_r = -60.0;
            }
        } else {
            self.peak_r = new_r;
        }
    }

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
            PwEvent::FilterStateChanged(state) => {
                self.filter_state = state;
            }
            PwEvent::FilterReady { node_id } => {
                self.filter_node_id = Some(node_id);
            }
            PwEvent::NullSinkCreated { module_id } => {
                self.null_sink_loaded = true;
                self.null_sink_module_id = Some(module_id);
            }
            PwEvent::NullSinkInputState { has_source } => {
                self.null_sink_has_source = has_source;
            }
            PwEvent::NullSinkError(e) => {
                tracing::error!(%e, "Null sink error");
            }
            PwEvent::Error(e) => {
                tracing::error!(%e, "PW error");
            }
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn sync_bands(&self) -> AppResult<()> {
        self.pipeline.set_bands(self.eq_bands.clone(), 48000.0)
    }

    pub fn is_device_connected(&self, id: u32) -> bool {
        self.connected_devices.contains(&id)
    }

    /// Toggle connection state for a device. Returns the PW command to
    /// execute, or `None` if the filter isn't ready yet.
    pub fn toggle_device_connection(&mut self, id: u32) -> Option<PwCommand> {
        let filter_id = self.filter_node_id?;
        if self.is_device_connected(id) {
            self.connected_devices.retain(|d| *d != id);
            Some(PwCommand::DisconnectDevice {
                filter_id,
                node_id: id,
            })
        } else {
            self.connected_devices.push(id);
            Some(PwCommand::ConnectDevice {
                filter_id,
                node_id: id,
            })
        }
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

        let margin = f32::EPSILON;
        assert!((app.peak_l - (-60.0_f32)).abs() < margin);
        assert!((app.peak_r - (-60.0_f32)).abs() < margin);
        assert!(!app.null_sink_loaded);
        assert_eq!(app.null_sink_module_id, None);
        assert!(app.connected_devices.is_empty());
        assert_eq!(app.filter_node_id, None);
        assert!(!app.null_sink_has_source);
        assert_eq!(app.filter_state, "UNCONNECTED");
    }
}
