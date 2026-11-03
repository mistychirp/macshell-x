//! Talks to KWin: application windows through KDE's window management protocol, on a Wayland
//! connection of our own, and keyboard layouts through KWin's D-Bus interface.
//!
//! KWin only offers window management to clients whose desktop entry asks for it with
//! `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` and whose `Exec` matches their
//! executable; see `contrib/macshell.desktop`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::thread;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt, Stream, StreamExt, stream};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::dbus;
use crate::plasma_protocol::org_kde_plasma_window::{self, OrgKdePlasmaWindow};
use crate::plasma_protocol::org_kde_plasma_window_management::{
    self, OrgKdePlasmaWindowManagement,
};

/// The parts of a window the shell cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Ours, not KWin's; stable while the window lives.
    pub id: u64,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub is_focused: bool,
    /// The window is asking for attention.
    pub is_urgent: bool,
    /// When it last got focus, as an ever-increasing count, for most-recently-used ordering.
    pub focused_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLayouts {
    /// Display names, like "English (US)".
    pub names: Vec<String>,
    /// Index of the active one in `names`.
    pub current: usize,
}

/// The full window list, ordered by when windows opened, every time it changes.
pub fn windows() -> Subscription<Vec<WindowInfo>> {
    Subscription::run(window_stream)
}

/// The keyboard layouts, every time they or the active one change.
pub fn keyboard_layouts() -> Subscription<KeyboardLayouts> {
    Subscription::run(layout_stream)
}

/// Brings the window forward, unminimizing it if needed.
pub fn focus_window(id: u64) {
    with_window(id, |window| {
        window.set_state(STATE_ACTIVE | STATE_MINIMIZED, STATE_ACTIVE);
    });
}

/// Asks the window to close, the same as its close button would.
pub fn close_window(id: u64) {
    with_window(id, OrgKdePlasmaWindow::close);
}

pub async fn switch_layout(index: usize) {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        LayoutsProxy::new(&connection)
            .await?
            .set_layout(index as u32)
            .await
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't switch keyboard layout: {err}");
    }
}

// Window state flags (`org_kde_plasma_window_management.state`).
const STATE_ACTIVE: u32 = 0x1;
const STATE_MINIMIZED: u32 = 0x2;
const STATE_DEMANDS_ATTENTION: u32 = 0x80;
const STATE_SKIP_TASKBAR: u32 = 0x1000;

/// Version 16: the last before stacking order moved to its own object, which we don't need.
const VERSION: u32 = 16;

/// Window objects by our id, so requests can come from the UI thread.
static WINDOWS: Mutex<BTreeMap<u64, OrgKdePlasmaWindow>> = Mutex::new(BTreeMap::new());
static CONNECTION: OnceLock<Connection> = OnceLock::new();

fn with_window(id: u64, request: impl FnOnce(&OrgKdePlasmaWindow)) {
    let windows = WINDOWS.lock().unwrap();
    let Some(window) = windows.get(&id) else {
        return;
    };
    request(window);
    if let Some(Err(err)) = CONNECTION.get().map(Connection::flush) {
        log::error!("can't flush the window management connection: {err}");
    }
}

fn window_stream() -> impl Stream<Item = Vec<WindowInfo>> {
    iced_futures::stream::channel(16, |mut output: mpsc::Sender<Vec<WindowInfo>>| async move {
        let (tx, mut rx) = mpsc::unbounded();

        // Wayland dispatching blocks, so it gets its own thread.
        thread::spawn(move || {
            if let Err(err) = track_windows(tx) {
                log::error!("window tracking ended: {err}");
            }
        });

        while let Some(windows) = rx.next().await {
            if output.send(windows).await.is_err() {
                break;
            }
        }
    })
}

fn track_windows(tx: mpsc::UnboundedSender<Vec<WindowInfo>>) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<Tracker>(&connection).map_err(|err| err.to_string())?;
    let _manager: OrgKdePlasmaWindowManagement = globals
        .bind(&queue.handle(), 1..=VERSION, ())
        .map_err(|_| {
            "KWin doesn't offer window management; is macshell's desktop entry installed?"
                .to_owned()
        })?;
    let _ = CONNECTION.set(connection);

    let mut tracker = Tracker {
        windows: HashMap::new(),
        next_id: 1,
        focus_clock: 0,
    };
    let mut last_sent = None;
    loop {
        queue
            .blocking_dispatch(&mut tracker)
            .map_err(|err| err.to_string())?;
        let snapshot = tracker.snapshot();
        if last_sent.as_ref() != Some(&snapshot) {
            if tx.unbounded_send(snapshot.clone()).is_err() {
                return Ok(());
            }
            last_sent = Some(snapshot);
        }
    }
}

struct Tracker {
    windows: HashMap<u64, Tracked>,
    next_id: u64,
    focus_clock: u64,
}

#[derive(Default)]
struct Tracked {
    app_id: Option<String>,
    title: Option<String>,
    state: u32,
    focused_at: Option<u64>,
    /// KWin has sent everything it knows; until then the window is half described.
    ready: bool,
}

impl Tracker {
    fn snapshot(&self) -> Vec<WindowInfo> {
        let mut windows: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|(_, w)| w.ready && w.state & STATE_SKIP_TASKBAR == 0)
            .map(|(&id, w)| WindowInfo {
                id,
                app_id: w.app_id.clone(),
                title: w.title.clone(),
                is_focused: w.state & STATE_ACTIVE != 0,
                is_urgent: w.state & STATE_DEMANDS_ATTENTION != 0,
                focused_at: w.focused_at,
            })
            .collect();
        windows.sort_by_key(|w| w.id);
        windows
    }
}

impl Dispatch<OrgKdePlasmaWindowManagement, ()> for Tracker {
    fn event(
        state: &mut Self,
        manager: &OrgKdePlasmaWindowManagement,
        event: org_kde_plasma_window_management::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        if let org_kde_plasma_window_management::Event::WindowWithUuid { uuid, .. } = event {
            let id = state.next_id;
            state.next_id += 1;
            let window = manager.get_window_by_uuid(uuid, queue, id);
            WINDOWS.lock().unwrap().insert(id, window);
            state.windows.insert(id, Tracked::default());
        }
    }
}

impl Dispatch<OrgKdePlasmaWindow, u64> for Tracker {
    fn event(
        state: &mut Self,
        window: &OrgKdePlasmaWindow,
        event: org_kde_plasma_window::Event,
        &id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(tracked) = state.windows.get_mut(&id) else {
            return;
        };
        match event {
            org_kde_plasma_window::Event::TitleChanged { title } => tracked.title = Some(title),
            org_kde_plasma_window::Event::AppIdChanged { app_id } => {
                tracked.app_id = (!app_id.is_empty()).then_some(app_id);
            }
            org_kde_plasma_window::Event::StateChanged { flags } => {
                if flags & STATE_ACTIVE != 0 && tracked.state & STATE_ACTIVE == 0 {
                    state.focus_clock += 1;
                    tracked.focused_at = Some(state.focus_clock);
                }
                tracked.state = flags;
            }
            org_kde_plasma_window::Event::InitialState => tracked.ready = true,
            org_kde_plasma_window::Event::Unmapped => {
                state.windows.remove(&id);
                WINDOWS.lock().unwrap().remove(&id);
                window.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Tracker {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[zbus::proxy(
    interface = "org.kde.KeyboardLayouts",
    default_service = "org.kde.keyboard",
    default_path = "/Layouts"
)]
trait Layouts {
    // KDE names its members in camelCase.
    #[zbus(name = "getLayout")]
    fn get_layout(&self) -> zbus::Result<u32>;
    #[zbus(name = "setLayout")]
    fn set_layout(&self, index: u32) -> zbus::Result<bool>;
    /// Short name, variant and display name of each layout.
    #[zbus(name = "getLayoutsList")]
    fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;

    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;
    #[zbus(signal, name = "layoutListChanged")]
    fn layout_list_changed(&self) -> zbus::Result<()>;
}

fn layout_stream() -> impl Stream<Item = KeyboardLayouts> {
    iced_futures::stream::channel(4, |mut output: mpsc::Sender<KeyboardLayouts>| async move {
        let Some(connection) = dbus::session().await else {
            return;
        };
        let proxy = match LayoutsProxy::new(&connection).await {
            Ok(proxy) => proxy,
            Err(err) => {
                log::warn!("no keyboard layouts from KWin: {err}");
                return;
            }
        };
        let (Ok(changed), Ok(list_changed)) = (
            proxy.receive_layout_changed().await,
            proxy.receive_layout_list_changed().await,
        ) else {
            log::warn!("can't follow KWin's keyboard layouts");
            return;
        };

        // Reread everything on either signal; it's two small calls.
        let mut changes = stream::select(changed.map(|_| ()), list_changed.map(|_| ()));
        loop {
            let layouts = async {
                let list = proxy.get_layouts_list().await.ok()?;
                let current = proxy.get_layout().await.ok()? as usize;
                Some(KeyboardLayouts {
                    names: list.into_iter().map(|(_, _, name)| name).collect(),
                    current,
                })
            }
            .await;
            if let Some(layouts) = layouts
                && output.send(layouts).await.is_err()
            {
                return;
            }
            if changes.next().await.is_none() {
                return;
            }
        }
    })
}
