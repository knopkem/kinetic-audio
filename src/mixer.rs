//! Mixer: tracks, busses, and routing.

use crate::math::Frame;

/// Opaque handle to a mixer bus.
pub type BusHandle = slotmap::DefaultKey;

/// Opaque handle to a mixer track.
pub type TrackHandle = slotmap::DefaultKey;

/// Internal bus identifier used by the backend.
pub(crate) type BusId = BusHandle;

/// Configuration for a new sub-master bus.
#[derive(Clone, Debug)]
pub struct MixSettings {
    /// Human-readable name (e.g. `"sfx"`, `"music"`, `"ui"`).
    pub name: String,
    /// Initial linear gain.
    pub gain: f32,
    /// Whether the bus is muted.
    pub muted: bool,
    /// Whether the bus is soloed (when any bus is soloed, only soloed busses are heard).
    pub soloed: bool,
}

impl Default for MixSettings {
    fn default() -> Self {
        Self {
            name: "untitled".into(),
            gain: 1.0,
            muted: false,
            soloed: false,
        }
    }
}

/// A mixer bus that collects voices, applies per-bus effects, and routes to
/// the master output (or to another bus for nested submixes).
pub struct Bus {
    pub(crate) id: BusId,
    pub(crate) settings: MixSettings,
}

/// Configuration for a per-voice send to a bus.
#[derive(Clone, Debug, Default)]
pub struct Send {
    /// Target bus.
    pub bus: BusId,
    /// Send amount (0.0 = dry, 1.0 = full wet).
    pub amount: f32,
    /// Pre-fader vs post-fader.
    pub pre_fader: bool,
}

// ── Utility ------------------------------------------------------------------

/// Mix a vector of `Frame` buffers together.
pub fn mix_buffers(buffers: &mut Vec<Vec<Frame>>) {
    if buffers.is_empty() {
        return;
    }
    // Find the longest buffer to determine output length.
    let len = buffers.iter().map(|b| b.len()).max().unwrap_or(0);
    for buf in buffers.iter_mut() {
        buf.resize(len, Frame::SILENCE);
    }
    // Sum all buffers into the first one.
    for i in 0..len {
        let mut acc = Frame::SILENCE;
        for buf in buffers.iter() {
            acc = acc + buf[i];
        }
        buffers[0][i] = acc.clamp();
    }
    // Truncate to first.
    buffers.truncate(1);
}
