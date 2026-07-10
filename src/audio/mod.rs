pub mod codec;
pub mod sine;

use codec::Codec;
use sine::SineGenerator;

use crate::board::{AudioResources, AUDIO_SAMPLE_RATE};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    dma_buffers,
    i2s::master::{Channels, Config, DataFormat, I2s, I2sTx},
    time::Rate,
};

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
}

impl<'d> Audio<'d> {
    pub fn new(res: AudioResources<'d>) -> Result<Self, AudioError> {
        // Init codec over I2C
        let codec = Codec::init(res.i2c0, res.sda, res.scl).map_err(|_| AudioError::CodecInit)?;

        let (_, _, tx_buffer, tx_descriptors) = dma_buffers!(DMA_BUF_SIZE, DMA_BUF_SIZE);

        let i2s = I2s::new(
            res.i2s0,
            res.dma_ch,
            Config::new_tdm_philips().with_msb_shift(true)
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

        let _ = codec;

        Ok(Self {
            i2s_tx,
            tx_buf: tx_buffer,
        })
    }

    /// Play a sine wave tone for `duration_ms` milliseconds
    pub async fn play_tone(
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
    pub async fn beep(&mut self) -> Result<(), AudioError> {
        self.play_tone(440, 0.5, 500).await
    }

    /// Play ascending tones on WiFi connect
    pub async fn play_connected(&mut self) -> Result<(), AudioError> {
        // Phase 1: Soft rising arpeggio (C major)
        self.play_tone(262, 0.2, 100).await?; // C4
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(330, 0.2, 100).await?; // E4
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(392, 0.2, 100).await?; // G4
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(523, 0.25, 100).await?; // C5

        Timer::after(Duration::from_millis(80)).await;

        // Phase 2: Middle flourish
        self.play_tone(523, 0.25, 80).await?; // C5
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(587, 0.25, 80).await?; // D5
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(659, 0.25, 80).await?; // E5
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(784, 0.25, 80).await?; // G5
        Timer::after(Duration::from_millis(20)).await;
        self.play_tone(880, 0.3, 80).await?; // A5

        Timer::after(Duration::from_millis(80)).await;

        // Phase 3: Final resolved chord tones held longer
        self.play_tone(1047, 0.35, 300).await?; // C6
        Timer::after(Duration::from_millis(40)).await;
        self.play_tone(880, 0.3, 200).await?; // A5
        Timer::after(Duration::from_millis(40)).await;
        self.play_tone(1047, 0.4, 500).await?; // C6 - final hold

        // Total: ~2.1 seconds
        Ok(())
    }
}
