//! Audio backend abstraction.
//!
//! Each platform provides an implementation of the [`Backend`] trait.
//!
//! - **Native:** [`CpalBackend`](cpal::CpalBackend)
//! - **WASM:** [`WebAudioBackend`](web::WebAudioBackend)
//! - **Testing:** [`NullBackend`](null::NullBackend)

pub mod null;

#[cfg(not(target_arch = "wasm32"))]
pub mod cpal;

#[cfg(target_arch = "wasm32")]
pub mod web;

use std::sync::Arc;
use std::time::Duration;

use crate::math::Frame;
use crate::spatial::{Listener, SpatialSettings, Vec3};

/// Opaque handle for a decoded buffer that has been uploaded to a backend.
pub type BufferHandle = slotmap::DefaultKey;

/// Opaque handle for a single playing voice.
pub type VoiceId = slotmap::DefaultKey;

/// Opaque handle for a mixer bus.
pub type BusId = slotmap::DefaultKey;

/// Opaque handle for a mixer track.
pub type TrackId = slotmap::DefaultKey;

/// Parameter that can be changed on a running voice.
#[derive(Clone, Debug)]
pub enum VoiceParam {
    /// Linear gain (0.0 .. ∞).
    Volume(f32),
    /// Stereo pan (-1.0 left .. 1.0 right).
    Pan(f32),
    /// Playback rate (0.5 .. 4.0).
    Rate(f32),
    /// Stop the voice at the end of the current loop (if looping).
    StopAfterLoop,
    /// Immediately fade out and stop.
    FadeOut(Duration),
    /// 3D world-space position for spatial audio.
    Position(Vec3),
}

/// Settings used when a voice is first created.
#[derive(Clone, Debug)]
pub struct VoiceSettings {
    /// Initial linear amplitude.
    pub volume: f32,
    /// Initial stereo pan.
    pub pan: f32,
    /// Initial playback rate.
    pub rate: f32,
    /// Whether to loop the sound.
    pub looped: bool,
    /// Which bus the voice should be routed to.
    pub bus: Option<BusId>,
    /// 3D spatial configuration (backend may ignore if unsupported).
    pub spatial: Option<SpatialSettings>,
    /// First sample frame to play (inclusive). `None` means 0.
    pub start_sample: Option<usize>,
    /// Last sample frame to play (exclusive). `None` means end-of-buffer.
    pub end_sample: Option<usize>,
    /// Silence to render before playback begins (in sample frames).
    pub delay_samples: usize,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.0,
            rate: 1.0,
            looped: false,
            bus: None,
            spatial: None,
            start_sample: None,
            end_sample: None,
            delay_samples: 0,
        }
    }
}

/// Configuration for the audio output device.
#[derive(Clone, Debug)]
pub struct DeviceConfig {
    /// Preferred sample rate (Hz). Zero lets the backend pick.
    pub sample_rate: u32,
    /// Preferred buffer size in samples (latency vs. CPU tradeoff).
    pub buffer_size: u32,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            sample_rate: 0,
            buffer_size: 512,
        }
    }
}

/// Low-level audio output contract.
///
/// Implementors are responsible for talking to the OS / browser and
/// playing back a mixed stereo stream.
pub trait Backend {
    /// Start the audio device. Called exactly once.
    fn start(config: DeviceConfig) -> Result<Self, AudioError>
    where
        Self: Sized;

    /// Gracefully stop the device and release resources.
    fn stop(&mut self);

    /// Upload decoded samples to the backend.
    fn upload_buffer(&mut self, samples: Arc<Vec<Frame>>, rate: u32) -> BufferHandle;

    /// Play a buffer. Returns a [`VoiceId`] for later control.
    fn play(
        &mut self,
        buffer: BufferHandle,
        settings: VoiceSettings,
    ) -> Result<VoiceId, AudioError>;

    /// Change a parameter on a running voice.
    fn set_param(&mut self, voice: VoiceId, param: VoiceParam);

    /// Stop a voice (it becomes eligible for reuse).
    fn stop_voice(&mut self, voice: VoiceId);

    /// Report voices that have naturally finished this frame.
    fn finished_voices(&mut self) -> Vec<VoiceId>;

    /// Advance time by `dt`.
    ///
    /// For native backends this is a no-op (the audio thread runs
    /// independently). For WASM it is the only opportunity to push
    /// parameter changes into the JS node graph.
    fn tick(&mut self, _dt: Duration) {}

    /// Update the backend listener state used for spatial audio.
    fn set_listener(&mut self, _listener: Listener) {}

    /// Resume audio playback if the backend supports explicit lifecycle control.
    fn resume(&mut self) -> Result<(), AudioError> {
        Ok(())
    }

    /// Current output sample rate.
    fn sample_rate(&self) -> u32;

    // ── Bus management (optional — default no-ops for WASM / Null) ──────────

    /// Register a new mixer bus so the backend can route voices to it.
    fn register_bus(&mut self, _id: BusId) {}

    /// Unregister a bus.
    fn unregister_bus(&mut self, _id: BusId) {}

    /// Update bus gain and mute state.
    fn set_bus_config(&mut self, _id: BusId, _gain: f32, _muted: bool) {}

    /// Append a DSP effect to a bus's effect chain.
    fn add_bus_effect(&mut self, _id: BusId, _effect: Box<dyn crate::effects::Effect + Send>) {}
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Top-level audio error.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The audio device could not be opened.
    #[error("device unavailable: {0}")]
    DeviceUnavailable(String),
    /// Decoding failed.
    #[error("decode error: {0}")]
    Decode(String),
    /// A voice could not be allocated.
    #[error("voice limit reached")]
    VoiceLimit,
    /// Invalid handle used.
    #[error("invalid handle")]
    InvalidHandle,
    /// Platform-specific backend error.
    #[error("backend error: {0}")]
    Backend(String),
}
