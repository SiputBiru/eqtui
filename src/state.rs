use std::fmt;

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

pub struct AppState {
    pub nodes: Vec<NodeInfo>,
    pub selected: usize,
    pub pw_connected: bool,
    pub status: String,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            selected: 0,
            pw_connected: false,
            status: "initializing...".into(),
        }
    }

    pub fn handle_event(&mut self, event: PwEvent) {
        match event {
            PwEvent::NodeList(list) => {
                self.nodes = list;
                if self.selected >= self.nodes.len() {
                    self.selected = self.nodes.len().saturating_sub(1);
                }
                self.status = format!("nodes: {}", self.nodes.len());
            }
            PwEvent::NodeAdded(node) => {
                self.nodes.push(node);
                self.status = format!("nodes: {}", self.nodes.len());
            }
            PwEvent::NodeRemoved(id) => {
                self.nodes.retain(|n| n.id != id);
                if self.selected >= self.nodes.len() {
                    self.selected = self.nodes.len().saturating_sub(1);
                }
                self.status = format!("nodes: {}", self.nodes.len());
            }
            PwEvent::Connected => {
                self.pw_connected = true;
            }
            PwEvent::Error(msg) => {
                self.status = format!("error: {msg}");
            }
        }
    }

    pub fn select_next(&mut self) {
        if !self.nodes.is_empty() {
            self.selected = (self.selected + 1) % self.nodes.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.nodes.is_empty() {
            self.selected = self.selected.checked_sub(1)
                .unwrap_or(self.nodes.len() - 1);
        }
    }
}
