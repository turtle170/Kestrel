//! WHPX (Windows Hypervisor Platform) execution backend.
//! Uses WHvGetCapability to detect support, then runs the Kestrel kernel
//! as a lightweight WHPX partition for near bare-metal performance.

use anyhow::{bail, Context, Result};
use log::{debug, info, warn};

#[cfg(target_os = "windows")]
use windows::Win32::System::Hypervisor::*;
#[cfg(target_os = "windows")]
use windows::Win32::System::Memory::*;

/// Check if WHPX is available on this machine.
pub fn is_whpx_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        // Try to query WHV capability: WHvCapabilityCodeHypervisorPresent = 0
        let mut capability = WHV_CAPABILITY::default();
        let result = unsafe {
            WHvGetCapability(
                WHV_CAPABILITY_CODE(0),
                &mut capability as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<WHV_CAPABILITY>() as u32,
                None,
            )
        };
        match result {
            Ok(_) => unsafe { capability.HypervisorPresent.as_bool() },
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    false
}

/// Run the Kestrel kernel under WHPX.
pub fn run() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        info!("[WHPX] Creating hypervisor partition...");

        // Create partition — WHvCreatePartition() takes 0 args and returns Result<WHV_PARTITION_HANDLE>
        let partition: WHV_PARTITION_HANDLE =
            unsafe { WHvCreatePartition() }.context("Failed to create WHPX partition")?;

        info!("[WHPX] Setting up partition properties...");

        // Set processor count = 1  (WHvPartitionPropertyCodeProcessorCount = 1)
        let mut prop = WHV_PARTITION_PROPERTY::default();
        unsafe {
            prop.ProcessorCount = 1;
            WHvSetPartitionProperty(
                partition,
                WHV_PARTITION_PROPERTY_CODE(1),
                &prop as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<WHV_PARTITION_PROPERTY>() as u32,
            )
        }
        .context("Failed to set processor count")?;

        info!("[WHPX] Setting up partition...");
        unsafe { WHvSetupPartition(partition) }.context("Failed to setup WHPX partition")?;

        // Allocate guest memory
        info!(
            "[WHPX] Allocating {}MB guest memory...",
            crate::GUEST_MEMORY_SIZE / 1024 / 1024
        );
        let host_mem = unsafe {
            VirtualAlloc(
                None,
                crate::GUEST_MEMORY_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        };
        if host_mem.is_null() {
            bail!("Failed to allocate guest physical memory");
        }

        // Map memory into guest (Read=1, Write=2, Execute=4 => flags=7)
        unsafe {
            WHvMapGpaRange(
                partition,
                host_mem,
                crate::KERNEL_LOAD_ADDR,
                crate::GUEST_MEMORY_SIZE as u64,
                WHV_MAP_GPA_RANGE_FLAGS(7),
            )
        }
        .context("Failed to map guest physical memory")?;

        // Load kernel image
        info!("[WHPX] Loading Kestrel kernel image...");
        crate::loader::load_kernel(host_mem, crate::GUEST_MEMORY_SIZE)?;

        // Create virtual processor
        info!("[WHPX] Creating virtual processor 0...");
        unsafe { WHvCreateVirtualProcessor(partition, 0, 0) }
            .context("Failed to create virtual processor")?;

        // Set initial CPU registers
        info!("[WHPX] Setting initial CPU registers...");
        setup_vcpu_registers(partition)?;

        // Spawn terminal
        info!("[WHPX] Spawning Kestrel Terminal...");
        crate::ipc::spawn_terminal();

        // Run execution loop
        info!("[WHPX] Starting Kestrel execution loop...");
        run_loop(partition)?;

        // Cleanup
        unsafe {
            let _ = WHvDeleteVirtualProcessor(partition, 0);
            let _ = WHvDeletePartition(partition);
        }
        info!("[WHPX] Partition shut down cleanly.");
    }
    #[cfg(not(target_os = "windows"))]
    {
        bail!("WHPX is only supported on Windows");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn setup_vcpu_registers(partition: WHV_PARTITION_HANDLE) -> Result<()> {
    // WHV_REGISTER_NAME numeric values (from WinHvPlatformDefs.h):
    //   Rip    = 0x00000040
    //   Rflags = 0x00000041
    //   Rsp    = 0x00000044
    // WHV_REGISTER_NAME inner type is i32 in windows-rs 0.62
    let reg_names = [
        WHV_REGISTER_NAME(0x00000040i32), // Rip
        WHV_REGISTER_NAME(0x00000041i32), // Rflags
        WHV_REGISTER_NAME(0x00000044i32), // Rsp
    ];
    let mut reg_values = [WHV_REGISTER_VALUE::default(); 3];
    unsafe {
        reg_values[0].Reg64 = crate::KERNEL_LOAD_ADDR; // RIP = kernel load address
        reg_values[1].Reg64 = 0x0002; // RFLAGS: reserved bit 1 set (always required)
        reg_values[2].Reg64 = (crate::GUEST_MEMORY_SIZE - 0x1000) as u64; // RSP

        WHvSetVirtualProcessorRegisters(
            partition,
            0,
            reg_names.as_ptr(),
            reg_names.len() as u32,
            reg_values.as_ptr(),
        )
    }
    .context("Failed to set vCPU registers")?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_loop(partition: WHV_PARTITION_HANDLE) -> Result<()> {
    let mut exit_context = WHV_RUN_VP_EXIT_CONTEXT::default();

    loop {
        unsafe {
            WHvRunVirtualProcessor(
                partition,
                0,
                &mut exit_context as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<WHV_RUN_VP_EXIT_CONTEXT>() as u32,
            )
        }
        .context("WHvRunVirtualProcessor failed")?;

        match exit_context.ExitReason {
            // WHvRunVpExitReasonX64Halt = 0x00000000
            WHV_RUN_VP_EXIT_REASON(0x00000000) => {
                info!("[WHPX] Guest halted (HLT). Exiting.");
                break;
            }
            // WHvRunVpExitReasonX64IoPortAccess = 0x00001000
            WHV_RUN_VP_EXIT_REASON(0x00001000) => {
                // IoPortAccess lives in the Anonymous union field
                crate::ipc::handle_io_port(unsafe { exit_context.Anonymous.IoPortAccess });
            }
            // WHvRunVpExitReasonMemoryAccess = 0x00000001
            WHV_RUN_VP_EXIT_REASON(0x00000001) => {
                warn!("[WHPX] Memory access exit — unhandled GPA");
            }
            // WHvRunVpExitReasonCanceled = 0x00002001
            WHV_RUN_VP_EXIT_REASON(0x00002001) => {
                info!("[WHPX] Execution cancelled. Exiting.");
                break;
            }
            reason => {
                debug!("[WHPX] Unhandled exit reason: 0x{:08x}", reason.0);
            }
        }
    }
    Ok(())
}
