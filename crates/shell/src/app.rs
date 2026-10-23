//! The shell process: owns every layer surface and the state they share.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::backend::native::tokio::time;
use iced_layershell::daemon;
use iced_layershell::reexport::{IcedId, Task, WithConnection};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::Space;
use iced_widget::core::{Color, Event, Theme, theme, window};
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
    SurfaceOpened(IcedId),
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
    dock_size: (u32, u32),
    material: Option<Rc<Material>>,
    dock_material: Option<MaterialSurface>,
}

impl Shell {
    fn new(material: Option<Rc<Material>>) -> (Self, Task<Message>) {
        let config = Config::load();
        let mut apps = AppIndex::load();
        let dock = dock::items(&config.dock.pinned, &[], &mut apps);

        let (menubar_id, open_menubar) = Message::layershell_open(menubar::settings());
        let (dock_id, open_dock) = Message::layershell_open(dock::settings(dock.len()));

        let shell = Self {
            surfaces: HashMap::from([(menubar_id, Surface::MenuBar), (dock_id, Surface::Dock)]),
            apps,
            pinned: config.dock.pinned,
            dock_style: config.dock.glass,
            focused_app: None,
            clock: menubar::format_clock(),
            dock_size: dock::size(dock.len()),
            dock,
            dock_id,
            material,
            dock_material: None,
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
                return self.resize_dock();
            }
            Message::Tick => self.clock = menubar::format_clock(),
            Message::DockClicked(app_id) => {
                if let Some(item) = self.dock.iter().find(|item| item.app.id == app_id) {
                    dock::activate(item);
                }
            }
            Message::SurfaceOpened(id) => {
                // Attaching twice is a protocol error, so only ever do it once.
                if id == self.dock_id
                    && self.dock_material.is_none()
                    && let Some(material) = &self.material
                {
                    let connection = material.connection().clone();
                    return iced_runtime::window::run(id, move |window| {
                        material::surface_of(&connection, window)
                    })
                    .map(Message::DockSurface);
                }
            }
            Message::DockSurface(Some(surface)) => {
                if let Some(material) = &self.material {
                    let target = material.attach(surface);
                    material.set_shapes(
                        &target,
                        &dock::material_shapes(self.dock_size, self.dock_style),
                    );
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

    /// Fits the dock surface (and its material) to the current number of items.
    fn resize_dock(&mut self) -> Task<Message> {
        let size = dock::size(self.dock.len());
        if size == self.dock_size {
            return Task::none();
        }
        self.dock_size = size;

        if let (Some(material), Some(target)) = (&self.material, &self.dock_material) {
            material.set_shapes(target, &dock::material_shapes(size, self.dock_style));
        }
        Task::done(Message::SizeChange {
            id: self.dock_id,
            size,
        })
    }

    fn view(&self, id: IcedId) -> Element<'_> {
        match self.surfaces.get(&id) {
            Some(Surface::MenuBar) => menubar::view(self.focused_app.as_deref(), &self.clock),
            Some(Surface::Dock) => dock::view(&self.dock, self.material.as_ref().map(|m| m.kind())),
            None => Space::new().into(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            niri::windows().map(Message::WindowsChanged),
            time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            iced_futures::event::listen_with(|event, _, id| match event {
                Event::Window(window::Event::Opened { .. }) => Some(Message::SurfaceOpened(id)),
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
