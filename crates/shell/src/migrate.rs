//! Moves what was kept under the name this had before, macshell, to makshell. It runs before
//! anything reads its config, and does nothing once there's nothing under the old name.

use std::path::{Path, PathBuf};
use std::{env, fs};

const OLD: &str = "macshell";
const NEW: &str = "makshell";

/// A base directory, as XDG says, falling back on the one in the home folder.
fn base(variable: &str, home: &Path, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(fallback))
}

pub fn from_old_name() {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let config = base("XDG_CONFIG_HOME", &home, ".config");
    let state = base("XDG_STATE_HOME", &home, ".local/state");
    let data = base("XDG_DATA_HOME", &home, ".local/share");

    for dir in [&config, &state, &data] {
        move_path(&dir.join(OLD), &dir.join(NEW));
    }
    // The Settings app's ID, among the dock's apps.
    for file in [
        config.join(NEW).join("config.toml"),
        state.join(NEW).join("recent-apps"),
        state.join(NEW).join("desktop.toml"),
    ] {
        replace_in(&file, "macshell-settings", "makshell-settings");
    }

    // What Match Apps gave other apps, which is found by its name.
    for version in ["gtk-3.0", "gtk-4.0"] {
        let dir = config.join(version);
        if move_path(&dir.join("macshell.css"), &dir.join("makshell.css")) {
            replace_in(&dir.join("gtk.css"), "macshell.css", "makshell.css");
        }
    }
    let alacritty = config.join("alacritty");
    if move_path(
        &alacritty.join("themes/macshell.toml"),
        &alacritty.join("themes/makshell.toml"),
    ) {
        replace_in(
            &alacritty.join("alacritty.toml"),
            "themes/macshell.toml",
            "themes/makshell.toml",
        );
    }
    if move_path(
        &data.join("color-schemes/Macshell.colors"),
        &data.join("color-schemes/Makshell.colors"),
    ) {
        let globals = config.join("kdeglobals");
        replace_in(&globals, "ColorScheme=Macshell\n", "ColorScheme=Makshell\n");
        replace_in(&globals, "Name=Macshell\n", "Name=Makshell\n");
    }
    let telegram = config.join("telegram-desktop/themes");
    move_path(
        &telegram.join("macshell.tdesktop-theme"),
        &telegram.join("makshell.tdesktop-theme"),
    );
}

/// Moves something to its new name, unless there's nothing to move or something's there already.
/// Returns whether it moved.
fn move_path(from: &Path, to: &Path) -> bool {
    if fs::symlink_metadata(from).is_err() || fs::symlink_metadata(to).is_ok() {
        return false;
    }
    match fs::rename(from, to) {
        Ok(()) => {
            log::info!("moved {} to {}", from.display(), to.display());
            true
        }
        Err(err) => {
            log::warn!("can't move {} to {}: {err}", from.display(), to.display());
            false
        }
    }
}

/// Changes text in a file of the user's, leaving a link, like one into the Nix store, as it is.
fn replace_in(path: &Path, old: &str, new: &str) {
    if !fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file()) {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    if text.contains(old)
        && let Err(err) = fs::write(path, text.replace(old, new))
    {
        log::warn!("can't update {}: {err}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_kept_under_the_old_name_moves_once() {
        let root = env::temp_dir().join(format!("makshell-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let old = root.join("macshell");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("config.toml"),
            "pinned = [\"macshell-settings\"]\n",
        )
        .unwrap();
        let new = root.join("makshell");
        assert!(move_path(&old, &new));
        replace_in(
            &new.join("config.toml"),
            "macshell-settings",
            "makshell-settings",
        );
        assert_eq!(
            fs::read_to_string(new.join("config.toml")).unwrap(),
            "pinned = [\"makshell-settings\"]\n"
        );
        // Nothing left to move, and what's there stays.
        assert!(!move_path(&old, &new));
        fs::create_dir_all(&old).unwrap();
        assert!(!move_path(&old, &new));
        let _ = fs::remove_dir_all(&root);
    }
}
