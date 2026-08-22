//! Physical AHCI and NVMe backends for the GhostFS block queue.

use ghostos_ghostfs::BlockDevice;

use crate::storage::{
    AhciCommandList, AhciCommandTable, AhciPort, DriverError, NvmeCommand, NvmeQueue,
};

pub struct AhciBlockDevice<'a> {
    port: &'a mut AhciPort,
    command_list: &'a mut AhciCommandList,
    command_table: &'a mut AhciCommandTable,
    command_table_physical: u64,
    dma: &'a mut [u8],
    dma_physical: u64,
    sector_size: usize,
    spin_limit: usize,
}

impl<'a> AhciBlockDevice<'a> {
    pub fn new(
        port: &'a mut AhciPort,
        command_list: &'a mut AhciCommandList,
        command_table: &'a mut AhciCommandTable,
        command_table_physical: u64,
        dma: &'a mut [u8],
        dma_physical: u64,
        sector_size: usize,
        spin_limit: usize,
    ) -> Result<Self, DriverError> {
        if !matches!(sector_size, 512 | 4096)
            || dma.len() < ghostos_ghostfs::MAX_BLOCK_IO_BYTES.max(512)
            || dma_physical == 0
        {
            return Err(DriverError::InvalidRequest)
        }
        Ok(Self {
            port,
            command_list,
            command_table,
            command_table_physical,
            dma,
            dma_physical,
            sector_size,
            spin_limit,
        })
    }

    fn transfer(&mut self, block: u64, bytes: usize, write: bool) -> Result<(), ()> {
        if bytes == 0 || bytes > self.dma.len() || bytes % self.sector_size != 0 {
            return Err(())
        }
        let sectors = u16::try_from(bytes / self.sector_size).map_err(|_| ())?;
        self.command_table
            .prepare_dma(write, block, sectors, self.dma_physical, bytes as u32)
            .map_err(|_| ())?;
        self.command_table.prepare_header(
            &mut self.command_list.entries[0],
            self.command_table_physical,
            write,
        );
        self.port.issue(0, self.spin_limit).map_err(|_| ())
    }
}

impl BlockDevice for AhciBlockDevice<'_> {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), ()> {
        self.transfer(block, output.len(), false)?;
        output.copy_from_slice(&self.dma[..output.len()]);
        Ok(())
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), ()> {
        if input.len() > self.dma.len() {
            return Err(())
        }
        self.dma[..input.len()].copy_from_slice(input);
        self.transfer(block, input.len(), true)
    }

    fn flush(&mut self) -> Result<(), ()> {
        self.command_table.prepare_flush(
            &mut self.command_list.entries[0],
            self.command_table_physical,
        );
        self.port.issue(0, self.spin_limit).map_err(|_| ())
    }

    fn discard_block(&mut self, block: u64) -> Result<(), ()> {
        self.dma[..512].fill(0);
        self.dma[..6].copy_from_slice(&block.to_le_bytes()[..6]);
        self.dma[6..8].copy_from_slice(&1u16.to_le_bytes());
        self.command_table.prepare_trim(
            &mut self.command_list.entries[0],
            self.command_table_physical,
            self.dma_physical,
        );
        self.port.issue(0, self.spin_limit).map_err(|_| ())
    }
}

pub struct NvmeBlockDevice<'a> {
    queue: &'a mut NvmeQueue,
    namespace_id: u32,
    dma: &'a mut [u8],
    dma_physical: u64,
    block_size: usize,
    spin_limit: usize,
}

impl<'a> NvmeBlockDevice<'a> {
    pub fn new(
        queue: &'a mut NvmeQueue,
        namespace_id: u32,
        dma: &'a mut [u8],
        dma_physical: u64,
        block_size: usize,
        spin_limit: usize,
    ) -> Result<Self, DriverError> {
        if namespace_id == 0
            || !matches!(block_size, 512 | 4096)
            || dma.len() < ghostos_ghostfs::MAX_BLOCK_IO_BYTES
            || dma_physical == 0
            || dma_physical & 0xfff != 0
        {
            return Err(DriverError::InvalidRequest)
        }
        Ok(Self {
            queue,
            namespace_id,
            dma,
            dma_physical,
            block_size,
            spin_limit,
        })
    }

    fn transfer(&mut self, block: u64, bytes: usize, write: bool) -> Result<(), ()> {
        if bytes == 0 || bytes > self.dma.len() || bytes % self.block_size != 0 {
            return Err(())
        }
        let blocks = u16::try_from(bytes / self.block_size).map_err(|_| ())?;
        let command = if write {
            NvmeCommand::write(self.namespace_id, block, blocks - 1, self.dma_physical)
        } else {
            NvmeCommand::read(self.namespace_id, block, blocks - 1, self.dma_physical)
        };
        self.queue.submit(command, self.spin_limit).map(|_| ()).map_err(|_| ())
    }
}

impl BlockDevice for NvmeBlockDevice<'_> {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), ()> {
        self.transfer(block, output.len(), false)?;
        output.copy_from_slice(&self.dma[..output.len()]);
        Ok(())
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), ()> {
        if input.len() > self.dma.len() {
            return Err(())
        }
        self.dma[..input.len()].copy_from_slice(input);
        self.transfer(block, input.len(), true)
    }

    fn flush(&mut self) -> Result<(), ()> {
        self.queue
            .submit(NvmeCommand::flush(self.namespace_id), self.spin_limit)
            .map(|_| ())
            .map_err(|_| ())
    }

    fn discard_block(&mut self, block: u64) -> Result<(), ()> {
        self.dma[..16].fill(0);
        self.dma[4..8].copy_from_slice(&1u32.to_le_bytes());
        self.dma[8..16].copy_from_slice(&block.to_le_bytes());
        self.queue
            .submit(
                NvmeCommand::discard(self.namespace_id, self.dma_physical),
                self.spin_limit,
            )
            .map(|_| ())
            .map_err(|_| ())
    }
}
