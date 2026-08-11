// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum NullSinkState {
    #[default]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum FilterState {
    #[default]
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
    /// Sent when `pw-link -I` failed — the null-sink input state
    /// could not be determined (e.g. binary missing, `PipeWire` down).
    NullSinkInputUnknown,
    NullSinkError(String),
    Error(String),
}

pub enum PwCommand {
    Terminate,
    ConnectDevice { filter_id: u32, node_id: u32 },
    DisconnectDevice { filter_id: u32, node_id: u32 },
    UpdateEq { bands: Vec<EqBand> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqBand {
    pub frequency: f32,
    pub gain: f32,
    pub q: f32,
    pub filter_type: FilterType,
}

/// Maximum number of EQ bands per request / preset.
pub const MAX_BANDS: usize = 31;
/// Biquad stability window at 48 kHz (Nyquist = 24 kHz).
pub const MIN_FREQ_HZ: f32 = 10.0;
pub const MAX_FREQ_HZ: f32 = 24_000.0;
/// Per-band gain. `AutoEQ` output rarely exceeds ±20 dB; 40 is generous headroom.
pub const MAX_ABS_GAIN_DB: f32 = 40.0;
/// Q outside this window is a typo, not a filter.
pub const MIN_Q: f32 = 0.1;
pub const MAX_Q: f32 = 10.0;
/// Preamp headroom. Beyond ±40 dB something is wrong with the input.
pub const MAX_ABS_PREAMP_DB: f32 = 40.0;

impl EqBand {
    /// Validate against DSP-safe ranges. Returns a human-readable reason.
    pub fn validate(&self) -> Result<(), String> {
        if !self.frequency.is_finite() || !(MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&self.frequency) {
            return Err(format!(
                "frequency {} Hz out of range {MIN_FREQ_HZ}..={MAX_FREQ_HZ}",
                self.frequency
            ));
        }
        if !self.gain.is_finite() || self.gain.abs() > MAX_ABS_GAIN_DB {
            return Err(format!(
                "gain {} dB out of range ±{MAX_ABS_GAIN_DB}",
                self.gain
            ));
        }
        if !self.q.is_finite() || !(MIN_Q..=MAX_Q).contains(&self.q) {
            return Err(format!("Q {} out of range {MIN_Q}..={MAX_Q}", self.q));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_non_finite_and_out_of_range() {
        let ok = EqBand {
            frequency: 1000.0,
            gain: 3.0,
            q: 1.0,
            filter_type: FilterType::Peak,
        };
        assert!(ok.validate().is_ok());

        for bad in [f32::NAN, f32::INFINITY, 5.0, 30_000.0] {
            assert!(
                EqBand {
                    frequency: bad,
                    ..ok.clone()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            EqBand {
                gain: f32::NAN,
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            EqBand {
                gain: 41.0,
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            EqBand {
                q: 0.05,
                ..ok.clone()
            }
            .validate()
            .is_err()
        );

        // Boundaries are inclusive:
        assert!(
            EqBand {
                frequency: MIN_FREQ_HZ,
                ..ok.clone()
            }
            .validate()
            .is_ok()
        );
        assert!(
            EqBand {
                frequency: MAX_FREQ_HZ,
                ..ok.clone()
            }
            .validate()
            .is_ok()
        );
        assert!(
            EqBand {
                gain: MAX_ABS_GAIN_DB,
                ..ok.clone()
            }
            .validate()
            .is_ok()
        );
        assert!(
            EqBand {
                q: MIN_Q,
                ..ok.clone()
            }
            .validate()
            .is_ok()
        );
        assert!(
            EqBand {
                q: MAX_Q,
                ..ok.clone()
            }
            .validate()
            .is_ok()
        );
    }
}
