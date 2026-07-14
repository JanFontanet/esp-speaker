use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Sender;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Pull};

use crate::board::{ButtonRuntime, EIOKey};
use crate::boot;
use crate::config::{
    BUTTON_DEBOUNCE_MS, BUTTON_HOLD_DURATION_SECS, BUTTON_POLL_INTERVAL_MS, CHANNEL_SIZE,
    FACTORY_RESET_REBOOT_DELAY_MS,
};
use crate::led::{Color, LedCommand, led_send};
use crate::mqtt::msg_protocol::AppEvent;

const HOLD_DURATION: Duration = Duration::from_secs(BUTTON_HOLD_DURATION_SECS);

pub fn button_spawn<const N: u8>(
    spawner: &Spawner,
    button: EIOKey<N>,
    event_tx: Sender<'static, CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>,
    event: AppEvent,
) {
    spawner.spawn(button_task(button.into_runtime(), event_tx, event).unwrap());
}

#[embassy_executor::task]
async fn button_task(
    button: ButtonRuntime,
    event_tx: Sender<'static, CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>,
    event: AppEvent,
) {
    let mut was_pressed = false;

    loop {
        let pressed = button.is_pressed().await;
        if pressed && !was_pressed {
            let _ = event_tx.send(event).await;
            was_pressed = true;
        } else if !pressed {
            was_pressed = false;
        }

        Timer::after(Duration::from_millis(BUTTON_DEBOUNCE_MS)).await;
    }
}

pub fn boot_button_spawn(spawner: &Spawner, pin: AnyPin<'static>) {
    spawner.spawn(boot_button_task(pin).unwrap());
}

#[embassy_executor::task]
async fn boot_button_task(pin: AnyPin<'static>) {
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
