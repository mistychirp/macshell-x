//! The focused window's menus, which go in the menu bar. Apps that export them say where, and
//! KWin passes that on with the window.

use std::collections::HashMap;
use std::sync::Mutex;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{FutureExt as _, SinkExt as _, Stream, StreamExt as _, future};
use zbus::message::Header;
use zbus::zvariant::OwnedObjectPath;
use zbus::{MatchRule, MessageStream, message};

use crate::dbus;
use crate::dbusmenu::{self, Address, Item};

/// What Qt apps look for before exporting their menus. KDE's app menu widget normally brings it.
const REGISTRAR_SERVICE: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";

#[derive(Default)]
struct Registrar {
    windows: Mutex<HashMap<u32, (String, OwnedObjectPath)>>,
}

#[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
impl Registrar {
    fn register_window(
        &self,
        window_id: u32,
        menu_object_path: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) {
        let service = header.sender().map(ToString::to_string).unwrap_or_default();
        self.windows
            .lock()
            .expect("registrar's windows")
            .insert(window_id, (service, menu_object_path));
    }

    fn unregister_window(&self, window_id: u32) {
        self.windows
            .lock()
            .expect("registrar's windows")
            .remove(&window_id);
    }

    fn get_menu_for_window(&self, window_id: u32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        self.windows
            .lock()
            .expect("registrar's windows")
            .get(&window_id)
            .cloned()
            .ok_or_else(|| zbus::fdo::Error::Failed(String::from("no menu for that window")))
    }
}

/// Serves the registrar for as long as the shell runs. It reports nothing.
pub fn registrar<T: Send + 'static>() -> Subscription<T> {
    Subscription::run(serve_registrar::<T>)
}

fn serve_registrar<T: Send + 'static>() -> impl Stream<Item = T> {
    iced_futures::stream::channel(1, |_: mpsc::Sender<T>| async move {
        let Some(connection) = dbus::session().await else {
            return;
        };
        let served = connection
            .object_server()
            .at(REGISTRAR_PATH, Registrar::default())
            .await;
        if let Err(err) = served {
            log::warn!("can't serve the app menu registrar: {err}");
            return;
        }
        if let Err(err) = connection.request_name(REGISTRAR_SERVICE).await {
            // Something else is doing it, which is just as good.
            log::debug!("not serving the app menu registrar: {err}");
        }
        future::pending::<()>().await;
    })
}

/// Follows the menu at this address, reporting all of it whenever it changes.
pub fn follow(address: Address) -> Subscription<(Address, Vec<Item>)> {
    Subscription::run_with(address, |address| stream(address.clone()))
}

fn stream(address: Address) -> impl Stream<Item = (Address, Vec<Item>)> {
    iced_futures::stream::channel(
        4,
        |mut output: mpsc::Sender<(Address, Vec<Item>)>| async move {
            let Some(connection) = dbus::session().await else {
                return;
            };
            let Some(changes) = changes(&connection).await else {
                log::warn!("can't follow the menu of {}", address.service);
                return;
            };
            let (pump, mut changes) = dbus::drain(changes);

            let work = async move {
                let items = address.read(&connection).await;
                if output.send((address.clone(), items.clone())).await.is_err() {
                    return;
                }
                // Some apps only fill their menus when told they're about to show. Only ever once,
                // since an app can answer with a change, which brings us back to reading.
                let ids: Vec<i32> = items.iter().map(|item| item.id).collect();
                address.about_to_show(&connection, &ids).await;

                loop {
                    let items = address.read(&connection).await;
                    if output.send((address.clone(), items)).await.is_err() {
                        return;
                    }
                    // Every app's menus are announced; only this one matters.
                    loop {
                        let Some(change) = changes.next().await else {
                            return;
                        };
                        let header = change.header();
                        if header
                            .sender()
                            .is_some_and(|sender| sender.as_str() == address.service)
                        {
                            break;
                        }
                    }
                    // Apps tend to change several things at once.
                    while let Some(Some(_)) = changes.next().now_or_never() {}
                }
            };
            future::select(Box::pin(pump), Box::pin(work)).await;
        },
    )
}

async fn changes(connection: &zbus::Connection) -> Option<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(dbusmenu::INTERFACE)
        .ok()?
        .build();
    MessageStream::for_match_rule(rule, connection, Some(16))
        .await
        .ok()
}
