//! kestrel-bridge: Linux kernel bridge for Kestrel OS.
//!
//! This Rust library is compiled as a Linux kernel module that acts as the
//! paravirtualized hardware abstraction layer between the Kestrel Linux guest
//! and the Windows host (kestrel-host.exe).
//!
//! Communication channels:
//!   - Shared memory ring buffers via a virtio-like protocol
//!   - I/O port 0x3F8 (COM1) for the serial console / kestrel-term
//!   - I/O port 0xE00x for Kestrel-specific hypercalls
//!
//! The module registers:
//!   - A virtual network driver (kestrel_net) for raw socket forwarding
//!   - A virtual block driver (kestrel_blk) for .kstl loop-mounting
//!   - A serial console driver for kestrel-term

pub mod ring_buffer;
pub mod net;
pub mod blk;
pub mod console;
pub mod hypercall;

use ring_buffer::SharedRingBuffer;
use log::{info, warn, debug};

/// Magic number for the Kestrel shared memory region header.
pub const KESTREL_SHM_MAGIC: u32 = 0x4B53544C; // 'KSTL'

/// Base I/O port for Kestrel hypercalls.
pub const KESTREL_IO_BASE: u16 = 0xE000;

/// Kestrel hypercall identifiers (written to KESTREL_IO_BASE).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KestrelHypercall {
    /// Notify host that a graphics framebuffer update is available.
    GraphicsUpdate   = 0x01,
    /// Request a network packet transmission.
    NetTx            = 0x02,
    /// Acknowledge a received network packet.
    NetRxAck         = 0x03,
    /// Request a block I/O operation.
    BlkRequest       = 0x04,
    /// Notify host that a serial console byte is available (kestrel-term).
    SerialTx         = 0x05,
    /// Request system time from the Windows host.
    GetTime          = 0x06,
    /// Graceful shutdown request.
    Shutdown         = 0xFF,
}

/// The main Kestrel Bridge context.
pub struct KestrelBridge {
    /// Shared memory ring buffer for graphics commands.
    pub graphics_ring: SharedRingBuffer,
    /// Shared memory ring buffer for network packets (TX path).
    pub net_tx_ring:   SharedRingBuffer,
    /// Shared memory ring buffer for network packets (RX path).
    pub net_rx_ring:   SharedRingBuffer,
    /// Shared memory ring buffer for block device I/O.
    pub blk_ring:      SharedRingBuffer,
    /// Shared memory ring buffer for serial console data.
    pub serial_ring:   SharedRingBuffer,
}

impl KestrelBridge {
    /// Initialize the bridge with a pointer to the shared memory region.
    ///
    /// The shared memory layout (all offsets from base):
    ///   [0x00000]  Magic (4 bytes) = KSTL
    ///   [0x00004]  Version (4 bytes)
    ///   [0x00100]  Graphics ring buffer (1MB)
    ///   [0x10100]  Net TX ring buffer (256KB)
    ///   [0x14100]  Net RX ring buffer (256KB)
    ///   [0x18100]  Block ring buffer (256KB)
    ///   [0x1C100]  Serial ring buffer (4KB)
    pub unsafe fn from_shared_memory(base: *mut u8) -> Result<Self, &'static str> {
        // Verify magic
        let magic = (base as *const u32).read_volatile();
        if magic != KESTREL_SHM_MAGIC {
            return Err("Invalid shared memory magic");
        }

        Ok(Self {
            graphics_ring: SharedRingBuffer::new(base.add(0x00100), 1024 * 1024),
            net_tx_ring:   SharedRingBuffer::new(base.add(0x10100), 256 * 1024),
            net_rx_ring:   SharedRingBuffer::new(base.add(0x14100), 256 * 1024),
            blk_ring:      SharedRingBuffer::new(base.add(0x18100), 256 * 1024),
            serial_ring:   SharedRingBuffer::new(base.add(0x1C100), 4096),
        })
    }

    /// Forward a graphics framebuffer command to the Windows host.
    pub fn send_graphics_update(&mut self, data: &[u8]) -> bool {
        if self.graphics_ring.write(data) {
            self.hypercall(KestrelHypercall::GraphicsUpdate);
            true
        } else {
            warn!("[Bridge] Graphics ring buffer full");
            false
        }
    }

    /// Transmit a raw network packet to the Windows network stack.
    pub fn send_net_packet(&mut self, packet: &[u8]) -> bool {
        if self.net_tx_ring.write(packet) {
            self.hypercall(KestrelHypercall::NetTx);
            true
        } else {
            warn!("[Bridge] Net TX ring buffer full");
            false
        }
    }

    /// Receive a raw network packet from the Windows network stack.
    pub fn recv_net_packet(&mut self, buf: &mut [u8]) -> Option<usize> {
        let n = self.net_rx_ring.read(buf)?;
        self.hypercall(KestrelHypercall::NetRxAck);
        Some(n)
    }

    /// Send a byte to the serial console (kestrel-term).
    pub fn serial_write(&mut self, data: &[u8]) -> bool {
        if self.serial_ring.write(data) {
            self.hypercall(KestrelHypercall::SerialTx);
            true
        } else {
            false
        }
    }

    /// Issue a Kestrel hypercall via I/O port.
    #[inline(always)]
    fn hypercall(&self, call: KestrelHypercall) {
        debug!("[Bridge] Hypercall {:?}", call);
        // In a real kernel module this uses outw(port, val)
        // Here we use a volatile write to indicate the call
        unsafe {
            // outw(KESTREL_IO_BASE, call as u16)
            // Represented as inline asm in a real kernel module:
            // core::arch::asm!("outw %ax, %dx", in("dx") KESTREL_IO_BASE, in("ax") call as u16);
            let _ = (KESTREL_IO_BASE, call as u16); // placeholder to avoid dead code
        }
    }
}
