//! Popup menus in the macOS style: items with optional check marks, and separators. Shared by
//! every surface that opens menus.

use iced_widget::core::text::{LineHeight, Paragraph as _, Shaping, Wrapping};
use iced_widget::core::{Alignment, Background, Border, Color, Font, Length, Size, alignment};
use iced_widget::{Space, button, column, container, row, text};

use crate::app::{Element, Message};
use crate::material;

const TEXT_SIZE: f32 = 13.0;
const ROW_HEIGHT: f32 = 24.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
/// Around the list of items, between them and the menu's edge.
const PADDING: f32 = 5.0;
/// Inside an item, between its highlight and its text.
const ITEM_PADDING_X: f32 = 8.0;
const CHECK_WIDTH: f32 = 18.0;
const MIN_WIDTH: f32 = 160.0;
/// Labels longer than this many characters are cut short with an ellipsis.
const MAX_LABEL_CHARS: usize = 48;
pub const RADIUS: f32 = 10.0;
/// Opacity of the dark tint over glass, shared by the shell's other popups.
pub const TINT: f32 = 0.55;

/// macOS's selection blue.
const ACCENT: Color = Color::from_rgba(0.0, 0.45, 1.0, 0.95);

#[derive(Debug, Clone)]
pub enum Entry {
    Item {
        label: String,
        checked: bool,
        on_select: Message,
    },
    Separator,
}

impl Entry {
    pub fn item(label: &str, on_select: Message) -> Self {
        Self::Item {
            label: truncate(label),
            checked: false,
            on_select,
        }
    }

    pub fn checked(mut self, value: bool) -> Self {
        if let Self::Item { checked, .. } = &mut self {
            *checked = value;
        }
        self
    }
}

fn truncate(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL_CHARS {
        return label.to_owned();
    }
    let mut short: String = label.chars().take(MAX_LABEL_CHARS - 1).collect();
    short.push('…');
    short
}

/// Size of a menu with these entries, in logical pixels.
pub fn size(entries: &[Entry]) -> (u32, u32) {
    let label_width = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item { label, .. } => Some(text_width(label, TEXT_SIZE) + CHECK_WIDTH),
            Entry::Separator => None,
        })
        .fold(0.0, f32::max);
    let width = (label_width + 2.0 * ITEM_PADDING_X + 2.0 * PADDING).max(MIN_WIDTH);

    let height = 2.0 * PADDING
        + entries
            .iter()
            .map(|entry| match entry {
                Entry::Item { .. } => ROW_HEIGHT,
                Entry::Separator => SEPARATOR_HEIGHT,
            })
            .sum::<f32>();

    (width.ceil() as u32, height.ceil() as u32)
}

pub fn material_shapes((width, height): (u32, u32)) -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, width as i32, height as i32),
        radius: RADIUS as i32,
    }]
}

pub fn view(entries: &[Entry], material: Option<material::Kind>) -> Element<'_> {
    let rows = entries.iter().map(|entry| match entry {
        Entry::Item {
            label,
            checked,
            on_select,
        } => button(
            row![
                text(if *checked { "✓" } else { "" })
                    .size(TEXT_SIZE)
                    .width(CHECK_WIDTH),
                // The width is measured to fit, but rounding mustn't wrap it onto two lines.
                text(label).size(TEXT_SIZE).wrapping(Wrapping::None),
            ]
            .align_y(Alignment::Center),
        )
        .on_press(on_select.clone())
        .width(Length::Fill)
        .height(ROW_HEIGHT)
        .padding([0.0, ITEM_PADDING_X])
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(ACCENT)),
            text_color: Color::WHITE,
            border: Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into(),
        Entry::Separator => container(container(Space::new()).width(Length::Fill).height(1).style(
            |_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.15))),
                ..Default::default()
            },
        ))
        .height(SEPARATOR_HEIGHT)
        .padding([0.0, ITEM_PADDING_X])
        .align_y(alignment::Vertical::Center)
        .into(),
    });

    // Clear glass needs a fair dark tint to keep text legible over bright backgrounds; with
    // nothing behind the menu, it needs to be nearly opaque.
    let tint = if material.is_some() { TINT } else { 0.9 };
    container(column(rows))
        .padding(PADDING)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(Color::from_rgba(0.1, 0.1, 0.12, tint))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.12),
                width: 1.0,
                radius: RADIUS.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Width of `text` on one line, in logical pixels.
pub fn text_width(text: &str, size: f32) -> f32 {
    iced_widget::graphics::text::Paragraph::with_text(iced_widget::core::Text {
        content: text,
        bounds: Size::INFINITE,
        size: size.into(),
        line_height: LineHeight::default(),
        font: Font::default(),
        align_x: Default::default(),
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
    })
    .min_width()
}
