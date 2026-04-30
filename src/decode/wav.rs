//! WAV decoder via `hound`.

use crate::decode::DecodeError;
use crate::math::Frame;

/// Decode a WAV file to interleaved stereo `Frame`s.
pub fn decode_wav(bytes: &[u8]) -> Result<(Vec<Frame>, u32), DecodeError> {
    use hound::WavReader;
    use std::io::Cursor;

    let reader = WavReader::new(Cursor::new(bytes))?;
    let spec = reader.spec();
    let rate = spec.sample_rate;

    let mut frames = Vec::new();
    let mut samples: Vec<f32> = Vec::new();

    match (spec.bits_per_sample, spec.sample_format) {
        (16, hound::SampleFormat::Int) => {
            for s in reader.into_samples::<i16>() {
                samples.push(s? as f32 / i16::MAX as f32);
            }
        }
        (24, hound::SampleFormat::Int) => {
            // hound reads 24-bit as i32 scaled to full 32-bit range.
            for s in reader.into_samples::<i32>() {
                samples.push(s? as f32 / 8388608.0); // 2^23
            }
        }
        (32, hound::SampleFormat::Float) => {
            for s in reader.into_samples::<f32>() {
                samples.push(s?);
            }
        }
        (bits, fmt) => {
            return Err(DecodeError::Unsupported(format!(
                "WAV {}-bit {:?}",
                bits, fmt
            )));
        }
    }

    // Convert to interleaved stereo Frame.
    match spec.channels {
        1 => {
            // Duplicate mono to stereo.
            for s in samples {
                frames.push(Frame::mono(s));
            }
        }
        2 => {
            for chunk in samples.chunks_exact(2) {
                frames.push(Frame {
                    l: chunk[0],
                    r: chunk[1],
                });
            }
        }
        _ => {
            // Downmix first two channels only.
            for chunk in samples.chunks_exact(spec.channels as usize) {
                let l = chunk[0];
                let r = chunk.get(1).copied().unwrap_or(l);
                let m = (l + r) * 0.5;
                frames.push(Frame::mono(m));
            }
        }
    }

    Ok((frames, rate))
}
