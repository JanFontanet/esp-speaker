use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    channel::{Receiver, Sender},
};
use heapless::{String, Vec};

use super::Audio;
use crate::{
    board::{AudioResources, I2cBus},
    config::{AUDIO_QUEUE_DEPTH, CHANNEL_SIZE},
    mqtt::msg_protocol::AppEvent,
};

static AUDIO_CHANNEL: Channel<CriticalSectionRawMutex, Sound, AUDIO_QUEUE_DEPTH> = Channel::new();
static ANNOUNCEMENT_CHANNEL: Channel<CriticalSectionRawMutex, [u8; 256], AUDIO_QUEUE_DEPTH> =
    Channel::new();

pub const STREAM_URL_CAPACITY: usize = 256;
pub const ANNOUNCEMENT_PAYLOAD_CAPACITY: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCommand {
    Play,
    Pause,
    Stop,
    SetVolume(u8),
    PlayUrl(String<STREAM_URL_CAPACITY>),
    PlayAnnouncement(String<ANNOUNCEMENT_PAYLOAD_CAPACITY>),
    PlayAnnouncementChunk([u8; 256]),
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

pub fn audio_send_announcement_chunk(chunk: [u8; 256]) {
    if ANNOUNCEMENT_CHANNEL.try_send(chunk).is_err() {
        defmt::warn!("audio: announcement queue full, dropping chunk");
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

    loop {
        // Idle here with the amplifier powered down (no idle hiss).
        let sound_fut = AUDIO_CHANNEL.receive();
        let announcement_fut = ANNOUNCEMENT_CHANNEL.receive();
        let cmd_fut = cmd_rx.receive();

        match select3(sound_fut, announcement_fut, cmd_fut).await {
            Either3::First(s) => {
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
            Either3::Second(chunk) => {
                audio.set_output_enabled(true).await;
                let _ = event_tx.send(AppEvent::PlaybackStarted).await;
                if let Err(e) = audio.play_announcement_chunk(&chunk).await {
                    defmt::error!("audio: announcement playback error: {:?}", e);
                }
                audio.set_output_enabled(false).await;
                let _ = event_tx.send(AppEvent::PlaybackStopped).await;
            }
            Either3::Third(cmd) => match cmd {
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
                AudioCommand::SetVolume(level) => {
                    defmt::info!("audio: volume set to {}", level);
                    audio.set_volume(level).await;
                }
                AudioCommand::PlayUrl(uri) => {
                    defmt::info!("PlayUri received! {}", uri.as_str());
                }
                AudioCommand::PlayAnnouncement(payload) => {
                    if let Some(bytes) = decode_announcement_payload(payload.as_str()) {
                        let mut offset = 0usize;
                        while offset < bytes.len() {
                            let end = (offset + 256).min(bytes.len());
                            let mut chunk = [0u8; 256];
                            chunk[..end - offset].copy_from_slice(&bytes[offset..end]);
                            if let Err(e) =
                                audio.play_announcement_chunk(&chunk[..end - offset]).await
                            {
                                defmt::error!("audio: announcement playback error: {:?}", e);
                                break;
                            }
                            offset = end;
                        }
                    } else {
                        defmt::warn!("audio: announcement payload was not decodable");
                    }
                }
                AudioCommand::PlayAnnouncementChunk(chunk) => {
                    if let Err(e) = audio.play_announcement_chunk(&chunk).await {
                        defmt::error!("audio: announcement playback error: {:?}", e);
                    }
                }
            },
        }
    }
}

fn decode_announcement_payload(payload: &str) -> Option<Vec<u8, ANNOUNCEMENT_PAYLOAD_CAPACITY>> {
    let cleaned = payload.trim();
    if cleaned.is_empty() {
        return None;
    }

    if let Some(bytes) = decode_hex(cleaned) {
        return Some(bytes);
    }

    decode_base64(cleaned)
}

fn decode_hex(payload: &str) -> Option<Vec<u8, ANNOUNCEMENT_PAYLOAD_CAPACITY>> {
    let mut bytes = Vec::<u8, ANNOUNCEMENT_PAYLOAD_CAPACITY>::new();
    let mut chars = payload
        .as_bytes()
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace());

    while let Some(high) = chars.next() {
        let low = chars.next()?;
        let high_nibble = nibble_value(high)?;
        let low_nibble = nibble_value(low)?;
        bytes.push((high_nibble << 4) | low_nibble).ok()?;
    }

    Some(bytes)
}

fn decode_base64(payload: &str) -> Option<Vec<u8, ANNOUNCEMENT_PAYLOAD_CAPACITY>> {
    let mut out = Vec::<u8, ANNOUNCEMENT_PAYLOAD_CAPACITY>::new();
    let mut chars = [0u8; 4];
    let mut count = 0usize;

    for byte in payload
        .as_bytes()
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
    {
        if byte == b'=' {
            break;
        }
        chars[count % 4] = byte;
        count += 1;

        if count % 4 == 0 {
            let a = b64_value(chars[0])?;
            let b = b64_value(chars[1])?;
            let c = b64_value(chars[2])?;
            let d = b64_value(chars[3])?;
            out.push((a << 2) | (b >> 4)).ok()?;
            out.push(((b & 0x0F) << 4) | (c >> 2)).ok()?;
            out.push(((c & 0x03) << 6) | d).ok()?;
        }
    }

    if count % 4 != 0 {
        let rem = count % 4;
        if rem == 2 {
            let a = b64_value(chars[0])?;
            let b = b64_value(chars[1])?;
            out.push((a << 2) | (b >> 4)).ok()?;
        } else if rem == 3 {
            let a = b64_value(chars[0])?;
            let b = b64_value(chars[1])?;
            let c = b64_value(chars[2])?;
            out.push((a << 2) | (b >> 4)).ok()?;
            out.push(((b & 0x0F) << 4) | (c >> 2)).ok()?;
        }
    }

    Some(out)
}

fn nibble_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(10 + byte - b'a'),
        b'A'..=b'F' => Some(10 + byte - b'A'),
        _ => None,
    }
}

fn b64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(26 + byte - b'a'),
        b'0'..=b'9' => Some(52 + byte - b'0'),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_base64, decode_hex};

    #[test]
    fn decodes_hex_payload() {
        let bytes = decode_hex("48656c6c6f").unwrap();
        assert_eq!(bytes.as_slice(), b"Hello");
    }

    #[test]
    fn decodes_base64_payload() {
        let bytes = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(bytes.as_slice(), b"Hello");
    }
}
