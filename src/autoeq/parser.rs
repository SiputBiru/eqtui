// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::fs;
use std::path::Path;

use crate::state::{EqBand, FilterType};

#[derive(Debug, Clone, PartialEq)]
pub struct PeqPreset {
    pub preamp: f32,
    pub bands: Vec<EqBand>,
}

#[derive(Debug)]
pub enum PeqError {
    Io(std::io::Error),
    NoFilters,
    InvalidPreamp { line: usize, raw: String },
}

impl std::fmt::Display for PeqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeqError::Io(e) => write!(f, "Failed to read PEQ file: {e}"),
            PeqError::NoFilters => write!(f, "No filters found in PEQ file"),
            PeqError::InvalidPreamp { line, raw } => {
                write!(f, "Invalid preamp value at line {line}: {raw}")
            }
        }
    }
}

impl std::error::Error for PeqError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PeqError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PeqError {
    fn from(e: std::io::Error) -> Self {
        PeqError::Io(e)
    }
}

pub fn parse_peq(path: &Path) -> Result<PeqPreset, PeqError> {
    parse_peq_str(&fs::read_to_string(path)?)
}

/// Parse a PEQ filter line of the form:
/// `Filter <N>: ON <TYPE> Fc <FREQ> Hz Gain <GAIN> dB Q <Q>`
fn parse_filter_line(trimmed: &str) -> Option<EqBand> {
    let rest = trimmed.split(": ON ").nth(1)?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 9 {
        return None;
    }

    let filter_type = match parts[0] {
        "PK" => FilterType::Peak,
        "LSC" => FilterType::LowShelf,
        "HSC" => FilterType::HighShelf,
        _ => return None,
    };

    let frequency = parts[2].parse().unwrap_or(1000.0);
    let gain = parts[5].parse().unwrap_or(0.0);
    let q = parts[8].parse().unwrap_or(1.0);

    Some(EqBand {
        frequency,
        gain,
        q,
        filter_type,
    })
}

pub fn parse_peq_str(input: &str) -> Result<PeqPreset, PeqError> {
    let mut preamp: Option<f32> = None;
    let mut bands = Vec::new();

    for (lineno, line) in input.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse "Preamp: <val> dB"
        if let Some(val) = trimmed.strip_prefix("Preamp:") {
            let val = val.trim();
            let raw = val.strip_suffix("dB").map_or(val, |s| s.trim()).to_string();
            let gain: f32 = raw.parse().map_err(|_| PeqError::InvalidPreamp {
                line: lineno + 1,
                raw,
            })?;
            preamp = Some(gain);
            continue;
        }

        // Parse "Filter <N>: ON <TYPE> Fc <FREQ> Hz Gain <GAIN> dB Q <Q>"
        if trimmed.contains(": ON ")
            && let Some(band) = parse_filter_line(trimmed)
        {
            bands.push(band);
        }
    }

    if bands.is_empty() {
        return Err(PeqError::NoFilters);
    }

    Ok(PeqPreset {
        preamp: preamp.unwrap_or(0.0),
        bands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_peq() {
        let input = "\
Preamp: -6.0 dB
Filter 1: ON PK Fc 32 Hz Gain 2.5 dB Q 0.71
Filter 2: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.71
Filter 3: ON HSC Fc 10000 Hz Gain -2.0 dB Q 0.70
";
        let preset = parse_peq_str(input).unwrap();
        assert!((preset.preamp - (-6.0)).abs() < 0.01);
        assert_eq!(preset.bands.len(), 3);

        assert_eq!(preset.bands[0].filter_type, FilterType::Peak);
        assert!((preset.bands[0].frequency - 32.0).abs() < 0.1);
        assert!((preset.bands[0].gain - 2.5).abs() < 0.01);
        assert!((preset.bands[0].q - 0.71).abs() < 0.01);

        assert_eq!(preset.bands[1].filter_type, FilterType::LowShelf);
        assert!((preset.bands[1].frequency - 105.0).abs() < 0.1);

        assert_eq!(preset.bands[2].filter_type, FilterType::HighShelf);
        assert!((preset.bands[2].gain - (-2.0)).abs() < 0.01);
    }

    #[test]
    fn parse_with_comments_and_blanks() {
        let input = "\
# My HD600 preset
Preamp: -4.5 dB

Filter 1: ON PK Fc 1000 Hz Gain 6.0 dB Q 1.2
# Some comment
Filter 2: ON PK Fc 3000 Hz Gain -3.0 dB Q 2.0
";
        let preset = parse_peq_str(input).unwrap();
        assert!((preset.preamp - (-4.5)).abs() < 0.01);
        assert_eq!(preset.bands.len(), 2);
    }

    #[test]
    fn no_filters_returns_error() {
        let err = parse_peq_str("Preamp: -6.0 dB\n").unwrap_err();
        assert!(matches!(err, PeqError::NoFilters));
    }

    #[test]
    fn skips_unknown_filter_types() {
        let input = "\
Preamp: -3.0 dB
Filter 1: ON PK Fc 500 Hz Gain 1.0 dB Q 0.5
Filter 2: ON XYZ Fc 800 Hz Gain 2.0 dB Q 1.0
Filter 3: ON PK Fc 2000 Hz Gain -1.0 dB Q 0.8
";
        let preset = parse_peq_str(input).unwrap();
        assert_eq!(preset.bands.len(), 2);
    }
}
