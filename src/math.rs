//! Core math and DSP helpers.

use std::ops::{Add, Mul, Sub};
use std::time::Duration;

/// A single stereo audio frame (L, R) in f32.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// Left channel.
    pub l: f32,
    /// Right channel.
    pub r: f32,
}

impl Frame {
    /// Silence.
    pub const SILENCE: Self = Self { l: 0.0, r: 0.0 };

    /// Uniform (centre) frame with the same value on both channels.
    pub fn mono(v: f32) -> Self {
        Self { l: v, r: v }
    }

    /// Sample-by-sample multiply (amplitude modulation / gain).
    pub fn scale(self, gain: f32) -> Self {
        Self {
            l: self.l * gain,
            r: self.r * gain,
        }
    }

    /// Apply per-channel gain.
    pub fn scale_lr(self, gain_l: f32, gain_r: f32) -> Self {
        Self {
            l: self.l * gain_l,
            r: self.r * gain_r,
        }
    }

    /// Mix two frames.
    pub fn mix(a: Self, b: Self) -> Self {
        Self {
            l: a.l + b.l,
            r: a.r + b.r,
        }
    }

    /// Clamp both channels to [-1.0, 1.0] to avoid digital clipping.
    pub fn clamp(self) -> Self {
        Self {
            l: self.l.clamp(-1.0, 1.0),
            r: self.r.clamp(-1.0, 1.0),
        }
    }
}

impl Add for Frame {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::mix(self, rhs)
    }
}

impl Sub for Frame {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            l: self.l - rhs.l,
            r: self.r - rhs.r,
        }
    }
}

impl Mul<f32> for Frame {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        self.scale(rhs)
    }
}

// ── Decibels ────────────────────────────────────────────────────────────────

/// Helpers for converting between linear amplitude and decibels.
pub struct Decibels;

impl Decibels {
    /// Convert linear amplitude [0, ∞) to decibels.
    /// 0.0 → -∞ dB, 1.0 → 0 dB.
    pub fn from_linear(gain: f32) -> f32 {
        if gain <= 0.0 {
            f32::NEG_INFINITY
        } else {
            20.0 * gain.log10()
        }
    }

    /// Convert decibels to linear amplitude.
    /// -∞ dB → 0.0, 0 dB → 1.0.
    pub fn to_linear(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    /// Clamp decibel value for UI display.
    pub fn clamp_display(db: f32) -> f32 {
        db.clamp(-96.0, 12.0)
    }
}

// ── Panning ─────────────────────────────────────────────────────────────────

/// Pan law implementations.
pub struct Panning;

impl Panning {
    /// Constant-power stereo panning.
    ///
    /// * `pan` ∈ [-1.0, 1.0]
    /// * Returns `(left_gain, right_gain)`.
    pub fn constant_power(pan: f32) -> (f32, f32) {
        let angle = (pan + 1.0) * std::f32::consts::PI / 4.0;
        (angle.cos(), angle.sin())
    }

    /// Linear panning (amplitude drops to 0.5 at centre; not recommended
    /// for music but acceptable for one-shot SFX in games).
    pub fn linear(pan: f32) -> (f32, f32) {
        let l = (1.0 - pan) * 0.5;
        let r = (1.0 + pan) * 0.5;
        (l, r)
    }

    /// Apply pan to a frame.
    pub fn apply(frame: Frame, pan: f32) -> Frame {
        let (lg, rg) = Self::constant_power(pan);
        frame.scale_lr(lg, rg)
    }
}

// ── Sample-rate helpers ─────────────────────────────────────────────────────

/// Convert number of samples to duration at a given sample rate.
pub fn samples_to_duration(samples: usize, sample_rate: u32) -> Duration {
    let secs = samples as f32 / sample_rate as f32;
    Duration::from_secs_f32(secs)
}

/// Convert duration to number of samples at a given sample rate.
pub fn duration_to_samples(duration: Duration, sample_rate: u32) -> usize {
    (duration.as_secs_f32() * sample_rate as f32) as usize
}
