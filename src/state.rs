use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub class: String,
}

impl NodeInfo {
    pub fn class_label(&self) -> &str {
        if self.class == "Audio/Sink" {
            "Speaker"
        } else if self.class == "Audio/Source" {
            "Microphone"
        } else {
            &self.class
        }
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

pub enum PwEvent {
    NodeList(Vec<NodeInfo>),
    #[allow(dead_code)]
    NodeAdded(NodeInfo),
    #[allow(dead_code)]
    NodeRemoved(u32),
    Connected,
    Error(String),
}

pub enum PwCommand {
    Terminate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    pub frequency: f32,
    pub gain: f32,
    pub q: f32,
    pub filter_type: FilterType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
}
