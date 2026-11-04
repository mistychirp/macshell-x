//! Notification Center, which hangs from the clock: past notifications with no panel behind
//! them, grouped by app. A group shows as one card with the edge of the next peeking out from
//! under it, until it's opened.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use iced_widget::core::mouse::ScrollDelta;
use iced_widget::core::text::{Shaping, Wrapping};
use iced_widget::core::{Alignment, Color, Length};
use iced_widget::graphics::text::cosmic_text::{self, Attrs, Buffer, Metrics, SwashCache, Weight};
use iced_widget::{Space, container, image, mouse_area, pin, row, stack, text};

use crate::app::{Element, Message};
use crate::notifications::{
    CARD_WIDTH, CHIP_INK, CLOSE_SIZE, Notification, OVERHANG, RADIUS, card, chip_shape,
    close_button, close_shape, panel_style,
};
use crate::{dock, fonts, material, menu, menubar};

/// Wider than the cards, for the blur behind the message when there are none to fade out in.
pub const WIDTH: f32 = 540.0;
const RIGHT_PADDING: f32 = 8.0;
const CARD_X: f32 = WIDTH - RIGHT_PADDING - CARD_WIDTH;
const GAP: f32 = 8.0;
const HEADER_HEIGHT: f32 = 26.0;
const SHOW_LESS_WIDTH: f32 = 73.0;
/// How tall "Show less" is; its ✕ is a little shorter, centered beside it.
const SHOW_LESS_HEIGHT: f32 = 21.0;
const CONTROL_GAP: f32 = 8.0;
/// The edge of the group's next card, under its first.
const PEEK_INSET: f32 = 14.0;
const PEEK_HEIGHT: f32 = 9.0;
const EMPTY_TOP: f32 = 40.0;
const TITLE_SIZE: f32 = 15.0;
/// The height of a line of text, in text sizes: what the text widget uses unless told otherwise.
const LINE_HEIGHT: f32 = 1.3;
const SHADOW_REACH: f32 = 8.0;
/// Pixels per logical pixel of a shadow. It's soft, so this needn't follow the screen.
const SHADOW_SCALE: f32 = 2.0;
const SHADOW_BLUR_RADIUS: usize = 5;
const SHADOW_STRENGTH: f32 = 0.9;
/// How far down the blur goes, like Tahoe's, which only blurs behind this message.
const BACKDROP_HEIGHT: f32 = 231.0;
/// How far to scroll to move by a row: about a notch of a mouse wheel.
const PIXELS_PER_ROW: f32 = 25.0;

/// The height of Notification Center's surface on a screen this tall: down to the dock. Any taller
/// and the compositor slides it up over the bar to keep it out of the dock's reserved area.
pub fn height(screen_height: f32) -> f32 {
    (screen_height - menubar::HEIGHT as f32 - dock::RESERVED_HEIGHT - 16.0).max(100.0)
}

/// What can be scrolled past.
#[derive(Debug, Clone, Copy)]
enum Row<'a> {
    Header {
        app_key: &'a str,
        app: &'a str,
    },
    Card {
        notification: &'a Notification,
        /// It stands for a closed group of more than one.
        closed_group: bool,
    },
}

impl Row<'_> {
    fn height(&self) -> f32 {
        match self {
            Row::Header { .. } => HEADER_HEIGHT,
            Row::Card {
                notification,
                closed_group,
            } => notification.height() + if *closed_group { PEEK_HEIGHT } else { 0.0 },
        }
    }
}

/// The rows to show, most recent app first, with each group's most recent notification first.
fn rows<'a>(notifications: &'a [Notification], open: &HashSet<String>) -> Vec<Row<'a>> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for first in notifications {
        if !seen.insert(first.app_key.as_str()) {
            continue;
        }
        let group: Vec<&Notification> = notifications
            .iter()
            .filter(|notification| notification.app_key == first.app_key)
            .collect();
        if group.len() > 1 && open.contains(&first.app_key) {
            rows.push(Row::Header {
                app_key: &first.app_key,
                app: &first.app,
            });
            rows.extend(group.into_iter().map(|notification| Row::Card {
                notification,
                closed_group: false,
            }));
        } else {
            rows.push(Row::Card {
                notification: first,
                closed_group: group.len() > 1,
            });
        }
    }
    rows
}

/// The rows that fit, whole, from row `first` down, with where each goes. Scrolling moves by whole
/// rows: glass can't be clipped at the surface's edge, so a row is either shown or not.
fn placed<'a>(
    notifications: &'a [Notification],
    open: &HashSet<String>,
    first: usize,
    height: f32,
) -> Vec<(Row<'a>, f32)> {
    let mut y = OVERHANG;
    let mut placed = Vec::new();
    for row in rows(notifications, open).into_iter().skip(first) {
        if y + row.height() > height {
            break;
        }
        placed.push((row, y));
        y += row.height() + GAP;
    }
    placed
}

/// How many rows there are to scroll through.
pub fn row_count(notifications: &[Notification], open: &HashSet<String>) -> usize {
    rows(notifications, open).len()
}

pub fn material_shapes(
    notifications: &[Notification],
    open: &HashSet<String>,
    first: usize,
    hovered: Option<u32>,
    height: f32,
) -> Vec<material::Shape> {
    let shape = |x: f32, y: f32, width: f32, height: f32, radius: f32| material::Shape {
        rect: (x as i32, y as i32, width as i32, height as i32),
        radius: radius as i32,
        style: material::Style::Card,
    };
    if notifications.is_empty() {
        return vec![material::Shape {
            rect: (0, 0, WIDTH as i32, BACKDROP_HEIGHT as i32),
            radius: 0,
            style: material::Style::Backdrop([190, 30, 100, 110]),
        }];
    }
    let mut shapes = Vec::new();
    for (row, y) in placed(notifications, open, first, height) {
        match row {
            Row::Header { .. } => {
                let close_x = CARD_X + CARD_WIDTH - CLOSE_SIZE;
                shapes.push(chip_shape(
                    close_x - CONTROL_GAP - SHOW_LESS_WIDTH,
                    y,
                    SHOW_LESS_WIDTH,
                    SHOW_LESS_HEIGHT,
                ));
                shapes.push(close_shape(
                    close_x,
                    y + (SHOW_LESS_HEIGHT - CLOSE_SIZE) / 2.0,
                ));
            }
            Row::Card {
                notification,
                closed_group,
            } => {
                let card_height = notification.height();
                shapes.push(shape(CARD_X, y, CARD_WIDTH, card_height, RADIUS));
                // On top of the card's, so after it.
                if hovered == Some(notification.id) {
                    shapes.push(close_shape(CARD_X - OVERHANG, y - OVERHANG));
                }
                if closed_group {
                    shapes.push(shape(
                        CARD_X + PEEK_INSET,
                        y + card_height + 1.0,
                        CARD_WIDTH - 2.0 * PEEK_INSET,
                        PEEK_HEIGHT - 1.0,
                        (PEEK_HEIGHT - 1.0) / 2.0,
                    ));
                }
            }
        }
    }
    shapes
}

pub fn view<'a>(
    notifications: &'a [Notification],
    open: &HashSet<String>,
    first: usize,
    hovered: Option<u32>,
    height: f32,
    material: Option<material::Kind>,
) -> Element<'a> {
    let mut layers: Vec<Element<'a>> = Vec::new();
    if notifications.is_empty() {
        let message = "No recent notifications";
        let x = CARD_X + (CARD_WIDTH - menu::text_width(message, TITLE_SIZE, fonts::BOLD)) / 2.0;
        layers.push(
            pin(title(message))
                .x(x - SHADOW_REACH)
                .y(EMPTY_TOP - SHADOW_REACH)
                .into(),
        );
    }

    for (row, y) in placed(notifications, open, first, height) {
        match row {
            Row::Header { app_key, app } => {
                layers.push(pin(header(app_key, material)).x(CARD_X).y(y).into());
                layers.push(
                    pin(title(app))
                        .x(CARD_X - SHADOW_REACH)
                        .y(y - SHADOW_REACH)
                        .into(),
                );
            }
            Row::Card {
                notification,
                closed_group,
            } => {
                let id = notification.id;
                let (on_press, on_close) = if closed_group {
                    (
                        Message::NotificationGroupOpened(notification.app_key.clone()),
                        Message::NotificationGroupCleared(notification.app_key.clone()),
                    )
                } else {
                    (
                        Message::NotificationPressed(id),
                        Message::NotificationDismissed(id),
                    )
                };
                layers.push(
                    pin(card(
                        notification,
                        hovered == Some(id),
                        material,
                        on_press,
                        on_close,
                    ))
                    .x(CARD_X - OVERHANG)
                    .y(y - OVERHANG)
                    .into(),
                );
                if closed_group {
                    layers.push(
                        pin(container(Space::new())
                            .width(CARD_WIDTH - 2.0 * PEEK_INSET)
                            .height(PEEK_HEIGHT - 1.0)
                            .style(move |_| panel_style(material, (PEEK_HEIGHT - 1.0) / 2.0)))
                        .x(CARD_X + PEEK_INSET)
                        .y(y + notification.height() + 1.0)
                        .into(),
                    );
                }
            }
        }
    }

    // Scrolling anywhere works; presses on the empty part fall through and close the panel.
    mouse_area(
        container(stack(layers))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_scroll(|delta| {
        Message::NotificationCenterScrolled(match delta {
            // A wheel notch comes as both; touchpads only send pixels.
            ScrollDelta::Lines { .. } => 0.0,
            ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_ROW,
        })
    })
    .into()
}

/// A title directly on the desktop, with Tahoe's soft shadow, which keeps it readable on a bright
/// wallpaper. It starts [`SHADOW_REACH`] up and to the left of the text.
fn title(content: &str) -> Element<'_> {
    let width = menu::text_width(content, TITLE_SIZE, fonts::BOLD) + 2.0 * SHADOW_REACH;
    let height = LINE_HEIGHT * TITLE_SIZE + 2.0 * SHADOW_REACH;
    let shadow = SHADOWS.with_borrow_mut(|shadows| {
        shadows
            .entry(content.to_owned())
            .or_insert_with(|| text_shadow(content, width, height))
            .clone()
    });
    stack![
        image(shadow).width(width).height(height),
        pin(text(content)
            .size(TITLE_SIZE)
            .font(fonts::BOLD)
            .color(Color::WHITE)
            .shaping(Shaping::Advanced)
            .wrapping(Wrapping::None))
        .x(SHADOW_REACH)
        .y(SHADOW_REACH),
    ]
    .width(width)
    .height(height)
    .into()
}

thread_local! {
    /// Titles' shadows, which are slow to make and slower to upload again each frame.
    static SHADOWS: RefCell<HashMap<String, image::Handle>> = RefCell::default();
}

/// The shadow of a title, as an image of this size in logical pixels: its glyphs, drawn by the same
/// text engine that draws the text, and blurred, since a renderer can't blur text.
fn text_shadow(content: &str, width: f32, height: f32) -> image::Handle {
    let (pixel_width, pixel_height) = (
        (width * SHADOW_SCALE).ceil() as usize,
        (height * SHADOW_SCALE).ceil() as usize,
    );
    let mut coverage = vec![0.0_f32; pixel_width * pixel_height];

    let mut system = iced_widget::graphics::text::font_system()
        .write()
        .expect("text system");
    let fonts = system.raw();
    let metrics = Metrics::new(
        TITLE_SIZE * SHADOW_SCALE,
        LINE_HEIGHT * TITLE_SIZE * SHADOW_SCALE,
    );
    let mut buffer = Buffer::new(fonts, metrics);
    buffer.set_size(fonts, None, None);
    buffer.set_text(
        fonts,
        content,
        &Attrs::new()
            .family(cosmic_text::Family::Name("Inter"))
            .weight(Weight::SEMIBOLD),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(fonts, false);
    let offset = (SHADOW_REACH * SHADOW_SCALE) as i32;
    buffer.draw(
        fonts,
        &mut SwashCache::new(),
        cosmic_text::Color::rgb(255, 255, 255),
        |x, y, w, h, color| {
            for py in (y + offset)..(y + offset + h as i32) {
                for px in (x + offset)..(x + offset + w as i32) {
                    if px >= 0
                        && py >= 0
                        && (px as usize) < pixel_width
                        && let Some(value) =
                            coverage.get_mut(py as usize * pixel_width + px as usize)
                    {
                        *value = value.max(f32::from(color.a()) / 255.0);
                    }
                }
            }
        },
    );

    // Three box blurs come close to a Gaussian.
    for _ in 0..3 {
        box_blur(&mut coverage, pixel_width, pixel_height);
    }
    let pixels: Vec<u8> = coverage
        .iter()
        .flat_map(|value| [0, 0, 0, ((value * SHADOW_STRENGTH).min(1.0) * 255.0) as u8])
        .collect();
    image::Handle::from_rgba(pixel_width as u32, pixel_height as u32, pixels)
}

/// Blurs along rows, then along columns.
fn box_blur(values: &mut [f32], width: usize, height: usize) {
    let radius = SHADOW_BLUR_RADIUS;
    let blur_line = |get: &dyn Fn(usize) -> f32, length: usize, out: &mut Vec<f32>| {
        out.clear();
        out.extend((0..length).map(|center| {
            let first = center.saturating_sub(radius);
            let last = (center + radius).min(length - 1);
            (first..=last).map(get).sum::<f32>() / (2 * radius + 1) as f32
        }));
    };
    let mut line = Vec::new();
    for y in 0..height {
        let row: Vec<f32> = values[y * width..(y + 1) * width].to_vec();
        blur_line(&|x| row[x], width, &mut line);
        values[y * width..(y + 1) * width].copy_from_slice(&line);
    }
    for x in 0..width {
        let column: Vec<f32> = (0..height).map(|y| values[y * width + x]).collect();
        blur_line(&|y| column[y], height, &mut line);
        for (y, value) in line.iter().enumerate() {
            values[y * width + x] = *value;
        }
    }
}

/// An open group's "Show less", and the ✕ that clears it.
fn header<'a>(app_key: &str, material: Option<material::Kind>) -> Element<'a> {
    let show_less = mouse_area(
        container(text("Show less").size(12).color(CHIP_INK))
            .center_x(SHOW_LESS_WIDTH)
            .center_y(SHOW_LESS_HEIGHT)
            .style(move |_| panel_style(material, SHOW_LESS_HEIGHT / 2.0)),
    )
    .on_press(Message::NotificationGroupClosed(app_key.to_owned()));

    row![
        Space::new().width(Length::Fill),
        show_less,
        close_button(
            Message::NotificationGroupCleared(app_key.to_owned()),
            material
        ),
    ]
    .spacing(CONTROL_GAP)
    .width(CARD_WIDTH)
    .height(SHOW_LESS_HEIGHT)
    .align_y(Alignment::Center)
    .into()
}
