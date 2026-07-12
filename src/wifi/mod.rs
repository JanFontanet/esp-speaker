pub mod ap;
pub mod sta;

use esp_hal::peripherals::WIFI;
use esp_radio::wifi::{ControllerConfig, Interfaces, WifiController};
use heapless::String;
use serde::{Deserialize, Serialize};

pub struct WifiResources<'d> {
    pub controller: WifiController<'d>,
    pub interfaces: Interfaces<'d>,
}

pub fn init(wifi: WIFI<'static>) -> Result<WifiResources<'static>, WifiError> {
    // Start with empty config, ap/sta will set their own
    let (controller, interfaces) = esp_radio::wifi::new(wifi, ControllerConfig::default())
        .map_err(|_| WifiError::InitFailed)?;

    Ok(WifiResources {
        controller,
        interfaces,
    })
}

#[derive(Debug, defmt::Format)]
pub enum WifiError {
    InitFailed,
    ApStartFailed,
    StaConnectFailed,
    DhcpFailed,
    SocketError,
    Timeout,
    InvalidCredentials,
}

// TODO: move out of wifi module.
#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    ssid: String<32>,
    password: String<64>,
    name: String<32>,
    mqtt_address: String<32>,
    mqtt_user: String<32>,
    mqtt_pwd: String<64>,
}

impl DeviceConfig {
    pub fn new(
        ssid: &str,
        password: &str,
        name: &str,
        mqtt_address: &str,
        mqtt_user: &str,
        mqtt_pwd: &str,
    ) -> Self {
        Self {
            ssid: truncated(ssid),
            password: truncated(password),
            name: truncated(name),
            mqtt_address: truncated(mqtt_address),
            mqtt_user: truncated(mqtt_user),
            mqtt_pwd: truncated(mqtt_pwd),
        }
    }

    pub fn ssid(&self) -> &str {
        &self.ssid
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn mqtt_address(&self) -> &str {
        &self.mqtt_address
    }

    pub fn mqtt_user(&self) -> &str {
        &self.mqtt_user
    }

    pub fn mqtt_pwd(&self) -> &str {
        &self.mqtt_pwd
    }
}

/// Copy `s` into a fixed-capacity `String`, truncating at the last char
/// boundary that fits within `N`.
fn truncated<const N: usize>(s: &str) -> String<N> {
    let mut out = String::new();
    for ch in s.chars() {
        if out.push(ch).is_err() {
            break;
        }
    }
    out
}
