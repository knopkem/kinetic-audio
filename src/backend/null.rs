//! Null backend for unit tests.
//!
//! Records every command to a `Vec` so tests can assert on behaviour.

use std::sync::Arc;
use std::time::Duration;

use slotmap::SlotMap;

use crate::backend::{
    AudioError, Backend, BufferHandle, DeviceConfig, VoiceId, VoiceParam, VoiceSettings,
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
    pub sample_rate: u32,
}

/// A voice tracked by the null backend.
pub struct NullVoice {
    pub buffer: BufferHandle,
    pub settings: VoiceSettings,
    pub active: bool,
    pub finished: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackendLog {
    Start,
    Stop,
    Upload(BufferHandle, usize, u32),
    Play(BufferHandle),
    SetParam(VoiceId, String),
    StopVoice(VoiceId),
    Tick(Duration),
    SetListener,
    Resume,
}

impl Backend for NullBackend {
    fn start(config: DeviceConfig) -> Result<Self, AudioError> {
        Ok(Self {
            buffers: SlotMap::with_key(),
            voices: SlotMap::with_key(),
            log: vec![BackendLog::Start],
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
            active: true,
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
