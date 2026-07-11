// http://www.everest-semi.com/pdf/ES8311%20PB.pdf
// https://www.ti.com/lit/ds/symlink/tca9555.pdf

use crate::board::{AUDIO_SAMPLE_RATE, I2cBus};
use embassy_time::{Duration, Timer};
use es8311::{ClockConfig, Es8311, Resolution};
use esp_hal::{Blocking, delay::Delay, i2c::master::I2c};

const ES8311_ADDR: u8 = 0x18;
const TCA9555_ADDR: u8 = 0x20;
const TCA9555_REG_OUT1: u8 = 0x03;
const TCA9555_REG_CFG1: u8 = 0x07;

// ES8311 requires an MCLK; 256×fs is a valid ratio present in the driver's table.
const MCLK_FREQ: u32 = AUDIO_SAMPLE_RATE * 256;

pub struct Codec {
    bus: &'static I2cBus,
    es8311: Es8311,
}

impl Codec {
    pub async fn init(bus: &'static I2cBus) -> Result<Self, &'static str> {
        let codec = Es8311::new(ES8311_ADDR);
        let mut guard = bus.lock().await;

        let mut delay = Delay::new();

        // Keep the speaker amplifier OFF during codec bring-up. Enabling it
        // first means the codec's power-up transient is amplified into an
        // audible "pop"/clap.
        tca9555_pa_configure(&mut *guard)?;
        tca9555_pa_set(&mut *guard, false)?;

        let clk_cfg = ClockConfig {
            mclk_inverted: false,
            sclk_inverted: false,
            mclk_from_mclk_pin: true,
            mclk_frequency: MCLK_FREQ,
            sample_frequency: AUDIO_SAMPLE_RATE,
        };

        codec
            .init(
                &mut *guard,
                &clk_cfg,
                Resolution::Bits16,
                Resolution::Bits16,
                &mut delay,
            )
            .map_err(|e| {
                defmt::error!("ES8311 init error: {:?}", defmt::Debug2Format(&e));
                "ES8311 init failed"
            })?;

        codec
            .volume_set(&mut *guard, 70, None)
            .map_err(|_| "ES8311 volume set failed")?;

        codec
            .mute(&mut *guard, true)
            .map_err(|_| "ES8311 mute failed")?;

        drop(guard);
        defmt::info!(
            "audio: codec ready at {}Hz (output muted)",
            AUDIO_SAMPLE_RATE
        );
        Ok(Self { bus, es8311: codec })
    }

    /// Enable or disable the speaker output path.
    ///
    /// The amplifier is only powered while audio is actually playing, which
    /// keeps the speaker silent at idle (no amplified DAC noise floor) and
    /// keeps power-up transients out of the audio.
    pub async fn set_output_enabled(&mut self, on: bool) {
        if on {
            let _ = self.es8311.mute(&mut *self.bus.lock().await, false);
            Timer::after(Duration::from_millis(10)).await;
            let _ = tca9555_pa_set(&mut *self.bus.lock().await, true);
            Timer::after(Duration::from_millis(30)).await;
        } else {
            let _ = tca9555_pa_set(&mut *self.bus.lock().await, false);
            Timer::after(Duration::from_millis(5)).await;
            let _ = self.es8311.mute(&mut *self.bus.lock().await, true);
        }
    }
}

fn tca9555_write_reg(i2c: &mut I2c<'_, Blocking>, reg: u8, val: u8) -> Result<(), &'static str> {
    i2c.write(TCA9555_ADDR, &[reg, val])
        .map_err(|_| "TCA9555 write failed")
}

fn tca9555_pa_configure(i2c: &mut I2c<'_, Blocking>) -> Result<(), &'static str> {
    tca9555_write_reg(i2c, TCA9555_REG_CFG1, 0b1111_1110)
}

fn tca9555_pa_set(i2c: &mut I2c<'_, Blocking>, on: bool) -> Result<(), &'static str> {
    tca9555_write_reg(
        i2c,
        TCA9555_REG_OUT1,
        if on { 0b0000_0001 } else { 0b0000_0000 },
    )
}
