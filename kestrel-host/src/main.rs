mod whpx;
mod uml;
mod loader;
mod ipc;

use anyhow::Result;
use log::{info, warn};

pub const GUEST_MEMORY_SIZE: usize = 256 * 1024 * 1024; // 256 MB
pub const KERNEL_LOAD_ADDR: u64 = 0x100000; // 1MB physical
pub const KESTREL_KERNEL_PATH: &str = "kestrel-kernel.bzImage";

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    info!("╔══════════════════════════════════╗");
    info!("║       Kestrel OS Host v0.1       ║");
    info!("╚══════════════════════════════════╝");

    // Detect execution mode
    let use_whpx = whpx::is_whpx_available();

    if use_whpx {
        info!("[WHPX] Windows Hypervisor Platform detected — using 98% bare-metal mode");
        whpx::run()?;
    } else {
        warn!("[UML] WHPX not available — falling back to User-Mode Linux emulation");
        uml::run()?;
    }

    Ok(())
}
