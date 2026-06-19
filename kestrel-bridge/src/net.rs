//! Virtual network driver stub for Kestrel Bridge.

use crate::KestrelBridge;
use log::debug;

pub struct KestrelNetDriver<'a> {
    bridge: &'a mut KestrelBridge,
}

impl<'a> KestrelNetDriver<'a> {
    pub fn new(bridge: &'a mut KestrelBridge) -> Self {
        Self { bridge }
    }

    /// Send a raw Ethernet frame to the Windows host network stack.
    pub fn transmit(&mut self, frame: &[u8]) -> bool {
        debug!("[NetDriver] TX {} bytes", frame.len());
        self.bridge.send_net_packet(frame)
    }

    /// Receive a raw Ethernet frame from the Windows host.
    pub fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        let n = self.bridge.recv_net_packet(buf)?;
        debug!("[NetDriver] RX {} bytes", n);
        Some(n)
    }
}
