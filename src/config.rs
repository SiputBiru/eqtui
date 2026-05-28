// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::fs;
use std::path::PathBuf;

use ratatui::layout::Flex;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_layout")]
    pub layout: Flex,
    #[serde(default)]
    pub keys: Keys,
}

#[derive(Deserialize, Debug, Default)]
pub struct Keys {
    #[serde(default)]
    pub normal: NormalKeys,
    #[serde(default)]
    pub insert: InsertKeys,
}

#[derive(Deserialize, Debug)]
pub struct NormalKeys {
    #[serde(default = "NormalKeys::default_add_band")]
    pub add_band: char,
    #[serde(default = "NormalKeys::default_delete_band")]
    pub delete_band: char,
    #[serde(default = "NormalKeys::default_insert_mode")]
    pub insert_mode: char,
    #[serde(default = "NormalKeys::default_command_mode")]
    pub command_mode: char,
    #[serde(default = "NormalKeys::default_visual_mode")]
    pub visual_mode: char,
    #[serde(default = "NormalKeys::default_toggle_bypass")]
    pub toggle_bypass: char,
}

#[derive(Deserialize, Debug)]
pub struct InsertKeys {
    #[serde(default = "InsertKeys::default_confirm")]
    pub confirm: char,
    #[serde(default = "InsertKeys::default_cancel")]
    pub cancel: char,
    #[serde(default = "InsertKeys::default_bump_up")]
    pub bump_up: char,
    #[serde(default = "InsertKeys::default_bump_down")]
    pub bump_down: char,
}

impl NormalKeys {
    pub const DEFAULT_ADD_BAND: char = 'a';
    pub const DEFAULT_DELETE_BAND: char = 'd';
    pub const DEFAULT_INSERT_MODE: char = 'i';
    pub const DEFAULT_COMMAND_MODE: char = ':';
    pub const DEFAULT_VISUAL_MODE: char = 'v';
    pub const DEFAULT_TOGGLE_BYPASS: char = 'b';

    fn default_add_band() -> char {
        Self::DEFAULT_ADD_BAND
    }
    fn default_delete_band() -> char {
        Self::DEFAULT_DELETE_BAND
    }
    fn default_insert_mode() -> char {
        Self::DEFAULT_INSERT_MODE
    }
    fn default_command_mode() -> char {
        Self::DEFAULT_COMMAND_MODE
    }
    fn default_visual_mode() -> char {
        Self::DEFAULT_VISUAL_MODE
    }
    fn default_toggle_bypass() -> char {
        Self::DEFAULT_TOGGLE_BYPASS
    }
}

impl InsertKeys {
    pub const DEFAULT_CONFIRM: char = '\n';
    pub const DEFAULT_CANCEL: char = '\x1b';
    pub const DEFAULT_BUMP_UP: char = '+';
    pub const DEFAULT_BUMP_DOWN: char = '-';

    fn default_confirm() -> char {
        Self::DEFAULT_CONFIRM
    }
    fn default_cancel() -> char {
        Self::DEFAULT_CANCEL
    }
    fn default_bump_up() -> char {
        Self::DEFAULT_BUMP_UP
    }
    fn default_bump_down() -> char {
        Self::DEFAULT_BUMP_DOWN
    }
}

impl Default for NormalKeys {
    fn default() -> Self {
        Self {
            add_band: Self::DEFAULT_ADD_BAND,
            delete_band: Self::DEFAULT_DELETE_BAND,
            insert_mode: Self::DEFAULT_INSERT_MODE,
            command_mode: Self::DEFAULT_COMMAND_MODE,
            visual_mode: Self::DEFAULT_VISUAL_MODE,
            toggle_bypass: Self::DEFAULT_TOGGLE_BYPASS,
        }
    }
}

impl Default for InsertKeys {
    fn default() -> Self {
        Self {
            confirm: Self::DEFAULT_CONFIRM,
            cancel: Self::DEFAULT_CANCEL,
            bump_up: Self::DEFAULT_BUMP_UP,
            bump_down: Self::DEFAULT_BUMP_DOWN,
        }
    }
}

fn default_layout() -> Flex {
    Flex::SpaceBetween
}

impl Config {
    pub fn new(config_path: Option<PathBuf>) -> Self {
        let path = config_path.unwrap_or_else(|| {
            dirs::config_dir()
                .expect("XDG config directory not found — set $XDG_CONFIG_HOME or $HOME")
                .join("eqtui")
                .join("config.toml")
        });

        match fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: default_layout(),
            keys: Keys::default(),
        }
    }
}
