//! User configuration, read from `$XDG_CONFIG_HOME/macshell/config.toml`.

use std::path::PathBuf;
use std::{env, fs, io};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub dock: DockConfig,
    pub menubar: MenuBarConfig,
    /// Desktop entry id of the app that opens folders and the trash. Defaults to the system's
    /// default app for folders.
    pub file_manager: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    /// Desktop entry ids (or app ids) kept in the dock even when they aren't running.
    pub pinned: Vec<String>,
    /// Grow icons near the pointer.
    pub magnification: bool,
    /// Size of the icon right under the pointer, in logical pixels (icons are 48).
    pub magnified_size: f32,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            magnification: true,
            magnified_size: 80.0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MenuBarConfig {
    /// Icon name or path of the logo that opens the system menu. Defaults to the distro's.
    pub logo: Option<String>,
}

/// Rewrites `[dock] pinned` in the config file, keeping the rest of it (comments included).
pub fn save_pinned(pinned: &[String]) -> io::Result<()> {
    let path = config_path().ok_or_else(|| io::Error::other("no config directory"))?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };

    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    if !doc.contains_table("dock") {
        doc["dock"] = toml_edit::table();
    }
    doc["dock"]["pinned"] = toml_edit::value(pinned.iter().collect::<toml_edit::Array>());

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, doc.to_string())
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
