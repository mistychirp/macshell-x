//! The top menu bar, like macOS's: the distro logo with the system menu and the focused app's
//! name with its menu on the left; status items and the clock on the right.
//!
//! Titles have fixed widths, so [`Bar::titles`] knows where each one is to hang its menu under
//! it.

use std::fs;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Border, Color, Font, Length, Rectangle, font};
use iced_widget::{Space, container, image, mouse_area, row, text};

use crate::app::{Element, Message};
use crate::apps::App;
use crate::dock::MenuAction;
use crate::icons::{self, Glyph};
use crate::kwin::KeyboardLayouts;
use crate::material::{self, Shape};
use crate::{apps, menu};

pub const HEIGHT: u32 = 30;
pub const TEXT_SIZE: f32 = 13.0;
/// Between the screen edges and the outermost titles.
const PADDING_X: f32 = 8.0;
/// Between a title's text and the edges of its highlight.
const TITLE_PADDING_X: f32 = 10.0;
const TITLE_HEIGHT: f32 = 22.0;
const LOGO_SIZE: f32 = 16.0;
/// Width of a status item on the right, like the keyboard layout.
const STATUS_WIDTH: f32 = 34.0;
const INPUT_TEXT_SIZE: f32 = 10.0;
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
    /// The keyboard layout.
    Input,
    /// The Control Center's switches.
    ControlCenter,
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
        // Logging out ends the login session, which takes KWin and every app with it.
        let session = std::env::var("XDG_SESSION_ID").unwrap_or_default();
        let command: &[&str] = match self {
            SystemAction::Sleep => &["systemctl", "suspend"],
            SystemAction::Restart => &["systemctl", "reboot"],
            SystemAction::ShutDown => &["systemctl", "poweroff"],
            SystemAction::Lock => &["loginctl", "lock-session"],
            SystemAction::LogOut if session.is_empty() => {
                log::error!("can't log out: XDG_SESSION_ID isn't set");
                return;
            }
            SystemAction::LogOut => &["loginctl", "terminate-session", &session],
        };
        apps::spawn(command.iter().map(|arg| arg.to_string()).collect());
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
        // No background, like macOS's menu bar; the text adapts to the wallpaper instead.
        style: material::Style::Transparent,
    }]
}

/// What the bar shows, from the shell's state.
pub struct Bar<'a> {
    /// Width of the bar surface.
    pub width: f32,
    pub app_name: &'a str,
    pub clock: &'a str,
    /// The logo, in the backdrop's ink.
    pub logo: Option<&'a image::Handle>,
    /// The title whose menu is open.
    pub open: Option<Title>,
    /// `None` when the compositor can't tell how bright the background is.
    pub backdrop: Option<Backdrop>,
    /// Short name of the keyboard layout, when there's more than one.
    pub input: Option<&'a str>,
}

impl Bar<'_> {
    /// The titles, left to right, with where their highlights go in surface-local logical
    /// pixels.
    pub fn titles(&self) -> Vec<(Title, Rectangle)> {
        let y = (HEIGHT as f32 - TITLE_HEIGHT) / 2.0;
        let rect = |x, width| Rectangle {
            x,
            y,
            width,
            height: TITLE_HEIGHT,
        };

        let system = rect(PADDING_X, LOGO_SIZE + 2.0 * TITLE_PADDING_X);
        let app_width =
            (menu::text_width(self.app_name, TEXT_SIZE, BOLD) + 2.0 * TITLE_PADDING_X).ceil();
        let mut titles = vec![
            (Title::System, system),
            (Title::App, rect(system.x + system.width, app_width)),
        ];

        // Status items line up leftward from the clock.
        let mut x = self.width - PADDING_X - self.clock_width();
        x -= STATUS_WIDTH;
        titles.push((Title::ControlCenter, rect(x, STATUS_WIDTH)));
        if self.input.is_some() {
            x -= STATUS_WIDTH;
            titles.push((Title::Input, rect(x, STATUS_WIDTH)));
        }
        titles
    }

    fn clock_width(&self) -> f32 {
        (menu::text_width(self.clock, TEXT_SIZE, Font::DEFAULT) + 2.0 * TITLE_PADDING_X).ceil()
    }

    /// Where a title's menu attaches: under its left edge, a little below the bar.
    pub fn menu_anchor(&self, title: Title) -> Option<(i32, i32, i32, i32)> {
        let (_, rect) = self.titles().into_iter().find(|(t, _)| *t == title)?;
        Some((
            rect.x as i32,
            0,
            rect.width.ceil() as i32,
            HEIGHT as i32 + MENU_GAP,
        ))
    }
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

pub fn view<'a>(bar: &Bar<'a>) -> Element<'a> {
    // Like macOS, the bar has no background: knowing the backdrop, its text flips between white
    // and black. Without that, a dark tint keeps white text readable on bright wallpapers.
    let ink = bar.backdrop.unwrap_or(Backdrop::Dark).ink();
    let tint = match bar.backdrop {
        Some(_) => Color::TRANSPARENT,
        None => Color::from_rgba(0.0, 0.0, 0.0, 0.3),
    };
    let highlight = Color { a: 0.2, ..ink };

    let mut left = row![].align_y(Alignment::Center);
    let mut right: Vec<Element<'a>> = Vec::new();
    for (title, rect) in bar.titles() {
        let content: Element<'a> = match title {
            Title::System => match bar.logo {
                Some(handle) => image(handle.clone())
                    .width(LOGO_SIZE)
                    .height(LOGO_SIZE)
                    .into(),
                None => Space::new().width(LOGO_SIZE).into(),
            },
            Title::App => text(bar.app_name).font(BOLD).into(),
            Title::Input => input_badge(bar.input.unwrap_or_default(), ink),
            Title::ControlCenter => icons::icon(Glyph::ControlCenter, 17.0, ink),
        };
        let item = title_item(title, content, rect, bar.open, highlight);
        match title {
            Title::System | Title::App => left = left.push(item),
            // Laid out right to left, so each one goes in front of the last.
            _ => right.insert(0, item),
        }
    }
    let clock = container(text(bar.clock)).center_x(bar.clock_width());

    let right = row(right).align_y(Alignment::Center);
    container(row![left, Space::new().width(Length::Fill), right, clock].align_y(Alignment::Center))
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
fn title_item<'a>(
    title: Title,
    content: Element<'a>,
    rect: Rectangle,
    open: Option<Title>,
    highlight: Color,
) -> Element<'a> {
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

/// The keyboard layout's short name in a filled box, like macOS's input source icons.
fn input_badge<'a>(label: &str, ink: Color) -> Element<'a> {
    let cutout = if ink.r > 0.5 {
        Color::from_rgb(0.1, 0.1, 0.1)
    } else {
        Color::WHITE
    };
    container(
        text(label.to_owned())
            .size(INPUT_TEXT_SIZE)
            .font(BOLD)
            .color(cutout),
    )
    .padding([0, 3])
    .style(move |_| container::Style {
        background: Some(Background::Color(ink)),
        border: Border {
            radius: 3.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

/// The keyboard layout menu: every layout, with a check on the active one.
pub fn input_menu(layouts: &KeyboardLayouts) -> Vec<menu::Entry> {
    layouts
        .names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            menu::Entry::item(name, Message::SwitchLayout(index)).checked(index == layouts.current)
        })
        .collect()
}

/// A two-letter name for an XKB layout name like "English (US)" or "Russian".
pub fn input_label(name: &str) -> String {
    let language = name.split([' ', '(']).next().unwrap_or(name);
    let code = match language {
        "Arabic" => "AR",
        "Belarusian" => "BY",
        "Bulgarian" => "BG",
        "Chinese" => "ZH",
        "Croatian" => "HR",
        "Czech" => "CS",
        "Danish" => "DA",
        "Dutch" => "NL",
        "English" => "EN",
        "Estonian" => "ET",
        "Finnish" => "FI",
        "French" => "FR",
        "Georgian" => "KA",
        "German" => "DE",
        "Greek" => "EL",
        "Hebrew" => "HE",
        "Hungarian" => "HU",
        "Italian" => "IT",
        "Japanese" => "JA",
        "Kazakh" => "KK",
        "Korean" => "KO",
        "Latvian" => "LV",
        "Lithuanian" => "LT",
        "Norwegian" => "NO",
        "Polish" => "PL",
        "Portuguese" => "PT",
        "Romanian" => "RO",
        "Russian" => "RU",
        "Serbian" => "SR",
        "Slovak" => "SK",
        "Slovenian" => "SL",
        "Spanish" => "ES",
        "Swedish" => "SV",
        "Turkish" => "TR",
        "Ukrainian" => "UK",
        _ => return language.chars().take(2).collect::<String>().to_uppercase(),
    };
    code.to_owned()
}

pub fn format_clock() -> String {
    chrono::Local::now().format("%a %b %-d  %H:%M").to_string()
}
