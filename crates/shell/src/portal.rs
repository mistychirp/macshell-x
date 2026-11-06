//! Asking the desktop portal for a file, which brings its own file chooser.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use iced_futures::futures::StreamExt as _;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::dbus;

#[zbus::proxy(
    interface = "org.freedesktop.portal.FileChooser",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait FileChooser {
    fn open_file(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn save_file(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop"
)]
trait Request {
    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}

/// Which dialog to ask for.
#[derive(Clone, Copy)]
enum Dialog<'a> {
    Open,
    /// Suggests this file name.
    Save(&'a str),
}

/// Lets the user pick an image. `None` if they didn't.
pub async fn choose_image() -> Option<PathBuf> {
    let patterns = ["*.png", "*.jpg", "*.jpeg", "*.webp"];
    ask(Dialog::Open, "Choose a Wallpaper", "Images", &patterns).await
}

/// Lets the user pick a settings file to read. `None` if they didn't.
pub async fn choose_settings_file() -> Option<PathBuf> {
    ask(Dialog::Open, "Import Settings", "Settings", &["*.toml"]).await
}

/// Lets the user pick where to save a settings file. `None` if they didn't.
pub async fn choose_settings_destination(name: &str) -> Option<PathBuf> {
    ask(
        Dialog::Save(name),
        "Export Settings",
        "Settings",
        &["*.toml"],
    )
    .await
}

async fn ask(
    dialog: Dialog<'_>,
    title: &str,
    filter_name: &str,
    patterns: &[&str],
) -> Option<PathBuf> {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        // Different every time, so that this can't be taken for the answer to an earlier question.
        static ASKED: AtomicU32 = AtomicU32::new(0);
        let token = format!(
            "macshell{}_{}",
            std::process::id(),
            ASKED.fetch_add(1, Ordering::Relaxed)
        );
        // Listen where the answer will come before asking, or it can be missed.
        let sender = connection
            .unique_name()
            .ok_or(zbus::Error::Unsupported)?
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let request = RequestProxy::builder(&connection)
            .path(path)?
            .build()
            .await?;
        let mut responses = request.receive_response().await?;

        let filters = Value::from(vec![(
            filter_name,
            patterns
                .iter()
                .map(|pattern| (0u32, *pattern))
                .collect::<Vec<_>>(),
        )]);
        let mut options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("filters", filters),
        ]);
        let chooser = FileChooserProxy::new(&connection).await?;
        match dialog {
            Dialog::Open => {
                chooser.open_file("", title, options).await?;
            }
            Dialog::Save(name) => {
                options.insert("current_name", Value::from(name));
                chooser.save_file("", title, options).await?;
            }
        }

        let Some(answer) = responses.next().await else {
            return Ok(None);
        };
        let answer = answer.args()?;
        if answer.response != 0 {
            return Ok(None);
        }
        let uris = answer
            .results
            .get("uris")
            .and_then(|uris| <Vec<String>>::try_from(uris.clone()).ok())
            .unwrap_or_default();
        zbus::Result::Ok(uris.first().and_then(|uri| path_of(uri)))
    }
    .await;
    result
        .inspect_err(|err| log::warn!("can't ask for a file: {err}"))
        .ok()
        .flatten()
}

/// The path of a `file://` URI.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        let escaped = (encoded[index] == b'%')
            .then(|| encoded.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                bytes.push(byte);
                index += 3;
            }
            None => {
                bytes.push(encoded[index]);
                index += 1;
            }
        }
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}
