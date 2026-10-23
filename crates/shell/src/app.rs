//! The shell process: owns every layer surface and the state they share.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced_futures::Subscription;
use iced_futures::backend::native::tokio::time;
use iced_layershell::actions::{ActionCallback, IcedNewPopupSettings};
use iced_layershell::daemon;
use iced_layershell::reexport::{IcedId, PopupAnchor, PopupGravity, Task, WithConnection};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::Space;
use iced_widget::core::{Color, Event, Point, Size, Theme, mouse, theme, window};
use iced_widget::renderer::Renderer;
use wayland_client::Connection;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::apps::AppIndex;
use crate::config::{self, Config};
use crate::material::{self, Material, MaterialSurface};
use crate::niri::{self, WindowInfo};
use crate::{dock, menu, menubar};

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
    MenuBar,
    Dock,
    DockMenu,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    WindowsChanged(Vec<WindowInfo>),
    Tick,
    /// A dock icon was pressed; carries the app's id. It becomes a click or a drag.
    DockPressed(String),
    /// Raw pointer motion on a surface, which keeps coming during a drag even outside it.
    PointerMoved(IcedId, Point),
    /// The left button was released over (or while grabbed by) a surface.
    PointerReleased(IcedId),
    DockPointerMoved(Point),
    DockPointerLeft,
    /// An animation frame for dock magnification.
    DockFrame(Instant),
    SurfaceOpened(IcedId, Size),
    SurfaceResized(IcedId, Size),
    /// The dock's `wl_surface`, if it could be reached.
    DockSurface(Option<WlSurface>),
    /// A dock icon was right-clicked; carries the app's id.
    DockMenuRequested(String),
    DockMenu(dock::MenuAction),
    /// The open menu's `wl_surface`, if it could be reached.
    MenuSurface(Option<WlSurface>),
    SurfaceClosed(IcedId),
}

/// A popup menu that's open, or opening.
struct OpenMenu {
    id: IcedId,
    entries: Vec<menu::Entry>,
    size: (u32, u32),
    material: Option<MaterialSurface>,
}

struct Shell {
    surfaces: HashMap<IcedId, Surface>,
    apps: AppIndex,
    pinned: Vec<String>,
    dock_style: material::Style,
    focused_app: Option<String>,
    clock: String,
    /// The latest window list from niri.
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
    menu: Option<OpenMenu>,
}

struct DockPress {
    app_id: String,
    at: Point,
}

/// How far the pointer has to move with the button down before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 4.0;

/// How long a launch bounces before the dock gives up waiting for a window.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(8);

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

        let shell = Self {
            surfaces: HashMap::from([(menubar_id, Surface::MenuBar), (dock_id, Surface::Dock)]),
            apps,
            pinned: config.dock.pinned,
            dock_style: config.dock.glass,
            focused_app: None,
            clock: menubar::format_clock(),
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
            menu: None,
        };

        (shell, Task::batch([open_menubar, open_dock]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowsChanged(windows) => {
                self.focused_app = windows
                    .iter()
                    .find(|w| w.is_focused)
                    .and_then(|w| w.app_id.as_deref())
                    .map(|app_id| self.apps.resolve(app_id).name.clone());
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
            Message::Tick => self.clock = menubar::format_clock(),
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
                if self.dock_material.is_none()
                    && let Some(material) = &self.material
                {
                    let connection = material.connection().clone();
                    let attach = iced_runtime::window::run(id, move |window| {
                        material::surface_of(&connection, window)
                    })
                    .map(Message::DockSurface);
                    return Task::batch([relayout, attach]);
                }
                return relayout;
            }
            Message::SurfaceResized(id, size) if id == self.dock_id => {
                if size.width != self.dock_width {
                    self.dock_width = size.width;
                    return self.relayout_dock();
                }
            }
            Message::DockSurface(Some(surface)) => {
                if let Some(material) = &self.material {
                    let target = material.attach(surface);
                    self.dock_shapes = dock::material_shapes(&self.dock_layout, self.dock_style);
                    material.set_shapes(&target, &self.dock_shapes);
                    self.dock_material = Some(target);
                }
            }
            Message::DockSurface(None) => {
                log::warn!("can't reach the dock's wl_surface; no material")
            }
            Message::DockMenuRequested(app_id) => return self.open_dock_menu(&app_id),
            Message::DockMenu(action) => {
                self.run_dock_action(action);
                return self.close_menu();
            }
            Message::SurfaceOpened(id, _) if self.menu.as_ref().is_some_and(|m| m.id == id) => {
                if let Some(material) = &self.material {
                    let connection = material.connection().clone();
                    return iced_runtime::window::run(id, move |window| {
                        material::surface_of(&connection, window)
                    })
                    .map(Message::MenuSurface);
                }
            }
            Message::MenuSurface(Some(surface)) => {
                if let (Some(material), Some(menu)) = (&self.material, &mut self.menu) {
                    let target = material.attach(surface);
                    material.set_shapes(&target, &menu::material_shapes(menu.size));
                    menu.material = Some(target);
                }
            }
            Message::MenuSurface(None) => log::warn!("can't reach the menu's wl_surface"),
            Message::SurfaceClosed(id) => {
                self.surfaces.remove(&id);
                if self.menu.as_ref().is_some_and(|menu| menu.id == id)
                    && let Some(menu) = self.menu.take()
                    && let Some(material) = menu.material
                {
                    material.destroy();
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
        let Some(anchor) = dock::menu_anchor(&self.dock_layout, index) else {
            return Task::none();
        };

        let pinned = self.pinned.iter().any(|id| id == app_id);
        let entries = dock::menu_entries(&self.dock[index], pinned);
        let size = menu::size(&entries);

        // One menu at a time: replace any that's open.
        let close = self.close_menu();

        let id = IcedId::unique();
        self.surfaces.insert(id, Surface::DockMenu);
        self.menu = Some(OpenMenu {
            id,
            entries,
            size,
            material: None,
        });

        let settings = IcedNewPopupSettings::new(self.dock_id, size, anchor)
            .anchor(PopupAnchor::Top)
            .gravity(PopupGravity::Top);
        Task::batch([close, Task::done(Message::NewPopUp { settings, id })])
    }

    /// Closes the open menu, if any. Its state is cleaned up once the surface is gone.
    fn close_menu(&mut self) -> Task<Message> {
        match &self.menu {
            Some(menu) => iced_runtime::window::close(menu.id),
            None => Task::none(),
        }
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

    fn run_dock_action(&mut self, action: dock::MenuAction) {
        match action {
            dock::MenuAction::FocusWindow(id) => niri::focus_window(id),
            dock::MenuAction::Open(app_id) => {
                // The relayout happens on the next frame, which the bounce schedules.
                let _ = self.activate(&app_id);
            }
            dock::MenuAction::Launch(command) => niri::spawn(command),
            dock::MenuAction::Quit(ids) => ids.into_iter().for_each(niri::close_window),
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
        }
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

        let layout = dock::layout(
            self.dock_width,
            &self.dock,
            &self.dock_pointer,
            self.dock_max_icon,
            bounce,
            self.dock_drag.as_ref(),
        );
        if layout == self.dock_layout {
            return Task::none();
        }
        let input_region = dock::input_region(&layout);
        let input_changed = input_region != dock::input_region(&self.dock_layout);
        self.dock_layout = layout;

        let shapes = dock::material_shapes(&self.dock_layout, self.dock_style);
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
            Some(Surface::MenuBar) => menubar::view(self.focused_app.as_deref(), &self.clock),
            Some(Surface::Dock) => dock::view(
                &self.dock,
                &self.dock_layout,
                self.material.as_ref().map(|m| m.kind()),
            ),
            Some(Surface::DockMenu) => match &self.menu {
                Some(menu) => menu::view(&menu.entries, self.material.as_ref().map(|m| m.kind())),
                None => Space::new().into(),
            },
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

        Subscription::batch([
            dock_frames,
            niri::windows().map(Message::WindowsChanged),
            time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            iced_futures::event::listen_with(|event, _, id| match event {
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
