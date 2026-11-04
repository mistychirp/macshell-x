//! Tray icons: apps' status items, through the StatusNotifierItem protocols, and their menus,
//! through dbusmenu. KDE's watcher keeps the list of items when it's running; otherwise we do.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{fs, process};

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{FutureExt as _, SinkExt as _, Stream, StreamExt as _, future, stream};
use iced_widget::image;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{MatchRule, MessageStream, message};

use crate::apps::AppIndex;
use crate::dbusmenu::{self, Address};
use crate::{dbus, menu};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";
/// Used when an app gives neither a name nor pixels.
const FALLBACK_ICON: &str = "application-x-executable";
const SYMBOLIC_SUFFIX: &str = "-symbolic";
/// What symbolic icons are tinted to, as straight RGBA: the bar's ink, per its backdrop.
const ON_DARK: [u8; 4] = [255, 255, 255, 255];
const ON_LIGHT: [u8; 4] = [0, 0, 0, 217];

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait Watcher {
    fn register_status_notifier_host(&self, service: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;
}

#[zbus::proxy(interface = "org.kde.StatusNotifierItem")]
trait Item {
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
    #[zbus(property)]
    fn attention_icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn menu(&self) -> zbus::Result<OwnedObjectPath>;
}

/// An app's status item, as last read.
#[derive(Debug, Clone)]
pub struct Item {
    service: String,
    path: String,
    needs_attention: bool,
    icon_name: String,
    icon_pixels: Option<image::Handle>,
    attention_name: String,
    attention_pixels: Option<image::Handle>,
    theme_path: String,
    menu_address: Option<Address>,
    menu: Vec<dbusmenu::Item>,
}

/// A status item ready for the bar.
#[derive(Debug, Clone)]
pub struct Icon {
    pub item: Item,
    /// What it looks like on a dark backdrop.
    pub image: Option<image::Handle>,
    /// What it looks like on a light one, if different.
    pub image_on_light: Option<image::Handle>,
}

impl Icon {
    pub fn new(item: Item, apps: &mut AppIndex) -> Self {
        let (name, pixels) = if item.needs_attention {
            (&item.attention_name, &item.attention_pixels)
        } else {
            (&item.icon_name, &item.icon_pixels)
        };
        // Symbolic icons are drawn in a placeholder color, to be tinted like macOS's template icons.
        if name.ends_with(SYMBOLIC_SUFFIX) {
            let names = [icon_path(name, &item.theme_path)];
            let image = apps.silhouette(&names, ON_DARK);
            if image.is_some() {
                let image_on_light = apps.silhouette(&names, ON_LIGHT);
                return Self {
                    item,
                    image,
                    image_on_light,
                };
            }
        }
        let image = named_icon(name, &item.theme_path, apps)
            .or_else(|| pixels.clone())
            .or_else(|| named_icon(&item.icon_name, &item.theme_path, apps))
            .or_else(|| item.icon_pixels.clone())
            .or_else(|| apps.themed_icon(FALLBACK_ICON));
        Self {
            item,
            image,
            image_on_light: None,
        }
    }
}

/// An icon by name: in the app's own folder if it has one, or from the theme.
fn named_icon(name: &str, theme_path: &str, apps: &mut AppIndex) -> Option<image::Handle> {
    if name.is_empty() {
        return None;
    }
    apps.themed_icon(&icon_path(name, theme_path))
}

/// What to look an icon up by: its file in the app's own folder if it's there, or else its name.
fn icon_path(name: &str, theme_path: &str) -> String {
    (!theme_path.is_empty())
        .then(|| find_in(Path::new(theme_path), name))
        .flatten()
        .map_or_else(
            || name.to_owned(),
            |path| path.to_string_lossy().into_owned(),
        )
}

/// `name.svg` or `name.png` somewhere under `folder`.
fn find_in(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut subfolders = Vec::new();
    for entry in fs::read_dir(folder).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subfolders.push(path);
        } else if path.file_stem().is_some_and(|stem| stem == name)
            && path
                .extension()
                .is_some_and(|extension| extension == "svg" || extension == "png")
        {
            return Some(path);
        }
    }
    subfolders.iter().find_map(|folder| find_in(folder, name))
}

impl Item {
    pub fn has_menu(&self) -> bool {
        !self.menu.is_empty()
    }

    /// What the app calls this item when asked to activate it.
    pub fn activate(&self, x: i32, y: i32) -> impl Future<Output = ()> + use<> {
        let (service, path) = (self.service.clone(), self.path.clone());
        async move {
            let result = async {
                let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
                let item = ItemProxy::builder(&connection)
                    .destination(service)?
                    .path(path)?
                    .cache_properties(CacheProperties::No)
                    .build()
                    .await?;
                item.activate(x, y).await
            }
            .await;
            if let Err(err) = result {
                log::warn!("can't activate a tray icon: {err}");
            }
        }
    }

    /// The entries of its menu.
    pub fn entries(&self) -> Vec<menu::Entry> {
        self.menu_address
            .as_ref()
            .map_or_else(Vec::new, |address| address.entries(&self.menu))
    }
}

/// Pixels from the largest of an app's icons, which come as big-endian ARGB.
fn pixmap(mut pixmaps: Vec<(i32, i32, Vec<u8>)>) -> Option<image::Handle> {
    pixmaps.sort_by_key(|(width, _, _)| *width);
    let (width, height, argb) = pixmaps.pop()?;
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    if width == 0 || argb.len() != (width * height * 4) as usize {
        return None;
    }
    let rgba: Vec<u8> = argb
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[a, r, g, b]| [*r, *g, *b, *a])
        .collect();
    Some(image::Handle::from_rgba(width, height, rgba))
}

/// Splits what's registered with the watcher, `service` or `service/path`.
fn split_address(address: &str) -> (&str, String) {
    match address.find('/') {
        Some(slash) => (&address[..slash], address[slash..].to_owned()),
        None => (address, DEFAULT_ITEM_PATH.to_owned()),
    }
}

async fn read_item(
    connection: &zbus::Connection,
    address_text: &str,
    seen: &[String],
) -> Option<Item> {
    let (service, path) = split_address(address_text);
    let item = ItemProxy::builder(connection)
        .destination(service.to_owned())
        .ok()?
        .path(path.clone())
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    let status = item.status().await.ok()?;
    let menu_address = item
        .menu()
        .await
        .ok()
        .filter(|path| path.as_str() != "/")
        .map(|path| Address {
            service: service.to_owned(),
            path: path.to_string(),
        });
    let mut menu = Vec::new();
    if let Some(address) = &menu_address {
        // Some apps only fill their menus when told one's about to show. Only ever once, since an
        // app can answer with a change to its menu, which brings us back here.
        if !seen.iter().any(|known| known == address_text) {
            address.about_to_show(connection, &[0]).await;
        }
        menu = address.read(connection).await;
    }
    // Apps with nothing to say right now ask not to be shown.
    if status == "Passive" {
        return None;
    }
    Some(Item {
        service: service.to_owned(),
        path,
        needs_attention: status == "NeedsAttention",
        icon_name: item.icon_name().await.unwrap_or_default(),
        icon_pixels: item.icon_pixmap().await.ok().and_then(pixmap),
        attention_name: item.attention_icon_name().await.unwrap_or_default(),
        attention_pixels: item.attention_icon_pixmap().await.ok().and_then(pixmap),
        theme_path: item.icon_theme_path().await.unwrap_or_default(),
        menu_address,
        menu,
    })
}

/// Reads every item. `seen` has the addresses of the ones read last time, and is updated.
async fn read_items(connection: &zbus::Connection, seen: &mut Vec<String>) -> Vec<Item> {
    let Ok(watcher) = WatcherProxy::builder(connection)
        .cache_properties(CacheProperties::No)
        .build()
        .await
    else {
        return Vec::new();
    };
    let addresses = watcher
        .registered_status_notifier_items()
        .await
        .unwrap_or_default();
    let items = future::join_all(
        addresses
            .iter()
            .map(|address| read_item(connection, address, seen)),
    )
    .await
    .into_iter()
    .flatten()
    .collect();
    *seen = addresses;
    items
}

/// The watcher we serve when nothing else does.
struct OwnWatcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl OwnWatcher {
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header.sender().map(ToString::to_string).unwrap_or_default();
        // Apps give a bus name, or just their object's path.
        let address = if service.starts_with('/') {
            format!("{sender}{service}")
        } else {
            service
        };
        {
            let mut items = self.items.lock().expect("watcher's list");
            if items.contains(&address) {
                return;
            }
            items.push(address.clone());
        }
        let _ = Self::status_notifier_item_registered(&emitter, &address).await;
    }

    fn register_status_notifier_host(&self, _service: String) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().expect("watcher's list").clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
}

/// Serves the watcher, if no one else is. Returns its list of items if so.
async fn serve_watcher(connection: &zbus::Connection) -> Option<Arc<Mutex<Vec<String>>>> {
    let items = Arc::new(Mutex::new(Vec::new()));
    let watcher = OwnWatcher {
        items: items.clone(),
    };
    connection
        .object_server()
        .at(WATCHER_PATH, watcher)
        .await
        .inspect_err(|err| log::warn!("can't serve a tray watcher: {err}"))
        .ok()?;
    match connection
        .request_name_with_flags(WATCHER_SERVICE, RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
            log::info!("keeping the tray's list of items ourselves");
            Some(items)
        }
        _ => {
            let _ = connection
                .object_server()
                .remove::<OwnWatcher, _>(WATCHER_PATH)
                .await;
            None
        }
    }
}

async fn signals(connection: &zbus::Connection, interface: &'static str) -> Option<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(interface)
        .ok()?
        .build();
    MessageStream::for_match_rule(rule, connection, Some(16))
        .await
        .ok()
}

/// Follows the session's status items, reporting all of them whenever anything about one changes.
pub fn items() -> Subscription<Vec<Item>> {
    Subscription::run(follow)
}

fn follow() -> impl Stream<Item = Vec<Item>> {
    iced_futures::stream::channel(4, |mut output: mpsc::Sender<Vec<Item>>| async move {
        let Some(connection) = dbus::session().await else {
            return;
        };
        let own = serve_watcher(&connection).await;

        let mut changes = Vec::new();
        for interface in [
            ITEM_INTERFACE,
            dbusmenu::INTERFACE,
            WATCHER_SERVICE,
            "org.freedesktop.DBus",
        ] {
            if let Some(stream) = signals(&connection, interface).await {
                changes.push(stream);
            }
        }
        let (pump, mut changes) = dbus::drain(stream::select_all(changes));

        let work = async move {
            // Being a host is what tells apps there's somewhere for their icons to go.
            let host = format!("org.kde.StatusNotifierHost-{}", process::id());
            let _ = connection.request_name(host.as_str()).await;
            if let Ok(watcher) = WatcherProxy::new(&connection).await
                && let Err(err) = watcher.register_status_notifier_host(&host).await
            {
                log::warn!("can't register as a tray host: {err}");
            }

            let mut seen = Vec::new();
            let mut last = read_items(&connection, &mut seen).await;
            if output.send(last.clone()).await.is_err() {
                return;
            }
            loop {
                let Some(change) = changes.next().await else {
                    return;
                };
                // Every app coming and going is announced; only ones we're showing matter.
                if !is_relevant(&change, &last, own.as_deref()) {
                    continue;
                }
                // Apps tend to change several things at once.
                while let Some(Some(_)) = changes.next().now_or_never() {}

                last = read_items(&connection, &mut seen).await;
                if output.send(last.clone()).await.is_err() {
                    return;
                }
            }
        };
        future::select(Box::pin(pump), Box::pin(work)).await;
    })
}

/// Whether a signal could change what's shown. A name disappearing also ends its items, in our list
/// if we're the one keeping it.
fn is_relevant(
    change: &message::Message,
    shown: &[Item],
    own: Option<&Mutex<Vec<String>>>,
) -> bool {
    let header = change.header();
    if header
        .interface()
        .is_some_and(|interface| interface.as_str() == dbusmenu::INTERFACE)
    {
        // Every app's menus are announced too. A service can be a name rather than the sender's
        // own address, which can't be compared here.
        let sender = header.sender().map(|sender| sender.as_str());
        return shown
            .iter()
            .any(|item| !item.service.starts_with(':') || Some(item.service.as_str()) == sender);
    }
    if header
        .member()
        .is_none_or(|member| member != "NameOwnerChanged")
    {
        return true;
    }
    let Ok((name, _, new_owner)) = change.body().deserialize::<(String, String, String)>() else {
        return false;
    };
    if !new_owner.is_empty() {
        return false;
    }
    let mut relevant = shown.iter().any(|item| item.service == name);
    if let Some(own) = own {
        let mut items = own.lock().expect("watcher's list");
        let before = items.len();
        items.retain(|address| split_address(address).0 != name);
        relevant |= items.len() != before;
    }
    relevant
}
