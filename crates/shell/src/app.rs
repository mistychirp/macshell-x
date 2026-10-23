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
use crate::blur::{self, Blur, BlurredSurface};
use crate::config::Config;
use crate::niri::{self, WindowInfo};
use crate::{dock, menubar};

pub type Element<'a> = iced_widget::core::Element<'a, Message, Theme, Renderer>;

pub fn run() -> Result<(), iced_layershell::Error> {
    // Our own connection, shared with iced_layershell, so protocol objects we create can refer
    // to the surfaces it creates.
    let connection = Connection::connect_to_env()
        .inspect_err(|err| log::error!("can't connect to Wayland: {err}"))
        .ok();
    let blur = connection.as_ref().and_then(Blur::new).map(Rc::new);

    daemon(
        move || Shell::new(blur.clone()),
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
    focused_app: Option<String>,
    clock: String,
    dock: Vec<dock::Item>,
    dock_id: IcedId,
    dock_size: (u32, u32),
    blur: Option<Rc<Blur>>,
    dock_blur: Option<BlurredSurface>,
}

impl Shell {
    fn new(blur: Option<Rc<Blur>>) -> (Self, Task<Message>) {
        let config = Config::load();
        let mut apps = AppIndex::load();
        let dock = dock::items(&config.dock.pinned, &[], &mut apps);

        let (menubar_id, open_menubar) = Message::layershell_open(menubar::settings());
        let (dock_id, open_dock) = Message::layershell_open(dock::settings(dock.len()));

        let shell = Self {
            surfaces: HashMap::from([(menubar_id, Surface::MenuBar), (dock_id, Surface::Dock)]),
            apps,
            pinned: config.dock.pinned,
            focused_app: None,
            clock: menubar::format_clock(),
            dock_size: dock::size(dock.len()),
            dock,
            dock_id,
            blur,
            dock_blur: None,
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
                    && self.dock_blur.is_none()
                    && let Some(blur) = &self.blur
                {
                    let connection = blur.connection().clone();
                    return iced_runtime::window::run(id, move |window| {
                        blur::surface_of(&connection, window)
                    })
                    .map(Message::DockSurface);
                }
            }
            Message::DockSurface(Some(surface)) => {
                if let Some(blur) = &self.blur {
                    let target = blur.attach(surface);
                    blur.set_region(&target, &dock::blur_region(self.dock_size));
                    self.dock_blur = Some(target);
                }
            }
            Message::DockSurface(None) => log::warn!("can't reach the dock's wl_surface; no blur"),
            _ => {}
        }
        Task::none()
    }

    /// Fits the dock surface (and its blur) to the current number of items.
    fn resize_dock(&mut self) -> Task<Message> {
        let size = dock::size(self.dock.len());
        if size == self.dock_size {
            return Task::none();
        }
        self.dock_size = size;

        if let (Some(blur), Some(target)) = (&self.blur, &self.dock_blur) {
            blur.set_region(target, &dock::blur_region(size));
        }
        Task::done(Message::SizeChange {
            id: self.dock_id,
            size,
        })
    }

    fn view(&self, id: IcedId) -> Element<'_> {
        match self.surfaces.get(&id) {
            Some(Surface::MenuBar) => menubar::view(self.focused_app.as_deref(), &self.clock),
            Some(Surface::Dock) => dock::view(&self.dock),
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
