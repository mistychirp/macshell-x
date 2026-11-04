//! Alerts: a centered glass dialog that asks before doing something that can't be undone, like
//! macOS's alert sheets. A layer surface rather than a popup, so it doesn't depend on a pointer
//! grab and stays until answered.

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Background, Border, Color, Length};
use iced_widget::{Space, button, column, container, image, row, text};

use crate::app::{Element, Message};
use crate::{fonts, material, menu};

// Measured from Tahoe's alerts.
const WIDTH: f32 = 266.0;
const RADIUS: f32 = 26.0;
const PADDING_X: f32 = 21.0;
const PADDING_TOP: f32 = 21.0;
const PADDING_BOTTOM: f32 = 16.0;
const ICON_SIZE: f32 = 62.0;
const TEXT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 16.0;
const TEXT_WIDTH: f32 = WIDTH - 2.0 * PADDING_X;
const ICON_GAP: f32 = 12.0;
const TITLE_GAP: f32 = 10.0;
const BUTTONS_GAP: f32 = 16.0;
const BUTTON_HEIGHT: f32 = 28.0;
const BUTTON_SPACING: f32 = 8.0;

pub struct Alert {
    pub title: String,
    pub detail: String,
    pub confirm: String,
    /// What confirming does.
    pub on_confirm: Message,
    pub icon: Option<image::Handle>,
}

/// How tall an alert is, which depends on how its words wrap.
fn height(alert: &Alert) -> f32 {
    let lines =
        |words: &str, font| menu::text_height(words, TEXT_SIZE, LINE_HEIGHT, font, TEXT_WIDTH);
    PADDING_TOP
        + ICON_SIZE
        + ICON_GAP
        + lines(&alert.title, fonts::BOLD)
        + TITLE_GAP
        + lines(&alert.detail, fonts::REGULAR)
        + BUTTONS_GAP
        + BUTTON_HEIGHT
        + PADDING_BOTTOM
}

pub fn settings(alert: &Alert) -> NewLayerShellSettings {
    NewLayerShellSettings {
        // No anchors: the compositor centers the surface.
        anchor: Anchor::empty(),
        layer: Layer::Overlay,
        size: Some((WIDTH as u32, height(alert).ceil() as u32)),
        exclusive_zone: Some(-1),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        namespace: Some(String::from("macshell-alert")),
        ..Default::default()
    }
}

pub fn material_shapes(alert: &Alert) -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, WIDTH as i32, height(alert).ceil() as i32),
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

    let tint = material::panel_tint(material);
    container(column![
        icon,
        Space::new().height(ICON_GAP),
        text(&alert.title)
            .size(TEXT_SIZE)
            .line_height(LineHeight::Absolute(LINE_HEIGHT.into()))
            .font(fonts::BOLD),
        Space::new().height(TITLE_GAP),
        text(&alert.detail)
            .size(TEXT_SIZE)
            .line_height(LineHeight::Absolute(LINE_HEIGHT.into()))
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.85)),
        Space::new().height(BUTTONS_GAP),
        row![
            alert_button("Cancel", Message::AlertAnswered(false), false),
            alert_button(&alert.confirm, Message::AlertAnswered(true), true),
        ]
        .spacing(BUTTON_SPACING),
    ])
    .padding(iced_widget::core::Padding {
        top: PADDING_TOP,
        bottom: PADDING_BOTTOM,
        left: PADDING_X,
        right: PADDING_X,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.1, 0.1, 0.12, tint))),
        border: material::panel_border(material, RADIUS),
        text_color: Some(Color::WHITE),
        ..Default::default()
    })
    .into()
}

fn alert_button<'a>(label: &str, on_press: Message, primary: bool) -> Element<'a> {
    button(
        container(text(label.to_owned()).size(TEXT_SIZE))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press(on_press)
    .width(Length::Fill)
    .height(BUTTON_HEIGHT)
    .padding([0, 10])
    .style(move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let background = if primary {
            // Tahoe's default button, with its purple accent.
            Color::from_rgba(0.63, 0.31, 0.66, if hovered { 1.0 } else { 0.9 })
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.2 } else { 0.1 })
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: Color::WHITE,
            border: Border {
                radius: (BUTTON_HEIGHT / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}
