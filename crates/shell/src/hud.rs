//! The little capsule that shows the volume or the brightness when a key changes it, under the right
//! end of the bar, looking like Control Center's own.

use std::time::{Duration, Instant};

use iced_layershell::reexport::{
    Anchor, IcedId, KeyboardInteractivity, Layer, NewLayerShellSettings,
};
use iced_widget::core::{Alignment, Background, Border, Color, Length};
use iced_widget::{Space, column, container, row, stack, text};

use crate::anim::{Motion, Spring};
use crate::app::Element;
use crate::icons::{self, Glyph};
use crate::{fonts, material, menubar};

const WIDTH: f32 = 288.0;
const HEIGHT: f32 = 64.0;
const RADIUS: f32 = 30.0;
const GAP_UNDER_BAR: i32 = 8;
const MARGIN_RIGHT: i32 = 8;
const TITLE_SIZE: f32 = 13.0;
const BAR_HEIGHT: f32 = 4.0;
const ICON_SIZE: f32 = 17.0;
const BAR_WIDTH: f32 = WIDTH - 2.0 * 16.0 - 2.0 * (ICON_SIZE + 8.0);
/// How long it stays after the last change.
const SHOWN_FOR: Duration = Duration::from_millis(1500);
const BAR_SPRING: Spring = Spring::new(0.25, 0.85);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sound,
    Display,
}

pub struct Hud {
    pub id: IcedId,
    pub material: Option<material::MaterialSurface>,
    kind: Kind,
    level: f32,
    muted: bool,
    /// What the bar's showing on its way to `level`.
    shown: Motion,
    changed: Instant,
}

impl Hud {
    pub fn new(id: IcedId, kind: Kind, level: f32, muted: bool) -> Self {
        Self {
            id,
            material: None,
            kind,
            level,
            muted,
            shown: Motion::at(if muted { 0.0 } else { level }, BAR_SPRING, 0.002),
            changed: Instant::now(),
        }
    }

    /// Shows a new level, and stays for longer.
    pub fn update(&mut self, kind: Kind, level: f32, muted: bool) {
        let target = if muted { 0.0 } else { level };
        if kind == self.kind {
            self.shown.go_to(target);
        } else {
            self.shown.jump_to(target);
        }
        self.kind = kind;
        self.level = level;
        self.muted = muted;
        self.changed = Instant::now();
    }

    /// Whether the bar's on its way somewhere.
    pub fn is_animating(&self) -> bool {
        self.shown.is_moving()
    }

    /// Returns whether it's time for it to go.
    pub fn frame(&self) -> bool {
        self.changed.elapsed() >= SHOWN_FOR
    }

    pub fn view(&self) -> Element<'_> {
        let (title, low, high) = match (self.kind, self.muted) {
            (Kind::Sound, true) => ("Sound", Glyph::SpeakerMuted, Glyph::SpeakerHigh),
            (Kind::Sound, false) => ("Sound", Glyph::SpeakerLow, Glyph::SpeakerHigh),
            (Kind::Display, _) => ("Display", Glyph::SunLow, Glyph::SunHigh),
        };
        let line = |width: f32, color: Color| {
            container(Space::new())
                .width(width)
                .height(BAR_HEIGHT)
                .style(move |_| container::Style {
                    background: Some(Background::Color(color)),
                    border: Border {
                        radius: (BAR_HEIGHT / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        let bar = stack![
            line(BAR_WIDTH, Color::from_rgba(1.0, 1.0, 1.0, 0.25)),
            line(
                (self.shown.value() * BAR_WIDTH).clamp(0.0, BAR_WIDTH),
                Color::WHITE
            ),
        ];
        container(
            column![
                text(title).size(TITLE_SIZE).font(fonts::BOLD),
                row![
                    icons::icon(low, ICON_SIZE, Color::WHITE),
                    bar,
                    icons::icon(high, ICON_SIZE, Color::WHITE),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(7),
        )
        .padding([10.0, 16.0])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            text_color: Some(Color::WHITE),
            ..Default::default()
        })
        .into()
    }
}

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Right,
        layer: Layer::Overlay,
        size: Some((WIDTH as u32, HEIGHT as u32)),
        exclusive_zone: Some(-1),
        margin: Some((menubar::HEIGHT as i32 + GAP_UNDER_BAR, MARGIN_RIGHT, 0, 0)),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("dock")),
        ..Default::default()
    }
}

pub fn material_shapes() -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, WIDTH as i32, HEIGHT as i32),
        radius: RADIUS as i32,
        style: material::Style::Card,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hud(level: f32, muted: bool) -> Hud {
        Hud::new(IcedId::unique(), Kind::Sound, level, muted)
    }

    #[test]
    fn the_bar_follows_the_level_on_a_spring() {
        let mut hud = hud(0.2, false);
        assert_eq!(hud.shown.value(), 0.2);
        hud.update(Kind::Sound, 0.8, false);
        assert!(hud.is_animating());
        hud.shown = hud.shown.started_ago(0.05);
        assert!(hud.shown.value() > 0.2 && hud.shown.value() < 0.8);
        hud.shown = hud.shown.started_ago(2.0);
        assert_eq!(hud.shown.value(), 0.8);
        assert!(!hud.is_animating());
    }

    #[test]
    fn muted_goes_to_nothing_and_the_other_kind_is_there_at_once() {
        let mut hud = hud(0.8, false);
        hud.update(Kind::Sound, 0.8, true);
        hud.shown = hud.shown.started_ago(2.0);
        assert_eq!(hud.shown.value(), 0.0);
        hud.update(Kind::Display, 0.9, false);
        assert_eq!(hud.shown.value(), 0.9);
    }
}
