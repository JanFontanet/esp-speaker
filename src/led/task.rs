use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use super::{Animation, Color, LedController};
use esp_hal::{gpio::AnyPin, peripherals::RMT};

// Signal holds the latest command — new command overwrites pending one
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
    let mut led = LedController::<7>::new(rmt, pin);

    loop {
        // Wait for a command
        let command = LED_SIGNAL.wait().await;

        match command {
            LedCommand::Brightness(b) => {
                led.set_brightness(b); // takes effect on next frame
            }
            LedCommand::Set(i, color) => {
                led.set(i, color).await;
            }
            LedCommand::SetAll(color) => {
                led.set_all(color).await;
            }
            LedCommand::Clear => {
                led.clear().await;
            }
            LedCommand::Once(animation) => {
                run_cancellable(&mut led, animation, 1).await;
            }
            LedCommand::Loop(animation) => {
                run_cancellable(&mut led, animation, usize::MAX).await;
            }
        }
    }
}

/// Run animation for `times` cycles, cancelling if a new signal arrives
async fn run_cancellable<const N: usize>(
    led: &mut LedController<'_, N>,
    animation: Animation,
    times: usize,
) {
    for _ in 0..times {
        // Check for new command before each cycle
        if LED_SIGNAL.signaled() {
            return;
        }

        // Run one cycle step by step, checking for cancellation
        match animation {
            Animation::Solid(color) => {
                led.set_all(color).await;
                // Solid just waits for next command
                LED_SIGNAL.wait().await;
                LED_SIGNAL.signal(LED_SIGNAL.wait().await); // put it back
                return;
            }
            Animation::Rainbow { speed } => {
                if !run_rainbow(led, speed, times == 1).await {
                    return; // cancelled
                }
            }
            Animation::Pulse { color, speed } => {
                if !run_pulse(led, color, speed, times == 1).await {
                    return; // cancelled
                }
            }
            Animation::Chase { color, speed } => {
                if !run_chase(led, color, speed, times == 1).await {
                    return; // cancelled
                }
            }
        }
    }
}

/// Returns true if completed, false if cancelled
async fn run_rainbow<const N: usize>(
    led: &mut LedController<'_, N>,
    speed: u8,
    cancelable: bool,
) -> bool {
    use embassy_time::{Duration, Timer};

    let delay = Duration::from_millis(20u64.saturating_sub(speed as u64 / 5));

    for step in 0u16..256 {
        if cancelable && LED_SIGNAL.signaled() {
            return false;
        }
        let mut colors = [Color::BLACK; N];
        for i in 0..N {
            let hue = ((step + (i as u16 * 256 / N as u16)) % 256) as u8;
            colors[i] = Color::from_hsv(hue, 255, led.brightness);
        }
        led.set_raw(&colors).await;
        Timer::after(delay).await;
    }
    true
}

/// Returns true if completed, false if cancelled
async fn run_pulse<const N: usize>(
    led: &mut LedController<'_, N>,
    color: Color,
    speed: u8,
    cancelable: bool,
) -> bool {
    use embassy_time::{Duration, Timer};
    use micromath::F32Ext;

    let delay = Duration::from_millis(10u64.saturating_sub(speed as u64 / 10));

    // Fade in + out = one pulse cycle
    for step in 0u16..256 {
        if cancelable && LED_SIGNAL.signaled() {
            return false;
        }
        let t = step as f32 / 255.0;
        let scale = (t * core::f32::consts::PI).sin();
        let c = color.scale((scale * led.brightness as f32) as u8);
        led.set_all(c).await;
        Timer::after(delay).await;
    }
    true
}

/// Returns true if completed, false if cancelled
async fn run_chase<const N: usize>(
    led: &mut LedController<'_, N>,
    color: Color,
    speed: u8,
    cancelable: bool,
) -> bool {
    use embassy_time::{Duration, Timer};

    let delay = Duration::from_millis(100u64.saturating_sub(speed as u64 * 10));

    for i in 0..N {
        if cancelable && LED_SIGNAL.signaled() {
            return false;
        }
        let mut colors = [Color::BLACK; N];
        colors[i] = color.scale(led.brightness);
        if i > 0 {
            colors[i - 1] = color.scale(led.brightness / 3);
        }
        led.set_raw(&colors).await;
        Timer::after(delay).await;
    }
    true
}
