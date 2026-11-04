//! Popup menus in the macOS style: items with optional check marks, and separators. Shared by
//! every surface that opens menus.

use iced_widget::core::text::{LineHeight, Paragraph as _, Shaping, Wrapping};
use iced_widget::core::{Alignment, Background, Border, Color, Font, Length, Size, alignment};
use iced_widget::{Space, button, column, container, mouse_area, row, text};

use crate::app::{Element, Message};
use crate::icons::{self, Glyph};
use crate::material;

const TEXT_SIZE: f32 = 13.0;
const ROW_HEIGHT: f32 = 24.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
/// Around the list of items, between them and the menu's edge.
const PADDING: f32 = 5.0;
/// Inside an item, between its highlight and its text.
const ITEM_PADDING_X: f32 = 8.0;
const CHECK_WIDTH: f32 = 18.0;
const SHORTCUT_GAP: f32 = 24.0;
const CHEVRON_SIZE: f32 = 10.0;
/// How far a submenu overlaps its menu.
pub const SUBMENU_OVERLAP: i32 = 4;
/// Between a label and the chevron of a submenu, and the chevron.
const CHEVRON_ROOM: f32 = 16.0 + CHEVRON_SIZE;
const MIN_WIDTH: f32 = 160.0;
/// Labels longer than this many characters are cut short with an ellipsis.
const MAX_LABEL_CHARS: usize = 48;
pub const RADIUS: f32 = 13.0;

/// macOS's selection blue.
pub const ACCENT: Color = Color::from_rgba(0.0, 0.45, 1.0, 0.95);
const DISABLED_TEXT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.4);
const SHORTCUT_TEXT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.45);

#[derive(Debug, Clone)]
pub enum Entry {
    Item {
        label: String,
        checked: bool,
        enabled: bool,
        /// The keys that do the same, shown at its end.
        shortcut: Option<String>,
        /// What opens to its side, if it leads to a submenu.
        submenu: Option<Vec<Entry>>,
        on_select: Message,
    },
    Separator,
}

impl Entry {
    pub fn item(label: &str, on_select: Message) -> Self {
        Self::Item {
            label: truncate(label),
            checked: false,
            enabled: true,
            shortcut: None,
            submenu: None,
            on_select,
        }
    }

    pub fn checked(mut self, value: bool) -> Self {
        if let Self::Item { checked, .. } = &mut self {
            *checked = value;
        }
        self
    }

    pub fn enabled(mut self, value: bool) -> Self {
        if let Self::Item { enabled, .. } = &mut self {
            *enabled = value;
        }
        self
    }

    pub fn shortcut(mut self, keys: &str) -> Self {
        if let Self::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keys.to_owned());
        }
        self
    }

    /// Makes it lead to a submenu of these entries.
    pub fn submenu(mut self, entries: Vec<Entry>) -> Self {
        if let Self::Item { submenu, .. } = &mut self {
            *submenu = Some(entries);
        }
        self
    }

    /// What's in the submenu it leads to, unless it's disabled.
    pub fn submenu_entries(&self) -> Option<&[Entry]> {
        match self {
            Self::Item {
                submenu: Some(entries),
                enabled: true,
                ..
            } => Some(entries),
            _ => None,
        }
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
            Entry::Item {
                label,
                submenu,
                shortcut,
                ..
            } => {
                let end = if submenu.is_some() {
                    CHEVRON_ROOM
                } else if let Some(shortcut) = shortcut {
                    SHORTCUT_GAP + text_width(shortcut, TEXT_SIZE, Font::default())
                } else {
                    0.0
                };
                Some(text_width(label, TEXT_SIZE, Font::default()) + CHECK_WIDTH + end)
            }
            Entry::Separator => None,
        })
        .fold(0.0, f32::max);
    let width = (label_width + 2.0 * ITEM_PADDING_X + 2.0 * PADDING).max(MIN_WIDTH);

    let height = 2.0 * PADDING + entries.iter().map(entry_height).sum::<f32>();

    (width.ceil() as u32, height.ceil() as u32)
}

pub fn material_shapes((width, height): (u32, u32)) -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, width as i32, height as i32),
        radius: RADIUS as i32,
        style: material::Style::Frosted,
    }]
}

/// Where a submenu of the row at `index` attaches to its menu: beside the row, overlapping the menu
/// a little, so that the submenu's first row lines up with it.
pub fn submenu_anchor(entries: &[Entry], index: usize, menu_width: u32) -> (i32, i32, i32, i32) {
    let y = PADDING
        + entries[..index.min(entries.len())]
            .iter()
            .map(entry_height)
            .sum::<f32>();
    (
        SUBMENU_OVERLAP,
        (y - PADDING) as i32,
        menu_width as i32 - 2 * SUBMENU_OVERLAP,
        1,
    )
}

fn entry_height(entry: &Entry) -> f32 {
    match entry {
        Entry::Item { .. } => ROW_HEIGHT,
        Entry::Separator => SEPARATOR_HEIGHT,
    }
}

/// A menu `level` submenus deep, in which the row `open_row` has its submenu open.
pub fn view(
    entries: &[Entry],
    material: Option<material::Kind>,
    level: usize,
    open_row: Option<usize>,
) -> Element<'_> {
    let rows = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| match entry {
            Entry::Item {
                label,
                checked,
                enabled,
                shortcut,
                submenu,
                on_select,
            } => {
                let open = open_row == Some(index);
                let on_press = if submenu.is_some() {
                    Message::MenuRowPressed(level, index)
                } else {
                    on_select.clone()
                };
                let row = button(
                    row![
                        text(if *checked { "✓" } else { "" })
                            .size(TEXT_SIZE)
                            .width(CHECK_WIDTH),
                        // The width is measured to fit, but rounding mustn't wrap it onto two lines.
                        text(label).size(TEXT_SIZE).wrapping(Wrapping::None),
                        Space::new().width(Length::Fill),
                    ]
                    .push(
                        submenu
                            .is_some()
                            .then(|| icons::icon(Glyph::Chevron, CHEVRON_SIZE, Color::WHITE)),
                    )
                    .push(shortcut.as_deref().map(|keys| {
                        text(keys)
                            .size(TEXT_SIZE)
                            .wrapping(Wrapping::None)
                            .color(SHORTCUT_TEXT)
                    }))
                    .align_y(Alignment::Center),
                )
                .on_press_maybe(enabled.then_some(on_press))
                .width(Length::Fill)
                .height(ROW_HEIGHT)
                .padding([0.0, ITEM_PADDING_X])
                .style(move |_, status| button::Style {
                    background: (open
                        || matches!(status, button::Status::Hovered | button::Status::Pressed))
                    .then_some(Background::Color(ACCENT)),
                    text_color: if matches!(status, button::Status::Disabled) {
                        DISABLED_TEXT
                    } else {
                        Color::WHITE
                    },
                    border: Border {
                        // Concentric with the menu's corners.
                        radius: (RADIUS - PADDING).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
                mouse_area(row)
                    .on_enter(Message::MenuRowEntered(level, index))
                    .into()
            }
            Entry::Separator => container(
                container(Space::new())
                    .width(Length::Fill)
                    .height(1)
                    .style(|_| container::Style {
                        background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.15))),
                        ..Default::default()
                    }),
            )
            .height(SEPARATOR_HEIGHT)
            .padding([0.0, ITEM_PADDING_X])
            .align_y(alignment::Vertical::Center)
            .into(),
        });

    let tint = material::panel_tint(material);
    container(column(rows))
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

/// `name`, cut short with an ellipsis if it's too wide to fit on one line.
pub fn fit(name: &str, size: f32, font: Font, width: f32) -> String {
    if text_width(name, size, font) <= width {
        return name.to_owned();
    }
    let chars: Vec<char> = name.chars().collect();
    (1..chars.len())
        .rev()
        .map(|keep| {
            let mut short: String = chars[..keep].iter().collect();
            short.push('…');
            short
        })
        .find(|short| text_width(short, size, font) <= width)
        .unwrap_or_else(|| String::from("…"))
}

/// Height of `text` wrapped to `width`, in logical pixels.
pub fn text_height(text: &str, size: f32, line_height: f32, font: Font, width: f32) -> f32 {
    iced_widget::graphics::text::Paragraph::with_text(iced_widget::core::Text {
        content: text,
        bounds: Size::new(width, f32::INFINITY),
        size: size.into(),
        line_height: LineHeight::Absolute(line_height.into()),
        font,
        align_x: Default::default(),
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::default(),
    })
    .min_bounds()
    .height
}

/// Width of `text` on one line, in logical pixels.
pub fn text_width(text: &str, size: f32, font: Font) -> f32 {
    iced_widget::graphics::text::Paragraph::with_text(iced_widget::core::Text {
        content: text,
        bounds: Size::INFINITE,
        size: size.into(),
        line_height: LineHeight::default(),
        font,
        align_x: Default::default(),
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
    })
    .min_width()
}
