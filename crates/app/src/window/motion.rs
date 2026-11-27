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
    /// The token's name in DESIGN.md, which a test sees when the movement runs.
    pub(crate) name: &'static str,
    pub(crate) duration: Duration,
    pub(crate) curve: Curve,
}

impl Motion {
    const fn new(name: &'static str, millis: u64, curve: Curve) -> Self {
        Self {
            name,
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
        let (name, curve) = (self.name, self.curve);
        Animation::new(self.duration).with_easing(move |t| {
            seen::moving(name, t);
            curve.ease(t)
        })
    }
}

/// A color or what shows on hover changing under the pointer.
pub(crate) const HOVER: Motion = Motion::new("hover", 120, Curve::EaseOut);
/// A reply, a tool line or a card arriving.
pub(crate) const MESSAGE_ENTER: Motion = Motion::new("message-enter", 180, Curve::EaseOut);
/// The assistant's picture settling in as the onboarding window greets the owner.
pub(crate) const GREET: Motion = Motion::new("greet", 260, Curve::EaseOut);
/// One onboarding step giving way to the next as the whole screen slides in.
pub(crate) const STEP: Motion = Motion::new("step", 240, Curve::EaseOut);
/// The raised segment of a switch sliding to the one the owner picked.
pub(crate) const SWITCH: Motion = Motion::new("switch", 180, Curve::EaseOut);
/// The settings pane gliding down to the part a link names.
pub(crate) const GLIDE: Motion = Motion::new("glide", 320, Curve::EaseInOut);
/// The conversation following new text.
pub(crate) const FOLLOW: Motion = Motion::new("follow", 180, Curve::EaseOut);
/// One breath of the logo while dotori works or starts. The only motion that repeats.
pub(crate) const WORKING: Motion = Motion::new("working", 1600, Curve::EaseInOut);

/// How far what arrives rises as it fades in, `spacing.rise` of DESIGN.md.
pub(crate) const RISE: f32 = 6.;

/// How far an onboarding step slides in from the side it comes from, `spacing.slide` of
/// DESIGN.md.
pub(crate) const SLIDE: f32 = 28.;

impl Motion {
    /// The movement as an animation that waits `delay` before it starts, so a few things can
    /// arrive one after another.
    pub(crate) fn after(self, delay: Duration) -> Animation {
        let total = delay + self.duration;
        let (wait, length, curve) = (delay.as_secs_f32(), self.duration.as_secs_f32(), self.curve);
        let name = self.name;
        Animation::new(total).with_easing(move |t| {
            seen::moving(name, t);
            let elapsed = t * (wait + length);
            curve.ease(((elapsed - wait) / length).max(0.0))
        })
    }
}

impl Motion {
    /// The movement in and out again: nothing until `from`, in over the token's time, held, and
    /// out over the token's time ending at `until`, so a mark shows for a while and goes.
    pub(crate) fn between(self, from: Duration, until: Duration) -> Animation {
        let (start, end, length, curve) = (
            from.as_secs_f32(),
            until.as_secs_f32(),
            self.duration.as_secs_f32(),
            self.curve,
        );
        let name = self.name;
        Animation::new(until).with_easing(move |t| {
            seen::moving(name, t);
            let elapsed = t * end;
            let coming = curve.ease((elapsed - start) / length);
            let going = curve.ease((end - elapsed) / length);
            coming.min(going).max(0.0)
        })
    }
}

/// How bright the logo is `progress` of the way through one breath: full at both ends and
/// dimmest in the middle, so a breath runs into the next without a jump.
pub(crate) fn breath(progress: f32) -> f32 {
    let wave = 1.0 - (2.0 * progress.clamp(0.0, 1.0) - 1.0).abs();
    1.0 - 0.55 * WORKING.curve.ease(wave)
}

/// What moved while the windows drew, so a test can tell a movement nobody asked for, such as a
/// switch sliding again when its section opens. Outside tests it records nothing.
pub(crate) mod seen {
    #[cfg(test)]
    thread_local! {
        static MOVED: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Notes that the movement `name` drew a frame `progress` of the way through.
    pub(crate) fn moving(name: &'static str, progress: f32) {
        #[cfg(test)]
        if progress < 1.0 {
            MOVED.with(|moved| moved.borrow_mut().push(name));
        }
        #[cfg(not(test))]
        let _ = (name, progress);
    }

    /// The movements that drew a frame short of their end since the last call, each once.
    #[cfg(test)]
    pub(crate) fn take() -> Vec<&'static str> {
        let mut moved = MOVED.with(|moved| std::mem::take(&mut *moved.borrow_mut()));
        moved.sort_unstable();
        moved.dedup();
        moved
    }
}

#[cfg(test)]
#[path = "motion.test.rs"]
mod tests;
