//! Audio actor: owns the I2S/codec hardware in a background task and plays
//! sounds enqueued from anywhere via [`audio_send`].
//!
//! This mirrors the LED subsystem's task-based design: callers never touch the
//! `Audio` driver directly — they fire-and-forget [`Sound`]s through a channel,
//! keeping the caller (and the main loop) responsive.

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

use super::Audio;
use crate::board::{AudioResources, I2cBus};

/// How many pending sounds may be queued before new ones are dropped.
const QUEUE_DEPTH: usize = 8;

static AUDIO_CHANNEL: Channel<CriticalSectionRawMutex, Sound, QUEUE_DEPTH> = Channel::new();

/// A sound the audio task knows how to play. Queued sounds play sequentially.
#[derive(Clone, Copy, defmt::Format)]
pub enum Sound {
    /// Short 440 Hz beep.
    Beep,
    /// Rising jingle played on Wi-Fi connect.
    Connected,
    /// Raw sine tone.
    Tone {
        frequency: u32,
        amplitude: f32,
        duration_ms: u64,
    },
}

/// Queue a sound for playback. Non-blocking; if the queue is full the sound is
/// dropped (playback should never stall the caller).
pub fn audio_send(sound: Sound) {
    if AUDIO_CHANNEL.try_send(sound).is_err() {
        defmt::warn!("audio: queue full, dropping sound");
    }
}

/// Spawn the audio task. Call once from main.
pub fn audio_spawn(spawner: &Spawner, res: AudioResources<'static>, bus: &'static I2cBus) {
    spawner.spawn(audio_task(res, bus).unwrap());
}

#[embassy_executor::task]
async fn audio_task(res: AudioResources<'static>, bus: &'static I2cBus) {
    let mut audio = match Audio::new(res, bus).await {
        Ok(audio) => audio,
        Err(e) => {
            defmt::error!("audio: init failed: {:?}", e);
            return;
        }
    };
    defmt::info!("audio: task ready");

    loop {
        // Idle here with the amplifier powered down (no idle hiss).
        let sound = AUDIO_CHANNEL.receive().await;

        // Power the amp up once, drain everything currently queued, then power
        // it back down — this avoids clicking the amp on/off between
        // back-to-back sounds.
        audio.set_output_enabled(true).await;
        let mut next = Some(sound);
        while let Some(sound) = next {
            if let Err(e) = audio.play(sound).await {
                defmt::error!("audio: playback error: {:?}", e);
            }
            next = AUDIO_CHANNEL.try_receive().ok();
        }
        audio.set_output_enabled(false).await;
    }
}
