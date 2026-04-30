//! Smooth transitions between values over time.

use std::time::Duration;

/// Describes a smooth, time-based transition between two values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    /// Total duration of the transition.
    pub duration: Duration,
    /// Easing curve applied to the interpolation.
    pub easing: Easing,
}

impl Tween {
    /// Linear 0-duration tween (instant).
    pub const INSTANT: Self = Self {
        duration: Duration::ZERO,
        easing: Easing::Linear,
    };

    /// Create a tween with the given duration and easing.
    pub fn new(duration: Duration, easing: Easing) -> Self {
        Self { duration, easing }
    }

    /// Sample the tween at `t` seconds.
    /// Returns a value in [0.0, 1.0] ready to lerp with.
    pub fn sample(&self, t: f32) -> f32 {
        if self.duration.as_secs_f32() <= 0.0 {
            return 1.0;
        }
        let norm = (t / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        self.easing.evaluate(norm)
    }
}

impl Default for Tween {
    fn default() -> Self {
        Self::INSTANT
    }
}

/// Easing curve for tweens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    /// Constant rate.
    Linear,
    /// Slow start, fast end.
    EaseIn,
    /// Fast start, slow end.
    EaseOut,
    /// Smooth acceleration and deceleration.
    EaseInOut,
}

impl Easing {
    /// Evaluate the easing curve at `t` ∈ [0, 1].
    pub fn evaluate(self, t: f32) -> f32 {
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}
