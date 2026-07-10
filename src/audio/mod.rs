pub mod codec;
pub mod sine;
pub mod task;

pub use task::{Sound, audio_send, audio_spawn};

use codec::Codec;
use sine::SineGenerator;

use crate::board::{AUDIO_SAMPLE_RATE, AudioResources};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async, dma_buffers,
    i2s::master::{Channels, Config, DataFormat, I2s, I2sTx},
    time::Rate,
};
use micromath::F32Ext;

const DMA_BUF_SIZE: usize = 4096;

#[derive(Debug, defmt::Format)]
pub enum AudioError {
    CodecInit,
    I2sError,
    DmaError,
}

pub struct Audio<'d> {
    i2s_tx: I2sTx<'d, Async>,
    tx_buf: &'static mut [u8],
    codec: Codec<'d>,
}

impl<'d> Audio<'d> {
    pub fn new(res: AudioResources<'d>) -> Result<Self, AudioError> {
        // Init codec over I2C
        let codec = Codec::init(res.i2c0, res.sda, res.scl).map_err(|_| AudioError::CodecInit)?;

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
        })
    }

    /// Enable/disable the speaker amplifier + DAC output. The audio task calls
    /// this to keep the amp powered down except while a sound is playing.
    pub(crate) async fn set_output_enabled(&mut self, on: bool) {
        self.codec.set_output_enabled(on).await;
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
        let mut r#gen = SineGenerator::new(AUDIO_SAMPLE_RATE, frequency, amplitude);

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
                    let s = (osc.sample() as f32 * amplitude * env) as i16;
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
