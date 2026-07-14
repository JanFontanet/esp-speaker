use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    channel::{Receiver, Sender},
};
use heapless::String;

use super::Audio;
use crate::{
    board::{AudioResources, I2cBus},
    config::{AUDIO_QUEUE_DEPTH, CHANNEL_SIZE},
    mqtt::msg_protocol::AppEvent,
};

static AUDIO_CHANNEL: Channel<CriticalSectionRawMutex, Sound, AUDIO_QUEUE_DEPTH> = Channel::new();

pub const STREAM_URL_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCommand {
    Play,
    Pause,
    Stop,
    SetVolume(u8),
    PlayUrl(String<STREAM_URL_CAPACITY>),
}

#[derive(Clone, Copy, defmt::Format)]
pub enum Sound {
    Beep,
    Connected,
    Tone {
        frequency: u32,
        amplitude: f32,
        duration_ms: u64,
    },
}

pub fn audio_send(sound: Sound) {
    if AUDIO_CHANNEL.try_send(sound).is_err() {
        defmt::warn!("audio: queue full, dropping sound");
    }
}

pub fn audio_spawn(
    spawner: &Spawner,
    res: AudioResources<'static>,
    bus: &'static I2cBus,
    cmd_rx: Receiver<'static, CriticalSectionRawMutex, AudioCommand, CHANNEL_SIZE>,
    event_tx: Sender<'static, CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>,
) {
    spawner.spawn(audio_task(res, bus, cmd_rx, event_tx).unwrap());
}

#[embassy_executor::task]
async fn audio_task(
    res: AudioResources<'static>,
    bus: &'static I2cBus,
    cmd_rx: Receiver<'static, CriticalSectionRawMutex, AudioCommand, CHANNEL_SIZE>,
    event_tx: Sender<'static, CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>,
) {
    let mut audio = match Audio::new(res, bus).await {
        Ok(audio) => audio,
        Err(e) => {
            defmt::error!("audio: init failed: {:?}", e);
            return;
        }
    };
    defmt::info!("audio: task ready");

    // TODO: React to commands received by any channel & send events
    loop {
        // Idle here with the amplifier powered down (no idle hiss).
        let sound_fut = AUDIO_CHANNEL.receive();
        let cmd_fut = cmd_rx.receive();

        match select(sound_fut, cmd_fut).await {
            Either::First(s) => {
                // Power the amp up once, drain everything currently queued, then power
                // it back down — this avoids clicking the amp on/off between
                // back-to-back sounds.
                audio.set_output_enabled(true).await;
                let _ = event_tx.send(AppEvent::PlaybackStarted).await;

                let mut next = Some(s);
                while let Some(sound) = next {
                    if let Err(e) = audio.play(sound).await {
                        defmt::error!("audio: playback error: {:?}", e);
                    }
                    next = AUDIO_CHANNEL.try_receive().ok();
                }

                audio.set_output_enabled(false).await;
                let _ = event_tx.send(AppEvent::PlaybackStopped).await;
            }
            Either::Second(cmd) => match cmd {
                AudioCommand::Play => {
                    defmt::info!("Play received!");
                    audio.set_output_enabled(true).await;
                    let _ = event_tx.send(AppEvent::PlaybackStarted).await;
                    if let Err(_) = audio.play_connected().await {
                        defmt::error!("Error playing audio?");
                    }
                    audio.set_output_enabled(false).await;
                    let _ = event_tx.send(AppEvent::PlaybackStopped).await;
                    defmt::info!("Audio sent");
                }
                AudioCommand::Pause => defmt::warn!("audio: pause is not implemented"),
                AudioCommand::Stop => defmt::warn!("audio: stop is not implemented"),
                AudioCommand::SetVolume(_) => {
                    defmt::warn!("audio: volume control is not implemented")
                }
                AudioCommand::PlayUrl(uri) => {
                    defmt::info!("PlayUri received! {}", uri.as_str());
                }
            },
        }
    }
}
