use std::sync::Arc;

use tui_input::Input;

use crate::config::Config;
use crate::protocol::PushEvent;
use crate::state::{EqBand, FilterState, NodeInfo, NullSinkState};

use crate::client::DaemonClient;
use crate::AppResult;

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

#[derive(Debug, Clone)]
pub struct EqState {
    pub bypass: bool,
    pub bands: Vec<EqBand>,
    pub band_selected: usize,
    pub column_selected: usize,
    pub cell_input: Input,
}

impl Default for EqState {
    fn default() -> Self {
        Self {
            bypass: false,
            bands: Vec::new(),
            band_selected: 0,
            column_selected: 1,
            cell_input: Input::default(),
        }
    }
}

pub struct App {
    pub running: bool,
    pub config: Arc<Config>,
    pub focused_block: FocusedBlock,
    pub mode: Mode,

    // ── Audio-graph state (synced from daemon) ──
    pub nodes: Vec<NodeInfo>,
    pub pw_connected: bool,
    pub filter_node_id: Option<u32>,
    pub filter_state: FilterState,
    pub null_sink: NullSinkState,
    pub connected_devices: Vec<u32>,

    // ── EQ state (synced from daemon, mutated locally) ──
    pub eq: EqState,

    // ── Metering (computed from daemon push events + decay) ──
    pub peak_l: f32,
    pub peak_r: f32,
    cached_peak_l: f32,  // latest raw peak from daemon
    cached_peak_r: f32,

    // ── UI-only state ──
    pub nodes_selected: usize,
    pub command_input: String,
    pub last_key: Option<char>,

    // ── Daemon connection ──
    //
    // Wrapped in Option so unit tests can construct App without
    // a running daemon.  Production code always provides Some.
    client: Option<DaemonClient>,
}

impl App {
    /// Production constructor — requires a connected `DaemonClient`.
    pub fn new(config: Arc<Config>, client: DaemonClient) -> Self {
        Self {
            running: true,
            config,
            client: Some(client),
            focused_block: FocusedBlock::Devices,
            mode: Mode::Normal,
            nodes: Vec::new(),
            nodes_selected: 0,
            pw_connected: false,
            command_input: String::new(),
            eq: EqState::default(),
            last_key: None,
            peak_l: -60.0,
            peak_r: -60.0,
            // Raw linear peak values from daemon (0.0 = silence, 1.0 = full scale).
            // Converted to dBFS + decay in tick().
            cached_peak_l: 0.0,
            cached_peak_r: 0.0,
            null_sink: NullSinkState::NotLoaded,
            connected_devices: Vec::new(),
            filter_node_id: None,
            filter_state: FilterState::Unconnected,
        }
    }

    /// Access the daemon client (panics if called in tests without one).
    fn client(&mut self) -> &mut DaemonClient {
        self.client
            .as_mut()
            .expect("DaemonClient required — not available in unit tests")
    }

    // ── Event synchronization ─────────────────────────────────────────

    /// Drain all push events from the daemon (non-blocking).
    /// Call once per frame before rendering.  No-op when no
    /// daemon is connected (unit tests).
    pub fn drain_events(&mut self) -> AppResult<()> {
        loop {
            let event = {
                let Some(client) = &mut self.client else {
                    return Ok(());
                };
                client.try_read_event()?
            };
            let Some(event) = event else {
                break;
            };
            self.handle_push_event(event);
        }
        Ok(())
    }

    fn handle_push_event(&mut self, event: PushEvent) {
        match event {
            PushEvent::PeakUpdate { l, r } => {
                self.cached_peak_l = l;
                self.cached_peak_r = r;
            }
            PushEvent::NodeList { nodes } => {
                self.nodes = nodes;
                if self.nodes_selected >= self.nodes.len() {
                    self.nodes_selected = self.nodes.len().saturating_sub(1);
                }
            }
            PushEvent::StateChange { state: _ } => {
                // The full state is available via get_status().
                // For now, just note that something changed.
            }
            PushEvent::FilterReady { node_id } => {
                self.filter_node_id = Some(node_id);
            }
            PushEvent::NullSinkCreated { module_id } => {
                self.null_sink = NullSinkState::Loaded {
                    module_id,
                    has_source: false,
                };
            }
            PushEvent::SourceActive { active } => {
                self.null_sink.set_has_source(active);
            }
            PushEvent::Error { message } => {
                tracing::error!(%message, "Daemon error");
            }
        }
    }

    /// Pull a full status snapshot from the daemon (for initial sync
    /// and after major state transitions).
    pub fn full_sync(&mut self) -> AppResult<()> {
        let status = self.client().get_status()?;
        self.nodes = status.nodes;
        self.pw_connected = status.pw_connected;
        self.filter_state = status.filter_state;
        self.null_sink = status.null_sink;
        self.filter_node_id = status.filter_node_id;
        self.connected_devices = status.connected_devices;
        self.eq.bypass = status.bypass;
        self.eq.bands = status.bands;
        Ok(())
    }

    // ── Peak metering ─────────────────────────────────────────────────

    pub fn tick(&mut self) {
        // Convert raw linear peak (from daemon) to dBFS with decay.
        let mut new_l = 20.0 * (self.cached_peak_l + 1e-7).log10();
        let mut new_r = 20.0 * (self.cached_peak_r + 1e-7).log10();

        new_l = new_l.clamp(-60.0, 0.0);
        new_r = new_r.clamp(-60.0, 0.0);

        let decay_speed = 0.8; // ~24 dB/sec at 30 fps

        if new_l < self.peak_l {
            self.peak_l -= decay_speed;
            if self.peak_l < -60.0 {
                self.peak_l = -60.0;
            }
        } else {
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

    // ── Commands ─────────────────────────────────────────────────────

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Sync the local band configuration to the daemon DSP.
    pub fn sync_bands(&mut self) -> AppResult<()> {
        if let Some(client) = &mut self.client {
            client.set_bands(&self.eq.bands)
        } else {
            Ok(()) // no-op in tests
        }
    }

    pub fn sync_bypass(&mut self) -> AppResult<()> {
        if let Some(client) = &mut self.client {
            client.set_bypass(self.eq.bypass)
        } else {
            Ok(())
        }
    }

    pub fn is_device_connected(&self, id: u32) -> bool {
        self.connected_devices.contains(&id)
    }

    /// Toggle a device connection.  Communicates directly with the
    /// daemon when available (no-op in unit tests).
    pub fn toggle_device_connection(&mut self, id: u32) -> AppResult<()> {
        if self.filter_node_id.is_none() {
            return Ok(()); // filter not ready — nothing to do
        }
        if self.is_device_connected(id) {
            self.connected_devices.retain(|d| *d != id);
            if let Some(client) = &mut self.client {
                client.disconnect_device(id)?;
            }
        } else {
            self.connected_devices.push(id);
            if let Some(client) = &mut self.client {
                client.connect_device(id)?;
            }
        }
        Ok(())
    }
}

// ── Test helper (available to all test modules in the crate) ────────

#[cfg(test)]
impl App {
    pub(crate) fn new_test(config: Arc<Config>) -> Self {
        Self {
            running: true,
            config,
            client: None,
            focused_block: FocusedBlock::Devices,
            mode: Mode::Normal,
            nodes: Vec::new(),
            nodes_selected: 0,
            pw_connected: false,
            command_input: String::new(),
            eq: EqState::default(),
            last_key: None,
            peak_l: -60.0,
            peak_r: -60.0,
            // Raw linear peak values from daemon (0.0 = silence, 1.0 = full scale).
            // Converted to dBFS + decay in tick().
            cached_peak_l: 0.0,
            cached_peak_r: 0.0,
            null_sink: NullSinkState::NotLoaded,
            connected_devices: Vec::new(),
            filter_node_id: None,
            filter_state: FilterState::Unconnected,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_app_initialization() {
        let config = Arc::new(Config::default());
        let app = App::new_test(config);

        assert!(app.running);
        assert_eq!(app.eq.band_selected, 0);
        assert_eq!(app.eq.column_selected, 1);
        assert_eq!(app.eq.cell_input.value(), "");
        assert!(!app.eq.bypass);

        let margin = f32::EPSILON;
        assert!((app.peak_l - (-60.0_f32)).abs() < margin);
        assert!((app.peak_r - (-60.0_f32)).abs() < margin);
        assert!(!app.null_sink.is_loaded());
        assert_eq!(app.null_sink.module_id(), None);
        assert!(app.connected_devices.is_empty());
        assert_eq!(app.filter_node_id, None);
        assert!(!app.null_sink.has_source());
        assert_eq!(app.filter_state.to_string(), "UNCONNECTED");
    }
}
