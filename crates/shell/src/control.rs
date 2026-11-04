//! Control Center: the panel under the bar's switches icon, like macOS's. Modules sit on
//! rounded platters over frosted glass: connectivity and Now Playing side by side, then Sound.

use iced_widget::core::{Alignment, Background, Border, Color, Font, Length, font};
use iced_widget::{Space, column, container, image, mouse_area, row, slider, text};

use crate::app::{Element, Message};
use crate::audio::Volume;
use crate::bluetooth::Bluetooth;
use crate::icons::{self, Glyph};
use crate::media::{Control, Track};
use crate::network::Wifi;
use crate::{bluetooth, material, media, menu, network};

const WIDTH: f32 = 320.0;
const PADDING: f32 = 12.0;
const GAP: f32 = 10.0;
const RADIUS: f32 = 22.0;
const PLATTER_RADIUS: f32 = 18.0;
const PLATTER_PADDING: f32 = 12.0;
const TOP_HEIGHT: f32 = 124.0;
const SOUND_HEIGHT: f32 = 72.0;
const TOGGLE_SIZE: f32 = 32.0;
const ART_SIZE: f32 = 44.0;
const TITLE_SIZE: f32 = 13.0;
const DETAIL_SIZE: f32 = 11.0;
/// Longer names are cut short with an ellipsis.
const MAX_CHARS: usize = 16;

const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};
/// macOS's accent blue, for toggles that are on.
const ACCENT: Color = Color::from_rgb(0.0, 0.48, 1.0);
const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.6);

/// What the panel shows, from the shell's state. Each module is `None` when its service or
/// hardware is missing.
#[derive(Default)]
pub struct Panel {
    pub volume: Option<Volume>,
    pub wifi: Option<Wifi>,
    pub bluetooth: Option<Bluetooth>,
    pub track: Option<Track>,
    /// The track's cover art, loaded.
    pub art: Option<image::Handle>,
}

pub fn size() -> (u32, u32) {
    let height = 2.0 * PADDING + TOP_HEIGHT + GAP + SOUND_HEIGHT;
    (WIDTH as u32, height as u32)
}

pub fn material_shapes((width, height): (u32, u32)) -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, width as i32, height as i32),
        radius: RADIUS as i32,
        style: material::Style::Frosted,
    }]
}

pub fn view(panel: &Panel, material: Option<material::Kind>) -> Element<'_> {
    let mut top = row![].spacing(GAP).height(TOP_HEIGHT);
    if panel.wifi.is_some() || panel.bluetooth.is_some() {
        top = top.push(connectivity(panel));
    }
    top = top.push(now_playing(panel));

    let sound: Element<'_> = match panel.volume {
        Some(volume) => sound(volume),
        None => platter(text("No sound output").size(TITLE_SIZE).color(DIM))
            .height(SOUND_HEIGHT)
            .center_y(SOUND_HEIGHT)
            .into(),
    };

    // Like menus: a dark tint over glass, nearly opaque without it.
    let tint = if material.is_some() { menu::TINT } else { 0.9 };
    container(column![top, sound].spacing(GAP))
        .padding(PADDING)
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

/// Wi-Fi and Bluetooth, each a round toggle with its name and status.
fn connectivity(panel: &Panel) -> Element<'_> {
    let mut rows = column![].spacing(14);
    if let Some(wifi) = &panel.wifi {
        let status = match (&wifi.network, wifi.enabled) {
            (Some(network), true) => network.as_str(),
            (None, true) => "Not Connected",
            (_, false) => "Off",
        };
        rows = rows.push(toggle_row(
            Glyph::Wifi,
            "Wi-Fi",
            status,
            wifi.enabled,
            Message::ToggleWifi,
        ));
    }
    if let Some(bluetooth) = &panel.bluetooth {
        let status = match (bluetooth.connected.first(), bluetooth.powered) {
            (Some(device), true) => device.as_str(),
            (None, true) => "On",
            (_, false) => "Off",
        };
        rows = rows.push(toggle_row(
            Glyph::Bluetooth,
            "Bluetooth",
            status,
            bluetooth.powered,
            Message::ToggleBluetooth,
        ));
    }
    platter(rows)
        .height(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

fn toggle_row<'a>(
    glyph: Glyph,
    name: &'a str,
    status: &str,
    on: bool,
    on_press: Message,
) -> Element<'a> {
    let toggle = container(icons::icon(glyph, 16.0, Color::WHITE))
        .center(TOGGLE_SIZE)
        .style(move |_| container::Style {
            background: Some(Background::Color(if on {
                ACCENT
            } else {
                Color::from_rgba(1.0, 1.0, 1.0, 0.15)
            })),
            border: Border {
                radius: (TOGGLE_SIZE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

    row![
        mouse_area(toggle).on_press(on_press),
        column![
            text(name).size(TITLE_SIZE).font(BOLD),
            text(shorten(status)).size(DETAIL_SIZE).color(DIM),
        ]
        .spacing(1),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// The playing track with its cover and transport controls, or "Not Playing".
fn now_playing(panel: &Panel) -> Element<'_> {
    let Some(track) = &panel.track else {
        return platter(
            column![
                icons::icon(Glyph::Music, 28.0, DIM),
                text("Not Playing").size(TITLE_SIZE).color(DIM),
            ]
            .spacing(8)
            .align_x(Alignment::Center),
        )
        .height(Length::Fill)
        .center(Length::Fill)
        .into();
    };

    let art: Element<'_> = match &panel.art {
        Some(handle) => image(handle.clone())
            .width(ART_SIZE)
            .height(ART_SIZE)
            .border_radius(8.0)
            .into(),
        None => container(icons::icon(Glyph::Music, 22.0, DIM))
            .center(ART_SIZE)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.12))),
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    };

    let button = |glyph, control| {
        mouse_area(container(icons::icon(glyph, 18.0, Color::WHITE)).center(28.0))
            .on_press(Message::Media(control))
    };
    let play_pause = if track.playing {
        Glyph::Pause
    } else {
        Glyph::Play
    };

    platter(
        column![
            art,
            text(shorten(&track.title)).size(12).font(BOLD),
            text(shorten(track.artist.as_deref().unwrap_or_default()))
                .size(DETAIL_SIZE)
                .color(DIM),
            Space::new().height(Length::Fill),
            row![
                button(Glyph::Previous, Control::Previous),
                button(play_pause, Control::PlayPause),
                button(Glyph::Next, Control::Next),
            ]
            .spacing(4),
        ]
        .spacing(2),
    )
    .height(Length::Fill)
    .into()
}

/// The Sound module: a title, then the volume slider between a mute button and a loud speaker.
fn sound<'a>(volume: Volume) -> Element<'a> {
    let level = if volume.muted { 0.0 } else { volume.level };
    let low = if volume.muted {
        Glyph::SpeakerMuted
    } else {
        Glyph::SpeakerLow
    };

    let slider = slider(0.0..=1.0, level, Message::VolumeChanged)
        .on_release(Message::VolumeReleased)
        .step(0.01_f32)
        .height(22.0)
        .style(|_, _| slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(Color::WHITE),
                    Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.2)),
                ),
                width: 6.0,
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 9.0 },
                background: Background::Color(Color::WHITE),
                border_width: 0.5,
                border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
            },
        });

    platter(
        column![
            text("Sound").size(TITLE_SIZE).font(BOLD),
            row![
                mouse_area(icons::icon(low, 18.0, DIM)).on_press(Message::ToggleMute),
                slider,
                icons::icon(Glyph::SpeakerHigh, 18.0, DIM),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(8),
    )
    .height(SOUND_HEIGHT)
    .into()
}

/// A module's rounded background.
fn platter<'a>(content: impl Into<Element<'a>>) -> container::Container<'a, Message> {
    container(content)
        .padding(PLATTER_PADDING)
        .width(Length::Fill)
        .clip(true)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1))),
            border: Border {
                radius: PLATTER_RADIUS.into(),
                ..Default::default()
            },
            ..Default::default()
        })
}

fn shorten(name: &str) -> String {
    if name.chars().count() <= MAX_CHARS {
        return name.to_owned();
    }
    let mut short: String = name.chars().take(MAX_CHARS - 1).collect();
    short.push('…');
    short
}

/// The panel's slower state, read off the UI thread.
#[derive(Debug, Clone)]
pub struct Status {
    pub wifi: Option<Wifi>,
    pub bluetooth: Option<Bluetooth>,
    pub track: Option<Track>,
}

pub async fn status() -> Status {
    Status {
        wifi: network::wifi().await,
        bluetooth: bluetooth::bluetooth().await,
        track: media::now_playing().await,
    }
}
