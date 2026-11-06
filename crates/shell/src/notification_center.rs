//! Notification Center, which hangs from the clock: past notifications with no panel behind
//! them, grouped by app. A group shows as one card with the edge of the next peeking out from
//! under it, until it's opened.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use iced_widget::core::mouse::ScrollDelta;
use iced_widget::core::text::{Shaping, Wrapping};
use iced_widget::core::{Alignment, Color, Length};
use iced_widget::graphics::text::cosmic_text::{self, Attrs, Buffer, Metrics, SwashCache, Weight};
use iced_widget::{Space, container, image, mouse_area, pin, row, stack, text};

use crate::group::group;

use crate::app::{Element, Message};
use crate::notifications::{
    CARD_WIDTH, CHIP_INK, CLOSE_SIZE, HoverFade, Notification, OVERHANG, RADIUS, Spring, card,
    chip_shape, close_button, close_shape, closing_face, closing_scale, panel_style,
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
const PEEKS: [f32; 2] = [10.0, 22.0];
const PEEK_HEIGHT: f32 = 8.0;
const EMPTY_TOP: f32 = 40.0;
const TITLE_SIZE: f32 = 15.0;
const PANEL_TITLE_SIZE: f32 = 17.0;
const PANEL_TITLE_HEIGHT: f32 = 24.0;
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

/// How long a closed card takes to go, measured from Tahoe's, and how far it rises as it does.
const CLOSING_TIME: Duration = Duration::from_millis(110);
const CLOSING_RISE: f32 = 40.0;
/// How long a row that's new here takes to show, when a group opens.
const ARRIVAL_TIME: Duration = Duration::from_millis(180);

/// How rows get to where they should be, as others come and go, and the cards on their way out.
#[derive(Default)]
pub struct Motion {
    rows: HashMap<String, RowMotion>,
    closing: Vec<Closing>,
    last: Option<Instant>,
}

struct RowMotion {
    y: Spring,
    target: f32,
    /// When it arrived, unless it was there from the start.
    arrived: Option<Instant>,
}

struct Closing {
    notification: Notification,
    time: String,
    y: f32,
    started: Instant,
}

impl Closing {
    fn t(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() / CLOSING_TIME.as_secs_f32()).min(1.0)
    }

    fn y(&self, t: f32) -> f32 {
        self.y - CLOSING_RISE * t
    }
}

impl Motion {
    /// Moves everything along toward what's there now.
    pub fn sync(
        &mut self,
        notifications: &[Notification],
        open: &HashSet<String>,
        first: usize,
        height: f32,
        now: Instant,
    ) {
        let seconds = self
            .last
            .map_or(0.0, |last| {
                now.saturating_duration_since(last).as_secs_f32()
            })
            .min(0.05);
        let from_start = self.last.is_none();
        self.last = Some(now);
        let targets: Vec<(String, f32)> = placed(notifications, open, first, height)
            .into_iter()
            .map(|(row, y)| (row.key(), y))
            .collect();
        let mut above: Option<f32> = None;
        for (key, target) in &targets {
            let row = self.rows.entry(key.clone()).or_insert_with(|| RowMotion {
                // A new one comes out from under the one above it.
                y: Spring::at(if from_start {
                    *target
                } else {
                    above.unwrap_or(*target)
                }),
                target: *target,
                arrived: (!from_start).then_some(now),
            });
            row.target = *target;
            row.y.step(*target, seconds);
            if row
                .arrived
                .is_some_and(|arrived| now.saturating_duration_since(arrived) >= ARRIVAL_TIME)
            {
                row.arrived = None;
            }
            above = Some(row.y.value);
        }
        self.rows
            .retain(|key, _| targets.iter().any(|(target, _)| target == key));
        self.closing.retain(|closing| closing.t() < 1.0);
    }

    /// A card was closed: it goes from where it is.
    pub fn close(&mut self, notification: &Notification) {
        if let Some(row) = self.rows.get(&card_key(notification.id)) {
            self.closing.push(Closing {
                notification: notification.clone(),
                time: notification.time_label(),
                y: row.y.value,
                started: Instant::now(),
            });
        }
    }

    pub fn is_animating(&self) -> bool {
        !self.closing.is_empty()
            || self
                .rows
                .values()
                .any(|row| row.arrived.is_some() || !row.y.is_at(row.target))
    }

    fn y(&self, key: &str, target: f32) -> f32 {
        self.rows.get(key).map_or(target, |row| row.y.value)
    }

    fn shows(&self, key: &str) -> f32 {
        self.rows
            .get(key)
            .and_then(|row| row.arrived)
            .map_or(1.0, |arrived| {
                (arrived.elapsed().as_secs_f32() / ARRIVAL_TIME.as_secs_f32()).min(1.0)
            })
    }
}

fn card_key(id: u32) -> String {
    format!("card {id}")
}

/// The height of Notification Center's surface on a screen this tall: down to the dock. Any taller
/// and the compositor slides it up over the bar to keep it out of the dock's reserved area.
pub fn height(screen_height: f32) -> f32 {
    (screen_height - menubar::HEIGHT as f32 - dock::reserved_height() - 16.0).max(100.0)
}

/// What can be scrolled past.
#[derive(Debug, Clone, Copy)]
enum Row<'a> {
    /// The panel's own title, with the ✕ that clears everything.
    Title,
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
    /// What tells it from the others, wherever it moves to.
    fn key(&self) -> String {
        match self {
            Row::Title => String::from("title"),
            Row::Header { app_key, .. } => format!("header {app_key}"),
            Row::Card { notification, .. } => card_key(notification.id),
        }
    }

    fn height(&self) -> f32 {
        match self {
            Row::Title => PANEL_TITLE_HEIGHT,
            Row::Header { .. } => HEADER_HEIGHT,
            Row::Card {
                notification,
                closed_group,
            } => {
                notification.height()
                    + if *closed_group {
                        PEEKS.len() as f32 * PEEK_HEIGHT
                    } else {
                        0.0
                    }
            }
        }
    }
}

/// The rows to show, most recent app first, with each group's most recent notification first.
fn rows<'a>(notifications: &'a [Notification], open: &HashSet<String>) -> Vec<Row<'a>> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    if !notifications.is_empty() {
        rows.push(Row::Title);
    }
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

/// The glass, with how opaque each shape is.
#[allow(clippy::too_many_arguments)]
pub fn material_shapes(
    notifications: &[Notification],
    open: &HashSet<String>,
    first: usize,
    hovered: Option<u32>,
    height: f32,
    motion: &Motion,
    fade: &HoverFade,
) -> (Vec<material::Shape>, Vec<f32>) {
    let shape = |x: f32, y: f32, width: f32, height: f32, radius: f32| material::Shape {
        rect: (x as i32, y as i32, width as i32, height as i32),
        radius: radius as i32,
        style: material::Style::Card,
    };
    let mut shapes = Vec::new();
    let mut opacities = Vec::new();
    if notifications.is_empty() {
        shapes.push(material::Shape {
            rect: (0, 0, WIDTH as i32, BACKDROP_HEIGHT as i32),
            radius: 0,
            style: material::Style::Backdrop([190, 30, 100, 110]),
        });
        opacities.push(1.0);
    }
    for (row, y) in placed(notifications, open, first, height) {
        let key = row.key();
        let shows = motion.shows(&key);
        let y = motion.y(&key, y);
        let mut add = |new: material::Shape, opacity: f32| {
            shapes.push(new);
            opacities.push(opacity);
        };
        match row {
            Row::Title => {
                add(close_shape(CARD_X + CARD_WIDTH - CLOSE_SIZE, y), shows);
            }
            Row::Header { .. } => {
                let close_x = CARD_X + CARD_WIDTH - CLOSE_SIZE;
                add(
                    chip_shape(
                        close_x - CONTROL_GAP - SHOW_LESS_WIDTH,
                        y,
                        SHOW_LESS_WIDTH,
                        SHOW_LESS_HEIGHT,
                    ),
                    shows,
                );
                add(
                    close_shape(close_x, y + (SHOW_LESS_HEIGHT - CLOSE_SIZE) / 2.0),
                    shows,
                );
            }
            Row::Card {
                notification,
                closed_group,
            } => {
                let card_height = notification.height();
                add(shape(CARD_X, y, CARD_WIDTH, card_height, RADIUS), shows);
                // On top of the card's, so after it.
                let capsule = fade.amount(notification.id);
                if !closed_group
                    && capsule > 0.0
                    && let Some(rect) = notification.capsule_rect()
                {
                    add(
                        chip_shape(CARD_X + rect.x, y + rect.y, rect.width, rect.height),
                        capsule * shows,
                    );
                }
                if hovered == Some(notification.id) {
                    add(close_shape(CARD_X - OVERHANG, y - OVERHANG), shows);
                }
                if closed_group {
                    for (index, inset) in PEEKS.into_iter().enumerate() {
                        add(
                            shape(
                                CARD_X + inset,
                                y + card_height + 1.0 + index as f32 * PEEK_HEIGHT,
                                CARD_WIDTH - 2.0 * inset,
                                PEEK_HEIGHT - 1.0,
                                (PEEK_HEIGHT - 1.0) / 2.0,
                            ),
                            shows,
                        );
                    }
                }
            }
        }
    }
    for closing in &motion.closing {
        let t = closing.t();
        let scale = closing_scale(t);
        let (width, height) = (CARD_WIDTH * scale, closing.notification.height() * scale);
        shapes.push(shape(
            CARD_X + (CARD_WIDTH - width) / 2.0,
            closing.y(t) + (closing.notification.height() - height) / 2.0,
            width,
            height,
            RADIUS * scale,
        ));
        opacities.push(1.0 - t);
    }
    (shapes, opacities)
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    notifications: &'a [Notification],
    open: &HashSet<String>,
    first: usize,
    hovered: Option<u32>,
    height: f32,
    motion: &'a Motion,
    fade: &HoverFade,
    material: Option<material::Kind>,
) -> Element<'a> {
    let mut layers: Vec<Element<'a>> = Vec::new();
    if notifications.is_empty() {
        let message = "No recent notifications";
        let x = CARD_X + (CARD_WIDTH - menu::text_width(message, TITLE_SIZE, fonts::BOLD)) / 2.0;
        layers.push(
            pin(title(message, TITLE_SIZE))
                .x(x - SHADOW_REACH)
                .y(EMPTY_TOP - SHADOW_REACH)
                .into(),
        );
    }

    for (row, y) in placed(notifications, open, first, height) {
        let key = row.key();
        let shows = motion.shows(&key);
        let y = motion.y(&key, y);
        match row {
            Row::Title => {
                layers.push(
                    pin(title("Notification Center", PANEL_TITLE_SIZE))
                        .x(CARD_X - SHADOW_REACH)
                        .y(y - SHADOW_REACH - 2.0)
                        .into(),
                );
                layers.push(
                    pin(close_button(Message::NotificationsCleared, material))
                        .x(CARD_X + CARD_WIDTH - CLOSE_SIZE)
                        .y(y)
                        .into(),
                );
            }
            Row::Header { app_key, app } => {
                layers.push(
                    pin(group(header(app_key, material)).opacity(shows))
                        .x(CARD_X)
                        .y(y)
                        .into(),
                );
                layers.push(
                    pin(group(title(app, TITLE_SIZE)).opacity(shows))
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
                    pin(group(card(
                        notification,
                        Some(notification.time_label()),
                        hovered == Some(id),
                        if closed_group { 0.0 } else { fade.amount(id) },
                        (CARD_X, y),
                        material,
                        on_press,
                        on_close,
                    ))
                    .opacity(shows))
                    .x(CARD_X - OVERHANG)
                    .y(y - OVERHANG)
                    .into(),
                );
                if closed_group {
                    for (index, inset) in PEEKS.into_iter().enumerate() {
                        layers.push(
                            pin(container(Space::new())
                                .width(CARD_WIDTH - 2.0 * inset)
                                .height(PEEK_HEIGHT - 1.0)
                                .style(move |_| panel_style(material, (PEEK_HEIGHT - 1.0) / 2.0)))
                            .x(CARD_X + inset)
                            .y(y + notification.height() + 1.0 + index as f32 * PEEK_HEIGHT)
                            .into(),
                        );
                    }
                }
            }
        }
    }
    for closing in &motion.closing {
        let t = closing.t();
        layers.push(
            pin(closing_face(
                &closing.notification,
                Some(closing.time.clone()),
                t,
                material,
            ))
            .x(CARD_X)
            .y(closing.y(t))
            .into(),
        );
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
fn title(content: &str, size: f32) -> Element<'_> {
    let width = menu::text_width(content, size, fonts::BOLD) + 2.0 * SHADOW_REACH;
    let height = LINE_HEIGHT * size + 2.0 * SHADOW_REACH;
    let shadow = SHADOWS.with_borrow_mut(|shadows| {
        shadows
            .entry(content.to_owned())
            .or_insert_with(|| text_shadow(content, size, width, height))
            .clone()
    });
    stack![
        image(shadow).width(width).height(height),
        pin(text(content)
            .size(size)
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
fn text_shadow(content: &str, size: f32, width: f32, height: f32) -> image::Handle {
    let (pixel_width, pixel_height) = (
        (width * SHADOW_SCALE).ceil() as usize,
        (height * SHADOW_SCALE).ceil() as usize,
    );
    let mut coverage = vec![0.0_f32; pixel_width * pixel_height];

    let mut system = iced_widget::graphics::text::font_system()
        .write()
        .expect("text system");
    let fonts = system.raw();
    let metrics = Metrics::new(size * SHADOW_SCALE, LINE_HEIGHT * size * SHADOW_SCALE);
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

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn nothing_moves_when_nothing_is_there() {
        let mut motion = Motion::default();
        motion.sync(&[], &HashSet::new(), 0, 500.0, Instant::now());
        assert!(!motion.is_animating());
    }
}
