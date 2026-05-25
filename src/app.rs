// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::sync::Arc;
use std::time::{Duration, Instant};

use tui_input::Input;

use crate::AppResult;
use crate::client::DaemonClient;
use crate::config::Config;
use crate::profiles::{self, Profile};
use crate::protocol::PushEvent;
use crate::state::{EqBand, FilterState, NodeInfo, NullSinkState};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DaemonConnection {
    Connected,
    Reconnecting,
    Disconnected,
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

    pub nodes: Vec<NodeInfo>,
    pub pw_connected: bool,
    pub daemon: DaemonConnection,
    pub filter_node_id: Option<u32>,
    pub filter_state: FilterState,
    pub null_sink: NullSinkState,
    /// True when null-audio-sink creation failed — the filter runs
    /// but processes silence because no audio source is wired.
    pub null_sink_missing: bool,
    pub connected_devices: Vec<u32>,

    pub eq: EqState,
    pub preamp: f32,

    pub profiles: Vec<Profile>,
    pub active_profile: usize,

    pub peak_l: f32,
    pub peak_r: f32,
    cached_peak_l: f32,
    cached_peak_r: f32,

    pub nodes_selected: usize,
    pub command_input: Input,
    pub last_key: Option<char>,

    /// Transient status message with deadline instant.
    /// Cleared automatically when the deadline passes.
    pub notification: Option<(String, Instant)>,

    /// Wrapped in `Option` so unit tests can exist without a daemon.
    client: Option<DaemonClient>,
}

impl App {
    pub fn new(config: Arc<Config>, client: DaemonClient) -> Self {
        let profiles = profiles::load();
        let active = 0;
        let (bands, preamp) = if let Some(p) = profiles.get(active) {
            (p.bands.clone(), p.preamp)
        } else {
            (Vec::new(), 0.0)
        };

        Self {
            running: true,
            config,
            client: Some(client),
            focused_block: FocusedBlock::Devices,
            mode: Mode::Normal,
            nodes: Vec::new(),
            nodes_selected: 0,
            pw_connected: false,
            daemon: DaemonConnection::Connected,
            command_input: Input::default(),
            eq: EqState {
                bands,
                ..EqState::default()
            },
            preamp,
            profiles,
            active_profile: active,
            last_key: None,
            peak_l: -60.0,
            peak_r: -60.0,
            cached_peak_l: 0.0,
            cached_peak_r: 0.0,
            null_sink: NullSinkState::NotLoaded,
            null_sink_missing: false,
            connected_devices: Vec::new(),
            filter_node_id: None,
            filter_state: FilterState::Unconnected,
            notification: None,
        }
    }

    fn client(&mut self) -> &mut DaemonClient {
        self.client
            .as_mut()
            .expect("DaemonClient required — not available in unit tests")
    }

    /// Attempt to reconnect after the daemon disconnected (e.g. `PipeWire`
    /// restart triggered a daemon self-restart).  Restores the active
    /// profile's bands, preamp, and bypass to the new daemon.
    pub fn reconnect(&mut self) -> AppResult<()> {
        let new_client = DaemonClient::connect()?;
        self.client = Some(new_client);
        self.full_sync()?;

        // Restore the active profile to the new daemon.
        if let Some(p) = self.profiles.get(self.active_profile) {
            self.eq.bands.clone_from(&p.bands);
            self.preamp = p.preamp;
        }
        let bands = self.eq.bands.clone();
        let preamp = self.preamp;
        let bypass = self.eq.bypass;
        self.client().set_bands(&bands)?;
        self.client().set_preamp(preamp)?;
        self.client().set_bypass(bypass)?;

        // Device links were destroyed with the old PipeWire graph.
        // The TUI shows them as disconnected; the user reconnects manually.
        self.connected_devices.clear();

        Ok(())
    }

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
            PushEvent::FilterStateChanged { state } => {
                self.filter_state = state;
            }
            PushEvent::StateChange { .. } => {}
            PushEvent::FilterReady { node_id } => {
                self.filter_node_id = Some(node_id);
            }
            PushEvent::NullSinkCreated { module_id } => {
                self.null_sink_missing = false;
                self.null_sink = NullSinkState::Loaded {
                    module_id,
                    has_source: false,
                };
            }
            PushEvent::NullSinkMissing => {
                self.null_sink_missing = true;
            }
            PushEvent::SourceActive { active } => {
                self.null_sink.set_has_source(active);
            }
            PushEvent::Error { message } => {
                tracing::error!(%message, "Daemon error");
            }
        }
    }

    /// Pull a full state snapshot from the daemon on initial connect.
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
        self.preamp = status.preamp;
        Ok(())
    }

    pub fn tick(&mut self) {
        if let Some((_, deadline)) = &self.notification
            && Instant::now() >= *deadline
        {
            self.notification = None;
        }

        let mut new_l = 20.0 * (self.cached_peak_l + 1e-7).log10();
        let mut new_r = 20.0 * (self.cached_peak_r + 1e-7).log10();

        new_l = new_l.clamp(-60.0, 0.0);
        new_r = new_r.clamp(-60.0, 0.0);

        let decay_speed = 0.8;

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

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.notification = Some((msg.into(), Instant::now() + Duration::from_secs(3)));
    }

    pub fn sync_bands(&mut self) -> AppResult<()> {
        if let Some(p) = self.profiles.get(self.active_profile) {
            // Prevent accidental modification of external profiles
            if p.path.is_some() {
                self.notify("Cannot save: Profile is read-only (linked to file)");
                return Ok(());
            }
        }

        if let Some(client) = &mut self.client {
            client.set_bands(&self.eq.bands)?;
            client.set_preamp(self.preamp)?;
        }
        if let Some(p) = self.profiles.get_mut(self.active_profile) {
            p.bands.clone_from(&self.eq.bands);
            p.preamp = self.preamp;
        }
        match profiles::save(&self.profiles) {
            Ok(()) => {
                self.notify(format!(
                    "Saved {} bands, preamp {:.1} dB",
                    self.eq.bands.len(),
                    self.preamp
                ));
            }
            Err(e) => {
                self.notify(format!("Failed to save: {e}"));
            }
        }
        Ok(())
    }

    pub fn sync_bypass(&mut self) -> AppResult<()> {
        if let Some(client) = &mut self.client {
            client.set_bypass(self.eq.bypass)
        } else {
            Ok(())
        }
    }

    pub fn load_peq(&mut self, path: &str) -> AppResult<()> {
        let preset = crate::autoeq::parse_peq(std::path::Path::new(path))?;
        self.preamp = preset.preamp;
        self.eq.bands = preset.bands;
        self.eq.band_selected = 0;
        let _ = self.sync_bands();
        self.notify(format!(
            "Loaded {} bands, preamp {:.1} dB",
            self.eq.bands.len(),
            self.preamp
        ));
        Ok(())
    }

    pub fn switch_profile(&mut self, dir: isize) {
        #[allow(clippy::cast_possible_wrap)]
        let count = self.profiles.len() as isize;
        if count == 0 {
            return;
        }

        #[allow(clippy::cast_possible_wrap)]
        let idx = (self.active_profile as isize + dir).rem_euclid(count) as usize;

        if let Some(p) = self.profiles.get(idx) {
            self.active_profile = idx;
            self.eq.bands.clone_from(&p.bands);
            self.preamp = p.preamp;
            self.eq.band_selected = 0;
        }
    }

    pub fn is_device_connected(&self, id: u32) -> bool {
        self.connected_devices.contains(&id)
    }

    pub fn toggle_device_connection(&mut self, id: u32) -> AppResult<()> {
        if self.filter_node_id.is_none() {
            return Ok(());
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
            daemon: DaemonConnection::Connected,
            command_input: Input::default(),
            eq: EqState::default(),
            preamp: 0.0,
            profiles: vec![Profile {
                name: "Test".into(),
                bands: vec![],
                preamp: 0.0,
                path: None,
            }],
            active_profile: 0,
            last_key: None,
            peak_l: -60.0,
            peak_r: -60.0,
            cached_peak_l: 0.0,
            cached_peak_r: 0.0,
            null_sink: NullSinkState::NotLoaded,
            null_sink_missing: false,
            connected_devices: Vec::new(),
            filter_node_id: None,
            filter_state: FilterState::Unconnected,
            notification: None,
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
        let app = App::new_test(config);

        assert!(app.running);
        assert_eq!(app.eq.band_selected, 0);
        assert_eq!(app.eq.column_selected, 1);
        assert_eq!(app.eq.cell_input.value(), "");
        assert!(!app.eq.bypass);

        let margin = 1e-3;
        assert!((app.peak_l - (-60.0_f32)).abs() < margin);
        assert!((app.peak_r - (-60.0_f32)).abs() < margin);
        assert!(!app.null_sink.is_loaded());
        assert_eq!(app.null_sink.module_id(), None);
        assert!(app.connected_devices.is_empty());
        assert_eq!(app.filter_node_id, None);
        assert!(!app.null_sink.has_source());
        assert_eq!(app.filter_state.to_string(), "UNCONNECTED");
    }

    #[test]
    fn switch_profile_updates_memory_only() {
        let config = std::sync::Arc::new(crate::config::Config::default());
        let mut app = App::new_test(config);

        // Ensure have 2 profiles
        app.profiles.push(crate::profiles::Profile {
            name: "Profile 2".into(),
            bands: vec![],
            preamp: 0.0,
            path: None,
        });

        // Setup profile 2 with different bands
        app.profiles[1].bands = vec![crate::state::EqBand {
            frequency: 500.0,
            gain: 3.0,
            q: 1.0,
            filter_type: crate::state::FilterType::Peak,
        }];

        // Switch to profile 2
        app.switch_profile(1);
        assert_eq!(app.active_profile, 1);
        assert_eq!(app.eq.bands.len(), 1);
        assert!((app.eq.bands[0].frequency - 500.0).abs() < f32::EPSILON);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_read_only_profile_guard() {
        let config = std::sync::Arc::new(crate::config::Config::default());
        let mut app = App::new_test(config);

        // Profile 0: Read-only (linked to file)
        app.profiles[0].path = Some("some_path.txt".into());
        app.profiles[0].preamp = -1.0;
        app.profiles[0].bands = vec![crate::state::EqBand {
            frequency: 100.0,
            gain: 0.0,
            q: 1.0,
            filter_type: crate::state::FilterType::Peak,
        }];

        // Profile 1: Normal
        app.profiles.push(crate::profiles::Profile {
            name: "Normal".into(),
            bands: vec![],
            preamp: 0.0,
            path: None,
        });

        // 1. Test sync_bands guard
        app.active_profile = 0;
        app.eq.bands = vec![]; // Try to clear bands
        app.preamp = 5.0; // Try to change preamp
        let result = app.sync_bands();
        assert!(result.is_ok());
        // Verify profile 0 was NOT updated
        assert_eq!(app.profiles[0].preamp, -1.0);
        assert_eq!(app.profiles[0].bands.len(), 1);

        // 2. Test switch_profile guard
        // Switch from 0 to 1
        app.switch_profile(1);
        assert_eq!(app.active_profile, 1);
        // Verify profile 0 was NOT updated when switching away
        assert_eq!(app.profiles[0].preamp, -1.0);
        assert_eq!(app.profiles[0].bands.len(), 1);

        // Modify something in profile 1
        app.preamp = 2.0;
        // Switch from 1 back to 0
        app.switch_profile(-1);
        assert_eq!(app.active_profile, 0);
        // Verify profile 1 was NOT updated when switching away
        // (auto-save on switch was removed — only explicit :w saves)
        assert_eq!(app.profiles[1].preamp, 0.0);
    }
}
