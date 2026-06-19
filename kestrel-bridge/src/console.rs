//! Serial console driver for kestrel-term.

use crate::KestrelBridge;
use log::debug;
use core::fmt;

pub struct KestrelConsole<'a> {
    bridge: &'a mut KestrelBridge,
}

impl<'a> KestrelConsole<'a> {
    pub fn new(bridge: &'a mut KestrelBridge) -> Self {
        Self { bridge }
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
        debug!("[Console] TX {} bytes", data.len());
        self.bridge.serial_write(data);
    }
}

impl<'a> fmt::Write for KestrelConsole<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_bytes(s.as_bytes());
        Ok(())
    }
}
