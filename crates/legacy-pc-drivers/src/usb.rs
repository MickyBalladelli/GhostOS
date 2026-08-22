//! USB class protocol support shared by xHCI host drivers.

use ghostos_ghostfs::BlockDevice;

pub const BOT_CBW_SIGNATURE: u32 = 0x4342_5355;
pub const BOT_CSW_SIGNATURE: u32 = 0x5342_5355;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbError {
    InvalidDescriptor,
    InvalidResponse,
    Stall,
    Transport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HidKeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
}

impl HidKeyboardReport {
    pub fn decode(bytes: &[u8]) -> Result<Self, UsbError> {
        let report = bytes.get(..8).ok_or(UsbError::InvalidResponse)?;
        let mut keys = [0; 6];
        keys.copy_from_slice(&report[2..8]);
        Ok(Self {
            modifiers: report[0],
            keys,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HidMouseReport {
    pub buttons: u8,
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
}

impl HidMouseReport {
    pub fn decode(bytes: &[u8]) -> Result<Self, UsbError> {
        let report = bytes.get(..3).ok_or(UsbError::InvalidResponse)?;
        Ok(Self {
            buttons: report[0] & 0x1f,
            x: report[1] as i8,
            y: report[2] as i8,
            wheel: bytes.get(3).copied().unwrap_or(0) as i8,
        })
    }
}

#[derive(Clone, Copy)]
pub struct ScsiCommand {
    bytes: [u8; 16],
    length: u8,
}

impl ScsiCommand {
    pub fn read_10(lba: u32, blocks: u16) -> Self {
        let mut bytes = [0; 16];
        bytes[0] = 0x28;
        bytes[2..6].copy_from_slice(&lba.to_be_bytes());
        bytes[7..9].copy_from_slice(&blocks.to_be_bytes());
        Self { bytes, length: 10 }
    }

    pub fn write_10(lba: u32, blocks: u16) -> Self {
        let mut command = Self::read_10(lba, blocks);
        command.bytes[0] = 0x2a;
        command
    }

    pub const fn synchronize_cache() -> Self {
        let mut bytes = [0; 16];
        bytes[0] = 0x35;
        Self { bytes, length: 10 }
    }

    pub const fn unmap(parameter_length: u16) -> Self {
        let mut bytes = [0; 16];
        bytes[0] = 0x42;
        bytes[7] = (parameter_length >> 8) as u8;
        bytes[8] = parameter_length as u8;
        Self { bytes, length: 10 }
    }
}

#[derive(Clone, Copy)]
pub struct BotCommandBlockWrapper {
    pub tag: u32,
    pub transfer_bytes: u32,
    pub data_in: bool,
    pub logical_unit: u8,
    pub command: ScsiCommand,
}

impl BotCommandBlockWrapper {
    pub fn encode(self) -> [u8; 31] {
        let mut bytes = [0; 31];
        bytes[0..4].copy_from_slice(&BOT_CBW_SIGNATURE.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.tag.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.transfer_bytes.to_le_bytes());
        bytes[12] = if self.data_in { 0x80 } else { 0 };
        bytes[13] = self.logical_unit & 0x0f;
        bytes[14] = self.command.length;
        bytes[15..31].copy_from_slice(&self.command.bytes);
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BotCommandStatusWrapper {
    pub tag: u32,
    pub residue: u32,
    pub status: u8,
}

impl BotCommandStatusWrapper {
    pub fn decode(bytes: &[u8]) -> Result<Self, UsbError> {
        let bytes = bytes.get(..13).ok_or(UsbError::InvalidResponse)?;
        if u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([0; 4])) != BOT_CSW_SIGNATURE {
            return Err(UsbError::InvalidResponse)
        }
        Ok(Self {
            tag: u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4])),
            residue: u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4])),
            status: bytes[12],
        })
    }
}

/// Bulk endpoints supplied by an initialized USB host controller.
pub trait UsbBulkTransport {
    fn bulk_out(&mut self, endpoint: u8, bytes: &[u8]) -> Result<(), UsbError>;
    fn bulk_in(&mut self, endpoint: u8, bytes: &mut [u8]) -> Result<(), UsbError>;
    fn reset_mass_storage(&mut self, interface: u8) -> Result<(), UsbError>;
    fn clear_halt(&mut self, endpoint: u8) -> Result<(), UsbError>;
}

pub struct UsbMassStorage<T> {
    transport: T,
    interface: u8,
    bulk_in: u8,
    bulk_out: u8,
    logical_unit: u8,
    block_size: usize,
    next_tag: u32,
}

impl<T: UsbBulkTransport> UsbMassStorage<T> {
    pub fn new(
        transport: T,
        interface: u8,
        bulk_in: u8,
        bulk_out: u8,
        logical_unit: u8,
        block_size: usize,
    ) -> Result<Self, UsbError> {
        if bulk_in & 0x80 == 0
            || bulk_out & 0x80 != 0
            || bulk_in & 0x0f == 0
            || bulk_out & 0x0f == 0
            || !matches!(block_size, 512 | 4096)
        {
            return Err(UsbError::InvalidDescriptor)
        }
        Ok(Self {
            transport,
            interface,
            bulk_in,
            bulk_out,
            logical_unit,
            block_size,
            next_tag: 1,
        })
    }

    fn command(
        &mut self,
        command: ScsiCommand,
        data_in: bool,
        mut input: Option<&mut [u8]>,
        output: Option<&[u8]>,
    ) -> Result<(), UsbError> {
        let transfer_bytes = input
            .as_ref()
            .map_or_else(|| output.map_or(0, |bytes| bytes.len()), |bytes| bytes.len());
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1).max(1);
        let cbw = BotCommandBlockWrapper {
            tag,
            transfer_bytes: u32::try_from(transfer_bytes).map_err(|_| UsbError::Transport)?,
            data_in,
            logical_unit: self.logical_unit,
            command,
        }
        .encode();
        self.transport.bulk_out(self.bulk_out, &cbw)?;
        if let Some(bytes) = input.as_deref_mut() {
            self.transport.bulk_in(self.bulk_in, bytes)?
        } else if let Some(bytes) = output {
            self.transport.bulk_out(self.bulk_out, bytes)?
        }
        let mut status = [0; 13];
        self.transport.bulk_in(self.bulk_in, &mut status)?;
        let status = BotCommandStatusWrapper::decode(&status)?;
        if status.tag != tag || status.residue != 0 || status.status != 0 {
            let _ = self.transport.reset_mass_storage(self.interface);
            let _ = self.transport.clear_halt(self.bulk_in);
            let _ = self.transport.clear_halt(self.bulk_out);
            return Err(UsbError::InvalidResponse)
        }
        Ok(())
    }
}

impl<T: UsbBulkTransport> BlockDevice for UsbMassStorage<T> {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), ()> {
        if output.is_empty() || output.len() % self.block_size != 0 {
            return Err(())
        }
        self.command(
            ScsiCommand::read_10(
                u32::try_from(block).map_err(|_| ())?,
                u16::try_from(output.len() / self.block_size).map_err(|_| ())?,
            ),
            true,
            Some(output),
            None,
        )
        .map_err(|_| ())
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), ()> {
        if input.is_empty() || input.len() % self.block_size != 0 {
            return Err(())
        }
        self.command(
            ScsiCommand::write_10(
                u32::try_from(block).map_err(|_| ())?,
                u16::try_from(input.len() / self.block_size).map_err(|_| ())?,
            ),
            false,
            None,
            Some(input),
        )
        .map_err(|_| ())
    }

    fn flush(&mut self) -> Result<(), ()> {
        self.command(ScsiCommand::synchronize_cache(), false, None, None)
            .map_err(|_| ())
    }

    fn discard_block(&mut self, block: u64) -> Result<(), ()> {
        let mut parameter = [0; 24];
        parameter[1] = 22;
        parameter[3] = 16;
        parameter[8..16].copy_from_slice(&block.to_be_bytes());
        parameter[16..20].copy_from_slice(&1u32.to_be_bytes());
        self.command(
            ScsiCommand::unmap(parameter.len() as u16),
            false,
            None,
            Some(&parameter),
        )
        .map_err(|_| ())
    }
}
