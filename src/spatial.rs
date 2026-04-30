//! 3D spatial audio types.

pub use glam::Vec3;

/// Settings for a spatial voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialSettings {
    /// Fall-off model.
    pub model: DistanceModel,
    /// Distance at which gain starts attenuating.
    pub ref_distance: f32,
    /// Max distance beyond which gain won't drop further.
    pub max_distance: f32,
    /// Rolloff factor.
    pub rolloff_factor: f32,
    /// World-space position of the sound source.
    pub position: Vec3,
}

impl Default for SpatialSettings {
    fn default() -> Self {
        Self {
            model: DistanceModel::Inverse,
            ref_distance: 1.0,
            max_distance: 10_000.0,
            rolloff_factor: 1.0,
            position: Vec3::ZERO,
        }
    }
}

/// Distance attenuation algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceModel {
    /// `1 / (ref + rolloff * (dist - ref))`
    Inverse,
    /// `1 - rolloff * (dist - ref) / (max - ref)`
    Linear,
    /// `pow(dist / ref, -rolloff)`
    Exponential,
}

impl DistanceModel {
    /// Compute attenuation gain for a given distance.
    pub fn evaluate(self, distance: f32, ref_d: f32, max_d: f32, rolloff: f32) -> f32 {
        if distance <= ref_d {
            return 1.0;
        }
        match self {
            DistanceModel::Inverse => 1.0 / (1.0 + rolloff * (distance - ref_d)),
            DistanceModel::Linear => {
                let denom = max_d - ref_d;
                if denom <= 0.0 {
                    return 1.0;
                }
                1.0 - rolloff * (distance - ref_d) / denom
            }
            DistanceModel::Exponential => (distance / ref_d).powf(-rolloff),
        }
        .clamp(0.0, 1.0)
    }
}

/// Listener state for the 3D scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    /// World-space position.
    pub position: Vec3,
    /// Forward direction (normalized).
    pub forward: Vec3,
    /// Up direction (normalized, orthogonal to forward).
    pub up: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            forward: Vec3::Z,
            up: Vec3::Y,
        }
    }
}

impl Listener {
    /// Right-hand side basis vector.
    pub fn right(&self) -> Vec3 {
        self.up.cross(self.forward).normalize_or_zero()
    }
}
