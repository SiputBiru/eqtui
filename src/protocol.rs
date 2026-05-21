use serde::{Deserialize, Serialize};

use crate::state::{EqBand, FilterState, NodeInfo, NullSinkState};

// ── Client → Daemon ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum Request {
    GetStatus,
    SetBands { bands: Vec<EqBand> },
    SetPreamp { gain: f32 },
    SetBypass { bypass: bool },
    ConnectDevice { node_id: u32 },
    DisconnectDevice { node_id: u32 },
    LoadPeq { path: String },
    Shutdown,
}

// ── Daemon → Client (response to a request) ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<DaemonStatus>,
}

// ── Daemon → All Clients (unsolicited push) ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum PushEvent {
    PeakUpdate { l: f32, r: f32 },
    NodeList { nodes: Vec<NodeInfo> },
    FilterReady { node_id: u32 },
    StateChange { state: String },
    NullSinkCreated { module_id: u32 },
    SourceActive { active: bool },
    Error { message: String },
}

// ── Full daemon state snapshot ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub bands: Vec<EqBand>,
    pub bypass: bool,
    pub preamp: f32,
    pub nodes: Vec<NodeInfo>,
    pub pw_connected: bool,
    pub filter_state: FilterState,
    pub null_sink: NullSinkState,
    pub filter_node_id: Option<u32>,
    pub connected_devices: Vec<u32>,
}
