//! Notification banners: glass cards that slide in at the top right and out again, newest on
//! top. The surface exists only while there are banners, and only takes input over them.

use std::time::{Duration, Instant};

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::Length;
use iced_widget::{container, pin, stack};

use crate::anim::{self, Motion, Spring};
use crate::app::{Element, Message};
use crate::notifications::{
    CARD_WIDTH, HoverFade, Notification, OVERHANG, RADIUS, card, chip_shape, close_shape,
};
use crate::{material, menubar};

const MAX_BANNERS: usize = 4;
const GAP: f32 = 8.0;
/// Between the bar and the first banner.
const TOP_MARGIN: i32 = 8;
const RIGHT_MARGIN: f32 = 16.0;
const WIDTH: f32 = OVERHANG + CARD_WIDTH + RIGHT_MARGIN;
/// Room for [`MAX_BANNERS`] of the tallest cards.
const HEIGHT: f32 = OVERHANG + MAX_BANNERS as f32 * (134.0 + GAP) + 360.0;
// Measured from a recording of Tahoe's.
/// How fast a banner comes in: how long it takes to get about two thirds of the way, in seconds. It's
/// fastest at the start.
const SLIDE_IN: f32 = 0.042;
/// How long it takes to slide out.
const SLIDE_OUT: Duration = Duration::from_millis(200);
/// How the ones under one that's gone move up.
const ROW_SPRING: Spring = Spring::new(0.285, 0.92);
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
        namespace: Some(String::from("dock")),
        ..Default::default()
    }
}

struct Banner {
    notification: Notification,
    expires: Option<Instant>,
    arrived: Instant,
    leaving: bool,
    /// When it started sliding out, and from where, once it has.
    left: Option<(Instant, f32)>,
    y: Motion,
}

impl Banner {
    /// How far it has slid out: 0 in place, 1 off the screen.
    fn slide(&self) -> f32 {
        match self.left {
            Some((started, from)) => {
                let t = (anim::seconds_since(started) / SLIDE_OUT.as_secs_f32()).min(1.0);
                from + (1.0 - from) * t * t * (3.0 - 2.0 * t)
            }
            None => {
                let out = (-anim::seconds_since(self.arrived) / SLIDE_IN).exp();
                if out < 0.002 { 0.0 } else { out }
            }
        }
    }

    fn x(&self) -> f32 {
        OVERHANG + self.slide() * (CARD_WIDTH + RIGHT_MARGIN)
    }

    fn leave(&mut self) {
        self.leaving = true;
        if self.left.is_none() {
            self.left = Some((anim::now(), self.slide()));
        }
    }
}

/// The banners on screen, top to bottom.
#[derive(Default)]
pub struct Banners {
    banners: Vec<Banner>,
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
        self.banners.iter().any(|banner| {
            banner.left.is_some()
                || banner.slide() != 0.0
                || banner.y.is_moving()
                || banner.notification.is_expanding()
        })
    }

    /// Whether any banner is waiting to expire.
    pub fn counting_down(&self) -> bool {
        self.banners.iter().any(|banner| banner.expires.is_some())
    }

    /// Sends each one toward where it should be now.
    fn rearrange(&mut self) {
        for index in 0..self.banners.len() {
            let target = self.y_target(index);
            self.banners[index].y.go_to(target);
        }
    }

    /// Shows a new banner on top, sending off the oldest if there are too many.
    pub fn show(&mut self, notification: Notification, now: Instant) {
        self.banners.insert(
            0,
            Banner {
                expires: notification.timeout.map(|timeout| now + timeout),
                notification,
                arrived: anim::now(),
                leaving: false,
                left: None,
                y: Motion::at(0.0, ROW_SPRING, 0.25),
            },
        );
        if let Some(oldest) = self
            .banners
            .iter_mut()
            .filter(|banner| !banner.leaving)
            .nth(MAX_BANNERS)
        {
            oldest.leave();
        }
        self.rearrange();
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
        self.rearrange();
        true
    }

    /// Takes away the banner showing this notification, if any, at once, which is how one goes when
    /// it's closed. Those under it move up.
    pub fn remove(&mut self, id: u32) {
        self.banners.retain(|banner| banner.notification.id != id);
        self.rearrange();
    }

    /// Sends off the banner showing this notification, if any.
    pub fn dismiss(&mut self, id: u32) {
        if let Some(banner) = self
            .banners
            .iter_mut()
            .find(|banner| banner.notification.id == id)
        {
            banner.leave();
        }
        self.rearrange();
    }

    pub fn dismiss_all(&mut self) {
        for banner in &mut self.banners {
            banner.leave();
        }
    }

    /// Opens up the banner showing this notification to show its picture, or closes it, and it stays
    /// until it's dismissed, since someone's looking at it.
    pub fn toggle_expanded(&mut self, id: u32) {
        if let Some(banner) = self
            .banners
            .iter_mut()
            .find(|banner| banner.notification.id == id)
        {
            banner.notification.toggle_expanded();
            banner.expires = None;
        }
        self.rearrange();
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
        let mut gone = false;
        for banner in &mut self.banners {
            if !banner.leaving
                && hovered != Some(banner.notification.id)
                && banner.expires.is_some_and(|expires| now >= expires)
            {
                banner.leave();
                gone = true;
                if banner.notification.transient {
                    over.push(banner.notification.id);
                }
            }
        }
        self.banners
            .retain(|banner| !(banner.leaving && banner.slide() >= 1.0));
        if gone {
            self.rearrange();
        }
        over
    }

    /// Where a banner is headed: under the ones above it that aren't leaving.
    fn y_target(&self, index: usize) -> f32 {
        self.banners[..index]
            .iter()
            .filter(|banner| !banner.leaving)
            .map(|banner| banner.notification.target_height() + GAP)
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
                    (banner.y.value() + OVERHANG) as i32,
                    CARD_WIDTH as i32,
                    banner.notification.height() as i32,
                ),
                radius: RADIUS as i32,
                style: material::Style::Card,
            });
            opacities.push(1.0);
            // On top of the card's, so after it.
            let capsule = banner.notification.capsule_shows(fade.amount(id));
            if capsule > 0.0
                && let Some(rect) = banner.notification.capsule_rect()
            {
                shapes.push(chip_shape(
                    banner.x() + rect.x,
                    banner.y.value() + OVERHANG + rect.y,
                    rect.width,
                    rect.height,
                ));
                opacities.push(capsule);
            }
            if hovered == Some(id) {
                shapes.push(close_shape(banner.x() - OVERHANG, banner.y.value()));
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
                    banner.y.value() as i32,
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
                    (banner.x(), banner.y.value() + OVERHANG),
                    material,
                    Message::NotificationPressed(id),
                    Message::NotificationDismissed(id),
                ))
                .x(banner.x() - OVERHANG)
                .y(banner.y.value())
                .into()
            })
            .collect();
        container(stack(layers))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
