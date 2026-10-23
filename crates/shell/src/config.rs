//! User configuration, read from `$XDG_CONFIG_HOME/macshell/config.toml`.

use std::path::PathBuf;
use std::{env, fs, io};

use serde::Deserialize;

use crate::material::Style;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub dock: DockConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    /// Desktop entry ids (or app ids) kept in the dock even when they aren't running.
    pub pinned: Vec<String>,
    /// Glass style behind the dock, when the compositor offers glass.
    pub glass: Style,
    /// Grow icons near the pointer.
    pub magnification: bool,
    /// Size of the icon right under the pointer, in logical pixels (icons are 48).
    pub magnified_size: f32,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            glass: Style::default(),
            magnification: true,
            magnified_size: 80.0,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };

        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                log::error!("invalid {}: {err}", path.display());
                Self::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                log::error!("can't read {}: {err}", path.display());
                Self::default()
            }
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("macshell/config.toml"))
}
