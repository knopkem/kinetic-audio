//! Main control surface: `AudioManager`.
//!
//! This is the only type most users interact with directly.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use slotmap::SlotMap;

#[cfg(target_arch = "wasm32")]
use crate::backend::web::WebCommandQueue;
use crate::backend::{Backend, BusId, TrackId, VoiceId, VoiceParam, VoiceSettings};
#[cfg(not(target_arch = "wasm32"))]
use crate::math::Panning;
use crate::math::{duration_to_samples, samples_to_duration, Frame};
use crate::mixer::{Bus, MixSettings};
use crate::sound::{
    ManagerCommand, PlaybackSettings, SoundData, SoundHandle, SoundKey, WeakHandleBridge,
};
use crate::spatial::{Listener, SpatialSettings};
use crate::sprite::{SpriteData, SpriteKey, SpriteRegion};
use crate::tween::Tween;

// ── Active Tween ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TweenTarget {
    Volume,
    Pan,
}

/// A tween that is currently being driven by `AudioManager::update()`.
struct ActiveTween {
    voice: VoiceId,
    target: TweenTarget,
    start_val: f32,
    end_val: f32,
    tween: Tween,
    elapsed_secs: f32,
}

struct ManagedVoiceState {
    base_volume: f32,
    base_pan: f32,
    sample_rate: u32,
    region_len_samples: usize,
    position_samples: f64,
    rate: f32,
    looped: bool,
    paused: bool,
    delay_remaining_secs: f64,
    spatial: Option<SpatialSettings>,
    spatialized_once: bool,
}

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
    sprites: SlotMap<SpriteKey, SpriteData>,
    buses: SlotMap<BusId, Bus>,
    listener: Listener,
    active_tweens: Vec<ActiveTween>,
    voice_states: HashMap<VoiceId, ManagedVoiceState>,
    /// Voices that have naturally finished playback (populated by `update()`).
    finished: HashSet<VoiceId>,
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
        });
        buses[master_id].id = master_id;

        #[cfg(not(target_arch = "wasm32"))]
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        let listener = Listener::default();
        let mut manager = Self {
            backend,
            sounds: SlotMap::with_key(),
            sprites: SlotMap::with_key(),
            buses,
            listener,
            active_tweens: Vec::new(),
            voice_states: HashMap::new(),
            finished: HashSet::new(),
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

    /// Decode an audio byte slice and return a key.
    ///
    /// `hint` should be the file extension (e.g. `"wav"`, `"ogg"`, `"mp3"`).
    pub fn load_sound(&mut self, bytes: &[u8], hint: &str) -> Result<SoundKey, crate::backend::AudioError> {
        let (frames, rate) = crate::decode::decode(bytes, hint)
            .map_err(|e| crate::backend::AudioError::Decode(e.to_string()))?;
        let n_frames = frames.len();
        let samples = Arc::new(frames);
        let handle = self.backend.upload_buffer(samples.clone(), rate);
        let key = self.sounds.insert(SoundData {
            buffer: handle,
            samples,
            sample_rate: rate,
            duration: samples_to_duration(n_frames, rate),
            channels: 2,
        });
        Ok(key)
    }

    /// Decode a WAV byte slice as a sprite sheet.
    pub fn load_sprite(&mut self, bytes: &[u8]) -> Result<SoundKey, crate::backend::AudioError> {
        self.load_sound(bytes, "wav")
    }

    /// Register named regions over an already-loaded [`SoundKey`].
    ///
    /// Returns a [`SpriteKey`] that can be passed to [`play_sprite`](Self::play_sprite).
    pub fn add_sprite(&mut self, sound: SoundKey, regions: &[SpriteRegion]) -> Result<SpriteKey, crate::backend::AudioError> {
        let rate = self
            .sounds
            .get(sound)
            .ok_or(crate::backend::AudioError::InvalidHandle)?
            .sample_rate;
        let data = SpriteData::from_sound(sound, rate, regions);
        Ok(self.sprites.insert(data))
    }

    /// Play a named region from a sprite sheet.
    ///
    /// `settings` overrides applied on top of the region's own loop flag.
    pub fn play_sprite(
        &mut self,
        sprite: SpriteKey,
        region_name: &str,
        mut settings: PlaybackSettings,
    ) -> Result<SoundHandle, crate::backend::AudioError> {
        let sprite_data = self
            .sprites
            .get(sprite)
            .ok_or(crate::backend::AudioError::InvalidHandle)?;
        let region = sprite_data
            .region(region_name)
            .ok_or_else(|| crate::backend::AudioError::Backend(format!("unknown sprite region: {region_name}")))?;

        let start = region.start_sample;
        let end = region.end_sample;
        let region_looped = region.looped;
        let sound_key = sprite_data.sound;

        // Sprite's loop flag takes effect unless the caller explicitly sets looped.
        settings.looped = settings.looped || region_looped;

        let sound = self
            .sounds
            .get(sound_key)
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
            start_sample: Some(start),
            end_sample: Some(end),
            delay_samples: duration_to_samples(settings.delay, self.backend.sample_rate()),
        };

        let voice = self.backend.play(sound.buffer, voice_settings)?;
        self.voice_states.insert(
            voice,
            ManagedVoiceState {
                base_volume: settings.volume,
                base_pan: settings.pan,
                sample_rate: sound.sample_rate,
                region_len_samples: end.saturating_sub(start),
                position_samples: 0.0,
                rate: settings.rate,
                looped: settings.looped,
                paused: false,
                delay_remaining_secs: settings.delay.as_secs_f64(),
                spatial,
                spatialized_once: spatial.is_some(),
            },
        );
        let bridge = self.make_bridge();
        Ok(SoundHandle { voice, manager: bridge, volume, pan, rate: settings.rate })
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
            start_sample: None,
            end_sample: None,
            delay_samples: duration_to_samples(settings.delay, self.backend.sample_rate()),
        };

        let voice = self.backend.play(sound.buffer, voice_settings)?;
        self.voice_states.insert(
            voice,
            ManagedVoiceState {
                base_volume: settings.volume,
                base_pan: settings.pan,
                sample_rate: sound.sample_rate,
                region_len_samples: sound.samples.len(),
                position_samples: 0.0,
                rate: settings.rate,
                looped: settings.looped,
                paused: false,
                delay_remaining_secs: settings.delay.as_secs_f64(),
                spatial,
                spatialized_once: spatial.is_some(),
            },
        );

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
        self.refresh_spatial_voices();
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
        self.refresh_spatial_voices();
    }

    // -------------------------------------------------------------------
    // Buses / Mixer
    // -------------------------------------------------------------------

    /// Add a new sub-master bus.
    pub fn add_bus(&mut self, settings: MixSettings) -> Result<BusId, crate::backend::AudioError> {
        let id = self.buses.insert(Bus {
            id: BusId::default(),
            settings: settings.clone(),
        });
        self.buses[id].id = id;
        self.backend.register_bus(id);
        self.sync_bus_configs();
        Ok(id)
    }

    /// Remove a bus (voices routed here fall back to master).
    pub fn remove_bus(&mut self, id: BusId) {
        self.buses.remove(id);
        self.backend.unregister_bus(id);
        self.sync_bus_configs();
    }

    /// Set per-bus gain in decibels.
    pub fn set_bus_volume_db(&mut self, id: BusId, db: f32) {
        if let Some(bus) = self.buses.get_mut(id) {
            bus.settings.gain = crate::math::Decibels::to_linear(db).clamp(0.0, 2.0);
        }
        self.sync_bus_configs();
    }

    /// Mute or un-mute a bus.
    pub fn set_bus_muted(&mut self, id: BusId, muted: bool) {
        if let Some(bus) = self.buses.get_mut(id) {
            bus.settings.muted = muted;
        }
        self.sync_bus_configs();
    }

    /// Solo or un-solo a bus.
    pub fn set_bus_soloed(&mut self, id: BusId, soloed: bool) {
        if let Some(bus) = self.buses.get_mut(id) {
            bus.settings.soloed = soloed;
        }
        self.sync_bus_configs();
    }

    /// Add a DSP effect to a bus's effect chain.
    ///
    /// Effects are applied in insertion order every audio callback frame.
    pub fn add_bus_effect(&mut self, id: BusId, effect: Box<dyn crate::effects::Effect + Send>) {
        self.backend.add_bus_effect(id, effect);
    }

    // -------------------------------------------------------------------
    // Frame update
    // -------------------------------------------------------------------

    /// Must be called once per frame.
    ///
    /// * Native: drains command queue, advances tweens.
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

        let dt_secs_f64 = dt.as_secs_f64();
        for state in self.voice_states.values_mut() {
            if state.paused {
                continue;
            }

            let mut remaining_dt = dt_secs_f64;
            if state.delay_remaining_secs > 0.0 {
                if remaining_dt <= state.delay_remaining_secs {
                    state.delay_remaining_secs -= remaining_dt;
                    continue;
                }
                remaining_dt -= state.delay_remaining_secs;
                state.delay_remaining_secs = 0.0;
            }

            state.position_samples += remaining_dt * state.sample_rate as f64 * state.rate as f64;
            let region_len = state.region_len_samples as f64;
            if state.looped && region_len > 0.0 {
                state.position_samples %= region_len;
            } else if region_len > 0.0 {
                state.position_samples = state.position_samples.min(region_len);
            }
        }

        // Advance active tweens.
        let dt_secs = dt.as_secs_f32();
        let mut active_tweens = std::mem::take(&mut self.active_tweens);
        self.active_tweens = active_tweens
            .drain(..)
            .filter_map(|mut tw| {
            tw.elapsed_secs += dt_secs;
            let t = tw.tween.sample(tw.elapsed_secs);
            let val = tw.start_val + (tw.end_val - tw.start_val) * t;
            if let Some(state) = self.voice_states.get_mut(&tw.voice) {
                match tw.target {
                    TweenTarget::Volume => state.base_volume = val,
                    TweenTarget::Pan => state.base_pan = val,
                }
                self.refresh_voice_mix(tw.voice);
            } else {
                let param = match tw.target {
                    TweenTarget::Volume => VoiceParam::Volume(val),
                    TweenTarget::Pan => VoiceParam::Pan(val),
                };
                self.backend.set_param(tw.voice, param);
            }
            // Keep tween until it has fully reached its end.
            (tw.elapsed_secs < tw.tween.duration.as_secs_f32()).then_some(tw)
        })
        .collect();

        self.backend.tick(dt);
        for id in self.backend.finished_voices() {
            self.voice_states.remove(&id);
            self.active_tweens.retain(|tw| tw.voice != id);
            self.finished.insert(id);
        }
    }

    /// Returns `true` if the voice for this handle has naturally ended.
    ///
    /// The finished state is retained until the handle is dropped or
    /// [`AudioManager::update`] is called and the entry is evicted, so
    /// short-lived callers that only check once per frame will not miss it.
    pub fn is_finished(&self, handle: &crate::sound::SoundHandle) -> bool {
        self.finished.contains(&handle.voice)
    }

    /// Returns the current playback position within the sound or sprite region.
    pub fn playback_position(&self, handle: &crate::sound::SoundHandle) -> Option<Duration> {
        self.voice_states.get(&handle.voice).map(|state| {
            let secs = (state.position_samples / state.sample_rate as f64).max(0.0);
            Duration::from_secs_f64(secs)
        })
    }

    /// Returns whether the voice is currently paused.
    pub fn is_paused(&self, handle: &crate::sound::SoundHandle) -> bool {
        self.voice_states
            .get(&handle.voice)
            .map(|state| state.paused)
            .unwrap_or(false)
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

    /// Expose the backend for inspection in tests and advanced integrations.
    pub fn backend(&self) -> &B {
        &self.backend
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

    fn refresh_voice_mix(&mut self, voice: VoiceId) {
        let Some(state) = self.voice_states.get(&voice) else {
            return;
        };
        let (volume, pan) =
            self.effective_pan_volume(state.base_volume, state.base_pan, state.spatial);
        self.backend.set_param(voice, VoiceParam::Volume(volume));
        self.backend.set_param(voice, VoiceParam::Pan(pan));

        if let Some(spatial) = state.spatial {
            self.backend
                .set_param(voice, VoiceParam::Position(spatial.position));
        } else if state.spatialized_once {
            self.backend
                .set_param(voice, VoiceParam::Position(self.listener.position));
        }
    }

    fn refresh_spatial_voices(&mut self) {
        let voices: Vec<VoiceId> = self.voice_states.keys().copied().collect();
        for voice in voices {
            self.refresh_voice_mix(voice);
        }
    }

    fn sync_bus_configs(&mut self) {
        let any_soloed = self.buses.values().any(|bus| bus.settings.soloed);
        let configs: Vec<(BusId, f32, bool)> = self
            .buses
            .iter()
            .map(|(id, bus)| {
                let effective_muted =
                    bus.settings.muted || (any_soloed && !bus.settings.soloed);
                (id, bus.settings.gain, effective_muted)
            })
            .collect();

        for (id, gain, muted) in configs {
            self.backend.set_bus_config(id, gain, muted);
        }
    }

    fn track_to_bus(&self, track: TrackId) -> BusId {
        track
    }

    fn apply_command(&mut self, cmd: ManagerCommand) {
        use ManagerCommand::*;
        match cmd {
            SetVolume(voice, vol) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.base_volume = vol;
                    self.refresh_voice_mix(voice);
                } else {
                    self.backend.set_param(voice, VoiceParam::Volume(vol));
                }
                // Cancel any in-flight volume tween for this voice.
                self.active_tweens
                    .retain(|tw| !(tw.voice == voice && tw.target == TweenTarget::Volume));
            }
            SetPan(voice, pan) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.base_pan = pan;
                    self.refresh_voice_mix(voice);
                } else {
                    self.backend.set_param(voice, VoiceParam::Pan(pan));
                }
                self.active_tweens
                    .retain(|tw| !(tw.voice == voice && tw.target == TweenTarget::Pan));
            }
            SetRate(voice, rate) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.rate = rate;
                }
                self.backend.set_param(voice, VoiceParam::Rate(rate));
            }
            SetPosition(voice, maybe_pos) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    match maybe_pos {
                        Some(pos) => {
                            let mut spatial = state.spatial.unwrap_or_default();
                            spatial.position = pos;
                            state.spatial = Some(spatial);
                            state.spatialized_once = true;
                        }
                        None => {
                            state.spatial = None;
                        }
                    }
                    self.refresh_voice_mix(voice);
                } else if let Some(pos) = maybe_pos {
                    self.backend.set_param(voice, VoiceParam::Position(pos));
                }
            }
            FadeVolume(voice, target, tween) => {
                if tween.duration.is_zero() {
                    if let Some(state) = self.voice_states.get_mut(&voice) {
                        state.base_volume = target;
                        self.refresh_voice_mix(voice);
                    } else {
                        self.backend.set_param(voice, VoiceParam::Volume(target));
                    }
                    self.active_tweens
                        .retain(|tw| !(tw.voice == voice && tw.target == TweenTarget::Volume));
                } else {
                    // Use the last known in-flight value, otherwise the tracked voice state.
                    let start_val = self
                        .active_tweens
                        .iter()
                        .rfind(|tw| tw.voice == voice && tw.target == TweenTarget::Volume)
                        .map(|tw| tw.start_val + (tw.end_val - tw.start_val) * tw.tween.sample(tw.elapsed_secs))
                        .or_else(|| self.voice_states.get(&voice).map(|s| s.base_volume))
                        .unwrap_or(0.0);
                    self.active_tweens
                        .retain(|tw| !(tw.voice == voice && tw.target == TweenTarget::Volume));
                    self.active_tweens.push(ActiveTween {
                        voice,
                        target: TweenTarget::Volume,
                        start_val,
                        end_val: target,
                        tween,
                        elapsed_secs: 0.0,
                    });
                }
            }
            FadePan(voice, target, tween) => {
                if tween.duration.is_zero() {
                    if let Some(state) = self.voice_states.get_mut(&voice) {
                        state.base_pan = target;
                        self.refresh_voice_mix(voice);
                    } else {
                        self.backend.set_param(voice, VoiceParam::Pan(target));
                    }
                    self.active_tweens
                        .retain(|tw| !(tw.voice == voice && tw.target == TweenTarget::Pan));
                } else {
                    let start_val = self
                        .active_tweens
                        .iter()
                        .rfind(|tw| tw.voice == voice && tw.target == TweenTarget::Pan)
                        .map(|tw| tw.start_val + (tw.end_val - tw.start_val) * tw.tween.sample(tw.elapsed_secs))
                        .or_else(|| self.voice_states.get(&voice).map(|s| s.base_pan))
                        .unwrap_or(0.0);
                    self.active_tweens
                        .retain(|tw| !(tw.voice == voice && tw.target == TweenTarget::Pan));
                    self.active_tweens.push(ActiveTween {
                        voice,
                        target: TweenTarget::Pan,
                        start_val,
                        end_val: target,
                        tween,
                        elapsed_secs: 0.0,
                    });
                }
            }
            Pause(voice) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.paused = true;
                }
                self.backend.set_param(voice, VoiceParam::Pause);
            }
            Resume(voice) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.paused = false;
                }
                self.backend.set_param(voice, VoiceParam::Resume);
            }
            SeekTo(voice, position) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.delay_remaining_secs = 0.0;
                    let target = duration_to_samples(position, state.sample_rate) as f64;
                    let max = state.region_len_samples as f64;
                    state.position_samples = if state.looped && max > 0.0 {
                        target % max
                    } else {
                        target.clamp(0.0, max)
                    };
                    self.backend
                        .set_param(voice, VoiceParam::Seek(state.position_samples as usize));
                } else {
                    self.backend.set_param(
                        voice,
                        VoiceParam::Seek(duration_to_samples(position, self.backend.sample_rate())),
                    );
                }
            }
            SeekBy(voice, delta_seconds) => {
                if let Some(state) = self.voice_states.get_mut(&voice) {
                    state.delay_remaining_secs = 0.0;
                    let delta = delta_seconds * state.sample_rate as f64;
                    let max = state.region_len_samples as f64;
                    let target = state.position_samples + delta;
                    state.position_samples = if state.looped && max > 0.0 {
                        target.rem_euclid(max)
                    } else {
                        target.clamp(0.0, max)
                    };
                    self.backend
                        .set_param(voice, VoiceParam::Seek(state.position_samples as usize));
                }
            }
            Stop(voice) => {
                // Cancel any tweens targeting this voice.
                self.active_tweens.retain(|tw| tw.voice != voice);
                self.voice_states.remove(&voice);
                self.backend.stop_voice(voice);
            }
            StopAfterLoop(voice) => {
                self.backend.set_param(voice, VoiceParam::StopAfterLoop);
            }
            FadeOut(voice, dur) => {
                self.active_tweens.retain(|tw| tw.voice != voice);
                self.backend.set_param(voice, VoiceParam::FadeOut(dur));
            }
        }
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
    fn moving_listener_recomputes_active_voice_mix() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        manager.set_listener_position(Vec3::ZERO);
        manager.set_listener_orientation(Vec3::Z, Vec3::Y);
        let key = manager.load_raw(Arc::new(vec![Frame::mono(0.25); 32]), 44_100);
        let handle = manager
            .play(
                key,
                PlaybackSettings {
                    spatial: Some(SpatialSettings {
                        model: DistanceModel::Linear,
                        ref_distance: 0.0,
                        max_distance: 100.0,
                        rolloff_factor: 1.0,
                        position: Vec3::new(50.0, 0.0, 0.0),
                    }),
                    ..Default::default()
                },
            )
            .unwrap();

        let initial_pan = manager.backend.voices.get(handle.voice).unwrap().settings.pan;
        manager.set_listener_position(Vec3::new(50.0, 0.0, 0.0));
        let voice = manager.backend.voices.get(handle.voice).unwrap();

        assert!(initial_pan > 0.0);
        assert_eq!(voice.settings.volume, 1.0);
        assert!(voice.settings.pan.abs() < 1e-4);
    }

    #[test]
    fn moving_voice_updates_active_spatial_mix() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        manager.set_listener_position(Vec3::ZERO);
        manager.set_listener_orientation(Vec3::Z, Vec3::Y);
        let key = manager.load_raw(Arc::new(vec![Frame::mono(0.25); 32]), 44_100);
        let mut handle = manager.play(key, PlaybackSettings::default()).unwrap();

        handle.set_position(Vec3::new(25.0, 0.0, 25.0));
        manager.update(Duration::ZERO);

        let voice = manager.backend.voices.get(handle.voice).unwrap();
        assert!(voice.settings.volume < 1.0);
        assert!(voice.settings.pan > 0.0);
    }

    #[test]
    fn clear_position_restores_base_mix() {
        let mut manager = AudioManager::<NullBackend>::new(Default::default()).unwrap();
        manager.set_listener_position(Vec3::ZERO);
        manager.set_listener_orientation(Vec3::Z, Vec3::Y);
        let key = manager.load_raw(Arc::new(vec![Frame::mono(0.25); 32]), 44_100);
        let mut handle = manager
            .play(
                key,
                PlaybackSettings {
                    volume: 0.8,
                    pan: -0.25,
                    spatial: Some(SpatialSettings {
                        position: Vec3::new(40.0, 0.0, 0.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();

        handle.clear_position();
        manager.update(Duration::ZERO);

        let voice = manager.backend.voices.get(handle.voice).unwrap();
        assert!((voice.settings.volume - 0.8).abs() < 1e-4);
        assert!((voice.settings.pan + 0.25).abs() < 1e-4);
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
