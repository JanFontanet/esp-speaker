#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::Pin;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;

use espeaker::{
    audio::Audio,
    led::{Animation, Color, LedCommand, led_send, led_spawn},
    nvs::Nvs,
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

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    // -------------- Embassy initialized ----------------
    info!("Start Rock&Roll!");

//     use esp_hal::i2c::master::{Config as I2cConfig, I2c};
// let mut i2c_scan = I2c::new(peripherals.I2C0, I2cConfig::default())
//     .unwrap()
//     .with_sda(peripherals.GPIO11)
//     .with_scl(peripherals.GPIO10);

// defmt::info!("Scanning I2C bus...");
// for addr in 0x08..=0x77u8 {
//     let mut buf = [0u8; 1];
//     match i2c_scan.read(addr, &mut buf) {
//         Ok(_)  => defmt::info!("  Found device at 0x{:02x}", addr),
//         Err(_) => {}
//     }
// }
// defmt::info!("I2C scan done");
// loop {
//     Timer::after(Duration::from_secs(1)).await;
// }


    let flash = peripherals.FLASH;
    let mut nvs = Nvs::new(flash);
    led_spawn(&spawner, peripherals.RMT, peripherals.GPIO38.degrade());
    led_send(LedCommand::Brightness(10));
    led_send(LedCommand::Loop(Animation::Chase {
        color: Color::GREEN,
        speed: 2,
    }));

    // TODO: Add a way to do a factory reset.

    let wifi = wifi::init(peripherals.WIFI).unwrap();

    match nvs.load_credentials() {
        Ok(creds) => {
            info!(
                "Loaded WiFi credentials: SSID: {:?}, pwd: {}",
                &creds.ssid_str(),
                &creds.password_str()
            );
            let audio = Audio::new(
                peripherals.I2S0,
                peripherals.DMA_CH0,
                peripherals.GPIO12,
                peripherals.GPIO13,
                peripherals.GPIO14,
                peripherals.GPIO15,
                peripherals.I2C0,
                peripherals.GPIO11,
                peripherals.GPIO10,
            )
            .unwrap();
            main_loop(spawner, wifi, creds, audio).await;
            esp_hal::system::software_reset();
        }
        Err(e) => {
            info!("No WiFi credentials found in NVS: {:?}", e);
            no_creds_boot(nvs, spawner, wifi).await;
            Timer::after(Duration::from_secs(1)).await;
            esp_hal::system::software_reset();
        }
    };
}

async fn main_loop(
    spawner: Spawner,
    wifi: wifi::WifiResources<'static>,
    creds: wifi::WifiCredentials,
    mut audio: Audio<'_>,
) -> ! {
    let _stack = wifi::sta::connect(&spawner, wifi.controller, wifi.interfaces.station, &creds)
        .await
        .unwrap();

    defmt::info!("Ready! Stack is up.");
    led_send(LedCommand::Clear);

    audio.play_connected().await.unwrap();
    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(1)).await;
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
