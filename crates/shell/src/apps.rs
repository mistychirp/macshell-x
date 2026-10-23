//! Resolves window app ids and pinned ids to desktop entries: names, icons and launch commands.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use freedesktop_desktop_entry::unicase::Ascii;
use freedesktop_desktop_entry::{DesktopEntry, find_app_by_id};

#[derive(Debug)]
pub struct App {
    /// Desktop entry id (e.g. `org.kde.dolphin`), or the raw app id when no entry matched.
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
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
                icon: entry.icon().and_then(|icon| self.find_icon(icon)),
                command: entry.parse_exec().ok(),
            },
            None => App {
                id: app_id.to_owned(),
                name: fallback_name(app_id),
                icon: self.find_icon(app_id),
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
