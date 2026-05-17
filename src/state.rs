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

pub enum PwEvent {
    NodeList(Vec<NodeInfo>),
    #[allow(dead_code)]
    NodeAdded(NodeInfo),
    #[allow(dead_code)]
    NodeRemoved(u32),
    Connected,
    NullSinkCreated { module_id: u32 },
    NullSinkError(String),
    Error(String),
}

pub enum PwCommand {
    Terminate,
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
