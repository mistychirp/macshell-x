//! The wallpaper: an image behind everything, on the compositor's background layer. KWin doesn't
//! draw one itself; in Plasma, that's plasmashell's job.

use std::env;
use std::path::PathBuf;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{ContentFit, Length};
use iced_widget::image;

use crate::app::Element;

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Background,
        size: Some((0, 0)),
        // Under the bar too, rather than in the space it leaves.
        exclusive_zone: Some(-1),
        // So that Show Desktop can take the keyboard from the window it hides, or that stays active.
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        // KWin goes by this exact name. It's what makes this what Show Desktop leaves showing and
        // Overview draws behind the windows, and it keeps our other surfaces from counting as windows.
        namespace: Some(String::from("desktop")),
        ..Default::default()
    }
}

/// The file at a configured path, which may start with `~/`, if it's there.
pub fn resolve(configured: &str) -> Option<PathBuf> {
    let path = match configured.strip_prefix("~/") {
        Some(rest) => PathBuf::from(env::var_os("HOME")?).join(rest),
        None => PathBuf::from(configured),
    };
    if !path.is_file() {
        log::error!("no wallpaper at {}", path.display());
        return None;
    }
    Some(path)
}

/// Keeps a link to the wallpaper where the lock screen looks for it, or takes it away without one.
/// The lock screen is QML that KDE's screen locker runs, which can't read the config file.
pub fn link_for_lock_screen(configured: Option<&str>) {
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")));
    let Some(dir) = state.map(|state| state.join("macshell")) else {
        return;
    };
    let link = dir.join("lock-wallpaper");
    let _ = std::fs::remove_file(&link);
    let Some(path) = configured.and_then(resolve) else {
        return;
    };
    let linked =
        std::fs::create_dir_all(&dir).and_then(|()| std::os::unix::fs::symlink(&path, &link));
    if let Err(err) = linked {
        log::warn!("can't show the wallpaper on the lock screen: {err}");
    }
}

/// The image at a configured path, if it's there.
pub fn load(configured: &str) -> Option<image::Handle> {
    resolve(configured).map(image::Handle::from_path)
}

pub fn view<'a>(handle: &image::Handle) -> Element<'a> {
    image(handle.clone())
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(ContentFit::Cover)
        .into()
}
