use embassy_time::{Duration, Timer};
use es8311::{ClockConfig, Es8311, Resolution};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{InputPin, OutputPin},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::I2C0,
};

use crate::board::AUDIO_SAMPLE_RATE;

const ES8311_ADDR: u8 = 0x18;
const TCA9555_ADDR: u8 = 0x20;
const TCA9555_REG_OUT1: u8 = 0x03;
const TCA9555_REG_CFG1: u8 = 0x07;

// ES8311 requires an MCLK; 256×fs is a valid ratio present in the driver's table.
const MCLK_FREQ: u32 = AUDIO_SAMPLE_RATE * 256;

pub struct Codec<'d> {
    i2c: I2c<'d, Blocking>,
    es8311: Es8311,
}

impl<'d> Codec<'d> {
    pub fn init(
        i2c0: I2C0<'d>,
        sda_pin: impl OutputPin + InputPin + 'd,
        scl_pin: impl OutputPin + InputPin + 'd,
    ) -> Result<Self, &'static str> {
        let mut i2c = I2c::new(i2c0, I2cConfig::default())
            .unwrap()
            .with_sda(sda_pin)
            .with_scl(scl_pin);

        let mut delay = Delay::new();

        // Keep the speaker amplifier OFF during codec bring-up. Enabling it
        // first (as we used to) means the codec's power-up transient is
        // amplified into an audible "pop"/clap.
        tca9555_pa_configure(&mut i2c)?;
        tca9555_pa_set(&mut i2c, false)?;

        // Init ES8311
        let codec = Es8311::new(ES8311_ADDR);

        let clk_cfg = ClockConfig {
            mclk_inverted: false,
            sclk_inverted: false,
            mclk_from_mclk_pin: true,
            mclk_frequency: MCLK_FREQ,
            sample_frequency: AUDIO_SAMPLE_RATE,
        };

        codec
            .init(
                &mut i2c,
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
            .volume_set(&mut i2c, 70, None)
            .map_err(|_| "ES8311 volume set failed")?;

        // Start muted with the PA off. The audio task unmutes and powers the
        // amplifier only while a sound is actually playing (see
        // `set_output_enabled`), which avoids the boot clap and keeps the idle
        // DAC noise floor from being amplified.
        codec
            .mute(&mut i2c, true)
            .map_err(|_| "ES8311 mute failed")?;

        defmt::info!(
            "audio: codec ready at {}Hz (output muted)",
            AUDIO_SAMPLE_RATE
        );
        Ok(Self { i2c, es8311: codec })
    }

    /// Enable or disable the speaker output path.
    ///
    /// The amplifier is only powered while audio is actually playing, which
    /// keeps the speaker silent at idle (no amplified DAC noise floor) and
    /// keeps power-up transients out of the audio.
    pub async fn set_output_enabled(&mut self, on: bool) {
        if on {
            // Unmute the DAC first (still silent, PA is off), let it settle,
            // then power the amplifier up into a stable output.
            let _ = self.es8311.mute(&mut self.i2c, false);
            Timer::after(Duration::from_millis(10)).await;
            let _ = tca9555_pa_set(&mut self.i2c, true);
            Timer::after(Duration::from_millis(30)).await;
        } else {
            // Power the amplifier down first so the DAC-mute transient isn't
            // amplified, then mute the DAC.
            let _ = tca9555_pa_set(&mut self.i2c, false);
            Timer::after(Duration::from_millis(5)).await;
            let _ = self.es8311.mute(&mut self.i2c, true);
        }
    }
}

fn tca9555_write_reg(i2c: &mut I2c<'_, Blocking>, reg: u8, val: u8) -> Result<(), &'static str> {
    i2c.write(TCA9555_ADDR, &[reg, val])
        .map_err(|_| "TCA9555 write failed")
}

/// Configure the TCA9555 P10 pin (speaker PA enable) as an output.
fn tca9555_pa_configure(i2c: &mut I2c<'_, Blocking>) -> Result<(), &'static str> {
    tca9555_write_reg(i2c, TCA9555_REG_CFG1, 0b1111_1110)
}

/// Drive the speaker PA enable pin (TCA9555 P10) high (on) or low (off).
fn tca9555_pa_set(i2c: &mut I2c<'_, Blocking>, on: bool) -> Result<(), &'static str> {
    tca9555_write_reg(
        i2c,
        TCA9555_REG_OUT1,
        if on { 0b0000_0001 } else { 0b0000_0000 },
    )
}
