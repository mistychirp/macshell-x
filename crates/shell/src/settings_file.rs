//! The file settings are exported to and imported from: the config file and the glass, in one place.
//! Machine-local state like where widgets and icons are isn't part of it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::Config;

const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SettingsFile {
    /// Which format this is, so that a newer one isn't misread.
    version: u32,
    /// The glass, by the keys of our KWin plugin.
    pub glass: BTreeMap<String, f32>,
    /// What's in the config file.
    config: toml::Table,
}

impl SettingsFile {
    /// Fails if the config file isn't valid, since then there's nothing good to export.
    pub fn new(glass: BTreeMap<String, f32>, config_text: &str) -> Result<Self, String> {
        let config: toml::Table = config_text
            .parse()
            .map_err(|err| format!("Your config file isn't valid: {err}"))?;
        Ok(Self {
            version: VERSION,
            glass,
            config,
        })
    }

    pub fn to_text(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|err| err.to_string())
    }

    /// Reads one, refusing anything that couldn't be applied. Nothing has changed if this fails.
    pub fn parse(text: &str) -> Result<Self, String> {
        // The version first, since a newer file may not have the rest in this shape.
        #[derive(Deserialize)]
        struct Versioned {
            version: Option<u32>,
        }
        let versioned: Versioned = toml::from_str(text)
            .map_err(|_| String::from("This isn't a makshell settings file."))?;
        match versioned.version {
            None => return Err(String::from("This isn't a makshell settings file.")),
            Some(version) if version > VERSION => {
                return Err(String::from(
                    "This file is from a newer makshell than this one.",
                ));
            }
            Some(_) => {}
        }
        let file: Self =
            toml::from_str(text).map_err(|_| String::from("This settings file is damaged."))?;
        file.config
            .clone()
            .try_into::<Config>()
            .map_err(|err| format!("This file's settings aren't valid: {err}"))?;
        Ok(file)
    }

    pub fn config_text(&self) -> Result<String, String> {
        toml::to_string_pretty(&self.config).map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "[dock]\npinned = [\"a\", \"b\"]\nmagnification = false\n\n[desktop]\nwallpaper = \"~/w.png\"\nalways_glassy_widgets = true\n";

    fn glass() -> BTreeMap<String, f32> {
        BTreeMap::from([(String::from("GlassGloss"), 0.5)])
    }

    #[test]
    fn survives_a_round_trip() {
        let file = SettingsFile::new(glass(), CONFIG).unwrap();
        let read = SettingsFile::parse(&file.to_text().unwrap()).unwrap();
        assert_eq!(read, file);
        let config: Config = toml::from_str(&read.config_text().unwrap()).unwrap();
        assert_eq!(config.dock.pinned, ["a", "b"]);
        assert!(!config.dock.magnification);
        assert!(config.desktop.always_glassy_widgets);
        assert_eq!(config.desktop.wallpaper.as_deref(), Some("~/w.png"));
    }

    #[test]
    fn an_empty_config_is_fine() {
        let file = SettingsFile::new(BTreeMap::new(), "").unwrap();
        assert!(SettingsFile::parse(&file.to_text().unwrap()).is_ok());
    }

    #[test]
    fn refuses_what_it_cannot_apply() {
        assert!(SettingsFile::parse("not toml at all [").is_err());
        assert!(SettingsFile::parse("[dock]\npinned = []\n").is_err());
        assert!(SettingsFile::parse("version = 99\n").is_err());
        assert!(SettingsFile::parse("version = 1\n").is_err());
        let wrong_type = "version = 1\n[glass]\n[config.dock]\nmagnification = \"yes\"\n";
        assert!(SettingsFile::parse(wrong_type).is_err());
    }
}
