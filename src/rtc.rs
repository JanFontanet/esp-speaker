//! Minimal PCF85063 RTC driver (on the shared I2C bus).
//! https://www.nxp.com/docs/en/data-sheet/PCF85063A.pdf

use crate::board::I2cBus;

const PCF85063_ADDR: u8 = 0x51;
const REG_SECONDS: u8 = 0x04; // seconds..years are 7 consecutive BCD registers

#[derive(Clone, Copy, defmt::Format)]
pub struct DateTime {
    pub year: u16, // full year, e.g. 2026
    pub month: u8, // 1-12
    pub day: u8,   // 1-31
    pub hour: u8,  // 0-23
    pub minute: u8,
    pub second: u8,
}

pub struct Rtc {
    bus: &'static I2cBus,
}

impl Rtc {
    pub fn new(bus: &'static I2cBus) -> Self {
        Self { bus }
    }

    pub async fn set(&mut self, dt: &DateTime) -> Result<(), &'static str> {
        let payload = [
            REG_SECONDS,
            bin2bcd(dt.second) & 0x7F, // bit 7 is the oscillator-stop flag
            bin2bcd(dt.minute) & 0x7F,
            bin2bcd(dt.hour) & 0x3F, // 24-hour mode
            bin2bcd(dt.day) & 0x3F,
            0, // weekday: not tracked
            bin2bcd(dt.month) & 0x1F,
            bin2bcd((dt.year % 100) as u8),
        ];
        let mut i2c = self.bus.lock().await;
        i2c.write(PCF85063_ADDR, &payload)
            .map_err(|_| "RTC write failed")
    }

    pub async fn get(&mut self) -> Result<DateTime, &'static str> {
        let mut buf = [0u8; 7];
        {
            let mut i2c = self.bus.lock().await;
            i2c.write_read(PCF85063_ADDR, &[REG_SECONDS], &mut buf)
                .map_err(|_| "RTC read failed")?;
        }
        Ok(DateTime {
            second: bcd2bin(buf[0] & 0x7F),
            minute: bcd2bin(buf[1] & 0x7F),
            hour: bcd2bin(buf[2] & 0x3F),
            day: bcd2bin(buf[3] & 0x3F),
            month: bcd2bin(buf[5] & 0x1F),
            year: 2000 + bcd2bin(buf[6]) as u16,
        })
    }
}

fn bin2bcd(v: u8) -> u8 {
    ((v / 10) << 4) | (v % 10)
}

fn bcd2bin(v: u8) -> u8 {
    (v >> 4) * 10 + (v & 0x0F)
}
