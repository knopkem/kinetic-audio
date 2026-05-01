//! Integration tests using `NullBackend` (no real audio device required).

use kinetic_audio::{
    AudioConfig, AudioManager, MixSettings, PlaybackSettings, SpriteRegion,
    Tween, Easing,
};
use kinetic_audio::backend::null::NullBackend;
use kinetic_audio::effects::delay::DelayLine;
use std::time::Duration;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a minimal 16-bit mono WAV byte vector at 44 100 Hz, `n_frames` long.
fn make_wav(n_frames: u16) -> Vec<u8> {
    // WAV format: RIFF header + fmt chunk + data chunk.
    let data_len = n_frames as u32 * 2; // 1 channel × 2 bytes per sample
    let riff_len = 36 + data_len;
    let mut w = Vec::with_capacity(44 + data_len as usize);

    // RIFF header
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&riff_len.to_le_bytes());
    w.extend_from_slice(b"WAVE");

    // fmt chunk (PCM, 1 ch, 44100 Hz, 16-bit)
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());   // chunk size
    w.extend_from_slice(&1u16.to_le_bytes());    // PCM
    w.extend_from_slice(&1u16.to_le_bytes());    // channels
    w.extend_from_slice(&44_100u32.to_le_bytes()); // sample rate
    w.extend_from_slice(&(44_100u32 * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes());    // block align
    w.extend_from_slice(&16u16.to_le_bytes());   // bits per sample

    // data chunk
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..n_frames {
        w.extend_from_slice(&0i16.to_le_bytes()); // silence
    }
    w
}

fn null_manager() -> AudioManager<NullBackend> {
    AudioManager::new(AudioConfig::default()).expect("NullBackend::start failed")
}

// ── basic playback ────────────────────────────────────────────────────────────

#[test]
fn load_and_play_wav() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m.play(key, PlaybackSettings::default()).expect("play failed");
    // Voice should be live before any update.
    assert!(!m.is_finished(&handle));
}

#[test]
fn stop_voice_immediately() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m.play(key, PlaybackSettings::default()).expect("play failed");
    handle.stop();
    // After update the manager processes the Stop command.
    m.update(Duration::from_millis(16));
}

// ── tween interpolation ───────────────────────────────────────────────────────

#[test]
fn fade_volume_tween_advances() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let mut handle = m.play(key, PlaybackSettings::default()).expect("play failed");

    let tween = Tween {
        duration: Duration::from_millis(100),
        easing: Easing::Linear,
    };
    handle.fade_volume(0.0, tween);

    // Drive the tween for its full duration. Should not panic.
    for _ in 0..10 {
        m.update(Duration::from_millis(10));
    }
}

#[test]
fn fade_pan_tween_advances() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let mut handle = m.play(key, PlaybackSettings::default()).expect("play failed");

    let tween = Tween {
        duration: Duration::from_millis(50),
        easing: Easing::Linear,
    };
    handle.fade_pan(-1.0, tween);

    for _ in 0..5 {
        m.update(Duration::from_millis(10));
    }
}

// ── fade-out / stop-after-loop ────────────────────────────────────────────────

#[test]
fn fade_out_marks_voice_finished() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m.play(key, PlaybackSettings::default()).expect("play failed");

    handle.fade_out(Duration::from_millis(50));

    // The NullBackend marks voices finished immediately on FadeOut.
    m.update(Duration::from_millis(16));
    assert!(m.is_finished(&handle), "voice should be finished after fade-out");
}

#[test]
fn stop_after_loop_sends_command() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m.play(
        key,
        PlaybackSettings { looped: true, ..Default::default() },
    ).expect("play failed");

    handle.stop_after_loop();
    m.update(Duration::from_millis(16));
    // No panic expected; the NullBackend logs StopAfterLoop via SetParam.
}

// ── sprite playback ───────────────────────────────────────────────────────────

#[test]
fn sprite_regions_play_correctly() {
    // 44 100 Hz × 3 seconds = 132 300 frames, but keep it small for tests.
    let bytes = make_wav(1000);
    let mut m = null_manager();
    let sound = m.load_sound(&bytes, "wav").expect("load failed");

    let regions = [
        SpriteRegion {
            name: "a",
            start: Duration::from_millis(0),
            end: Duration::from_millis(5),
            looped: false,
        },
        SpriteRegion {
            name: "b",
            start: Duration::from_millis(5),
            end: Duration::from_millis(10),
            looped: false,
        },
    ];
    let sprite = m.add_sprite(sound, &regions).expect("add_sprite failed");

    let ha = m.play_sprite(sprite, "a", PlaybackSettings::default()).expect("play a");
    let hb = m.play_sprite(sprite, "b", PlaybackSettings::default()).expect("play b");

    // Both handles are live.
    assert!(!m.is_finished(&ha));
    assert!(!m.is_finished(&hb));
}

#[test]
fn unknown_sprite_region_returns_error() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let sound = m.load_sound(&bytes, "wav").expect("load failed");

    let regions = [SpriteRegion {
        name: "click",
        start: Duration::from_millis(0),
        end: Duration::from_millis(5),
        looped: false,
    }];
    let sprite = m.add_sprite(sound, &regions).expect("add_sprite failed");

    let result = m.play_sprite(sprite, "nonexistent", PlaybackSettings::default());
    assert!(result.is_err(), "expected error for unknown region");
}

// ── bus routing ───────────────────────────────────────────────────────────────

#[test]
fn add_and_remove_bus() {
    let mut m = null_manager();

    let bus_id = m
        .add_bus(MixSettings {
            name: "sfx".into(),
            gain: 1.0,
            muted: false,
            soloed: false,
        })
        .expect("add_bus failed");

    m.set_bus_volume_db(bus_id, -6.0);
    m.set_bus_muted(bus_id, true);
    m.remove_bus(bus_id);
    // No panic expected.
}

// ── effect chain ─────────────────────────────────────────────────────────────

#[test]
fn add_effect_to_bus() {
    let mut m = null_manager();

    let bus_id = m
        .add_bus(MixSettings {
            name: "reverb".into(),
            gain: 1.0,
            muted: false,
            soloed: false,
        })
        .expect("add_bus failed");

    let delay = DelayLine::new(1.0, 44_100);
    m.add_bus_effect(bus_id, Box::new(delay));
    // No panic expected.
}

// ── finished tracking ─────────────────────────────────────────────────────────

#[test]
fn finished_set_populated_by_update() {
    let bytes = make_wav(4); // very short so NullBackend would finish quickly
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");

    // NullBackend finishes voices when `finished` flag is set via FadeOut.
    let handle = m.play(key, PlaybackSettings::default()).expect("play failed");
    handle.fade_out(Duration::ZERO);

    m.update(Duration::from_millis(16));
    assert!(m.is_finished(&handle));
}
