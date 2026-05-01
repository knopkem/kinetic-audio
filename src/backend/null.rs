//! Null backend for unit tests.
//!
//! Records every command to a `Vec` so tests can assert on behaviour.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use slotmap::SlotMap;

use crate::backend::{
    AudioError, Backend, BufferHandle, BusId, DeviceConfig, VoiceId, VoiceParam, VoiceSettings,
};
use crate::math::Frame;
use crate::spatial::Listener;

/// Backend that records to a buffer instead of a real audio device.
pub struct NullBackend {
    /// All uploaded buffers.
    pub buffers: SlotMap<BufferHandle, Arc<Vec<Frame>>>,
    /// All voices and their latest settings.
    pub voices: SlotMap<VoiceId, NullVoice>,
    /// Log of every backend call (for assertions).
    pub log: Vec<BackendLog>,
    /// Latest bus config state.
    pub bus_configs: HashMap<BusId, (f32, bool)>,
    /// Output sample rate reported by the backend.
    pub sample_rate: u32,
}

/// A voice tracked by the null backend.
pub struct NullVoice {
    /// Uploaded buffer being played.
    pub buffer: BufferHandle,
    /// Last-known voice settings.
    pub settings: VoiceSettings,
    /// Current playback offset within the region, in source sample frames.
    pub position_samples: usize,
    /// Whether the voice is still active.
    pub active: bool,
    /// Whether the voice is paused.
    pub paused: bool,
    /// Whether the voice should be reported as finished.
    pub finished: bool,
}

/// Event log emitted by [`NullBackend`].
#[derive(Clone, Debug, PartialEq)]
pub enum BackendLog {
    /// Backend startup.
    Start,
    /// Backend shutdown.
    Stop,
    /// Buffer upload: `(handle, frames, sample_rate)`.
    Upload(BufferHandle, usize, u32),
    /// Voice start for a buffer.
    Play(BufferHandle),
    /// Runtime parameter change: `(voice, debug_repr)`.
    SetParam(VoiceId, String),
    /// Immediate voice stop.
    StopVoice(VoiceId),
    /// Tick/update call.
    Tick(Duration),
    /// Listener update.
    SetListener,
    /// Resume request.
    Resume,
    /// Bus registration.
    RegisterBus(BusId),
    /// Bus removal.
    UnregisterBus(BusId),
    /// Bus gain/mute update: `(bus, gain, muted)`.
    SetBusConfig(BusId, f32, bool),
}

impl Backend for NullBackend {
    fn start(config: DeviceConfig) -> Result<Self, AudioError> {
        Ok(Self {
            buffers: SlotMap::with_key(),
            voices: SlotMap::with_key(),
            log: vec![BackendLog::Start],
            bus_configs: HashMap::new(),
            sample_rate: if config.sample_rate == 0 {
                44_100
            } else {
                config.sample_rate
            },
        })
    }

    fn stop(&mut self) {
        self.log.push(BackendLog::Stop);
    }

    fn upload_buffer(&mut self, samples: Arc<Vec<Frame>>, rate: u32) -> BufferHandle {
        let handle = self.buffers.insert(samples.clone());
        self.log
            .push(BackendLog::Upload(handle, samples.len(), rate));
        handle
    }

    fn play(
        &mut self,
        buffer: BufferHandle,
        settings: VoiceSettings,
    ) -> Result<VoiceId, AudioError> {
        let id = self.voices.insert(NullVoice {
            buffer,
            settings,
            position_samples: 0,
            active: true,
            paused: false,
            finished: false,
        });
        self.log.push(BackendLog::Play(buffer));
        Ok(id)
    }

    fn set_param(&mut self, voice: VoiceId, param: VoiceParam) {
        if let Some(v) = self.voices.get_mut(voice) {
            match &param {
                VoiceParam::Volume(g) => v.settings.volume = *g,
                VoiceParam::Pan(p) => v.settings.pan = *p,
                VoiceParam::Rate(r) => v.settings.rate = *r,
                VoiceParam::Pause => v.paused = true,
                VoiceParam::Resume => v.paused = false,
                VoiceParam::Seek(offset) => v.position_samples = *offset,
                VoiceParam::Position(_) => {}
                VoiceParam::StopAfterLoop => {}
                VoiceParam::FadeOut(_) => {
                    // Simulate fade-out completing instantly in the null backend.
                    v.active = false;
                    v.finished = true;
                }
            }
        }
        self.log
            .push(BackendLog::SetParam(voice, format!("{:?}", param)));
    }

    fn stop_voice(&mut self, voice: VoiceId) {
        if let Some(v) = self.voices.get_mut(voice) {
            v.active = false;
        }
        self.log.push(BackendLog::StopVoice(voice));
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        let done: Vec<VoiceId> = self
            .voices
            .iter()
            .filter_map(|(id, v)| v.finished.then_some(id))
            .collect();
        for id in &done {
            self.voices.remove(*id);
        }
        done
    }

    fn tick(&mut self, dt: Duration) {
        self.log.push(BackendLog::Tick(dt));
    }

    fn set_listener(&mut self, _listener: Listener) {
        self.log.push(BackendLog::SetListener);
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        self.log.push(BackendLog::Resume);
        Ok(())
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn register_bus(&mut self, id: BusId) {
        self.bus_configs.entry(id).or_insert((1.0, false));
        self.log.push(BackendLog::RegisterBus(id));
    }

    fn unregister_bus(&mut self, id: BusId) {
        self.bus_configs.remove(&id);
        self.log.push(BackendLog::UnregisterBus(id));
    }

    fn set_bus_config(&mut self, id: BusId, gain: f32, muted: bool) {
        self.bus_configs.insert(id, (gain, muted));
        self.log.push(BackendLog::SetBusConfig(id, gain, muted));
    }
}

// ── Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_backend_lifecycle() {
        let mut b = NullBackend::start(DeviceConfig::default()).unwrap();
        let buf = Arc::new(vec![Frame::mono(0.5); 100]);
        let h = b.upload_buffer(buf, 44_100);
        let v = b.play(h, VoiceSettings::default()).unwrap();
        b.set_param(v, VoiceParam::Volume(0.5));
        b.stop_voice(v);
        b.stop();

        assert!(b.log.contains(&BackendLog::Start), "expected Start in log");
        assert!(b.log.contains(&BackendLog::Stop), "expected Stop in log");
    }
}
