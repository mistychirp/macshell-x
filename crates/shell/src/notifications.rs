//! Notifications from apps: the session's `org.freedesktop.Notifications` service, what a
//! notification holds, and the card that shows one, in a banner or in Notification Center.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt as _, Stream, future};
use iced_widget::core::text::{LineHeight, Shaping};
use iced_widget::core::{Alignment, Background, Color, Length, Rectangle, Size};
use iced_widget::{Space, column, container, image, mouse_area, pin, row, stack, text};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::anim;
use crate::app::{Element, Message};
use crate::apps::AppIndex;
use crate::group::group;
use crate::icons::{self, Glyph};
use crate::{dbus, fonts, material, menu};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// The action a press on the card runs, if the app gave one.
pub const DEFAULT_ACTION: &str = "default";

/// How long a banner stays when the app leaves it up to us.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Why a notification closed, from the specification.
pub const CLOSED_EXPIRED: u32 = 1;
pub const CLOSED_DISMISSED: u32 = 2;
const CLOSED_BY_APP: u32 = 3;

pub const CARD_WIDTH: f32 = 342.0;
/// How far a card's close button sticks out past its top left corner.
pub const OVERHANG: f32 = 8.0;
pub const RADIUS: f32 = 20.0;
const PADDING_X: f32 = 12.0;
const PADDING_END: f32 = 16.0;
const PADDING_Y: f32 = 12.0;
const ICON_SIZE: f32 = 32.0;
const ICON_GAP: f32 = 13.0;
const TEXT_WIDTH: f32 = CARD_WIDTH - PADDING_X - ICON_SIZE - ICON_GAP - PADDING_END;
const TEXT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 16.0;
const MAX_BODY_LINES: f32 = 4.0;
/// Room left beside a title for the time.
const TIME_ROOM: f32 = 76.0;
const TIME_SIZE: f32 = 11.0;
const TIME_INK: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.55);
const ATTACHMENT_SIZE: f32 = 32.0;
const ATTACHMENT_RADIUS: f32 = 7.0;
const ATTACHMENT_GAP: f32 = 10.0;
/// Far more than fits on four lines, so that measuring a huge message stays quick.
const MAX_BODY_CHARS: usize = 600;
const MAX_ACTIONS: usize = 3;
/// The capsule that shows under the pointer, for what can be done with a notification.
const CAPSULE_HEIGHT: f32 = 22.0;
const CAPSULE_TEXT_SIZE: f32 = 12.0;
const CAPSULE_PADDING: f32 = 10.0;
const CAPSULE_CHEVRON: f32 = 8.0;
const CAPSULE_INSET_X: f32 = 12.0;
const CAPSULE_INSET_BOTTOM: f32 = 10.0;
/// How wide the text fades out over before it would run under the capsule, and in how many steps,
/// since a renderer can't draw a gradient of opacity.
const TEXT_FADE: f32 = 30.0;
const TEXT_FADE_STEPS: usize = 6;
/// How long the capsule takes to show and to go, measured from Tahoe's.
const HOVER_IN: Duration = Duration::from_millis(130);
const HOVER_OUT: Duration = Duration::from_millis(250);
pub const CLOSE_SIZE: f32 = 19.0;
/// What's on a chip, like the ✕ on its button.
pub const CHIP_INK: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.75);

/// What the service hears from apps.
#[derive(Debug, Clone)]
pub enum Event {
    Notified(Incoming),
    /// The app took a notification back.
    Closed(u32),
}

#[derive(Debug, Clone)]
pub struct Incoming {
    pub id: u32,
    /// Whether this updates the notification with this id, rather than being a new one.
    pub replaces: bool,
    app_name: String,
    desktop_entry: Option<String>,
    /// The icon the app named for itself.
    app_icon: String,
    icon: IncomingIcon,
    summary: String,
    body: String,
    actions: Vec<(String, String)>,
    timeout: Option<Duration>,
    transient: bool,
}

#[derive(Debug, Clone)]
enum IncomingIcon {
    Pixels(image::Handle),
    Named(String),
}

/// A notification, ready to show.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    /// What groups it with others: its desktop entry, or else the name the app gave.
    pub app_key: String,
    pub app: String,
    icon: Option<image::Handle>,
    /// A picture of its own, like who a message is from, beside the text.
    attachment: Option<image::Handle>,
    received: DateTime<Local>,
    title: String,
    body: String,
    body_height: f32,
    /// Buttons: `(key, label)`.
    actions: Vec<(String, String)>,
    pub has_default_action: bool,
    /// How long its banner stays; `None` until it's dismissed.
    pub timeout: Option<Duration>,
    /// It's only a banner, and isn't kept in Notification Center.
    pub transient: bool,
}

impl Notification {
    pub fn new(incoming: Incoming, apps: &mut AppIndex) -> Self {
        let entry = incoming.desktop_entry.as_deref();
        let app = apps.resolve(entry.unwrap_or(&incoming.app_name));
        // Without an entry, the name the app gave is better than one made up from its id.
        let name = if app.command.is_some() || incoming.app_name.is_empty() {
            app.name.clone()
        } else {
            incoming.app_name.clone()
        };
        let own_icon = (!incoming.app_icon.is_empty())
            .then(|| apps.themed_icon(file_path(&incoming.app_icon)))
            .flatten()
            .or_else(|| app.icon.clone());
        let (icon, attachment) = match incoming.icon {
            // Pixels are usually something particular to this notification, like who a message
            // is from. The app's icon goes first if it has one, like on macOS.
            IncomingIcon::Pixels(handle) => match own_icon {
                Some(icon) => (Some(icon), Some(handle)),
                None => (Some(handle), None),
            },
            IncomingIcon::Named(name) => (apps.themed_icon(file_path(&name)).or(own_icon), None),
        };
        let icon = icon.or_else(|| apps.themed_icon("dialog-information"));

        let title = menu::fit(
            &incoming.summary,
            TEXT_SIZE,
            fonts::BOLD,
            TEXT_WIDTH - TIME_ROOM,
        );
        let body_width = body_width(attachment.is_some());
        let body = clamp_lines(&plain_text(&incoming.body), body_width);
        let body_height = if body.is_empty() {
            0.0
        } else {
            body_height(&body, body_width)
        };

        Self {
            id: incoming.id,
            app_key: incoming.desktop_entry.unwrap_or(incoming.app_name),
            app: name,
            icon,
            attachment,
            received: Local::now(),
            title,
            body,
            body_height,
            has_default_action: incoming
                .actions
                .iter()
                .any(|(key, _)| key == DEFAULT_ACTION),
            actions: incoming
                .actions
                .into_iter()
                .filter(|(key, _)| key != DEFAULT_ACTION)
                .take(MAX_ACTIONS)
                .collect(),
            timeout: incoming.timeout,
            transient: incoming.transient,
        }
    }

    /// When it came, the way macOS says it: how long ago, or the day once that's too long.
    pub fn time_label(&self) -> String {
        time_ago(self.received, Local::now())
    }

    /// What its capsule says, and whether that opens a menu of what can be done, if anything can.
    fn capsule(&self) -> Option<(&str, bool)> {
        match self.actions.as_slice() {
            [] => None,
            [(_, label)] => Some((label, false)),
            _ => Some(("Options", true)),
        }
    }

    /// Where its capsule goes, from the card's top left corner.
    pub fn capsule_rect(&self) -> Option<Rectangle> {
        let (label, opens_menu) = self.capsule()?;
        let label = menu::fit(label, CAPSULE_TEXT_SIZE, fonts::REGULAR, CARD_WIDTH / 2.0);
        let width = (menu::text_width(&label, CAPSULE_TEXT_SIZE, fonts::REGULAR)
            + 2.0 * CAPSULE_PADDING
            + if opens_menu {
                CAPSULE_CHEVRON + 4.0
            } else {
                0.0
            })
        .ceil();
        Some(Rectangle {
            x: CARD_WIDTH - CAPSULE_INSET_X - width,
            y: self.height() - CAPSULE_INSET_BOTTOM - CAPSULE_HEIGHT,
            width,
            height: CAPSULE_HEIGHT,
        })
    }

    /// The menu of what can be done with it, if it has more than one thing.
    pub fn options(&self) -> Vec<menu::Entry> {
        self.actions
            .iter()
            .map(|(key, label)| {
                menu::Entry::item(label, Message::NotificationAction(self.id, key.clone()))
            })
            .collect()
    }

    /// How tall its card is, without the close button's overhang.
    pub fn height(&self) -> f32 {
        (LINE_HEIGHT + self.body_height).max(ICON_SIZE) + 2.0 * PADDING_Y
    }
}

/// How long ago something happened, the way Tahoe puts it.
fn time_ago(then: DateTime<Local>, now: DateTime<Local>) -> String {
    let minutes = (now - then).num_minutes();
    let day = then.date_naive();
    let today = now.date_naive();
    if minutes < 1 {
        String::from("now")
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if day == today {
        format!("{}h ago", minutes / 60)
    } else if today.pred_opt() == Some(day) {
        String::from("Yesterday")
    } else {
        then.format("%b %-d").to_string()
    }
}

/// How much a card's capsule shows, which comes and goes gradually with the pointer.
#[derive(Debug, Default)]
pub struct HoverFade {
    current: Option<u32>,
    /// Those on their way: which, from how much, and since when. The one that's [`Self::current`]
    /// is on its way in, and the others out.
    moving: Vec<(u32, f32, Instant)>,
}

impl HoverFade {
    fn time(&self, id: u32) -> Duration {
        if self.current == Some(id) {
            HOVER_IN
        } else {
            HOVER_OUT
        }
    }

    pub fn amount(&self, id: u32) -> f32 {
        let target = if self.current == Some(id) { 1.0 } else { 0.0 };
        self.moving
            .iter()
            .find(|(moving, ..)| *moving == id)
            .map_or(target, |(_, from, started)| {
                let t = (anim::seconds_since(*started) / self.time(id).as_secs_f32()).min(1.0);
                from + (target - from) * t * t * (3.0 - 2.0 * t)
            })
    }

    /// Says which card the pointer's over now.
    pub fn aim(&mut self, hovered: Option<u32>) {
        if hovered != self.current {
            let starts: Vec<(u32, f32)> = [self.current, hovered]
                .into_iter()
                .flatten()
                .map(|id| (id, self.amount(id)))
                .collect();
            self.current = hovered;
            self.moving
                .retain(|(id, ..)| !starts.iter().any(|(start, _)| start == id));
            self.moving
                .extend(starts.into_iter().map(|(id, from)| (id, from, anim::now())));
        }
        let moving = std::mem::take(&mut self.moving);
        self.moving = moving
            .into_iter()
            .filter(|(id, _, started)| anim::seconds_since(*started) < self.time(*id).as_secs_f32())
            .collect();
    }

    pub fn is_animating(&self) -> bool {
        !self.moving.is_empty()
    }
}

fn file_path(icon: &str) -> &str {
    icon.strip_prefix("file://").unwrap_or(icon)
}

/// How wide the text under the title can be, with an attachment beside it or without.
fn body_width(has_attachment: bool) -> f32 {
    if has_attachment {
        TEXT_WIDTH - ATTACHMENT_SIZE - ATTACHMENT_GAP
    } else {
        TEXT_WIDTH
    }
}

fn body_height(body: &str, width: f32) -> f32 {
    menu::text_height(body, TEXT_SIZE, LINE_HEIGHT, fonts::REGULAR, width)
}

/// `body`, cut short with an ellipsis if it takes more lines than a card shows.
fn clamp_lines(body: &str, width: f32) -> String {
    let fits = |text: &str| body_height(text, width) <= MAX_BODY_LINES * LINE_HEIGHT;
    let chars: Vec<char> = body.chars().take(MAX_BODY_CHARS).collect();
    if chars.len() == body.chars().count() && fits(body) {
        return body.to_owned();
    }
    let shortened = |keep: usize| {
        let mut text: String = chars[..keep].iter().collect();
        text.truncate(text.trim_end().len());
        text.push('…');
        text
    };
    // The longest prefix that fits.
    let (mut fitting, mut too_long) = (0, chars.len());
    while too_long - fitting > 1 {
        let middle = (fitting + too_long) / 2;
        if fits(&shortened(middle)) {
            fitting = middle;
        } else {
            too_long = middle;
        }
    }
    shortened(fitting)
}

/// `body` without markup. We don't advertise `body-markup`, but apps send it regardless, most
/// often bold text and links.
fn plain_text(body: &str) -> String {
    let mut plain = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find('<') {
        plain.push_str(&rest[..start]);
        rest = &rest[start..];
        match markup_tag(rest) {
            Some((end, replacement)) => {
                plain.push_str(replacement);
                rest = &rest[end..];
            }
            None => {
                plain.push('<');
                rest = &rest[1..];
            }
        }
    }
    plain.push_str(rest);
    plain
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// If `text` starts with a tag that's markup rather than, say, `a < b`: where the tag ends, and
/// what replaces it.
fn markup_tag(text: &str) -> Option<(usize, &'static str)> {
    let end = text.find('>')? + 1;
    let tag = text[1..end - 1].trim_start_matches('/');
    let name = tag
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or_default();
    match name.to_ascii_lowercase().as_str() {
        "b" | "i" | "u" | "a" | "img" | "span" => Some((end, "")),
        "br" => Some((end, "\n")),
        _ => None,
    }
}

/// What a notification says, on a card of [`CARD_WIDTH`] and [`Notification::height`]. `capsule`:
/// how much its capsule shows, which the text fades out in front of.
fn face<'a>(
    notification: &'a Notification,
    time: Option<String>,
    capsule: f32,
    material: Option<material::Kind>,
) -> Element<'a> {
    let height = notification.height();
    let icon: Element<'a> = match &notification.icon {
        Some(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .border_radius(7.0)
            .into(),
        None => Space::new().width(ICON_SIZE).height(ICON_SIZE).into(),
    };

    let width = body_width(notification.attachment.is_some());
    let line_height = LineHeight::Absolute(LINE_HEIGHT.into());
    let heading = row![
        text(&notification.title)
            .size(TEXT_SIZE)
            .line_height(line_height)
            .font(fonts::BOLD)
            .shaping(Shaping::Advanced),
        Space::new().width(Length::Fill),
    ]
    .push(time.map(|time| {
        text(time)
            .size(TIME_SIZE)
            .line_height(line_height)
            .color(TIME_INK)
    }))
    .width(width);

    let body = || {
        text(&notification.body)
            .size(TEXT_SIZE)
            .line_height(line_height)
            .shaping(Shaping::Advanced)
            .width(width)
    };
    let text_left = PADDING_X + ICON_SIZE + ICON_GAP;
    let under: Element<'a> = match notification.capsule_rect().filter(|_| capsule > 0.0) {
        Some(rect) if !notification.body.is_empty() => {
            let room = (rect.x - text_left - 4.0).min(width);
            let area = Size::new(width, notification.body_height);
            let solid = (room - TEXT_FADE).max(0.0);
            // Side by side, so that nothing's drawn twice, which would dim or thicken it: what's clear
            // of the capsule as it is, what would run under it going, and strips of less and less
            // of it in between.
            let mut layers: Vec<Element<'a>> = vec![
                group(body()).clip(Size::new(solid, area.height)).into(),
                group(body())
                    .opacity(1.0 - capsule)
                    .clip(area)
                    .clip_left(room)
                    .into(),
            ];
            let strip = (room - solid) / TEXT_FADE_STEPS as f32;
            for step in 0..TEXT_FADE_STEPS {
                let left = solid + strip * step as f32;
                let thins_to = 1.0 - (step as f32 + 0.5) / TEXT_FADE_STEPS as f32;
                layers.push(
                    group(body())
                        .opacity(1.0 - capsule * (1.0 - thins_to))
                        .clip(Size::new(left + strip, area.height))
                        .clip_left(left)
                        .into(),
                );
            }
            stack(layers).width(width).height(area.height).into()
        }
        _ if notification.body.is_empty() => Space::new().into(),
        _ => body().into(),
    };

    let mut content = row![icon, column![heading, under]]
        .spacing(ICON_GAP)
        .align_y(Alignment::Center);
    if let Some(handle) = &notification.attachment {
        // The capsule takes its place.
        let shows = if notification.capsule().is_some() {
            1.0 - capsule
        } else {
            1.0
        };
        content = content
            .push(Space::new().width(ATTACHMENT_GAP - ICON_GAP))
            .push(
                image(handle.clone())
                    .width(ATTACHMENT_SIZE)
                    .height(ATTACHMENT_SIZE)
                    .border_radius(ATTACHMENT_RADIUS)
                    .opacity(shows),
            );
    }
    container(content)
        .padding(iced_widget::core::Padding {
            top: PADDING_Y,
            bottom: PADDING_Y,
            left: PADDING_X,
            right: PADDING_END,
        })
        .width(CARD_WIDTH)
        .height(height)
        .align_y(Alignment::Center)
        .style(move |_| panel_style(material, RADIUS))
        .into()
}

/// A notification's card with its close button, which shows while `hovered`, and its capsule, which
/// shows as much as `capsule`. It takes up [`CARD_WIDTH`] and [`Notification::height`], plus
/// [`OVERHANG`] above and to the left. `origin` is where the card's corner is on its surface.
#[allow(clippy::too_many_arguments)]
pub fn card<'a>(
    notification: &'a Notification,
    time: Option<String>,
    hovered: bool,
    capsule: f32,
    origin: (f32, f32),
    material: Option<material::Kind>,
    on_press: Message,
    on_close: Message,
) -> Element<'a> {
    let id = notification.id;
    let height = notification.height();
    let mut layers: Vec<Element<'a>> = vec![
        pin(face(notification, time, capsule, material))
            .x(OVERHANG)
            .y(OVERHANG)
            .into(),
    ];
    if capsule > 0.0
        && let (Some((label, opens_menu)), Some(rect)) =
            (notification.capsule(), notification.capsule_rect())
    {
        let on_capsule = if opens_menu {
            Message::NotificationOptions(
                id,
                (
                    (origin.0 + rect.x) as i32,
                    (origin.1 + rect.y) as i32,
                    rect.width as i32,
                    rect.height as i32,
                ),
            )
        } else {
            let key = notification.actions[0].0.clone();
            Message::NotificationAction(id, key)
        };
        let words = menu::fit(label, CAPSULE_TEXT_SIZE, fonts::REGULAR, CARD_WIDTH / 2.0);
        let mut inside = row![text(words).size(CAPSULE_TEXT_SIZE).color(Color::WHITE)]
            .spacing(4)
            .align_y(Alignment::Center);
        if opens_menu {
            inside = inside.push(icons::icon(
                Glyph::ChevronDown,
                CAPSULE_CHEVRON,
                Color::WHITE,
            ));
        }
        layers.push(
            pin(group(
                mouse_area(
                    container(inside)
                        .center_x(rect.width)
                        .center_y(rect.height)
                        .style(move |_| panel_style(material, CAPSULE_HEIGHT / 2.0)),
                )
                .on_press(on_capsule),
            )
            .opacity(capsule))
            .x(OVERHANG + rect.x)
            .y(OVERHANG + rect.y)
            .into(),
        );
    }
    if hovered {
        layers.push(close_button(on_close, material));
    }
    mouse_area(
        container(stack(layers))
            .width(CARD_WIDTH + OVERHANG)
            .height(height + OVERHANG),
    )
    .on_press(on_press)
    .on_enter(Message::NotificationEntered(id))
    .on_exit(Message::NotificationLeft(id))
    .into()
}

/// How a closed card goes in Notification Center, measured from Tahoe's: shrinking to about half,
/// going out of focus and fading, `t` of the way through. A renderer can't blur, so it's drawn
/// several times around a growing circle, which over this little time reads the same.
pub fn closing_face<'a>(
    notification: &'a Notification,
    time: Option<String>,
    t: f32,
    material: Option<material::Kind>,
) -> Element<'a> {
    const COPIES: usize = 8;
    const BLUR: f32 = 7.0;
    let shows = 1.0 - t;
    // What each copy has to be for all of them together to come to that.
    let each = 1.0 - (1.0 - shows).powf(1.0 / COPIES as f32);
    let size = Size::new(CARD_WIDTH, notification.height());
    let layers: Vec<Element<'a>> = (0..COPIES)
        .map(|copy| {
            let angle = copy as f32 / COPIES as f32 * std::f32::consts::TAU;
            pin(group(face(notification, time.clone(), 0.0, material)).opacity(each))
                .x(angle.cos() * BLUR * t)
                .y(angle.sin() * BLUR * t)
                .into()
        })
        .collect();
    group(stack(layers).width(size.width).height(size.height))
        .scale(closing_scale(t))
        .into()
}

/// How big a closed card is, `t` of the way through going.
pub fn closing_scale(t: f32) -> f32 {
    1.0 - 0.5 * t
}

/// The round ✕ that dismisses something, on the glass of [`close_shape`].
pub fn close_button<'a>(on_press: Message, material: Option<material::Kind>) -> Element<'a> {
    mouse_area(
        container(icons::icon(Glyph::Close, 9.0, CHIP_INK))
            .center(CLOSE_SIZE)
            .style(move |_| panel_style(material, CLOSE_SIZE / 2.0)),
    )
    .on_press(on_press)
    .into()
}

/// The glass of a [`close_button`] at this position.
pub fn close_shape(x: f32, y: f32) -> material::Shape {
    chip_shape(x, y, CLOSE_SIZE, CLOSE_SIZE)
}

/// The glass of a small button.
pub fn chip_shape(x: f32, y: f32, width: f32, height: f32) -> material::Shape {
    material::Shape {
        rect: (x as i32, y as i32, width as i32, height as i32),
        radius: (height / 2.0) as i32,
        style: material::Style::Chip,
    }
}

/// A panel's own fill: nothing over glass, enough to read its text otherwise.
pub fn panel_style(material: Option<material::Kind>, radius: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(
            0.1,
            0.1,
            0.12,
            material::panel_tint(material),
        ))),
        border: material::panel_border(material, radius),
        text_color: Some(Color::WHITE),
        ..Default::default()
    }
}

struct Server {
    next_id: AtomicU32,
    events: mpsc::Sender<Event>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id == 0 {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        } else {
            replaces_id
        };
        let critical = matches!(hint(&hints, &["urgency"]), Some(Value::U8(2)));
        let timeout = match expire_timeout {
            0 => None,
            -1 if critical => None,
            milliseconds if milliseconds > 0 => {
                Some(Duration::from_millis(milliseconds.unsigned_abs().into()))
            }
            _ => Some(DEFAULT_TIMEOUT),
        };
        let incoming = Incoming {
            id,
            replaces: replaces_id != 0,
            desktop_entry: match hint(&hints, &["desktop-entry"]) {
                Some(Value::Str(entry)) if !entry.is_empty() => Some(entry.to_string()),
                _ => None,
            },
            icon: incoming_icon(&hints, app_icon.clone()),
            app_icon,
            app_name,
            summary,
            body,
            actions: actions
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[key, label]| (key.clone(), label.clone()))
                .collect(),
            timeout,
            transient: matches!(hint(&hints, &["transient"]), Some(Value::Bool(true))),
        };
        let mut events = self.events.clone();
        if events.send(Event::Notified(incoming)).await.is_err() {
            log::warn!("nothing is listening for notifications");
        }
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let mut events = self.events.clone();
        let _ = events.send(Event::Closed(id)).await;
        if let Err(err) = Self::notification_closed(&emitter, id, CLOSED_BY_APP).await {
            log::warn!("can't report a closed notification: {err}");
        }
    }

    // Persistence: a notification lives on in Notification Center after its banner, so it isn't
    // closed, and its actions keep working, until it's dismissed there.
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["actions", "body", "icon-static", "persistence"]
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("macshell", "macshell-x", env!("CARGO_PKG_VERSION"), "1.2")
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

fn hint<'a>(hints: &'a HashMap<String, OwnedValue>, names: &[&str]) -> Option<&'a Value<'static>> {
    names
        .iter()
        .find_map(|name| hints.get(*name))
        .map(|value| &**value)
}

/// The most specific picture the app offered, in the specification's order.
fn incoming_icon(hints: &HashMap<String, OwnedValue>, app_icon: String) -> IncomingIcon {
    if let Some(handle) =
        hint(hints, &["image-data", "image_data", "icon_data"]).and_then(image_data)
    {
        return IncomingIcon::Pixels(handle);
    }
    match hint(hints, &["image-path", "image_path"]) {
        Some(Value::Str(path)) if !path.is_empty() => IncomingIcon::Named(path.to_string()),
        _ => IncomingIcon::Named(app_icon),
    }
}

/// Pixels sent as `(width, height, rowstride, has_alpha, bits_per_sample, channels, data)`.
fn image_data(value: &Value<'_>) -> Option<image::Handle> {
    let Value::Structure(structure) = value else {
        return None;
    };
    let [
        Value::I32(width),
        Value::I32(height),
        Value::I32(stride),
        Value::Bool(_),
        Value::I32(8),
        Value::I32(channels),
        Value::Array(data),
    ] = structure.fields()
    else {
        return None;
    };
    let (width, height, stride, channels) = (
        usize::try_from(*width).ok()?,
        usize::try_from(*height).ok()?,
        usize::try_from(*stride).ok()?,
        usize::try_from(*channels).ok()?,
    );
    if !(3..=4).contains(&channels) || stride < width * channels {
        return None;
    }
    let bytes: Vec<u8> = data
        .iter()
        .map(|byte| match byte {
            Value::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if bytes.len() < stride * height.checked_sub(1)? + width * channels {
        return None;
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for line in bytes.chunks(stride).take(height) {
        for pixel in line[..width * channels].chunks_exact(channels) {
            rgba.extend_from_slice(&pixel[..3]);
            rgba.push(pixel.get(3).copied().unwrap_or(u8::MAX));
        }
    }
    Some(image::Handle::from_rgba(width as u32, height as u32, rgba))
}

/// Serves the session's notifications, and reports what apps send.
pub fn events() -> Subscription<Event> {
    Subscription::run(service)
}

fn service() -> impl Stream<Item = Event> {
    iced_futures::stream::channel(32, |output: mpsc::Sender<Event>| async move {
        let Some(connection) = dbus::session().await else {
            return;
        };
        let server = Server {
            next_id: AtomicU32::new(1),
            events: output,
        };
        if let Err(err) = connection.object_server().at(PATH, server).await {
            log::warn!("can't serve notifications: {err}");
            return;
        }
        let flags = RequestNameFlags::AllowReplacement | RequestNameFlags::ReplaceExisting;
        match connection.request_name_with_flags(SERVICE, flags).await {
            Ok(RequestNameReply::InQueue) => {
                log::warn!("another notification service is running; waiting for it to go away");
            }
            Ok(_) => log::info!("serving the session's notifications"),
            Err(err) => log::warn!("can't take the notification service's name: {err}"),
        }
        // Requests arrive on the connection's own task, for as long as the shell runs.
        future::pending::<()>().await;
    })
}

/// Tells the app that one of a notification's actions was invoked.
pub async fn invoke_action(id: u32, key: String) {
    emit("ActionInvoked", &(id, key)).await;
}

/// Tells the app that a notification closed.
pub async fn report_closed(id: u32, reason: u32) {
    emit("NotificationClosed", &(id, reason)).await;
}

async fn emit<B>(signal: &str, body: &B)
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let Some(connection) = dbus::session().await else {
        return;
    };
    if let Err(err) = connection
        .emit_signal(None::<&str>, PATH, SERVICE, signal, body)
        .await
    {
        log::warn!("can't send {signal}: {err}");
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use chrono::TimeZone as _;

    #[test]
    fn times_read_like_macos() {
        let at = |day, hour, minute| {
            Local
                .with_ymd_and_hms(2026, 11, day, hour, minute, 0)
                .unwrap()
        };
        let now = at(6, 18, 40);
        assert_eq!(time_ago(at(6, 18, 40), now), "now");
        assert_eq!(time_ago(at(6, 18, 39), now), "1m ago");
        assert_eq!(time_ago(at(6, 16, 10), now), "2h ago");
        assert_eq!(time_ago(at(5, 23, 0), now), "Yesterday");
        assert_eq!(time_ago(at(1, 9, 0), now), "Nov 1");
    }

    #[test]
    fn a_capsule_comes_and_goes_gradually() {
        let mut fade = HoverFade::default();
        assert_eq!(fade.amount(1), 0.0);
        fade.aim(Some(1));
        assert!(fade.is_animating());
        assert!(fade.amount(1) < 0.5);
        fade.moving[0].2 -= HOVER_IN * 2;
        assert_eq!(fade.amount(1), 1.0);
        fade.aim(Some(1));
        assert!(!fade.is_animating());
        fade.aim(None);
        assert!(fade.amount(1) > 0.5);
    }
}
