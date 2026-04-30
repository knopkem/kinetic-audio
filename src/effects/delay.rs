//! Simple delay line with feedback.

use crate::effects::Effect;
use crate::math::Frame;

/// A circular-buffer delay line.
#[derive(Clone, Debug)]
pub struct DelayLine {
    /// Delay time in seconds.
    pub time: f32,
    /// Feedback amount (0.0 = 1 echo, 1.0 = infinite, >1.0 = unstable).
    pub feedback: f32,
    /// Wet/dry mix (0.0 = fully dry, 1.0 = fully wet).
    pub mix: f32,
    // Internal buffer.
    buffer: Vec<Frame>,
    // Write head.
    write: usize,
    // Cached sample rate.
    sample_rate: u32,
}

impl DelayLine {
    /// Create a delay line. `max_time` reserves the worst-case buffer size.
    pub fn new(max_time: f32, sample_rate: u32) -> Self {
        let samples = (max_time * sample_rate as f32).ceil() as usize;
        Self {
            time: 0.0,
            feedback: 0.3,
            mix: 0.5,
            buffer: vec![Frame::SILENCE; samples.max(1)],
            write: 0,
            sample_rate,
        }
    }

    /// Reset the delay buffer to silence.
    pub fn clear(&mut self) {
        for f in &mut self.buffer {
            *f = Frame::SILENCE;
        }
        self.write = 0;
    }

    /// Read from the delay line at the current `time` offset.
    fn read(&self) -> Frame {
        let delay_samples = self.time * self.sample_rate as f32;
        let len = self.buffer.len();
        let idx_f = (self.write as f32) - delay_samples;
        let idx = idx_f.rem_euclid(len as f32);
        let i0 = idx.floor() as usize % len;
        let i1 = (i0 + 1) % len;
        let frac = idx - idx.floor();
        // Linear interpolation.
        Frame {
            l: self.buffer[i0].l + frac * (self.buffer[i1].l - self.buffer[i0].l),
            r: self.buffer[i0].r + frac * (self.buffer[i1].r - self.buffer[i0].r),
        }
    }

    /// Write one frame into the buffer.
    fn write(&mut self, frame: Frame) {
        self.buffer[self.write] = frame;
        self.write = (self.write + 1) % self.buffer.len();
    }
}

impl Effect for DelayLine {
    fn name(&self) -> &str {
        "DelayLine"
    }

    fn process(&mut self, input: &mut [Frame], rate: u32) {
        if rate != self.sample_rate {
            self.sample_rate = rate;
            // Resize buffer if necessary — simplistic: just clear.
            self.clear();
        }
        for frame in input.iter_mut() {
            let delayed = self.read();
            // Feedback into the delay line.
            let to_buffer = Frame::mix(*frame, delayed.scale(self.feedback));
            self.write(to_buffer);
            // Wet/dry mix.
            *frame = Frame::mix(frame.scale(1.0 - self.mix), delayed.scale(self.mix));
        }
    }

    fn reset(&mut self) {
        self.clear();
    }
}
