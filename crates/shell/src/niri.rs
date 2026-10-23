//! Talks to niri: an event-stream subscription for window state, and one-shot actions.

use std::collections::BTreeMap;
use std::io;
use std::thread;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt, Stream, StreamExt};
use niri_ipc::socket::Socket;
use niri_ipc::{Action, Event, Request, Window};

/// The parts of a niri window the shell cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    pub app_id: Option<String>,
    pub is_focused: bool,
    /// Last focus time as `(secs, nanos)`, for most-recently-used ordering.
    pub focused_at: Option<(u64, u32)>,
}

impl From<&Window> for WindowInfo {
    fn from(window: &Window) -> Self {
        Self {
            id: window.id,
            app_id: window.app_id.clone(),
            is_focused: window.is_focused,
            focused_at: window.focus_timestamp.map(|t| (t.secs, t.nanos)),
        }
    }
}

/// Emits the full window list, ordered by id, every time it changes.
pub fn windows() -> Subscription<Vec<WindowInfo>> {
    Subscription::run(windows_stream)
}

pub fn focus_window(id: u64) {
    run_action(Action::FocusWindow { id });
}

/// Spawns through niri so the process gets niri's environment, not ours.
pub fn spawn(command: Vec<String>) {
    run_action(Action::Spawn { command });
}

fn run_action(action: Action) {
    let result = Socket::connect().and_then(|mut socket| {
        socket
            .send(Request::Action(action))?
            .map(drop)
            .map_err(io::Error::other)
    });
    if let Err(err) = result {
        log::error!("niri action failed: {err}");
    }
}

fn windows_stream() -> impl Stream<Item = Vec<WindowInfo>> {
    iced_futures::stream::channel(16, |mut output: mpsc::Sender<Vec<WindowInfo>>| async move {
        let (tx, mut rx) = mpsc::unbounded();

        // The niri socket API is blocking, so it gets its own thread.
        thread::spawn(move || {
            if let Err(err) = track_windows(tx) {
                log::error!("niri event stream ended: {err}");
            }
        });

        while let Some(windows) = rx.next().await {
            if output.send(windows).await.is_err() {
                break;
            }
        }
    })
}

fn track_windows(tx: mpsc::UnboundedSender<Vec<WindowInfo>>) -> io::Result<()> {
    let mut socket = Socket::connect()?;
    socket
        .send(Request::EventStream)?
        .map_err(io::Error::other)?;
    let mut read_event = socket.read_events();

    // niri doesn't promise ids increase, but in practice id order is open order.
    let mut windows: BTreeMap<u64, WindowInfo> = BTreeMap::new();
    let mut last_sent: Option<Vec<WindowInfo>> = None;

    loop {
        match read_event()? {
            Event::WindowsChanged { windows: all } => {
                windows = all.iter().map(|w| (w.id, WindowInfo::from(w))).collect();
            }
            Event::WindowOpenedOrChanged { window } => {
                if window.is_focused {
                    for w in windows.values_mut() {
                        w.is_focused = false;
                    }
                }
                windows.insert(window.id, WindowInfo::from(&window));
            }
            Event::WindowClosed { id } => {
                windows.remove(&id);
            }
            Event::WindowFocusChanged { id } => {
                for w in windows.values_mut() {
                    w.is_focused = Some(w.id) == id;
                }
            }
            Event::WindowFocusTimestampChanged {
                id,
                focus_timestamp,
            } => {
                if let Some(w) = windows.get_mut(&id) {
                    w.focused_at = focus_timestamp.map(|t| (t.secs, t.nanos));
                }
            }
            _ => continue,
        }

        let snapshot: Vec<WindowInfo> = windows.values().cloned().collect();
        if last_sent.as_ref() != Some(&snapshot) {
            if tx.unbounded_send(snapshot.clone()).is_err() {
                return Ok(());
            }
            last_sent = Some(snapshot);
        }
    }
}
