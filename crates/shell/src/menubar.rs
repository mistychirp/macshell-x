//! The top menu bar: focused app name on the left, clock on the right.

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Color, Font, Length, font};
use iced_widget::{Space, container, row, text};

use crate::app::Element;

const HEIGHT: u32 = 30;
pub const TEXT_SIZE: f32 = 13.0;

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

pub fn view<'a>(focused_app: Option<&'a str>, clock: &'a str) -> Element<'a> {
    let bold = Font {
        weight: font::Weight::Bold,
        ..Font::default()
    };

    container(
        row![
            text(focused_app.unwrap_or("Desktop")).font(bold),
            Space::new().width(Length::Fill),
            text(clock),
        ]
        .align_y(Alignment::Center),
    )
    .padding([0, 16])
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Alignment::Center)
    .style(|_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.25))),
        ..Default::default()
    })
    .into()
}

pub fn format_clock() -> String {
    chrono::Local::now().format("%a %b %-d  %H:%M").to_string()
}
