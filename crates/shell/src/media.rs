//! Now Playing, through MPRIS media players on the session bus.

use std::collections::HashMap;
use std::path::PathBuf;

use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

use crate::dbus;

const PREFIX: &str = "org.mpris.MediaPlayer2.";

#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait Player {
    fn play_pause(&self) -> zbus::Result<()>;
    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// The player's bus name.
    player: String,
    pub title: String,
    pub artist: Option<String>,
    /// Local cover art.
    pub art: Option<PathBuf>,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Previous,
    Next,
}

/// What's playing: the first player that's playing, else the first that's paused.
pub async fn now_playing() -> Option<Track> {
    let connection = dbus::session().await?;
    let names = zbus::fdo::DBusProxy::new(&connection)
        .await
        .ok()?
        .list_names()
        .await
        .ok()?;

    let mut paused = None;
    for name in names.iter().filter(|name| name.starts_with(PREFIX)) {
        let Some(track) = track(&connection, name.as_str()).await else {
            continue;
        };
        if track.playing {
            return Some(track);
        }
        paused.get_or_insert(track);
    }
    paused
}

async fn track(connection: &zbus::Connection, player: &str) -> Option<Track> {
    let proxy = PlayerProxy::builder(connection)
        .destination(player.to_owned())
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    let status = proxy.playback_status().await.ok()?;
    if status == "Stopped" {
        return None;
    }
    let metadata = proxy.metadata().await.ok()?;
    let string = |key: &str| {
        metadata
            .get(key)
            .and_then(|value| String::try_from(value.clone()).ok())
            .filter(|value| !value.is_empty())
    };
    let artist = metadata
        .get("xesam:artist")
        .and_then(|value| Vec::<String>::try_from(value.clone()).ok())
        .map(|artists| artists.join(", "))
        .filter(|artists| !artists.is_empty());

    Some(Track {
        player: player.to_owned(),
        title: string("xesam:title")?,
        artist,
        art: string("mpris:artUrl").and_then(|url| file_path(&url)),
        playing: status == "Playing",
    })
}

pub async fn control(track: Track, control: Control) {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        let proxy = PlayerProxy::builder(&connection)
            .destination(track.player)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        match control {
            Control::PlayPause => proxy.play_pause().await,
            Control::Previous => proxy.previous().await,
            Control::Next => proxy.next().await,
        }
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't control the media player: {err}");
    }
}

/// The local path of a `file://` URL, percent-decoded.
fn file_path(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%'
            && let Some(hex) = tail.get(..2)
            && let Ok(decoded) = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16)
        {
            bytes.push(decoded);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}
