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
        // Ensure we always have exactly PROFILE_COUNT profiles.
        while pf.profiles.len() < PROFILE_COUNT {
            pf.profiles.push(Profile {
                name: format!("Profile {}", pf.profiles.len() + 1),
                bands: Vec::new(),
                preamp: 0.0,
            });
        }
        pf.profiles.truncate(PROFILE_COUNT);
        pf.profiles
    } else {
        let defaults = ProfilesFile::default();
        let _ = save_raw(&defaults, &path);
        defaults.profiles
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
    let contents = toml::to_string_pretty(pf)
        .unwrap_or_else(|_| String::from("# Failed to serialize\n"));
    std::fs::write(path, contents)
}

fn profiles_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("eqtui")
        .join("profiles.toml")
}
