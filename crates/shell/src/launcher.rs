//! Count badges for the dock. Apps announce them with the `LauncherEntry` signal, which Unity started
//! and most of the desktop kept.

use std::collections::HashMap;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt as _, Stream, StreamExt as _, future};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, MessageStream, message};

use crate::dbus;

const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

/// An app's news about its badge. Either part can be missing, which leaves it as it was.
#[derive(Debug, Clone)]
pub struct Update {
    /// The app's desktop entry id, in lowercase.
    pub app: String,
    pub count: Option<u32>,
    pub visible: Option<bool>,
}

pub fn updates() -> Subscription<Update> {
    Subscription::run(follow)
}

fn follow() -> impl Stream<Item = Update> {
    iced_futures::stream::channel(8, |mut output: mpsc::Sender<Update>| async move {
        let Some(connection) = dbus::session().await else {
            return;
        };
        let Some(signals) = signals(&connection).await else {
            return;
        };
        let (pump, mut signals) = dbus::drain(signals);
        let work = async move {
            while let Some(signal) = signals.next().await {
                let body = signal.body();
                let Ok((uri, properties)) =
                    body.deserialize::<(String, HashMap<String, OwnedValue>)>()
                else {
                    continue;
                };
                let app = uri
                    .strip_prefix("application://")
                    .unwrap_or(&uri)
                    .strip_suffix(".desktop")
                    .unwrap_or(&uri)
                    .to_lowercase();
                let count = match properties.get("count").map(|count| &**count) {
                    Some(Value::I64(count)) => u32::try_from(*count).ok(),
                    _ => None,
                };
                let visible = match properties.get("count-visible").map(|shown| &**shown) {
                    Some(Value::Bool(visible)) => Some(*visible),
                    _ => None,
                };
                let update = Update {
                    app,
                    count,
                    visible,
                };
                if output.send(update).await.is_err() {
                    return;
                }
            }
        };
        future::select(Box::pin(pump), Box::pin(work)).await;
    })
}

async fn signals(connection: &zbus::Connection) -> Option<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(INTERFACE)
        .ok()?
        .build();
    MessageStream::for_match_rule(rule, connection, Some(16))
        .await
        .ok()
}
