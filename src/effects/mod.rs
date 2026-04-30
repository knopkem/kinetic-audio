//! Audio effect chain.
//!
//! Effects are applied per-voice or per-bus. Built-in effects include
//! Biquad filters (EQ), Delay, and a placeholder for ConvolutionReverb.

pub mod biquad;
pub mod delay;

use crate::math::Frame;

/// A single DSP effect.
pub trait Effect: Send {
    /// Human-readable name for debugging.
    fn name(&self) -> &str;

    /// Process one frame.
    ///
    /// * `input`  – input buffer (in-place or replaced)
    /// * `output` – destination buffer (same length as `input`)
    /// * `rate`   – sample rate (Hz)
    fn process(&mut self, input: &mut [Frame], rate: u32);

    /// Reset DSP state (for seamless loop transitions).
    fn reset(&mut self) {}
}

// ── Re-export built-ins ----------------------------------------------------

pub use biquad::{BiquadFilter, FilterMode, FilterSlope};
pub use delay::DelayLine;
