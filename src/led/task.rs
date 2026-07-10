use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use super::{Animation, Color, LedController};
use crate::board::LED_COUNT;
use esp_hal::{gpio::AnyPin, peripherals::RMT};

// Signal holds the latest command — a new command overwrites a pending one.
static LED_SIGNAL: Signal<CriticalSectionRawMutex, LedCommand> = Signal::new();

#[derive(Clone, Copy)]
pub enum LedCommand {
    /// Run animation once then stop
    Once(Animation),
    /// Run animation forever until next command
    Loop(Animation),
    /// Immediately set one LED
    Set(usize, Color),
    /// Immediately set all LEDs
    SetAll(Color),
    /// Turn off all LEDs
    Clear,
    /// Set global brightness (0-255)
    Brightness(u8),
}

/// Send a command to the LED task — cancels any running animation
pub fn led_send(command: LedCommand) {
    LED_SIGNAL.signal(command);
}

/// Spawn the LED task. Call once from main.
pub fn led_spawn(spawner: &Spawner, rmt: RMT<'static>, pin: AnyPin<'static>) {
    spawner.spawn(led_task(rmt, pin).unwrap());
}

#[embassy_executor::task]
async fn led_task(rmt: RMT<'static>, pin: AnyPin<'static>) {
    let mut led = LedController::<LED_COUNT>::new(rmt, pin);

    // Block for the first command, then run each command until the next one
    // arrives. `execute` returns the command that should run next.
    let mut command = LED_SIGNAL.wait().await;
    loop {
        command = execute(&mut led, command).await;
    }
}

/// Execute one command and return the command to run next.
///
/// Long-running animations race against `LED_SIGNAL`: when a new command
/// arrives the animation future is dropped at its current `.await` point,
/// giving clean, poll-free cancellation. The animation stepping logic lives in
/// a single place (`animations.rs`) and is not duplicated here.
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
                // Finished a single cycle — idle until the next command.
                Either::First(()) => LED_SIGNAL.wait().await,
                // Interrupted by a new command.
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
