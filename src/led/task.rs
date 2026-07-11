use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use super::{Animation, Color, LedController};
use crate::board::LED_COUNT;
use esp_hal::{gpio::AnyPin, peripherals::RMT};

static LED_SIGNAL: Signal<CriticalSectionRawMutex, LedCommand> = Signal::new();

#[derive(Clone, Copy)]
pub enum LedCommand {
    Once(Animation),
    Loop(Animation),
    Set(usize, Color),
    SetAll(Color),
    Clear,
    Brightness(u8),
}

pub fn led_send(command: LedCommand) {
    LED_SIGNAL.signal(command);
}

pub fn led_spawn(spawner: &Spawner, rmt: RMT<'static>, pin: AnyPin<'static>) {
    spawner.spawn(led_task(rmt, pin).unwrap());
}

#[embassy_executor::task]
async fn led_task(rmt: RMT<'static>, pin: AnyPin<'static>) {
    let mut led = LedController::<LED_COUNT>::new(rmt, pin);

    let mut command = LED_SIGNAL.wait().await;
    loop {
        command = execute(&mut led, command).await;
    }
}

/// Execute one command and return the command to run next.
///
/// Long-running animations race against `LED_SIGNAL`: when a new command
/// arrives the animation future is dropped at its current `.await` point,
/// giving clean, poll-free cancellation.
async fn execute<const N: usize>(
    led: &mut LedController<'_, N>,
    command: LedCommand,
) -> LedCommand {
    match command {
        LedCommand::Brightness(b) => {
            led.set_brightness(b); // takes effect on the next frame
            LED_SIGNAL.wait().await
        }
        LedCommand::Set(i, color) => {
            led.set(i, color).await;
            LED_SIGNAL.wait().await
        }
        LedCommand::SetAll(color) => {
            led.set_all(color).await;
            LED_SIGNAL.wait().await
        }
        LedCommand::Clear => {
            led.clear().await;
            LED_SIGNAL.wait().await
        }
        LedCommand::Once(animation) => {
            match select(animation.run_once(led), LED_SIGNAL.wait()).await {
                Either::First(()) => LED_SIGNAL.wait().await,
                Either::Second(next) => next,
            }
        }
        LedCommand::Loop(animation) => {
            match select(animation.run_forever(led), LED_SIGNAL.wait()).await {
                Either::First(_) => unreachable!("run_forever never returns"),
                Either::Second(next) => next,
            }
        }
    }
}
