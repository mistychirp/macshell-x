//! The shell process: owns every layer surface and the state they share.

use std::collections::HashMap;
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::backend::native::tokio::time;
use iced_layershell::daemon;
use iced_layershell::reexport::{IcedId, Task};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::Space;
use iced_widget::core::{Color, Theme, theme};
use iced_widget::renderer::Renderer;

use crate::apps::AppIndex;
use crate::config::Config;
use crate::niri::{self, WindowInfo};
use crate::{dock, menubar};

pub type Element<'a> = iced_widget::core::Element<'a, Message, Theme, Renderer>;

pub fn run() -> Result<(), iced_layershell::Error> {
    daemon(Shell::new, namespace, Shell::update, Shell::view)
        .style(style)
        .subscription(Shell::subscription)
        .settings(Settings {
            // No implicit surface; `Shell::new` opens each one explicitly.
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..Default::default()
            },
            default_text_size: menubar::TEXT_SIZE.into(),
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
}

struct Shell {
    surfaces: HashMap<IcedId, Surface>,
    apps: AppIndex,
    pinned: Vec<String>,
    focused_app: Option<String>,
    clock: String,
    dock: Vec<dock::Item>,
}

impl Shell {
    fn new() -> (Self, Task<Message>) {
        let config = Config::load();
        let (menubar_id, open_menubar) = Message::layershell_open(menubar::settings());
        let (dock_id, open_dock) = Message::layershell_open(dock::settings());

        let mut shell = Self {
            surfaces: HashMap::from([(menubar_id, Surface::MenuBar), (dock_id, Surface::Dock)]),
            apps: AppIndex::load(),
            pinned: config.dock.pinned,
            focused_app: None,
            clock: menubar::format_clock(),
            dock: Vec::new(),
        };
        shell.dock = dock::items(&shell.pinned, &[], &mut shell.apps);

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
            }
            Message::Tick => self.clock = menubar::format_clock(),
            Message::DockClicked(app_id) => {
                if let Some(item) = self.dock.iter().find(|item| item.app.id == app_id) {
                    dock::activate(item);
                }
            }
            _ => {}
        }
        Task::none()
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
