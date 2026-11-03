//! Bluetooth, through BlueZ on the system bus.

use zbus::fdo::ObjectManagerProxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::dbus;

const SERVICE: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bluetooth {
    adapter: OwnedObjectPath,
    pub powered: bool,
    /// Names of the connected devices.
    pub connected: Vec<String>,
}

/// The first adapter's state, or `None` without BlueZ or an adapter.
pub async fn bluetooth() -> Option<Bluetooth> {
    let connection = dbus::system().await?;
    let objects = ObjectManagerProxy::builder(&connection)
        .destination(SERVICE)
        .ok()?
        .path("/")
        .ok()?
        .build()
        .await
        .ok()?
        .get_managed_objects()
        .await
        .inspect_err(|err| log::debug!("no BlueZ: {err}"))
        .ok()?;

    let flag = |value: Option<&OwnedValue>| {
        value
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false)
    };

    let (adapter, properties) = objects
        .iter()
        .filter_map(|(path, interfaces)| Some((path, interfaces.get(ADAPTER)?)))
        .min_by_key(|(path, _)| path.as_str())?;
    let powered = flag(properties.get("Powered"));

    let mut connected: Vec<String> = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let device = interfaces.get(DEVICE)?;
            let on_adapter = path.as_str().starts_with(&format!("{}/", adapter.as_str()));
            (on_adapter && flag(device.get("Connected"))).then(|| {
                device
                    .get("Alias")
                    .or_else(|| device.get("Name"))
                    .and_then(|name| String::try_from(name.clone()).ok())
                    .unwrap_or_else(|| String::from("Device"))
            })
        })
        .collect();
    connected.sort();

    Some(Bluetooth {
        adapter: adapter.clone(),
        powered,
        connected,
    })
}

pub async fn set_powered(bluetooth: Bluetooth, powered: bool) {
    let result = async {
        let connection = dbus::system().await.ok_or(zbus::Error::Unsupported)?;
        let adapter = zbus::Proxy::new(&connection, SERVICE, bluetooth.adapter, ADAPTER).await?;
        adapter.set_property("Powered", powered).await?;
        zbus::Result::Ok(())
    }
    .await;
    if let Err(err) = result {
        log::warn!(
            "can't turn Bluetooth {}: {err}",
            if powered { "on" } else { "off" }
        );
    }
}
