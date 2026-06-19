//! binfmt handler for Kestrel: detects and loads Windows PE binaries.

use log::info;
use crate::pe::PeLoader;

pub struct KestrelBinfmt;

impl KestrelBinfmt {
    /// Check if this binary should be handled by Reverse-Proton.
    pub fn can_handle(data: &[u8]) -> bool {
        PeLoader::is_pe(data)
    }

    /// Load a Windows .exe into Kestrel guest memory and return the entry point.
    pub fn load_and_exec(
        file_data: &[u8],
        guest_mem: &mut [u8],
        load_base: usize,
    ) -> Result<usize, &'static str> {
        let pe = PeLoader::parse(file_data)?;
        let entry = PeLoader::map_into_memory(&pe, file_data, guest_mem, load_base)?;
        info!("[binfmt] Windows PE loaded at 0x{:x}, entry 0x{:x}", load_base, entry);
        Ok(entry)
    }
}
