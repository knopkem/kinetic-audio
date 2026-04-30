# `kinetic-audio`

> Cross-platform game audio engine for Rust — native (cpal) + WASM (Web Audio API).

## Features

- **Platform support:** Desktop (macOS / Linux / Windows) via `cpal`, Browser via Web Audio API.
- **Positional audio:** Distance attenuation + stereo panning, no expensive HRTF required.
- **Sound sprites:** Pack many short SFX into one file to reduce HTTP requests in WASM builds.
- **Mixer busses + per-bus effects:** Biquad EQ, delay line, expandable `Effect` trait.
- **Zero allocation during playback:** All audio-thread logic works on pre-allocated buffers.

## Quick Start

```rust
use kinetic_audio::{AudioManager, DefaultBackend, AudioConfig};

let mut manager = AudioManager::<DefaultBackend>::new(AudioConfig::default())?;
let sound = manager.load_sound(include_bytes!("gunshot.wav"))?;
let handle = manager.play(sound, Default::default())?;
```

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `cpal-backend` | ✓ | Native audio via cpal |
| `web-backend` |   | Web Audio API backend (WASM only) |
| `symphonia` |   | Enable OGG / MP3 / FLAC decoders |
| `hrtf` |   | HRTF tables for headphone spatial audio (large) |

## License

MIT OR Apache-2.0
