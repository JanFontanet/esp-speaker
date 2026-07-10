//! Factory-reset button.
//!
//! Holding the BOOT button for a few seconds requests a credential wipe: we set
//! a persistent flag and reboot, and `main` performs the wipe on the next boot
//! (the button task can't own the flash, which belongs to NVS in `main`).

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Pull};

use crate::boot;
use crate::led::{Color, LedCommand, led_send};

/// How long the button must be held to trigger a factory reset.
const HOLD_DURATION: Duration = Duration::from_secs(3);

/// Spawn the button task. Call once from main.
pub fn button_spawn(spawner: &Spawner, pin: AnyPin<'static>) {
    spawner.spawn(button_task(pin).unwrap());
}

#[embassy_executor::task]
async fn button_task(pin: AnyPin<'static>) {
    let button = Input::new(pin, InputConfig::default().with_pull(Pull::Up));

    loop {
        // BOOT button is active-low (pressed = low).
        if button.is_low() {
            let start = Instant::now();
            while button.is_low() {
                if start.elapsed() >= HOLD_DURATION {
                    defmt::warn!("button: factory reset requested");
                    led_send(LedCommand::SetAll(Color::RED));
                    boot::request_factory_reset();
                    Timer::after(Duration::from_millis(600)).await;
                    esp_hal::system::software_reset();
                }
                Timer::after(Duration::from_millis(50)).await;
            }
        }
        Timer::after(Duration::from_millis(80)).await;
    }
}
