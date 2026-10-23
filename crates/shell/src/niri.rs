//! Bridges niri's blocking IPC event stream into iced subscriptions.

use std::collections::HashMap;
use std::io;
use std::thread;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt, Stream, StreamExt};
use niri_ipc::socket::Socket;
use niri_ipc::{Event, Request, Window};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedWindow {
    pub app_id: Option<String>,
    pub title: Option<String>,
}

/// Emits the focused window every time it (or its title) changes.
pub fn focused_window() -> Subscription<Option<FocusedWindow>> {
    Subscription::run(focused_window_stream)
}

fn focused_window_stream() -> impl Stream<Item = Option<FocusedWindow>> {
    iced_futures::stream::channel(
        16,
        |mut output: mpsc::Sender<Option<FocusedWindow>>| async move {
            let (tx, mut rx) = mpsc::unbounded();

            // The niri socket API is blocking, so it gets its own thread.
            thread::spawn(move || {
                if let Err(err) = track_focus(tx) {
                    log::error!("niri event stream ended: {err}");
                }
            });

            while let Some(focused) = rx.next().await {
                if output.send(focused).await.is_err() {
                    break;
                }
            }
        },
    )
}

fn track_focus(tx: mpsc::UnboundedSender<Option<FocusedWindow>>) -> io::Result<()> {
    let mut socket = Socket::connect()?;
    socket
        .send(Request::EventStream)?
        .map_err(io::Error::other)?;
    let mut read_event = socket.read_events();

    let mut windows: HashMap<u64, Window> = HashMap::new();
    let mut focused_id: Option<u64> = None;
    let mut last_sent: Option<Option<FocusedWindow>> = None;

    loop {
        match read_event()? {
            Event::WindowsChanged { windows: all } => {
                focused_id = all.iter().find(|w| w.is_focused).map(|w| w.id);
                windows = all.into_iter().map(|w| (w.id, w)).collect();
            }
            Event::WindowOpenedOrChanged { window } => {
                if window.is_focused {
                    focused_id = Some(window.id);
                }
                windows.insert(window.id, window);
            }
            Event::WindowClosed { id } => {
                windows.remove(&id);
                if focused_id == Some(id) {
                    focused_id = None;
                }
            }
            Event::WindowFocusChanged { id } => focused_id = id,
            _ => continue,
        }

        let focused = focused_id
            .and_then(|id| windows.get(&id))
            .map(|w| FocusedWindow {
                app_id: w.app_id.clone(),
                title: w.title.clone(),
            });

        if last_sent.as_ref() != Some(&focused) {
            if tx.unbounded_send(focused.clone()).is_err() {
                return Ok(());
            }
            last_sent = Some(focused);
        }
    }
}
