use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Pull};

use crate::boot;
use crate::config::{
    BUTTON_DEBOUNCE_MS, BUTTON_HOLD_DURATION_SECS, BUTTON_POLL_INTERVAL_MS,
    FACTORY_RESET_REBOOT_DELAY_MS,
};
use crate::led::{Color, LedCommand, led_send};

const HOLD_DURATION: Duration = Duration::from_secs(BUTTON_HOLD_DURATION_SECS);

pub fn button_spawn(spawner: &Spawner, pin: AnyPin<'static>) {
    spawner.spawn(button_task(pin).unwrap());
}

#[embassy_executor::task]
async fn button_task(pin: AnyPin<'static>) {
    let button = Input::new(pin, InputConfig::default().with_pull(Pull::Up));

    loop {
        if button.is_low() {
            let start = Instant::now();
            while button.is_low() {
                if start.elapsed() >= HOLD_DURATION {
                    defmt::warn!("button: factory reset requested");
                    led_send(LedCommand::SetAll(Color::RED));
                    boot::request_factory_reset();
                    Timer::after(Duration::from_millis(FACTORY_RESET_REBOOT_DELAY_MS)).await;
                    esp_hal::system::software_reset();
                }
                Timer::after(Duration::from_millis(BUTTON_POLL_INTERVAL_MS)).await;
            }
        }
        Timer::after(Duration::from_millis(BUTTON_DEBOUNCE_MS)).await;
    }
}
