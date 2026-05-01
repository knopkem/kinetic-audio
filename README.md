# `kinetic-audio`

> Cross-platform game audio engine for Rust — identical API on native desktop (via `cpal`) and in the browser (via the Web Audio API).

**Status: pre-release / active development.** The core playback pipeline is working. Several features listed below are partially implemented or planned for upcoming milestones.

## What works today

| Feature | Native | WASM |
|---------|--------|------|
| WAV playback | ✅ | ✅ |
| Volume / pan / rate control | ✅ | ✅ |
| Looping | ✅ | ✅ |
| 3D positional audio (distance attenuation + stereo pan) | ✅ | ✅ (PannerNode) |
| Spatial listener (position + orientation) | ✅ | ✅ |
| Biquad filter DSP (7 modes, 12/24 dB slopes) | ✅ | ✅ |
| Delay line (feedback, wet/dry) | ✅ | ✅ |
| Mixer busses (gain, mute, solo) | ✅ | ✅ |
| Sound sprite definitions | ✅ | ✅ |
| Tween / easing types | ✅ | ✅ |
| NullBackend for unit tests | ✅ | ✅ |

## Known limitations (not yet implemented)

- **Tween interpolation:** `fade_volume` / `fade_pan` apply the target instantly — time-based interpolation in `update()` is a stub.
- **Sprite playback:** `SpriteData` / `SpriteRegion` types exist but `AudioManager::play_sprite()` is not yet implemented.
- **Bus effect processing:** `BiquadFilter` and `DelayLine` are fully working DSP units, but the per-bus effect chain is not yet wired into the cpal audio callback.
- **Bus routing (native):** Voices are all mixed directly to the master output; the `bus` field on `PlaybackSettings` has no effect on the native backend.
- **`PlaybackSettings::delay`:** Field is reserved but ignored during playback.
- **`FadeOut` / `StopAfterLoop`:** `VoiceParam` variants are defined but are no-ops in both backends.
- **OGG / MP3 / FLAC:** The `symphonia` feature flag is reserved; only WAV is decoded today.
- **HRTF:** The `hrtf` feature flag is reserved; no HRTF tables or DSP are included yet.

## Quick start

```toml
# Cargo.toml
[dependencies]
kinetic-audio = { version = "0.1", features = ["cpal-backend"] }

# Browser / WASM targets
[target.'cfg(target_arch = "wasm32")'.dependencies]
kinetic-audio = { version = "0.1", features = ["web-backend"] }
```

```rust
use kinetic_audio::{AudioManager, DefaultBackend, AudioConfig, PlaybackSettings};

// Create the manager (starts the audio device).
let mut manager = AudioManager::<DefaultBackend>::new(AudioConfig::default())?;

// Load a WAV file (returns a re-usable key).
let shot = manager.load_sound(include_bytes!("assets/gunshot.wav"))?;

// Play it — get back a handle for live control.
let mut handle = manager.play(shot, PlaybackSettings::default())?;

// Change parameters while playing.
handle.set_volume(0.6);
handle.set_pan(-0.4);  // slightly left

// Stop explicitly (dropping the handle does NOT stop the sound).
handle.stop();

// Call once per frame to flush commands and reclaim finished voices.
manager.update(std::time::Duration::from_millis(16));
```

## 3D positional audio

```rust
use kinetic_audio::{PlaybackSettings, SpatialSettings, DistanceModel};
use glam::Vec3;

// Place the listener in the world.
manager.set_listener_position(Vec3::new(0.0, 0.0, 0.0));
manager.set_listener_orientation(Vec3::Z, Vec3::Y);  // forward, up

// Play a sound at a world-space position.
let handle = manager.play(
    explosion,
    PlaybackSettings {
        spatial: Some(SpatialSettings {
            model: DistanceModel::Inverse,
            ref_distance: 1.0,
            max_distance: 500.0,
            rolloff_factor: 1.0,
            position: Vec3::new(120.0, 0.0, 80.0),
        }),
        ..Default::default()
    },
)?;

// Move the source in real-time.
handle.set_position(Vec3::new(125.0, 0.0, 80.0));
```

On native, attenuation and constant-power stereo panning are computed from the listener's frame of reference. On WASM, the browser's `PannerNode` handles both.

## Effects DSP

Effects are working DSP units. Bus-level wiring into the audio callback is not yet complete (see limitations above), but the types are available for standalone use:

```rust
use kinetic_audio::effects::{BiquadFilter, FilterMode, DelayLine};

// Low-pass filter at 800 Hz.
let mut lpf = BiquadFilter::new(FilterMode::LowPass, 44_100);
lpf.cutoff_hz = 800.0;
lpf.resonance = 0.707;
lpf.recalc_coefficients();

// Delay with 300 ms time, 40% feedback, 50% wet.
let mut delay = DelayLine::new(1.0 /* max time */, 44_100);
delay.time = 0.3;
delay.feedback = 0.4;
delay.mix = 0.5;
```

## Mixer busses

```rust
use kinetic_audio::MixSettings;

let sfx_bus = manager.add_bus(MixSettings {
    name: "sfx".into(),
    gain: 0.8,
    muted: false,
    soloed: false,
})?;

// Route a sound to the bus.
let handle = manager.play(
    sound,
    PlaybackSettings { track: Some(sfx_bus), ..Default::default() },
)?;

// Adjust bus volume in decibels.
manager.set_bus_volume_db(sfx_bus, -6.0);
```

> **Note:** Bus routing is not yet active in the native (cpal) backend; all voices currently mix to master.

## WASM / browser notes

The Web Audio API requires a user gesture before audio can play. Call `manager.resume()` inside your click/keydown handler:

```rust
// In your WASM event handler:
manager.resume()?;
```

## Cargo features

| Feature | Default | Description |
|---------|:-------:|-------------|
| `cpal-backend` | ✅ | Native audio device via `cpal` |
| `web-backend` | | Web Audio API backend (WASM targets) |
| `symphonia` | | OGG / MP3 / FLAC decoders *(reserved — not yet implemented)* |
| `hrtf` | | HRTF spatial tables *(reserved — not yet implemented)* |

## Minimum Rust version

1.70

## Architecture

```
User API  (AudioManager, SoundHandle, SpriteData, …)
    │
    ▼
Backend trait
    ├── CpalBackend   — native desktop (macOS / Linux / Windows)
    │     real-time audio thread, mpsc command channel
    └── WebAudioBackend — browser WASM
          AudioContext, GainNode, PannerNode, BiquadFilterNode
```

`DefaultBackend` resolves to `CpalBackend` on native and `WebAudioBackend` on WASM, so most crates can write `AudioManager::<DefaultBackend>::new(…)` and compile unchanged on both targets.

## License

MIT OR Apache-2.0
