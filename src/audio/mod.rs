pub mod codec;
pub mod sine;
pub mod task;

pub use task::{AudioCommand, Sound, audio_send, audio_spawn};

use codec::Codec;
use sine::SineGenerator;

use crate::board::{AudioResources, I2cBus};
use crate::config::{AUDIO_DMA_BUF_SIZE, AUDIO_SAMPLE_RATE, DEFAULT_VOLUME};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async, dma_buffers,
    i2s::master::{Channels, Config, DataFormat, I2s, I2sTx},
    time::Rate,
};
use micromath::F32Ext;

const DMA_BUF_SIZE: usize = AUDIO_DMA_BUF_SIZE;

#[derive(Debug, defmt::Format)]
pub enum AudioError {
    CodecInit,
    I2sError,
    DmaError,
}

pub struct Audio<'d> {
    i2s_tx: I2sTx<'d, Async>,
    tx_buf: &'static mut [u8],
    codec: Codec,
    volume: f32,
    announcement: Option<AnnouncementStream>,
}

impl<'d> Audio<'d> {
    pub async fn new(res: AudioResources<'d>, bus: &'static I2cBus) -> Result<Self, AudioError> {
        let codec = Codec::init(bus).await.map_err(|_| AudioError::CodecInit)?;

        let (_, _, tx_buffer, tx_descriptors) = dma_buffers!(DMA_BUF_SIZE, DMA_BUF_SIZE);

        let i2s = I2s::new(
            res.i2s0,
            res.dma_ch,
            Config::new_tdm_philips()
                .with_msb_shift(true)
                .with_sample_rate(Rate::from_hz(AUDIO_SAMPLE_RATE))
                .with_data_format(DataFormat::Data16Channel16)
                .with_channels(Channels::STEREO),
        )
        .map_err(|_| AudioError::I2sError)?
        .into_async()
        .with_mclk(res.mclk);

        let i2s_tx = i2s
            .i2s_tx
            .with_bclk(res.bclk)
            .with_ws(res.lrck)
            .with_dout(res.dout)
            .build(tx_descriptors);

        Ok(Self {
            i2s_tx,
            tx_buf: tx_buffer,
            codec,
            volume: volume_multiplier(DEFAULT_VOLUME),
            announcement: None,
        })
    }

    /// Enable/disable the speaker amplifier + DAC output. The audio task calls
    /// this to keep the amp powered down except while a sound is playing.
    pub(crate) async fn set_output_enabled(&mut self, on: bool) {
        self.codec.set_output_enabled(on).await;
    }

    pub(crate) async fn set_volume(&mut self, level: u8) {
        self.volume = volume_multiplier(level);
        self.codec.set_volume(level).await;
    }

    pub(crate) async fn play_announcement_chunk(&mut self, chunk: &[u8]) -> Result<(), AudioError> {
        let stream = self
            .announcement
            .get_or_insert_with(AnnouncementStream::new);
        stream.push_chunk(chunk);

        loop {
            let count = stream.take_bytes(&mut self.tx_buf[..]);
            if count == 0 {
                break;
            }
            self.i2s_tx
                .write_dma_async(&mut self.tx_buf[..count])
                .await
                .map_err(|_| AudioError::I2sError)?;
        }

        Ok(())
    }

    /// Play a queued [`Sound`]. Called by the audio task.
    pub(crate) async fn play(&mut self, sound: Sound) -> Result<(), AudioError> {
        match sound {
            Sound::Beep => self.beep().await,
            Sound::Connected => self.play_connected().await,
            Sound::Tone {
                frequency,
                amplitude,
                duration_ms,
            } => self.play_tone(frequency, amplitude, duration_ms).await,
        }
    }

    /// Play a sine wave tone for `duration_ms` milliseconds
    async fn play_tone(
        &mut self,
        frequency: u32,
        amplitude: f32,
        duration_ms: u64,
    ) -> Result<(), AudioError> {
        defmt::info!(
            "play_tone: {}Hz amp={} dur={}ms",
            frequency,
            amplitude,
            duration_ms
        );
        let mut r#gen = SineGenerator::new(AUDIO_SAMPLE_RATE, frequency, amplitude * self.volume);

        let end = embassy_time::Instant::now() + Duration::from_millis(duration_ms);

        while embassy_time::Instant::now() < end {
            // Fill buffer with i16 samples
            let samples = unsafe {
                core::slice::from_raw_parts_mut(
                    self.tx_buf.as_mut_ptr() as *mut i16,
                    self.tx_buf.len() / 2,
                )
            };
            r#gen.fill(samples);

            self.i2s_tx
                .write_dma_async(&mut self.tx_buf)
                .await
                .map_err(|_| AudioError::I2sError)?;
        }

        Ok(())
    }

    /// Play a beep (440Hz, 0.5s)
    async fn beep(&mut self) -> Result<(), AudioError> {
        self.play_tone(440, 0.5, 500).await
    }

    /// Play a single percussive note: a short click-free attack followed by an
    /// exponential decay. This gives a "dum" character instead of a flat beep,
    /// which is what makes the connect sound read as a Netflix-style "ta-dum".
    async fn play_note(
        &mut self,
        frequency: u32,
        amplitude: f32,
        duration_ms: u64,
    ) -> Result<(), AudioError> {
        let total_frames = (duration_ms * AUDIO_SAMPLE_RATE as u64 / 1000).max(1) as u32;
        let attack = (AUDIO_SAMPLE_RATE / 80).max(1); // ~12 ms, gentle onset
        let release = (AUDIO_SAMPLE_RATE / 25).max(1); // ~40 ms fade fully to silence
        // Full-scale oscillator; amplitude + envelope are applied per sample.
        let mut osc = SineGenerator::new(AUDIO_SAMPLE_RATE, frequency, 1.0);

        let mut n: u32 = 0;
        while n < total_frames {
            let samples = unsafe {
                core::slice::from_raw_parts_mut(
                    self.tx_buf.as_mut_ptr() as *mut i16,
                    self.tx_buf.len() / 2,
                )
            };
            let mut i = 0;
            while i + 1 < samples.len() {
                let out = if n < total_frames {
                    let env = note_envelope(n, total_frames, attack, release);
                    let s = (osc.sample() as f32 * amplitude * self.volume * env) as i16;
                    n += 1;
                    s
                } else {
                    0 // pad the tail of the final buffer with silence
                };
                samples[i] = out;
                samples[i + 1] = out;
                i += 2;
            }
            self.i2s_tx
                .write_dma_async(&mut self.tx_buf)
                .await
                .map_err(|_| AudioError::I2sError)?;
        }
        Ok(())
    }

    /// Netflix-style "ta-dum": two low percussive notes, the second lower,
    /// louder and longer. Pitches/timings are easy to tune by ear below.
    async fn play_connected(&mut self) -> Result<(), AudioError> {
        self.play_note(165, 0.4, 190).await?; // "ta"  — D3, short
        Timer::after(Duration::from_millis(30)).await;
        self.play_note(110, 0.55, 850).await?; // "dum" — G2, long decay
        Ok(())
    }
}

/// Percussive amplitude envelope in `0.0..=1.0` for frame `n` of `total`:
/// a gentle linear attack, an exponential decay, and a linear release that
/// fades fully to silence so notes don't end on a step (which clicks).
struct AnnouncementStream {
    buffer: [u8; 4096],
    head: usize,
    len: usize,
    header_bytes: [u8; 44],
    header_len: usize,
    header_parsed: bool,
}

impl AnnouncementStream {
    fn new() -> Self {
        Self {
            buffer: [0; 4096],
            head: 0,
            len: 0,
            header_bytes: [0; 44],
            header_len: 0,
            header_parsed: false,
        }
    }

    fn push_chunk(&mut self, chunk: &[u8]) {
        for &byte in chunk {
            if self.len >= self.buffer.len() {
                break;
            }
            self.buffer[(self.head + self.len) % self.buffer.len()] = byte;
            self.len += 1;
        }
    }

    fn take_bytes(&mut self, dst: &mut [u8]) -> usize {
        if self.len == 0 {
            return 0;
        }

        if !self.header_parsed {
            while self.header_len < self.header_bytes.len() && self.len > 0 {
                self.header_bytes[self.header_len] = self.take_one();
                self.header_len += 1;
            }
            if self.header_len >= self.header_bytes.len() {
                self.header_parsed = true;
            }
            if !self.header_parsed {
                return 0;
            }
        }

        let count = core::cmp::min(self.len, dst.len());
        for i in 0..count {
            dst[i] = self.take_one();
        }
        count
    }

    fn take_one(&mut self) -> u8 {
        let byte = self.buffer[self.head];
        self.head = (self.head + 1) % self.buffer.len();
        self.len -= 1;
        byte
    }
}

fn volume_multiplier(level: u8) -> f32 {
    if level == 0 {
        0.0
    } else {
        level as f32 / 100.0
    }
}

fn note_envelope(n: u32, total: u32, attack: u32, release: u32) -> f32 {
    let attack_gain = if n < attack {
        n as f32 / attack as f32
    } else {
        1.0
    };
    let release_gain = if n + release > total {
        total.saturating_sub(n) as f32 / release as f32
    } else {
        1.0
    };
    let progress = n as f32 / total as f32;
    let decay = (-3.0 * progress).exp();
    attack_gain * release_gain * decay
}

#[cfg(test)]
mod tests {
    use super::{AnnouncementStream, volume_multiplier};

    #[test]
    fn volume_multiplier_maps_zero_and_full() {
        assert_eq!(volume_multiplier(0), 0.0);
        assert_eq!(volume_multiplier(100), 1.0);
        assert_eq!(volume_multiplier(50), 0.5);
    }

    #[test]
    fn announcement_stream_skips_wav_header_and_returns_pcm_bytes() {
        let mut stream = AnnouncementStream::new();
        let mut header = [0u8; 44];
        header[..4].copy_from_slice(b"RIFF");
        stream.push_chunk(&header);
        stream.push_chunk(&[0x01, 0x02, 0x03, 0x04]);

        let mut out = [0u8; 4];
        let first = stream.take_bytes(&mut out);
        assert_eq!(first, 4);
        assert_eq!(out, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(stream.take_bytes(&mut out), 0);
    }
}
