# `kinetic-audio` Project Plan

**Date:** April 29, 2026
**Purpose:** Cross-platform game audio engine for Rust (native + WASM)

This plan is the source of truth for the initial v0.1 implementation. It documents architecture, APIs, phased rollout, and how it connects to existing projects.

## 1. Problem Statement

No existing Rust crate provides a kira-quality game-audio API that runs on both native (desktop) and WASM (browser):
- **rodio** - Native-only (depends on cpal). Open GitHub issues #313, #489 confirm no WASM path.
- **kira** - Compiles for WASM but has no working audio backend. README says "Kira can also be used in wasm environments with limitations... Static sounds cannot be loaded from files."

The browser's Web Audio API is actually more feature-rich than most desktop APIs (built-in effects, spatialization, analysis), but there is no Rust game-audio crate that wraps it.

## 2. Solution Overview

Build `kinetic-audio` - a new crate with **backend-trait architecture** (like wgpu/winit).

Two backends shipped:
1. **Native:** `cpal` (real-time cross-platform audio I/O)
2. **WASM:** `web-sys` Web Audio API bindings

User-facing API is **identical** on both platforms. Games compile and run unchanged on native vs. browser.

## 3. Architecture

```
User-Facing API
- AudioManager::new()
- AudioManager::load_sound()
- AudioManager::play()
- SoundHandle::set_volume()
- SoundHandle::set_pan()
- SoundHandle::set_position()  <-- 3D spatial audio
- SoundHandle::set_filter()
- Track::add_effect()
- Listener::set_position()
|
Backend Trait (Backend)
- device_start()
- upload_buffer()
- play_instance()
- set_voice_param()
- stop_voice()
- tick()
|
+---------------+---------------+
|               |               |
CpalBackend   WebAudioBackend
(Native only)  (WASM only)
- cpal Stream   - AudioContext
- audio thread  - GainNode
- ring buffer   - PannerNode
                - BiquadFilterNode
                - ConvolverNode
```

## 4. Feature Set (v0.1)

| Feature | Status | Notes |
|---------|--------|-------|
| Sound Data (WAV decode) | Required | Via `hound` crate |
| Voice Pool (256 max) | Required | Configurable |
| Per-voice volume/pan/rate | Required | |
| **3D Positional Audio (HRTF)** | **Day 1 requirement** | PannerNode on WASM, custom DSP on native |
| **Sound Sprites** | **Day 1 requirement** | Single buffer, multiple named regions |
| **Effect Chain** | **Day 1 requirement** | BiquadFilter per voice, Delay, Reverb send |
| Mixer Busses | Required | Master + named sub-busses |
| Tween System | Required | Easing curves for volume/pan/param transitions |
| Spatial Listener | Required | Position + orientation |
| Sound Sprites | Required | One buffer, multiple start/end regions |

## 5. API Design

See `src/lib.rs` and other source files for the actual implementation.

Key types:
- `AudioManager<B: Backend>` - Main control surface
- `SoundData` - Decoded audio buffer
- `SoundHandle` - Playing instance control
- `Frame` - Stereo f32 sample
- `Tween` / `Easing` - Smooth parameter transitions
- `Listener` / `SpatialSettings` - 3D audio positioning
- `SpriteData` / `SpriteRegion` - Sound sprite definitions

## 6. Phased Implementation

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| P0: Scaffold | 2 days | Crate compiles on native + WASM, Backend trait, NullBackend |
| P1: Native Audio | 5 days | cpal backend, ring buffer, voice mixing, WAV playback |
| P2: WASM Audio | 5 days | Web Audio backend, decode_audio_data, GainNode, PannerNode |
| P3: Game Features | 5 days | Sprites, mixer, effects, tweens, spatial, listener |
| P4: Polish | 3 days | Benchmarks, docs, tuning |
| P5: Publish | 1 day | crates.io, README, CI |

**Total: ~3 weeks for v0.1**

## 7. Dependencies

### Core (all platforms)
- `slotmap` - Handle management
- `web-time` - Instant/Duration on WASM
- `log` - Logging facade
- `glam` - Vec3 math

### Native-only
- `cpal` - Audio I/O
- `rtrb` - Real-time ring buffer

### WASM-only
- `wasm-bindgen` - JS interop
- `wasm-bindgen-futures` - Async bridge
- `js-sys` - JS Promise handling
- `web-sys` - Web Audio API bindings

### Optional
- `hound` - WAV decoding [feature: wav]
- `symphonia` - MP3/OGG/FLAC decoding [feature: symphonia]

## 8. Project Status

**Current phase: P0 (Scaffolding)**

Files written so far:
- `Cargo.toml` - Feature flags, backends, decoders
- `src/lib.rs` - Public API exports, DefaultBackend alias
- `src/math.rs` - Frame, Decibels, Panning, Tween/Easing

Files remaining:
- `src/backend/mod.rs` - Backend trait
- `src/backend/null.rs` - Test backend
- `src/sound.rs` - SoundData, SoundHandle, SoundKey
- `src/sprite.rs` - SpriteData, SpriteRegion, sprite playback
- `src/mixer.rs` - Bus/Track system
- `src/spatial.rs` - Listener, SpatialSettings
- `src/effects.rs` - Effect trait, BiquadFilter, Delay
- `src/manager.rs` - AudioManager implementation
- `src/decode/mod.rs` - WAV decoder integration

## 9. Integration with Strategy Game

Once `kinetic-audio` is ready, the RTS migration is:

```rust
// Replace rodio with kinetic-audio in crates/app/Cargo.toml

// Before (native only):
#[cfg(not(target_arch = "wasm32"))]
use rodio::{OutputStream, Sink};

// After (unified):
use kinetic_audio::{AudioManager, DefaultBackend};
```

WASM builds will suddenly have real positional audio for explosions, gunfire, and unit deaths.

## 10. License & Distribution

- License: MIT OR Apache-2.0
- Repository: github.com/your-username/kinetic-audio
- crates.io: `kinetic-audio`

---

**Plan Generated:** April 29, 2026
