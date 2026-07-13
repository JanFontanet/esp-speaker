extern crate alloc;

use alloc::string::String;
use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::{Config, Interface, WifiController, sta::StationConfig};
use static_cell::StaticCell;

use super::{DeviceConfig, WifiError};
use crate::config::{DHCP_TIMEOUT_SECS, STA_RECONNECT_DELAY_SECS};

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

pub async fn connect(
    spawner: &Spawner,
    controller: WifiController<'static>,
    device: Interface<'static>,
    creds: &DeviceConfig,
) -> Result<Stack<'static>, WifiError> {
    defmt::info!("wifi: connecting to '{}'", creds.ssid());
    static CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();
    let controller = CONTROLLER.init(controller);

    controller
        .set_config(&Config::Station(
            StationConfig::default()
                .with_ssid(creds.ssid())
                .with_password(String::from(creds.password())),
        ))
        .map_err(|_| WifiError::StaConnectFailed)?;

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        device,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<5>, StackResources::<5>::new()),
        seed,
    );

    spawner.spawn(sta_net_task(runner).unwrap());
    spawner.spawn(sta_wifi_task(controller).unwrap());

    // Wait for DHCP
    defmt::info!("wifi: waiting for DHCP...");
    embassy_time::with_timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        stack.wait_config_up(),
    )
    .await
    .map_err(|_| WifiError::DhcpFailed)?;

    let config = stack.config_v4().unwrap();
    let addr = config.address.address();
    defmt::info!(
        "wifi: connected! IP = {}.{}.{}.{}",
        addr.octets()[0],
        addr.octets()[1],
        addr.octets()[2],
        addr.octets()[3],
    );

    Ok(stack)
}

// ── Embassy tasks ─────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn sta_net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn sta_wifi_task(controller: &'static mut WifiController<'static>) {
    loop {
        match controller.connect_async().await {
            Ok(_) => {
                defmt::info!("wifi: connected");
                // Wait for disconnect
                match controller.wait_for_disconnect_async().await {
                    Ok(_) => defmt::warn!("wifi: disconnected, reconnecting..."),
                    Err(_) => defmt::warn!("wifi: disconnect error, reconnecting..."),
                }
            }
            Err(_) => {
                defmt::warn!("wifi: connect failed, retrying in 5s...");
            }
        }
        Timer::after(Duration::from_secs(STA_RECONNECT_DELAY_SECS)).await;
    }
}
