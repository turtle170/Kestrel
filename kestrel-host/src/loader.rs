//! Kestrel kernel image loader.
//! Reads the bzImage/ELF kernel binary and copies it into guest memory.
//! Returns the entry-point offset within guest memory.

use anyhow::{bail, Context, Result};
use log::{info, warn};
use std::fs;

/// Load the Kestrel kernel into guest memory.
///
/// - `guest_mem`      : pointer to the host-side allocation that represents guest RAM.
/// - `guest_mem_size` : total size of that allocation in bytes.
///
/// Returns the **byte offset** within `guest_mem` at which execution should begin.
pub fn load_kernel(guest_mem: *mut std::ffi::c_void, guest_mem_size: usize) -> Result<usize> {
    let kernel_path = std::env::current_exe()
        .context("Cannot determine executable path")?
        .parent()
        .map(|p| p.join(crate::KESTREL_KERNEL_PATH))
        .context("Cannot determine executable directory")?;

    if !kernel_path.exists() {
        warn!(
            "[Loader] Kernel image not found at {:?} — running in stub mode",
            kernel_path
        );
        // Write a single HLT (0xF4) at KERNEL_LOAD_ADDR so the VM exits cleanly.
        let offset = crate::KERNEL_LOAD_ADDR as usize;
        if offset >= guest_mem_size {
            bail!("KERNEL_LOAD_ADDR exceeds guest memory size");
        }
        unsafe {
            let entry = (guest_mem as *mut u8).add(offset);
            *entry = 0xF4; // HLT opcode
        }
        return Ok(offset);
    }

    let kernel_data = fs::read(&kernel_path)
        .with_context(|| format!("Failed to read kernel image: {:?}", kernel_path))?;

    info!(
        "[Loader] Kernel image: {:?} ({} KB)",
        kernel_path,
        kernel_data.len() / 1024
    );

    let load_offset = crate::KERNEL_LOAD_ADDR as usize;
    let available = guest_mem_size
        .checked_sub(load_offset)
        .context("KERNEL_LOAD_ADDR exceeds guest memory size")?;

    if kernel_data.len() > available {
        bail!(
            "Kernel image ({} bytes) too large for available guest memory ({} bytes)",
            kernel_data.len(),
            available
        );
    }

    // Copy kernel into guest memory at KERNEL_LOAD_ADDR
    unsafe {
        let dest = (guest_mem as *mut u8).add(load_offset);
        std::ptr::copy_nonoverlapping(kernel_data.as_ptr(), dest, kernel_data.len());
    }

    info!(
        "[Loader] Kernel loaded at guest physical 0x{:08x} ({} KB)",
        crate::KERNEL_LOAD_ADDR,
        kernel_data.len() / 1024
    );

    Ok(load_offset)
}
