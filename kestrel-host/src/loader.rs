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
        let offset = crate::KERNEL_LOAD_ADDR as usize;
        if offset >= guest_mem_size {
            bail!("KERNEL_LOAD_ADDR exceeds guest memory size");
        }
        
        let mut stub = Vec::new();
        // mov dx, 0x3F8
        stub.extend_from_slice(&[0xBA, 0xF8, 0x03]);
        
        let msg = b"Kestrel OS Stub Mode (No Kernel Image). Type anything to echo. Press Ctrl+Q to exit.\r\n";
        for &byte in msg {
            stub.extend_from_slice(&[0xB0, byte, 0xEE]);
        }
        
        // Print initial prompt
        let initial_prompt = b"[kestrel-stub /]> ";
        for &byte in initial_prompt {
            stub.extend_from_slice(&[0xB0, byte, 0xEE]);
        }

        let wait_rx_offset = stub.len();
        
        // mov dx, 0x3FD
        stub.extend_from_slice(&[0xBA, 0xFD, 0x03]);
        // in al, dx
        stub.push(0xEC);
        // test al, 0x01
        stub.extend_from_slice(&[0xA8, 0x01]);
        
        // jz wait_rx
        let jz_instr_offset = stub.len();
        let target_offset = wait_rx_offset as isize - (jz_instr_offset as isize + 2);
        stub.extend_from_slice(&[0x74, target_offset as u8]);

        // mov dx, 0x3F8
        stub.extend_from_slice(&[0xBA, 0xF8, 0x03]);
        // in al, dx
        stub.push(0xEC);
        
        // cmp al, 13 (\r)
        stub.extend_from_slice(&[0x3C, 13]);
        
        // je handle_enter (0x74, rel8)
        let je_instr_offset = stub.len();
        stub.extend_from_slice(&[0x74, 0x00]); // Placeholder

        // Echo path (characters other than \r)
        // out dx, al
        stub.push(0xEE);
        // jmp wait_rx
        let jmp_wait_rx_offset = stub.len();
        let target_offset = wait_rx_offset as isize - (jmp_wait_rx_offset as isize + 2);
        stub.extend_from_slice(&[0xEB, target_offset as u8]);

        // Label: handle_enter
        let handle_enter_offset = stub.len();
        let je_target = handle_enter_offset as isize - (je_instr_offset as isize + 2);
        stub[je_instr_offset + 1] = je_target as u8;

        // handle_enter code:
        // out dx, al (echo '\r')
        stub.push(0xEE);
        // mov al, 10 (\n)
        stub.extend_from_slice(&[0xB0, 10]);
        // out dx, al
        stub.push(0xEE);
        
        // print prompt
        for &byte in initial_prompt {
            stub.extend_from_slice(&[0xB0, byte, 0xEE]);
        }
        
        // jmp wait_rx
        let final_jmp_offset = stub.len();
        let target_offset = wait_rx_offset as isize - (final_jmp_offset as isize + 2);
        stub.extend_from_slice(&[0xEB, target_offset as u8]);
        
        if offset + stub.len() > guest_mem_size {
            bail!("Loader stub exceeds guest memory size");
        }
        
        unsafe {
            let dest = (guest_mem as *mut u8).add(offset);
            std::ptr::copy_nonoverlapping(stub.as_ptr(), dest, stub.len());
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
