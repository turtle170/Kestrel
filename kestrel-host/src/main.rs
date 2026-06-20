mod whpx;
mod uml;
mod loader;
mod ipc;
pub mod snapshot;

use anyhow::Result;
use clap::Parser;
use log::{info, warn};
use std::path::PathBuf;

pub const GUEST_MEMORY_SIZE: usize = 1024 * 1024 * 1024; // 1 GB
pub const KERNEL_LOAD_ADDR: u64 = 0x100000; // 1 MB physical
pub const KESTREL_KERNEL_PATH: &str = "kestrel-kernel.bzImage";

/// Boot mode passed to the execution backends.
#[derive(Debug, Clone)]
pub enum BootMode {
    /// Fresh kernel boot (default).
    Normal,
    /// Restore from a .kstl snapshot file.
    Snapshot(PathBuf),
    /// Save state to a .kstl snapshot on graceful shutdown.
    SaveOnExit(PathBuf),
    /// Restore snapshot AND save a new one on exit.
    SnapshotAndSave { load: PathBuf, save: PathBuf },
}

#[derive(Parser, Debug)]
#[command(
    name = "kestrel-host",
    about = "Kestrel OS Host — boots a minimal Linux kernel on Windows",
    version = "0.1.0"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Resume from a .kstl snapshot file instead of a cold kernel boot.
    /// The file must have been created by --save-state.
    #[arg(long, value_name = "PATH.kstl")]
    state: Option<PathBuf>,

    /// Save machine state to a .kstl snapshot file on graceful shutdown.
    #[arg(long, value_name = "PATH.kstl")]
    save_state: Option<PathBuf>,

    /// Run the host as an orchestration daemon only (no VM boot).
    #[arg(long)]
    daemon: bool,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Spawn a new Kestrel Hatchling (container) in the currently running VM
    Hatch {
        /// The .kstl package to spawn
        #[arg(value_name = "APP.kstl")]
        app: PathBuf,
        /// Windows-to-Linux named volumes (e.g. C:\WinData:/lin_data)
        #[arg(short, long, value_name = "WIN:LIN")]
        volume: Vec<String>,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    info!("╔══════════════════════════════════╗");
    info!("║       Kestrel OS Host v0.1       ║");
    info!("╚══════════════════════════════════╝");
    info!("[Host] Configured guest memory limit: {}MB", GUEST_MEMORY_SIZE / 1024 / 1024);

    let args = Args::parse();

    if let Some(Commands::Hatch { app, volume }) = args.command {
        // We are in Hatch mode. Connect to the running Kestrel VM via IPC.
        info!("Sending Hatchling spawn request for {:?}...", app);
        crate::ipc::send_hatch_request(app, volume)?;
        info!("Hatchling spawned successfully.");
        return Ok(());
    }

    // Start background IPC control listener for Hatchling commands
    crate::ipc::start_control_listener();

    if args.daemon {
        info!("[Host] Running in orchestration daemon mode. Press Ctrl+C to exit.");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // Determine boot mode from flags.
    let boot_mode = match (args.state, args.save_state) {
        (Some(load), Some(save)) => {
            info!("[Boot] Snapshot restore mode: {:?}", load);
            info!("[Boot] Will save new snapshot on exit: {:?}", save);
            BootMode::SnapshotAndSave { load, save }
        }
        (Some(load), None) => {
            info!("[Boot] Snapshot restore mode: {:?}", load);
            BootMode::Snapshot(load)
        }
        (None, Some(save)) => {
            info!("[Boot] Normal boot — state will be saved on exit to {:?}", save);
            BootMode::SaveOnExit(save)
        }
        (None, None) => {
            info!("[Boot] Normal kernel boot");
            BootMode::Normal
        }
    };

    // Detect execution backend.
    let use_whpx = whpx::is_whpx_available();

    if use_whpx {
        info!("[WHPX] Windows Hypervisor Platform detected — using 98% bare-metal mode");
        whpx::run(boot_mode)?;
    } else {
        warn!("[UML] WHPX not available — falling back to User-Mode Linux emulation");
        uml::run(boot_mode)?;
    }

    Ok(())
}
