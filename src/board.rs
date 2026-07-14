//! Board support for the Waveshare ESP32-S3-AUDIO-Board.
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use esp_hal::{
    Blocking,
    gpio::{AnyPin, Pin},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{DMA_CH0, FLASH, I2S0, Peripherals, RMT, SW_INTERRUPT, TIMG0, WIFI},
};
use static_cell::StaticCell;

/// Number of on-board WS2812 RGB LEDs (7-LED ring).
pub const LED_COUNT: usize = 7;

/// Shared I2C0 bus. Several on-board devices hang off it (ES8311 codec,
/// TCA9555 expander, PCF85063 RTC, and later the ES7210 mic), so it is wrapped
/// in an async mutex and shared by `&'static` reference between tasks.
pub type I2cBus = Mutex<CriticalSectionRawMutex, I2c<'static, Blocking>>;

// TCA9555 register map constants
const TCA9555_ADDR: u8 = 0x20;
const REG_INPUT_PORT_1: u8 = 0x01; // Read Input Port 1 (Pins 8-15)
const REG_CONFIGURATION_PORT_1: u8 = 0x07; // Config Port 1 (0 = Output, 1 = Input)

/// Peripherals and pins required by the audio subsystem (I2S only; I2C is on
/// the shared bus).
pub struct AudioResources<'d> {
    pub i2s0: I2S0<'d>,
    pub dma_ch: DMA_CH0<'d>,
    pub mclk: AnyPin<'d>,
    pub bclk: AnyPin<'d>,
    pub lrck: AnyPin<'d>,
    pub dout: AnyPin<'d>,
}

// The button id is the pin number on the TCA9555 Port 1 register.
pub struct EIOKey<const N: u8> {
    bus: &'static I2cBus,
}

impl<const N: u8> EIOKey<N> {
    const MASK: u8 = 1 << N;

    pub fn new(bus: &'static I2cBus) -> Self {
        const { assert!(N < 8, "TCA9555 Port 1 only has pins 0 to 7") };
        Self { bus }
    }

    pub fn into_runtime(self) -> ButtonRuntime {
        ButtonRuntime::new(self.bus, Self::MASK)
    }

    pub async fn init(&self) {
        let mut i2c = self.bus.lock().await;
        let mut config = [0u8; 1];

        // Read existing configuration, set Bit 1, and write it back
        if i2c
            .write_read(TCA9555_ADDR, &[REG_CONFIGURATION_PORT_1], &mut config)
            .is_ok()
        {
            let new_config = config[0] | Self::MASK; // 1 = Input mode
            let _ = i2c.write(TCA9555_ADDR, &[REG_CONFIGURATION_PORT_1, new_config]);
        }
    }
}

pub struct ButtonRuntime {
    bus: &'static I2cBus,
    mask: u8,
}

impl ButtonRuntime {
    fn new(bus: &'static I2cBus, mask: u8) -> Self {
        Self { bus, mask }
    }

    pub async fn is_pressed(&self) -> bool {
        let mut i2c = self.bus.lock().await;
        let mut port_data = [0u8; 1];

        if i2c
            .write_read(TCA9555_ADDR, &[REG_INPUT_PORT_1], &mut port_data)
            .is_ok()
        {
            (port_data[0] & self.mask) == 0
        } else {
            false
        }
    }
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

    pub i2c_bus: &'static I2cBus,
    pub key1: EIOKey<1>,
    pub key2: EIOKey<2>,
    pub key3: EIOKey<3>,
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
        let i2c_driver = I2c::new(p.I2C0, I2cConfig::default())
            .unwrap()
            .with_sda(p.GPIO11)
            .with_scl(p.GPIO10);
        static I2C_BUS_CELL: StaticCell<I2cBus> = StaticCell::new();
        let i2c_bus_ref = I2C_BUS_CELL.init(Mutex::new(i2c_driver));

        Self {
            timg0: p.TIMG0,
            sw_interrupt: p.SW_INTERRUPT,
            flash: p.FLASH,
            rmt: p.RMT,
            led_pin: p.GPIO38.degrade(),
            boot_button: p.GPIO0.degrade(),
            wifi: p.WIFI,
            i2c_bus: i2c_bus_ref,
            key1: EIOKey::<1>::new(i2c_bus_ref),
            key2: EIOKey::<2>::new(i2c_bus_ref),
            key3: EIOKey::<3>::new(i2c_bus_ref),
            audio: AudioResources {
                i2s0: p.I2S0,
                dma_ch: p.DMA_CH0,
                mclk: p.GPIO12.degrade(),
                bclk: p.GPIO13.degrade(),
                lrck: p.GPIO14.degrade(),
                dout: p.GPIO16.degrade(),
            },
        }
    }
}
