pub mod ap;
pub mod sta;

use esp_hal::peripherals::WIFI;
use esp_radio::wifi::{ControllerConfig, Interfaces, WifiController};

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

// Credenciales con longitudes fijas (máx. 32 bytes para SSID, 64 para contraseña)
pub struct WifiCredentials {
    pub ssid: [u8; 32],
    pub password: [u8; 64],
    pub ssid_len: usize,     // Longitud real del SSID
    pub password_len: usize, // Longitud real de la contraseña
}

impl WifiCredentials {
    pub fn new(ssid: &str, password: &str) -> Self {
        let mut ssid_bytes = [0u8; 32];
        let mut password_bytes = [0u8; 64];

        // Copia SSID y contraseña a los arrays
        ssid_bytes[..ssid.len()].copy_from_slice(ssid.as_bytes());
        password_bytes[..password.len()].copy_from_slice(password.as_bytes());

        Self {
            ssid: ssid_bytes,
            password: password_bytes,
            ssid_len: ssid.len(),
            password_len: password.len(),
        }
    }

    pub fn ssid_str(&self) -> &str {
        core::str::from_utf8(&self.ssid[..self.ssid_len]).unwrap_or("")
    }

    pub fn password_str(&self) -> &str {
        core::str::from_utf8(&self.password[..self.password_len]).unwrap_or("")
    }
}
