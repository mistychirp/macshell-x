//! Notification banners: glass cards that slide in at the top right and out again, newest on
//! top. The surface exists only while there are banners, and only takes input over them.

use std::time::{Duration, Instant};

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::Length;
use iced_widget::{container, pin, stack};

use crate::app::{Element, Message};
use crate::notifications::{
    CARD_WIDTH, HoverFade, Notification, OVERHANG, RADIUS, Spring, card, chip_shape, close_shape,
};
use crate::{material, menubar};

const MAX_BANNERS: usize = 4;
const GAP: f32 = 8.0;
/// Between the bar and the first banner.
const TOP_MARGIN: i32 = 8;
const RIGHT_MARGIN: f32 = 16.0;
const WIDTH: f32 = OVERHANG + CARD_WIDTH + RIGHT_MARGIN;
/// Room for [`MAX_BANNERS`] of the tallest cards.
const HEIGHT: f32 = OVERHANG + MAX_BANNERS as f32 * (134.0 + GAP);
/// Time constant of how a banner slides in, in seconds.
const EASING: f32 = 0.07;
/// How long it takes to slide out, picking up speed, measured from Tahoe's.
const SLIDE_OUT: Duration = Duration::from_millis(220);
/// A banner that was being pointed at stays this long after the pointer leaves.
const AFTER_HOVER: std::time::Duration = std::time::Duration::from_secs(2);

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Right,
        layer: Layer::Overlay,
        size: Some((WIDTH as u32, HEIGHT as u32)),
        exclusive_zone: Some(-1),
        margin: Some((menubar::HEIGHT as i32 + TOP_MARGIN, 0, 0, 0)),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-notifications")),
        ..Default::default()
    }
}

struct Banner {
    notification: Notification,
    expires: Option<Instant>,
    /// How far it has slid out: 0 in place, 1 off the screen.
    slide: f32,
    leaving: bool,
    /// When it started sliding out, and from where, once it has.
    left: Option<(Instant, f32)>,
    y: Spring,
}

impl Banner {
    fn x(&self) -> f32 {
        OVERHANG + self.slide * (CARD_WIDTH + RIGHT_MARGIN)
    }
}

/// The banners on screen, top to bottom.
#[derive(Default)]
pub struct Banners {
    banners: Vec<Banner>,
    last_frame: Option<Instant>,
}

impl Banners {
    pub fn is_empty(&self) -> bool {
        self.banners.is_empty()
    }

    /// The notification a banner shows; transient ones aren't kept anywhere else.
    pub fn find(&self, id: u32) -> Option<&Notification> {
        self.banners
            .iter()
            .map(|banner| &banner.notification)
            .find(|notification| notification.id == id)
    }

    /// Whether anything is moving, which takes frames.
    pub fn animating(&self) -> bool {
        self.banners.iter().enumerate().any(|(index, banner)| {
            banner.slide != slide_target(banner) || !banner.y.is_at(self.y_target(index))
        })
    }

    /// Whether any banner is waiting to expire.
    pub fn counting_down(&self) -> bool {
        self.banners.iter().any(|banner| banner.expires.is_some())
    }

    /// Shows a new banner on top, sending off the oldest if there are too many.
    pub fn show(&mut self, notification: Notification, now: Instant) {
        self.banners.insert(
            0,
            Banner {
                expires: notification.timeout.map(|timeout| now + timeout),
                notification,
                slide: 1.0,
                leaving: false,
                left: None,
                y: Spring::default(),
            },
        );
        if let Some(oldest) = self
            .banners
            .iter_mut()
            .filter(|banner| !banner.leaving)
            .nth(MAX_BANNERS)
        {
            oldest.leaving = true;
        }
        // It starts where it's going to be; only the slide shows.
        self.banners[0].y = Spring::at(self.y_target(0));
        self.last_frame = None;
    }

    /// Updates the banner showing this notification, if any, and restarts its time.
    pub fn replace(&mut self, notification: &Notification, now: Instant) -> bool {
        let Some(banner) = self
            .banners
            .iter_mut()
            .find(|banner| !banner.leaving && banner.notification.id == notification.id)
        else {
            return false;
        };
        banner.expires = notification.timeout.map(|timeout| now + timeout);
        banner.notification = notification.clone();
        self.last_frame = None;
        true
    }

    /// Takes away the banner showing this notification, if any, at once, which is how one goes when
    /// it's closed. Those under it move up.
    pub fn remove(&mut self, id: u32) {
        self.banners.retain(|banner| banner.notification.id != id);
        self.last_frame = None;
    }

    /// Sends off the banner showing this notification, if any.
    pub fn dismiss(&mut self, id: u32) {
        if let Some(banner) = self
            .banners
            .iter_mut()
            .find(|banner| banner.notification.id == id)
        {
            banner.leaving = true;
        }
        self.last_frame = None;
    }

    pub fn dismiss_all(&mut self) {
        for banner in &mut self.banners {
            banner.leaving = true;
        }
        self.last_frame = None;
    }

    /// The pointer left a banner: gives it a moment before it goes.
    pub fn pointer_left(&mut self, id: u32, now: Instant) {
        if let Some(banner) = self
            .banners
            .iter_mut()
            .find(|banner| banner.notification.id == id)
            && let Some(expires) = &mut banner.expires
        {
            *expires = (*expires).max(now + AFTER_HOVER);
        }
    }

    /// Moves everything along. Returns the transient notifications whose banners just expired,
    /// which are over; the others live on in Notification Center.
    pub fn frame(&mut self, now: Instant, hovered: Option<u32>) -> Vec<u32> {
        let mut over = Vec::new();
        for banner in &mut self.banners {
            if !banner.leaving
                && hovered != Some(banner.notification.id)
                && banner.expires.is_some_and(|expires| now >= expires)
            {
                banner.leaving = true;
                if banner.notification.transient {
                    over.push(banner.notification.id);
                }
            }
        }

        // Exponential easing, independent of how regularly frames arrive.
        let dt = self
            .last_frame
            .map_or(0.016, |last| (now - last).as_secs_f32())
            .min(0.05);
        self.last_frame = Some(now);
        let ease = 1.0 - (-dt / EASING).exp();

        for index in 0..self.banners.len() {
            let y_target = self.y_target(index);
            let banner = &mut self.banners[index];
            banner.y.step(y_target, dt);
            if banner.leaving {
                let (started, from) = *banner.left.get_or_insert((now, banner.slide));
                let t = (now.saturating_duration_since(started).as_secs_f32()
                    / SLIDE_OUT.as_secs_f32())
                .min(1.0);
                banner.slide = from + (1.0 - from) * t * t;
            } else {
                banner.slide = approach(banner.slide, 0.0, ease);
            }
        }
        self.banners
            .retain(|banner| !(banner.leaving && banner.slide == 1.0));
        over
    }

    /// Where a banner is headed: under the ones above it that aren't leaving.
    fn y_target(&self, index: usize) -> f32 {
        self.banners[..index]
            .iter()
            .filter(|banner| !banner.leaving)
            .map(|banner| banner.notification.height() + GAP)
            .sum()
    }

    /// The glass, with how opaque each shape is.
    pub fn material_shapes(
        &self,
        hovered: Option<u32>,
        fade: &HoverFade,
    ) -> (Vec<material::Shape>, Vec<f32>) {
        let mut shapes = Vec::new();
        let mut opacities = Vec::new();
        for banner in &self.banners {
            let id = banner.notification.id;
            shapes.push(material::Shape {
                rect: (
                    banner.x() as i32,
                    (banner.y.value + OVERHANG) as i32,
                    CARD_WIDTH as i32,
                    banner.notification.height() as i32,
                ),
                radius: RADIUS as i32,
                style: material::Style::Card,
            });
            opacities.push(1.0);
            // On top of the card's, so after it.
            let capsule = fade.amount(id);
            if capsule > 0.0
                && let Some(rect) = banner.notification.capsule_rect()
            {
                shapes.push(chip_shape(
                    banner.x() + rect.x,
                    banner.y.value + OVERHANG + rect.y,
                    rect.width,
                    rect.height,
                ));
                opacities.push(capsule);
            }
            if hovered == Some(id) {
                shapes.push(close_shape(banner.x() - OVERHANG, banner.y.value));
                opacities.push(1.0);
            }
        }
        (shapes, opacities)
    }

    /// Where the surface takes input: over banners that are staying, with their close buttons.
    pub fn input_region(&self) -> Vec<(i32, i32, i32, i32)> {
        self.banners
            .iter()
            .filter(|banner| !banner.leaving)
            .map(|banner| {
                (
                    (banner.x() - OVERHANG) as i32,
                    banner.y.value as i32,
                    (CARD_WIDTH + OVERHANG) as i32,
                    (banner.notification.height() + OVERHANG) as i32,
                )
            })
            .collect()
    }

    pub fn view(
        &self,
        hovered: Option<u32>,
        fade: &HoverFade,
        material: Option<material::Kind>,
    ) -> Element<'_> {
        let layers: Vec<Element<'_>> = self
            .banners
            .iter()
            .map(|banner| {
                let id = banner.notification.id;
                pin(card(
                    &banner.notification,
                    None,
                    hovered == Some(id),
                    fade.amount(id),
                    (banner.x(), banner.y.value + OVERHANG),
                    material,
                    Message::NotificationPressed(id),
                    Message::NotificationDismissed(id),
                ))
                .x(banner.x() - OVERHANG)
                .y(banner.y.value)
                .into()
            })
            .collect();
        container(stack(layers))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn slide_target(banner: &Banner) -> f32 {
    if banner.leaving { 1.0 } else { 0.0 }
}

fn approach(value: f32, target: f32, ease: f32) -> f32 {
    let next = value + (target - value) * ease;
    if (target - next).abs() < 0.004 * target.abs().max(1.0) {
        target
    } else {
        next
    }
}
