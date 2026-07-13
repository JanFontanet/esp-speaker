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
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use static_cell::StaticCell;

use espeaker::{
    audio::{Sound, audio_send, audio_spawn},
    board::{Board, I2cBus},
    boot,
    button::button_spawn,
    led::{Animation, Color, LedCommand, led_send, led_spawn},
    mqtt::{
        mqtt::{self, CHANNEL_SIZE, CmdSender, EventReceiver},
        msg_protocol::{AppEvent, AudioCommand},
    },
    nvs::{Nvs, NvsError},
    time, wifi,
};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

static CMD_CHANNEL: StaticCell<Channel<CriticalSectionRawMutex, AudioCommand, CHANNEL_SIZE>> =
    StaticCell::new();
static EVENT_CHANNEL: StaticCell<Channel<CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>> =
    StaticCell::new();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    let board = Board::new(peripherals);

    // -------------- Initializing embassy ----------------
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(board.timg0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(board.sw_interrupt);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    // -------------- Embassy initialized ----------------
    info!("Start Rock&Roll!");

    let cmd_chan = CMD_CHANNEL.init(Channel::new());
    let event_chan = EVENT_CHANNEL.init(Channel::new());
    let cmd_tx: CmdSender = cmd_chan.sender();
    let cmd_rx = cmd_chan.receiver();
    let event_tx = event_chan.sender();
    let event_rx: EventReceiver<AppEvent> = event_chan.receiver();

    let i2c = I2c::new(board.i2c0, I2cConfig::default())
        .unwrap()
        .with_sda(board.i2c_sda)
        .with_scl(board.i2c_scl);
    static I2C_BUS: StaticCell<I2cBus> = StaticCell::new();
    let i2c_bus: &'static I2cBus = I2C_BUS.init(Mutex::new(i2c));

    let mut nvs = Nvs::new(board.flash);
    led_spawn(&spawner, board.rmt, board.led_pin); // TODO: we can use cmds & events here too
    button_spawn(&spawner, board.boot_button); // TODO: we can use cmds & events here too
    audio_spawn(&spawner, board.audio, i2c_bus, cmd_rx, event_tx.clone());

    led_send(LedCommand::Brightness(10));
    led_send(LedCommand::Loop(Animation::Chase {
        color: Color::GREEN,
        speed: 2,
    }));

    // ---------------- Factory reset requested! ---------------
    if boot::take_factory_reset() {
        info!("Factory reset: clearing stored WiFi credentials");
        let _ = nvs.clear_config();
        boot::set_sta_fail_count(0);
    }

    // User AP may be down or he moved, requesting config again,
    // user can reset to try again with existing config
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
        nvs.load_config()
    };

    match creds {
        Ok(creds) => {
            info!(
                "Loaded config: name={:?}, ssid={:?}",
                creds.name(),
                creds.ssid()
            );
            // Count this attempts since it is cleared on success.
            boot::set_sta_fail_count(boot::sta_fail_count() + 1);
            match wifi::sta::connect(&spawner, wifi.controller, wifi.interfaces.station, &creds)
                .await
            {
                Ok(stack) => {
                    boot::set_sta_fail_count(0);
                    time::time_spawn(&spawner, stack, i2c_bus);
                    mqtt::mqtt_spawn(&spawner, stack, &creds, cmd_tx, event_rx);
                    ready().await
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

async fn ready() -> ! {
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

    match wifi::ap::start_ap(
        &spawner,
        wifi.controller,
        wifi.interfaces.access_point,
        |_req| include_str!("../html/portal.html"),
    )
    .await
    {
        Ok(config) => {
            // Save credentials to NVS
            if let Err(e) = nvs.save_config(&config) {
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
    led_send(LedCommand::SetAll(Color::RED));
    esp_hal::system::software_reset();
}
