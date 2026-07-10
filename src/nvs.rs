use core::result::Result;
use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::{self, Error as PartitionError, FlashRegion};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

use crate::wifi::DeviceConfig;

const CONFIG_OFFSET: u32 = 0;
/// Fixed-size flash record: [magic(2)][version(1)][postcard DeviceConfig...].
/// A multiple of 4 (flash alignment), comfortably larger than the encoding.
const CONFIG_SIZE: usize = 160;
const CONFIG_MAGIC: u16 = 0xE59E;
const CONFIG_VERSION: u8 = 1;

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

    pub fn save_config(&mut self, config: &DeviceConfig) -> Result<(), NvsError> {
        let mut bytes = [0u8; CONFIG_SIZE];
        bytes[0..2].copy_from_slice(&CONFIG_MAGIC.to_le_bytes());
        bytes[2] = CONFIG_VERSION;
        postcard::to_slice(config, &mut bytes[3..]).map_err(|_| NvsError::InvalidData)?;

        self.nvs_region()?
            .write(CONFIG_OFFSET, &bytes)
            .map_err(|_| NvsError::InvalidData)?;

        defmt::info!("nvs: config saved");
        Ok(())
    }

    pub fn load_config(&mut self) -> Result<DeviceConfig, NvsError> {
        let mut bytes = [0u8; CONFIG_SIZE];

        self.nvs_region()?
            .read(CONFIG_OFFSET, &mut bytes)
            .map_err(|_| NvsError::InvalidData)?;

        // Reject unset/cleared flash and any older/foreign record layout.
        if u16::from_le_bytes([bytes[0], bytes[1]]) != CONFIG_MAGIC || bytes[2] != CONFIG_VERSION {
            return Err(NvsError::NotFound);
        }

        let (config, _) = postcard::take_from_bytes::<DeviceConfig>(&bytes[3..])
            .map_err(|_| NvsError::InvalidData)?;
        if config.ssid().is_empty() {
            return Err(NvsError::NotFound);
        }

        defmt::info!("nvs: config loaded");
        Ok(config)
    }

    pub fn clear_config(&mut self) -> Result<(), NvsError> {
        self.nvs_region()?
            .write(CONFIG_OFFSET, &[0u8; CONFIG_SIZE])
            .map_err(|_| NvsError::InvalidData)?;

        defmt::info!("nvs: config cleared");
        Ok(())
    }
}
