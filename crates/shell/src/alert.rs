//! Alerts: a glass dialog that asks before doing something that can't be undone, like macOS's. An
//! ordinary window without a title bar, which can be dragged by anything that isn't a button. Our
//! KWin plugin centers it and keeps it on top of other windows, since clients can't.

use iced_layershell::actions::IcedXdgWindowSettings;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Background, Border, Color, Length};
use iced_widget::{Space, button, column, container, image, mouse_area, row, text, text_input};

use crate::app::{Element, Message};
use crate::icons::{self, Glyph};
use crate::{fonts, material, menu};

// Measured from Tahoe's alerts.
const WIDTH: f32 = 266.0;
pub const RADIUS: f32 = 26.0;
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
const FIELD_GAP: f32 = 12.0;
const FIELD_HEIGHT: f32 = 28.0;
const SHOW_GAP: f32 = 10.0;
const SHOW_HEIGHT: f32 = 16.0;
const BOX_SIZE: f32 = 14.0;
/// What the KWin plugin recognizes dialogs by.
pub const APP_ID: &str = "macshell-alert";

pub struct Alert {
    pub title: String,
    pub detail: String,
    pub confirm: String,
    /// What confirming does.
    pub on_confirm: Message,
    pub icon: Icon,
    /// Something to type in, like a network's password.
    pub field: Option<Field>,
}

pub enum Icon {
    None,
    Image(image::Handle),
    Glyph(Glyph),
}

/// What an alert's field is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A network's password, for one of NetworkManager's requests.
    Password(u64),
    /// Where the weather widget should look.
    City,
}

pub struct Field {
    pub purpose: Purpose,
    pub value: String,
    /// Whether what's typed shows. A password can choose.
    pub visible: bool,
    pub min_length: usize,
}

impl Field {
    pub fn is_acceptable(&self) -> bool {
        self.value.trim().chars().count() >= self.min_length
    }
}

impl Alert {
    /// Whether it can be confirmed yet.
    pub fn can_confirm(&self) -> bool {
        self.field.as_ref().is_none_or(Field::is_acceptable)
    }

    /// The widget to give the keyboard when it opens.
    pub fn field_id() -> iced_widget::Id {
        iced_widget::Id::new("alert-field")
    }
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
        + alert
            .field
            .as_ref()
            .map_or(0.0, |field| match field.purpose {
                Purpose::Password(_) => FIELD_GAP + FIELD_HEIGHT + SHOW_GAP + SHOW_HEIGHT,
                Purpose::City => FIELD_GAP + FIELD_HEIGHT,
            })
        + BUTTONS_GAP
        + BUTTON_HEIGHT
        + PADDING_BOTTOM
}

pub fn settings(alert: &Alert) -> IcedXdgWindowSettings {
    IcedXdgWindowSettings {
        size: Some((WIDTH as u32, height(alert).ceil() as u32)),
        client_side_decorations: true,
        app_id: Some(APP_ID),
        title: Some("macshell"),
        fixed_size: true,
    }
}

/// The middle of an alert, which it grows out of.
pub fn middle(alert: &Alert) -> (i32, i32) {
    ((WIDTH / 2.0) as i32, (height(alert) / 2.0) as i32)
}

pub fn material_shapes(alert: &Alert, active: bool) -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, WIDTH as i32, height(alert).ceil() as i32),
        radius: RADIUS as i32,
        style: material::Style::Window { active },
    }]
}

pub fn view(alert: &Alert, active: bool, material: Option<material::Kind>) -> Element<'_> {
    let icon: Element<'_> = match &alert.icon {
        Icon::Image(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        Icon::Glyph(glyph) => icons::icon(*glyph, ICON_SIZE, Color::WHITE),
        Icon::None => Space::new().height(ICON_SIZE).into(),
    };
    let field: Element<'_> = match &alert.field {
        Some(field) => field_view(field),
        None => Space::new().into(),
    };

    let tint = material::panel_tint(material);
    let content = container(column![
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
        field,
        Space::new().height(BUTTONS_GAP),
        row![
            alert_button("Cancel", Some(Message::AlertAnswered(false)), false),
            alert_button(
                &alert.confirm,
                alert.can_confirm().then_some(Message::AlertAnswered(true)),
                active,
            ),
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
    });
    // Buttons take their own presses, so this gets the rest.
    mouse_area(content).on_press(Message::AlertDragged).into()
}

fn field_view(field: &Field) -> Element<'_> {
    let placeholder = match field.purpose {
        Purpose::Password(_) => "Password",
        Purpose::City => "City",
    };
    let input = text_input(placeholder, &field.value)
        .id(Alert::field_id())
        .secure(!field.visible)
        .on_input(Message::AlertFieldChanged)
        .size(TEXT_SIZE)
        .line_height(LineHeight::Absolute(LINE_HEIGHT.into()))
        .padding([(FIELD_HEIGHT - LINE_HEIGHT - 2.0) / 2.0, 9.0])
        .style(|_, status| {
            let focused = matches!(status, text_input::Status::Focused { .. });
            text_input::Style {
                background: Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)),
                border: Border {
                    color: if focused {
                        crate::appearance::accent()
                    } else {
                        Color::from_rgba(1.0, 1.0, 1.0, 0.18)
                    },
                    width: 1.0,
                    radius: 8.0.into(),
                },
                icon: Color::WHITE,
                placeholder: Color::from_rgba(1.0, 1.0, 1.0, 0.45),
                value: Color::WHITE,
                selection: crate::appearance::accent(),
            }
        });

    if field.purpose == Purpose::City {
        return column![Space::new().height(FIELD_GAP), input].into();
    }

    let visible = field.visible;
    let tick: Element<'_> = if visible {
        icons::icon(Glyph::Check, BOX_SIZE - 3.0, Color::WHITE)
    } else {
        Space::new().into()
    };
    let check_box = container(tick)
        .center_x(BOX_SIZE)
        .center_y(BOX_SIZE)
        .style(move |_| container::Style {
            background: Some(Background::Color(if visible {
                crate::appearance::accent()
            } else {
                Color::from_rgba(1.0, 1.0, 1.0, 0.1)
            })),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.18),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });
    let show = mouse_area(
        row![
            check_box,
            text("Show password")
                .size(TEXT_SIZE)
                .line_height(LineHeight::Absolute(SHOW_HEIGHT.into())),
        ]
        .spacing(7)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::AlertFieldShown(!visible));

    column![
        Space::new().height(FIELD_GAP),
        input,
        Space::new().height(SHOW_GAP),
        show,
    ]
    .into()
}

fn alert_button<'a>(label: &str, on_press: Option<Message>, primary: bool) -> Element<'a> {
    button(
        container(text(label.to_owned()).size(TEXT_SIZE))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press_maybe(on_press)
    .width(Length::Fill)
    .height(BUTTON_HEIGHT)
    .padding([0, 10])
    .style(move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = status == button::Status::Disabled;
        let background = if primary && !disabled {
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
