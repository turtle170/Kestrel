//! User-Mode Linux (UML) execution backend.
//! Allocates guest memory via VirtualAlloc, loads the kernel ELF/bzImage,
//! and uses Windows Vectored Exception Handling (VEH) to intercept
//! hardware-level faults and emulate them in software.

use anyhow::{bail, Result};
use log::{debug, info, warn};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::*;
#[cfg(target_os = "windows")]
use windows::Win32::System::Diagnostics::Debug::*;
#[cfg(target_os = "windows")]
use windows::Win32::System::Memory::*;

/// Vectored Exception Handler for UML mode.
/// Catches privileged instruction faults and emulates them.
#[cfg(target_os = "windows")]
unsafe extern "system" fn veh_handler(exception_info: *mut EXCEPTION_POINTERS) -> i32 {
    const EXCEPTION_CONTINUE_EXECUTION: i32 = 0;
    const EXCEPTION_CONTINUE_SEARCH: i32 = -1;

    if exception_info.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let record = &*(*exception_info).ExceptionRecord;
    let code = record.ExceptionCode;

    if code == EXCEPTION_PRIV_INSTRUCTION {
        // Emulate privileged instruction (HLT, WRMSR, RDMSR, etc.)
        // For a single-byte privileged instruction, advance RIP by 1.
        debug!(
            "[UML] Privileged instruction at {:?}",
            record.ExceptionAddress
        );
        let ctx = &mut *(*exception_info).ContextRecord;
        ctx.Rip += 1;
        EXCEPTION_CONTINUE_EXECUTION
    } else if code == EXCEPTION_ACCESS_VIOLATION {
        // ExceptionInformation[0] = 0 (read) / 1 (write) / 8 (DEP)
        // ExceptionInformation[1] = faulting address
        let fault_addr = record.ExceptionInformation[1] as u64;
        warn!("[UML] Access violation at GPA 0x{:016x}", fault_addr);
        crate::ipc::handle_mmio_access(fault_addr);
        EXCEPTION_CONTINUE_EXECUTION
    } else {
        EXCEPTION_CONTINUE_SEARCH
    }
}

/// Run the Kestrel kernel under UML mode.
pub fn run() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        info!(
            "[UML] Allocating {}MB guest memory...",
            crate::GUEST_MEMORY_SIZE / 1024 / 1024
        );

        // Allocate contiguous executable guest memory
        let host_mem = unsafe {
            VirtualAlloc(
                None,
                crate::GUEST_MEMORY_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        };
        if host_mem.is_null() {
            bail!("Failed to allocate UML guest memory");
        }
        info!("[UML] Guest memory allocated at {:?}", host_mem);

        // Register Vectored Exception Handler (first = highest priority)
        info!("[UML] Registering Vectored Exception Handler...");
        let veh_handle = unsafe { AddVectoredExceptionHandler(1, Some(veh_handler)) };
        if veh_handle.is_null() {
            bail!("Failed to register VEH handler");
        }

        // Load kernel image into guest memory
        info!("[UML] Loading Kestrel kernel image...");
        let entry_offset = crate::loader::load_kernel(host_mem, crate::GUEST_MEMORY_SIZE)?;

        // Spawn terminal before jumping to kernel
        info!("[UML] Spawning Kestrel Terminal...");
        crate::ipc::spawn_terminal();

        // Jump to kernel entry point — this only returns if the kernel is a stub (HLT)
        info!(
            "[UML] Jumping to kernel entry at host address 0x{:x}...",
            host_mem as usize + entry_offset
        );
        unsafe {
            let entry_fn: unsafe extern "C" fn() =
                std::mem::transmute((host_mem as usize + entry_offset) as *const ());
            entry_fn();
        }

        // Remove VEH on graceful return (stub mode HLT is caught and we return here)
        unsafe {
            let _ = RemoveVectoredExceptionHandler(veh_handle);
        }
        info!("[UML] Kernel returned. UML session complete.");
    }
    #[cfg(not(target_os = "windows"))]
    {
        bail!("UML host is only supported on Windows");
    }
    Ok(())
}
