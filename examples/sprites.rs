//! Audio sprite example.
//!
//! Loads a single audio file that contains multiple packed sounds and plays
//! each one using its named region.
//!
//! Run with:
//!   cargo run --example sprites

use kinetic_audio::{AudioConfig, AudioManager, PlaybackSettings, SpriteRegion};
use kinetic_audio::backend::cpal::CpalBackend;
use std::time::Duration;

// Sprite sheet layout (milliseconds):
// 0 – 500 ms   : "click"
// 500 – 1000 ms: "whoosh"
// 1000 – 1500 ms: "hit"
const REGIONS: &[SpriteRegion] = &[
    SpriteRegion {
        name: "click",
        start: Duration::from_millis(0),
        end: Duration::from_millis(500),
        looped: false,
    },
    SpriteRegion {
        name: "whoosh",
        start: Duration::from_millis(500),
        end: Duration::from_millis(1000),
        looped: false,
    },
    SpriteRegion {
        name: "hit",
        start: Duration::from_millis(1000),
        end: Duration::from_millis(1500),
        looped: false,
    },
];

fn main() {
    let wav_bytes = match std::fs::read("assets/sprite_sheet.wav") {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "Place a WAV file at assets/sprite_sheet.wav and re-run.\n\
                 The file should be at least 1.5 seconds long."
            );
            std::process::exit(1);
        }
    };

    let mut manager: AudioManager<CpalBackend> =
        AudioManager::new(AudioConfig::default()).expect("failed to create AudioManager");

    let sound = manager
        .load_sound(&wav_bytes, "wav")
        .expect("failed to decode WAV");

    let sprite = manager
        .add_sprite(sound, REGIONS)
        .expect("failed to register sprite regions");

    for region_name in &["click", "whoosh", "hit"] {
        println!("Playing region: {region_name}");

        let handle = manager
            .play_sprite(sprite, region_name, PlaybackSettings::default())
            .expect("failed to play sprite region");

        // Wait for the region to finish playing.
        loop {
            manager.update(Duration::from_millis(16));
            if manager.is_finished(&handle) {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        }

        // Brief pause between regions.
        std::thread::sleep(Duration::from_millis(200));
    }

    println!("Done.");
}
