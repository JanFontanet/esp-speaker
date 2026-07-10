use es8311::{ClockConfig, Es8311, Resolution};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{InputPin, OutputPin},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::I2C0,
};

const ES8311_ADDR: u8 = 0x18;
const TCA9555_ADDR: u8 = 0x20;
const TCA9555_REG_OUT1: u8 = 0x03;
const TCA9555_REG_CFG1: u8 = 0x07;

const SAMPLE_RATE: u32 = 48000;
const MCLK_FREQ:   u32 = 12288000; // exactly in table for 16kHz

pub struct Codec {
    pub es8311: Es8311,
}

impl Codec {
    pub fn init(
        i2c0: I2C0<'static>,
        sda_pin: impl OutputPin + InputPin + 'static,
        scl_pin: impl OutputPin + InputPin + 'static,
    ) -> Result<Self, &'static str> {
        let mut i2c = I2c::new(i2c0, I2cConfig::default())
            .unwrap()
            .with_sda(sda_pin)
            .with_scl(scl_pin);

        let mut delay = Delay::new();

        // Enable PA via TCA9555
        pa_enable(&mut i2c)?;

        // Init ES8311
        let codec = Es8311::new(ES8311_ADDR);

        let clk_cfg = ClockConfig {
            mclk_inverted: false,
            sclk_inverted: false,
            mclk_from_mclk_pin: true,
            mclk_frequency: MCLK_FREQ,
            sample_frequency: SAMPLE_RATE,
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

        codec
            .mute(&mut i2c, false)
            .map_err(|_| "ES8311 unmute failed")?;

        defmt::info!("audio: codec ready at {}Hz", SAMPLE_RATE);
        Ok(Self { es8311: codec })
    }
}

fn tca9555_write_reg(i2c: &mut I2c<'_, Blocking>, reg: u8, val: u8) -> Result<(), &'static str> {
    i2c.write(TCA9555_ADDR, &[reg, val])
        .map_err(|_| "TCA9555 write failed")
}

fn pa_enable(i2c: &mut I2c<'_, Blocking>) -> Result<(), &'static str> {
    tca9555_write_reg(i2c, TCA9555_REG_CFG1, 0b1111_1110)?;
    tca9555_write_reg(i2c, TCA9555_REG_OUT1, 0b0000_0001)?;
    defmt::info!("audio: PA enabled");
    Ok(())
}
