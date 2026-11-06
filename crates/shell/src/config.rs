//! User configuration, read from `$XDG_CONFIG_HOME/macshell/config.toml`.

use std::path::PathBuf;
use std::{env, fs, io};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub desktop: DesktopConfig,
    pub dock: DockConfig,
    pub menubar: MenuBarConfig,
    /// Desktop entry id of the app that opens folders and the trash. Defaults to the system's
    /// default app for folders.
    pub file_manager: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    /// Path of the wallpaper image, which may start with `~/`. Without one, the shell leaves the
    /// background to whatever else draws it.
    pub wallpaper: Option<String>,
    /// Keep widgets as glass even when the desktop has focus, instead of in color.
    pub always_glassy_widgets: bool,
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
    edit(|doc| {
        if !doc.contains_table("dock") {
            doc["dock"] = toml_edit::table();
        }
        doc["dock"]["pinned"] = toml_edit::value(pinned.iter().collect::<toml_edit::Array>());
    })
}

/// Rewrites `[desktop] wallpaper` in the config file, the same way.
pub fn save_wallpaper(wallpaper: &std::path::Path) -> io::Result<()> {
    edit(|doc| {
        if !doc.contains_table("desktop") {
            doc["desktop"] = toml_edit::table();
        }
        doc["desktop"]["wallpaper"] = toml_edit::value(wallpaper.to_string_lossy().as_ref());
    })
}

/// Rewrites `[dock] magnification`, the same way.
pub fn save_magnification(magnification: bool) -> io::Result<()> {
    save("dock", "magnification", toml_edit::value(magnification))
}

/// Rewrites `[dock] magnified_size`, the same way.
pub fn save_magnified_size(size: f32) -> io::Result<()> {
    save("dock", "magnified_size", toml_edit::value(f64::from(size)))
}

/// Rewrites `[desktop] always_glassy_widgets`, the same way.
pub fn save_always_glassy_widgets(always: bool) -> io::Result<()> {
    save("desktop", "always_glassy_widgets", toml_edit::value(always))
}

fn save(table: &str, key: &str, value: toml_edit::Item) -> io::Result<()> {
    edit(|doc| {
        if !doc.contains_table(table) {
            doc[table] = toml_edit::table();
        }
        doc[table][key] = value;
    })
}

fn edit(change: impl FnOnce(&mut toml_edit::DocumentMut)) -> io::Result<()> {
    let path = config_path().ok_or_else(|| io::Error::other("no config directory"))?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };

    let mut doc: toml_edit::DocumentMut = text.parse().map_err(io::Error::other)?;
    change(&mut doc);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, doc.to_string())
}

/// The config file as it is, or nothing if there isn't one.
pub fn read_text() -> io::Result<String> {
    let path = config_path().ok_or_else(|| io::Error::other("no config directory"))?;
    match fs::read_to_string(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

/// Replaces the config file.
pub fn write_text(text: &str) -> io::Result<()> {
    let path = config_path().ok_or_else(|| io::Error::other("no config directory"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, text)
}

/// A file next to the config file.
pub fn sibling(name: &str) -> Option<PathBuf> {
    Some(config_path()?.parent()?.join(name))
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
