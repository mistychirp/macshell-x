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

/// How bright what's behind the bar is, which decides its text color, like on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    Dark,
    Light,
}

/// Relative luminance above which a dark backdrop turns light, and below which a light one
/// turns dark again. Black and white text contrast equally at about 0.18; the gap around it
/// keeps the bar from flickering.
const LIGHT_ABOVE: f32 = 0.24;
const DARK_BELOW: f32 = 0.16;

impl Backdrop {
    pub fn from_luminance(previous: Option<Self>, luminance: f32) -> Self {
        match previous {
            Some(Backdrop::Light) if luminance >= DARK_BELOW => Backdrop::Light,
            Some(Backdrop::Dark) if luminance <= LIGHT_ABOVE => Backdrop::Dark,
            _ if luminance > (LIGHT_ABOVE + DARK_BELOW) / 2.0 => Backdrop::Light,
            _ => Backdrop::Dark,
        }
    }

    /// The color of text and the logo.
    pub fn ink(self) -> Color {
        match self {
            Backdrop::Dark => Color::WHITE,
            Backdrop::Light => Color::from_rgba(0.0, 0.0, 0.0, 0.85),
        }
    }
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
        style: material::Style::Clear,
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

/// `backdrop` is `None` when the compositor can't tell how bright the background is; `logo`
/// should match its ink.
pub fn view<'a>(
    app_name: &'a str,
    clock: &'a str,
    logo: Option<&'a image::Handle>,
    open: Option<Title>,
    material: Option<material::Kind>,
    backdrop: Option<Backdrop>,
) -> Element<'a> {
    let logo: Element<'a> = match logo {
        Some(handle) => image(handle.clone())
            .width(LOGO_SIZE)
            .height(LOGO_SIZE)
            .into(),
        None => Space::new().width(LOGO_SIZE).into(),
    };

    // Knowing the backdrop, the text flips between white and black and a faint tint is enough.
    // Otherwise a darker tint keeps white text readable on bright wallpapers.
    let ink = backdrop.unwrap_or(Backdrop::Dark).ink();
    let tint = match (backdrop, material) {
        (Some(Backdrop::Light), _) => Color::from_rgba(1.0, 1.0, 1.0, 0.12),
        (Some(Backdrop::Dark), _) => Color::from_rgba(0.0, 0.0, 0.0, 0.12),
        (None, Some(_)) => Color::from_rgba(0.0, 0.0, 0.0, 0.3),
        (None, None) => Color::from_rgba(0.0, 0.0, 0.0, 0.4),
    };
    let highlight = Color { a: 0.2, ..ink };
    container(
        row![
            title(Title::System, logo, app_name, open, highlight),
            title(
                Title::App,
                text(app_name).font(BOLD).into(),
                app_name,
                open,
                highlight
            ),
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
        background: Some(Background::Color(tint)),
        text_color: Some(ink),
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
    highlight: Color,
) -> Element<'a> {
    let rect = title_rect(title, app_name);
    let highlighted = open == Some(title);
    // Menus open on press, like on macOS, while the press can still grab the pointer.
    mouse_area(
        container(content)
            .center_x(rect.width)
            .center_y(rect.height)
            .style(move |_| container::Style {
                background: highlighted.then_some(Background::Color(highlight)),
                border: Border {
                    radius: 7.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .on_press(Message::BarPressed(title))
    .on_enter(Message::BarEntered(title))
    .into()
}

pub fn format_clock() -> String {
    chrono::Local::now().format("%a %b %-d  %H:%M").to_string()
}
