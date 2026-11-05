//! Asking the desktop portal for a file, which brings its own file chooser.

use std::collections::HashMap;
use std::path::PathBuf;

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
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop"
)]
trait Request {
    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}

/// Lets the user pick an image. `None` if they didn't.
pub async fn choose_image() -> Option<PathBuf> {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        let token = format!("macshell{}", std::process::id());
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
            "Images",
            vec![
                (0u32, "*.png"),
                (0u32, "*.jpg"),
                (0u32, "*.jpeg"),
                (0u32, "*.webp"),
            ],
        )]);
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("filters", filters),
        ]);
        FileChooserProxy::new(&connection)
            .await?
            .open_file("", "Choose a Wallpaper", options)
            .await?;

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
fn path_of(uri: &str) -> Option<PathBuf> {
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
