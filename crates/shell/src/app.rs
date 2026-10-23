//! The shell process: owns every layer surface and the state they share.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced_futures::Subscription;
use iced_futures::backend::native::tokio::time;
use iced_layershell::actions::ActionCallback;
use iced_layershell::daemon;
use iced_layershell::reexport::{IcedId, Task, WithConnection};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::Space;
use iced_widget::core::{Color, Event, Point, Size, Theme, theme, window};
use iced_widget::renderer::Renderer;
use wayland_client::Connection;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::apps::AppIndex;
use crate::config::Config;
use crate::material::{self, Material, MaterialSurface};
use crate::niri::{self, WindowInfo};
use crate::{dock, menubar};

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
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    WindowsChanged(Vec<WindowInfo>),
    Tick,
    /// A dock icon was clicked; carries the app's id.
    DockClicked(String),
    DockPointerMoved(Point),
    DockPointerLeft,
    /// An animation frame for dock magnification.
    DockFrame(Instant),
    SurfaceOpened(IcedId, Size),
    SurfaceResized(IcedId, Size),
    /// The dock's `wl_surface`, if it could be reached.
    DockSurface(Option<WlSurface>),
}

struct Shell {
    surfaces: HashMap<IcedId, Surface>,
    apps: AppIndex,
    pinned: Vec<String>,
    dock_style: material::Style,
    focused_app: Option<String>,
    clock: String,
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
}

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
                self.dock = dock::items(&self.pinned, &windows, &mut self.apps);
                return self.relayout_dock();
            }
            Message::Tick => self.clock = menubar::format_clock(),
            Message::DockClicked(app_id) => {
                if let Some(item) = self.dock.iter().find(|item| item.app.id == app_id) {
                    dock::activate(item);
                }
            }
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
                // Exponential easing, independent of how regularly frames arrive.
                let dt = self
                    .dock_last_frame
                    .map_or(0.016, |last| (now - last).as_secs_f32())
                    .min(0.05);
                self.dock_last_frame = Some(now);

                let pointer = &mut self.dock_pointer;
                pointer.amount += (self.dock_target - pointer.amount) * (1.0 - (-dt / 0.06).exp());
                if (self.dock_target - pointer.amount).abs() < 0.005 {
                    pointer.amount = self.dock_target;
                    self.dock_last_frame = None;
                }
                if pointer.amount == 0.0 {
                    pointer.base_x = None;
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
            _ => {}
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

        let layout = dock::layout(
            self.dock_width,
            &self.dock,
            &self.dock_pointer,
            self.dock_max_icon,
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
            None => Space::new().into(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Only tick while magnification is easing in or out.
        let dock_frames = if self.dock_pointer.amount == self.dock_target {
            Subscription::none()
        } else {
            time::every(Duration::from_millis(16)).map(Message::DockFrame)
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
