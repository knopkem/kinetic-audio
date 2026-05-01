//! Basic playback example.
//!
//! Loads a WAV file from `assets/click.wav`, plays it at full volume, waits
//! for it to finish, then exits cleanly.
//!
//! Run with:
//!   cargo run --example basic_playback

use kinetic_audio::{AudioConfig, AudioManager, PlaybackSettings};
use kinetic_audio::backend::cpal::CpalBackend;
use std::time::Duration;

fn main() {
    let wav_bytes = match std::fs::read("assets/click.wav") {
        Ok(b) => b,
        Err(_) => {
            eprintln!("Place a WAV file at assets/click.wav and re-run.");
            std::process::exit(1);
        }
    };

    let mut manager: AudioManager<CpalBackend> =
        AudioManager::new(AudioConfig::default()).expect("failed to create AudioManager");

    let sound = manager
        .load_sound(&wav_bytes, "wav")
        .expect("failed to decode WAV");

    let handle = manager
        .play(sound, PlaybackSettings::default())
        .expect("failed to play");

    println!("Playing… (waiting for natural end)");

    loop {
        manager.update(Duration::from_millis(16));

        if manager.is_finished(&handle) {
            break;
        }

        std::thread::sleep(Duration::from_millis(16));
    }

    println!("Done.");
}
