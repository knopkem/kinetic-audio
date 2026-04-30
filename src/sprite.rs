//! Sound sprites: single buffer, multiple named regions.
//!
//! A sprite sheet for audio — useful for packing many short SFX into one
//! file and reducing HTTP requests in WASM builds.

use std::sync::Arc;
use std::time::Duration;

use slotmap::SlotMap;

use crate::math::{duration_to_samples, Frame};
use crate::sound::{PlaybackSettings, SoundData, SoundKey};

/// Opaque key referencing a loaded sprite.
pub type SpriteKey = slotmap::DefaultKey;

/// One region inside a [`SpriteData`] buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteRegion {
    /// Human-readable name (e.g. `"explosion_small"`).
    pub name: &'static str,
    /// Region start (inclusive).
    pub start: Duration,
    /// Region end (exclusive).
    pub end: Duration,
    /// Whether the region loops when played.
    pub looped: bool,
}

/// A decoded audio buffer with one or more named slices.
#[derive(Clone, Debug)]
pub struct SpriteData {
    /// Underlying buffer.
    pub(crate) sound: SoundKey,
    /// Sample rate of the buffer.
    pub(crate) sample_rate: u32,
    /// Named regions.
    pub(crate) regions: Vec<SpriteDef>,
}

/// Internal definition for a sprite region after upload.
#[derive(Clone, Debug)]
pub(crate) struct SpriteDef {
    pub name: String,
    pub start_sample: usize,
    pub end_sample: usize,
    pub looped: bool,
}

impl SpriteData {
    /// Define slices over an existing [`SoundData`].
    pub fn from_sound(sound: SoundKey, sample_rate: u32, regions: &[SpriteRegion]) -> Self {
        let defs = regions
            .iter()
            .map(|r| SpriteDef {
                name: r.name.to_string(),
                start_sample: duration_to_samples(r.start, sample_rate),
                end_sample: duration_to_samples(r.end, sample_rate),
                looped: r.looped,
            })
            .collect();
        Self {
            sound,
            sample_rate,
            regions: defs,
        }
    }

    /// Resolve a region by name.
    pub fn region(&self, name: &str) -> Option<&SpriteDef> {
        self.regions.iter().find(|r| r.name == name)
    }

    /// Number of defined regions.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

/// A handle to a sprite region ready for playback.
pub struct SpriteHandle {
    pub(crate) sprite: SpriteKey,
    pub(crate) region: String,
    pub(crate) settings: PlaybackSettings,
}

impl SpriteHandle {
    /// Name of the region this handle targets.
    pub fn region_name(&self) -> &str {
        &self.region
    }
}
