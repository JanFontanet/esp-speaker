use esp_hal::{
    Async,
    gpio::{AnyPin, Level},
    peripherals::RMT,
    rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator},
    time::Rate,
};

use super::Color;

// WS2812 timing at 80MHz / divider 2 = 40MHz = 25ns per tick
const T0H: u16 = 16; // 0.4µs
const T0L: u16 = 34; // 0.85µs
const T1H: u16 = 32; // 0.8µs
const T1L: u16 = 18; // 0.45µs

const MAX_LEDS: usize = 16;
const CODES_PER_LED: usize = 24;
const MAX_PULSES: usize = MAX_LEDS * CODES_PER_LED + 1;

pub struct Ws2812Driver<'d> {
    channel: Channel<'d, Async, Tx>,
}

impl<'d> Ws2812Driver<'d> {
    pub fn new(rmt: RMT<'d>, pin: AnyPin<'d>) -> Self {
        let rmt = Rmt::new(rmt, Rate::from_mhz(80)).unwrap().into_async();

        let config = TxChannelConfig::default()
            .with_clk_divider(2)
            .with_idle_output(true)
            .with_idle_output_level(Level::Low)
            .with_carrier_modulation(false);

        let channel = rmt.channel0.configure_tx(&config).unwrap().with_pin(pin);

        Self { channel }
    }

    pub async fn write<const N: usize>(
        &mut self,
        colors: &[Color; N],
    ) -> Result<(), esp_hal::rmt::Error> {
        let mut pulses = [PulseCode::end_marker(); MAX_PULSES];
        let total = N * CODES_PER_LED + 1;

        for (led_idx, color) in colors.iter().enumerate() {
            // WS2812 order is GRB
            let bytes = [color.r, color.g, color.b];
            for (byte_idx, byte) in bytes.iter().enumerate() {
                for bit in 0..8 {
                    let is_one = (byte >> (7 - bit)) & 1 == 1;
                    let idx = led_idx * CODES_PER_LED + byte_idx * 8 + bit;
                    pulses[idx] = if is_one {
                        PulseCode::new(Level::High, T1H, Level::Low, T1L)
                    } else {
                        PulseCode::new(Level::High, T0H, Level::Low, T0L)
                    };
                }
            }
        }

        // Reset pulse
        pulses[N * CODES_PER_LED] = PulseCode::new(Level::Low, 2000, Level::Low, 2000);

        self.channel.transmit(&pulses[..total]).await
    }
}
