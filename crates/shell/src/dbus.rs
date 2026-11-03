//! D-Bus connections shared by the shell's services, opened on first use.

use std::sync::OnceLock;

static SYSTEM: OnceLock<zbus::Connection> = OnceLock::new();
static SESSION: OnceLock<zbus::Connection> = OnceLock::new();

/// The system bus, for NetworkManager and BlueZ.
pub async fn system() -> Option<zbus::Connection> {
    if let Some(connection) = SYSTEM.get() {
        return Some(connection.clone());
    }
    let connection = zbus::Connection::system()
        .await
        .inspect_err(|err| log::warn!("can't connect to the system bus: {err}"))
        .ok()?;
    Some(SYSTEM.get_or_init(|| connection).clone())
}

/// The session bus, for media players.
pub async fn session() -> Option<zbus::Connection> {
    if let Some(connection) = SESSION.get() {
        return Some(connection.clone());
    }
    let connection = zbus::Connection::session()
        .await
        .inspect_err(|err| log::warn!("can't connect to the session bus: {err}"))
        .ok()?;
    Some(SESSION.get_or_init(|| connection).clone())
}
