#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]
use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

use espeaker::{
    audio::{Sound, audio_send, audio_spawn},
    board::Board,
    boot,
    button::button_spawn,
    led::{Animation, Color, LedCommand, led_send, led_spawn},
    nvs::{Nvs, NvsError},
    wifi,
};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // -------------- Initializing embassy ----------------
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    // Split raw peripherals into named board resources (see board.rs).
    let board = Board::new(peripherals);

    let timg0 = TimerGroup::new(board.timg0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(board.sw_interrupt);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    // -------------- Embassy initialized ----------------
    info!("Start Rock&Roll!");

    let mut nvs = Nvs::new(board.flash);
    led_spawn(&spawner, board.rmt, board.led_pin);
    audio_spawn(&spawner, board.audio);
    button_spawn(&spawner, board.boot_button);

    led_send(LedCommand::Brightness(10));
    led_send(LedCommand::Loop(Animation::Chase {
        color: Color::GREEN,
        speed: 2,
    }));

    // A long BOOT-button press (see button_task) requests a wipe via a
    // persistent flag and reboots; honor it here now that we own the flash.
    if boot::take_factory_reset() {
        info!("Factory reset: clearing stored WiFi credentials");
        let _ = nvs.clear_credentials();
        boot::set_sta_fail_count(0);
    }

    // After too many consecutive failed connects, stop retrying (likely-bad)
    // credentials and drop into the config portal instead of reboot-looping.
    let force_portal = boot::sta_fail_count() >= boot::MAX_STA_FAILS;
    if force_portal {
        warn!(
            "{} consecutive failed connects; starting config portal",
            boot::sta_fail_count()
        );
        boot::set_sta_fail_count(0);
    }

    let wifi = wifi::init(board.wifi).unwrap();

    let creds = if force_portal {
        Err(NvsError::NotFound)
    } else {
        nvs.load_credentials()
    };

    match creds {
        Ok(creds) => {
            info!("Loaded WiFi credentials: SSID: {:?}", &creds.ssid_str());
            // Count this attempt. It's cleared on success but persists across
            // the reboot we do on failure, so repeated failures eventually
            // open the portal via `force_portal` above.
            boot::set_sta_fail_count(boot::sta_fail_count() + 1);
            match wifi::sta::connect(&spawner, wifi.controller, wifi.interfaces.station, &creds)
                .await
            {
                Ok(stack) => {
                    boot::set_sta_fail_count(0);
                    ready(stack).await;
                }
                Err(e) => {
                    error!("WiFi connect failed: {:?}; rebooting to retry", e);
                    led_send(LedCommand::SetAll(Color::RED));
                    Timer::after(Duration::from_secs(2)).await;
                    esp_hal::system::software_reset();
                }
            }
        }
        Err(e) => {
            info!("No usable WiFi credentials ({:?}); starting portal", e);
            no_creds_boot(nvs, spawner, wifi).await;
            Timer::after(Duration::from_secs(1)).await;
            esp_hal::system::software_reset();
        }
    }
}

/// Steady state once the network is up. Never returns.
async fn ready(_stack: Stack<'static>) -> ! {
    defmt::info!("Ready! Stack is up.");
    led_send(LedCommand::Clear);
    audio_send(Sound::Connected);
    loop {
        Timer::after(Duration::from_secs(30)).await;
    }
}

async fn no_creds_boot(mut nvs: Nvs<'_>, spawner: Spawner, wifi: wifi::WifiResources<'static>) {
    led_send(LedCommand::Loop(Animation::Pulse {
        color: Color::BLUE,
        speed: 5,
    }));
    // Start AP mode to get credentials
    match wifi::ap::start_ap(
        &spawner,
        wifi.controller,
        wifi.interfaces.access_point,
        |_req| include_str!("../html/portal.html"),
    )
    .await
    {
        Ok(creds) => {
            // Save credentials to NVS
            if let Err(e) = nvs.save_credentials(creds.ssid_str(), creds.password_str()) {
                led_send(LedCommand::SetAll(Color::RED));
                error!("Failed to save WiFi credentials: {:?}", e);
            } else {
                led_send(LedCommand::SetAll(Color::GREEN));
                info!("WiFi credentials saved successfully.");
            }
        }
        Err(e) => {
            error!("Failed to start AP mode: {:?}", e);
            led_send(LedCommand::SetAll(Color::RED));
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("PANIC: {}", defmt::Display2Format(info));
    // do something before reset, e.g. turn LEDs red
    esp_hal::system::software_reset();
}
