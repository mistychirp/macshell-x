//! Resolves window app ids and pinned ids to desktop entries: names, icons and launch commands.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use freedesktop_desktop_entry::unicase::Ascii;
use freedesktop_desktop_entry::{DesktopEntry, find_app_by_id};
use iced_widget::image;
use resvg::{tiny_skia, usvg};

/// Pixel size SVG icons are rasterized at: enough for a 128px magnified dock icon at 2x scale,
/// so the GPU only ever scales them down.
const SVG_RASTER_SIZE: u32 = 256;

#[derive(Debug)]
pub struct App {
    /// Desktop entry id (e.g. `org.kde.dolphin`), or the raw app id when no entry matched.
    pub id: String,
    pub name: String,
    pub icon: Option<image::Handle>,
    pub command: Option<Vec<String>>,
}

pub struct AppIndex {
    entries: Vec<DesktopEntry>,
    locales: Vec<String>,
    icon_theme: String,
    cache: HashMap<String, Rc<App>>,
}

impl AppIndex {
    pub fn load() -> Self {
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        let entries = freedesktop_desktop_entry::desktop_entries(&locales);
        let icon_theme =
            freedesktop_icons::default_theme_gtk().unwrap_or_else(|| String::from("hicolor"));
        log::info!(
            "indexed {} desktop entries, icon theme {icon_theme}",
            entries.len()
        );

        Self {
            entries,
            locales,
            icon_theme,
            cache: HashMap::new(),
        }
    }

    pub fn resolve(&mut self, app_id: &str) -> Rc<App> {
        if let Some(app) = self.cache.get(app_id) {
            return app.clone();
        }

        let app = Rc::new(match find_app_by_id(&self.entries, Ascii::new(app_id)) {
            Some(entry) => App {
                id: entry.id().to_owned(),
                name: entry
                    .name(&self.locales)
                    .map_or_else(|| fallback_name(app_id), Cow::into_owned),
                icon: entry
                    .icon()
                    .and_then(|icon| self.find_icon(icon))
                    .and_then(|path| load_icon(&path)),
                command: entry.parse_exec().ok(),
            },
            None => App {
                id: app_id.to_owned(),
                name: fallback_name(app_id),
                icon: self.find_icon(app_id).and_then(|path| load_icon(&path)),
                command: None,
            },
        });

        log::debug!("resolved {app_id}: {app:?}");
        self.cache.insert(app_id.to_owned(), app.clone());
        app
    }

    fn find_icon(&self, name: &str) -> Option<PathBuf> {
        let path = Path::new(name);
        if path.is_absolute() {
            return path.exists().then(|| path.to_owned());
        }
        freedesktop_icons::lookup(name)
            .with_theme(&self.icon_theme)
            .with_size(128)
            .with_cache()
            .find()
    }
}

/// Turns an app id like `org.gnome.Nautilus` into `Nautilus`.
fn fallback_name(app_id: &str) -> String {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::from(app_id),
    }
}

/// Loads an icon as an image the GPU can scale smoothly. SVGs are rasterized once here, since
/// drawing them as SVGs would re-rasterize them for every size the dock animates through.
fn load_icon(path: &Path) -> Option<image::Handle> {
    if path.extension().is_none_or(|ext| ext != "svg") {
        return Some(image::Handle::from_path(path));
    }

    let data = fs::read(path)
        .inspect_err(|err| log::warn!("can't read {}: {err}", path.display()))
        .ok()?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .inspect_err(|err| log::warn!("can't parse {}: {err}", path.display()))
        .ok()?;

    let size = SVG_RASTER_SIZE;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let svg_size = tree.size();
    let scale = size as f32 / svg_size.width().max(svg_size.height());
    let offset_x = (size as f32 - svg_size.width() * scale) / 2.0;
    let offset_y = (size as f32 - svg_size.height() * scale) / 2.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied alpha; iced expects straight alpha.
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Some(image::Handle::from_rgba(size, size, rgba))
}
