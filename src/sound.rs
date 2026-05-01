//! Sound data and playback handles.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::{BufferHandle, VoiceId};
use crate::math::Frame;
use crate::spatial::{SpatialSettings, Vec3};
use crate::tween::Tween;

/// Immutable, shareable decoded audio buffer.
#[derive(Clone, Debug)]
pub struct SoundData {
    /// Backend-specific buffer handle (allocated on first upload).
    pub(crate) buffer: BufferHandle,
    /// Interleaved stereo f32 samples.
    pub(crate) samples: Arc<Vec<Frame>>,
    /// Sample rate at which the buffer should play back.
    pub(crate) sample_rate: u32,
    /// Duration.
    pub(crate) duration: Duration,
    /// Number of discrete channels in the source file (1 or 2 typically).
    pub(crate) channels: u16,
}

impl SoundData {
    /// Access the raw decoded frames.
    pub fn samples(&self) -> &Arc<Vec<Frame>> {
        &self.samples
    }

    /// Sample count per channel.
    pub fn len_samples(&self) -> usize {
        self.samples.len()
    }

    /// Playback duration at 1.0 rate.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Number of source channels before the backend normalizes to stereo frames.
    pub fn channels(&self) -> u16 {
        self.channels
    }
}

// ── Sound Key ───────────────────────────────────────────────────────────────

/// Opaque key referencing a [`SoundData`] inside an [`AudioManager`].
pub type SoundKey = slotmap::DefaultKey;

// ── Playback Settings ───────────────────────────────────────────────────────

/// Settings applied when calling `AudioManager::play()`.
#[derive(Clone, Debug)]
pub struct PlaybackSettings {
    /// Linear gain (0.0 = silent, 1.0 = original amplitude).
    pub volume: f32,
    /// Stereo pan (-1.0 left .. 1.0 right).
    pub pan: f32,
    /// Playback rate multiplier (0.5 = half speed, 2.0 = double).
    pub rate: f32,
    /// Whether the sound should loop.
    pub looped: bool,
    /// Delay before playback starts.
    pub delay: Duration,
    /// Full spatial playback settings. Preferred over the legacy `position` field.
    pub spatial: Option<SpatialSettings>,
    /// World-space position for spatial audio.
    ///
    /// Deprecated compatibility shortcut for `spatial = Some(SpatialSettings { position, ..Default::default() })`.
    pub position: Option<Vec3>,
    /// If specified the sound is panned but routed to this bus.
    pub track: Option<slotmap::DefaultKey>,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.0,
            rate: 1.0,
            looped: false,
            delay: Duration::ZERO,
            spatial: None,
            position: None,
            track: None,
        }
    }
}

// ── Sound Handle ────────────────────────────────────────────────────────────

/// A live handle to a single playing voice.
///
/// Dropping the handle does **not** stop playback automatically.
/// Use `.stop()` explicitly or let the sound finish naturally.
pub struct SoundHandle {
    pub(crate) voice: VoiceId,
    pub(crate) manager: WeakHandleBridge,
    /// Lazily cached current volume.
    pub(crate) volume: f32,
    /// Lazily cached current pan.
    pub(crate) pan: f32,
    /// Lazily cached current rate.
    pub(crate) rate: f32,
}

/// A thread-safe bridge so `SoundHandle` can send commands back to the
/// `AudioManager` without owning it.  In practice this is a thin wrapper
/// around `mpsc::Sender` (native) or a JS function handle (WASM).
///
/// We forward-declare the real type in `manager.rs`.
pub(crate) enum WeakHandleBridge {
    #[cfg(not(target_arch = "wasm32"))]
    Native(std::sync::mpsc::Sender<ManagerCommand>),
    #[cfg(target_arch = "wasm32")]
    Web(crate::backend::web::WebCommandQueue),
}

/// Commands that can be sent from a `SoundHandle` to the `AudioManager`.
#[derive(Clone, Debug)]
pub(crate) enum ManagerCommand {
    SetVolume(VoiceId, f32),
    SetPan(VoiceId, f32),
    SetRate(VoiceId, f32),
    SetPosition(VoiceId, Option<Vec3>),
    FadeVolume(VoiceId, f32, Tween),
    FadePan(VoiceId, f32, Tween),
    StopAfterLoop(VoiceId),
    FadeOut(VoiceId, Duration),
    Stop(VoiceId),
}

impl SoundHandle {
    /// Backend voice id for this handle.
    pub fn id(&self) -> VoiceId {
        self.voice
    }

    /// Stop the voice immediately.
    pub fn stop(self) {
        self.send_cmd(ManagerCommand::Stop(self.voice));
    }

    /// Set linear volume instantly.
    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 2.0);
        self.send_cmd(ManagerCommand::SetVolume(self.voice, self.volume));
    }

    /// Set volume with a smooth tween.
    pub fn fade_volume(&mut self, target: f32, tween: Tween) {
        self.send_cmd(ManagerCommand::FadeVolume(
            self.voice,
            target.clamp(0.0, 2.0),
            tween,
        ));
    }

    /// Set stereo pan instantly.
    pub fn set_pan(&mut self, pan: f32) {
        self.pan = pan.clamp(-1.0, 1.0);
        self.send_cmd(ManagerCommand::SetPan(self.voice, self.pan));
    }

    /// Set pan with a smooth tween.
    pub fn fade_pan(&mut self, target: f32, tween: Tween) {
        self.send_cmd(ManagerCommand::FadePan(
            self.voice,
            target.clamp(-1.0, 1.0),
            tween,
        ));
    }

    /// Set playback rate instantly.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.clamp(0.1, 4.0);
        self.send_cmd(ManagerCommand::SetRate(self.voice, self.rate));
    }

    /// Move the sound in 3D space.
    pub fn set_position(&mut self, pos: Vec3) {
        self.send_cmd(ManagerCommand::SetPosition(self.voice, Some(pos)));
    }

    /// Clear the 3D position (reverts to 2D pan).
    pub fn clear_position(&mut self) {
        self.send_cmd(ManagerCommand::SetPosition(self.voice, None));
    }

    /// Stop after the current loop completes (no-op for non-looping sounds).
    pub fn stop_after_loop(&self) {
        self.send_cmd(ManagerCommand::StopAfterLoop(self.voice));
    }

    /// Fade out over `duration` then stop.
    pub fn fade_out(&self, duration: Duration) {
        self.send_cmd(ManagerCommand::FadeOut(self.voice, duration));
    }

    #[inline]
    fn send_cmd(&self, cmd: ManagerCommand) {
        match &self.manager {
            #[cfg(not(target_arch = "wasm32"))]
            WeakHandleBridge::Native(tx) => {
                let _ = tx.send(cmd);
            }
            #[cfg(target_arch = "wasm32")]
            WeakHandleBridge::Web(q) => {
                q.push(cmd);
            }
        }
    }
}
