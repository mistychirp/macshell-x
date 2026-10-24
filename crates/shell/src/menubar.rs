//! The top menu bar, like macOS's: the distro logo with the system menu, the focused app's
//! name with its menu, and the clock.
//!
//! Titles have fixed widths, so [`title_rect`] knows where each one is to hang its menu under it.

use std::fs;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Border, Color, Font, Length, Rectangle, font};
use iced_widget::{Space, container, image, mouse_area, row, text};

use crate::app::{Element, Message};
use crate::apps::App;
use crate::dock::MenuAction;
use crate::material::{self, Shape};
use crate::{menu, niri};

pub const HEIGHT: u32 = 30;
pub const TEXT_SIZE: f32 = 13.0;
/// Between the screen edges and the outermost titles.
const PADDING_X: f32 = 8.0;
/// Between a title's text and the edges of its highlight.
const TITLE_PADDING_X: f32 = 10.0;
const TITLE_HEIGHT: f32 = 22.0;
const LOGO_SIZE: f32 = 16.0;
/// Between the bottom of the bar and the top of a menu hanging from it.
const MENU_GAP: i32 = 4;

const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};

/// A title in the bar that opens a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Title {
    /// The distro logo.
    System,
    /// The focused app's name.
    App,
}

/// What the system menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAction {
    Sleep,
    Restart,
    ShutDown,
    Lock,
    LogOut,
}

/// What to ask before a system action that ends the session: title and confirm button.
pub struct Confirmation {
    pub title: &'static str,
    pub confirm: &'static str,
}

impl SystemAction {
    pub fn confirmation(self) -> Option<Confirmation> {
        match self {
            SystemAction::Restart => Some(Confirmation {
                title: "Are you sure you want to restart your computer now?",
                confirm: "Restart",
            }),
            SystemAction::ShutDown => Some(Confirmation {
                title: "Are you sure you want to shut down your computer now?",
                confirm: "Shut Down",
            }),
            SystemAction::LogOut => Some(Confirmation {
                title: "Are you sure you want to quit all applications and log out now?",
                confirm: "Log Out",
            }),
            SystemAction::Sleep | SystemAction::Lock => None,
        }
    }

    pub fn run(self) {
        let command: &[&str] = match self {
            SystemAction::Sleep => &["systemctl", "suspend"],
            SystemAction::Restart => &["systemctl", "reboot"],
            SystemAction::ShutDown => &["systemctl", "poweroff"],
            SystemAction::Lock => &["loginctl", "lock-session"],
            SystemAction::LogOut => return niri::quit(),
        };
        niri::spawn(command.iter().map(|arg| arg.to_string()).collect());
    }
}

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: Some((0, HEIGHT)),
        exclusive_zone: Some(HEIGHT as i32),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-menubar")),
        ..Default::default()
    }
}

pub fn material_shapes(width: f32) -> Vec<Shape> {
    vec![Shape {
        rect: (0, 0, width.ceil() as i32, HEIGHT as i32),
        radius: 0,
    }]
}

/// Where a title's highlight goes, in surface-local logical pixels.
pub fn title_rect(title: Title, app_name: &str) -> Rectangle {
    let system = Rectangle {
        x: PADDING_X,
        y: (HEIGHT as f32 - TITLE_HEIGHT) / 2.0,
        width: LOGO_SIZE + 2.0 * TITLE_PADDING_X,
        height: TITLE_HEIGHT,
    };
    match title {
        Title::System => system,
        Title::App => Rectangle {
            x: system.x + system.width,
            width: (menu::text_width(app_name, TEXT_SIZE, BOLD) + 2.0 * TITLE_PADDING_X).ceil(),
            ..system
        },
    }
}

/// Where a title's menu attaches: under its left edge, a little below the bar.
pub fn menu_anchor(title: Title, app_name: &str) -> (i32, i32, i32, i32) {
    let rect = title_rect(title, app_name);
    (
        rect.x as i32,
        0,
        rect.width.ceil() as i32,
        HEIGHT as i32 + MENU_GAP,
    )
}

/// Icon names to try for the distro logo: the config's choice, then os-release's `LOGO`.
/// `monochrome` tries its monochrome variants first.
pub fn logo_names(configured: Option<&str>, monochrome: bool) -> Vec<String> {
    let mut names: Vec<String> = configured.map(str::to_owned).into_iter().collect();

    let os_release = fs::read_to_string("/etc/os-release")
        .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let field = |key: &str| {
        os_release.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix('=')?;
            Some(value.trim_matches(|c| c == '"' || c == '\'').to_owned())
        })
    };

    let logo = field("LOGO").or_else(|| field("ID").map(|id| format!("distributor-logo-{id}")));
    if let Some(logo) = logo {
        if monochrome {
            names.extend([format!("{logo}-symbolic"), format!("{logo}-white")]);
        }
        names.push(logo);
    }
    names.push(String::from("distributor-logo"));
    names
}

pub fn system_menu() -> Vec<menu::Entry> {
    let item = |label, action| menu::Entry::item(label, Message::System(action));
    vec![
        item("Sleep", SystemAction::Sleep),
        item("Restart…", SystemAction::Restart),
        item("Shut Down…", SystemAction::ShutDown),
        menu::Entry::Separator,
        item("Lock Screen", SystemAction::Lock),
        menu::Entry::item(
            &format!("Log Out {}…", user_name()),
            Message::System(SystemAction::LogOut),
        ),
    ]
}

/// The focused app's menu: its desktop actions, then Quit.
pub fn app_menu(app: &App, windows: Vec<u64>) -> Vec<menu::Entry> {
    let mut entries: Vec<menu::Entry> = app
        .actions
        .iter()
        .map(|action| {
            menu::Entry::item(
                &action.name,
                Message::DockMenu(MenuAction::Launch(action.command.clone())),
            )
        })
        .collect();
    if !entries.is_empty() {
        entries.push(menu::Entry::Separator);
    }
    entries.push(menu::Entry::item(
        &format!("Quit {}", app.name),
        Message::DockMenu(MenuAction::Quit(windows)),
    ));
    entries
}

/// The user's full name from the passwd database, or the login name.
fn user_name() -> String {
    let login = std::env::var("USER").unwrap_or_default();
    let full_name = fs::read_to_string("/etc/passwd").ok().and_then(|passwd| {
        passwd.lines().find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.first() == Some(&login.as_str()))
                .then(|| fields.get(4)?.split(',').next())
                .flatten()
                .filter(|gecos| !gecos.is_empty())
                .map(str::to_owned)
        })
    });
    full_name.unwrap_or(login)
}

pub fn view<'a>(
    app_name: &'a str,
    clock: &'a str,
    logo: Option<&'a image::Handle>,
    open: Option<Title>,
    material: Option<material::Kind>,
) -> Element<'a> {
    let logo: Element<'a> = match logo {
        Some(handle) => image(handle.clone())
            .width(LOGO_SIZE)
            .height(LOGO_SIZE)
            .into(),
        None => Space::new().width(LOGO_SIZE).into(),
    };

    // Over glass, a dark tint keeps white text readable on bright wallpapers. (macOS flips the
    // text to black instead; that needs to know how bright the wallpaper is.)
    let tint = if material.is_some() { 0.3 } else { 0.4 };
    container(
        row![
            title(Title::System, logo, app_name, open),
            title(Title::App, text(app_name).font(BOLD).into(), app_name, open),
            Space::new().width(Length::Fill),
            container(text(clock)).padding([0.0, TITLE_PADDING_X]),
        ]
        .align_y(Alignment::Center),
    )
    .padding([0.0, PADDING_X])
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Alignment::Center)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, tint))),
        ..Default::default()
    })
    .into()
}

/// A title that opens its menu on press, highlighted while the menu is open.
fn title<'a>(
    title: Title,
    content: Element<'a>,
    app_name: &str,
    open: Option<Title>,
) -> Element<'a> {
    let rect = title_rect(title, app_name);
    let highlighted = open == Some(title);
    // Menus open on press, like on macOS, while the press can still grab the pointer.
    mouse_area(
        container(content)
            .center_x(rect.width)
            .center_y(rect.height)
            .style(move |_| container::Style {
                background: highlighted
                    .then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.2))),
                border: Border {
                    radius: 7.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .on_press(Message::BarPressed(title))
    .into()
}

pub fn format_clock() -> String {
    chrono::Local::now().format("%a %b %-d  %H:%M").to_string()
}
