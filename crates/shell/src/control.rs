//! Control Center, like Tahoe's: no panel, just separate glass bubbles on a four-column grid.
//! Capsule toggles two cells wide, round buttons one cell wide, Now Playing two by two, and
//! full-width sliders. [`modules`] places them; the view and the glass shapes both follow it.

use std::time::Instant;

use iced_widget::core::text::Wrapping;

use iced_widget::core::{
    Alignment, Background, Border, Color, Length, Padding, Rectangle, Size, alignment,
};
use iced_widget::scrollable::{Direction, Scrollbar};
use iced_widget::{
    Space, button, column, container, image, mouse_area, pin, row, scrollable, slider, stack, text,
};

use crate::anim::{self, Motion, Spring};
use crate::app::{Element, Message};
use crate::audio::{self, Output, Volume};
use crate::bluetooth::{Bluetooth, Device};
use crate::group::group;
use crate::icons::{self, Glyph};
use crate::kwin::KeyboardLayouts;
use crate::media::{Control, Track};
use crate::network::{Network, Wifi};
use crate::{bluetooth, fonts, material, media, menu, menubar, network};

const CELL: f32 = 63.0;
const GAP: f32 = 12.0;
/// Two cells and the gap between them.
const WIDE: f32 = 2.0 * CELL + GAP;
const WIDTH: f32 = 2.0 * WIDE + GAP;
/// How a module's glass turns into its detail and back, measured from Tahoe's: 18%, 65%, 85% and 97% of
/// the way there at 33, 83, 133 and 183ms, and a few pixels past.
const MORPH_SPRING: Spring = Spring::new(0.25, 0.8);
/// How the other modules fade, on the way there and back.
const OTHERS_SPRING: Spring = Spring::new(0.22, 1.0);
/// What a module does under the pointer, measured from Tahoe's: about 3% brighter, a brighter rim, and
/// a chevron coming up if pressing it opens something.
const HOVER_SPRING: Spring = Spring::new(0.18, 1.0);
/// How the modules come in when Control Center opens: each from a little smaller and unseen,
/// springing up past its size and back, one after another going out from the corner it opens from.
/// Tahoe only fades them in, which is a lot less fun.
const POP_SPRING: Spring = Spring::new(0.36, 0.6);
const POP_FROM: f32 = 0.8;
/// How long each waits per point it is from that corner, and how long it takes to show.
const POP_STAGGER: f32 = 0.00028;
const POP_FADE: f32 = 0.09;
/// Long enough for the last of them to have settled.
const POP_TIME: f32 = 0.9;
const HOVER_FILL: f32 = 0.045;
const HOVER_RIM: f32 = 0.16;
const HOVER_CHEVRON: f32 = 11.0;
const HOVER_CHEVRON_INSET: f32 = 20.0;
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

const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.65);
const FAINT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.3);
const OFF_FILL: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.25);
const INK_ON_WHITE: Color = Color::from_rgb(0.1, 0.1, 0.12);
const OUTPUTS_SIZE: f32 = 26.0;
const DETAIL_HEADER: f32 = 42.0;
const DETAIL_FOOTER: f32 = 36.0;
const DETAIL_SEPARATOR: f32 = 1.0;
const DETAIL_ROW: f32 = 30.0;
/// Between a detail panel's edge and its rows' highlights.
const DETAIL_INSET: f32 = 5.0;
/// Between a row's highlight and what's in it.
const DETAIL_ROW_PADDING: f32 = 9.0;
const DETAIL_PADDING_X: f32 = DETAIL_INSET + DETAIL_ROW_PADDING;
const DETAIL_ICON: f32 = 22.0;
const SWITCH_WIDTH: f32 = 36.0;
const SWITCH_HEIGHT: f32 = 20.0;
const SWITCH_KNOB: f32 = 16.0;

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
    /// The module whose detail is showing, instead of the bubbles.
    pub detail: Option<Detail>,
    /// A module's glass on its way to its detail or back.
    pub morph: Option<Morph>,
    /// What the pointer's over, and how much each module is lit up by it, on its way in or out.
    hovered: Option<Module>,
    hover: Vec<(Module, Motion)>,
    pub networks: Vec<Network>,
    pub devices: Vec<Device>,
    pub outputs: Vec<Output>,
    /// When it last opened, which the modules come in from.
    pub opened: Option<Instant>,
}

/// A module's own glass turning into its detail, or back, measured from Tahoe's: it moves and grows
/// with a slight overshoot, what's in it crossfades, and the other modules fade where they are. It
/// can change its mind on the way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Morph {
    /// How far along it is, from 0 (the module) to 1 (the detail).
    value: Motion,
    /// How much of the other modules shows.
    others: Motion,
    opening: bool,
}

impl Morph {
    /// Starts toward the detail or the module, from wherever `panel` has got to.
    pub fn start(panel: &Panel, opening: bool) -> Self {
        let (mut value, mut others) = match panel.morph {
            Some(morph) => (morph.value, morph.others),
            None => {
                let at_detail = panel.detail.is_some();
                (
                    Motion::at(f32::from(u8::from(at_detail)), MORPH_SPRING, 0.002),
                    Motion::at(f32::from(u8::from(!at_detail)), OTHERS_SPRING, 0.002),
                )
            }
        };
        value.go_to(f32::from(u8::from(opening)));
        others.go_to(f32::from(u8::from(!opening)));
        Self {
            value,
            others,
            opening,
        }
    }

    pub fn is_done(self) -> bool {
        !self.value.is_moving() && !self.others.is_moving()
    }

    pub fn is_opening(self) -> bool {
        self.opening
    }

    /// How far along it is, from 0 to 1, going a little past on the way.
    fn value(self) -> f32 {
        self.value.value()
    }

    /// How much of the other modules shows.
    fn others(self) -> f32 {
        self.others.value().clamp(0.0, 1.0)
    }
}

impl Panel {
    /// The pointer came onto a module, or left it.
    pub fn set_hovered(&mut self, module: Module, over: bool) {
        let hovered = if over {
            Some(module)
        } else if self.hovered == Some(module) {
            None
        } else {
            return;
        };
        if hovered == self.hovered {
            return;
        }
        self.hovered = hovered;
        for (lit, motion) in &mut self.hover {
            motion.go_to(if hovered == Some(*lit) { 1.0 } else { 0.0 });
        }
        if let Some(module) = hovered
            && !self.hover.iter().any(|(lit, _)| *lit == module)
        {
            let mut motion = Motion::at(0.0, HOVER_SPRING, 0.01);
            motion.go_to(1.0);
            self.hover.push((module, motion));
        }
    }

    /// Forgets what the pointer was over, which it isn't when this opens again.
    pub fn clear_hover(&mut self) {
        self.hovered = None;
        self.hover.clear();
    }

    /// How much a module is lit up by the pointer.
    fn lit(&self, module: Module) -> f32 {
        self.hover
            .iter()
            .find(|(lit, _)| *lit == module)
            .map_or(0.0, |(_, motion)| motion.value().clamp(0.0, 1.0))
    }

    pub fn hover_is_moving(&self) -> bool {
        self.hover.iter().any(|(_, motion)| motion.is_moving())
    }
}

/// What a module opens to: a panel with what it's connected to, in place of the bubbles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Wifi,
    Bluetooth,
    Keyboard,
    Sound,
}

impl Detail {
    fn title(self) -> &'static str {
        match self {
            Detail::Wifi => "Wi-Fi",
            Detail::Bluetooth => "Bluetooth",
            Detail::Keyboard => "Keyboard",
            Detail::Sound => "Sound Output",
        }
    }

    /// The row that leads to the system's settings for it.
    fn settings(self) -> (&'static str, &'static str) {
        match self {
            Detail::Wifi => ("Wi-Fi Settings…", "kcm_networkmanagement"),
            Detail::Bluetooth => ("Bluetooth Settings…", "kcm_bluetooth"),
            Detail::Keyboard => ("Keyboard Settings…", "kcm_keyboard"),
            Detail::Sound => ("Sound Settings…", "kcm_pulseaudio"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
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

/// How much room the bubbles take, top to bottom.
fn bubbles_height(panel: &Panel) -> f32 {
    modules(panel)
        .iter()
        .map(|(_, rect, _)| rect.y + rect.height)
        .fold(0.0, f32::max)
}

/// Which module a detail opens out of.
fn module_of(detail: Detail) -> Module {
    match detail {
        Detail::Wifi => Module::Wifi,
        Detail::Bluetooth => Module::Bluetooth,
        Detail::Keyboard => Module::Keyboard,
        Detail::Sound => Module::Sound,
    }
}

/// Where a detail goes, with its corner radius.
fn detail_rect(panel: &Panel) -> (Rectangle, f32) {
    (
        Rectangle {
            x: BLUR_LEFT,
            y: 0.0,
            width: WIDTH,
            height: detail_height(panel),
        },
        menu::RADIUS,
    )
}

/// Where the morphing glass is now, with its corner radius.
fn morph_rect(panel: &Panel, detail: Detail, morph: Morph) -> (Rectangle, f32) {
    let (module, module_radius) = modules(panel)
        .into_iter()
        .find(|(module, ..)| *module == module_of(detail))
        .map_or((Rectangle::default(), 0.0), |(_, rect, radius)| {
            (rect, radius)
        });
    let (end, end_radius) = detail_rect(panel);
    let v = morph.value();
    let along = |from: f32, to: f32| from + (to - from) * v;
    (
        Rectangle {
            x: along(module.x, end.x),
            y: along(module.y, end.y),
            width: along(module.width, end.width).max(1.0),
            height: along(module.height, end.height).max(1.0),
        },
        along(module_radius, end_radius).max(0.0),
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
    let module_shape = |rect: &Rectangle, radius: f32| material::Shape {
        rect: (
            rect.x as i32,
            rect.y as i32,
            rect.width as i32,
            rect.height as i32,
        ),
        radius: radius as i32,
        style: material::Style::Control,
    };
    let Some(detail) = panel.detail else {
        return std::iter::once(backdrop)
            .chain(modules(panel).iter().map(|(_, rect, radius)| {
                let (scale, _) = pop(panel, rect);
                module_shape(&scaled(rect, scale), *radius * scale)
            }))
            .collect();
    };
    let Some(morph) = panel.morph else {
        let (rect, radius) = detail_rect(panel);
        return vec![backdrop, module_shape(&rect, radius)];
    };
    // The moving glass is in one big rectangle that stays put, so its textures last.
    let (rect, radius) = morph_rect(panel, detail, morph);
    let milli = |value: f32| (value * 1000.0).round() as i32;
    let source = module_of(detail);
    std::iter::once(backdrop)
        .chain(
            modules(panel)
                .iter()
                .filter(|(module, ..)| *module != source)
                .map(|(_, rect, radius)| module_shape(rect, *radius)),
        )
        .chain(std::iter::once(material::Shape {
            rect: (0, 0, width as i32, height as i32),
            radius: 0,
            style: material::Style::ControlBox([
                milli(rect.x + rect.width / 2.0),
                milli(rect.y + rect.height / 2.0),
                milli(rect.width),
                milli(rect.height),
                milli(radius),
            ]),
        }))
        .collect()
}

/// How opaque each of [`material_shapes`] is: the other modules fade while one morphs.
pub fn material_opacities(panel: &Panel) -> Vec<f32> {
    let (Some(detail), Some(morph)) = (panel.detail, panel.morph) else {
        if panel.detail.is_none() && is_popping(panel) {
            return std::iter::once(1.0)
                .chain(modules(panel).iter().map(|(_, rect, _)| pop(panel, rect).1))
                .collect();
        }
        return Vec::new();
    };
    let source = module_of(detail);
    let others = modules(panel)
        .iter()
        .filter(|(module, ..)| *module != source)
        .count();
    let mut opacities = vec![1.0];
    opacities.extend(std::iter::repeat_n(morph.others(), others));
    opacities.push(1.0);
    opacities
}

/// How big a module is, and how much of it shows, on its way in as Control Center opens.
fn pop(panel: &Panel, rect: &Rectangle) -> (f32, f32) {
    let Some(opened) = panel.opened else {
        return (1.0, 1.0);
    };
    let (width, _) = size(panel);
    let corner = (width as f32 - MARGIN_RIGHT, 0.0);
    let center = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let distance = (center.0 - corner.0).hypot(center.1 - corner.1);
    let seconds = anim::seconds_since(opened) - distance * POP_STAGGER;
    if seconds <= 0.0 {
        return (POP_FROM, 0.0);
    }
    let scale = POP_FROM + (1.0 - POP_FROM) * POP_SPRING.progress(seconds);
    (scale, (seconds / POP_FADE).min(1.0))
}

/// Whether the modules are still coming in.
pub fn is_popping(panel: &Panel) -> bool {
    panel
        .opened
        .is_some_and(|opened| anim::seconds_since(opened) < POP_TIME)
}

fn scaled(rect: &Rectangle, scale: f32) -> Rectangle {
    let (width, height) = (rect.width * scale, rect.height * scale);
    Rectangle {
        x: rect.x + (rect.width - width) / 2.0,
        y: rect.y + (rect.height - height) / 2.0,
        width,
        height,
    }
}

/// What's in a module.
fn module_content(module: Module, panel: &Panel) -> Element<'_> {
    match module {
        Module::Wifi => wifi(panel),
        Module::Bluetooth => bluetooth_toggle(panel),
        Module::NowPlaying => now_playing(panel),
        Module::Keyboard => keyboard(panel),
        Module::Screenshot => round_button(Glyph::Camera, Message::Screenshot),
        Module::Lock => round_button(Glyph::Lock, Message::LockScreen),
        Module::Display => display(panel.brightness.unwrap_or_default()),
        Module::Sound => sound(panel.volume),
    }
}

pub fn view(panel: &Panel, material: Option<material::Kind>) -> Element<'_> {
    let bubble = |module: Module, rect: Rectangle, radius: f32| {
        let lit = panel.lit(module);
        let opens_something = matches!(module, Module::Wifi | Module::Bluetooth | Module::Keyboard);
        let glow = container(Space::new())
            .width(rect.width)
            .height(rect.height)
            .style(move |_| container::Style {
                background: Some(Background::Color(Color::from_rgba(
                    1.0, 1.0, 1.0, HOVER_FILL,
                ))),
                border: Border {
                    color: Color::from_rgba(1.0, 1.0, 1.0, HOVER_RIM),
                    width: 1.0,
                    radius: radius.into(),
                },
                ..Default::default()
            });
        let chevron = icons::icon(
            Glyph::Chevron,
            HOVER_CHEVRON,
            Color::from_rgba(1.0, 1.0, 1.0, 0.75),
        );
        container(
            stack![
                group(glow).opacity(lit),
                module_content(module, panel),
                pin(group(chevron).opacity(if opens_something { lit } else { 0.0 }))
                    .x(rect.width - HOVER_CHEVRON_INSET)
                    .y((rect.height - HOVER_CHEVRON) / 2.0),
            ]
            .width(rect.width)
            .height(rect.height),
        )
        .width(rect.width)
        .height(rect.height)
        .style(move |_| bubble_style(material, radius))
    };
    let layers: Vec<Element<'_>> = match (panel.detail, panel.morph) {
        (Some(detail), None) => {
            // A press beside it goes back to the modules; the detail keeps its own presses.
            return mouse_area(
                container(pin(detail_view(detail, panel, material)).x(BLUR_LEFT))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ControlDetail(None))
            .into();
        }
        (Some(detail), Some(morph)) => {
            let source = module_of(detail);
            let others = morph.others();
            let (rect, _) = morph_rect(panel, detail, morph);
            // What's in the module goes first, and the detail comes in over it.
            let v = morph.value();
            let module_shows = (1.0 - v / 0.45).clamp(0.0, 1.0);
            let detail_shows = ((v - 0.3) / 0.55).clamp(0.0, 1.0);
            let module = modules(panel)
                .into_iter()
                .find(|(module, ..)| *module == source);
            let mut layers: Vec<Element<'_>> = modules(panel)
                .into_iter()
                .filter(|(module, ..)| *module != source)
                .map(|(module, rect, radius)| {
                    pin(group(bubble(module, rect, radius)).opacity(others))
                        .x(rect.x)
                        .y(rect.y)
                        .into()
                })
                .collect();
            // The detail first, since a stack takes its size from its first layer.
            let mut inside: Vec<Element<'_>> = vec![
                group(detail_view(detail, panel, material))
                    .opacity(detail_shows)
                    .into(),
            ];
            if let Some((module, module_rect, radius)) = module {
                inside.push(
                    group(bubble(module, module_rect, radius))
                        .opacity(module_shows)
                        .into(),
                );
            }
            layers.push(
                pin(group(stack(inside)).clip(Size::new(rect.width, rect.height)))
                    .x(rect.x)
                    .y(rect.y)
                    .into(),
            );
            layers
        }
        (None, _) => modules(panel)
            .into_iter()
            .map(|(module, rect, radius)| {
                let (scale, shows) = pop(panel, &rect);
                pin(mouse_area(
                    group(bubble(module, rect, radius))
                        .scale(scale)
                        .opacity(shows),
                )
                .on_enter(Message::ControlHover(module, true))
                .on_exit(Message::ControlHover(module, false)))
                .x(rect.x)
                .y(rect.y)
                .into()
            })
            .collect(),
    };

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
        return capsule(Glyph::WifiOff, "Wi-Fi", "Unavailable", false, None, None);
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
        Some(Detail::Wifi),
    )
}

fn bluetooth_toggle(panel: &Panel) -> Element<'_> {
    let Some(bluetooth) = &panel.bluetooth else {
        return capsule(
            Glyph::BluetoothOff,
            "Bluetooth",
            "Unavailable",
            false,
            None,
            None,
        );
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
        Some(Detail::Bluetooth),
    )
}

/// The keyboard layout: its circle moves to the next one, and the rest lists them.
fn keyboard(panel: &Panel) -> Element<'_> {
    let layouts = panel.keyboard.as_ref();
    let name = layouts
        .and_then(|layouts| layouts.names.get(layouts.current))
        .map_or("Unavailable", String::as_str);
    let several = layouts.is_some_and(|layouts| layouts.names.len() > 1);

    let circle = container(text(menubar::input_label(name)).size(13).font(fonts::BOLD))
        .center(TOGGLE_SIZE)
        .style(|_| toggle_style(false));
    capsule_row(
        pressable(circle.into(), several.then_some(Message::NextLayout)),
        "Keyboard",
        name,
        several.then_some(Detail::Keyboard),
    )
}

/// A capsule toggle: a round icon that toggles it, and a name and a status that open its detail.
fn capsule<'a>(
    glyph: Glyph,
    name: &'a str,
    status: &str,
    on: bool,
    on_toggle: Option<Message>,
    detail: Option<Detail>,
) -> Element<'a> {
    let ink = if on { INK_ON_WHITE } else { Color::WHITE };
    let circle = container(icons::icon(glyph, 18.0, ink))
        .center(TOGGLE_SIZE)
        .style(move |_| toggle_style(on));
    capsule_row(pressable(circle.into(), on_toggle), name, status, detail)
}

fn pressable(content: Element<'_>, on_press: Option<Message>) -> Element<'_> {
    match on_press {
        Some(message) => mouse_area(content).on_press(message).into(),
        None => content,
    }
}

fn capsule_row<'a>(
    circle: Element<'a>,
    name: &'a str,
    status: &str,
    detail: Option<Detail>,
) -> Element<'a> {
    let content = container(
        row![
            circle,
            column![
                text(name).size(TITLE_SIZE).font(fonts::BOLD),
                text(menu::fit(
                    status,
                    DETAIL_SIZE,
                    fonts::REGULAR,
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

    pressable(
        content.into(),
        detail.map(|detail| Message::ControlDetail(Some(detail))),
    )
}

/// White when active, like Tahoe's toggles; a translucent grey otherwise.
fn toggle_style(on: bool) -> container::Style {
    toggle_style_of(TOGGLE_SIZE, on)
}

fn toggle_style_of(size: f32, on: bool) -> container::Style {
    container::Style {
        background: Some(Background::Color(if on { Color::WHITE } else { OFF_FILL })),
        border: Border {
            radius: (size / 2.0).into(),
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
            menu::fit(&track.title, TITLE_SIZE, fonts::BOLD, TILE_TEXT_WIDTH),
            menu::fit(
                track.artist.as_deref().unwrap_or_default(),
                DETAIL_SIZE,
                fonts::REGULAR,
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
            text(title).size(TITLE_SIZE).font(fonts::BOLD),
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
        None,
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
        Some(Message::ControlDetail(Some(Detail::Sound))),
    )
}

#[allow(clippy::too_many_arguments)]
/// A title, then a slider between two icons. Tahoe's sliders are a thin line with no knob.
fn slider_module<'a>(
    title: &'a str,
    low: Glyph,
    on_low_press: Option<Message>,
    level: f32,
    on_change: fn(f32) -> Message,
    on_release: Message,
    high: Glyph,
    on_outputs: Option<Message>,
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
            text(title).size(TITLE_SIZE).font(fonts::BOLD),
            row![low_icon, line, icons::icon(high, 18.0, Color::WHITE)]
                .push(on_outputs.map(|message| {
                    mouse_area(
                        container(icons::icon(Glyph::Outputs, 15.0, Color::WHITE))
                            .center(OUTPUTS_SIZE)
                            .style(|_| toggle_style_of(OUTPUTS_SIZE, false)),
                    )
                    .on_press(message)
                }))
                .spacing(8)
                .align_y(Alignment::Center),
        ]
        .spacing(5),
    )
    .padding([9.0, 16.0])
    .width(Length::Fill)
    .into()
}

/// How many rows a detail panel has: what it lists, or a line saying there's nothing.
fn detail_rows(panel: &Panel, detail: Detail) -> usize {
    let listed = match detail {
        Detail::Wifi => {
            let known = panel
                .networks
                .iter()
                .filter(|network| network.known)
                .count();
            let others = panel.networks.len() - known;
            // A heading above each group.
            panel.networks.len() + usize::from(known > 0) + usize::from(others > 0)
        }
        Detail::Bluetooth => panel.devices.len(),
        Detail::Keyboard => panel.keyboard.as_ref().map_or(0, |list| list.names.len()),
        Detail::Sound => panel.outputs.len(),
    };
    listed.max(1)
}

fn detail_height(panel: &Panel) -> f32 {
    let rows = panel.detail.map_or(1, |detail| detail_rows(panel, detail));
    let full = DETAIL_HEADER + 2.0 * DETAIL_SEPARATOR + rows as f32 * DETAIL_ROW + DETAIL_FOOTER;
    full.min(bubbles_height(panel))
}

fn detail_view<'a>(
    detail: Detail,
    panel: &'a Panel,
    material: Option<material::Kind>,
) -> Element<'a> {
    let (settings_label, settings_page) = detail.settings();
    let header = container(
        row![
            text(detail.title()).size(TITLE_SIZE).font(fonts::BOLD),
            Space::new().width(Length::Fill),
        ]
        .push(switch_for(detail, panel))
        .align_y(Alignment::Center),
    )
    .height(DETAIL_HEADER)
    .padding([0.0, DETAIL_PADDING_X])
    .align_y(alignment::Vertical::Center);

    let list = scrollable(column(detail_list(detail, panel)).padding([0.0, DETAIL_INSET]))
        .direction(Direction::Vertical(
            Scrollbar::new().width(0).scroller_width(0),
        ))
        .height(Length::Fill)
        .width(Length::Fill);

    let settings = detail_button(
        container(text(settings_label).size(TITLE_SIZE))
            .padding([0.0, DETAIL_ROW_PADDING])
            .height(Length::Fill)
            .align_y(alignment::Vertical::Center)
            .into(),
        Some(Message::ControlSettings(settings_page)),
    );

    // Pressing the panel itself goes back, as on macOS; what's on it keeps its own presses.
    mouse_area(
        container(column![
            header,
            detail_separator(),
            list,
            detail_separator(),
            container(settings)
                .height(DETAIL_FOOTER)
                .padding([4.0, DETAIL_INSET]),
        ])
        .width(WIDTH)
        .height(detail_height(panel))
        .style(move |_| {
            let mut style = container::Style {
                background: Some(Background::Color(Color::from_rgba(
                    0.1,
                    0.1,
                    0.12,
                    material::panel_tint(material),
                ))),
                border: material::panel_border(material, menu::RADIUS),
                ..Default::default()
            };
            style.text_color = Some(Color::WHITE);
            style
        }),
    )
    .on_press(Message::ControlDetail(None))
    .into()
}

/// The switch at the top of a detail panel, for what can be turned off.
fn switch_for(detail: Detail, panel: &Panel) -> Option<Element<'_>> {
    let (on, message) = match detail {
        Detail::Wifi => (panel.wifi.as_ref()?.enabled, Message::ToggleWifi),
        Detail::Bluetooth => (panel.bluetooth.as_ref()?.powered, Message::ToggleBluetooth),
        Detail::Keyboard | Detail::Sound => return None,
    };
    let knob = container(Space::new())
        .width(SWITCH_KNOB)
        .height(SWITCH_KNOB)
        .style(|_| toggle_style_of(SWITCH_KNOB, true));
    let spacer = Space::new().width(Length::Fill);
    let content: Element<'_> = if on {
        row![spacer, knob].into()
    } else {
        row![knob, spacer].into()
    };
    let track = container(content)
        .width(SWITCH_WIDTH)
        .height(SWITCH_HEIGHT)
        .padding((SWITCH_HEIGHT - SWITCH_KNOB) / 2.0)
        .style(move |_| container::Style {
            background: Some(Background::Color(if on {
                crate::appearance::accent()
            } else {
                Color::from_rgba(1.0, 1.0, 1.0, 0.15)
            })),
            border: Border {
                radius: (SWITCH_HEIGHT / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    Some(mouse_area(track).on_press(message).into())
}

fn detail_list(detail: Detail, panel: &Panel) -> Vec<Element<'_>> {
    let mut rows: Vec<Element<'_>> = Vec::new();
    match detail {
        Detail::Wifi => {
            let mut heading = None;
            for network in &panel.networks {
                let wanted = if network.known {
                    "Known Networks"
                } else {
                    "Other Networks"
                };
                if heading != Some(wanted) {
                    heading = Some(wanted);
                    rows.push(detail_heading(wanted));
                }
                let glyph = match network.strength {
                    0..=33 => Glyph::WifiWeak,
                    34..=66 => Glyph::WifiMedium,
                    _ => Glyph::Wifi,
                };
                let lock = network
                    .secured
                    .then(|| icons::icon(Glyph::LockSmall, 13.0, DIM));
                rows.push(detail_row(
                    Some(glyph),
                    &network.ssid,
                    network.connected,
                    lock,
                    (!network.connected).then(|| Message::WifiConnect(network.ssid.clone())),
                ));
            }
            if rows.is_empty() {
                let enabled = panel.wifi.as_ref().is_some_and(|wifi| wifi.enabled);
                rows.push(detail_note(if enabled {
                    "No networks found"
                } else {
                    "Wi-Fi is off"
                }));
            }
        }
        Detail::Bluetooth => {
            for device in &panel.devices {
                let status = device
                    .connected
                    .then(|| text("Connected").size(DETAIL_SIZE).color(DIM).into());
                rows.push(detail_row(
                    Some(Glyph::Bluetooth),
                    &device.name,
                    device.connected,
                    status,
                    Some(Message::BluetoothConnect(
                        device.path.clone(),
                        !device.connected,
                    )),
                ));
            }
            if rows.is_empty() {
                rows.push(detail_note("No paired devices"));
            }
        }
        Detail::Keyboard => {
            if let Some(layouts) = &panel.keyboard {
                for (index, name) in layouts.names.iter().enumerate() {
                    rows.push(detail_row(
                        None,
                        name,
                        index == layouts.current,
                        None,
                        Some(Message::ControlLayout(index)),
                    ));
                }
            }
        }
        Detail::Sound => {
            for (index, output) in panel.outputs.iter().enumerate() {
                rows.push(detail_row(
                    Some(Glyph::SpeakerHigh),
                    &output.name,
                    output.current,
                    None,
                    Some(Message::ControlOutput(index)),
                ));
            }
            if rows.is_empty() {
                rows.push(detail_note("No sound outputs"));
            }
        }
    }
    rows
}

fn detail_heading(heading: &str) -> Element<'_> {
    container(text(heading).size(DETAIL_SIZE).font(fonts::BOLD).color(DIM))
        .height(DETAIL_ROW)
        .padding([0.0, DETAIL_ROW_PADDING])
        .align_y(alignment::Vertical::Center)
        .into()
}

fn detail_note(note: &str) -> Element<'_> {
    container(text(note).size(TITLE_SIZE).color(DIM))
        .height(DETAIL_ROW)
        .padding([0.0, DETAIL_ROW_PADDING])
        .align_y(alignment::Vertical::Center)
        .into()
}

/// A row with a round icon, or a check mark without one; `active` lights it.
fn detail_row<'a>(
    glyph: Option<Glyph>,
    name: &str,
    active: bool,
    trailing: Option<Element<'a>>,
    on_press: Option<Message>,
) -> Element<'a> {
    let lead: Element<'a> = match glyph {
        Some(glyph) => {
            let ink = if active { INK_ON_WHITE } else { Color::WHITE };
            container(icons::icon(glyph, 13.0, ink))
                .center(DETAIL_ICON)
                .style(move |_| toggle_style_of(DETAIL_ICON, active))
                .into()
        }
        None => text(if active { "✓" } else { "" })
            .size(TITLE_SIZE)
            .width(DETAIL_ICON)
            .into(),
    };
    let room = WIDTH
        - 2.0 * DETAIL_INSET
        - 2.0 * DETAIL_ROW_PADDING
        - DETAIL_ICON
        - 8.0
        - if trailing.is_some() { 74.0 } else { 0.0 };
    let label = menu::fit(name, TITLE_SIZE, fonts::REGULAR, room);
    detail_button(
        container(
            row![
                lead,
                text(label).size(TITLE_SIZE).wrapping(Wrapping::None),
                Space::new().width(Length::Fill),
            ]
            .push(trailing)
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([0.0, DETAIL_ROW_PADDING])
        .height(DETAIL_ROW)
        .align_y(alignment::Vertical::Center)
        .into(),
        on_press,
    )
}

/// A row that lights up under the pointer, like a menu's. Without a message, it doesn't.
fn detail_button(content: Element<'_>, on_press: Option<Message>) -> Element<'_> {
    button(content)
        .on_press_maybe(on_press)
        .width(Length::Fill)
        .padding(0)
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.12))),
            text_color: Color::WHITE,
            border: Border {
                radius: (menu::RADIUS - DETAIL_INSET).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn detail_separator<'a>() -> Element<'a> {
    container(
        container(Space::new())
            .width(Length::Fill)
            .height(1)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.15))),
                ..Default::default()
            }),
    )
    .height(DETAIL_SEPARATOR)
    .padding([0.0, DETAIL_PADDING_X])
    .align_y(alignment::Vertical::Center)
    .into()
}

/// The panel's slower state, read off the UI thread.
#[derive(Debug, Clone)]
pub struct Status {
    pub wifi: Option<Wifi>,
    pub bluetooth: Option<Bluetooth>,
    pub track: Option<Track>,
    pub networks: Vec<Network>,
    pub devices: Vec<Device>,
    pub outputs: Vec<Output>,
}

/// Reads what's shown, including the lists of the detail that's open.
pub async fn status() -> Status {
    let wifi = network::wifi().await;
    let wifi_on = wifi.as_ref().is_some_and(|wifi| wifi.enabled);
    // All of it, even with no detail open: a module opens into a panel that's already its full size.
    Status {
        wifi,
        bluetooth: bluetooth::bluetooth().await,
        track: media::now_playing().await,
        networks: if wifi_on {
            network::networks().await
        } else {
            Vec::new()
        },
        devices: bluetooth::devices().await,
        outputs: audio::outputs(),
    }
}

#[cfg(test)]
mod morph_tests {
    use super::*;
    use std::time::Duration;

    fn morph(opening: bool, seconds: f32) -> Morph {
        let panel = Panel {
            detail: (!opening).then_some(Detail::Wifi),
            ..Default::default()
        };
        let mut morph = Morph::start(&panel, opening);
        morph.value = morph.value.started_ago(seconds);
        morph.others = morph.others.started_ago(seconds);
        morph
    }

    #[test]
    fn it_goes_from_end_to_end_and_a_little_past() {
        assert!(morph(true, 0.0).value().abs() < 0.05);
        assert_eq!(morph(true, 2.0).value(), 1.0);
        assert_eq!(morph(false, 2.0).value(), 0.0);
        let peak = (0..60)
            .map(|step| morph(true, step as f32 / 100.0).value())
            .fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.06, "went to {peak}");
        assert!(morph(true, 2.0).is_done());
        assert!(!morph(true, 0.05).is_done());
    }

    #[test]
    fn changing_its_mind_picks_up_from_where_it_was() {
        let there = morph(true, 0.08);
        let halfway = there.value();
        let panel = Panel {
            detail: Some(Detail::Wifi),
            morph: Some(there),
            ..Default::default()
        };
        assert!((Morph::start(&panel, false).value() - halfway).abs() < 0.02);
    }

    #[test]
    fn the_other_modules_fade_out_and_come_back() {
        assert!(morph(true, 0.0).others() > 0.99);
        assert_eq!(morph(true, 2.0).others(), 0.0);
        assert!(morph(false, 0.0).others() < 0.01);
        assert_eq!(morph(false, 2.0).others(), 1.0);
    }

    #[test]
    fn the_pointer_lights_one_module_at_a_time() {
        let mut panel = Panel::default();
        panel.set_hovered(Module::Wifi, true);
        panel.set_hovered(Module::Bluetooth, true);
        // Leaving one it's no longer over changes nothing.
        panel.set_hovered(Module::Wifi, false);
        assert_eq!(panel.hovered, Some(Module::Bluetooth));
        panel.set_hovered(Module::Bluetooth, false);
        assert_eq!(panel.hovered, None);
    }

    #[test]
    fn every_shape_has_an_opacity_while_it_morphs() {
        let mut panel = Panel {
            detail: Some(Detail::Wifi),
            ..Default::default()
        };
        panel.morph = Some(morph(true, 0.1));
        assert_eq!(
            material_shapes(&panel).len(),
            material_opacities(&panel).len()
        );
        panel.morph = None;
        assert!(material_opacities(&panel).is_empty());
    }

    #[test]
    fn a_morph_starts_from_where_things_are() {
        let mut panel = Panel::default();
        assert!(Morph::start(&panel, true).value() < 0.01);
        panel.detail = Some(Detail::Sound);
        assert!(Morph::start(&panel, false).value() > 0.99);
    }
}
