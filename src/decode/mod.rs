//! Audio format decoders.
//!
//! Currently supports WAV via `hound`.
//! OGG Vorbis, MP3, FLAC, and more are available via the `"symphonia"` feature flag.

pub mod wav;

#[cfg(feature = "symphonia")]
pub mod symphonia;

#[cfg(feature = "symphonia")]
pub use self::symphonia::decode_symphonia;
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
/// `hint` should be the file extension (e.g. `"wav"`, `"ogg"`, `"mp3"`).
/// When the `"symphonia"` feature is enabled, all formats symphonia
/// supports are tried automatically for non-WAV extensions.
pub fn decode(bytes: &[u8], hint: &str) -> Result<(Vec<crate::math::Frame>, u32), DecodeError> {
    let ext = hint.trim_start_matches('.');

    if ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("wave") {
        return decode_wav(bytes);
    }

    #[cfg(feature = "symphonia")]
    {
        decode_symphonia(bytes)
    }

    #[cfg(not(feature = "symphonia"))]
    Err(DecodeError::Unsupported(format!(
        "no decoder available for '{}' — enable the \"symphonia\" feature for OGG/MP3/FLAC",
        ext
    )))
}
