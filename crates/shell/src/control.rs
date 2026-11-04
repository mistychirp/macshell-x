//! Control Center, like Tahoe's: no panel, just separate glass bubbles on a four-column grid.
//! Capsule toggles two cells wide, round buttons one cell wide, Now Playing two by two, and
//! full-width sliders. [`modules`] places them; the view and the glass shapes both follow it.

use iced_widget::core::text::Wrapping;
use iced_widget::core::{
    Alignment, Background, Border, Color, Font, Length, Padding, Rectangle, alignment, font,
};
use iced_widget::{Space, column, container, image, mouse_area, pin, row, slider, stack, text};

use crate::app::{Element, Message};
use crate::audio::Volume;
use crate::bluetooth::Bluetooth;
use crate::icons::{self, Glyph};
use crate::kwin::KeyboardLayouts;
use crate::media::{Control, Track};
use crate::network::Wifi;
use crate::{bluetooth, material, media, menu, menubar, network};

const CELL: f32 = 63.0;
const GAP: f32 = 12.0;
/// Two cells and the gap between them.
const WIDE: f32 = 2.0 * CELL + GAP;
const WIDTH: f32 = 2.0 * WIDE + GAP;
const SLIDER_HEIGHT: f32 = 64.0;
const SLIDER_RADIUS: f32 = 30.0;
const TILE_RADIUS: f32 = 30.0;
const TOGGLE_SIZE: f32 = 37.0;
const TOGGLE_INSET: f32 = (CELL - TOGGLE_SIZE) / 2.0;
const ART_SIZE: f32 = 40.0;
const TITLE_SIZE: f32 = 13.0;
const DETAIL_SIZE: f32 = 12.0;
/// Room for a capsule's text, and Now Playing's; longer text is cut short with an ellipsis.
const CAPSULE_TEXT_WIDTH: f32 = WIDE - TOGGLE_INSET - CAPSULE_END_PADDING - TOGGLE_SIZE - TEXT_GAP;
const CAPSULE_END_PADDING: f32 = 6.0;
const TILE_TEXT_WIDTH: f32 = WIDE - 2.0 * TILE_PADDING;
const TEXT_GAP: f32 = 9.0;
const TILE_PADDING: f32 = 14.0;
/// Room around the bubbles for the blur behind them to fade out, like Tahoe's: it's at full strength
/// about 50 points to their left, and gone by about 100.
const BLUR_LEFT: f32 = 110.0;
const BLUR_BOTTOM: f32 = 90.0;
const BLUR_TOP_FADE: f32 = 30.0;
/// Between the bubbles and the edge of the screen, which the blur reaches.
const MARGIN_RIGHT: f32 = 8.0;

const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};
const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.65);
const FAINT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.3);
const OFF_FILL: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.25);
const INK_ON_WHITE: Color = Color::from_rgb(0.1, 0.1, 0.12);

/// What the panel shows, from the shell's state. Each is `None` when its service or hardware is
/// missing.
#[derive(Default)]
pub struct Panel {
    pub volume: Option<Volume>,
    pub brightness: Option<f32>,
    pub wifi: Option<Wifi>,
    pub bluetooth: Option<Bluetooth>,
    pub keyboard: Option<KeyboardLayouts>,
    pub track: Option<Track>,
    /// The track's cover art, loaded.
    pub art: Option<image::Handle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Module {
    Wifi,
    Bluetooth,
    NowPlaying,
    Keyboard,
    Screenshot,
    Lock,
    Display,
    Sound,
}

/// Where each module goes, in surface-local logical pixels, with its corner radius.
fn modules(panel: &Panel) -> Vec<(Module, Rectangle, f32)> {
    let rect = |x: f32, y, width, height| Rectangle {
        x: BLUR_LEFT + x,
        y,
        width,
        height,
    };
    let capsule = CELL / 2.0;
    let row = |n: f32| n * (CELL + GAP);
    let right = WIDE + GAP;

    let mut modules = vec![
        (Module::Wifi, rect(0.0, row(0.0), WIDE, CELL), capsule),
        (Module::Bluetooth, rect(0.0, row(1.0), WIDE, CELL), capsule),
        (
            Module::NowPlaying,
            rect(right, row(0.0), WIDE, WIDE),
            TILE_RADIUS,
        ),
        (Module::Keyboard, rect(0.0, row(2.0), WIDE, CELL), capsule),
        (
            Module::Screenshot,
            rect(right, row(2.0), CELL, CELL),
            capsule,
        ),
        (
            Module::Lock,
            rect(right + CELL + GAP, row(2.0), CELL, CELL),
            capsule,
        ),
    ];

    let mut y = row(3.0);
    if panel.brightness.is_some() {
        modules.push((
            Module::Display,
            rect(0.0, y, WIDTH, SLIDER_HEIGHT),
            SLIDER_RADIUS,
        ));
        y += SLIDER_HEIGHT + GAP;
    }
    modules.push((
        Module::Sound,
        rect(0.0, y, WIDTH, SLIDER_HEIGHT),
        SLIDER_RADIUS,
    ));
    modules
}

pub fn size(panel: &Panel) -> (u32, u32) {
    let height = modules(panel)
        .iter()
        .map(|(_, rect, _)| rect.y + rect.height)
        .fold(0.0, f32::max);
    (
        (BLUR_LEFT + WIDTH + MARGIN_RIGHT) as u32,
        (height + BLUR_BOTTOM) as u32,
    )
}

pub fn material_shapes(panel: &Panel) -> Vec<material::Shape> {
    let (width, height) = size(panel);
    let backdrop = material::Shape {
        rect: (0, 0, width as i32, height as i32),
        radius: 0,
        style: material::Style::Backdrop([
            BLUR_LEFT as u32,
            BLUR_TOP_FADE as u32,
            MARGIN_RIGHT as u32,
            BLUR_BOTTOM as u32,
        ]),
    };
    std::iter::once(backdrop)
        .chain(
            modules(panel)
                .iter()
                .map(|(_, rect, radius)| material::Shape {
                    rect: (
                        rect.x as i32,
                        rect.y as i32,
                        rect.width as i32,
                        rect.height as i32,
                    ),
                    radius: *radius as i32,
                    style: material::Style::Control,
                }),
        )
        .collect()
}

pub fn view(panel: &Panel, material: Option<material::Kind>) -> Element<'_> {
    let layers: Vec<Element<'_>> = modules(panel)
        .into_iter()
        .map(|(module, rect, radius)| {
            let content = match module {
                Module::Wifi => wifi(panel),
                Module::Bluetooth => bluetooth_toggle(panel),
                Module::NowPlaying => now_playing(panel),
                Module::Keyboard => keyboard(panel),
                Module::Screenshot => round_button(Glyph::Camera, Message::Screenshot),
                Module::Lock => round_button(Glyph::Lock, Message::LockScreen),
                Module::Display => display(panel.brightness.unwrap_or_default()),
                Module::Sound => sound(panel.volume),
            };
            let bubble = container(content)
                .width(rect.width)
                .height(rect.height)
                .style(move |_| bubble_style(material, radius));
            pin(bubble).x(rect.x).y(rect.y).into()
        })
        .collect();

    container(stack(layers))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            text_color: Some(Color::WHITE),
            ..Default::default()
        })
        .into()
}

/// A bubble's own fill: nothing over glass, a panel's fill otherwise.
fn bubble_style(material: Option<material::Kind>, radius: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(
            0.1,
            0.1,
            0.12,
            material::panel_tint(material),
        ))),
        border: material::panel_border(material, radius),
        ..Default::default()
    }
}

fn wifi(panel: &Panel) -> Element<'_> {
    let Some(wifi) = &panel.wifi else {
        return capsule(Glyph::WifiOff, "Wi-Fi", "Unavailable", false, None);
    };
    let status = match (&wifi.network, wifi.enabled) {
        (Some(network), true) => network.as_str(),
        (None, true) => "No Network",
        (_, false) => "Off",
    };
    let glyph = if wifi.enabled {
        Glyph::Wifi
    } else {
        Glyph::WifiOff
    };
    capsule(
        glyph,
        "Wi-Fi",
        status,
        wifi.network.is_some(),
        Some(Message::ToggleWifi),
    )
}

fn bluetooth_toggle(panel: &Panel) -> Element<'_> {
    let Some(bluetooth) = &panel.bluetooth else {
        return capsule(Glyph::BluetoothOff, "Bluetooth", "Unavailable", false, None);
    };
    let status = match (bluetooth.connected.first(), bluetooth.powered) {
        (Some(device), true) => device.as_str(),
        (None, true) => "On",
        (_, false) => "Off",
    };
    let glyph = if bluetooth.powered {
        Glyph::Bluetooth
    } else {
        Glyph::BluetoothOff
    };
    capsule(
        glyph,
        "Bluetooth",
        status,
        bluetooth.powered,
        Some(Message::ToggleBluetooth),
    )
}

/// The keyboard layout: pressing it moves to the next one.
fn keyboard(panel: &Panel) -> Element<'_> {
    let layouts = panel.keyboard.as_ref();
    let name = layouts
        .and_then(|layouts| layouts.names.get(layouts.current))
        .map_or("Unavailable", String::as_str);
    let on_press = layouts
        .filter(|layouts| layouts.names.len() > 1)
        .map(|_| Message::NextLayout);

    let circle = container(text(menubar::input_label(name)).size(13).font(BOLD))
        .center(TOGGLE_SIZE)
        .style(|_| toggle_style(false));
    capsule_row(circle.into(), "Keyboard", name, on_press)
}

/// A capsule toggle: a round icon, a name and a status.
fn capsule<'a>(
    glyph: Glyph,
    name: &'a str,
    status: &str,
    on: bool,
    on_press: Option<Message>,
) -> Element<'a> {
    let ink = if on { INK_ON_WHITE } else { Color::WHITE };
    let circle = container(icons::icon(glyph, 18.0, ink))
        .center(TOGGLE_SIZE)
        .style(move |_| toggle_style(on));
    capsule_row(circle.into(), name, status, on_press)
}

fn capsule_row<'a>(
    circle: Element<'a>,
    name: &'a str,
    status: &str,
    on_press: Option<Message>,
) -> Element<'a> {
    let content = container(
        row![
            circle,
            column![
                text(name).size(TITLE_SIZE).font(BOLD),
                text(menu::fit(
                    status,
                    DETAIL_SIZE,
                    Font::DEFAULT,
                    CAPSULE_TEXT_WIDTH
                ))
                .size(DETAIL_SIZE)
                .wrapping(Wrapping::None)
                .color(DIM),
            ],
        ]
        .spacing(TEXT_GAP)
        .align_y(Alignment::Center),
    )
    .padding(Padding {
        left: TOGGLE_INSET,
        right: CAPSULE_END_PADDING,
        ..Padding::ZERO
    })
    .height(Length::Fill)
    .width(Length::Fill)
    .align_y(alignment::Vertical::Center);

    match on_press {
        Some(message) => mouse_area(content).on_press(message).into(),
        None => content.into(),
    }
}

/// White when active, like Tahoe's toggles; a translucent grey otherwise.
fn toggle_style(on: bool) -> container::Style {
    container::Style {
        background: Some(Background::Color(if on { Color::WHITE } else { OFF_FILL })),
        border: Border {
            radius: (TOGGLE_SIZE / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn round_button<'a>(glyph: Glyph, on_press: Message) -> Element<'a> {
    mouse_area(
        container(icons::icon(glyph, 24.0, Color::WHITE))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press(on_press)
    .into()
}

/// Cover art, title and artist, and transport controls; dimmed and idle with nothing playing.
fn now_playing(panel: &Panel) -> Element<'_> {
    let art: Element<'_> = match &panel.art {
        Some(handle) if panel.track.is_some() => image(handle.clone())
            .width(ART_SIZE)
            .height(ART_SIZE)
            .border_radius(8.0)
            .into(),
        _ => container(Space::new())
            .width(ART_SIZE)
            .height(ART_SIZE)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1))),
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    };

    let (title, artist, play_glyph) = match &panel.track {
        Some(track) => (
            menu::fit(&track.title, TITLE_SIZE, BOLD, TILE_TEXT_WIDTH),
            menu::fit(
                track.artist.as_deref().unwrap_or_default(),
                DETAIL_SIZE,
                Font::DEFAULT,
                TILE_TEXT_WIDTH,
            ),
            if track.playing {
                Glyph::Pause
            } else {
                Glyph::Play
            },
        ),
        None => (String::from("Not Playing"), String::new(), Glyph::Play),
    };
    let active = panel.track.is_some();
    let control = |glyph, control, idle_ink| -> Element<'_> {
        let ink = if active { Color::WHITE } else { idle_ink };
        let icon = container(icons::icon(glyph, 20.0, ink)).center(30.0);
        if active {
            mouse_area(icon).on_press(Message::Media(control)).into()
        } else {
            icon.into()
        }
    };

    container(
        column![
            art,
            Space::new().height(Length::Fill),
            text(title).size(TITLE_SIZE).font(BOLD),
        ]
        .push(active.then(|| text(artist).size(DETAIL_SIZE).color(DIM)))
        .push(Space::new().height(4))
        .push(
            row![
                control(Glyph::Previous, Control::Previous, FAINT),
                Space::new().width(Length::Fill),
                control(play_glyph, Control::PlayPause, DIM),
                Space::new().width(Length::Fill),
                control(Glyph::Next, Control::Next, FAINT),
            ]
            .align_y(Alignment::Center),
        )
        .height(Length::Fill),
    )
    .padding(TILE_PADDING)
    .into()
}

fn display<'a>(level: f32) -> Element<'a> {
    slider_module(
        "Display",
        Glyph::SunLow,
        None,
        level,
        Message::BrightnessChanged,
        Message::BrightnessReleased,
        Glyph::SunHigh,
    )
}

fn sound<'a>(volume: Option<Volume>) -> Element<'a> {
    let Some(volume) = volume else {
        return container(text("No sound output").size(TITLE_SIZE).color(DIM))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };
    let level = if volume.muted { 0.0 } else { volume.level };
    let low = if volume.muted {
        Glyph::SpeakerMuted
    } else {
        Glyph::SpeakerLow
    };
    slider_module(
        "Sound",
        low,
        Some(Message::ToggleMute),
        level,
        Message::VolumeChanged,
        Message::VolumeReleased,
        Glyph::SpeakerHigh,
    )
}

/// A title, then a slider between two icons. Tahoe's sliders are a thin line with no knob.
fn slider_module<'a>(
    title: &'a str,
    low: Glyph,
    on_low_press: Option<Message>,
    level: f32,
    on_change: fn(f32) -> Message,
    on_release: Message,
    high: Glyph,
) -> Element<'a> {
    let line = slider(0.0..=1.0, level, on_change)
        .on_release(on_release)
        .step(0.01_f32)
        .height(18.0)
        .style(|_, _| slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(Color::WHITE),
                    Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.25)),
                ),
                width: 4.0,
                border: Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 0.0 },
                background: Background::Color(Color::TRANSPARENT),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        });

    let low_icon = icons::icon(low, 16.0, Color::WHITE);
    let low_icon: Element<'a> = match on_low_press {
        Some(message) => mouse_area(low_icon).on_press(message).into(),
        None => low_icon,
    };

    container(
        column![
            text(title).size(TITLE_SIZE).font(BOLD),
            row![low_icon, line, icons::icon(high, 18.0, Color::WHITE)]
                .spacing(8)
                .align_y(Alignment::Center),
        ]
        .spacing(5),
    )
    .padding([9.0, 16.0])
    .width(Length::Fill)
    .into()
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
