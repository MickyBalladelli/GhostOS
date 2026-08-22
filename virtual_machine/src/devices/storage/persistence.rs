use super::{DiskImage, StorageError};
use crate::devices::{DeviceError, PortDevice};
use std::cell::RefCell;
use std::rc::Rc;
use ghostos_boot_protocol::{
    GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_DATA_PORT,
    GHOSTOS_PERSISTENCE_LENGTH_PORT, GHOSTOS_PERSISTENCE_LOAD, GHOSTOS_PERSISTENCE_MAX_BYTES,
    GHOSTOS_PERSISTENCE_FLUSH, GHOSTOS_PERSISTENCE_SAVE,
};

const SECTOR_SIZE: usize = 512;
const REGION_BYTES: usize = 64 * 1024;
const HEADER_BYTES: usize = SECTOR_SIZE;
const REGION_SECTORS: u64 = (REGION_BYTES / SECTOR_SIZE) as u64;
const MAGIC: &[u8; 8] = b"SYNOPS01";
const VERSION: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Read,
    Write,
}

/// Private VM port used by the shell's tiny filesystem until a real guest
/// filesystem driver is available. State is stored in the tail of a
/// persistent disk image, outside the controller's normal guest sectors.
pub struct SynosPersistencePort {
    image: Option<DiskImage>,
    base_sector: u64,
    bytes: [u8; GHOSTOS_PERSISTENCE_MAX_BYTES],
    length: usize,
    cursor: usize,
    expected_length: usize,
    mode: Mode,
}

impl SynosPersistencePort {
    pub fn new() -> Self {
        Self {
            image: None,
            base_sector: 0,
            bytes: [0; GHOSTOS_PERSISTENCE_MAX_BYTES],
            length: 0,
            cursor: 0,
            expected_length: 0,
            mode: Mode::Idle,
        }
    }

    pub fn attach_image(&mut self, mut image: DiskImage) -> Result<(), StorageError> {
        if image.sector_count() < REGION_SECTORS {
            return Err(StorageError::InvalidImage(
                "disk is too small for GhostOS persistence metadata".to_string(),
            ));
        }
        self.base_sector = image.sector_count() - REGION_SECTORS;
        self.length = 0;
        self.cursor = 0;
        self.mode = Mode::Idle;
        self.load_region(&mut image)?;
        self.image = Some(image);
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), StorageError> {
        if let Some(image) = self.image.as_mut() {
            image.sync()
        } else {
            Ok(())
        }
    }

    pub fn has_image(&self) -> bool {
        self.image.is_some()
    }

    fn load_region(&mut self, image: &mut DiskImage) -> Result<(), StorageError> {
        self.length = 0;
        self.bytes.fill(0);
        let mut region = [0u8; REGION_BYTES];
        for (index, sector) in region.chunks_exact_mut(SECTOR_SIZE).enumerate() {
            let Ok(sector) = <&mut [u8; SECTOR_SIZE]>::try_from(sector) else {
                return Err(StorageError::InvalidImage("invalid persistence sector".to_string()))
            };
            image.read_sector(self.base_sector + index as u64, sector)?;
        }
        if &region[..8] != MAGIC
            || u32::from_le_bytes([region[8], region[9], region[10], region[11]]) != VERSION
        {
            return Ok(())
        }
        let length = u32::from_le_bytes([region[12], region[13], region[14], region[15]]) as usize;
        let stored_checksum = u32::from_le_bytes([region[16], region[17], region[18], region[19]]);
        if length > GHOSTOS_PERSISTENCE_MAX_BYTES {
            return Ok(())
        }
        let payload = &region[HEADER_BYTES..HEADER_BYTES + length];
        if checksum(payload) != stored_checksum {
            return Ok(())
        }
        self.bytes[..length].copy_from_slice(payload);
        self.length = length;
        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), StorageError> {
        let Some(mut image) = self.image.take() else {
            return Ok(())
        };
        let result = self.load_region(&mut image);
        self.image = Some(image);
        result
    }

    fn persist(&mut self) -> Result<(), StorageError> {
        let Some(image) = self.image.as_mut() else {
            return Ok(())
        };
        let mut region = [0u8; REGION_BYTES];
        region[..8].copy_from_slice(MAGIC);
        region[8..12].copy_from_slice(&VERSION.to_le_bytes());
        region[12..16].copy_from_slice(&(self.length as u32).to_le_bytes());
        region[16..20].copy_from_slice(&checksum(&self.bytes[..self.length]).to_le_bytes());
        region[HEADER_BYTES..HEADER_BYTES + self.length]
            .copy_from_slice(&self.bytes[..self.length]);
        for (index, sector) in region.chunks_exact(SECTOR_SIZE).enumerate() {
            let Ok(sector) = <&[u8; SECTOR_SIZE]>::try_from(sector) else {
                return Err(StorageError::InvalidImage("invalid persistence sector".to_string()))
            };
            image.write_sector(self.base_sector + index as u64, sector)?;
        }
        image.sync()
    }

    fn begin_write(&mut self) {
        self.mode = Mode::Write;
        self.cursor = 0;
        self.expected_length = 0;
        self.length = 0;
        self.bytes.fill(0);
    }

    fn write_length(&mut self, value: u32) -> Result<(), DeviceError> {
        let length = value as usize;
        if length > GHOSTOS_PERSISTENCE_MAX_BYTES {
            return Err(DeviceError::InvalidAddress)
        }
        self.mode = Mode::Write;
        self.bytes.fill(0);
        self.expected_length = length;
        self.length = length;
        self.cursor = 0;
        Ok(())
    }

    fn write_data(&mut self, value: u32) -> Result<(), DeviceError> {
        if self.cursor >= self.expected_length {
            return Err(DeviceError::InvalidAddress)
        }
        let count = (self.expected_length - self.cursor).min(4);
        self.bytes[self.cursor..self.cursor + count]
            .copy_from_slice(&value.to_le_bytes()[..count]);
        self.cursor = self.cursor.saturating_add(4);
        Ok(())
    }

    fn write_data_byte(&mut self, value: u8) -> Result<(), DeviceError> {
        if self.cursor >= self.expected_length {
            return Err(DeviceError::InvalidAddress)
        }
        self.bytes[self.cursor] = value;
        self.cursor += 1;
        Ok(())
    }
}

impl Default for SynosPersistencePort {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for SynosPersistencePort {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        match port {
            GHOSTOS_PERSISTENCE_LENGTH_PORT if size == 4 => Ok(self.length as u64),
            GHOSTOS_PERSISTENCE_DATA_PORT if size == 1 && self.mode == Mode::Read => {
                let value = self.bytes.get(self.cursor).copied().unwrap_or(0);
                self.cursor = self.cursor.saturating_add(1);
                Ok(value as u64)
            }
            GHOSTOS_PERSISTENCE_DATA_PORT if size == 4 && self.mode == Mode::Read => {
                let mut word = [0u8; 4];
                let remaining = self.length.saturating_sub(self.cursor);
                let count = remaining.min(4);
                word[..count].copy_from_slice(&self.bytes[self.cursor..self.cursor + count]);
                self.cursor = self.cursor.saturating_add(4);
                Ok(u32::from_le_bytes(word) as u64)
            }
            _ => Ok(0),
        }
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        match port {
            GHOSTOS_PERSISTENCE_COMMAND_PORT if size == 1 => match value as u8 {
                GHOSTOS_PERSISTENCE_LOAD => {
                    self.load_from_disk().map_err(|_| DeviceError::NotReady)?;
                    self.mode = Mode::Read;
                    self.cursor = 0;
                    Ok(())
                }
                GHOSTOS_PERSISTENCE_SAVE => {
                    self.begin_write();
                    Ok(())
                }
                GHOSTOS_PERSISTENCE_FLUSH => {
                    self.persist().map_err(|_| DeviceError::NotReady)?;
                    self.mode = Mode::Idle;
                    Ok(())
                }
                _ => Err(DeviceError::InvalidAddress),
            },
            GHOSTOS_PERSISTENCE_LENGTH_PORT if size == 4 => self.write_length(value as u32),
            GHOSTOS_PERSISTENCE_DATA_PORT if size == 1 => self.write_data_byte(value as u8),
            GHOSTOS_PERSISTENCE_DATA_PORT if size == 4 => self.write_data(value as u32),
            _ => Err(DeviceError::UnsupportedSize),
        }
    }

    fn reset(&mut self) {
        let _ = self.persist();
        self.mode = Mode::Idle;
        self.cursor = 0;
    }
}

impl PortDevice for Rc<RefCell<SynosPersistencePort>> {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.borrow_mut().read(port, size)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.borrow_mut().write(port, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for byte in bytes {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}
