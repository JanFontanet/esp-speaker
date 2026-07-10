//! Board support for the Waveshare ESP32-S3-AUDIO-Board.
//!
//! This is the single source of truth for pin assignments and on-board device
//! wiring. Bundling pins into named structs (rather than passing a long list of
//! same-typed `AnyPin`s) makes it impossible to accidentally transpose two pins
//! — e.g. swapping the I2S `DOUT` and `DIN` lines.
//!
//! Pin map verified against Waveshare's factory firmware.

use esp_hal::{
    gpio::{AnyPin, Pin},
    peripherals::{DMA_CH0, FLASH, I2C0, I2S0, Peripherals, RMT, SW_INTERRUPT, TIMG0, WIFI},
};

/// Number of on-board WS2812 RGB LEDs (7-LED ring).
pub const LED_COUNT: usize = 7;

/// I2S playback sample rate, in Hz.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;

/// Peripherals and pins required by the audio subsystem.
pub struct AudioResources<'d> {
    pub i2s0: I2S0<'d>,
    pub dma_ch: DMA_CH0<'d>,
    pub mclk: AnyPin<'d>,
    pub bclk: AnyPin<'d>,
    pub lrck: AnyPin<'d>,
    pub dout: AnyPin<'d>,
    pub i2c0: I2C0<'d>,
    pub sda: AnyPin<'d>,
    pub scl: AnyPin<'d>,
}

/// All board resources, with every pin already assigned its documented role.
pub struct Board<'d> {
    pub timg0: TIMG0<'d>,
    pub sw_interrupt: SW_INTERRUPT<'d>,
    pub flash: FLASH<'d>,
    pub rmt: RMT<'d>,
    pub led_pin: AnyPin<'d>,
    pub boot_button: AnyPin<'d>,
    pub wifi: WIFI<'d>,
    pub audio: AudioResources<'d>,
}

impl Board<'static> {
    /// Split the raw [`Peripherals`] into named board resources.
    ///
    /// This is the ONE place where physical GPIO numbers map to roles:
    ///
    /// | Role            | GPIO   |
    /// |-----------------|--------|
    /// | I2S MCLK        | 12     |
    /// | I2S BCLK        | 13     |
    /// | I2S LRCK / WS   | 14     |
    /// | I2S DOUT (spk)  | 16     |
    /// | I2C SDA         | 11     |
    /// | I2C SCL         | 10     |
    /// | WS2812 LEDs     | 38     |
    ///
    /// On-board I2C devices: ES8311 codec @ `0x18`, TCA9555 expander @ `0x20`.
    pub fn new(p: Peripherals) -> Self {
        Self {
            timg0: p.TIMG0,
            sw_interrupt: p.SW_INTERRUPT,
            flash: p.FLASH,
            rmt: p.RMT,
            led_pin: p.GPIO38.degrade(),
            boot_button: p.GPIO0.degrade(),
            wifi: p.WIFI,
            audio: AudioResources {
                i2s0: p.I2S0,
                dma_ch: p.DMA_CH0,
                mclk: p.GPIO12.degrade(),
                bclk: p.GPIO13.degrade(),
                lrck: p.GPIO14.degrade(),
                dout: p.GPIO16.degrade(),
                i2c0: p.I2C0,
                sda: p.GPIO11.degrade(),
                scl: p.GPIO10.degrade(),
            },
        }
    }
}
