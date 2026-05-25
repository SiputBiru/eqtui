// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::state::EqBand;

pub const PROFILE_COUNT: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub bands: Vec<EqBand>,
    #[serde(default)]
    pub preamp: f32,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfilesFile {
    profiles: Vec<Profile>,
}

impl Default for ProfilesFile {
    fn default() -> Self {
        Self {
            profiles: (1..=PROFILE_COUNT)
                .map(|i| Profile {
                    name: format!("Profile {i}"),
                    bands: Vec::new(),
                    preamp: 0.0,
                    path: None,
                })
                .collect(),
        }
    }
}

pub fn load() -> Vec<Profile> {
    let path = profiles_path();
    let Ok(contents) = std::fs::read_to_string(&path) else {
        let defaults = ProfilesFile::default();
        let _ = save_raw(&defaults, &path);
        return defaults.profiles;
    };

    if let Ok(mut pf) = toml::from_str::<ProfilesFile>(&contents) {
        // Enforces exactly PROFILE_COUNT profiles.
        while pf.profiles.len() < PROFILE_COUNT {
            pf.profiles.push(Profile {
                name: format!("Profile {}", pf.profiles.len() + 1),
                bands: Vec::new(),
                preamp: 0.0,
                path: None,
            });
        }
        pf.profiles.truncate(PROFILE_COUNT);

        update_external_profiles(&mut pf.profiles);

        pf.profiles
    } else {
        let mut defaults = ProfilesFile::default();
        update_external_profiles(&mut defaults.profiles);
        let _ = save_raw(&defaults, &path);
        defaults.profiles
    }
}

/// Updates profiles that are linked to external PEQ files.
fn update_external_profiles(profiles: &mut [Profile]) {
    for profile in profiles.iter_mut() {
        if let Some(ref path) = profile.path {
            let full_path = resolve_path(path);
            match crate::autoeq::parse_peq(&full_path) {
                Ok(preset) => {
                    profile.bands = preset.bands;
                    profile.preamp = preset.preamp;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load external profile from {}: {}",
                        full_path.display(),
                        e
                    );
                }
            }
        }
    }
}

/// Resolves a profile path, supporting the `@` prefix for portability.
///
/// If a path starts with `@`, it is resolved relative to the directory
/// containing the profiles.toml file (usually ~/.config/eqtui/).
pub fn resolve_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix('@') {
        let mut base = profiles_path();
        base.pop(); // Remove "profiles.toml"
        base.join(stripped)
    } else {
        PathBuf::from(path)
    }
}

pub fn save(profiles: &[Profile]) {
    let pf = ProfilesFile {
        profiles: profiles.to_vec(),
    };
    let _ = save_raw(&pf, &profiles_path());
}

fn save_raw(pf: &ProfilesFile, path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents =
        toml::to_string_pretty(pf).unwrap_or_else(|_| String::from("# Failed to serialize\n"));
    std::fs::write(path, contents)
}

fn profiles_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("eqtui")
        .join("profiles.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_deserialization_with_path() {
        let toml_str = r#"
            name = "Test Profile"
            path = "@/path/to/profile.txt"
            bands = []
            preamp = 0.0
        "#;
        let profile: Profile = toml::from_str(toml_str).unwrap();
        assert_eq!(profile.path, Some("@/path/to/profile.txt".to_string()));
    }

    #[test]
    fn test_profile_deserialization_without_path() {
        let toml_str = r#"
            name = "Test Profile"
            bands = []
            preamp = 0.0
        "#;
        let profile: Profile = toml::from_str(toml_str).unwrap();
        assert_eq!(profile.path, None);
    }

    #[test]
    fn test_load_with_external_file() {
        let peq_path = std::path::PathBuf::from("test_load.txt");
        std::fs::write(
            &peq_path,
            "Preamp: -5.0 dB\nFilter 1: ON PK Fc 100 Hz Gain 2.0 dB Q 1.0\n",
        )
        .unwrap();

        let mut profile = Profile {
            name: "External".into(),
            bands: vec![],
            preamp: 0.0,
            path: Some(peq_path.to_str().unwrap().to_string()),
        };

        // Mock the logic inside load()
        let full_path = resolve_path(profile.path.as_ref().unwrap());
        let preset = crate::autoeq::parse_peq(&full_path).unwrap();
        profile.bands = preset.bands;
        profile.preamp = preset.preamp;

        std::fs::remove_file(&peq_path).unwrap();

        assert!(
            (profile.preamp - (-5.0)).abs() < f32::EPSILON,
            "preamp mismatch"
        );
        assert_eq!(profile.bands.len(), 1);
        assert!(
            (profile.bands[0].frequency - 100.0).abs() < f32::EPSILON,
            "frequency mismatch"
        );
    }
}
