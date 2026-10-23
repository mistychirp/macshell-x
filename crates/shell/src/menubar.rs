//! The top menu bar: focused app name on the left, clock on the right.

use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::backend::native::tokio::time;
use iced_layershell::application;
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, Task};
use iced_layershell::settings::{LayerShellSettings, Settings};
use iced_layershell::to_layer_message;
use iced_widget::core::{Alignment, Color, Font, Length, Theme, font, theme};
use iced_widget::renderer::Renderer;
use iced_widget::{Space, container, row, text};

use crate::niri::{self, FocusedWindow};

type Element<'a, Message> = iced_widget::core::Element<'a, Message, Theme, Renderer>;

const HEIGHT: u32 = 30;
const TEXT_SIZE: f32 = 13.0;

pub fn run() -> Result<(), iced_layershell::Error> {
    application(MenuBar::new, namespace, update, view)
        .style(style)
        .subscription(subscription)
        .settings(Settings {
            layer_settings: LayerShellSettings {
                anchor: Anchor::Top | Anchor::Left | Anchor::Right,
                layer: Layer::Top,
                size: Some((0, HEIGHT)),
                exclusive_zone: HEIGHT as i32,
                keyboard_interactivity: KeyboardInteractivity::None,
                ..Default::default()
            },
            default_text_size: TEXT_SIZE.into(),
            ..Default::default()
        })
        .run()
}

struct MenuBar {
    focused: Option<FocusedWindow>,
    clock: String,
}

impl MenuBar {
    fn new() -> Self {
        Self {
            focused: None,
            clock: format_clock(),
        }
    }
}

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {
    FocusChanged(Option<FocusedWindow>),
    Tick,
}

fn namespace() -> String {
    String::from("macshell-menubar")
}

fn update(bar: &mut MenuBar, message: Message) -> Task<Message> {
    match message {
        Message::FocusChanged(focused) => bar.focused = focused,
        Message::Tick => bar.clock = format_clock(),
        _ => {}
    }
    Task::none()
}

fn view(bar: &MenuBar) -> Element<'_, Message> {
    let app_name = bar
        .focused
        .as_ref()
        .and_then(|w| w.app_id.as_deref())
        .map_or_else(|| String::from("Desktop"), display_name);

    let bold = Font {
        weight: font::Weight::Bold,
        ..Font::default()
    };

    container(
        row![
            text(app_name).font(bold),
            Space::new().width(Length::Fill),
            text(&bar.clock),
        ]
        .align_y(Alignment::Center),
    )
    .padding([0, 16])
    .height(Length::Fill)
    .align_y(Alignment::Center)
    .into()
}

fn subscription(_: &MenuBar) -> Subscription<Message> {
    Subscription::batch([
        niri::focused_window().map(Message::FocusChanged),
        time::every(Duration::from_secs(1)).map(|_| Message::Tick),
    ])
}

fn style(_: &MenuBar, _: &Theme) -> theme::Style {
    theme::Style {
        background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
        text_color: Color::WHITE,
    }
}

fn format_clock() -> String {
    chrono::Local::now().format("%a %b %-d  %H:%M").to_string()
}

/// Turns an app id like `org.gnome.Nautilus` into `Nautilus`.
// TODO: look up `Name=` from the app's .desktop entry instead.
fn display_name(app_id: &str) -> String {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::from(app_id),
    }
}
