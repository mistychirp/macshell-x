//! The shell process: owns every layer surface and the state they share.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced_futures::Subscription;
use iced_futures::backend::native::tokio::time;
use iced_layershell::actions::{ActionCallback, IcedNewPopupSettings};
use iced_layershell::daemon;
use iced_layershell::reexport::{IcedId, PopupAnchor, PopupGravity, Task, WithConnection};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::core::{Color, Event, Point, Size, Theme, event, mouse, theme, window};
use iced_widget::renderer::Renderer;
use iced_widget::{Space, image};
use wayland_client::Connection;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::alert::{self, Alert};
use crate::apps::{self, App, AppIndex};
use crate::banners::{self, Banners};
use crate::config::{self, Config};
use crate::kwin::{self, KeyboardLayouts, WindowInfo};
use crate::material::{self, Material, MaterialSurface};
use crate::notifications::{self, Notification};
use crate::places::{self, Place};
use crate::{
    audio, bluetooth, brightness, control, dock, media, menu, menubar, network,
    notification_center, stack, tray, wallpaper,
};

pub type Element<'a> = iced_widget::core::Element<'a, Message, Theme, Renderer>;

pub fn run() -> Result<(), iced_layershell::Error> {
    // Our own connection, shared with iced_layershell, so protocol objects we create can refer
    // to the surfaces it creates.
    let connection = Connection::connect_to_env()
        .inspect_err(|err| log::error!("can't connect to Wayland: {err}"))
        .ok();
    let material = connection.as_ref().and_then(Material::new).map(Rc::new);

    daemon(
        move || Shell::new(material.clone()),
        namespace,
        Shell::update,
        Shell::view,
    )
    .style(style)
    .subscription(Shell::subscription)
    .settings(Settings {
        // No implicit surface; `Shell::new` opens each one explicitly.
        layer_settings: LayerShellSettings {
            start_mode: StartMode::Background,
            ..Default::default()
        },
        default_text_size: menubar::TEXT_SIZE.into(),
        with_connection: connection.map(WithConnection::Value),
        ..Default::default()
    })
    .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Wallpaper,
    MenuBar,
    Dock,
    Popup,
    Alert,
    Banners,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    WindowsChanged(Vec<WindowInfo>),
    KeyboardLayoutsChanged(KeyboardLayouts),
    /// A layout was chosen in the keyboard layout menu; carries its index.
    SwitchLayout(usize),
    /// The Control Center's keyboard module was pressed.
    NextLayout,
    Screenshot,
    LockScreen,
    /// The Control Center's display slider moved.
    BrightnessChanged(f32),
    BrightnessReleased,
    /// The monitor's brightness was read, if it could be.
    BrightnessRead(Option<f32>),
    /// The Control Center's volume slider moved.
    VolumeChanged(f32),
    VolumeReleased,
    ToggleMute,
    ToggleWifi,
    ToggleBluetooth,
    Media(media::Control),
    /// Time to reread what the Control Center shows.
    ControlRefresh,
    ControlStatus(control::Status),
    Tick,
    /// A dock icon was pressed; carries the app's id. It becomes a click or a drag.
    DockPressed(String),
    /// Raw pointer motion on a surface, which keeps coming during a drag even outside it.
    PointerMoved(IcedId, Point),
    /// The left button was released over (or while grabbed by) a surface.
    PointerReleased(IcedId),
    /// A button was pressed on a surface, and no widget there took it.
    SurfacePressed(IcedId),
    DockPointerMoved(Point),
    DockPointerLeft,
    /// An animation frame for dock magnification.
    DockFrame(Instant),
    SurfaceOpened(IcedId, Size),
    SurfaceResized(IcedId, Size),
    /// The wallpaper finished uploading to the GPU.
    WallpaperLoaded(Result<iced_runtime::image::Allocation, iced_runtime::image::Error>),
    /// A surface's `wl_surface`, if it could be reached, to put material behind.
    SurfaceReached(IcedId, Option<WlSurface>),
    /// A menu bar title was pressed.
    BarPressed(menubar::Title),
    /// The pointer moved onto a menu bar title.
    BarEntered(menubar::Title),
    /// Something was chosen in the system menu.
    System(menubar::SystemAction),
    /// A system action was confirmed in its alert.
    SystemConfirmed(menubar::SystemAction),
    /// Time to check for news about the brightness behind the bar.
    BarBrightness,
    /// A dock icon was right-clicked; carries the app's id.
    DockMenuRequested(String),
    /// Something was chosen in a dock popup.
    DockMenu(dock::MenuAction),
    PlaceClicked(Place),
    PlaceMenuRequested(Place),
    /// The alert was answered: confirmed, or cancelled.
    AlertAnswered(bool),
    SurfaceClosed(IcedId),
    /// An app sent or took back a notification.
    Notification(notifications::Event),
    /// An animation frame for banners, or a check on whether one has expired.
    BannerFrame(Instant),
    NotificationEntered(u32),
    NotificationLeft(u32),
    /// A notification's card was pressed.
    NotificationPressed(u32),
    /// One of a notification's buttons was pressed; carries the action's key.
    NotificationAction(u32, String),
    /// A notification's ✕ was pressed.
    NotificationDismissed(u32),
    /// A group in Notification Center was opened, closed or cleared; carries the app's key.
    NotificationGroupOpened(String),
    NotificationGroupClosed(String),
    NotificationGroupCleared(String),
    /// The session's tray icons changed.
    TrayItems(Vec<tray::Item>),
    /// Something was chosen in a tray icon's menu: its service, menu and the item's id.
    TrayMenuItem(String, String, i32),
    /// A tray icon's menu should go to a submenu: its service, and the ids leading there.
    TraySubmenu(String, Vec<i32>),
    /// Scrolled over Notification Center, in rows.
    NotificationCenterScrolled(f32),
}

/// A popup that's open, or opening: a menu, or the Downloads stack.
struct Popup {
    id: IcedId,
    /// The menu bar title it hangs from, if any; otherwise it belongs to the dock.
    title: Option<menubar::Title>,
    content: PopupContent,
    size: (u32, u32),
    material: Option<MaterialSurface>,
    /// Shapes last sent, for content that changes while it's open.
    shapes: Vec<material::Shape>,
}

struct OpenAlert {
    id: IcedId,
    alert: Alert,
    material: Option<MaterialSurface>,
}

enum PopupContent {
    Menu(Vec<menu::Entry>),
    Downloads(Vec<stack::Tile>),
    /// Shows [`Shell::control`].
    ControlCenter,
    /// Shows [`Shell::notifications`].
    NotificationCenter,
}

struct Shell {
    surfaces: HashMap<IcedId, Surface>,
    /// A description of what each surface last showed, to tell when it needs a redraw.
    surface_contents: HashMap<IcedId, String>,
    wallpaper: Option<image::Handle>,
    /// Keeps the wallpaper on the GPU. Big images upload in the background and draw as nothing
    /// until they're ready, and the wallpaper doesn't otherwise redraw to catch up.
    wallpaper_loaded: Option<iced_runtime::image::Allocation>,
    apps: AppIndex,
    pinned: Vec<String>,
    /// The app of the focused window.
    focused_app: Option<Rc<App>>,
    clock: String,
    menubar_id: IcedId,
    /// Width of the menu bar surface; 0 until the compositor has configured it.
    menubar_width: f32,
    menubar_material: Option<MaterialSurface>,
    /// The distro logo as silhouettes, for dark and light backdrops.
    logo: Option<image::Handle>,
    logo_dark: Option<image::Handle>,
    /// How bright the background behind the bar is, if the compositor can tell.
    bar_backdrop: Option<menubar::Backdrop>,
    /// Whether the compositor reports the brightness behind the bar.
    bar_tracks_brightness: bool,
    keyboard_layouts: Option<KeyboardLayouts>,
    /// What the Control Center shows, refreshed when it opens.
    control: control::Panel,
    /// When the volume slider last set the volume, to throttle `wpctl` while dragging.
    volume_set_at: Option<Instant>,
    /// Short name of the active keyboard layout, when there's more than one.
    input_label: Option<String>,
    /// The distro logo in color, for alerts about the whole system.
    system_icon: Option<image::Handle>,
    /// The latest window list from KWin.
    windows: Vec<WindowInfo>,
    dock: Vec<dock::Item>,
    dock_id: IcedId,
    /// Width of the dock surface; 0 until the compositor has configured it.
    dock_width: f32,
    dock_pointer: dock::Pointer,
    /// What `dock_pointer.amount` is easing toward.
    dock_target: f32,
    dock_last_frame: Option<Instant>,
    /// Size of a fully magnified icon.
    dock_max_icon: f32,
    dock_layout: dock::Layout,
    material: Option<Rc<Material>>,
    dock_material: Option<MaterialSurface>,
    /// Shapes last sent for the dock, to skip redundant updates while icons animate.
    dock_shapes: Vec<material::Shape>,
    /// Latest pointer position over the dock surface.
    dock_cursor: Point,
    /// A press on a dock icon that hasn't become a click or a drag yet.
    dock_press: Option<DockPress>,
    dock_drag: Option<dock::Drag>,
    /// Apps launched from the dock that haven't opened a window yet, and when.
    launching: HashMap<String, Instant>,
    /// Shared start time for attention bounces, so they stay in step.
    started: Instant,
    popup: Option<Popup>,
    /// A menu bar title whose menu opens once the open popup is gone.
    next_bar_menu: Option<menubar::Title>,
    /// A tray icon's submenu that opens once the open popup is gone.
    next_submenu: Option<(menubar::Title, Vec<menu::Entry>)>,
    tray: Vec<tray::Icon>,
    alert: Option<OpenAlert>,
    /// Desktop entry id of the app that opens folders and the trash.
    file_manager: String,
    downloads_dir: PathBuf,
    trash_dir: PathBuf,
    /// How many items were in the trash at the last check.
    trash_count: usize,
    /// The current icon of each of [`Place::ALL`].
    place_icons: Vec<Option<image::Handle>>,
    /// Height of the screen, once the wallpaper's surface has said.
    screen_height: f32,
    /// What Notification Center holds, most recent first.
    notifications: Vec<Notification>,
    /// The notification the pointer is over.
    notification_hovered: Option<u32>,
    /// Groups opened in Notification Center, by app.
    open_groups: HashSet<String>,
    /// How far Notification Center is scrolled, in rows.
    center_scroll: f32,
    banners: Banners,
    banners_surface: Option<IcedId>,
    /// The surface was asked to close, and hasn't yet.
    banners_closing: bool,
    banners_material: Option<MaterialSurface>,
    banners_shapes: Vec<material::Shape>,
    banners_region: Vec<(i32, i32, i32, i32)>,
}

/// A dock icon a popup can be anchored to.
enum DockSlot {
    App(usize),
    /// An offset into [`Place::ALL`].
    Place(usize),
}

fn place_slot(place: Place) -> DockSlot {
    DockSlot::Place(Place::ALL.iter().position(|&p| p == place).unwrap_or(0))
}

struct DockPress {
    app_id: String,
    at: Point,
}

/// How far the pointer has to move with the button down before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 4.0;

/// How long a launch bounces before the dock gives up waiting for a window.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(8);

/// The screen's height until the wallpaper's surface says, if there is one.
const FALLBACK_SCREEN_HEIGHT: f32 = 700.0;
/// How many past notifications Notification Center keeps.
const MAX_NOTIFICATIONS: usize = 100;

impl Shell {
    fn new(material: Option<Rc<Material>>) -> (Self, Task<Message>) {
        let config = Config::load();
        let mut apps = AppIndex::load();
        let dock = dock::items(&config.dock.pinned, &[], &mut apps);

        let (menubar_id, open_menubar) = Message::layershell_open(menubar::settings());
        let dock_max_icon = if config.dock.magnification {
            config.dock.magnified_size.clamp(dock::ICON_SIZE, 128.0)
        } else {
            dock::ICON_SIZE
        };
        let (dock_id, open_dock) = Message::layershell_open(dock::settings(dock_max_icon));

        let file_manager = config
            .file_manager
            .clone()
            .or_else(|| apps::default_app("inode/directory"))
            .unwrap_or_default();
        let trash_dir = places::trash_dir();
        let trash_count = places::trash_count(&trash_dir);
        let configured_logo = config.menubar.logo.as_deref();
        let logo_names = menubar::logo_names(configured_logo, true);
        let logo = apps.silhouette(&logo_names, [255, 255, 255, 255]);
        let logo_dark = apps.silhouette(&logo_names, [0, 0, 0, 217]);
        let system_icon = menubar::logo_names(configured_logo, false)
            .iter()
            .find_map(|name| apps.themed_icon(name));

        let shell = Self {
            surfaces: HashMap::from([(menubar_id, Surface::MenuBar), (dock_id, Surface::Dock)]),
            surface_contents: HashMap::new(),
            wallpaper_loaded: None,
            wallpaper: config
                .desktop
                .wallpaper
                .as_deref()
                .and_then(wallpaper::load),
            apps,
            pinned: config.dock.pinned,
            focused_app: None,
            clock: menubar::format_clock(),
            menubar_id,
            menubar_width: 0.0,
            menubar_material: None,
            logo,
            logo_dark,
            bar_backdrop: None,
            bar_tracks_brightness: false,
            keyboard_layouts: None,
            control: control::Panel::default(),
            volume_set_at: None,
            input_label: None,
            system_icon,
            windows: Vec::new(),
            dock,
            dock_id,
            dock_width: 0.0,
            dock_pointer: dock::Pointer::default(),
            dock_target: 0.0,
            dock_last_frame: None,
            dock_max_icon,
            dock_layout: dock::Layout::default(),
            material,
            dock_material: None,
            dock_shapes: Vec::new(),
            dock_cursor: Point::ORIGIN,
            dock_press: None,
            dock_drag: None,
            launching: HashMap::new(),
            started: Instant::now(),
            popup: None,
            next_bar_menu: None,
            next_submenu: None,
            tray: Vec::new(),
            alert: None,
            file_manager,
            downloads_dir: places::downloads_dir(),
            trash_dir,
            trash_count,
            place_icons: Vec::new(),
            screen_height: FALLBACK_SCREEN_HEIGHT,
            notifications: Vec::new(),
            notification_hovered: None,
            open_groups: HashSet::new(),
            center_scroll: 0.0,
            banners: Banners::default(),
            banners_surface: None,
            banners_closing: false,
            banners_material: None,
            banners_shapes: Vec::new(),
            banners_region: Vec::new(),
        };
        let mut shell = shell;
        shell.update_place_icons();

        // Read now, so Control Center knows its size when it opens; the read takes a while.
        let mut opening = vec![
            open_menubar,
            open_dock,
            Task::perform(brightness::level(), Message::BrightnessRead),
        ];
        if shell.wallpaper.is_some() {
            let (id, open) = Message::layershell_open(wallpaper::settings());
            shell.surfaces.insert(id, Surface::Wallpaper);
            opening.push(open);
        }

        (shell, Task::batch(opening))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.apply(message);
        self.update_center_shapes();
        self.mark_changed_surfaces();
        task
    }

    /// Asks for a redraw of the surfaces whose content changed, so that a clock tick doesn't
    /// redraw the wallpaper.
    fn mark_changed_surfaces(&mut self) {
        let surfaces = &self.surfaces;
        self.surface_contents
            .retain(|id, _| surfaces.contains_key(id));

        for (&id, &surface) in &self.surfaces {
            let content = match surface {
                Surface::Wallpaper => self.wallpaper_loaded.is_some().to_string(),
                Surface::MenuBar => format!("{:?}", self.bar()),
                Surface::Dock => {
                    format!(
                        "{:?} {:?} {:?}",
                        self.dock, self.place_icons, self.dock_layout
                    )
                }
                // Short-lived, with live content.
                Surface::Popup | Surface::Alert | Surface::Banners => {
                    iced_layershell::redraw::mark_changed(id);
                    continue;
                }
            };
            if self.surface_contents.get(&id) != Some(&content) {
                iced_layershell::redraw::mark_changed(id);
                self.surface_contents.insert(id, content);
            }
        }
    }

    fn apply(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowsChanged(windows) => {
                self.focused_app = windows
                    .iter()
                    .find(|w| w.is_focused)
                    .and_then(|w| w.app_id.as_deref())
                    .map(|app_id| self.apps.resolve(app_id));
                self.windows = windows;
                self.dock = dock::items(&self.pinned, &self.windows, &mut self.apps);
                // A window showing up is what ends a launch bounce.
                let dock = &self.dock;
                self.launching.retain(|app_id, _| {
                    !dock
                        .iter()
                        .any(|item| item.app.id == *app_id && item.is_running())
                });
                return self.relayout_dock();
            }
            Message::KeyboardLayoutsChanged(layouts) => {
                self.input_label = (layouts.names.len() > 1)
                    .then(|| layouts.names.get(layouts.current))
                    .flatten()
                    .map(|name| menubar::input_label(name));
                self.control.keyboard = Some(layouts.clone());
                self.keyboard_layouts = Some(layouts);
            }
            // A popup's grab only closes it for clicks on other apps. The wallpaper, the bar and the
            // dock are ours, so they have to do it themselves.
            Message::SurfacePressed(id) => {
                let panel_open = matches!(
                    self.popup.as_ref().map(|popup| &popup.content),
                    Some(PopupContent::NotificationCenter | PopupContent::ControlCenter)
                );
                // The surfaces of these panels are mostly empty.
                if matches!(
                    self.surfaces.get(&id),
                    Some(Surface::Wallpaper | Surface::MenuBar | Surface::Dock)
                ) || (panel_open && self.surfaces.get(&id) == Some(&Surface::Popup))
                {
                    return self.close_popup();
                }
            }
            Message::NextLayout => {
                if let Some(layouts) = &self.keyboard_layouts {
                    let next = (layouts.current + 1) % layouts.names.len().max(1);
                    return Task::future(kwin::switch_layout(next)).discard();
                }
            }
            Message::Screenshot => {
                apps::spawn(vec![String::from("spectacle")]);
                return self.close_popup();
            }
            Message::LockScreen => {
                menubar::SystemAction::Lock.run();
                return self.close_popup();
            }
            Message::BrightnessChanged(level) => {
                self.control.brightness = Some(level);
                brightness::set_level(level);
            }
            Message::BrightnessReleased => {
                if let Some(level) = self.control.brightness {
                    brightness::set_level(level);
                }
            }
            Message::BrightnessRead(level) => self.control.brightness = level,
            Message::VolumeChanged(level) => {
                if let Some(volume) = &mut self.control.volume {
                    volume.level = level;
                    // Turning the volume up unmutes, like on macOS.
                    if std::mem::replace(&mut volume.muted, false) {
                        audio::set_muted(false);
                    }
                }
                // Dragging changes the level many times a second; wpctl needn't keep up.
                if self
                    .volume_set_at
                    .is_none_or(|at| at.elapsed() >= Duration::from_millis(40))
                {
                    self.volume_set_at = Some(Instant::now());
                    audio::set_level(level);
                }
            }
            Message::VolumeReleased => {
                if let Some(volume) = self.control.volume {
                    audio::set_level(volume.level);
                }
            }
            Message::ToggleMute => {
                if let Some(volume) = &mut self.control.volume {
                    volume.muted = !volume.muted;
                    audio::set_muted(volume.muted);
                }
            }
            Message::ToggleWifi => {
                if let Some(wifi) = &mut self.control.wifi {
                    wifi.enabled = !wifi.enabled;
                    let task = network::set_wifi_enabled(wifi.enabled);
                    return Task::perform(task, |_| Message::ControlRefresh);
                }
            }
            Message::ToggleBluetooth => {
                if let Some(bluetooth) = &mut self.control.bluetooth {
                    bluetooth.powered = !bluetooth.powered;
                    let task = bluetooth::set_powered(bluetooth.clone(), bluetooth.powered);
                    return Task::perform(task, |_| Message::ControlRefresh);
                }
            }
            Message::Media(control) => {
                if let Some(track) = &self.control.track {
                    let task = media::control(track.clone(), control);
                    return Task::perform(task, |_| Message::ControlRefresh);
                }
            }
            Message::ControlRefresh => {
                return Task::perform(control::status(), Message::ControlStatus);
            }
            Message::ControlStatus(status) => {
                let art = status.track.as_ref().and_then(|track| track.art.clone());
                if self
                    .control
                    .track
                    .as_ref()
                    .and_then(|track| track.art.as_ref())
                    != art.as_ref()
                {
                    self.control.art = art.map(image::Handle::from_path);
                }
                self.control.wifi = status.wifi;
                self.control.bluetooth = status.bluetooth;
                self.control.track = status.track;
            }
            Message::SwitchLayout(index) => {
                let switch = Task::future(kwin::switch_layout(index)).discard();
                return Task::batch([self.close_popup(), switch]);
            }
            Message::Tick => {
                self.clock = menubar::format_clock();
                let trash_count = places::trash_count(&self.trash_dir);
                if trash_count != self.trash_count {
                    self.trash_count = trash_count;
                    self.update_place_icons();
                }
            }
            Message::DockPressed(app_id) => {
                self.dock_press = Some(DockPress {
                    app_id,
                    at: self.dock_cursor,
                });
            }
            Message::PointerMoved(id, point) if id == self.dock_id => {
                self.dock_cursor = point;
                if let Some(drag) = &mut self.dock_drag {
                    drag.pointer = point;
                    return self.relayout_dock();
                }
                if let Some(press) = &self.dock_press
                    && press.at.distance(point) > DRAG_THRESHOLD
                {
                    return self.start_drag();
                }
            }
            Message::PointerReleased(id) if id == self.dock_id => {
                let Some(press) = self.dock_press.take() else {
                    return Task::none();
                };
                match self.dock_drag.take() {
                    Some(drag) => {
                        self.drop_dragged(&drag);
                        return self.relayout_dock();
                    }
                    None => return self.activate(&press.app_id),
                }
            }
            Message::DockPointerMoved(_) if self.dock_drag.is_some() => {}
            Message::DockPointerMoved(point) => {
                self.dock_pointer.base_x = Some(dock::base_x(&self.dock_layout, point.x));
                self.dock_pointer.hovered = dock::item_at(&self.dock_layout, point);
                if self.dock_max_icon > dock::ICON_SIZE {
                    self.dock_target = 1.0;
                }
                return self.relayout_dock();
            }
            Message::DockPointerLeft => {
                self.dock_pointer.hovered = None;
                self.dock_target = 0.0;
                return self.relayout_dock();
            }
            Message::DockFrame(now) => {
                self.launching
                    .retain(|_, started| now.duration_since(*started) < LAUNCH_TIMEOUT);

                let pointer = &mut self.dock_pointer;
                if pointer.amount == self.dock_target {
                    self.dock_last_frame = None;
                } else {
                    // Exponential easing, independent of how regularly frames arrive.
                    let dt = self
                        .dock_last_frame
                        .map_or(0.016, |last| (now - last).as_secs_f32())
                        .min(0.05);
                    self.dock_last_frame = Some(now);

                    pointer.amount +=
                        (self.dock_target - pointer.amount) * (1.0 - (-dt / 0.06).exp());
                    if (self.dock_target - pointer.amount).abs() < 0.005 {
                        pointer.amount = self.dock_target;
                    }
                    if pointer.amount == 0.0 {
                        pointer.base_x = None;
                    }
                }
                return self.relayout_dock();
            }
            Message::SurfaceOpened(id, size) if id == self.dock_id => {
                self.dock_width = size.width;
                let relayout = self.relayout_dock();

                // Attaching twice is a protocol error, so only ever do it once.
                if self.dock_material.is_none() {
                    return Task::batch([relayout, self.reach_surface(id)]);
                }
                return relayout;
            }
            Message::SurfaceOpened(id, size)
                if self.surfaces.get(&id) == Some(&Surface::Wallpaper) =>
            {
                self.screen_height = size.height;
                if let Some(handle) = &self.wallpaper {
                    return iced_runtime::image::allocate(handle.clone())
                        .map(Message::WallpaperLoaded);
                }
            }
            Message::WallpaperLoaded(Ok(allocation)) => self.wallpaper_loaded = Some(allocation),
            Message::WallpaperLoaded(Err(err)) => log::error!("can't load the wallpaper: {err}"),
            Message::SurfaceOpened(id, size) if id == self.menubar_id => {
                self.menubar_width = size.width;
                if self.menubar_material.is_none() {
                    return self.reach_surface(id);
                }
            }
            Message::SurfaceResized(id, size) if id == self.menubar_id => {
                self.menubar_width = size.width;
                if let (Some(material), Some(target)) = (&self.material, &self.menubar_material) {
                    material.set_shapes(target, &menubar::material_shapes(size.width));
                }
            }
            Message::SurfaceResized(id, size) if id == self.dock_id => {
                if size.width != self.dock_width {
                    self.dock_width = size.width;
                    return self.relayout_dock();
                }
            }
            Message::SurfaceReached(id, Some(surface)) => self.attach_material(id, surface),
            Message::SurfaceReached(_, None) => log::warn!("can't reach a wl_surface; no material"),
            Message::BarPressed(title) => return self.open_bar_menu(title),
            // Like on macOS, while a bar menu is open, pointing at another title switches to its
            // menu. The new popup can only take the grab once the old one is destroyed, so it
            // opens when the old one reports closed.
            Message::BarEntered(title) => {
                let has_menu = match title {
                    menubar::Title::Tray(index) => self
                        .tray
                        .get(index)
                        .is_some_and(|icon| icon.item.has_menu()),
                    _ => true,
                };
                if has_menu
                    && let Some(open) = self.popup.as_ref().and_then(|popup| popup.title)
                    && open != title
                {
                    self.next_bar_menu = Some(title);
                    return self.close_popup();
                }
            }
            Message::System(action) => {
                let close = self.close_popup();
                let Some(confirmation) = action.confirmation() else {
                    action.run();
                    return close;
                };
                let alert = Alert {
                    title: confirmation.title.to_owned(),
                    detail: String::from("Unsaved changes in open apps may be lost."),
                    confirm: confirmation.confirm.to_owned(),
                    on_confirm: Message::SystemConfirmed(action),
                    icon: self.system_icon.clone(),
                };
                return Task::batch([close, self.open_alert(alert)]);
            }
            Message::SystemConfirmed(action) => action.run(),
            Message::BarBrightness => {
                if let (Some(material), Some(target)) = (&self.material, &self.menubar_material)
                    && let Some(luminance) = material.brightness(target)
                {
                    let backdrop = menubar::Backdrop::from_luminance(self.bar_backdrop, luminance);
                    if self.bar_backdrop != Some(backdrop) {
                        log::debug!("menu bar backdrop {backdrop:?} at luminance {luminance}");
                        self.bar_backdrop = Some(backdrop);
                    }
                }
            }
            Message::DockMenuRequested(app_id) => return self.open_dock_menu(&app_id),
            Message::DockMenu(action) => {
                let close = self.close_popup();
                return Task::batch([close, self.run_dock_action(action)]);
            }
            Message::AlertAnswered(confirmed) => {
                let Some(open) = &self.alert else {
                    return Task::none();
                };
                let close = iced_runtime::window::close(open.id);
                if confirmed {
                    return Task::batch([close, Task::done(open.alert.on_confirm.clone())]);
                }
                return close;
            }
            Message::SurfaceOpened(id, _) if self.alert.as_ref().is_some_and(|a| a.id == id) => {
                return self.reach_surface(id);
            }
            Message::PlaceClicked(place) => match place {
                Place::Downloads => {
                    let tiles: Vec<stack::Tile> =
                        places::recent_downloads(&self.downloads_dir, stack::LIMIT)
                            .into_iter()
                            .map(|download| stack::Tile {
                                icon: self.apps.themed_icon(places::icon_name(&download)),
                                download,
                            })
                            .collect();
                    let size = stack::size(tiles.len());
                    return self.open_dock_popup(
                        place_slot(place),
                        PopupContent::Downloads(tiles),
                        size,
                    );
                }
                Place::Trash => self.open_place(Place::Trash),
            },
            Message::PlaceMenuRequested(place) => {
                let entries = dock::place_menu_entries(place, self.trash_count);
                let size = menu::size(&entries);
                return self.open_dock_popup(place_slot(place), PopupContent::Menu(entries), size);
            }
            Message::SurfaceOpened(id, _) if self.popup.as_ref().is_some_and(|p| p.id == id) => {
                return self.reach_surface(id);
            }
            Message::SurfaceOpened(id, _) if self.banners_surface == Some(id) => {
                self.banners_region.clear();
                return Task::batch([self.reach_surface(id), self.sync_banners()]);
            }
            Message::Notification(event) => return self.handle_notification(event),
            Message::BannerFrame(now) => {
                let over = self.banners.frame(now, self.notification_hovered);
                let mut tasks = vec![self.sync_banners()];
                tasks.extend(over.into_iter().map(|id| {
                    Task::future(notifications::report_closed(
                        id,
                        notifications::CLOSED_EXPIRED,
                    ))
                    .discard()
                }));
                return Task::batch(tasks);
            }
            Message::NotificationEntered(id) => {
                self.notification_hovered = Some(id);
                return self.sync_banners();
            }
            Message::NotificationLeft(id) => {
                if self.notification_hovered == Some(id) {
                    self.notification_hovered = None;
                }
                self.banners.pointer_left(id, Instant::now());
                return self.sync_banners();
            }
            Message::NotificationPressed(id) => {
                let Some(notification) = self.find_notification(id) else {
                    return Task::none();
                };
                let app_key = notification.app_key.clone();
                let has_default_action = notification.has_default_action;
                self.focus_app(&app_key);
                let action = has_default_action.then(|| {
                    Task::future(notifications::invoke_action(
                        id,
                        notifications::DEFAULT_ACTION.to_owned(),
                    ))
                    .discard()
                });
                let dismiss = self.dismiss_notification(id);
                let close = self.close_popup();
                return Task::batch(action.into_iter().chain([dismiss, close]));
            }
            Message::NotificationAction(id, key) => {
                let invoke = Task::future(notifications::invoke_action(id, key)).discard();
                return Task::batch([invoke, self.dismiss_notification(id)]);
            }
            Message::NotificationDismissed(id) => return self.dismiss_notification(id),
            Message::TrayItems(items) => {
                self.tray = items
                    .into_iter()
                    .map(|item| tray::Icon::new(item, &mut self.apps))
                    .collect();
            }
            Message::TrayMenuItem(service, menu_path, id) => {
                let chosen = Task::future(tray::menu_item_chosen(service, menu_path, id)).discard();
                return Task::batch([self.close_popup(), chosen]);
            }
            Message::TraySubmenu(service, path) => {
                let Some(index) = self
                    .tray
                    .iter()
                    .position(|icon| icon.item.service() == service)
                else {
                    return Task::none();
                };
                self.next_submenu = Some((
                    menubar::Title::Tray(index),
                    self.tray[index].item.entries(&path),
                ));
                return self.close_popup();
            }
            Message::NotificationGroupOpened(app_key) => {
                self.open_groups.insert(app_key);
            }
            Message::NotificationGroupClosed(app_key) => {
                self.open_groups.remove(&app_key);
            }
            Message::NotificationGroupCleared(app_key) => {
                let ids: Vec<u32> = self
                    .notifications
                    .iter()
                    .filter(|notification| notification.app_key == app_key)
                    .map(|notification| notification.id)
                    .collect();
                self.open_groups.remove(&app_key);
                return Task::batch(ids.into_iter().map(|id| self.dismiss_notification(id)));
            }
            Message::NotificationCenterScrolled(amount) => {
                let rows = notification_center::row_count(&self.notifications, &self.open_groups);
                self.center_scroll =
                    (self.center_scroll - amount).clamp(0.0, rows.saturating_sub(1) as f32);
            }
            Message::SurfaceClosed(id) => {
                self.surfaces.remove(&id);
                if self.popup.as_ref().is_some_and(|popup| popup.id == id)
                    && let Some(popup) = self.popup.take()
                {
                    if let Some(material) = popup.material {
                        material.destroy();
                    }
                    let relayout = self.relayout_dock();
                    if let Some((title, entries)) = self.next_submenu.take() {
                        let size = menu::size(&entries);
                        let open = self.open_bar_popup(title, size, PopupContent::Menu(entries));
                        return Task::batch([relayout, open]);
                    }
                    return match self.next_bar_menu.take() {
                        Some(title) => Task::batch([relayout, self.open_bar_menu(title)]),
                        None => relayout,
                    };
                }
                if self.alert.as_ref().is_some_and(|open| open.id == id)
                    && let Some(open) = self.alert.take()
                    && let Some(material) = open.material
                {
                    material.destroy();
                }
                if self.banners_surface == Some(id) {
                    self.banners_surface = None;
                    self.banners_closing = false;
                    if let Some(material) = self.banners_material.take() {
                        material.destroy();
                    }
                    // Something new came in while it was closing.
                    return self.sync_banners();
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn open_dock_menu(&mut self, app_id: &str) -> Task<Message> {
        let Some(index) = self.dock.iter().position(|item| item.app.id == app_id) else {
            return Task::none();
        };
        let pinned = self.pinned.iter().any(|id| id == app_id);
        let entries = dock::menu_entries(&self.dock[index], pinned);
        let size = menu::size(&entries);
        self.open_dock_popup(DockSlot::App(index), PopupContent::Menu(entries), size)
    }

    /// Opens a popup above a dock icon.
    fn open_dock_popup(
        &mut self,
        slot: DockSlot,
        content: PopupContent,
        size: (u32, u32),
    ) -> Task<Message> {
        let index = match slot {
            DockSlot::App(index) => index,
            DockSlot::Place(offset) => self.dock_layout.apps + offset,
        };
        let Some(anchor) = dock::menu_anchor(&self.dock_layout, index) else {
            return Task::none();
        };
        let settings = IcedNewPopupSettings::new(self.dock_id, size, anchor)
            .anchor(PopupAnchor::Top)
            .gravity(PopupGravity::Top);
        self.open_popup(None, settings, content)
    }

    /// Opens a menu bar title's menu under it.
    fn open_bar_menu(&mut self, title: menubar::Title) -> Task<Message> {
        let entries = match title {
            menubar::Title::System => menubar::system_menu(),
            menubar::Title::App => {
                let Some(app) = &self.focused_app else {
                    return Task::none();
                };
                let windows = self
                    .dock
                    .iter()
                    .find(|item| item.app.id == app.id)
                    .map(dock::Item::window_ids)
                    .unwrap_or_default();
                menubar::app_menu(app, windows)
            }
            menubar::Title::Input => match &self.keyboard_layouts {
                Some(layouts) => menubar::input_menu(layouts),
                None => return Task::none(),
            },
            menubar::Title::ControlCenter => {
                self.control.volume = audio::volume();
                let reread = Task::perform(brightness::level(), Message::BrightnessRead);
                let anchor = self.bar().screen_edge_anchor();
                let settings = IcedNewPopupSettings::new(
                    self.menubar_id,
                    control::size(&self.control),
                    anchor,
                )
                .anchor(PopupAnchor::BottomRight)
                .gravity(PopupGravity::BottomLeft);
                let open = self.open_popup(Some(title), settings, PopupContent::ControlCenter);
                return Task::batch([open, reread, Task::done(Message::ControlRefresh)]);
            }
            menubar::Title::Tray(index) => {
                let Some(icon) = self.tray.get(index) else {
                    return Task::none();
                };
                if !icon.item.has_menu() {
                    return Task::future(icon.item.activate(0, 0)).discard();
                }
                icon.item.entries(&[])
            }
            menubar::Title::Clock => {
                self.center_scroll = 0.0;
                self.open_groups.clear();
                let size = (
                    notification_center::WIDTH as u32,
                    notification_center::height(self.screen_height) as u32,
                );
                let settings = IcedNewPopupSettings::new(
                    self.menubar_id,
                    size,
                    self.bar().right_edge_anchor(),
                )
                .anchor(PopupAnchor::BottomRight)
                .gravity(PopupGravity::BottomLeft);
                let open = self.open_popup(Some(title), settings, PopupContent::NotificationCenter);
                // Everything's in the panel now, and a banner would sit on top of it.
                self.banners.dismiss_all();
                return Task::batch([open, self.sync_banners()]);
            }
        };
        let size = menu::size(&entries);
        self.open_bar_popup(title, size, PopupContent::Menu(entries))
    }

    /// Opens a popup under a menu bar title.
    fn open_bar_popup(
        &mut self,
        title: menubar::Title,
        size: (u32, u32),
        content: PopupContent,
    ) -> Task<Message> {
        let Some(anchor) = self.bar().menu_anchor(title) else {
            return Task::none();
        };
        let settings = IcedNewPopupSettings::new(self.menubar_id, size, anchor)
            .anchor(PopupAnchor::BottomLeft)
            .gravity(PopupGravity::BottomRight);
        self.open_popup(Some(title), settings, content)
    }

    /// Opens a popup, unless one is open: then it closes that one instead.
    fn open_popup(
        &mut self,
        title: Option<menubar::Title>,
        settings: IcedNewPopupSettings,
        content: PopupContent,
    ) -> Task<Message> {
        // xdg-shell only lets the topmost popup grab, so a new popup can't open until the old
        // one is gone. Close it, and let the next click open the new one.
        if self.popup.is_some() {
            return self.close_popup();
        }

        let id = IcedId::unique();
        self.surfaces.insert(id, Surface::Popup);
        self.popup = Some(Popup {
            id,
            title,
            content,
            size: settings.size,
            material: None,
            shapes: Vec::new(),
        });
        Task::batch([
            self.relayout_dock(),
            Task::done(Message::NewPopUp { settings, id }),
        ])
    }

    /// Reaches the `wl_surface` behind an iced surface, to put material behind it.
    fn reach_surface(&self, id: IcedId) -> Task<Message> {
        let Some(material) = &self.material else {
            return Task::none();
        };
        let connection = material.connection().clone();
        iced_runtime::window::run(id, move |window| material::surface_of(&connection, window))
            .map(move |surface| Message::SurfaceReached(id, surface))
    }

    fn attach_material(&mut self, id: IcedId, surface: WlSurface) {
        let Some(material) = &self.material else {
            return;
        };
        let target = material.attach(surface);
        match self.surfaces.get(&id) {
            Some(Surface::MenuBar) => {
                self.bar_tracks_brightness = material.track_brightness(&target);
                material.set_shapes(&target, &menubar::material_shapes(self.menubar_width));
                self.menubar_material = Some(target);
            }
            Some(Surface::Dock) => {
                self.dock_shapes = dock::material_shapes(&self.dock_layout);
                material.set_shapes(&target, &self.dock_shapes);
                self.dock_material = Some(target);
            }
            Some(Surface::Popup) => match &mut self.popup {
                Some(popup) if popup.id == id => {
                    let shapes = match popup.content {
                        PopupContent::Menu(_) => menu::material_shapes(popup.size),
                        PopupContent::Downloads(_) => stack::material_shapes(popup.size),
                        PopupContent::ControlCenter => control::material_shapes(&self.control),
                        PopupContent::NotificationCenter => notification_center::material_shapes(
                            &self.notifications,
                            &self.open_groups,
                            self.center_scroll as usize,
                            self.notification_hovered,
                            popup.size.1 as f32,
                        ),
                    };
                    material.set_shapes(&target, &shapes);
                    popup.shapes = shapes;
                    popup.material = Some(target);
                }
                _ => target.destroy(),
            },
            Some(Surface::Alert) => match &mut self.alert {
                Some(open) if open.id == id => {
                    material.set_shapes(&target, &alert::material_shapes());
                    open.material = Some(target);
                }
                _ => target.destroy(),
            },
            Some(Surface::Banners) => {
                self.banners_shapes = self.banners.material_shapes(self.notification_hovered);
                material.set_shapes(&target, &self.banners_shapes);
                self.banners_material = Some(target);
            }
            // Closed before it could be reached, or not a surface with material.
            Some(Surface::Wallpaper) | None => target.destroy(),
        }
    }

    fn bar(&self) -> menubar::Bar<'_> {
        menubar::Bar {
            width: self.menubar_width,
            app_name: self.app_name(),
            clock: &self.clock,
            logo: match self.bar_backdrop {
                Some(menubar::Backdrop::Light) => self.logo_dark.as_ref(),
                _ => self.logo.as_ref(),
            },
            open: self.popup.as_ref().and_then(|popup| popup.title),
            backdrop: self.bar_backdrop,
            input: self.input_label.as_deref(),
            tray: &self.tray,
        }
    }

    fn find_notification(&self, id: u32) -> Option<&Notification> {
        self.notifications
            .iter()
            .find(|notification| notification.id == id)
            .or_else(|| self.banners.find(id))
    }

    fn handle_notification(&mut self, event: notifications::Event) -> Task<Message> {
        let now = Instant::now();
        match event {
            notifications::Event::Notified(incoming) => {
                let replaces = incoming.replaces;
                let notification = Notification::new(incoming, &mut self.apps);
                let known = replaces
                    && self
                        .notifications
                        .iter_mut()
                        .find(|known| known.id == notification.id)
                        .map(|known| *known = notification.clone())
                        .is_some();
                if !known && !notification.transient {
                    self.notifications.insert(0, notification.clone());
                    self.notifications.truncate(MAX_NOTIFICATIONS);
                }
                let center_open = matches!(
                    self.popup.as_ref().map(|popup| &popup.content),
                    Some(PopupContent::NotificationCenter)
                );
                if !self.banners.replace(&notification, now)
                    && (!center_open || notification.transient)
                {
                    self.banners.show(notification, now);
                }
            }
            notifications::Event::Closed(id) => {
                self.notifications.retain(|known| known.id != id);
                self.banners.dismiss(id);
            }
        }
        self.sync_banners()
    }

    /// Takes a notification away, and tells its app.
    fn dismiss_notification(&mut self, id: u32) -> Task<Message> {
        self.notifications.retain(|known| known.id != id);
        self.banners.dismiss(id);
        if self.notification_hovered == Some(id) {
            self.notification_hovered = None;
        }
        Task::batch([
            self.sync_banners(),
            Task::future(notifications::report_closed(
                id,
                notifications::CLOSED_DISMISSED,
            ))
            .discard(),
        ])
    }

    /// Brings the app's most recently used window forward, if it has one.
    fn focus_app(&mut self, app_key: &str) {
        let window = self
            .windows
            .iter()
            .filter(|window| {
                window.app_id.as_deref().is_some_and(|app_id| {
                    app_id.eq_ignore_ascii_case(app_key) || self.apps.resolve(app_id).id == app_key
                })
            })
            .max_by_key(|window| window.focused_at);
        if let Some(window) = window {
            kwin::focus_window(window.id);
        }
    }

    /// Opens or closes the banners' surface as needed, and keeps its glass and input region with
    /// the banners.
    fn sync_banners(&mut self) -> Task<Message> {
        let Some(id) = self.banners_surface else {
            if self.banners.is_empty() {
                return Task::none();
            }
            let (id, open) = Message::layershell_open(banners::settings());
            self.surfaces.insert(id, Surface::Banners);
            self.banners_surface = Some(id);
            return open;
        };
        if self.banners.is_empty() {
            if std::mem::replace(&mut self.banners_closing, true) {
                return Task::none();
            }
            return iced_runtime::window::close(id);
        }

        let shapes = self.banners.material_shapes(self.notification_hovered);
        if shapes != self.banners_shapes
            && let (Some(material), Some(target)) = (&self.material, &self.banners_material)
        {
            material.set_shapes(target, &shapes);
            self.banners_shapes = shapes;
        }

        let region = self.banners.input_region();
        if region == self.banners_region {
            return Task::none();
        }
        self.banners_region.clone_from(&region);
        Task::done(Message::SetInputRegion {
            id,
            callback: ActionCallback::new(move |wl_region| {
                for &(x, y, width, height) in &region {
                    wl_region.add(x, y, width, height);
                }
            }),
        })
    }

    /// Keeps Notification Center's glass under its cards.
    fn update_center_shapes(&mut self) {
        let Some(popup) = &mut self.popup else {
            return;
        };
        if !matches!(popup.content, PopupContent::NotificationCenter) {
            return;
        }
        let shapes = notification_center::material_shapes(
            &self.notifications,
            &self.open_groups,
            self.center_scroll as usize,
            self.notification_hovered,
            popup.size.1 as f32,
        );
        if shapes != popup.shapes
            && let (Some(material), Some(target)) = (&self.material, &popup.material)
        {
            material.set_shapes(target, &shapes);
            popup.shapes = shapes;
        }
    }

    /// What the menu bar calls the focused app; "Desktop" when nothing is focused.
    fn app_name(&self) -> &str {
        self.focused_app.as_ref().map_or("Desktop", |app| &app.name)
    }

    /// Opens an alert, unless one is already up.
    fn open_alert(&mut self, alert: Alert) -> Task<Message> {
        if self.alert.is_some() {
            return Task::none();
        }
        let (id, open) = Message::layershell_open(alert::settings());
        self.surfaces.insert(id, Surface::Alert);
        self.alert = Some(OpenAlert {
            id,
            alert,
            material: None,
        });
        open
    }

    /// Closes the open popup, if any. Its state is cleaned up once the surface is gone.
    fn close_popup(&mut self) -> Task<Message> {
        match &self.popup {
            Some(popup) => iced_runtime::window::close(popup.id),
            None => Task::none(),
        }
    }

    /// Opens a place in the file manager.
    fn open_place(&self, place: Place) {
        let uri = match place {
            Place::Downloads => places::file_uri(&self.downloads_dir),
            Place::Trash => String::from("trash:///"),
        };
        match self.apps.command_with_uris(&self.file_manager, &[&uri]) {
            Some(command) => apps::spawn(command),
            None => apps::spawn(vec![String::from("xdg-open"), uri]),
        }
    }

    fn update_place_icons(&mut self) {
        let trash = if self.trash_count == 0 {
            "user-trash"
        } else {
            "user-trash-full"
        };
        self.place_icons = Place::ALL
            .iter()
            .map(|place| match place {
                Place::Downloads => self.apps.themed_icon("folder-download"),
                Place::Trash => self.apps.themed_icon(trash),
            })
            .collect();
    }

    fn start_drag(&mut self) -> Task<Message> {
        let Some(press) = &self.dock_press else {
            return Task::none();
        };
        let Some(index) = self
            .dock
            .iter()
            .position(|item| item.app.id == press.app_id)
        else {
            return Task::none();
        };
        let Some(grab) = dock::grab_offset(&self.dock_layout, index, press.at) else {
            return Task::none();
        };

        self.dock_drag = Some(dock::Drag {
            index,
            pointer: self.dock_cursor,
            grab,
            removable: self.pinned.contains(&press.app_id),
        });
        // Icons are dragged at their plain size, so magnification snaps off.
        self.dock_pointer = dock::Pointer::default();
        self.dock_target = 0.0;
        self.relayout_dock()
    }

    /// Applies what the layout says releasing `drag` does, and saves the pinned apps.
    fn drop_dragged(&mut self, drag: &dock::Drag) {
        let Some(item) = self.dock.get(drag.index) else {
            return;
        };
        let app_id = item.app.id.clone();

        match self.dock_layout.drop {
            Some(dock::Drop::Remove) => self.pinned.retain(|id| *id != app_id),
            // Dropping an icon somewhere keeps it there, like on macOS: the dock's new order,
            // filtered down to pinned apps (now including this one), becomes the pinned list.
            Some(dock::Drop::Slot(slot)) => {
                let mut order: Vec<String> = self
                    .dock
                    .iter()
                    .enumerate()
                    .filter(|&(index, _)| index != drag.index)
                    .map(|(_, item)| item.app.id.clone())
                    .collect();
                order.insert(slot.min(order.len()), app_id.clone());

                let mut pinned: HashSet<String> = self.pinned.drain(..).collect();
                pinned.insert(app_id);
                self.pinned = order.into_iter().filter(|id| pinned.contains(id)).collect();
            }
            None => return,
        }

        if let Err(err) = config::save_pinned(&self.pinned) {
            log::error!("can't save pinned apps: {err}");
        }
        self.dock = dock::items(&self.pinned, &self.windows, &mut self.apps);
    }

    /// Focuses the app, or launches it with a bounce if it isn't running.
    fn activate(&mut self, app_id: &str) -> Task<Message> {
        let Some(item) = self.dock.iter().find(|item| item.app.id == app_id) else {
            return Task::none();
        };
        if !item.is_running() && item.app.command.is_some() {
            self.launching.insert(app_id.to_owned(), Instant::now());
        }
        dock::activate(item);
        self.relayout_dock()
    }

    fn run_dock_action(&mut self, action: dock::MenuAction) -> Task<Message> {
        match action {
            dock::MenuAction::FocusWindow(id) => kwin::focus_window(id),
            dock::MenuAction::Open(app_id) => return self.activate(&app_id),
            dock::MenuAction::Launch(command) => apps::spawn(command),
            dock::MenuAction::Quit(ids) => ids.into_iter().for_each(kwin::close_window),
            dock::MenuAction::TogglePin(app_id) => {
                if let Some(index) = self.pinned.iter().position(|id| *id == app_id) {
                    self.pinned.remove(index);
                } else {
                    self.pinned.push(app_id);
                }
                if let Err(err) = config::save_pinned(&self.pinned) {
                    log::error!("can't save pinned apps: {err}");
                }
                self.dock = dock::items(&self.pinned, &self.windows, &mut self.apps);
            }
            dock::MenuAction::OpenPath(path) => {
                apps::spawn(vec![String::from("xdg-open"), path.display().to_string()]);
            }
            dock::MenuAction::OpenPlace(place) => self.open_place(place),
            dock::MenuAction::ConfirmEmptyTrash => {
                let items = if self.trash_count == 1 {
                    "item"
                } else {
                    "items"
                };
                let alert = Alert {
                    title: format!(
                        "Are you sure you want to permanently erase the {} {items} in the Trash?",
                        self.trash_count
                    ),
                    detail: String::from("You can't undo this action."),
                    confirm: String::from("Empty Trash"),
                    on_confirm: Message::DockMenu(dock::MenuAction::EmptyTrash),
                    icon: self.apps.themed_icon("user-trash-full"),
                };
                return self.open_alert(alert);
            }
            dock::MenuAction::EmptyTrash => {
                if let Err(err) = places::empty_trash(&self.trash_dir) {
                    log::error!("can't empty the trash: {err}");
                }
                self.trash_count = places::trash_count(&self.trash_dir);
                self.update_place_icons();
            }
        }
        Task::none()
    }

    /// Recomputes the dock layout, and pushes whatever changed to the material and the input
    /// region.
    fn relayout_dock(&mut self) -> Task<Message> {
        // Nothing to place until the compositor has given the surface a size.
        if self.dock_width == 0.0 {
            return Task::none();
        }

        let now = Instant::now();
        let bounce = self
            .dock
            .iter()
            .map(|item| match self.launching.get(&item.app.id) {
                Some(started) => dock::launch_bounce(now.duration_since(*started).as_secs_f32()),
                None if item.wants_attention() => {
                    dock::attention_bounce(now.duration_since(self.started).as_secs_f32())
                }
                None => 0.0,
            })
            .collect();

        let mut layout = dock::layout(
            self.dock_width,
            &self.dock,
            &self.dock_pointer,
            self.dock_max_icon,
            bounce,
            self.dock_drag.as_ref(),
        );
        // Like macOS, an open menu or stack hides the name over the icon. It would show
        // through the popup's clear glass anyway.
        if self.popup.is_some() {
            layout.label = None;
        }
        if layout == self.dock_layout {
            return Task::none();
        }
        let input_region = dock::input_region(&layout);
        let input_changed = input_region != dock::input_region(&self.dock_layout);
        self.dock_layout = layout;

        let shapes = dock::material_shapes(&self.dock_layout);
        if shapes != self.dock_shapes
            && let (Some(material), Some(target)) = (&self.material, &self.dock_material)
        {
            material.set_shapes(target, &shapes);
            self.dock_shapes = shapes;
        }

        if !input_changed {
            return Task::none();
        }
        Task::done(Message::SetInputRegion {
            id: self.dock_id,
            callback: ActionCallback::new(move |region| {
                for &(x, y, width, height) in &input_region {
                    region.add(x, y, width, height);
                }
            }),
        })
    }

    fn view(&self, id: IcedId) -> Element<'_> {
        match self.surfaces.get(&id) {
            Some(Surface::Wallpaper) => match &self.wallpaper {
                Some(handle) => wallpaper::view(handle),
                None => Space::new().into(),
            },
            Some(Surface::MenuBar) => menubar::view(&self.bar()),
            Some(Surface::Dock) => dock::view(
                &self.dock,
                &self.place_icons,
                &self.dock_layout,
                self.material.as_ref().map(|m| m.kind()),
            ),
            Some(Surface::Popup) => {
                let material = self.material.as_ref().map(|m| m.kind());
                match self.popup.as_ref().map(|popup| &popup.content) {
                    Some(PopupContent::Menu(entries)) => menu::view(entries, material),
                    Some(PopupContent::Downloads(tiles)) => stack::view(tiles, material),
                    Some(PopupContent::ControlCenter) => control::view(&self.control, material),
                    Some(PopupContent::NotificationCenter) => notification_center::view(
                        &self.notifications,
                        &self.open_groups,
                        self.center_scroll as usize,
                        self.notification_hovered,
                        self.popup.as_ref().map_or(0.0, |popup| popup.size.1 as f32),
                        material,
                    ),
                    None => Space::new().into(),
                }
            }
            Some(Surface::Alert) => match &self.alert {
                Some(open) => alert::view(&open.alert, self.material.as_ref().map(|m| m.kind())),
                None => Space::new().into(),
            },
            Some(Surface::Banners) => self.banners.view(
                self.notification_hovered,
                self.material.as_ref().map(|m| m.kind()),
            ),
            None => Space::new().into(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Only tick while something in the dock moves.
        let animating = self.dock_pointer.amount != self.dock_target
            || !self.launching.is_empty()
            || self.dock.iter().any(dock::Item::wants_attention);
        let dock_frames = if animating {
            time::every(Duration::from_millis(16)).map(Message::DockFrame)
        } else {
            Subscription::none()
        };

        // What the Control Center shows can change under it, like a track ending.
        let control_open = matches!(
            self.popup.as_ref().map(|popup| &popup.content),
            Some(PopupContent::ControlCenter)
        );
        let control_refresh = if control_open {
            time::every(Duration::from_secs(2)).map(|_| Message::ControlRefresh)
        } else {
            Subscription::none()
        };

        // Brightness events arrive on the material's queue, which nothing else dispatches.
        let bar_brightness = if self.bar_tracks_brightness {
            time::every(Duration::from_millis(250)).map(|_| Message::BarBrightness)
        } else {
            Subscription::none()
        };

        // Banners take frames while they move, and otherwise only need watching to expire.
        let banner_frames = if self.banners.animating() {
            time::every(Duration::from_millis(16)).map(Message::BannerFrame)
        } else if self.banners.counting_down() {
            time::every(Duration::from_millis(250)).map(Message::BannerFrame)
        } else {
            Subscription::none()
        };

        Subscription::batch([
            dock_frames,
            banner_frames,
            notifications::events().map(Message::Notification),
            tray::items().map(Message::TrayItems),
            bar_brightness,
            control_refresh,
            kwin::windows().map(Message::WindowsChanged),
            kwin::keyboard_layouts().map(Message::KeyboardLayoutsChanged),
            time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            iced_futures::event::listen_with(|event, status, id| match event {
                Event::Window(window::Event::Opened { size, .. }) => {
                    Some(Message::SurfaceOpened(id, size))
                }
                Event::Window(window::Event::Resized(size)) => {
                    Some(Message::SurfaceResized(id, size))
                }
                Event::Window(window::Event::Closed) => Some(Message::SurfaceClosed(id)),
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::PointerMoved(id, position))
                }
                Event::Mouse(mouse::Event::ButtonPressed(_))
                    if status == event::Status::Ignored =>
                {
                    Some(Message::SurfacePressed(id))
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::PointerReleased(id))
                }
                _ => None,
            }),
        ])
    }
}

fn namespace() -> String {
    String::from("macshell")
}

/// Surfaces are transparent; each view draws its own background.
fn style(_: &Shell, _: &Theme) -> theme::Style {
    theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    }
}
