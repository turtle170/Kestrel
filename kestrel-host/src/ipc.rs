//! IPC bridge between Kestrel Linux and the Windows host.
//! Handles I/O port interception from WHPX, MMIO fault handling from UML,
//! and terminal process spawning.

use log::{debug, info, warn};
use std::process::Command;

#[cfg(target_os = "windows")]
use windows::Win32::System::Hypervisor::WHV_X64_IO_PORT_ACCESS_CONTEXT;

// ─── Terminal ────────────────────────────────────────────────────────────────

/// Attempt to launch `kestrel-term.exe` from the same directory as this executable.
pub fn spawn_terminal() {
    let result = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("kestrel-term.exe")));

    match result {
        Some(path) if path.exists() => {
            info!("[IPC] Spawning Kestrel Terminal: {:?}", path);
            match Command::new(&path).spawn() {
                Ok(_) => info!("[IPC] kestrel-term.exe launched successfully"),
                Err(e) => warn!("[IPC] Failed to launch kestrel-term.exe: {}", e),
            }
        }
        Some(path) => {
            warn!(
                "[IPC] kestrel-term.exe not found at {:?} — terminal will not auto-open",
                path
            );
        }
        None => {
            warn!("[IPC] Could not determine executable path for terminal launch");
        }
    }
}

// ─── WHPX I/O Port Handler ───────────────────────────────────────────────────

/// Handle an I/O port access exit delivered by the WHPX virtual processor.
/// In a full implementation this would bridge COM1 to a named pipe read by kestrel-term.
#[cfg(target_os = "windows")]
pub fn handle_io_port(ctx: WHV_X64_IO_PORT_ACCESS_CONTEXT) {
    let port = ctx.PortNumber;
    debug!("[IPC] I/O port access: 0x{:04x}", port);

    match port {
        // COM1 UART data register — route serial output to Kestrel terminal pipe
        0x3F8 => {
            // TODO: write ctx.Rax low byte to \\.\pipe\kestrel-serial
            debug!("[IPC] COM1 TX byte (stub)");
        }
        // ISA debug port — safe to ignore
        0x0080 => {}
        // COM2 data register
        0x2F8 => {
            debug!("[IPC] COM2 TX byte (stub)");
        }
        _ => {
            debug!("[IPC] Unhandled I/O port 0x{:04x}", port);
        }
    }
}

// Non-Windows stub so the module compiles on all targets
#[cfg(not(target_os = "windows"))]
pub fn handle_io_port(_ctx: ()) {}

// ─── UML MMIO Handler ────────────────────────────────────────────────────────

/// Handle an MMIO access fault intercepted by the UML VEH.
/// `gpa` is the guest physical address that triggered the fault.
pub fn handle_mmio_access(gpa: u64) {
    debug!("[IPC] MMIO access at GPA 0x{:016x}", gpa);

    match gpa {
        // Legacy VGA text/graphics framebuffer
        0x000A_0000..=0x000B_FFFF => {
            debug!("[IPC] VGA framebuffer write at 0x{:016x} — forwarding to Windows GDI (stub)", gpa);
            // TODO: translate VGA writes to Win32 GDI / DirectX surface updates
        }
        // VGA BIOS ROM
        0x000C_0000..=0x000C_7FFF => {
            debug!("[IPC] VGA BIOS ROM access (read-only, ignored)");
        }
        // ACPI MMIO range (placeholder)
        0xFEC0_0000..=0xFECF_FFFF => {
            debug!("[IPC] ACPI MMIO at 0x{:016x} (stub)", gpa);
        }
        // Local APIC (xAPIC default base)
        0xFEE0_0000..=0xFEEF_FFFF => {
            debug!("[IPC] Local APIC MMIO at 0x{:016x} (stub)", gpa);
        }
        _ => {
            warn!("[IPC] Unhandled MMIO at GPA 0x{:016x}", gpa);
        }
    }
}
