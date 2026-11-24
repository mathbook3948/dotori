//! The motion tokens of DESIGN.md: how long each kind of movement takes and how it eases. A test
//! reads DESIGN.md, so a change there fails until it is made here too.

use std::time::Duration;

use gpui::Animation;
use gpui::ease_in_out;
use gpui::ease_out_quint;

/// How a movement eases between its start and its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Curve {
    /// Fast at first and slow at the end, for what arrives.
    EaseOut,
    /// Slow at both ends, for what changes in place.
    EaseInOut,
}

impl Curve {
    /// Where a movement `progress` of the way through its time is, from 0 to 1.
    pub(crate) fn ease(self, progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        match self {
            Self::EaseOut => ease_out_quint()(t),
            Self::EaseInOut => ease_in_out(t),
        }
    }
}

/// One motion token: a duration and a curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Motion {
    pub(crate) duration: Duration,
    pub(crate) curve: Curve,
}

impl Motion {
    const fn new(millis: u64, curve: Curve) -> Self {
        Self {
            duration: Duration::from_millis(millis),
            curve,
        }
    }

    /// How far along the movement is `elapsed` after it began, eased, from 0 to 1.
    pub(crate) fn at(self, elapsed: Duration) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        self.curve
            .ease(elapsed.as_secs_f32() / self.duration.as_secs_f32())
    }

    /// Whether the movement is still going `elapsed` after it began.
    pub(crate) fn moving(self, elapsed: Duration) -> bool {
        elapsed < self.duration
    }

    /// The movement as a GPUI animation that runs once.
    pub(crate) fn animation(self) -> Animation {
        let curve = self.curve;
        Animation::new(self.duration).with_easing(move |t| curve.ease(t))
    }
}

/// A color or what shows on hover changing under the pointer.
pub(crate) const HOVER: Motion = Motion::new(120, Curve::EaseOut);
/// A reply, a tool line or a card arriving.
pub(crate) const MESSAGE_ENTER: Motion = Motion::new(180, Curve::EaseOut);
/// The conversation following new text.
pub(crate) const FOLLOW: Motion = Motion::new(180, Curve::EaseOut);
/// One breath of the logo while dotori works. The only motion that repeats.
pub(crate) const WORKING: Motion = Motion::new(1600, Curve::EaseInOut);

/// How far what arrives rises as it fades in, `spacing.rise` of DESIGN.md.
pub(crate) const RISE: f32 = 6.;

/// How bright the logo is `progress` of the way through one breath: full at both ends and
/// dimmest in the middle, so a breath runs into the next without a jump.
pub(crate) fn breath(progress: f32) -> f32 {
    let wave = 1.0 - (2.0 * progress.clamp(0.0, 1.0) - 1.0).abs();
    1.0 - 0.55 * WORKING.curve.ease(wave)
}

#[cfg(test)]
#[path = "motion.test.rs"]
mod tests;
