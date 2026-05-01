//! `kinetic-audio` — Cross-platform game audio engine.
//!
//! Runs natively on desktop (via `cpal`) and in the browser
//! (via the Web Audio API). A unified API drives both.
//!
//! ```ignore
//! use kinetic_audio::{AudioManager, DefaultBackend, AudioConfig};
//!
//! let mut manager = AudioManager::<DefaultBackend>::new(AudioConfig::default())?;
//! let sound = manager.load_sound(include_bytes!("gunshot.wav"), "wav")?;
//! let handle = manager.play(sound, Default::default())?;
//! ```

#![warn(missing_docs)]

pub mod backend;
pub mod decode;
pub mod effects;
pub mod math;
pub mod mixer;
pub mod spatial;
pub mod sprite;
pub mod tween;

mod manager;
mod sound;

// Re-export primary user-facing types.
pub use backend::{AudioError, Backend, BufferHandle, VoiceId, VoiceParam, VoiceSettings};
pub use effects::{biquad::BiquadFilter, delay::DelayLine, Effect};
pub use manager::{AudioConfig, AudioManager};
pub use math::{Decibels, Frame, Panning};
pub use mixer::{BusHandle, MixSettings, TrackHandle};
pub use sound::{PlaybackSettings, SoundData, SoundHandle, SoundKey};
pub use spatial::{DistanceModel, Listener, SpatialSettings};
pub use sprite::{SpriteData, SpriteHandle, SpriteKey, SpriteRegion};
pub use tween::{Easing, Tween};

// Convenience alias: picks the correct backend for the current target.
#[cfg(not(target_arch = "wasm32"))]
/// Native desktop backend (cpal).
pub type DefaultBackend = backend::cpal::CpalBackend;

#[cfg(target_arch = "wasm32")]
/// Browser backend (Web Audio API).
pub type DefaultBackend = backend::web::WebAudioBackend;
