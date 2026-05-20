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
    #[serde(default = "default_add_band")]
    pub add_band: char,
    #[serde(default = "default_delete_band")]
    pub delete_band: char,
    #[serde(default = "default_insert_mode")]
    pub insert_mode: char,
    #[serde(default = "default_command_mode")]
    pub command_mode: char,
    #[serde(default = "default_visual_mode")]
    pub visual_mode: char,
    #[serde(default = "default_toggle_bypass")]
    pub toggle_bypass: char,
}

#[derive(Deserialize, Debug)]
pub struct InsertKeys {
    #[serde(default = "default_confirm")]
    pub confirm: char,
    #[serde(default = "default_cancel")]
    pub cancel: char,
    #[serde(default = "default_bump_up")]
    pub bump_up: char,
    #[serde(default = "default_bump_down")]
    pub bump_down: char,
}

impl Default for NormalKeys {
    fn default() -> Self {
        Self {
            add_band: 'a',
            delete_band: 'd',
            insert_mode: 'i',
            command_mode: ':',
            visual_mode: 'v',
            toggle_bypass: 'b',
        }
    }
}

impl Default for InsertKeys {
    fn default() -> Self {
        Self {
            confirm: '\n',  // Enter
            cancel: '\x1b', // Esc
            bump_up: '+',
            bump_down: '-',
        }
    }
}

fn default_layout() -> Flex {
    Flex::SpaceBetween
}

fn default_add_band() -> char {
    'a'
}
fn default_delete_band() -> char {
    'd'
}
fn default_insert_mode() -> char {
    'i'
}
fn default_command_mode() -> char {
    ':'
}
fn default_visual_mode() -> char {
    'v'
}
fn default_toggle_bypass() -> char {
    'b'
}

fn default_confirm() -> char {
    '\n'
}
fn default_cancel() -> char {
    '\x1b'
}
fn default_bump_up() -> char {
    '+'
}
fn default_bump_down() -> char {
    '-'
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
