//! WASM / Web Audio API backend.
//!
//! Uses `web_sys` to create `AudioBufferSourceNode`s, `GainNode`s,
//! `PannerNode`s, etc.  All audio rendering happens inside the browser's
//! audio thread; Rust only issues high-level commands during `tick()`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use js_sys::Float32Array;
use slotmap::SlotMap;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    AudioBuffer, AudioBufferOptions, AudioBufferSourceNode, AudioContext, AudioContextOptions,
    AudioScheduledSourceNode, DistanceModelType, GainNode, PannerNode, PanningModelType,
    StereoPannerNode,
};

use crate::backend::{
    AudioError, Backend, BufferHandle, BusId, DeviceConfig, VoiceId, VoiceParam, VoiceSettings,
};
use crate::math::Frame;
use crate::spatial::{DistanceModel, Listener, SpatialSettings};

/// Shared command queue used by both `SoundHandle` (producer) and
/// `AudioManager::update()` (consumer) on the WASM main thread.
#[derive(Clone, Debug, Default)]
pub struct WebCommandQueue {
    inner: Rc<RefCell<VecDeque<crate::sound::ManagerCommand>>>,
}

impl WebCommandQueue {
    /// Push a command for later draining by `AudioManager::update()`.
    pub(crate) fn push(&self, cmd: crate::sound::ManagerCommand) {
        self.inner.borrow_mut().push_back(cmd);
    }

    /// Take all pending commands (called by `AudioManager::update()`).
    pub(crate) fn drain(&self) -> Vec<crate::sound::ManagerCommand> {
        self.inner.borrow_mut().drain(..).collect()
    }
}

/// Web Audio API backend.
pub struct WebAudioBackend {
    context: AudioContext,
    master: GainNode,
    buses: HashMap<BusId, WebBus>,
    buffers: SlotMap<BufferHandle, AudioBuffer>,
    voices: SlotMap<VoiceId, WebVoice>,
    finished_queue: Rc<RefCell<Vec<VoiceId>>>,
    listener: Listener,
    sample_rate: u32,
}

struct WebBus {
    gain: GainNode,
    linear_gain: f32,
    muted: bool,
}

struct WebVoice {
    source: AudioBufferSourceNode,
    gain: GainNode,
    panner: Option<PannerNode>,
    stereo_panner: Option<StereoPannerNode>,
    onended: Option<Closure<dyn FnMut()>>,
    buffer: BufferHandle,
    buffer_sample_rate: f64,
    region_start_secs: f64,
    scheduled_start_time: f64,
    region_duration_secs: f64,
    looped: bool,
    rate: f32,
    paused: bool,
    paused_offset_secs: f64,
}

fn scheduled_source(source: &AudioBufferSourceNode) -> &AudioScheduledSourceNode {
    source.unchecked_ref::<AudioScheduledSourceNode>()
}

impl WebAudioBackend {
    fn current_offset_secs_at(voice: &WebVoice, now: f64) -> f64 {
        if voice.paused {
            return voice.paused_offset_secs.min(voice.region_duration_secs);
        }

        if now < voice.scheduled_start_time {
            return 0.0;
        }

        let elapsed = (now - voice.scheduled_start_time) * voice.rate.max(0.001) as f64;
        if voice.looped && voice.region_duration_secs > 0.0 {
            elapsed % voice.region_duration_secs
        } else {
            elapsed.min(voice.region_duration_secs)
        }
    }

    fn replace_source(
        &mut self,
        voice_id: VoiceId,
        offset_secs: f64,
        start_when: f64,
    ) -> Result<(), AudioError> {
        let Some(voice) = self.voices.get_mut(voice_id) else {
            return Err(AudioError::InvalidHandle);
        };

        let Some(buffer) = self.buffers.get(voice.buffer).cloned() else {
            return Err(AudioError::InvalidHandle);
        };

        let offset_secs = offset_secs.clamp(0.0, voice.region_duration_secs);
        if !voice.looped && offset_secs >= voice.region_duration_secs {
            scheduled_source(&voice.source).set_onended(None);
            let _ = scheduled_source(&voice.source).stop();
            self.finished_queue.borrow_mut().push(voice_id);
            return Ok(());
        }

        scheduled_source(&voice.source).set_onended(None);
        let _ = scheduled_source(&voice.source).stop();

        let source = AudioBufferSourceNode::new(&self.context)
            .map_err(|e| AudioError::DeviceUnavailable(format!("source node: {:?}", e)))?;
        source.set_buffer(Some(&buffer));
        source.playback_rate().set_value(voice.rate);
        if voice.looped {
            source.set_loop(true);
            source.set_loop_start(voice.region_start_secs);
            source.set_loop_end(voice.region_start_secs + voice.region_duration_secs);
        }
        source
            .connect_with_audio_node(voice.panner.as_ref().expect("panner exists"))
            .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;

        let finished_queue = self.finished_queue.clone();
        let onended = Closure::wrap(Box::new(move || {
            finished_queue.borrow_mut().push(voice_id);
        }) as Box<dyn FnMut()>);
        scheduled_source(&source).set_onended(Some(onended.as_ref().unchecked_ref()));

        let absolute_offset = voice.region_start_secs + offset_secs;
        let start_result = if voice.looped {
            source.start_with_when_and_grain_offset(start_when, absolute_offset)
        } else {
            source.start_with_when_and_grain_offset_and_grain_duration(
                start_when,
                absolute_offset,
                voice.region_duration_secs - offset_secs,
            )
        };
        start_result.map_err(|e| AudioError::DeviceUnavailable(format!("start: {:?}", e)))?;

        voice.source = source;
        voice.onended = Some(onended);
        voice.paused = false;
        voice.paused_offset_secs = offset_secs;
        voice.scheduled_start_time = start_when - offset_secs / voice.rate.max(0.001) as f64;
        Ok(())
    }
}

impl Backend for WebAudioBackend {
    fn start(config: DeviceConfig) -> Result<Self, AudioError> {
        let opts = AudioContextOptions::new();
        if config.sample_rate != 0 {
            opts.set_sample_rate(config.sample_rate as f32);
        }

        let context_res: Result<AudioContext, JsValue> =
            AudioContext::new_with_context_options(&opts);
        let context = context_res
            .map_err(|e| AudioError::DeviceUnavailable(format!("AudioContext: {:?}", e)))?;

        let sample_rate = context.sample_rate() as u32;

        let master: GainNode = GainNode::new(&context)
            .map_err(|e| AudioError::DeviceUnavailable(format!("GainNode: {:?}", e)))?;

        let destination = context.destination();
        master
            .connect_with_audio_node(&destination)
            .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;

        let mut backend = Self {
            context,
            master,
            buses: HashMap::new(),
            buffers: SlotMap::with_key(),
            voices: SlotMap::with_key(),
            finished_queue: Rc::new(RefCell::new(Vec::new())),
            listener: Listener::default(),
            sample_rate,
        };
        backend.set_listener(Listener::default());
        Ok(backend)
    }

    fn stop(&mut self) {
        let _ = self.context.close();
    }

    fn upload_buffer(&mut self, samples: Arc<Vec<Frame>>, rate: u32) -> BufferHandle {
        let n_frames = samples.len();
        let buf_opts = AudioBufferOptions::new(n_frames as u32, rate as f32);
        buf_opts.set_number_of_channels(2);

        let buf = AudioBuffer::new(&buf_opts).expect("AudioBuffer allocation failed");

        // De-interleave stereo frames into separate channels.
        let left = Float32Array::new_with_length(n_frames as u32);
        let right = Float32Array::new_with_length(n_frames as u32);
        for (i, f) in samples.iter().enumerate() {
            left.set_index(i as u32, f.l);
            right.set_index(i as u32, f.r);
        }

        let _ = buf.copy_to_channel_with_f32_array(&left, 0);
        let _ = buf.copy_to_channel_with_f32_array(&right, 1);

        self.buffers.insert(buf)
    }

    fn play(
        &mut self,
        buffer: BufferHandle,
        settings: VoiceSettings,
    ) -> Result<VoiceId, AudioError> {
        self.resume()?;
        let buf = self
            .buffers
            .get(buffer)
            .ok_or(AudioError::InvalidHandle)?
            .clone();
        let buffer_len = buf.length() as usize;
        let start_sample = settings.start_sample.unwrap_or(0).min(buffer_len);
        let end_sample = settings.end_sample.unwrap_or(buffer_len).min(buffer_len);
        if end_sample <= start_sample {
            return Err(AudioError::Backend("invalid playback region".into()));
        }

        let buffer_rate = buf.sample_rate() as f64;
        let offset_secs = start_sample as f64 / buffer_rate;
        let region_duration_secs = (end_sample - start_sample) as f64 / buffer_rate;
        let scheduled_start_time =
            self.context.current_time() + settings.delay_samples as f64 / self.sample_rate as f64;

        let source: AudioBufferSourceNode = AudioBufferSourceNode::new(&self.context)
            .map_err(|e| AudioError::DeviceUnavailable(format!("source node: {:?}", e)))?;
        source.set_buffer(Some(&buf));

        let gain: GainNode = GainNode::new(&self.context)
            .map_err(|e| AudioError::DeviceUnavailable(format!("gain node: {:?}", e)))?;
        gain.gain().set_value(settings.volume);

        let panner = {
            let p: PannerNode = PannerNode::new(&self.context)
                .map_err(|e| AudioError::DeviceUnavailable(format!("panner: {:?}", e)))?;
            let spatial = settings.spatial.unwrap_or(SpatialSettings {
                position: self.listener.position,
                ..Default::default()
            });
            p.set_panning_model(PanningModelType::Equalpower);
            p.set_distance_model(match spatial.model {
                DistanceModel::Inverse => DistanceModelType::Inverse,
                DistanceModel::Linear => DistanceModelType::Linear,
                DistanceModel::Exponential => DistanceModelType::Exponential,
            });
            p.set_ref_distance(spatial.ref_distance as f64);
            p.set_max_distance(spatial.max_distance as f64);
            p.set_rolloff_factor(spatial.rolloff_factor as f64);
            p.set_position(
                spatial.position.x as f64,
                spatial.position.y as f64,
                spatial.position.z as f64,
            );
            Some(p)
        };

        let stereo_panner = {
            let node = StereoPannerNode::new(&self.context)
                .map_err(|e| AudioError::DeviceUnavailable(format!("stereo panner: {:?}", e)))?;
            node.pan().set_value(settings.pan);
            Some(node)
        };

        source
            .connect_with_audio_node(panner.as_ref().expect("panner exists"))
            .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;

        if let Some(ref p) = panner {
            p.connect_with_audio_node(
                stereo_panner
                    .as_ref()
                    .map(|sp| sp.as_ref())
                    .unwrap_or(gain.as_ref()),
            )
            .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
        }

        if let Some(ref sp) = stereo_panner {
            sp.connect_with_audio_node(&gain)
                .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
        }

        if let Some(bus_id) = settings.bus {
            if let Some(bus) = self.buses.get(&bus_id) {
                gain.connect_with_audio_node(&bus.gain)
                    .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
            } else {
                gain.connect_with_audio_node(&self.master)
                    .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
            }
        } else {
            gain.connect_with_audio_node(&self.master)
                .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
        }

        if settings.looped {
            source.set_loop(true);
            source.set_loop_start(offset_secs);
            source.set_loop_end(offset_secs + region_duration_secs);
        }
        source.playback_rate().set_value(settings.rate);
        let id = self.voices.insert(WebVoice {
            source: source.clone(),
            gain: gain.clone(),
            panner: panner.clone(),
            stereo_panner: stereo_panner.clone(),
            onended: None,
            buffer,
            buffer_sample_rate: buffer_rate,
            region_start_secs: offset_secs,
            scheduled_start_time,
            region_duration_secs,
            looped: settings.looped,
            rate: settings.rate,
            paused: false,
            paused_offset_secs: 0.0,
        });
        let finished_queue = self.finished_queue.clone();
        let onended = Closure::wrap(Box::new(move || {
            finished_queue.borrow_mut().push(id);
        }) as Box<dyn FnMut()>);
        scheduled_source(&source).set_onended(Some(onended.as_ref().unchecked_ref()));
        if let Some(v) = self.voices.get_mut(id) {
            v.onended = Some(onended);
        }

        let start_result = if settings.looped {
            if start_sample == 0 && end_sample == buffer_len {
                source.start_with_when(scheduled_start_time)
            } else {
                source.start_with_when_and_grain_offset(scheduled_start_time, offset_secs)
            }
        } else if start_sample == 0 && end_sample == buffer_len {
            source.start_with_when(scheduled_start_time)
        } else {
            source.start_with_when_and_grain_offset_and_grain_duration(
                scheduled_start_time,
                offset_secs,
                region_duration_secs,
            )
        };
        start_result.map_err(|e| AudioError::DeviceUnavailable(format!("start: {:?}", e)))?;

        Ok(id)
    }

    fn set_param(&mut self, voice: VoiceId, param: VoiceParam) {
        let Some(v) = self.voices.get_mut(voice) else {
            return;
        };
        match param {
            VoiceParam::Volume(vol) => v.gain.gain().set_value(vol),
            VoiceParam::Pan(pan) => {
                if let Some(ref stereo_panner) = v.stereo_panner {
                    stereo_panner.pan().set_value(pan);
                }
            }
            VoiceParam::Rate(rate) => {
                let now = self.context.current_time();
                let offset = Self::current_offset_secs_at(v, now);
                v.rate = rate;
                v.source.playback_rate().set_value(rate);
                v.scheduled_start_time = now - offset / v.rate.max(0.001) as f64;
            }
            VoiceParam::Pause => {
                let now = self.context.current_time();
                v.paused_offset_secs = Self::current_offset_secs_at(v, now);
                v.paused = true;
                scheduled_source(&v.source).set_onended(None);
                let _ = scheduled_source(&v.source).stop();
            }
            VoiceParam::Resume => {
                if v.paused {
                    let offset = v.paused_offset_secs;
                    let now = self.context.current_time();
                    let _ = self.replace_source(voice, offset, now);
                }
            }
            VoiceParam::Seek(offset_samples) => {
                let offset_secs = offset_samples as f64 / v.buffer_sample_rate;
                if v.paused {
                    v.paused_offset_secs = offset_secs.min(v.region_duration_secs);
                } else {
                    let now = self.context.current_time();
                    let _ = self.replace_source(voice, offset_secs, now);
                }
            }
            VoiceParam::Position(pos) => {
                if let Some(ref p) = v.panner {
                    p.set_position(pos.x as f64, pos.y as f64, pos.z as f64);
                }
            }
            VoiceParam::StopAfterLoop => {
                if v.looped {
                    let now = self.context.current_time();
                    let playback_rate = v.rate.max(0.001) as f64;
                    let cycle_secs = v.region_duration_secs / playback_rate;
                    let offset = Self::current_offset_secs_at(v, now);
                    let remaining = if v.paused {
                        cycle_secs - (offset / playback_rate)
                    } else if now < v.scheduled_start_time {
                        (v.scheduled_start_time - now) + cycle_secs
                    } else {
                        let phase = offset / playback_rate;
                        if phase == 0.0 { cycle_secs } else { cycle_secs - phase }
                    };
                    let _ = scheduled_source(&v.source).stop_with_when(now + remaining.max(0.0));
                }
            }
            VoiceParam::FadeOut(duration) => {
                let now = self.context.current_time();
                let secs = duration.as_secs_f64();
                if secs <= 0.0 {
                    scheduled_source(&v.source).set_onended(None);
                    let _ = scheduled_source(&v.source).stop();
                    self.finished_queue.borrow_mut().push(voice);
                    return;
                }
                let param = v.gain.gain();
                let current = param.value();
                let _ = param.cancel_scheduled_values(now);
                let _ = param.set_value_at_time(current, now);
                let _ = param.linear_ramp_to_value_at_time(0.0, now + secs);
                if v.paused {
                    scheduled_source(&v.source).set_onended(None);
                    let _ = scheduled_source(&v.source).stop();
                    self.finished_queue.borrow_mut().push(voice);
                } else {
                    let _ = scheduled_source(&v.source).stop_with_when(now + secs);
                }
            }
        }
    }

    fn stop_voice(&mut self, voice: VoiceId) {
        if let Some(v) = self.voices.remove(voice) {
            scheduled_source(&v.source).set_onended(None);
            let _ = scheduled_source(&v.source).stop();
        }
        self.finished_queue.borrow_mut().retain(|id| *id != voice);
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        let finished: Vec<VoiceId> = self.finished_queue.borrow_mut().drain(..).collect();
        let mut cleaned = Vec::new();
        for id in finished {
            if let Some(v) = self.voices.remove(id) {
                scheduled_source(&v.source).set_onended(None);
                cleaned.push(id);
            }
        }
        cleaned
    }

    fn tick(&mut self, _dt: Duration) {}

    fn set_listener(&mut self, listener: Listener) {
        self.listener = listener;
        let audio_listener = self.context.listener();
        audio_listener.set_position(
            listener.position.x as f64,
            listener.position.y as f64,
            listener.position.z as f64,
        );
        audio_listener.set_orientation(
            listener.forward.x as f64,
            listener.forward.y as f64,
            listener.forward.z as f64,
            listener.up.x as f64,
            listener.up.y as f64,
            listener.up.z as f64,
        );
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        self.context
            .resume()
            .map(|_| ())
            .map_err(|e| AudioError::Backend(format!("resume: {:?}", e)))
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn register_bus(&mut self, id: BusId) {
        if self.buses.contains_key(&id) {
            return;
        }

        if let Ok(gain) = GainNode::new(&self.context) {
            let _ = gain.gain().set_value(1.0);
            let _ = gain.connect_with_audio_node(&self.master);
            self.buses.insert(
                id,
                WebBus {
                    gain,
                    linear_gain: 1.0,
                    muted: false,
                },
            );
        }
    }

    fn unregister_bus(&mut self, id: BusId) {
        if let Some(bus) = self.buses.remove(&id) {
            let _ = bus.gain.disconnect();
        }
    }

    fn set_bus_config(&mut self, id: BusId, gain: f32, muted: bool) {
        if let Some(bus) = self.buses.get_mut(&id) {
            bus.linear_gain = gain;
            bus.muted = muted;
            bus.gain
                .gain()
                .set_value(if muted { 0.0 } else { gain });
        }
    }
}
