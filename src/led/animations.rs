use embassy_time::{Duration, Timer};
use micromath::F32Ext;

use super::{Color, LedController};

#[derive(Clone, Copy)]
pub enum Animation {
    Solid(Color),
    Rainbow { speed: u8 },
    Pulse { color: Color, speed: u8 },
    Chase { color: Color, speed: u8 },
}

impl Animation {
    pub async fn run_once<const N: usize>(&self, led: &mut LedController<'_, N>) {
        match self {
            Animation::Solid(color) => {
                led.set_all(*color).await;
            }

            Animation::Rainbow { speed } => {
                let steps = 256u16;
                let delay = Duration::from_millis(20u64.saturating_sub(*speed as u64 / 5));
                for step in 0..steps {
                    let mut colors = [Color::BLACK; N];
                    for i in 0..N {
                        let hue = ((step + (i as u16 * 256 / N as u16)) % 256) as u8;
                        colors[i] = Color::from_hsv(hue, 255, led.brightness);
                    }
                    led.set_raw(&colors).await;
                    Timer::after(delay).await;
                }
            }

            Animation::Pulse { color, speed } => {
                let steps = 128u16;
                let delay = Duration::from_millis(10u64.saturating_sub(*speed as u64 / 10));
                for step in 0..steps {
                    let t = step as f32 / steps as f32;
                    let scale = (t * core::f32::consts::PI).sin();
                    let c = color.scale((scale * led.brightness as f32) as u8);
                    led.set_all(c).await;
                    Timer::after(delay).await;
                }
                for step in (0..steps).rev() {
                    let t = step as f32 / steps as f32;
                    let scale = (t * core::f32::consts::PI).sin();
                    let c = color.scale((scale * led.brightness as f32) as u8);
                    led.set_all(c).await;
                    Timer::after(delay).await;
                }
            }

            Animation::Chase { color, speed } => {
                let delay = Duration::from_millis(100u64.saturating_sub(*speed as u64 * 10));
                for i in 0..N {
                    let mut colors = [Color::BLACK; N];
                    colors[i] = color.scale(led.brightness);
                    if i > 0 {
                        colors[i - 1] = color.scale(led.brightness / 3);
                    }
                    led.set_raw(&colors).await;
                    Timer::after(delay).await;
                }
            }
        }
    }

    pub async fn run_forever<const N: usize>(&self, led: &mut LedController<'_, N>) -> ! {
        if let Animation::Solid(color) = self {
            led.set_all(*color).await;
            loop {
                core::future::pending::<()>().await;
            }
        }
        loop {
            self.run_once(led).await;
        }
    }

    pub async fn run_n<const N: usize>(&self, led: &mut LedController<'_, N>, times: usize) {
        for _ in 0..times {
            self.run_once(led).await;
        }
    }
}
