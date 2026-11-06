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
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-wallpaper")),
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
