//! Drag and drop with other apps, and between the shell's own surfaces, through wl_data_device.
//!
//! It has an event queue and a thread of its own, on the connection it shares with iced_layershell.
//! An app that's dropping something wants an answer right away, and the UI's loop only comes around
//! when it has something to do. Files travel as `text/uri-list`, which is what file managers use.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt as _, Stream, StreamExt as _, future};
use iced_widget::image;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};

use crate::{places, window_shadow};

const URI_LIST: &str = "text/uri-list";
/// How big a dragged icon's picture is, in pixels and logical pixels.
const ICON_PIXELS: u32 = 104;
const ICON_SCALE: i32 = 2;
/// How far apart the icons of a dragged pile are, and how many of them there are.
const PILE_OFFSET: u32 = 10;
const PILE: usize = 3;

#[derive(Debug, Clone)]
pub enum Event {
    /// A drag came over one of the shell's surfaces.
    Entered {
        surface: ObjectId,
        x: f32,
        y: f32,
    },
    Moved {
        x: f32,
        y: f32,
    },
    Left,
    Dropped {
        surface: ObjectId,
        x: f32,
        y: f32,
        uris: Vec<String>,
        /// It's a drag the shell started.
        own: bool,
    },
    /// A drag the shell started is over, wherever it ended.
    DragEnded,
}

/// A picture to drag: premultiplied BGRA, the way wl_shm has it.
pub struct DragIcon {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

struct Shared {
    connection: Connection,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    compositor: WlCompositor,
    shm: WlShm,
    queue: QueueHandle<State>,
    events: mpsc::UnboundedSender<Event>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static RECEIVER: Mutex<Option<mpsc::UnboundedReceiver<Event>>> = Mutex::new(None);
/// What the drag the shell started is offering, and the surface of its picture.
static OUTGOING: Mutex<Option<Outgoing>> = Mutex::new(None);

struct Outgoing {
    bytes: Vec<u8>,
    icon: Option<(WlSurface, WlBuffer)>,
}

/// A drag that's over one of the shell's surfaces.
struct Current {
    offer: WlDataOffer,
    surface: ObjectId,
    x: f32,
    y: f32,
    has_files: bool,
    /// The compositor has settled on an action, without which an offer can't be finished.
    action: Arc32,
}

type Arc32 = std::sync::Arc<AtomicU32>;

#[derive(Default)]
struct State {
    mimes: HashMap<ObjectId, Vec<String>>,
    current: Option<Current>,
    selection: Option<WlDataOffer>,
}

/// Starts listening for drags. Does nothing where the compositor has no data device.
pub fn start(connection: &Connection) {
    if let Err(err) = try_start(connection) {
        log::warn!("no drag and drop: {err}");
    }
}

fn try_start(connection: &Connection) -> Result<(), String> {
    let (globals, mut queue) =
        registry_queue_init::<State>(connection).map_err(|err| err.to_string())?;
    let handle = queue.handle();
    let bind_error = |err: wayland_client::globals::BindError| err.to_string();
    let manager: WlDataDeviceManager = globals.bind(&handle, 3..=3, ()).map_err(bind_error)?;
    let seat: WlSeat = globals.bind(&handle, 1..=1, ()).map_err(bind_error)?;
    // Below version 5, a dragged picture's hotspot can be set when attaching it.
    let compositor: WlCompositor = globals.bind(&handle, 4..=4, ()).map_err(bind_error)?;
    let shm: WlShm = globals.bind(&handle, 1..=1, ()).map_err(bind_error)?;
    let device = manager.get_data_device(&seat, &handle, ());

    let (events, receiver) = mpsc::unbounded();
    *RECEIVER.lock().expect("drag events") = Some(receiver);
    let _ = SHARED.set(Shared {
        connection: connection.clone(),
        manager,
        device,
        compositor,
        shm,
        queue: handle,
        events,
    });

    thread::Builder::new()
        .name(String::from("drag-and-drop"))
        .spawn(move || {
            let mut state = State::default();
            loop {
                if let Err(err) = queue.blocking_dispatch(&mut state) {
                    log::warn!("drag and drop stopped: {err}");
                    return;
                }
            }
        })
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// What's happening with drags.
pub fn events() -> Subscription<Event> {
    Subscription::run(follow)
}

fn follow() -> impl Stream<Item = Event> {
    iced_futures::stream::channel(16, |mut output: mpsc::Sender<Event>| async move {
        let Some(mut receiver) = RECEIVER.lock().expect("drag events").take() else {
            return future::pending().await;
        };
        while let Some(event) = receiver.next().await {
            let _ = output.send(event).await;
        }
    })
}

fn send(event: Event) {
    if let Some(shared) = SHARED.get() {
        let _ = shared.events.unbounded_send(event);
    }
}

fn flush() {
    if let Some(shared) = SHARED.get()
        && let Err(err) = shared.connection.flush()
    {
        log::warn!("can't flush a drag: {err}");
    }
}

/// Starts dragging files, from a press with this serial on `origin`. The icon follows the pointer.
pub fn start_drag(origin: &WlSurface, serial: u32, uris: &[String], icon: Option<DragIcon>) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let source = shared.manager.create_data_source(&shared.queue, ());
    source.offer(URI_LIST.to_owned());
    source.set_actions(DndAction::Copy | DndAction::Move);

    let mut bytes = Vec::new();
    for uri in uris {
        bytes.extend_from_slice(uri.as_bytes());
        bytes.extend_from_slice(b"\r\n");
    }
    let icon = icon.and_then(|icon| match icon_surface(shared, icon) {
        Ok(surface) => Some(surface),
        Err(err) => {
            log::warn!("can't make a dragged icon: {err}");
            None
        }
    });
    let surface = icon.as_ref().map(|(surface, ..)| surface.clone());
    shared
        .device
        .start_drag(Some(&source), origin, surface.as_ref(), serial);
    // It only counts as a dragged icon, whose hotspot the attach can set, once the drag has started.
    let icon = icon.map(|(surface, buffer, (width, height))| {
        surface.set_buffer_scale(ICON_SCALE);
        surface.attach(
            Some(&buffer),
            -width / (2 * ICON_SCALE),
            -height / (2 * ICON_SCALE),
        );
        surface.damage_buffer(0, 0, width, height);
        surface.commit();
        (surface, buffer)
    });
    *OUTGOING.lock().expect("outgoing drag") = Some(Outgoing { bytes, icon });
    flush();
}

fn icon_surface(shared: &Shared, icon: DragIcon) -> io::Result<(WlSurface, WlBuffer, (i32, i32))> {
    let file = window_shadow::anonymous_file(&icon.pixels)?;
    let pool = shared
        .shm
        .create_pool(file.as_fd(), icon.pixels.len() as i32, &shared.queue, ());
    let buffer = pool.create_buffer(
        0,
        icon.width as i32,
        icon.height as i32,
        icon.width as i32 * 4,
        Format::Argb8888,
        &shared.queue,
        (),
    );
    pool.destroy();
    let surface = shared.compositor.create_surface(&shared.queue, ());
    Ok((surface, buffer, (icon.width as i32, icon.height as i32)))
}

/// A picture of what's being dragged: the first few icons, in a pile.
pub fn drag_icon(handles: &[image::Handle]) -> Option<DragIcon> {
    let pile = handles.iter().take(PILE).filter_map(icon_pixels);
    let size = ICON_PIXELS + (PILE as u32 - 1) * PILE_OFFSET;
    let mut canvas = ::image::RgbaImage::new(size, size);
    let mut any = false;
    // The last one's on top, so draw from the back.
    let layers: Vec<_> = pile.collect();
    for (index, layer) in layers.iter().enumerate().rev() {
        let at = i64::from(index as u32 * PILE_OFFSET);
        ::image::imageops::overlay(&mut canvas, layer, at, at);
        any = true;
    }
    if !any {
        return None;
    }
    let mut pixels = canvas.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        let premultiplied = |channel: u8| ((u32::from(channel) * alpha + 127) / 255) as u8;
        let (red, green, blue) = (
            premultiplied(pixel[0]),
            premultiplied(pixel[1]),
            premultiplied(pixel[2]),
        );
        pixel[0] = blue;
        pixel[1] = green;
        pixel[2] = red;
    }
    Some(DragIcon {
        width: size,
        height: size,
        pixels,
    })
}

fn icon_pixels(handle: &image::Handle) -> Option<::image::RgbaImage> {
    let picture = match handle {
        image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } => ::image::RgbaImage::from_raw(*width, *height, pixels.to_vec())?,
        image::Handle::Path(_, path) => ::image::open(path).ok()?.to_rgba8(),
        image::Handle::Bytes(_, bytes) => ::image::load_from_memory(bytes).ok()?.to_rgba8(),
    };
    Some(::image::imageops::resize(
        &picture,
        ICON_PIXELS,
        ICON_PIXELS,
        ::image::imageops::FilterType::Triangle,
    ))
}

/// The local files among some URIs.
pub fn paths_of(uris: &[String]) -> Vec<std::path::PathBuf> {
    uris.iter()
        .filter_map(|uri| crate::portal::path_of(uri))
        .collect()
}

/// The URIs for some files.
pub fn uris_of(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths.iter().map(|path| places::file_uri(path)).collect()
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.mimes.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id: Some(offer),
            } => {
                let has_files = state
                    .mimes
                    .get(&offer.id())
                    .is_some_and(|mimes| mimes.iter().any(|mime| mime == URI_LIST));
                offer.accept(serial, has_files.then(|| URI_LIST.to_owned()));
                if has_files {
                    // Moving is what macOS does on the same disk; copying is for across disks.
                    offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Move);
                }
                flush();
                let (x, y) = (x as f32, y as f32);
                state.current = Some(Current {
                    offer,
                    surface: surface.id(),
                    x,
                    y,
                    has_files,
                    action: Default::default(),
                });
                send(Event::Entered {
                    surface: surface.id(),
                    x,
                    y,
                });
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                let (x, y) = (x as f32, y as f32);
                if let Some(current) = &mut state.current {
                    current.x = x;
                    current.y = y;
                }
                send(Event::Moved { x, y });
            }
            wl_data_device::Event::Leave => {
                if let Some(current) = state.current.take() {
                    state.mimes.remove(&current.offer.id());
                    current.offer.destroy();
                    flush();
                }
                send(Event::Left);
            }
            wl_data_device::Event::Drop => {
                if let Some(current) = state.current.take() {
                    state.mimes.remove(&current.offer.id());
                    receive(current);
                }
            }
            wl_data_device::Event::Selection { id } => {
                if let Some(old) = state.selection.take() {
                    state.mimes.remove(&old.id());
                    old.destroy();
                }
                state.selection = id;
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

/// Reads what was dropped, on another thread: if it's a drag of our own, this thread has to be free
/// to send it.
fn receive(current: Current) {
    let Current {
        offer,
        surface,
        x,
        y,
        has_files,
        action,
    } = current;
    if !has_files {
        offer.destroy();
        flush();
        send(Event::Left);
        return;
    }
    let Ok((mut reader, writer)) = io::pipe() else {
        offer.destroy();
        flush();
        send(Event::Left);
        return;
    };
    offer.receive(URI_LIST.to_owned(), writer.as_fd());
    flush();
    drop(writer);
    let spawned = thread::Builder::new()
        .name(String::from("drop"))
        .spawn(move || {
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            let uris: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned)
                .collect();
            // Before finishing, which ends a drag of our own: this has to be heard of first.
            let own = OUTGOING.lock().expect("outgoing drag").is_some();
            send(Event::Dropped {
                surface,
                x,
                y,
                uris,
                own,
            });
            if action.load(Ordering::Relaxed) != 0 {
                offer.finish();
            }
            offer.destroy();
            flush();
        });
    if let Err(err) = spawned {
        log::warn!("can't read what was dropped: {err}");
    }
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                state.mimes.entry(offer.id()).or_default().push(mime_type);
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => {
                if let Some(current) = &state.current
                    && current.offer == *offer
                {
                    current.action.store(action.bits(), Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        _: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { fd, .. } => {
                let bytes = OUTGOING
                    .lock()
                    .expect("outgoing drag")
                    .as_ref()
                    .map(|outgoing| outgoing.bytes.clone())
                    .unwrap_or_default();
                let mut file = File::from(fd);
                if let Err(err) = file.write_all(&bytes) {
                    log::warn!("can't send what's being dragged: {err}");
                }
            }
            wl_data_source::Event::DndFinished | wl_data_source::Event::Cancelled => {
                source.destroy();
                if let Some(Outgoing {
                    icon: Some((surface, buffer)),
                    ..
                }) = OUTGOING.lock().expect("outgoing drag").take()
                {
                    surface.destroy();
                    buffer.destroy();
                }
                flush();
                send(Event::DragEnded);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
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

delegate_noop!(State: WlDataDeviceManager);
delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore WlSurface);
