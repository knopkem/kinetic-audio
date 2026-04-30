//! WASM / Web Audio API backend.
//!
//! Uses `web_sys` to create `AudioBufferSourceNode`s, `GainNode`s,
//! `PannerNode`s, etc.  All audio rendering happens inside the browser's
//! audio thread; Rust only issues high-level commands during `tick()`.

use std::cell::RefCell;
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
    DistanceModelType, GainNode, PannerNode, PanningModelType,
};

use crate::backend::{
    AudioError, Backend, BufferHandle, DeviceConfig, VoiceId, VoiceParam, VoiceSettings,
};
use crate::math::Frame;
use crate::spatial::{DistanceModel, Listener};

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
    buffers: SlotMap<BufferHandle, AudioBuffer>,
    voices: SlotMap<VoiceId, WebVoice>,
    finished_queue: Rc<RefCell<Vec<VoiceId>>>,
    sample_rate: u32,
}

struct WebVoice {
    source: AudioBufferSourceNode,
    gain: GainNode,
    panner: Option<PannerNode>,
    onended: Option<Closure<dyn FnMut()>>,
    _buffer: BufferHandle,
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
            buffers: SlotMap::with_key(),
            voices: SlotMap::with_key(),
            finished_queue: Rc::new(RefCell::new(Vec::new())),
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

        let source: AudioBufferSourceNode = AudioBufferSourceNode::new(&self.context)
            .map_err(|e| AudioError::DeviceUnavailable(format!("source node: {:?}", e)))?;
        source.set_buffer(Some(&buf));

        let gain: GainNode = GainNode::new(&self.context)
            .map_err(|e| AudioError::DeviceUnavailable(format!("gain node: {:?}", e)))?;
        gain.gain().set_value(settings.volume);

        let panner = if settings.spatial.is_some() {
            let p: PannerNode = PannerNode::new(&self.context)
                .map_err(|e| AudioError::DeviceUnavailable(format!("panner: {:?}", e)))?;
            if let Some(ref s) = settings.spatial {
                p.set_panning_model(PanningModelType::Equalpower);
                p.set_distance_model(match s.model {
                    DistanceModel::Inverse => DistanceModelType::Inverse,
                    DistanceModel::Linear => DistanceModelType::Linear,
                    DistanceModel::Exponential => DistanceModelType::Exponential,
                });
                p.set_ref_distance(s.ref_distance as f64);
                p.set_max_distance(s.max_distance as f64);
                p.set_rolloff_factor(s.rolloff_factor as f64);
                p.set_position(
                    s.position.x as f64,
                    s.position.y as f64,
                    s.position.z as f64,
                );
            }
            source
                .connect_with_audio_node(&p)
                .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
            p.connect_with_audio_node(&gain)
                .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
            Some(p)
        } else {
            source
                .connect_with_audio_node(&gain)
                .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;
            None
        };

        gain.connect_with_audio_node(&self.master)
            .map_err(|e| AudioError::DeviceUnavailable(format!("connect: {:?}", e)))?;

        if settings.looped {
            source.set_loop(true);
        }
        source.playback_rate().set_value(settings.rate);
        let _ = settings.bus;
        let id = self.voices.insert(WebVoice {
            source: source.clone(),
            gain: gain.clone(),
            panner: panner.clone(),
            onended: None,
            _buffer: buffer,
        });
        let finished_queue = self.finished_queue.clone();
        let onended = Closure::wrap(Box::new(move || {
            finished_queue.borrow_mut().push(id);
        }) as Box<dyn FnMut()>);
        source.set_onended(Some(onended.as_ref().unchecked_ref()));
        if let Some(v) = self.voices.get_mut(id) {
            v.onended = Some(onended);
        }
        source
            .start()
            .map_err(|e| AudioError::DeviceUnavailable(format!("start: {:?}", e)))?;

        Ok(id)
    }

    fn set_param(&mut self, voice: VoiceId, param: VoiceParam) {
        let Some(v) = self.voices.get(voice) else {
            return;
        };
        match param {
            VoiceParam::Volume(vol) => v.gain.gain().set_value(vol),
            VoiceParam::Pan(_) => {
                // Web Audio spatial panning is handled via PannerNode.
                // We'll rely on the spatial position for precise placement.
            }
            VoiceParam::Rate(rate) => v.source.playback_rate().set_value(rate),
            VoiceParam::Position(pos) => {
                if let Some(ref p) = v.panner {
                    p.set_position(pos.x as f64, pos.y as f64, pos.z as f64);
                }
            }
            VoiceParam::StopAfterLoop => {}
            VoiceParam::FadeOut(_) => {}
        }
    }

    fn stop_voice(&mut self, voice: VoiceId) {
        if let Some(v) = self.voices.remove(voice) {
            v.source.set_onended(None);
            let _ = v.source.stop();
        }
        self.finished_queue.borrow_mut().retain(|id| *id != voice);
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        let finished: Vec<VoiceId> = self.finished_queue.borrow_mut().drain(..).collect();
        let mut cleaned = Vec::new();
        for id in finished {
            if let Some(v) = self.voices.remove(id) {
                v.source.set_onended(None);
                cleaned.push(id);
            }
        }
        cleaned
    }

    fn tick(&mut self, _dt: Duration) {}

    fn set_listener(&mut self, listener: Listener) {
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
}
