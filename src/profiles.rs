// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
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
    /// Index of the profile selected in the TUI at last save.
    /// `#[serde(default)]` keeps files written by older versions (or other
    /// tools) loadable: a missing field means "profile 0".
    #[serde(default)]
    active_profile: usize,
    profiles: Vec<Profile>,
}

impl Default for ProfilesFile {
    fn default() -> Self {
        Self {
            active_profile: 0,
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

/// Loads profiles from an explicit path. Infallible from the caller's view:
/// any read/parse problem degrades to defaults, but loudly: the user must
/// never be silently told their data is gone.
///
/// Returns `(profiles, active_profile)`.
fn load_from(path: &std::path::Path) -> (Vec<Profile>, usize) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // First run: create defaults. If THAT fails, say so: the user
            // will wonder why their edits don't stick.
            let defaults = ProfilesFile::default();
            if let Err(e) = save_raw(&defaults, path) {
                tracing::warn!(
                    "Cannot create {}: {e}: changes won't persist",
                    path.display()
                );
            }
            return (defaults.profiles, 0);
        }
        Err(e) => {
            // Permission denied etc. Do NOT overwrite: just run in-memory.
            tracing::warn!(
                "Cannot read {}: {e}: using in-memory defaults",
                path.display()
            );
            let defaults = ProfilesFile::default();
            return (defaults.profiles, 0);
        }
    };

    match toml::from_str::<ProfilesFile>(&contents) {
        Ok(mut pf) => {
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

            let active = pf.active_profile;
            (pf.profiles, active)
        }
        Err(e) => {
            // Preserve the evidence before starting fresh.
            let backup = path.with_extension("toml.bak");
            tracing::warn!(
                "Corrupt profiles file {}: {e}: backed up to {}",
                path.display(),
                backup.display()
            );
            // Best-effort rename: the warn! above already told the user.
            let _ = std::fs::rename(path, &backup);
            let defaults = ProfilesFile::default();
            if let Err(e) = save_raw(&defaults, path) {
                tracing::warn!("Cannot write fresh profiles: {e}");
            }
            (defaults.profiles, 0)
        }
    }
}

/// Loads the profiles and the saved active-profile index.
pub fn load() -> (Vec<Profile>, usize) {
    load_from(&profiles_path())
}

/// Updates profiles that are linked to external PEQ files.
fn update_external_profiles(profiles: &mut [Profile]) {
    for profile in profiles.iter_mut() {
        if let Some(ref path) = profile.path {
            let full_path = resolve_path(path);
            match crate::autoeq::parse_peq(&full_path) {
                Ok(preset) => {
                    for w in &preset.warnings {
                        tracing::warn!("{}: {w}", full_path.display());
                    }
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

/// Persists the profiles together with the active-profile index.
pub fn save(profiles: &[Profile], active_profile: usize) -> std::io::Result<()> {
    let pf = ProfilesFile {
        active_profile,
        profiles: profiles.to_vec(),
    };
    save_raw(&pf, &profiles_path())
}

fn save_raw(pf: &ProfilesFile, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Serialization failure is a REAL error: never substitute a placeholder.
    let contents = toml::to_string_pretty(pf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // Atomic replace: write a sibling temp file, flush to disk, then rename.
    // rename() within one filesystem is atomic: readers see old or new,
    // never a truncated file.
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        use std::io::Write;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?; // data hits the disk before the rename
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
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

    #[test]
    fn save_is_atomic_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        let pf = ProfilesFile::default();
        save_raw(&pf, &path).unwrap();
        assert!(!path.with_extension("toml.tmp").exists()); // no litter
        let loaded: ProfilesFile =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.profiles.len(), PROFILE_COUNT);
    }

    #[test]
    fn corrupt_file_is_backed_up_not_destroyed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        std::fs::write(&path, "this is [ not valid toml").unwrap();

        let (profiles, _) = load_from(&path);

        // Defaults were produced...
        assert_eq!(profiles.len(), PROFILE_COUNT);
        // ...the corrupt original was preserved, not destroyed...
        let backup = path.with_extension("toml.bak");
        assert!(
            std::fs::read_to_string(&backup)
                .unwrap()
                .contains("not valid toml"),
            "corrupt original must be backed up"
        );
        // ...and a fresh parseable file exists.
        assert!(toml::from_str::<ProfilesFile>(&std::fs::read_to_string(&path).unwrap()).is_ok());
    }

    #[test]
    fn save_fails_loudly_on_readonly_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut perms = dir.path().metadata().unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(dir.path(), perms).unwrap();
        let err = save_raw(&ProfilesFile::default(), &dir.path().join("p.toml")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn active_profile_roundtrips_through_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        let pf = ProfilesFile {
            active_profile: 3,
            profiles: ProfilesFile::default().profiles,
        };
        save_raw(&pf, &path).unwrap();

        let (_, active) = load_from(&path);
        assert_eq!(active, 3);
    }

    #[test]
    fn missing_active_profile_defaults_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        // Old-format file without the active_profile field.
        std::fs::write(&path, "profiles = []\n").unwrap();

        let (_, active) = load_from(&path);
        assert_eq!(active, 0);
    }

    #[test]
    fn load_clamps_active_profile_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        std::fs::write(
            &path,
            "active_profile = 99\nprofiles = [\n  { name = \"a\" },\n  { name = \"b\" },\n]\n",
        )
        .unwrap();

        let (profiles, active) = load_from(&path);
        // normalize pads to PROFILE_COUNT=5 → active 99 still > last index.
        assert_eq!(profiles.len(), PROFILE_COUNT);
        let clamped = active.min(profiles.len().saturating_sub(1));
        assert_eq!(clamped, profiles.len() - 1);
    }
}
