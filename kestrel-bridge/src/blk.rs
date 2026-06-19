//! Virtual block device driver for Kestrel Bridge.
//! Used for loop-mounting .kstl SquashFS images.

use crate::KestrelBridge;
use log::{info, debug};

pub const KESTREL_BLK_SECTOR_SIZE: usize = 512;

pub struct KestrelBlkDriver<'a> {
    bridge: &'a mut KestrelBridge,
}

impl<'a> KestrelBlkDriver<'a> {
    pub fn new(bridge: &'a mut KestrelBridge) -> Self {
        info!("[BlkDriver] Kestrel virtual block device initialized");
        Self { bridge }
    }

    /// Read sectors from the virtual block device (backed by .kstl SquashFS).
    pub fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> bool {
        debug!("[BlkDriver] Read LBA={} len={}", lba, buf.len());
        // Encode request: [8 bytes lba][4 bytes len][0x00 = READ]
        let mut req = Vec::with_capacity(13);
        req.extend_from_slice(&lba.to_le_bytes());
        req.extend_from_slice(&(buf.len() as u32).to_le_bytes());
        req.push(0x00); // READ
        self.bridge.blk_ring.write(&req)
    }

    /// Write sectors to the virtual block device.
    pub fn write_sectors(&mut self, lba: u64, data: &[u8]) -> bool {
        debug!("[BlkDriver] Write LBA={} len={}", lba, data.len());
        let mut req = Vec::with_capacity(13 + data.len());
        req.extend_from_slice(&lba.to_le_bytes());
        req.extend_from_slice(&(data.len() as u32).to_le_bytes());
        req.push(0x01); // WRITE
        req.extend_from_slice(data);
        self.bridge.blk_ring.write(&req)
    }
}
