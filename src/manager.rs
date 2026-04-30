//! Main control surface: `AudioManager`.
//!
//! This is the only type most users interact with directly.

use std::sync::Arc;
use std::time::Duration;

use slotmap::SlotMap;

#[cfg(target_arch = "wasm32")]
use crate::backend::web::WebCommandQueue;
use crate::backend::{Backend, BusId, TrackId, VoiceParam, VoiceSettings};
#[cfg(not(target_arch = "wasm32"))]
use crate::math::Panning;
use crate::math::{samples_to_duration, Frame};
use crate::mixer::{Bus, MixSettings};
use crate::sound::{
    ManagerCommand, PlaybackSettings, SoundData, SoundHandle, SoundKey, WeakHandleBridge,
};
use crate::spatial::{Listener, SpatialSettings};

/// Configuration passed to `AudioManager::new()`.
#[derive(Clone, Debug)]
pub struct AudioConfig {
    /// Preferred sample rate (Hz). Zero lets the backend pick.
    pub sample_rate: u32,
    /// Maximum concurrent voices.
    pub max_voices: usize,
    /// Preferred output buffer size (latency vs. CPU).
    pub buffer_size: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100, // Match common source material; cpal will pick closest supported.
            max_voices: 256,
            buffer_size: 512,
        }
    }
}

/// Top-level audio control.
///
/// * `B` — the platform-specific [`Backend`] (use [`DefaultBackend`](crate::DefaultBackend)).
pub struct AudioManager<B: Backend> {
    backend: B,
    sounds: SlotMap<SoundKey, SoundData>,
    buses: SlotMap<BusId, Bus>,
    // Master listener.
    listener: Listener,
    // Command channel (native) or shared queue (WASM).
    #[cfg(not(target_arch = "wasm32"))]
    command_rx: std::sync::mpsc::Receiver<ManagerCommand>,
    #[cfg(not(target_arch = "wasm32"))]
    command_tx: std::sync::mpsc::Sender<ManagerCommand>,
    #[cfg(target_arch = "wasm32")]
    command_queue_web: WebCommandQueue,
}

impl<B: Backend> AudioManager<B> {
    /// Start the audio device and create the manager.
    pub fn new(config: AudioConfig) -> Result<Self, crate::backend::AudioError> {
        let backend = B::start(crate::backend::DeviceConfig {
            sample_rate: config.sample_rate,
            buffer_size: config.buffer_size,
        })?;

        let mut buses = SlotMap::with_key();
        let master_id = buses.insert(Bus {
            id: BusId::default(), // placeholder, fixed below
            settings: MixSettings {
                name: "master".into(),
                ..Default::default()
            },
            effects: Vec::new(),
            input: Vec::new(),
        });
        buses[master_id].id = master_id;

        #[cfg(not(target_arch = "wasm32"))]
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        let listener = Listener::default();
        let mut manager = Self {
            backend,
            sounds: SlotMap::with_key(),
            buses,
            listener,
            #[cfg(not(target_arch = "wasm32"))]
            command_rx,
            #[cfg(not(target_arch = "wasm32"))]
            command_tx,
            #[cfg(target_arch = "wasm32")]
            command_queue_web: WebCommandQueue::default(),
        };
        manager.backend.set_listener(listener);
        Ok(manager)
    }

    // -------------------------------------------------------------------
    // Loading
    // -------------------------------------------------------------------

    /// Decode a WAV byte slice and return a handle.
    pub fn load_sound(&mut self, bytes: &[u8]) -> Result<SoundKey, crate::backend::AudioError> {
        let (frames, rate) = crate::decode::decode_wav(bytes)
            .map_err(|e| crate::backend::AudioError::Decode(e.to_string()))?;
        let n_frames = frames.len();
        let samples = Arc::new(frames);
        let handle = self.backend.upload_buffer(samples.clone(), rate);
        let key = self.sounds.insert(SoundData {
            buffer: handle,
            samples,
            sample_rate: rate,
            duration: samples_to_duration(n_frames, rate),
            channels: 2, // post-decoder always stereo interleaved
        });
        Ok(key)
    }

    /// Decode a WAV byte slice as a sprite sheet.
    pub fn load_sprite(&mut self, bytes: &[u8]) -> Result<SoundKey, crate::backend::AudioError> {
        // For now sprites reuse the same load path — caller defines regions later.
        self.load_sound(bytes)
    }

    /// Upload pre-decoded interleaved stereo samples directly.
    ///
    /// This bypasses the decoder and is useful for procedural / runtime-generated audio.
    pub fn load_raw(&mut self, samples: Arc<Vec<Frame>>, rate: u32) -> SoundKey {
        let n_samples = samples.len();
        let handle = self.backend.upload_buffer(samples.clone(), rate);
        self.sounds.insert(SoundData {
            buffer: handle,
            samples,
            sample_rate: rate,
            duration: samples_to_duration(n_samples, rate),
            channels: 2,
        })
    }

    // -------------------------------------------------------------------
    // Playback
    // -------------------------------------------------------------------

    /// Play a sound.
    pub fn play(
        &mut self,
        key: SoundKey,
        settings: PlaybackSettings,
    ) -> Result<SoundHandle, crate::backend::AudioError> {
        let sound = self
            .sounds
            .get(key)
            .ok_or(crate::backend::AudioError::InvalidHandle)?;

        let spatial = settings.spatial.or_else(|| {
            settings
                .position
                .map(|pos| crate::spatial::SpatialSettings {
                    position: pos,
                    ..Default::default()
                })
        });
        let (volume, pan) = self.effective_pan_volume(settings.volume, settings.pan, spatial);

        let voice_settings = VoiceSettings {
            volume,
            pan,
            rate: settings.rate,
            looped: settings.looped,
            bus: settings.track.map(|t| self.track_to_bus(t)),
            spatial,
        };

        let voice = self.backend.play(sound.buffer, voice_settings)?;

        let bridge = self.make_bridge();
        let handle = SoundHandle {
            voice,
            manager: bridge,
            volume,
            pan,
            rate: settings.rate,
        };

        Ok(handle)
    }

    // -------------------------------------------------------------------
    // Listener
    // -------------------------------------------------------------------

    /// Set the listener's world-space position.
    pub fn set_listener_position(&mut self, pos: glam::Vec3) {
        self.listener.position = pos;
        self.backend.set_listener(self.listener);
    }

    /// Set the listener's orientation.
    pub fn set_listener_orientation(&mut self, forward: glam::Vec3, up: glam::Vec3) {
        let forward = if forward.length_squared() > 1e-6 {
            forward.normalize()
        } else {
            self.listener.forward
        };
        let up = if up.length_squared() > 1e-6 {
            up.normalize()
        } else {
            self.listener.up
        };
        self.listener.forward = forward;
        self.listener.up = up;
        self.backend.set_listener(self.listener);
    }

    // -------------------------------------------------------------------
    // Buses / Mixer
    // -------------------------------------------------------------------

    /// Add a new sub-master bus.
    pub fn add_bus(&mut self, settings: MixSettings) -> Result<BusId, crate::backend::AudioError> {
        let id = self.buses.insert(Bus {
            id: BusId::default(),
            settings,
            effects: Vec::new(),
            input: Vec::new(),
        });
        self.buses[id].id = id;
        Ok(id)
    }

    /// Remove a bus (voices routed here fall back to master).
    pub fn remove_bus(&mut self, id: BusId) {
        self.buses.remove(id);
    }

    /// Set per-bus gain in decibels.
    pub fn set_bus_volume_db(&mut self, id: BusId, db: f32) {
        if let Some(bus) = self.buses.get_mut(id) {
            bus.settings.gain = crate::math::Decibels::to_linear(db).clamp(0.0, 2.0);
        }
    }

    // -------------------------------------------------------------------
    // Frame update
    // -------------------------------------------------------------------

    /// Must be called once per frame.
    ///
    /// * Native: drains command queue, updates tweens.
    /// * WASM:   pushes parameter changes into the JS node graph.
    pub fn update(&mut self, dt: Duration) {
        // Drain command queue.
        #[cfg(not(target_arch = "wasm32"))]
        {
            while let Ok(cmd) = self.command_rx.try_recv() {
                self.apply_command(cmd);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            for cmd in self.command_queue_web.drain() {
                self.apply_command(cmd);
            }
        }

        // Update tweens... (placeholder)
        let _ = dt;

        self.backend.tick(dt);
        let _ = self.backend.finished_voices();
    }

    /// Ask the backend to resume playback if it is lifecycle-gated.
    pub fn resume(&mut self) -> Result<(), crate::backend::AudioError> {
        self.backend.resume()
    }

    /// Gracefully stop the audio device.
    pub fn shutdown(&mut self) {
        self.backend.stop();
    }

    // -------------------------------------------------------------------
    // Query
    // -------------------------------------------------------------------

    /// Retrieve a loaded sound by key. Returns `None` if the key is invalid.
    pub fn get_sound(&self, key: SoundKey) -> Option<&SoundData> {
        self.sounds.get(key)
    }
    // -------------------------------------------------------------------

    #[inline]
    fn make_bridge(&self) -> WeakHandleBridge {
        #[cfg(not(target_arch = "wasm32"))]
        {
            WeakHandleBridge::Native(self.command_tx.clone())
        }
        #[cfg(target_arch = "wasm32")]
        {
            WeakHandleBridge::Web(self.command_queue_web.clone())
        }
    }

    fn effective_pan_volume(
        &self,
        base_volume: f32,
        base_pan: f32,
        spatial: Option<SpatialSettings>,
    ) -> (f32, f32) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = spatial;
            (base_volume, base_pan)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(spatial) = spatial else {
                return (base_volume, base_pan);
            };
            let delta = spatial.position - self.listener.position;
            let dist = delta.length();
            let gain = spatial
                .model
                .evaluate(
                    dist,
                    spatial.ref_distance.max(0.001),
                    spatial.max_distance.max(spatial.ref_distance.max(0.001)),
                    spatial.rolloff_factor.max(0.0),
                )
                .clamp(0.0, 1.0);
            let dir = delta.normalize_or_zero();
            let pan = if dir.length_squared() > 0.0 {
                let spatial_pan = dir.dot(self.listener.right()).clamp(-1.0, 1.0);
                let (sl, sr) = Panning::constant_power(spatial_pan);
                let (bl, br) = Panning::constant_power(base_pan);
                let mixed_l = sl * bl;
                let mixed_r = sr * br;
                (mixed_r - mixed_l).clamp(-1.0, 1.0)
            } else {
                base_pan
            };
            (base_volume * gain, pan)
        }
    }

    fn track_to_bus(&self, _track: TrackId) -> BusId {
        // Placeholder: for now map every track to master.
        self.buses.keys().next().unwrap_or_else(BusId::default)
    }

    fn apply_command(&mut self, cmd: ManagerCommand) {
        use ManagerCommand::*;
        match cmd {
            SetVolume(voice, vol) => {
                self.backend.set_param(voice, VoiceParam::Volume(vol));
            }
            SetPan(voice, pan) => {
                self.backend.set_param(voice, VoiceParam::Pan(pan));
            }
            SetRate(voice, rate) => {
                self.backend.set_param(voice, VoiceParam::Rate(rate));
            }
            SetPosition(voice, maybe_pos) => {
                if let Some(pos) = maybe_pos {
                    self.backend.set_param(voice, VoiceParam::Position(pos));
                }
            }
            FadeVolume(voice, target, _) => {
                // TODO: tween system
                self.backend.set_param(voice, VoiceParam::Volume(target));
            }
            FadePan(voice, target, _) => {
                self.backend.set_param(voice, VoiceParam::Pan(target));
            }
            Stop(voice) => {
                self.backend.stop_voice(voice);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::AudioManager;
    use crate::backend::null::{BackendLog, NullBackend};
    use crate::math::Frame;
    use crate::sound::PlaybackSettings;
    use crate::spatial::{DistanceModel, SpatialSettings, Vec3};

    #[test]
    fn listener_updates_are_forwarded_to_backend() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        manager.set_listener_position(Vec3::new(1.0, 2.0, 3.0));
        manager.set_listener_orientation(Vec3::new(0.0, 0.0, 1.0), Vec3::Y);
        assert!(
            manager
                .backend
                .log
                .iter()
                .filter(|entry| matches!(entry, BackendLog::SetListener))
                .count()
                >= 2
        );
    }

    #[test]
    fn native_spatial_playback_applies_attenuation_and_pan() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        manager.set_listener_position(Vec3::ZERO);
        manager.set_listener_orientation(Vec3::Z, Vec3::Y);
        let key = manager.load_raw(Arc::new(vec![Frame::mono(0.25); 32]), 44_100);
        let handle = manager
            .play(
                key,
                PlaybackSettings {
                    volume: 1.0,
                    spatial: Some(SpatialSettings {
                        model: DistanceModel::Linear,
                        ref_distance: 0.0,
                        max_distance: 100.0,
                        rolloff_factor: 1.0,
                        position: Vec3::new(50.0, 0.0, 50.0),
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        let voice = manager.backend.voices.get(handle.voice).unwrap();
        assert!(voice.settings.volume < 1.0);
        assert!(voice.settings.pan > 0.0);
    }

    #[test]
    fn resume_is_forwarded_to_backend() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        manager.resume().unwrap();
        manager.update(Duration::ZERO);
        assert!(manager.backend.log.contains(&BackendLog::Resume));
    }

    #[test]
    fn update_reclaims_finished_voices() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        let key = manager.load_raw(Arc::new(vec![Frame::mono(0.25); 32]), 44_100);
        let handle = manager.play(key, PlaybackSettings::default()).unwrap();
        manager
            .backend
            .voices
            .get_mut(handle.voice)
            .unwrap()
            .finished = true;

        manager.update(Duration::from_millis(16));

        assert!(
            manager.backend.voices.get(handle.voice).is_none(),
            "finished voices should be reclaimed during update"
        );
    }
}

// ============================================================================
// Convenience impl for DefaultBackend
// ============================================================================

#[cfg(not(target_arch = "wasm32"))]
impl AudioManager<crate::DefaultBackend> {
    /// Create using the platform's default backend.
    pub fn new_default(config: AudioConfig) -> Result<Self, crate::backend::AudioError> {
        Self::new(config)
    }
}
