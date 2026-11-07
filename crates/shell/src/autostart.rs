//! "Open at Login": apps in the XDG autostart folder. The shell starts them itself when the session
//! begins, since this session doesn't run the system's autostart, which is mostly for Plasma.

use std::path::PathBuf;
use std::{env, fs};

use freedesktop_desktop_entry::DesktopEntry;

use crate::apps;

fn folder() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(env::var_os("HOME")?).join(".config")))
        .map(|config| config.join("autostart"))
}

fn entry(app_id: &str) -> Option<PathBuf> {
    Some(folder()?.join(format!("{app_id}.desktop")))
}

pub fn is_enabled(app_id: &str) -> bool {
    entry(app_id).is_some_and(|path| path.exists())
}

/// Turns it on with a copy of the app's desktop entry, or off.
pub fn set_enabled(app_id: &str, source: Option<PathBuf>, enabled: bool) {
    let result = (|| {
        let path = entry(app_id).ok_or(std::io::ErrorKind::NotFound)?;
        if enabled {
            let source = source.ok_or(std::io::ErrorKind::NotFound)?;
            if let Some(folder) = path.parent() {
                fs::create_dir_all(folder)?;
            }
            fs::write(&path, fs::read(source)?)
        } else {
            fs::remove_file(&path)
        }
    })();
    if let Err(err) = result {
        log::warn!("can't change whether {app_id} opens at login: {err}");
    }
}

/// Starts what's in the folder, once per session: the shell can restart during one.
pub fn run_once() {
    let session = env::var("XDG_SESSION_ID").unwrap_or_default();
    let Some(runtime) = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return;
    };
    let marker = runtime.join(format!("makshell-autostarted-{session}"));
    if marker.exists() || fs::write(&marker, b"").is_err() {
        return;
    }
    let Some(entries) = folder().and_then(|folder| fs::read_dir(folder).ok()) else {
        return;
    };
    let locales = freedesktop_desktop_entry::get_languages_from_env();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path
            .extension()
            .is_none_or(|extension| extension != "desktop")
        {
            continue;
        }
        let Ok(entry) = DesktopEntry::from_path(path, Some(&locales)) else {
            continue;
        };
        if entry.desktop_entry("Hidden") == Some("true") {
            continue;
        }
        if let Ok(command) = entry.parse_exec() {
            apps::spawn(command);
        }
    }
}
