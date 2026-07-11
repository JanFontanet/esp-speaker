pub mod animations;
pub mod task;
pub mod ws2812;

pub use animations::Animation;
pub use task::{LedCommand, led_send, led_spawn};
use ws2812::Ws2812Driver;

use esp_hal::{gpio::AnyPin, peripherals::RMT};

// ── Color ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, defmt::Format)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
    };
    pub const RED: Self = Self { r: 255, g: 0, b: 0 };
    pub const GREEN: Self = Self { r: 0, g: 255, b: 0 };
    pub const BLUE: Self = Self { r: 0, g: 0, b: 255 };
    pub const YELLOW: Self = Self {
        r: 255,
        g: 255,
        b: 0,
    };
    pub const CYAN: Self = Self {
        r: 0,
        g: 255,
        b: 255,
    };
    pub const PURPLE: Self = Self {
        r: 128,
        g: 0,
        b: 128,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn scale(self, brightness: u8) -> Self {
        let s = brightness as u16;
        Self {
            r: ((self.r as u16 * s) / 255) as u8,
            g: ((self.g as u16 * s) / 255) as u8,
            b: ((self.b as u16 * s) / 255) as u8,
        }
    }

    pub fn from_hsv(hue: u8, sat: u8, val: u8) -> Self {
        if sat == 0 {
            return Self::new(val, val, val);
        }

        let region = hue / 43;
        let rem = (hue - region * 43) * 6;
        let p = ((val as u16 * (255 - sat as u16)) / 255) as u8;
        let q = ((val as u16 * (255 - ((sat as u16 * rem as u16) / 255))) / 255) as u8;
        let t = ((val as u16 * (255 - ((sat as u16 * (255 - rem as u16)) / 255))) / 255) as u8;

        match region {
            0 => Self::new(val, t, p),
            1 => Self::new(q, val, p),
            2 => Self::new(p, val, t),
            3 => Self::new(p, q, val),
            4 => Self::new(t, p, val),
            _ => Self::new(val, p, q),
        }
    }
}

// ── LedController ─────────────────────────────────────────────────────────────

pub struct LedController<'d, const N: usize> {
    driver: Ws2812Driver<'d>,
    colors: [Color; N],
    pub brightness: u8,
}

impl<'d, const N: usize> LedController<'d, N> {
    pub fn new(rmt: RMT<'d>, pin: AnyPin<'d>) -> Self {
        Self {
            driver: Ws2812Driver::new(rmt, pin),
            colors: [Color::BLACK; N],
            brightness: 128,
        }
    }

    // ── Simple API ────────────────────────────────────────────────────────────

    pub async fn set(&mut self, index: usize, color: Color) {
        if index < N {
            self.colors[index] = color.scale(self.brightness);
            self.flush().await;
        }
    }

    pub async fn set_all(&mut self, color: Color) {
        let scaled = color.scale(self.brightness);
        self.colors = [scaled; N];
        self.flush().await;
    }

    pub async fn clear(&mut self) {
        self.set_all(Color::BLACK).await;
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
    }

    // ── Animation API ─────────────────────────────────────────────────────────

    pub async fn animate_once(&mut self, animation: Animation) {
        animation.run_once(self).await;
    }

    pub async fn animate_n(&mut self, animation: Animation, times: usize) {
        animation.run_n(self, times).await;
    }

    pub async fn animate(&mut self, animation: Animation) -> ! {
        animation.run_forever(self).await
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    pub(crate) async fn set_raw(&mut self, colors: &[Color; N]) {
        self.colors = *colors;
        self.flush().await;
    }

    async fn flush(&mut self) {
        self.driver.write(&self.colors).await.ok();
    }
}
