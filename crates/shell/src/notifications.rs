//! Notifications from apps: the session's `org.freedesktop.Notifications` service, what a
//! notification holds, and the card that shows one, in a banner or in Notification Center.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::{DateTime, Local};
use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{SinkExt as _, Stream, future};
use iced_widget::core::text::{LineHeight, Shaping};
use iced_widget::core::{Alignment, Background, Border, Color, Length};
use iced_widget::{Space, button, column, container, image, mouse_area, pin, row, stack, text};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::app::{Element, Message};
use crate::apps::AppIndex;
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
const ATTACHMENT_SIZE: f32 = 30.0;
const ATTACHMENT_GAP: f32 = 8.0;
/// Far more than fits on four lines, so that measuring a huge message stays quick.
const MAX_BODY_CHARS: usize = 600;
const MAX_ACTIONS: usize = 3;
const ACTIONS_HEIGHT: f32 = 32.0;
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

    /// When it came, the way macOS says it: the time today, or more than that otherwise.
    pub fn time_label(&self) -> String {
        let today = Local::now().date_naive();
        let day = self.received.date_naive();
        if day == today {
            self.received.format("%H:%M").to_string()
        } else if today.pred_opt() == Some(day) {
            self.received.format("Yesterday, %H:%M").to_string()
        } else {
            self.received.format("%b %-d").to_string()
        }
    }

    /// How tall its card is, without the close button's overhang.
    pub fn height(&self) -> f32 {
        let beside = if self.attachment.is_some() {
            ATTACHMENT_SIZE
        } else {
            0.0
        };
        let text = LINE_HEIGHT + self.body_height.max(beside);
        let actions = if self.actions.is_empty() {
            0.0
        } else {
            ACTIONS_HEIGHT
        };
        text.max(ICON_SIZE) + actions + 2.0 * PADDING_Y
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

/// A notification's card with its close button, which shows while `hovered`. It takes up
/// [`CARD_WIDTH`] and [`Notification::height`], plus [`OVERHANG`] above and to the left.
pub fn card<'a>(
    notification: &'a Notification,
    time: Option<String>,
    hovered: bool,
    material: Option<material::Kind>,
    on_press: Message,
    on_close: Message,
) -> Element<'a> {
    let id = notification.id;
    let height = notification.height();

    let icon: Element<'a> = match &notification.icon {
        Some(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .border_radius(7.0)
            .into(),
        None => Space::new().width(ICON_SIZE).height(ICON_SIZE).into(),
    };

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
    .width(TEXT_WIDTH);

    let mut under = row![].width(TEXT_WIDTH);
    if !notification.body.is_empty() {
        under = under.push(
            text(&notification.body)
                .size(TEXT_SIZE)
                .line_height(line_height)
                .shaping(Shaping::Advanced)
                .width(body_width(notification.attachment.is_some())),
        );
    }
    if let Some(handle) = &notification.attachment {
        under = under.push(Space::new().width(Length::Fill)).push(
            image(handle.clone())
                .width(ATTACHMENT_SIZE)
                .height(ATTACHMENT_SIZE)
                .border_radius(ATTACHMENT_SIZE / 2.0),
        );
    }
    let mut words = column![heading, under];
    if !notification.actions.is_empty() {
        let width = (TEXT_WIDTH - 6.0 * (notification.actions.len() as f32 - 1.0))
            / notification.actions.len() as f32;
        words = words.push(Space::new().height(8)).push(
            row(notification.actions.iter().map(|(key, label)| {
                action_button(label, width, Message::NotificationAction(id, key.clone()))
            }))
            .spacing(6),
        );
    }

    let content = container(
        row![icon, words]
            .spacing(ICON_GAP)
            .align_y(Alignment::Center),
    )
    .padding(iced_widget::core::Padding {
        top: PADDING_Y,
        bottom: PADDING_Y,
        left: PADDING_X,
        right: PADDING_END,
    })
    .width(CARD_WIDTH)
    .height(height)
    .align_y(Alignment::Center)
    .style(move |_| panel_style(material, RADIUS));

    let mut layers: Vec<Element<'a>> = vec![pin(content).x(OVERHANG).y(OVERHANG).into()];
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

fn action_button<'a>(label: &str, width: f32, on_press: Message) -> Element<'a> {
    button(
        text(menu::fit(label, 12.0, fonts::REGULAR, width - 16.0))
            .size(12)
            .center()
            .width(Length::Fill),
    )
    .on_press(on_press)
    .width(width)
    .height(24)
    .padding([0, 8])
    .style(|_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(Color::from_rgba(
                1.0,
                1.0,
                1.0,
                if hovered { 0.28 } else { 0.16 },
            ))),
            text_color: Color::WHITE,
            border: Border {
                radius: 12.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
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
