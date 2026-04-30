//! Native desktop backend using `cpal`.
//!
//! Mixer runs inside the real-time cpal callback.  The main thread
//! communicates with the callback via a bounded `std::sync::mpsc`.

use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use slotmap::SlotMap;

use crate::backend::{
    AudioError, Backend, BufferHandle, DeviceConfig, VoiceId, VoiceParam, VoiceSettings,
};
use crate::math::{Frame, Panning};

const MAX_VOICES: usize = 256;
const COMMAND_QUEUE_CAP: usize = 512;

/// cpal-based native backend.
pub struct CpalBackend {
    sample_rate: u32,
    buffers: SlotMap<BufferHandle, Arc<Vec<Frame>>>,
    voice_ids: SlotMap<VoiceId, ()>,
    command_tx: SyncSender<AudioCommand>,
    finished_rx: Receiver<VoiceId>,
    _stream: cpal::Stream,
}

#[derive(Clone)]
enum AudioCommand {
    Play {
        id: VoiceId,
        buffer: Arc<Vec<Frame>>,
        settings: VoiceSettings,
    },
    SetParam {
        id: VoiceId,
        param: VoiceParam,
    },
    Stop {
        id: VoiceId,
    },
}

struct Voice {
    buffer: Arc<Vec<Frame>>,
    cursor: f64,
    volume: f32,
    pan: f32,
    rate: f32,
    looped: bool,
    active: bool,
}

impl Backend for CpalBackend {
    fn start(_config: DeviceConfig) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::DeviceUnavailable("no default output device".into()))?;

        let configs: Vec<_> = device
            .supported_output_configs()
            .map_err(|e| AudioError::DeviceUnavailable(e.to_string()))?
            .collect();

        let range = configs
            .iter()
            .find(|c| c.sample_format() == cpal::SampleFormat::F32 && c.channels() == 2)
            .or_else(|| {
                configs
                    .iter()
                    .find(|c| c.sample_format() == cpal::SampleFormat::F32)
            })
            .or_else(|| configs.first())
            .ok_or_else(|| AudioError::DeviceUnavailable("no supported stream config".into()))?
            .clone();

        let sample_rate = if _config.sample_rate == 0 {
            // Let cpal pick its preferred rate (usually 44.1 kHz or 48 kHz).
            range.min_sample_rate().0
        } else {
            // Clamp user request to what the device actually supports.
            _config
                .sample_rate
                .clamp(range.min_sample_rate().0, range.max_sample_rate().0)
        };

        let stream_config = range
            .with_sample_rate(cpal::SampleRate(sample_rate))
            .config();
        let channels = stream_config.channels as usize;
        let actual_rate = stream_config.sample_rate.0;

        let (tx, rx) = sync_channel::<AudioCommand>(COMMAND_QUEUE_CAP);
        let (finished_tx, finished_rx) = sync_channel::<VoiceId>(MAX_VOICES * 2);

        let err_fn = |err| log::error!("cpal stream error: {}", err);

        let stream = device
            .build_output_stream(
                &stream_config,
                {
                    let mut voices: HashMap<VoiceId, Voice> = HashMap::with_capacity(MAX_VOICES);
                    let mut rx = rx;
                    let finished_tx = finished_tx;
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        // Drain commands.
                        while let Ok(cmd) = rx.try_recv() {
                            match cmd {
                                AudioCommand::Play {
                                    id,
                                    buffer,
                                    settings,
                                } => {
                                    if voices.len() < MAX_VOICES {
                                        voices.insert(
                                            id,
                                            Voice {
                                                buffer,
                                                cursor: 0.0,
                                                volume: settings.volume,
                                                pan: settings.pan,
                                                rate: settings.rate,
                                                looped: settings.looped,
                                                active: true,
                                            },
                                        );
                                    }
                                }
                                AudioCommand::SetParam { id, param } => {
                                    if let Some(v) = voices.get_mut(&id) {
                                        match param {
                                            VoiceParam::Volume(x) => v.volume = x,
                                            VoiceParam::Pan(x) => v.pan = x,
                                            VoiceParam::Rate(x) => v.rate = x,
                                            VoiceParam::StopAfterLoop => {}
                                            VoiceParam::FadeOut(_) => {}
                                            VoiceParam::Position(_) => {}
                                        }
                                    }
                                }
                                AudioCommand::Stop { id } => {
                                    voices.remove(&id);
                                }
                            }
                        }

                        // Render.
                        let frames = data.len() / channels.max(1);
                        for i in 0..frames {
                            let mut acc = Frame::SILENCE;
                            for (id, v) in voices.iter_mut() {
                                if !v.active {
                                    continue;
                                }
                                let idx = v.cursor as usize;
                                if idx >= v.buffer.len() {
                                    if v.looped {
                                        v.cursor -= v.buffer.len() as f64;
                                    } else {
                                        v.active = false;
                                        let _ = finished_tx.try_send(*id);
                                        continue;
                                    }
                                }
                                let s = v.buffer[v.cursor as usize];
                                let (lg, rg) = Panning::constant_power(v.pan);
                                acc.l += s.l * lg * v.volume;
                                acc.r += s.r * rg * v.volume;
                                v.cursor += v.rate as f64;
                            }
                            acc = acc.clamp();
                            if channels >= 2 {
                                data[i * channels] = acc.l;
                                data[i * channels + 1] = acc.r;
                            } else {
                                data[i] = (acc.l + acc.r) * 0.5;
                            }
                            for c in 2..channels {
                                data[i * channels + c] = 0.0;
                            }
                        }
                        voices.retain(|_, v| v.active);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| AudioError::DeviceUnavailable(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioError::DeviceUnavailable(e.to_string()))?;

        Ok(Self {
            sample_rate: actual_rate,
            buffers: SlotMap::with_key(),
            voice_ids: SlotMap::with_key(),
            command_tx: tx,
            finished_rx,
            _stream: stream,
        })
    }

    fn stop(&mut self) {}

    fn upload_buffer(&mut self, samples: Arc<Vec<Frame>>, _rate: u32) -> BufferHandle {
        self.buffers.insert(samples)
    }

    fn play(
        &mut self,
        buffer: BufferHandle,
        settings: VoiceSettings,
    ) -> Result<VoiceId, AudioError> {
        let buf = self
            .buffers
            .get(buffer)
            .cloned()
            .ok_or(AudioError::InvalidHandle)?;

        let id = self.voice_ids.insert(());

        self.command_tx
            .try_send(AudioCommand::Play {
                id,
                buffer: buf,
                settings,
            })
            .map_err(|_| AudioError::VoiceLimit)?;

        Ok(id)
    }

    fn set_param(&mut self, voice: VoiceId, param: VoiceParam) {
        let _ = self
            .command_tx
            .try_send(AudioCommand::SetParam { id: voice, param });
    }

    fn stop_voice(&mut self, voice: VoiceId) {
        let _ = self.command_tx.try_send(AudioCommand::Stop { id: voice });
        self.voice_ids.remove(voice);
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        let mut done = Vec::new();
        while let Ok(id) = self.finished_rx.try_recv() {
            self.voice_ids.remove(id);
            done.push(id);
        }
        done
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
