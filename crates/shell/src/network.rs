//! Wi-Fi and wired networks, through NetworkManager on the system bus. The shell is also the
//! session's secret agent: NetworkManager comes to it when a network needs a password.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::futures::channel::{mpsc, oneshot};
use iced_futures::futures::{SinkExt as _, Stream, StreamExt as _, future};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::dbus;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;
    #[zbus(name = "AddAndActivateConnection2")]
    fn add_and_activate_connection2(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedValue>,
    )>;

    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ActiveConnection {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AgentManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/AgentManager"
)]
trait AgentManager {
    fn register(&self, identifier: &str) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
trait AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait SavedConnection {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;
    fn save(&self) -> zbus::Result<()>;
}

/// NetworkManager's device types.
const DEVICE_TYPE_ETHERNET: u32 = 1;
const DEVICE_TYPE_WIFI: u32 = 2;
/// NetworkManager's device state for being fully connected.
const DEVICE_ACTIVATED: u32 = 100;
/// NetworkManager's states of an active connection.
const CONNECTION_ACTIVATED: u32 = 2;
const CONNECTION_DEACTIVATED: u32 = 4;
/// How long to wait for a new connection to work out, mostly for someone typing its password.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wifi {
    pub enabled: bool,
    /// The network it's connected to.
    pub network: Option<String>,
}

/// A Wi-Fi network in range. Access points of the same network are one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// From 0 to 100.
    pub strength: u8,
    pub secured: bool,
    /// It has been connected to before.
    pub known: bool,
    pub connected: bool,
}

/// The first Wi-Fi device.
async fn wifi_device(
    connection: &zbus::Connection,
    manager: &NetworkManagerProxy<'_>,
) -> Option<OwnedObjectPath> {
    for path in manager.get_devices().await.ok()? {
        let proxy = DeviceProxy::builder(connection)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()?;
        if proxy.device_type().await.ok() == Some(DEVICE_TYPE_WIFI) {
            return Some(path);
        }
    }
    None
}

/// Wi-Fi's state, or `None` without NetworkManager or a Wi-Fi device.
pub async fn wifi() -> Option<Wifi> {
    let connection = dbus::system().await?;
    let manager = NetworkManagerProxy::new(&connection).await.ok()?;
    let device = wifi_device(&connection, &manager).await?;

    let enabled = manager.wireless_enabled().await.ok()?;
    let network = async {
        let wireless = WirelessProxy::builder(&connection)
            .path(device)
            .ok()?
            .build()
            .await
            .ok()?;
        let access_point = wireless.active_access_point().await.ok()?;
        // "/" when not connected.
        if access_point.as_str() == "/" {
            return None;
        }
        let access_point = AccessPointProxy::builder(&connection)
            .path(access_point)
            .ok()?
            .build()
            .await
            .ok()?;
        Some(String::from_utf8_lossy(&access_point.ssid().await.ok()?).into_owned())
    }
    .await;

    Some(Wifi { enabled, network })
}

pub async fn set_wifi_enabled(enabled: bool) {
    let result = async {
        let connection = dbus::system().await.ok_or(zbus::Error::Unsupported)?;
        NetworkManagerProxy::new(&connection)
            .await?
            .set_wireless_enabled(enabled)
            .await
    }
    .await;
    if let Err(err) = result {
        log::warn!(
            "can't turn Wi-Fi {}: {err}",
            if enabled { "on" } else { "off" }
        );
    }
}

/// Asks for a fresh look at what's in range. The results come in over the next few seconds.
pub async fn scan() {
    let result = async {
        let connection = dbus::system().await.ok_or(zbus::Error::Unsupported)?;
        let manager = NetworkManagerProxy::new(&connection).await?;
        let device = wifi_device(&connection, &manager)
            .await
            .ok_or(zbus::Error::Unsupported)?;
        WirelessProxy::builder(&connection)
            .path(device)?
            .build()
            .await?
            .request_scan(HashMap::new())
            .await
    }
    .await;
    // Scanning too often is refused; the last results are still good.
    if let Err(err) = result {
        log::debug!("can't scan for Wi-Fi networks: {err}");
    }
}

/// A saved Wi-Fi connection.
struct Saved {
    path: OwnedObjectPath,
    /// It has ever connected. One that hasn't says little about the network being known.
    used: bool,
}

/// The bytes of a byte array in a connection's settings.
fn bytes_of(value: &Value<'_>) -> Option<Vec<u8>> {
    let Value::Array(bytes) = value else {
        return None;
    };
    bytes
        .iter()
        .map(|byte| match byte {
            Value::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect()
}

/// The saved Wi-Fi connections, by network name.
async fn saved_connections(connection: &zbus::Connection) -> HashMap<Vec<u8>, Saved> {
    let mut saved = HashMap::new();
    let Ok(settings) = SettingsProxy::new(connection).await else {
        return saved;
    };
    for path in settings.list_connections().await.unwrap_or_default() {
        let Ok(builder) = SavedConnectionProxy::builder(connection).path(path.clone()) else {
            continue;
        };
        let Ok(proxy) = builder.build().await else {
            continue;
        };
        let Ok(settings) = proxy.get_settings().await else {
            continue;
        };
        let ssid = settings
            .get("802-11-wireless")
            .and_then(|wireless| wireless.get("ssid"))
            .and_then(|ssid| bytes_of(ssid));
        if let Some(ssid) = ssid {
            let used = settings
                .get("connection")
                .and_then(|connection| connection.get("timestamp"))
                .is_some_and(|timestamp| matches!(**timestamp, Value::U64(when) if when > 0));
            saved.insert(ssid, Saved { path, used });
        }
    }
    saved
}

/// What's in range: the connected network first, then known ones, then the rest, strongest first.
pub async fn networks() -> Vec<Network> {
    async {
        let connection = dbus::system().await?;
        let manager = NetworkManagerProxy::new(&connection).await.ok()?;
        let device = wifi_device(&connection, &manager).await?;
        let wireless = WirelessProxy::builder(&connection)
            .path(device)
            .ok()?
            .build()
            .await
            .ok()?;
        let active = wireless.active_access_point().await.ok();
        let saved = saved_connections(&connection).await;

        let mut networks: Vec<Network> = Vec::new();
        for path in wireless.get_all_access_points().await.ok()? {
            let connected = active.as_ref() == Some(&path);
            let Ok(builder) = AccessPointProxy::builder(&connection).path(path) else {
                continue;
            };
            let Ok(point) = builder.build().await else {
                continue;
            };
            let Ok(ssid) = point.ssid().await else {
                continue;
            };
            // A hidden network doesn't say its name.
            if ssid.is_empty() {
                continue;
            }
            let secured = point.flags().await.unwrap_or(0) != 0
                || point.wpa_flags().await.unwrap_or(0) != 0
                || point.rsn_flags().await.unwrap_or(0) != 0;
            let strength = point.strength().await.unwrap_or(0);
            let known = connected || saved.get(&ssid).is_some_and(|saved| saved.used);
            let name = String::from_utf8_lossy(&ssid).into_owned();
            match networks.iter_mut().find(|network| network.ssid == name) {
                Some(network) => {
                    network.strength = network.strength.max(strength);
                    network.connected |= connected;
                    network.known |= connected;
                }
                None => networks.push(Network {
                    ssid: name,
                    strength,
                    secured,
                    known,
                    connected,
                }),
            }
        }
        networks.sort_by_key(|network| {
            (
                !network.connected,
                !network.known,
                u8::MAX - network.strength,
            )
        });
        Some(networks)
    }
    .await
    .unwrap_or_default()
}

/// Connects to a network: with its saved connection if it has one. Otherwise NetworkManager makes
/// one, which only stays if it works out, so a mistyped password or a change of mind leaves nothing
/// behind. If it needs a password, it comes to the [`secret_agent`]. Returns once that's settled.
pub async fn connect(ssid: String) {
    let result = async {
        let connection = dbus::system().await.ok_or(zbus::Error::Unsupported)?;
        let manager = NetworkManagerProxy::new(&connection).await?;
        let device = wifi_device(&connection, &manager)
            .await
            .ok_or(zbus::Error::Unsupported)?;
        let none = ObjectPath::try_from("/")?;
        if let Some(saved) = saved_connections(&connection).await.get(ssid.as_bytes()) {
            manager
                .activate_connection(&saved.path, &device, &none)
                .await?;
            return Ok(());
        }
        let wireless = WirelessProxy::builder(&connection)
            .path(device.clone())?
            .build()
            .await?;
        let mut strongest: Option<(u8, OwnedObjectPath)> = None;
        for path in wireless.get_all_access_points().await? {
            let point = AccessPointProxy::builder(&connection)
                .path(path.clone())?
                .build()
                .await?;
            if point.ssid().await.ok().as_deref() == Some(ssid.as_bytes()) {
                let strength = point.strength().await.unwrap_or(0);
                if strongest.as_ref().is_none_or(|(best, _)| strength > *best) {
                    strongest = Some((strength, path));
                }
            }
        }
        let (_, point) = strongest.ok_or(zbus::Error::Unsupported)?;
        let options = HashMap::from([("persist", Value::from("volatile"))]);
        let (saved, active, _) = manager
            .add_and_activate_connection2(HashMap::new(), &device, &point, options)
            .await?;
        if connection_works_out(&connection, active).await {
            SavedConnectionProxy::builder(&connection)
                .path(saved)?
                .build()
                .await?
                .save()
                .await?;
        }
        zbus::Result::Ok(())
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't connect to a Wi-Fi network: {err}");
    }
}

/// Waits to see whether a connection that's being activated gets there.
async fn connection_works_out(connection: &zbus::Connection, active: OwnedObjectPath) -> bool {
    let waited = tokio::time::timeout(CONNECT_TIMEOUT, async {
        loop {
            let Ok(builder) = ActiveConnectionProxy::builder(connection).path(active.clone())
            else {
                return false;
            };
            let Ok(proxy) = builder.cache_properties(CacheProperties::No).build().await else {
                return false;
            };
            // It's gone from the bus once it has failed.
            match proxy.state().await {
                Ok(CONNECTION_ACTIVATED) => return true,
                Ok(CONNECTION_DEACTIVATED) | Err(_) => return false,
                Ok(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    });
    waited.await.unwrap_or(false)
}

/// Whether to show the wired icon: a cable is connected and carrying the machine, with no Wi-Fi.
pub async fn on_ethernet_only() -> bool {
    async {
        let connection = dbus::system().await?;
        let manager = NetworkManagerProxy::new(&connection).await.ok()?;
        let mut wired = false;
        for path in manager.get_devices().await.ok()? {
            let proxy = DeviceProxy::builder(&connection)
                .path(path)
                .ok()?
                .cache_properties(CacheProperties::No)
                .build()
                .await
                .ok()?;
            if proxy.state().await.ok() != Some(DEVICE_ACTIVATED) {
                continue;
            }
            match proxy.device_type().await.ok()? {
                DEVICE_TYPE_ETHERNET => wired = true,
                DEVICE_TYPE_WIFI => return Some(false),
                _ => {}
            }
        }
        Some(wired)
    }
    .await
    .unwrap_or(false)
}

/// A network's password that NetworkManager wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRequest {
    pub id: u64,
    pub ssid: String,
    /// The last password didn't work.
    pub retry: bool,
    /// The fewest characters such a password can have.
    pub min_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    Asks(SecretRequest),
    /// NetworkManager doesn't need the answer any more.
    Withdrawn(u64),
}

/// Requests waiting for an answer.
struct Pending {
    connection: String,
    setting: String,
    reply: oneshot::Sender<Option<String>>,
}

static PENDING: Mutex<Vec<(u64, Pending)>> = Mutex::new(Vec::new());
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

/// Answers a [`SecretRequest`]: with a password, or `None` if there won't be one.
pub fn answer(id: u64, password: Option<String>) {
    let mut pending = PENDING.lock().expect("pending secret requests");
    if let Some(index) = pending.iter().position(|(other, _)| *other == id) {
        let (_, request) = pending.swap_remove(index);
        // It's gone if NetworkManager gave up in the meantime.
        let _ = request.reply.send(password);
    }
}

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NoSecrets(String),
    UserCanceled(String),
}

const WIRELESS_SECURITY: &str = "802-11-wireless-security";
/// NetworkManager's flags for asking for secrets.
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;

type ConnectionSettings = HashMap<String, HashMap<String, OwnedValue>>;

struct SecretAgent {
    events: mpsc::UnboundedSender<AgentEvent>,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl SecretAgent {
    async fn get_secrets(
        &self,
        connection: ConnectionSettings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
    ) -> Result<ConnectionSettings, AgentError> {
        let no_secrets = |why: &str| AgentError::NoSecrets(why.to_owned());
        if flags & ALLOW_INTERACTION == 0 {
            return Err(no_secrets("not allowed to ask"));
        }
        // Enterprise networks log in another way, for which there's no dialog yet.
        if setting_name != WIRELESS_SECURITY {
            return Err(no_secrets("only Wi-Fi passwords are supported"));
        }
        let ssid = connection
            .get("802-11-wireless")
            .and_then(|wireless| wireless.get("ssid"))
            .and_then(|ssid| bytes_of(ssid))
            .ok_or_else(|| no_secrets("not a Wi-Fi network"))?;
        let key_management = connection
            .get(WIRELESS_SECURITY)
            .and_then(|security| security.get("key-mgmt"))
            .and_then(|mode| match &**mode {
                Value::Str(mode) => Some(mode.as_str().to_owned()),
                _ => None,
            });
        let (key, min_length) = match key_management.as_deref() {
            Some("wpa-psk" | "sae") => ("psk", 8),
            Some("none" | "ieee8021x") => ("wep-key0", 5),
            _ => return Err(no_secrets("this kind of network isn't supported")),
        };

        let (reply, answer) = oneshot::channel();
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        PENDING.lock().expect("pending secret requests").push((
            id,
            Pending {
                connection: connection_path.to_string(),
                setting: setting_name.clone(),
                reply,
            },
        ));
        let request = SecretRequest {
            id,
            ssid: String::from_utf8_lossy(&ssid).into_owned(),
            retry: flags & REQUEST_NEW != 0,
            min_length,
        };
        let _ = self.events.unbounded_send(AgentEvent::Asks(request));

        let Ok(Some(password)) = answer.await else {
            return Err(AgentError::UserCanceled(String::from("no password given")));
        };
        let password = OwnedValue::try_from(Value::from(password)).map_err(zbus::Error::from)?;
        Ok(HashMap::from([(
            setting_name,
            HashMap::from([(key.to_owned(), password)]),
        )]))
    }

    fn cancel_get_secrets(&self, connection_path: OwnedObjectPath, setting_name: String) {
        let mut pending = PENDING.lock().expect("pending secret requests");
        let connection = connection_path.to_string();
        pending.retain(|(id, request)| {
            let cancelled = request.connection == connection && request.setting == setting_name;
            if cancelled {
                let _ = self.events.unbounded_send(AgentEvent::Withdrawn(*id));
            }
            !cancelled
        });
    }

    // Passwords go in NetworkManager's own connection files, so there's nothing for us to keep.
    fn save_secrets(&self, _connection: ConnectionSettings, _connection_path: OwnedObjectPath) {}

    fn delete_secrets(&self, _connection: ConnectionSettings, _connection_path: OwnedObjectPath) {}
}

/// Serves as the session's secret agent for as long as the shell runs.
pub fn secret_agent() -> Subscription<AgentEvent> {
    Subscription::run(serve_secret_agent)
}

fn serve_secret_agent() -> impl Stream<Item = AgentEvent> {
    iced_futures::stream::channel(8, |mut output: mpsc::Sender<AgentEvent>| async move {
        const PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
        let Some(connection) = dbus::system().await else {
            return future::pending().await;
        };
        let (events, mut incoming) = mpsc::unbounded();
        let served = connection
            .object_server()
            .at(PATH, SecretAgent { events })
            .await;
        if let Err(err) = served {
            log::warn!("can't serve as a secret agent: {err}");
            return future::pending().await;
        }

        // NetworkManager forgets its agents when it restarts.
        let register = async {
            let Ok(manager) = AgentManagerProxy::new(&connection).await else {
                return;
            };
            let Ok(mut owners) = manager.inner().receive_owner_changed().await else {
                return;
            };
            if let Err(err) = manager.register("org.makshell.shell").await {
                log::warn!("can't register as a secret agent: {err}");
            }
            while let Some(owner) = owners.next().await {
                if owner.is_some()
                    && let Err(err) = manager.register("org.makshell.shell").await
                {
                    log::warn!("can't register as a secret agent: {err}");
                }
            }
        };
        let forward = async {
            while let Some(event) = incoming.next().await {
                let _ = output.send(event).await;
            }
        };
        future::join(register, forward).await;
        future::pending().await
    })
}
