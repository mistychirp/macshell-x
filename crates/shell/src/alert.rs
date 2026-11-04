//! Alerts: a centered glass dialog that asks before doing something that can't be undone, like
//! macOS's alert sheets. A layer surface rather than a popup, so it doesn't depend on a pointer
//! grab and stays until answered.

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Border, Color, Length, alignment};
use iced_widget::{Space, button, column, container, image, row, text};

use crate::app::{Element, Message};
use crate::{material, menu};

const WIDTH: u32 = 300;
const HEIGHT: u32 = 196;
const RADIUS: f32 = 20.0;
const PADDING: f32 = 18.0;
const ICON_SIZE: f32 = 56.0;

pub struct Alert {
    pub title: String,
    pub detail: String,
    pub confirm: String,
    /// What confirming does.
    pub on_confirm: Message,
    pub icon: Option<image::Handle>,
}

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        // No anchors: the compositor centers the surface.
        anchor: Anchor::empty(),
        layer: Layer::Overlay,
        size: Some((WIDTH, HEIGHT)),
        exclusive_zone: Some(-1),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        namespace: Some(String::from("macshell-alert")),
        ..Default::default()
    }
}

pub fn material_shapes() -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, WIDTH as i32, HEIGHT as i32),
        radius: RADIUS as i32,
        style: material::Style::Frosted,
    }]
}

pub fn view(alert: &Alert, material: Option<material::Kind>) -> Element<'_> {
    let icon: Element<'_> = match &alert.icon {
        Some(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        None => Space::new().height(ICON_SIZE).into(),
    };

    let buttons = row![
        alert_button("Cancel", Message::AlertAnswered(false), false),
        alert_button(&alert.confirm, Message::AlertAnswered(true), true),
    ]
    .spacing(8);

    let tint = if material.is_some() { menu::TINT } else { 0.92 };
    container(
        column![
            icon,
            text(&alert.title)
                .size(13)
                .align_x(alignment::Horizontal::Center),
            text(&alert.detail)
                .size(11)
                .align_x(alignment::Horizontal::Center)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.65)),
            Space::new().height(Length::Fill),
            buttons,
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .padding(PADDING)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.1, 0.1, 0.12, tint))),
        border: material::panel_border(material, RADIUS),
        ..Default::default()
    })
    .into()
}

fn alert_button<'a>(label: &str, on_press: Message, primary: bool) -> Element<'a> {
    button(
        text(label.to_owned())
            .size(13)
            .align_x(alignment::Horizontal::Center),
    )
    .on_press(on_press)
    .width(Length::Fill)
    .padding([5, 10])
    .style(move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let background = if primary {
            // macOS's default-button blue.
            Color::from_rgba(0.0, 0.45, 1.0, if hovered { 1.0 } else { 0.9 })
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.25 } else { 0.15 })
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: Color::WHITE,
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}
