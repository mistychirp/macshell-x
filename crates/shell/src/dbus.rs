//! D-Bus connections shared by the shell's services, opened on first use.

use std::sync::OnceLock;

use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{Stream, StreamExt as _};

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

/// Keeps signals moving, whatever the receiver is up to. zbus stops reading from the bus while a
/// stream's queue is full, replies included, so a task that makes calls between reading a stream
/// can stall the whole connection, and with it everything else on the session bus, when an app
/// sends a burst of signals. The returned future has to be polled alongside the work, and does
/// nothing but move signals to the returned receiver, which has no limit.
pub fn drain<S>(
    mut signals: S,
) -> (
    impl Future<Output = ()>,
    mpsc::UnboundedReceiver<zbus::Message>,
)
where
    S: Stream<Item = zbus::Result<zbus::Message>> + Unpin,
{
    let (sender, receiver) = mpsc::unbounded();
    let pump = async move {
        while let Some(signal) = signals.next().await {
            if let Ok(signal) = signal
                && sender.unbounded_send(signal).is_err()
            {
                return;
            }
        }
    };
    (pump, receiver)
}
