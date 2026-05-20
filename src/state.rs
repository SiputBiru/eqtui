use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DeviceClass {
    Speaker,
    Headphone,
    Input,
}

impl DeviceClass {
    pub fn label(&self) -> &str {
        match self {
            DeviceClass::Speaker => "Speaker",
            DeviceClass::Headphone => "Headphone",
            DeviceClass::Input => "Input",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            DeviceClass::Speaker => "\u{f04c3} ",
            DeviceClass::Headphone => "\u{f025} ",
            DeviceClass::Input => "\u{ed03} ",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub class: DeviceClass,
}

impl NodeInfo {
    pub fn class_label(&self) -> &str {
        self.class.label()
    }
}

impl fmt::Display for NodeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.description.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}", self.description)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NullSinkState {
    NotLoaded,
    Loaded {
        module_id: u32,
        has_source: bool,
    },
}

impl NullSinkState {
    pub fn is_loaded(&self) -> bool {
        matches!(self, NullSinkState::Loaded { .. })
    }

    pub fn module_id(&self) -> Option<u32> {
        match self {
            NullSinkState::Loaded { module_id, .. } => Some(*module_id),
            NullSinkState::NotLoaded => None,
        }
    }

    pub fn has_source(&self) -> bool {
        match self {
            NullSinkState::Loaded { has_source, .. } => *has_source,
            NullSinkState::NotLoaded => false,
        }
    }

    pub fn set_has_source(&mut self, has_source: bool) {
        if let NullSinkState::Loaded { has_source: hs, .. } = self {
            *hs = has_source;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterState {
    Unconnected,
    Connecting,
    Paused,
    Streaming,
    Error(String),
}

impl fmt::Display for FilterState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterState::Unconnected => write!(f, "UNCONNECTED"),
            FilterState::Connecting => write!(f, "CONNECTING"),
            FilterState::Paused => write!(f, "PAUSED"),
            FilterState::Streaming => write!(f, "STREAMING"),
            FilterState::Error(_) => write!(f, "ERROR"),
        }
    }
}

pub enum PwEvent {
    NodeList(Vec<NodeInfo>),
    NodeAdded(NodeInfo),
    NodeRemoved(u32),
    Connected,
    FilterStateChanged(FilterState),
    /// Sent once when the DSP filter node ID is known. The TUI needs this
    /// to construct `ConnectDevice` / `DisconnectDevice` commands.
    FilterReady {
        node_id: u32,
    },
    NullSinkCreated {
        module_id: u32,
    },
    /// Whether an audio source is currently linked to the null-sink's
    /// `playback_FL` / `playback_FR` input ports.
    NullSinkInputState {
        has_source: bool,
    },
    NullSinkError(String),
    Error(String),
}

pub enum PwCommand {
    Terminate,
    ConnectDevice { filter_id: u32, node_id: u32 },
    DisconnectDevice { filter_id: u32, node_id: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqBand {
    pub frequency: f32,
    pub gain: f32,
    pub q: f32,
    pub filter_type: FilterType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
}
