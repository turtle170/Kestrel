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
pub fn run(boot_mode: crate::BootMode) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        info!("[WHPX] Creating hypervisor partition...");

        // Create partition — WHvCreatePartition() takes 0 args and returns Result<WHV_PARTITION_HANDLE>
        let partition: WHV_PARTITION_HANDLE =
            unsafe { WHvCreatePartition() }.context("Failed to create WHPX partition")?;

        info!("[WHPX] Setting up partition properties...");

        // Set processor count = 1
        let processor_count: u32 = 1;
        unsafe {
            WHvSetPartitionProperty(
                partition,
                WHvPartitionPropertyCodeProcessorCount,
                &processor_count as *const u32 as *const std::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
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
                0, // Guest physical memory starts at GPA 0
                crate::GUEST_MEMORY_SIZE as u64,
                WHV_MAP_GPA_RANGE_FLAGS(7),
            )
        }
        .context("Failed to map guest physical memory")?;

        let mut load_cpu_snapshot = None;

        match &boot_mode {
            crate::BootMode::Normal | crate::BootMode::SaveOnExit(_) => {
                // Load kernel image
                info!("[WHPX] Loading Kestrel kernel image...");
                crate::loader::load_kernel(host_mem, crate::GUEST_MEMORY_SIZE)?;
            }
            crate::BootMode::Snapshot(path) | crate::BootMode::SnapshotAndSave { load: path, .. } => {
                let (meta, snapshot_mem) = crate::snapshot::load_snapshot(path)?;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        snapshot_mem.as_ptr(),
                        host_mem as *mut u8,
                        snapshot_mem.len(),
                    );
                }
                load_cpu_snapshot = Some(meta.cpu);
            }
        }

        // Create virtual processor
        info!("[WHPX] Creating virtual processor 0...");
        unsafe { WHvCreateVirtualProcessor(partition, 0, 0) }
            .context("Failed to create virtual processor")?;

        if let Some(cpu) = load_cpu_snapshot {
            info!("[WHPX] Restoring CPU registers from snapshot...");
            set_vcpu_registers(partition, &cpu)?;
        } else {
            // Set initial CPU registers
            info!("[WHPX] Setting initial CPU registers...");
            setup_vcpu_registers(partition)?;
        }

        // Spawn terminal
        info!("[WHPX] Spawning Kestrel Terminal...");
        crate::ipc::spawn_terminal();

        // Run execution loop
        info!("[WHPX] Starting Kestrel execution loop...");
        run_loop(partition)?;

        match boot_mode {
            crate::BootMode::SaveOnExit(path) | crate::BootMode::SnapshotAndSave { save: path, .. } => {
                let cpu = get_vcpu_registers(partition)?;
                crate::snapshot::save_snapshot(&path, host_mem as *const u8, cpu, "whpx")?;
            }
            _ => {}
        }

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
    let reg_names = [
        WHvX64RegisterRip,
        WHvX64RegisterRflags,
        WHvX64RegisterRsp,
        WHvX64RegisterCs,
    ];
    let mut reg_values = [WHV_REGISTER_VALUE::default(); 4];
    unsafe {
        reg_values[0].Reg64 = 0x0010; // RIP = 16 (since CS base is 0xFFFF0, 0xFFFF0 + 16 = 0x100000 / 1MB)
        reg_values[1].Reg64 = 0x0002; // RFLAGS
        reg_values[2].Reg64 = (crate::GUEST_MEMORY_SIZE - 0x1000) as u64; // RSP

        // CS flat segment descriptor for Real Mode 1MB boot
        reg_values[3].Segment.Base = 0xFFFF0;
        reg_values[3].Segment.Limit = 0xFFFF;
        reg_values[3].Segment.Selector = 0xFFFF;
        reg_values[3].Segment.Anonymous.Attributes = 0x009B;

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
fn set_vcpu_registers(partition: WHV_PARTITION_HANDLE, cpu: &crate::snapshot::CpuSnapshot) -> Result<()> {
    let reg_names = [
        WHvX64RegisterRip,
        WHvX64RegisterRflags,
        WHvX64RegisterRsp,
        WHvX64RegisterRax,
        WHvX64RegisterRbx,
        WHvX64RegisterRcx,
        WHvX64RegisterRdx,
        WHvX64RegisterRsi,
        WHvX64RegisterRdi,
        WHvX64RegisterR8,
        WHvX64RegisterR9,
        WHvX64RegisterR10,
        WHvX64RegisterR11,
        WHvX64RegisterR12,
        WHvX64RegisterR13,
        WHvX64RegisterR14,
        WHvX64RegisterR15,
        WHvX64RegisterCr0,
        WHvX64RegisterCr3,
        WHvX64RegisterCr4,
        WHvX64RegisterEfer,
        WHvX64RegisterCs,
    ];
    let mut reg_values = [WHV_REGISTER_VALUE::default(); crate::snapshot::SAVED_REG_COUNT];

    unsafe {
        reg_values[0].Reg64 = cpu.rip;
        reg_values[1].Reg64 = cpu.rflags;
        reg_values[2].Reg64 = cpu.rsp;
        reg_values[3].Reg64 = cpu.rax;
        reg_values[4].Reg64 = cpu.rbx;
        reg_values[5].Reg64 = cpu.rcx;
        reg_values[6].Reg64 = cpu.rdx;
        reg_values[7].Reg64 = cpu.rsi;
        reg_values[8].Reg64 = cpu.rdi;
        reg_values[9].Reg64 = cpu.r8;
        reg_values[10].Reg64 = cpu.r9;
        reg_values[11].Reg64 = cpu.r10;
        reg_values[12].Reg64 = cpu.r11;
        reg_values[13].Reg64 = cpu.r12;
        reg_values[14].Reg64 = cpu.r13;
        reg_values[15].Reg64 = cpu.r14;
        reg_values[16].Reg64 = cpu.r15;
        reg_values[17].Reg64 = cpu.cr0;
        reg_values[18].Reg64 = cpu.cr3;
        reg_values[19].Reg64 = cpu.cr4;
        reg_values[20].Reg64 = cpu.efer;
        // Setting a segment register requires a full segment descriptor
        // For a snapshot, we do a basic load of the base
        reg_values[21].Segment.Base = cpu.cs_base;

        WHvSetVirtualProcessorRegisters(
            partition,
            0,
            reg_names.as_ptr(),
            reg_names.len() as u32,
            reg_values.as_ptr(),
        )
    }
    .context("Failed to set vCPU registers from snapshot")?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn get_vcpu_registers(partition: WHV_PARTITION_HANDLE) -> Result<crate::snapshot::CpuSnapshot> {
    let reg_names = [
        WHvX64RegisterRip,
        WHvX64RegisterRflags,
        WHvX64RegisterRsp,
        WHvX64RegisterRax,
        WHvX64RegisterRbx,
        WHvX64RegisterRcx,
        WHvX64RegisterRdx,
        WHvX64RegisterRsi,
        WHvX64RegisterRdi,
        WHvX64RegisterR8,
        WHvX64RegisterR9,
        WHvX64RegisterR10,
        WHvX64RegisterR11,
        WHvX64RegisterR12,
        WHvX64RegisterR13,
        WHvX64RegisterR14,
        WHvX64RegisterR15,
        WHvX64RegisterCr0,
        WHvX64RegisterCr3,
        WHvX64RegisterCr4,
        WHvX64RegisterEfer,
        WHvX64RegisterCs,
    ];
    let mut reg_values = [WHV_REGISTER_VALUE::default(); crate::snapshot::SAVED_REG_COUNT];

    unsafe {
        WHvGetVirtualProcessorRegisters(
            partition,
            0,
            reg_names.as_ptr(),
            reg_names.len() as u32,
            reg_values.as_mut_ptr(),
        ).context("Failed to get vCPU registers")?;
    }

    Ok(crate::snapshot::CpuSnapshot {
        rip: unsafe { reg_values[0].Reg64 },
        rflags: unsafe { reg_values[1].Reg64 },
        rsp: unsafe { reg_values[2].Reg64 },
        rax: unsafe { reg_values[3].Reg64 },
        rbx: unsafe { reg_values[4].Reg64 },
        rcx: unsafe { reg_values[5].Reg64 },
        rdx: unsafe { reg_values[6].Reg64 },
        rsi: unsafe { reg_values[7].Reg64 },
        rdi: unsafe { reg_values[8].Reg64 },
        r8:  unsafe { reg_values[9].Reg64 },
        r9:  unsafe { reg_values[10].Reg64 },
        r10: unsafe { reg_values[11].Reg64 },
        r11: unsafe { reg_values[12].Reg64 },
        r12: unsafe { reg_values[13].Reg64 },
        r13: unsafe { reg_values[14].Reg64 },
        r14: unsafe { reg_values[15].Reg64 },
        r15: unsafe { reg_values[16].Reg64 },
        cr0: unsafe { reg_values[17].Reg64 },
        cr3: unsafe { reg_values[18].Reg64 },
        cr4: unsafe { reg_values[19].Reg64 },
        efer: unsafe { reg_values[20].Reg64 },
        cs_base: unsafe { reg_values[21].Segment.Base },
    })
}

#[cfg(target_os = "windows")]
#[allow(non_upper_case_globals)]
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
            WHvRunVpExitReasonX64Halt => {
                info!("[WHPX] Guest halted (HLT). Exiting.");
                break;
            }
            WHvRunVpExitReasonX64IoPortAccess => {
                let io_context = unsafe { exit_context.Anonymous.IoPortAccess };
                if let Some(val) = crate::ipc::handle_io_port(io_context) {
                    let mut rax = get_register(partition, WHvX64RegisterRax)?;
                    rax = (rax & !0xFF) | (val as u64 & 0xFF);
                    set_register(partition, WHvX64RegisterRax, rax)?;
                }

                // Advance RIP to skip the emulated instruction
                let mut rip = get_register(partition, WHvX64RegisterRip)?;
                rip += (exit_context.VpContext._bitfield & 0x0F) as u64;
                set_register(partition, WHvX64RegisterRip, rip)?;
            }
            WHvRunVpExitReasonMemoryAccess => {
                let gpa = unsafe { exit_context.Anonymous.MemoryAccess.Gpa };
                warn!("[WHPX] Memory access exit — unhandled GPA: 0x{:016x}", gpa);
                bail!("WHPX guest crashed: unhandled GPA access");
            }
            WHvRunVpExitReasonCanceled => {
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

#[cfg(target_os = "windows")]
fn get_register(partition: WHV_PARTITION_HANDLE, name: WHV_REGISTER_NAME) -> Result<u64> {
    let mut value = WHV_REGISTER_VALUE::default();
    unsafe {
        WHvGetVirtualProcessorRegisters(
            partition,
            0,
            &name,
            1,
            &mut value,
        ).context("Failed to get virtual processor register")?;
        Ok(value.Reg64)
    }
}

#[cfg(target_os = "windows")]
fn set_register(partition: WHV_PARTITION_HANDLE, name: WHV_REGISTER_NAME, val: u64) -> Result<()> {
    let mut value = WHV_REGISTER_VALUE::default();
    value.Reg64 = val;
    unsafe {
        WHvSetVirtualProcessorRegisters(
            partition,
            0,
            &name,
            1,
            &value,
        ).context("Failed to set virtual processor register")?;
        Ok(())
    }
}
