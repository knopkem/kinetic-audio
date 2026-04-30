//! Audio format decoders.
//!
//! Currently supports WAV via `hound`.
//! OGG / MP3 / FLAC via symphonia are behind the `"symphonia"` feature flag.

pub mod wav;

pub use self::wav::decode_wav;

/// Generic decode error.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// WAV decoding error.
    #[error("wav decode error: {0}")]
    Wav(#[from] hound::Error),
    /// Unsupported format.
    #[error("unsupported format: {0}")]
    Unsupported(String),
    /// Decoder-specific error.
    #[error("decode failed: {0}")]
    Other(String),
}

/// Decode any supported format from a byte slice.
///
/// Returns `(interleaved stereo f32 frames, sample_rate)`.
pub fn decode(bytes: &[u8], _hint: &str) -> Result<(Vec<crate::math::Frame>, u32), DecodeError> {
    if _hint.ends_with("wav") || _hint.ends_with("wave") {
        return decode_wav(bytes);
    }

    Err(DecodeError::Unsupported(
        "no decoder available for this format".into(),
    ))
}
