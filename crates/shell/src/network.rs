//! Wi-Fi, through NetworkManager on the system bus.

use zbus::zvariant::OwnedObjectPath;

use crate::dbus;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

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
}

/// NetworkManager's device type for Wi-Fi.
const DEVICE_TYPE_WIFI: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wifi {
    pub enabled: bool,
    /// The network it's connected to.
    pub network: Option<String>,
}

/// Wi-Fi's state, or `None` without NetworkManager or a Wi-Fi device.
pub async fn wifi() -> Option<Wifi> {
    let connection = dbus::system().await?;
    let manager = NetworkManagerProxy::new(&connection).await.ok()?;

    let mut device = None;
    for path in manager.get_devices().await.ok()? {
        let proxy = DeviceProxy::builder(&connection)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()?;
        if proxy.device_type().await.ok() == Some(DEVICE_TYPE_WIFI) {
            device = Some(path);
            break;
        }
    }
    let device = device?;

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
