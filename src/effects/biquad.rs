//! Second-order IIR (biquad) filter.
//!
//! Useful for per-voice low-pass (distant occlusion), high-pass (radio),
//! band-pass, shelving EQ, etc.

use crate::effects::Effect;
use crate::math::Frame;

/// Filter topology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    /// Low-pass: allows frequencies below `cutoff`.
    LowPass,
    /// High-pass: allows frequencies above `cutoff`.
    HighPass,
    /// Band-pass: allows a band around `cutoff`.
    BandPass,
    /// Notch: attenuates a band around `cutoff`.
    Notch,
    /// Low-shelf: boosts or cuts low frequencies.
    LowShelf,
    /// High-shelf: boosts or cuts high frequencies.
    HighShelf,
    /// Peak bell: boost or cut around `cutoff`.
    Peak,
}

/// Filter slope / order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterSlope {
    /// 12 dB/octave (2-pole).
    Db12,
    /// 24 dB/octave (4-pole, cascaded biquads).
    Db24,
}

/// A biquad filter stage.
///
/// Transfer function:
/// ```text
/// y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
/// ```
#[derive(Clone, Debug)]
pub struct BiquadFilter {
    /// Filter topology.
    pub mode: FilterMode,
    /// Filter order / slope approximation.
    pub slope: FilterSlope,
    /// Cutoff or center frequency in Hz, depending on the filter mode.
    pub cutoff_hz: f32,
    /// Resonance / Q.
    pub resonance: f32,
    /// Shelf / peak gain in decibels.
    pub gain_db: f32,
    /// Processing sample rate in Hz.
    pub sample_rate: u32,
    // Coefficients.
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    // State (stereo).
    z1: Frame,
    z2: Frame,
}

impl BiquadFilter {
    /// Create a new filter. Call `recalc_coefficients()` after changing parameters.
    pub fn new(mode: FilterMode, sample_rate: u32) -> Self {
        Self {
            mode,
            slope: FilterSlope::Db12,
            cutoff_hz: 1000.0,
            resonance: 0.707,
            gain_db: 0.0,
            sample_rate,
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: Frame::SILENCE,
            z2: Frame::SILENCE,
        }
    }

    /// Recompute coefficients after parameter changes.
    pub fn recalc_coefficients(&mut self) {
        let sr = self.sample_rate as f32;
        let f0 = self.cutoff_hz.clamp(20.0, sr * 0.49);
        let q = self.resonance.max(0.01);
        let a = if self.gain_db.abs() > 0.001 {
            10.0_f32.powf(self.gain_db / 40.0)
        } else {
            1.0
        };

        let w0 = 2.0 * std::f32::consts::PI * f0 / sr;
        let cosw = w0.cos();
        let sinw = w0.sin();
        let alpha = sinw / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match self.mode {
            FilterMode::LowPass => {
                let b1 = 1.0 - cosw;
                let b0 = b1 / 2.0;
                let b2 = b0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cosw;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterMode::HighPass => {
                let b1 = -(1.0 + cosw);
                let b0 = (1.0 + cosw) / 2.0;
                let b2 = b0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cosw;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterMode::BandPass => {
                let b0 = alpha;
                let b1 = 0.0;
                let b2 = -alpha;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cosw;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterMode::Notch => {
                let b0 = 1.0;
                let b1 = -2.0 * cosw;
                let b2 = 1.0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cosw;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterMode::Peak => {
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cosw;
                let b2 = 1.0 - alpha * a;
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cosw;
                let a2 = 1.0 - alpha / a;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterMode::LowShelf => {
                let sqrt2a = 2.0 * a.sqrt() * alpha;
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                let b0 = a * (ap1 - am1 * cosw + sqrt2a);
                let b1 = 2.0 * a * (am1 - ap1 * cosw);
                let b2 = a * (ap1 - am1 * cosw - sqrt2a);
                let a0 = ap1 + am1 * cosw + sqrt2a;
                let a1 = -2.0 * (am1 + ap1 * cosw);
                let a2 = ap1 + am1 * cosw - sqrt2a;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterMode::HighShelf => {
                let sqrt2a = 2.0 * a.sqrt() * alpha;
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                let b0 = a * (ap1 + am1 * cosw + sqrt2a);
                let b1 = -2.0 * a * (am1 + ap1 * cosw);
                let b2 = a * (ap1 + am1 * cosw - sqrt2a);
                let a0 = ap1 - am1 * cosw + sqrt2a;
                let a1 = 2.0 * (am1 - ap1 * cosw);
                let a2 = ap1 - am1 * cosw - sqrt2a;
                (b0, b1, b2, a0, a1, a2)
            }
        };

        // Normalise by a0.
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    /// Process a single frame.
    fn tick(&mut self, input: Frame) -> Frame {
        let out = input + self.z1.scale(self.b1) + self.z2.scale(self.b2)
            - self.z1.scale(self.a1)
            - self.z2.scale(self.a2);

        // Shift state.
        self.z2 = self.z1;
        self.z1 = out;
        out.scale(self.b0)
    }
}

impl Effect for BiquadFilter {
    fn name(&self) -> &str {
        "BiquadFilter"
    }

    fn process(&mut self, input: &mut [Frame], rate: u32) {
        if rate != self.sample_rate {
            self.sample_rate = rate;
            self.recalc_coefficients();
        }
        for frame in input.iter_mut() {
            *frame = self.tick(*frame);
        }
    }

    fn reset(&mut self) {
        self.z1 = Frame::SILENCE;
        self.z2 = Frame::SILENCE;
    }
}
