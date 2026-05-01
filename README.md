# `kinetic-audio`

Cross-platform game audio engine for Rust with one API for native desktop (`cpal`) and browser/WASM (Web Audio API).

**Status:** v0.1 scope / public beta. The crate is aimed at 2D games and general game-audio playback on native + wasm. Core playback, spatial audio, sprites, tweening, buses, and optional multi-format decoding are implemented.

## Feature matrix

| Feature | Native | WASM |
|---------|--------|------|
| WAV playback | ✅ | ✅ |
| OGG / MP3 / FLAC / AAC via `symphonia` | ✅ | ✅ |
| Volume / pan / rate control | ✅ | ✅ |
| Delayed playback | ✅ | ✅ |
| Pause / resume / seek | ✅ | ✅ |
| Looping, stop-after-loop, fade-out | ✅ | ✅ |
| 3D positional audio | ✅ | ✅ |
| Live listener / source movement updates | ✅ | ✅ |
| Sound sprites / named regions | ✅ | ✅ |
| Mixer bus routing, gain, mute, solo | ✅ | ✅ |
| Bus DSP chain (`add_bus_effect`) | ✅ | ⚠️ |
| Tweened volume / pan | ✅ | ✅ |
| `NullBackend` for tests | ✅ | ✅ |
| Native HRTF convolution | ❌ | n/a |

## Current limitations / non-goals for v0.1

- `add_bus_effect()` runs Rust DSP on the native `cpal` backend. The Web Audio backend currently ignores Rust-side bus effects. This is intentionally **not** a v0.1 release blocker for 2D/native+wasm game playback.
- The `hrtf` feature flag remains reserved. Native spatialization currently uses distance attenuation plus constant-power pan rather than HRTF convolution. HRTF is future work, not part of the first public release target.

## Quick start

```toml
[dependencies]
# Native desktop:
kinetic-audio = { version = "0.1", features = ["cpal-backend"] }

# Or with extra decoders:
# kinetic-audio = { version = "0.1", features = ["cpal-backend", "symphonia"] }
```

```rust
use kinetic_audio::{AudioConfig, AudioManager, DefaultBackend, PlaybackSettings};

let mut manager = AudioManager::<DefaultBackend>::new(AudioConfig::default())?;
let sound = manager.load_sound(include_bytes!("assets/gunshot.wav"), "wav")?;

let mut handle = manager.play(sound, PlaybackSettings::default())?;
handle.set_volume(0.6);
handle.set_pan(-0.4);
handle.pause();
handle.resume();
handle.seek_to(std::time::Duration::from_millis(250));

manager.update(std::time::Duration::from_millis(16));
```

## Sound sprites

```rust
use kinetic_audio::{PlaybackSettings, SpriteRegion};
use std::time::Duration;

let sound = manager.load_sound(include_bytes!("assets/ui-sheet.wav"), "wav")?;
let sprite = manager.add_sprite(
    sound,
    &[
        SpriteRegion {
            name: "click",
            start: Duration::from_millis(0),
            end: Duration::from_millis(120),
            looped: false,
        },
        SpriteRegion {
            name: "hover",
            start: Duration::from_millis(120),
            end: Duration::from_millis(260),
            looped: false,
        },
    ],
)?;

let handle = manager.play_sprite(sprite, "click", PlaybackSettings::default())?;
```

## 3D positional audio

```rust
use kinetic_audio::{DistanceModel, PlaybackSettings, SpatialSettings};
use glam::Vec3;

manager.set_listener_position(Vec3::ZERO);
manager.set_listener_orientation(Vec3::Z, Vec3::Y);

let mut handle = manager.play(
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

handle.set_position(Vec3::new(125.0, 0.0, 80.0));
```

On native, `AudioManager` recomputes gain and pan for active spatial voices as the listener or source moves. On WASM, the browser's `PannerNode` handles spatialization.

## Mixer busses

```rust
use kinetic_audio::{DelayLine, MixSettings, PlaybackSettings};

let sfx_bus = manager.add_bus(MixSettings {
    name: "sfx".into(),
    gain: 1.0,
    muted: false,
    soloed: false,
})?;

manager.add_bus_effect(sfx_bus, Box::new(DelayLine::new(1.0, 44_100)));
manager.set_bus_volume_db(sfx_bus, -6.0);

let handle = manager.play(
    sound,
    PlaybackSettings {
        track: Some(sfx_bus),
        ..Default::default()
    },
)?;
```

Bus routing, gain, mute, and solo work on both backends. Native `cpal` also runs the Rust DSP effect chain attached with `add_bus_effect()`.

## Browser / WASM notes

Web Audio requires a user gesture before playback can start. Call `resume()` inside your click / key handler:

```rust
manager.resume()?;
```

## Cargo features

| Feature | Default | Description |
|---------|:-------:|-------------|
| `cpal-backend` | ✅ | Native audio output via `cpal` |
| `web-backend` |  | Marker feature for browser builds; current Web Audio compilation is driven by `target_arch = "wasm32"` rather than this flag |
| `symphonia` |  | OGG / MP3 / FLAC / AAC decoding |
| `hrtf` |  | Reserved for future HRTF assets / processing |

`cpal-backend` has the only checkmark because it is the only default feature. `web-backend` is intentionally unchecked: the browser backend works on `wasm32`, but today that support comes from target-specific dependencies and `cfg(target_arch = "wasm32")` code paths, not from the feature flag itself.

## Examples

- `cargo run --example basic_playback`
- `cargo run --example sprites`

## Minimum Rust version

1.70

## Architecture

```text
AudioManager / SoundHandle / SpriteData
                │
                ▼
            Backend trait
         ┌────────┴────────┐
         ▼                 ▼
    CpalBackend      WebAudioBackend
```

`DefaultBackend` resolves to `CpalBackend` on native and `WebAudioBackend` on `wasm32`, so most game code can compile unchanged on both targets.

## License

MIT OR Apache-2.0
