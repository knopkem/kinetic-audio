//! Native desktop backend using `cpal`.
//!
//! Mixer runs inside the real-time cpal callback.  The main thread
//! communicates with the callback via a bounded `std::sync::mpsc`.
//! Bus effect chains are shared via `Arc<Mutex<>>` (main thread adds/removes
//! effects; audio thread `try_lock()`s to apply them without blocking).

use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use slotmap::SlotMap;

use crate::backend::{
    AudioError, Backend, BusId, BufferHandle, DeviceConfig, VoiceId, VoiceParam, VoiceSettings,
};
use crate::effects::Effect;
use crate::math::{Frame, Panning};
// Listener is used via the Backend trait's set_listener default.

const MAX_VOICES: usize = 256;
const COMMAND_QUEUE_CAP: usize = 512;

// ── Shared bus effect chain ──────────────────────────────────────────────────

/// Per-bus state visible to both the main thread and the audio callback.
pub struct BusState {
    /// Linear gain applied after bus accumulation.
    pub gain: f32,
    /// Whether the bus is currently muted.
    pub muted: bool,
    /// Effect chain processed in the audio callback.
    pub effects: Vec<Box<dyn Effect + Send>>,
}

impl BusState {
    fn new(gain: f32) -> Self {
        Self { gain, muted: false, effects: Vec::new() }
    }
}

/// Thread-safe handle to a bus's shared state.
pub type SharedBus = Arc<Mutex<BusState>>;

// ── Commands ─────────────────────────────────────────────────────────────────

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
    /// Register a new bus (or re-register after recreation).
    AddBus {
        id: BusId,
        bus: SharedBus,
    },
    /// Unregister a bus (voices routed to it fall back to master).
    RemoveBus {
        id: BusId,
    },
}

// ── Voice ─────────────────────────────────────────────────────────────────────

struct Voice {
    buffer: Arc<Vec<Frame>>,
    /// Current read position (fractional sample index).
    cursor: f64,
    /// Effective start of the playback region.
    start_sample: usize,
    /// Effective end of the playback region.
    end_sample: usize,
    volume: f32,
    pan: f32,
    rate: f32,
    looped: bool,
    stop_after_loop: bool,
    delay_remaining: usize,
    /// Fade-out state: `Some((total_samples, elapsed_samples))`.
    fade_out: Option<(usize, usize)>,
    /// Target bus (None = master).
    bus: Option<BusId>,
    active: bool,
}

// ── Backend ──────────────────────────────────────────────────────────────────

/// cpal-based native backend.
pub struct CpalBackend {
    sample_rate: u32,
    buffers: SlotMap<BufferHandle, Arc<Vec<Frame>>>,
    voice_ids: SlotMap<VoiceId, ()>,
    /// Shared bus states; also held by the audio callback.
    buses: HashMap<BusId, SharedBus>,
    command_tx: SyncSender<AudioCommand>,
    finished_rx: Receiver<VoiceId>,
    _stream: cpal::Stream,
}

impl CpalBackend {
    /// Add an effect to a bus's effect chain (main thread).
    pub fn add_bus_effect(&self, bus: BusId, effect: Box<dyn Effect + Send>) {
        if let Some(shared) = self.buses.get(&bus) {
            if let Ok(mut state) = shared.lock() {
                state.effects.push(effect);
            }
        }
    }

    /// Update bus gain and mute state (main thread).
    pub fn set_bus_config(&self, bus: BusId, gain: f32, muted: bool) {
        if let Some(shared) = self.buses.get(&bus) {
            if let Ok(mut state) = shared.lock() {
                state.gain = gain;
                state.muted = muted;
            }
        }
    }

    /// Register a new bus and share it with the audio callback.
    pub fn register_bus(&mut self, id: BusId) -> SharedBus {
        let shared = Arc::new(Mutex::new(BusState::new(1.0)));
        self.buses.insert(id, shared.clone());
        let _ = self.command_tx.try_send(AudioCommand::AddBus {
            id,
            bus: shared.clone(),
        });
        shared
    }

    /// Remove a bus.
    pub fn unregister_bus(&mut self, id: BusId) {
        self.buses.remove(&id);
        let _ = self.command_tx.try_send(AudioCommand::RemoveBus { id });
    }
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

        let range = *configs
            .iter()
            .find(|c| c.sample_format() == cpal::SampleFormat::F32 && c.channels() == 2)
            .or_else(|| {
                configs
                    .iter()
                    .find(|c| c.sample_format() == cpal::SampleFormat::F32)
            })
            .or_else(|| configs.first())
            .ok_or_else(|| AudioError::DeviceUnavailable("no supported stream config".into()))?;

        let sample_rate = if _config.sample_rate == 0 {
            range.min_sample_rate().0
        } else {
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
                    // Per-bus accumulator buffers: bus_id -> (SharedBus, samples).
                    let mut bus_buffers: HashMap<BusId, (SharedBus, Vec<Frame>)> = HashMap::new();
                    let rx = rx;
                    let finished_tx = finished_tx;
                    let sample_rate = actual_rate;
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let frames = data.len() / channels.max(1);

                        // Drain commands.
                        while let Ok(cmd) = rx.try_recv() {
                            match cmd {
                                AudioCommand::Play { id, buffer, settings } => {
                                    if voices.len() < MAX_VOICES {
                                        let buf_len = buffer.len();
                                        let start = settings.start_sample.unwrap_or(0).min(buf_len);
                                        let end = settings.end_sample.unwrap_or(buf_len).min(buf_len);
                                        voices.insert(id, Voice {
                                            cursor: start as f64,
                                            start_sample: start,
                                            end_sample: end,
                                            buffer,
                                            volume: settings.volume,
                                            pan: settings.pan,
                                            rate: settings.rate,
                                            looped: settings.looped,
                                            stop_after_loop: false,
                                            delay_remaining: settings.delay_samples,
                                            fade_out: None,
                                            bus: settings.bus,
                                            active: true,
                                        });
                                    }
                                }
                                AudioCommand::SetParam { id, param } => {
                                    if let Some(v) = voices.get_mut(&id) {
                                        match param {
                                            VoiceParam::Volume(x) => v.volume = x,
                                            VoiceParam::Pan(x) => v.pan = x,
                                            VoiceParam::Rate(x) => v.rate = x,
                                            VoiceParam::StopAfterLoop => v.stop_after_loop = true,
                                            VoiceParam::FadeOut(dur) => {
                                                let total = (dur.as_secs_f32() * sample_rate as f32) as usize;
                                                v.fade_out = Some((total.max(1), 0));
                                            }
                                            VoiceParam::Position(_) => {}
                                        }
                                    }
                                }
                                AudioCommand::Stop { id } => { voices.remove(&id); }
                                AudioCommand::AddBus { id, bus } => {
                                    bus_buffers.insert(id, (bus, vec![Frame::SILENCE; frames]));
                                }
                                AudioCommand::RemoveBus { id } => { bus_buffers.remove(&id); }
                            }
                        }

                        // Resize bus buffers if the callback frame size changed.
                        for (_, (_, buf)) in bus_buffers.iter_mut() {
                            if buf.len() != frames {
                                buf.resize(frames, Frame::SILENCE);
                            }
                        }

                        // Clear accumulation buffers.
                        let mut master_buf = vec![Frame::SILENCE; frames];
                        for (_, (_, buf)) in bus_buffers.iter_mut() {
                            for f in buf.iter_mut() { *f = Frame::SILENCE; }
                        }

                        // Mix voices into their respective bus (or master).
                        for (id, v) in voices.iter_mut() {
                            if !v.active { continue; }

                            // Render this voice into a temporary per-voice buffer.
                            let dest: &mut Vec<Frame> = if let Some(bus_id) = v.bus {
                                if let Some((_, buf)) = bus_buffers.get_mut(&bus_id) {
                                    buf
                                } else {
                                    &mut master_buf
                                }
                            } else {
                                &mut master_buf
                            };

                            for frame in dest.iter_mut().take(frames) {
                                if v.delay_remaining > 0 {
                                    v.delay_remaining -= 1;
                                    continue;
                                }
                                let idx = v.cursor as usize;
                                if idx >= v.end_sample {
                                    if v.looped && !v.stop_after_loop {
                                        v.cursor = v.start_sample as f64 + (v.cursor - v.end_sample as f64);
                                    } else {
                                        v.active = false;
                                        let _ = finished_tx.try_send(*id);
                                        break;
                                    }
                                }
                                let s = v.buffer[v.cursor as usize];
                                let (lg, rg) = Panning::constant_power(v.pan);
                                let fade_gain = if let Some((total, elapsed)) = v.fade_out {
                                    let g = 1.0 - (elapsed as f32 / total as f32);
                                    v.fade_out = if elapsed + 1 >= total {
                                        v.active = false;
                                        let _ = finished_tx.try_send(*id);
                                        None
                                    } else {
                                        Some((total, elapsed + 1))
                                    };
                                    g.max(0.0)
                                } else {
                                    1.0
                                };
                                frame.l += s.l * lg * v.volume * fade_gain;
                                frame.r += s.r * rg * v.volume * fade_gain;
                                v.cursor += v.rate as f64;
                            }
                        }
                        voices.retain(|_, v| v.active);

                        // Apply bus effects and mix buses into master.
                        for (_, (shared_bus, bus_buf)) in bus_buffers.iter_mut() {
                            if let Ok(mut state) = shared_bus.try_lock() {
                                if state.muted {
                                    for f in bus_buf.iter_mut() { *f = Frame::SILENCE; }
                                } else {
                                    // Apply per-bus gain.
                                    let g = state.gain;
                                    for f in bus_buf.iter_mut() { *f = f.scale(g); }
                                    // Run effect chain.
                                    for fx in state.effects.iter_mut() {
                                        fx.process(bus_buf, sample_rate);
                                    }
                                }
                            }
                            // Mix bus output into master.
                            for (i, f) in bus_buf.iter().enumerate() {
                                master_buf[i].l += f.l;
                                master_buf[i].r += f.r;
                            }
                        }

                        // Write clamped master to output.
                        for i in 0..frames {
                            let acc = master_buf[i].clamp();
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
            buses: HashMap::new(),
            command_tx: tx,
            finished_rx,
            _stream: stream,
        })
    }

    fn stop(&mut self) {}

    fn upload_buffer(&mut self, samples: Arc<Vec<Frame>>, _rate: u32) -> BufferHandle {
        self.buffers.insert(samples)
    }

    fn play(&mut self, buffer: BufferHandle, settings: VoiceSettings) -> Result<VoiceId, AudioError> {
        let buf = self.buffers.get(buffer).cloned().ok_or(AudioError::InvalidHandle)?;
        let id = self.voice_ids.insert(());
        self.command_tx
            .try_send(AudioCommand::Play { id, buffer: buf, settings })
            .map_err(|_| AudioError::VoiceLimit)?;
        Ok(id)
    }

    fn set_param(&mut self, voice: VoiceId, param: VoiceParam) {
        let _ = self.command_tx.try_send(AudioCommand::SetParam { id: voice, param });
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

    fn register_bus(&mut self, id: BusId) {
        CpalBackend::register_bus(self, id);
    }

    fn unregister_bus(&mut self, id: BusId) {
        CpalBackend::unregister_bus(self, id);
    }

    fn set_bus_config(&mut self, id: BusId, gain: f32, muted: bool) {
        CpalBackend::set_bus_config(self, id, gain, muted);
    }

    fn add_bus_effect(&mut self, id: BusId, effect: Box<dyn crate::effects::Effect + Send>) {
        CpalBackend::add_bus_effect(self, id, effect);
    }
}
