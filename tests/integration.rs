//! Integration tests using `NullBackend` (no real audio device required).

use kinetic_audio::backend::null::{BackendLog, NullBackend};
use kinetic_audio::effects::delay::DelayLine;
use kinetic_audio::{
    AudioConfig, AudioManager, BufferHandle, Easing, Frame, MixSettings, PlaybackSettings,
    SpriteRegion, Tween,
};
use std::sync::Arc;
use std::time::Duration;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a minimal 16-bit mono WAV byte vector at 44 100 Hz, `n_frames` long.
fn make_wav(n_frames: u16) -> Vec<u8> {
    make_wav_at(n_frames, 44_100)
}

/// Build a minimal 16-bit mono WAV byte vector at `sample_rate`, `n_frames` long.
fn make_wav_at(n_frames: u16, sample_rate: u32) -> Vec<u8> {
    // WAV format: RIFF header + fmt chunk + data chunk.
    let data_len = n_frames as u32 * 2; // 1 channel × 2 bytes per sample
    let riff_len = 36 + data_len;
    let mut w = Vec::with_capacity(44 + data_len as usize);

    // RIFF header
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&riff_len.to_le_bytes());
    w.extend_from_slice(b"WAVE");

    // fmt chunk (PCM, 1 ch, `sample_rate`, 16-bit)
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // channels
    w.extend_from_slice(&sample_rate.to_le_bytes()); // sample rate
    w.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

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
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");
    // Voice should be live before any update.
    assert!(!m.is_finished(&handle));
}

#[test]
fn stop_voice_immediately() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");
    handle.stop();
    // After update the manager processes the Stop command.
    m.update(Duration::from_millis(16));
}

#[test]
fn pause_resume_and_position_tracking_work() {
    let bytes = make_wav(10_000);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");

    m.update(Duration::from_millis(100));
    let before_pause = m.playback_position(&handle).expect("position missing");
    assert!(before_pause >= Duration::from_millis(95));

    handle.pause();
    m.update(Duration::from_millis(50));
    let paused_position = m.playback_position(&handle).expect("position missing");
    assert_eq!(paused_position, before_pause);
    assert!(m.is_paused(&handle));

    handle.resume();
    m.update(Duration::from_millis(50));
    let after_resume = m.playback_position(&handle).expect("position missing");
    assert!(after_resume > paused_position);
    assert!(!m.is_paused(&handle));
}

#[test]
fn seek_controls_position() {
    let bytes = make_wav(10_000);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");

    handle.seek_to(Duration::from_millis(150));
    m.update(Duration::ZERO);
    let pos = m.playback_position(&handle).expect("position missing");
    assert!(pos >= Duration::from_millis(149));
    assert!(pos <= Duration::from_millis(151));

    handle.seek_by_seconds(-0.05);
    m.update(Duration::ZERO);
    let pos = m.playback_position(&handle).expect("position missing");
    assert!(pos >= Duration::from_millis(99));
    assert!(pos <= Duration::from_millis(101));
}

// ── tween interpolation ───────────────────────────────────────────────────────

#[test]
fn fade_volume_tween_advances() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let mut handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");

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
    let mut handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");

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
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");

    handle.fade_out(Duration::from_millis(50));

    // The NullBackend marks voices finished immediately on FadeOut.
    m.update(Duration::from_millis(16));
    assert!(
        m.is_finished(&handle),
        "voice should be finished after fade-out"
    );
}

#[test]
fn stop_after_loop_sends_command() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m
        .play(
            key,
            PlaybackSettings {
                looped: true,
                ..Default::default()
            },
        )
        .expect("play failed");

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

    let ha = m
        .play_sprite(sprite, "a", PlaybackSettings::default())
        .expect("play a");
    let hb = m
        .play_sprite(sprite, "b", PlaybackSettings::default())
        .expect("play b");

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

#[test]
fn soloed_bus_mutes_other_buses() {
    let mut m = null_manager();
    let a = m
        .add_bus(MixSettings {
            name: "a".into(),
            gain: 1.0,
            muted: false,
            soloed: false,
        })
        .expect("add_bus a failed");
    let b = m
        .add_bus(MixSettings {
            name: "b".into(),
            gain: 1.0,
            muted: false,
            soloed: false,
        })
        .expect("add_bus b failed");

    m.set_bus_soloed(a, true);

    assert_eq!(m.backend().bus_configs.get(&a), Some(&(1.0, false)));
    assert_eq!(m.backend().bus_configs.get(&b), Some(&(1.0, true)));

    m.set_bus_soloed(a, false);

    assert_eq!(m.backend().bus_configs.get(&a), Some(&(1.0, false)));
    assert_eq!(m.backend().bus_configs.get(&b), Some(&(1.0, false)));
}

#[test]
fn playback_track_routes_to_requested_bus() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let bus_id = m
        .add_bus(MixSettings {
            name: "ui".into(),
            gain: 1.0,
            muted: false,
            soloed: false,
        })
        .expect("add_bus failed");

    let handle = m
        .play(
            key,
            PlaybackSettings {
                track: Some(bus_id),
                ..Default::default()
            },
        )
        .expect("play failed");

    let voice = m.backend().voices.get(handle.id()).expect("voice missing");
    assert_eq!(voice.settings.bus, Some(bus_id));
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
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");
    handle.fade_out(Duration::ZERO);

    m.update(Duration::from_millis(16));
    assert!(m.is_finished(&handle));
}

// ── sample-rate conversion ────────────────────────────────────────────────────

#[test]
fn load_converts_source_audio_to_the_device_rate() {
    // 0.1 s of audio at 32 kHz loaded into a 44.1 kHz device.
    let bytes = make_wav_at(3_200, 32_000);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");

    let sound = m.get_sound(key).expect("sound should be loaded");
    assert_eq!(sound.sample_rate(), 44_100);
    let expected = (3_200.0 * 44_100.0 / 32_000.0) as i64;
    let len = sound.len_samples() as i64;
    assert!(
        (len - expected).abs() <= 1,
        "resampled length {len}, expected ~{expected}"
    );
    let secs = sound.duration().as_secs_f64();
    assert!(
        (secs - 0.1).abs() < 0.001,
        "duration {secs} s, expected 0.1 s"
    );
}

#[test]
fn upload_reports_the_device_rate_to_the_backend() {
    let bytes = make_wav_at(3_200, 32_000);
    let mut m = null_manager();
    m.load_sound(&bytes, "wav").expect("load failed");
    let rate = m
        .backend()
        .log
        .iter()
        .rev()
        .find_map(|entry| match entry {
            BackendLog::Upload(_, _, rate) => Some(*rate),
            _ => None,
        })
        .expect("an upload should be logged");
    assert_eq!(rate, 44_100, "backend must receive device-rate audio");
}

#[test]
fn raw_upload_at_the_device_rate_is_not_resampled() {
    let mut m = null_manager();
    let key = m.load_raw(Arc::new(vec![Frame::mono(0.25); 1_000]), 44_100);
    assert_eq!(m.get_sound(key).unwrap().len_samples(), 1_000);
}

#[test]
fn raw_upload_at_a_mismatched_rate_is_converted() {
    let mut m = null_manager();
    let key = m.load_raw(Arc::new(vec![Frame::mono(0.25); 1_000]), 32_000);
    let sound = m.get_sound(key).expect("sound should be loaded");
    assert_eq!(sound.sample_rate(), 44_100);
    let expected = (1_000.0 * 44_100.0 / 32_000.0) as i64;
    let len = sound.len_samples() as i64;
    assert!(
        (len - expected).abs() <= 1,
        "resampled length {len}, expected ~{expected}"
    );
}

// ── unloading ─────────────────────────────────────────────────────────────────

/// Handle recorded by the most recent buffer upload.
fn last_upload(m: &AudioManager<NullBackend>) -> BufferHandle {
    m.backend()
        .log
        .iter()
        .rev()
        .find_map(|entry| match entry {
            BackendLog::Upload(handle, _, _) => Some(*handle),
            _ => None,
        })
        .expect("a buffer upload should be logged")
}

#[test]
fn unload_sound_releases_the_backend_buffer() {
    let bytes = make_wav(1_000);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let handle = m
        .play(key, PlaybackSettings::default())
        .expect("play failed");
    let uploaded = last_upload(&m);

    assert!(m.unload_sound(key));

    assert!(m.get_sound(key).is_none(), "sound should be gone");
    assert!(
        m.backend().buffers.get(uploaded).is_none(),
        "backend should drop its reference"
    );
    assert!(m.backend().log.contains(&BackendLog::Release(uploaded)));
    assert!(
        m.play(key, PlaybackSettings::default()).is_err(),
        "reusing an unloaded key must fail"
    );
    // Playback started before the unload is not interrupted.
    assert!(!m.is_finished(&handle));
}

#[test]
fn unloading_twice_reports_the_sound_already_gone() {
    let bytes = make_wav(100);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    assert!(m.unload_sound(key));
    assert!(!m.unload_sound(key));
}

#[test]
fn unload_removes_sprites_registered_over_the_sound() {
    let bytes = make_wav(1_000);
    let mut m = null_manager();
    let key = m.load_sound(&bytes, "wav").expect("load failed");
    let sprite = m
        .add_sprite(
            key,
            &[SpriteRegion {
                name: "hit",
                start: Duration::ZERO,
                end: Duration::from_millis(20),
                looped: false,
            }],
        )
        .expect("sprite should register");

    assert!(m.unload_sound(key));
    assert!(m
        .play_sprite(sprite, "hit", PlaybackSettings::default())
        .is_err());
}
