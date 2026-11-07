//! Time and motion, for everything that moves.
//!
//! What's on screen in a frame was decided at one moment, which this module keeps: everything asks
//! it what time it is, rather than asking the system, so the glass and what's on it can't disagree.
//! That moment comes from the frames themselves, right after each one is drawn, so that motion is
//! sampled at a steady distance from when it's shown. On a timer of its own, the time between a
//! sample and the frame showing it keeps changing, which shows as uneven motion however smooth the
//! curve.
//!
//! Motion is on springs, in the terms Apple describes them in: how long a spring takes to get most of
//! the way there, and how little it overshoots. They're solved rather than stepped, so they don't
//! care how regular frames are, and they can change their mind on the way without a jolt.

use std::cell::Cell;
use std::f32::consts::TAU;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The most time that passes from one frame to the next, as far as motion goes.
const MAX_STEP: Duration = Duration::from_millis(25);

thread_local! {
    static NOW: Cell<Option<Instant>> = const { Cell::new(None) };
    static LAST_FRAME: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The moment the frame being prepared is for.
pub fn now() -> Instant {
    NOW.get().unwrap_or_else(Instant::now)
}

/// Says that nothing's moving, so that what starts moving starts now. Time doesn't go backward.
pub fn set_now(at: Instant) {
    NOW.set(Some(NOW.get().map_or(at, |now| now.max(at))));
    LAST_FRAME.set(Some(at));
}

/// Moves on to the frame drawn at this moment, and returns the time for it. Motion doesn't make up
/// for a late frame, which would be a jump: it carries on from where it was, a little late, which
/// nobody sees.
pub fn next_frame(at: Instant) -> Instant {
    let step = LAST_FRAME
        .get()
        .map_or(Duration::ZERO, |last| at.saturating_duration_since(last))
        .min(MAX_STEP)
        .div_f32(slow_motion());
    LAST_FRAME.set(Some(LAST_FRAME.get().map_or(at, |last| last.max(at))));
    let now = NOW.get().unwrap_or(at) + step;
    NOW.set(Some(now));
    now
}

/// How many times slower motion goes, from `MAKSHELL_SLOW_MOTION`, to see what it really does.
fn slow_motion() -> f32 {
    static SLOW_MOTION: OnceLock<f32> = OnceLock::new();
    *SLOW_MOTION.get_or_init(|| {
        std::env::var("MAKSHELL_SLOW_MOTION")
            .ok()
            .and_then(|slow| slow.parse().ok())
            .filter(|slow| *slow >= 1.0)
            .unwrap_or(1.0)
    })
}

pub fn seconds_since(then: Instant) -> f32 {
    now().saturating_duration_since(then).as_secs_f32()
}

/// A spring's character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// In seconds: about how long it takes to get there.
    pub response: f32,
    /// From 1, which doesn't overshoot at all, down toward 0, which never stops.
    pub damping: f32,
}

impl Spring {
    pub const fn new(response: f32, damping: f32) -> Self {
        Self { response, damping }
    }

    /// Where something is and how fast it's going, `seconds` after it was `offset` away from where
    /// the spring pulls it, going at `velocity`.
    fn sample(self, offset: f32, velocity: f32, seconds: f32) -> (f32, f32) {
        let omega = TAU / self.response;
        if self.damping >= 1.0 {
            let decay = (-omega * seconds).exp();
            let lead = velocity + omega * offset;
            let position = offset + lead * seconds;
            return (position * decay, (lead - omega * position) * decay);
        }
        let decay_rate = self.damping * omega;
        let ringing = omega * (1.0 - self.damping * self.damping).sqrt();
        let lead = (velocity + decay_rate * offset) / ringing;
        let decay = (-decay_rate * seconds).exp();
        let (sin, cos) = (ringing * seconds).sin_cos();
        (
            decay * (offset * cos + lead * sin),
            decay
                * ((lead * ringing - decay_rate * offset) * cos
                    - (offset * ringing + decay_rate * lead) * sin),
        )
    }

    /// How far along something is, from 0 toward 1 and perhaps a little past, `seconds` after
    /// setting off from a standstill.
    pub fn progress(self, seconds: f32) -> f32 {
        self.progress_from(0.0, seconds)
    }

    /// The same for something that set off at this speed, in whole distances a second.
    pub fn progress_from(self, speed: f32, seconds: f32) -> f32 {
        if seconds <= 0.0 {
            return 0.0;
        }
        1.0 - self.sample(1.0, -speed, seconds).0
    }
}

/// A value that follows a spring to wherever it's told to go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// Where it's going.
    target: f32,
    /// How far from there it was when it set off, and how fast it was going.
    offset: f32,
    velocity: f32,
    started: Instant,
    spring: Spring,
    /// How close counts as there.
    tolerance: f32,
}

impl Motion {
    /// A value that's at rest. `tolerance` is how close counts as being there.
    pub fn at(value: f32, spring: Spring, tolerance: f32) -> Self {
        Self {
            target: value,
            offset: 0.0,
            velocity: 0.0,
            started: now(),
            spring,
            tolerance,
        }
    }

    fn sample(&self, delay: f32) -> (f32, f32) {
        let seconds = seconds_since(self.started) - delay;
        if seconds <= 0.0 {
            return (self.offset, if delay > 0.0 { 0.0 } else { self.velocity });
        }
        self.spring.sample(self.offset, self.velocity, seconds)
    }

    fn settled(&self, delay: f32) -> bool {
        let (offset, velocity) = self.sample(delay);
        // Not before it's had the chance to set off.
        seconds_since(self.started) >= delay
            && offset.abs() <= self.tolerance
            && velocity.abs() <= self.tolerance * 10.0
    }

    /// Where it is now.
    pub fn value(&self) -> f32 {
        self.delayed(0.0)
    }

    /// Where it was this many seconds ago, for things that follow a little behind.
    pub fn delayed(&self, delay: f32) -> f32 {
        if self.settled(delay) {
            return self.target;
        }
        self.target + self.sample(delay).0
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_moving(&self) -> bool {
        !self.settled(0.0)
    }

    /// Whether something this many seconds behind it is still moving.
    pub fn is_moving_delayed(&self, delay: f32) -> bool {
        !self.settled(delay)
    }

    /// Heads for `target` from wherever it is, keeping its speed.
    pub fn go_to(&mut self, target: f32) {
        if target != self.target {
            self.set_off(target, self.spring);
        }
    }

    /// Heads for `target` on another spring, from wherever it is, keeping its speed.
    pub fn go_to_on(&mut self, target: f32, spring: Spring) {
        if target != self.target || spring != self.spring {
            self.set_off(target, spring);
        }
    }

    fn set_off(&mut self, target: f32, spring: Spring) {
        let (offset, velocity) = self.sample(0.0);
        let value = self.target + offset;
        self.target = target;
        self.offset = value - target;
        self.velocity = velocity;
        self.started = now();
        self.spring = spring;
    }

    /// Goes there at once, with nothing left of how it was moving.
    pub fn jump_to(&mut self, value: f32) {
        self.target = value;
        self.offset = 0.0;
        self.velocity = 0.0;
        self.started = now();
    }

    /// As if it had set off this long ago.
    #[cfg(test)]
    pub fn started_ago(mut self, seconds: f32) -> Self {
        self.started = now()
            .checked_sub(std::time::Duration::from_secs_f32(seconds))
            .unwrap_or_else(now);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tahoe's measured panel, opening: the share of the way there at 33, 83, 133 and 183ms.
    #[test]
    fn a_spring_of_a_quarter_second_is_about_as_fast_as_tahoes_panels() {
        let spring = Spring::new(0.25, 0.85);
        for (ms, there) in [(33.0, 0.18), (83.0, 0.65), (133.0, 0.85), (183.0, 0.97)] {
            let got = spring.progress(ms / 1000.0);
            assert!((got - there).abs() < 0.08, "{got} at {ms}ms, not {there}");
        }
    }

    #[test]
    fn it_overshoots_only_as_much_as_it_should() {
        let peak = |damping| {
            (0..200)
                .map(|ms| Spring::new(0.25, damping).progress(ms as f32 / 200.0))
                .fold(0.0, f32::max)
        };
        assert!(peak(1.0) <= 1.0);
        assert!(peak(0.8) > 1.005 && peak(0.8) < 1.03);
        assert!(peak(0.6) > 1.05 && peak(0.6) < 1.12);
    }

    #[test]
    fn a_late_frame_doesnt_make_motion_jump() {
        let start = Instant::now();
        set_now(start);
        assert_eq!(next_frame(start + MAX_STEP / 2), start + MAX_STEP / 2);
        let after = next_frame(start + Duration::from_millis(200));
        assert_eq!(after, start + MAX_STEP / 2 + MAX_STEP);
        // Nor does it go backward.
        assert_eq!(next_frame(start), after);
    }

    #[test]
    fn a_motion_gets_there_and_says_so() {
        let mut motion = Motion::at(0.0, Spring::new(0.25, 0.8), 0.002);
        assert!(!motion.is_moving());
        motion.go_to(1.0);
        assert!(motion.is_moving());
        assert!(motion.value() < 0.05);
        let motion = motion.started_ago(2.0);
        assert!(!motion.is_moving());
        assert_eq!(motion.value(), 1.0);
    }

    #[test]
    fn changing_its_mind_keeps_where_it_is_and_how_fast_its_going() {
        let mut motion = Motion::at(0.0, Spring::new(0.25, 0.8), 0.002);
        motion.go_to(1.0);
        let mut motion = motion.started_ago(0.06);
        let (before, speed) = (motion.value(), motion.sample(0.0).1);
        assert!(speed > 1.0);
        motion.go_to(0.0);
        assert!((motion.value() - before).abs() < 0.01);
        assert!((motion.sample(0.0).1 - speed).abs() < 0.2);
    }

    #[test]
    fn what_follows_behind_hasnt_set_off_until_its_turn() {
        let mut motion = Motion::at(0.0, Spring::new(0.25, 0.8), 0.002);
        motion.go_to(1.0);
        let motion = motion.started_ago(0.02);
        assert_eq!(motion.delayed(0.1), 0.0);
        assert!(motion.value() > 0.0);
        assert!(motion.is_moving_delayed(0.1));
    }
}
