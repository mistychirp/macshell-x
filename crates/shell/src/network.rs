//! Wi-Fi, through NetworkManager on the system bus.

use std::collections::HashMap;

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
    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

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
}

/// NetworkManager's device type for Wi-Fi.
const DEVICE_TYPE_WIFI: u32 = 2;

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
    /// There's a saved connection for it.
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

/// The saved Wi-Fi connections, by network name.
async fn saved_connections(connection: &zbus::Connection) -> HashMap<Vec<u8>, OwnedObjectPath> {
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
        let ssid = proxy.get_settings().await.ok().and_then(|settings| {
            let ssid = settings.get("802-11-wireless")?.get("ssid")?;
            let Value::Array(bytes) = &**ssid else {
                return None;
            };
            bytes
                .iter()
                .map(|byte| match byte {
                    Value::U8(byte) => Some(*byte),
                    _ => None,
                })
                .collect::<Option<Vec<u8>>>()
        });
        if let Some(ssid) = ssid {
            saved.insert(ssid, path);
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
            let known = saved.contains_key(&ssid);
            let name = String::from_utf8_lossy(&ssid).into_owned();
            match networks.iter_mut().find(|network| network.ssid == name) {
                Some(network) => {
                    network.strength = network.strength.max(strength);
                    network.connected |= connected;
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
/// one, and if it needs a password, asks the session's secret agent.
pub async fn connect(ssid: String) {
    let result = async {
        let connection = dbus::system().await.ok_or(zbus::Error::Unsupported)?;
        let manager = NetworkManagerProxy::new(&connection).await?;
        let device = wifi_device(&connection, &manager)
            .await
            .ok_or(zbus::Error::Unsupported)?;
        let none = ObjectPath::try_from("/")?;
        if let Some(saved) = saved_connections(&connection).await.get(ssid.as_bytes()) {
            manager.activate_connection(saved, &device, &none).await?;
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
        manager
            .add_and_activate_connection(HashMap::new(), &device, &point)
            .await?;
        zbus::Result::Ok(())
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't connect to a Wi-Fi network: {err}");
    }
}
