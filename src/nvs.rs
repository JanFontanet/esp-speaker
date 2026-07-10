use core::result::Result;
use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::{self, Error as PartitionError, FlashRegion};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

use crate::wifi::WifiCredentials;

const WIFI_CREDENTIALS_OFFSET: u32 = 0;
const CREDENTIALS_SIZE: usize = 100;

// ─── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, defmt::Format)]
pub enum NvsError {
    Partition(PartitionError),
    InvalidData,
    NotFound,
}

impl From<PartitionError> for NvsError {
    fn from(e: PartitionError) -> Self {
        NvsError::Partition(e)
    }
}

// ─── Serialization ───────────────────────────────────────────────────────────

trait IntoBytes {
    fn to_bytes(&self) -> [u8; 100];
    fn from_bytes(bytes: &[u8; 100]) -> Result<Self, NvsError>
    where
        Self: Sized;
}

impl IntoBytes for WifiCredentials {
    fn to_bytes(&self) -> [u8; CREDENTIALS_SIZE] {
        // Serializa manualmente: [ssid_len (1B)][ssid (32B)][password_len (1B)][password (64B)]
        let mut bytes = [0u8; CREDENTIALS_SIZE];
        bytes[0] = self.ssid_len as u8;
        bytes[1..33].copy_from_slice(&self.ssid);
        bytes[33] = self.password_len as u8;
        bytes[34..98].copy_from_slice(&self.password);
        bytes
    }

    fn from_bytes(bytes: &[u8; CREDENTIALS_SIZE]) -> Result<Self, NvsError> {
        let ssid_len = bytes[0] as usize;
        let password_len = bytes[33] as usize;

        if ssid_len > 32 || password_len > 64 {
            return Err(NvsError::InvalidData);
        }

        let mut ssid = [0u8; 32];
        let mut password = [0u8; 64];
        ssid[..ssid_len].copy_from_slice(&bytes[1..1 + ssid_len]);
        password[..password_len].copy_from_slice(&bytes[34..34 + password_len]);

        Ok(Self {
            ssid,
            password,
            ssid_len,
            password_len,
        })
    }
}

// ─── NVS Manager ─────────────────────────────────────────────────────────────

pub struct Nvs<'a> {
    flash: FlashStorage<'a>,
    pt_mem: [u8; partitions::PARTITION_TABLE_MAX_LEN],
}

impl<'a> Nvs<'a> {
    pub fn new(flash: FLASH<'a>) -> Self {
        Self {
            flash: FlashStorage::new(flash),
            pt_mem: [0u8; partitions::PARTITION_TABLE_MAX_LEN],
        }
    }

    /// Get the NVS flash region. Both flash and pt_mem live in self,
    /// so no lifetime issues.
    fn nvs_region(&mut self) -> Result<FlashRegion<'_, FlashStorage<'a>>, NvsError> {
        let pt = partitions::read_partition_table(&mut self.flash, &mut self.pt_mem)?;

        let nvs = pt
            .find_partition(partitions::PartitionType::Data(
                partitions::DataPartitionSubType::Nvs,
            ))?
            .ok_or(NvsError::InvalidData)?;

        Ok(nvs.as_embedded_storage(&mut self.flash))
    }

    // ── Public API ───────────────────────────────────────────────────────────

    pub fn save_credentials(&mut self, ssid: &str, password: &str) -> Result<(), NvsError> {
        let creds = WifiCredentials::new(ssid, password);
        let bytes = creds.to_bytes();

        self.nvs_region()?
            .write(WIFI_CREDENTIALS_OFFSET, &bytes)
            .map_err(|_| NvsError::InvalidData)?;

        defmt::info!("nvs: credentials saved");
        Ok(())
    }

    pub fn load_credentials(&mut self) -> Result<WifiCredentials, NvsError> {
        let mut bytes = [0u8; CREDENTIALS_SIZE];

        self.nvs_region()?
            .read(WIFI_CREDENTIALS_OFFSET, &mut bytes)
            .map_err(|_| NvsError::InvalidData)?;

        let creds = WifiCredentials::from_bytes(&bytes)?;
        defmt::info!("nvs: credentials loaded");
        Ok(creds)
    }

    pub fn clear_credentials(&mut self) -> Result<(), NvsError> {
        self.nvs_region()?
            .write(WIFI_CREDENTIALS_OFFSET, &[0u8; CREDENTIALS_SIZE])
            .map_err(|_| NvsError::InvalidData)?;

        defmt::info!("nvs: credentials cleared");
        Ok(())
    }

}
