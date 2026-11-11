//! Band-limited sample-rate conversion for audio uploaded to a backend.
//!
//! Backends mix at the output device's sample rate. Uploading a buffer whose
//! rate differs from the device would otherwise change its pitch and duration,
//! so [`AudioManager`](crate::manager::AudioManager) converts buffers to the
//! device rate at load time with this module. Conversion happens once per load
//! and never in the audio callback.
//!
//! The interpolator is a windowed-sinc (Blackman window) evaluated through a
//! small phase table, which keeps the per-sample work to a handful of
//! multiply-accumulates while still rejecting energy above the destination
//! Nyquist frequency.

use crate::math::Frame;

/// Interpolation taps per side of the source position.
const HALF_TAPS: usize = 16;
/// Total taps in the interpolation kernel.
const TAPS: usize = 2 * HALF_TAPS;
/// Number of fractional phases precomputed in the kernel table.
const PHASES: usize = 64;

/// Convert `frames` from `src_rate` to `dst_rate` Hz.
///
/// Both rates must be non-zero; a zero rate is treated as "no conversion" and
/// the input is returned unchanged (with a warning), because there is no
/// meaningful ratio to interpolate with.
///
/// Upsampling interpolates between source frames. Downsampling low-pass filters
/// first, so content above the destination Nyquist frequency is attenuated
/// instead of folding back into the audible band.
pub fn resample_frames(frames: &[Frame], src_rate: u32, dst_rate: u32) -> Vec<Frame> {
    if frames.is_empty() {
        return Vec::new();
    }
    if src_rate == 0 || dst_rate == 0 {
        log::warn!("kinetic-audio: refusing to resample with rate {src_rate} -> {dst_rate}");
        return frames.to_vec();
    }
    if src_rate == dst_rate {
        return frames.to_vec();
    }

    let kernel = build_kernel(dst_rate as f64 / src_rate as f64);
    let step = src_rate as f64 / dst_rate as f64; // Source frames per output frame.
    let out_len = ((frames.len() - 1) as f64 / step).floor() as usize + 1;
    let last_index = frames.len() - 1;

    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let position = n as f64 * step;
        let center = position.floor() as i64;
        let phase = ((position - center as f64) * PHASES as f64) as usize;
        let weights = &kernel[phase.min(PHASES - 1)];
        let first = center - (HALF_TAPS - 1) as i64;

        let mut l = 0.0f64;
        let mut r = 0.0f64;
        for (k, &weight) in weights.iter().enumerate() {
            let index = (first + k as i64).clamp(0, last_index as i64) as usize;
            l += frames[index].l as f64 * weight;
            r += frames[index].r as f64 * weight;
        }
        out.push(Frame {
            l: l as f32,
            r: r as f32,
        });
    }
    out
}

/// Build the phase table for a conversion to `bandwidth` times the source
/// Nyquist. `bandwidth` is below 1.0 when downsampling, where the low-pass is
/// needed to prevent aliasing.
fn build_kernel(bandwidth: f64) -> Vec<[f64; TAPS]> {
    let bandwidth = bandwidth.min(1.0);
    let mut kernel = Vec::with_capacity(PHASES);
    for phase in 0..PHASES {
        let t = phase as f64 / PHASES as f64;
        let mut weights = [0.0f64; TAPS];
        for (k, weight) in weights.iter_mut().enumerate() {
            // Distance, in source frames, from the output position to this tap.
            let d = t + (HALF_TAPS - 1) as f64 - k as f64;
            let window = blackman(d / HALF_TAPS as f64);
            *weight = window * sinc(d * bandwidth) * bandwidth;
        }
        // Unity DC gain keeps offsets and slow envelopes from drifting.
        let sum: f64 = weights.iter().sum();
        if sum.abs() > f64::EPSILON {
            for weight in weights.iter_mut() {
                *weight /= sum;
            }
        }
        kernel.push(weights);
    }
    kernel
}

/// Blackman window over `x` in (-1, 1).
fn blackman(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        return 0.0;
    }
    0.42 + 0.5 * (std::f64::consts::PI * x).cos() + 0.08 * (2.0 * std::f64::consts::PI * x).cos()
}

/// Normalised sinc: `sin(pi x) / (pi x)`, with `sinc(0) == 1`.
fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let px = std::f64::consts::PI * x;
    px.sin() / px
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure tone sampled at `rate`, in [-1, 1].
    fn sine(samples: usize, rate: f64, frequency: f64) -> Vec<Frame> {
        (0..samples)
            .map(|n| {
                let value = (2.0 * std::f64::consts::PI * frequency * n as f64 / rate).sin() as f32;
                Frame::mono(value)
            })
            .collect()
    }

    /// Peak absolute value over the middle 80% of the buffer, ignoring edges
    /// where the finite kernel ramps up.
    fn mid_peak(frames: &[Frame]) -> f32 {
        let start = frames.len() / 10;
        let end = frames.len() - frames.len() / 10;
        frames[start..end]
            .iter()
            .map(|f| f.l.abs().max(f.r.abs()))
            .fold(0.0f32, f32::max)
    }

    /// RMS over the middle 80% of the buffer.
    fn mid_rms(frames: &[Frame]) -> f32 {
        let start = frames.len() / 10;
        let end = frames.len() - frames.len() / 10;
        let sum: f64 = frames[start..end]
            .iter()
            .map(|f| (f.l as f64) * (f.l as f64))
            .sum();
        (sum / (end - start) as f64).sqrt() as f32
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(resample_frames(&[], 44_100, 48_000).is_empty());
    }

    #[test]
    fn same_rate_round_trips_exactly() {
        let input = sine(1_000, 44_100.0, 440.0);
        assert_eq!(resample_frames(&input, 44_100, 44_100), input);
    }

    #[test]
    fn zero_rate_is_not_converted() {
        let input = sine(100, 44_100.0, 440.0);
        assert_eq!(resample_frames(&input, 0, 48_000), input);
        assert_eq!(resample_frames(&input, 44_100, 0), input);
    }

    #[test]
    fn output_length_tracks_the_rate_ratio() {
        let input = sine(44_100, 44_100.0, 440.0);
        let up = resample_frames(&input, 44_100, 48_000);
        let down = resample_frames(&input, 44_100, 22_050);
        assert!(
            up.len() + 1 >= 48_000 && up.len() <= 48_000 + 1,
            "up {}",
            up.len()
        );
        assert!(
            down.len() + 1 >= 22_050 && down.len() <= 22_050 + 1,
            "down {}",
            down.len()
        );
    }

    #[test]
    fn upsample_preserves_tone_amplitude() {
        let input = sine(44_100, 44_100.0, 1_000.0);
        let up = resample_frames(&input, 44_100, 48_000);
        let peak = mid_peak(&up);
        assert!(
            (peak - 1.0).abs() < 0.01,
            "expected ~1.0 peak after upsampling, got {peak}"
        );
    }

    #[test]
    fn upsample_does_not_change_duration_seconds() {
        let input = sine(44_100, 44_100.0, 440.0);
        let up = resample_frames(&input, 44_100, 48_000);
        let before = input.len() as f64 / 44_100.0;
        let after = up.len() as f64 / 48_000.0;
        assert!((before - after).abs() < 0.001, "{before} vs {after}");
    }

    #[test]
    fn downsample_attenuates_content_above_the_destination_nyquist() {
        // 30 kHz at 96 kHz sits above the 22.05 kHz destination Nyquist, so a
        // naive decimator would fold it down to 6 kHz at full level.
        let input = sine(96_000, 96_000.0, 30_000.0);
        let down = resample_frames(&input, 96_000, 44_100);
        let rms = mid_rms(&down);
        assert!(rms < 0.01, "aliasing not suppressed, rms {rms}");
    }

    #[test]
    fn downsample_preserves_content_below_the_destination_nyquist() {
        let input = sine(96_000, 96_000.0, 3_000.0);
        let down = resample_frames(&input, 96_000, 44_100);
        let peak = mid_peak(&down);
        assert!((peak - 1.0).abs() < 0.02, "expected ~1.0 peak, got {peak}");
    }

    #[test]
    fn silence_stays_silent() {
        let input = vec![Frame::SILENCE; 5_000];
        let down = resample_frames(&input, 48_000, 44_100);
        assert!(down.iter().all(|f| f.l == 0.0 && f.r == 0.0));
    }

    #[test]
    fn dc_offset_is_not_amplified() {
        let input = vec![Frame::mono(0.5); 5_000];
        let up = resample_frames(&input, 44_100, 48_000);
        let start = up.len() / 10;
        let end = up.len() - up.len() / 10;
        assert!(
            up[start..end]
                .iter()
                .all(|f| (f.l - 0.5).abs() < 1e-3 && (f.r - 0.5).abs() < 1e-3),
            "DC gain drifted away from unity"
        );
    }

    #[test]
    fn balanced_input_stays_balanced() {
        let input = sine(5_000, 44_100.0, 440.0);
        let up = resample_frames(&input, 44_100, 48_000);
        assert!(up.iter().all(|f| (f.l - f.r).abs() < 1e-6));

        let panned: Vec<Frame> = input.iter().map(|f| Frame { l: f.l, r: 0.0 }).collect();
        let up_panned = resample_frames(&panned, 44_100, 48_000);
        assert!(up_panned.iter().all(|f| f.r.abs() < 1e-6));
    }
}
