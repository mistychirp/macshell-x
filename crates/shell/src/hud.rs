//! The little capsule that shows the volume or the brightness when a key changes it, under the right
//! end of the bar, looking like Control Center's own.

use std::time::{Duration, Instant};

use iced_layershell::reexport::{
    Anchor, IcedId, KeyboardInteractivity, Layer, NewLayerShellSettings,
};
use iced_widget::core::{Alignment, Background, Border, Color, Length};
use iced_widget::{Space, column, container, row, stack, text};

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
/// How much of the way to the new level the bar goes each frame.
const EASE: f32 = 0.28;

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
    shown: f32,
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
            shown: if muted { 0.0 } else { level },
            changed: Instant::now(),
        }
    }

    /// Shows a new level, and stays for longer.
    pub fn update(&mut self, kind: Kind, level: f32, muted: bool) {
        if kind != self.kind {
            self.shown = if muted { 0.0 } else { level };
        }
        self.kind = kind;
        self.level = level;
        self.muted = muted;
        self.changed = Instant::now();
    }

    /// Moves the bar along. Returns whether it's time for it to go.
    pub fn frame(&mut self) -> bool {
        let target = if self.muted { 0.0 } else { self.level };
        self.shown += (target - self.shown) * EASE;
        if (target - self.shown).abs() < 0.002 {
            self.shown = target;
        }
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
            line((self.shown * BAR_WIDTH).max(0.0), Color::WHITE),
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
        namespace: Some(String::from("macshell-hud")),
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

/// What it grows out of: the top of its right end, toward the bar's status items.
pub fn origin() -> (i32, i32) {
    ((WIDTH * 0.75) as i32, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_eases_to_the_level_and_settles() {
        let mut hud = Hud::new(IcedId::unique(), Kind::Sound, 0.2, false);
        hud.update(Kind::Sound, 0.8, false);
        hud.frame();
        assert!(hud.shown > 0.2 && hud.shown < 0.8);
        for _ in 0..60 {
            hud.frame();
        }
        assert_eq!(hud.shown, 0.8);
    }

    #[test]
    fn muting_empties_the_bar_and_switching_kinds_jumps() {
        let mut hud = Hud::new(IcedId::unique(), Kind::Sound, 0.5, true);
        assert_eq!(hud.shown, 0.0);
        hud.update(Kind::Display, 0.9, false);
        assert_eq!(hud.shown, 0.9);
    }
}
