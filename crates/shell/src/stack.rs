//! The Downloads stack: the newest downloads in a grid popup, like macOS's Grid view.

use iced_widget::core::{Alignment, Background, Border, Color, Length, alignment};
use iced_widget::{button, column, container, image, row, text};

use crate::app::{Element, Message};
use crate::dock::MenuAction;
use crate::places::{Download, Place};
use crate::{material, menu};

/// How many downloads the stack shows.
pub const LIMIT: usize = 12;
const COLUMNS: usize = 4;
const CELL_WIDTH: f32 = 88.0;
const CELL_HEIGHT: f32 = 84.0;
const ICON_SIZE: f32 = 48.0;
const NAME_SIZE: f32 = 11.0;
/// Names longer than this many characters are shortened in the middle.
const MAX_NAME_CHARS: usize = 16;
const PADDING: f32 = 12.0;
const FOOTER_HEIGHT: f32 = 32.0;
const RADIUS: f32 = 14.0;

pub struct Tile {
    pub download: Download,
    pub icon: Option<image::Handle>,
}

pub fn size(count: usize) -> (u32, u32) {
    // Even an empty stack is wide enough to say so.
    let columns = count.clamp(2, COLUMNS) as f32;
    let rows = count.div_ceil(COLUMNS).max(1) as f32;
    let width = 2.0 * PADDING + columns * CELL_WIDTH;
    let height = 2.0 * PADDING + rows * CELL_HEIGHT + FOOTER_HEIGHT;
    (width as u32, height as u32)
}

pub fn material_shapes((width, height): (u32, u32)) -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, width as i32, height as i32),
        radius: RADIUS as i32,
        style: material::Style::Frosted,
    }]
}

pub fn view(tiles: &[Tile], material: Option<material::Kind>) -> Element<'_> {
    let grid: Element<'_> = if tiles.is_empty() {
        container(text("No downloads").size(13))
            .center_x(Length::Fill)
            .center_y(CELL_HEIGHT)
            .style(|_| container::Style {
                text_color: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                ..Default::default()
            })
            .into()
    } else {
        column(
            tiles
                .chunks(COLUMNS)
                .map(|chunk| row(chunk.iter().map(cell)).into()),
        )
        .into()
    };

    let footer = button(text("Open in Files").size(12))
        .on_press(Message::DockMenu(MenuAction::OpenPlace(Place::Downloads)))
        .padding([4, 12])
        .style(|_, status| button::Style {
            background: Some(Background::Color(Color::from_rgba(
                1.0,
                1.0,
                1.0,
                if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    0.25
                } else {
                    0.12
                },
            ))),
            text_color: Color::WHITE,
            border: Border {
                radius: 10.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    // Like menus: a dark tint over glass, nearly opaque without it.
    let tint = if material.is_some() { menu::TINT } else { 0.9 };
    container(
        column![
            grid,
            container(footer)
                .center_x(Length::Fill)
                .height(FOOTER_HEIGHT)
                .align_y(alignment::Vertical::Bottom),
        ]
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

fn cell(tile: &Tile) -> Element<'_> {
    let icon: Element<'_> = match &tile.icon {
        Some(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        None => container(text("?").size(22)).center(ICON_SIZE).into(),
    };

    button(
        column![
            icon,
            text(shorten(&tile.download.name))
                .size(NAME_SIZE)
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center)
        ]
        .spacing(4)
        // Fill the cell, so icons center on it rather than on their names.
        .width(Length::Fill)
        .align_x(Alignment::Center),
    )
    .on_press(Message::DockMenu(MenuAction::OpenPath(
        tile.download.path.clone(),
    )))
    .width(CELL_WIDTH)
    .height(CELL_HEIGHT)
    .padding([6, 4])
    .style(|_, status| button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.15))),
        text_color: Color::WHITE,
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

/// Shortens a file name in the middle, so its start and its extension stay readable.
fn shorten(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= MAX_NAME_CHARS {
        return name.to_owned();
    }
    let keep = (MAX_NAME_CHARS - 1) / 2;
    let start: String = chars[..keep].iter().collect();
    let end: String = chars[chars.len() - keep..].iter().collect();
    format!("{start}…{end}")
}
