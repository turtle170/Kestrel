//! kestrel: Package manager and Beak FS virtual disk tool for Kestrel OS.
//!
//! Provides CLI utilities for packaging .kstl archives and formatting/managing
//! custom Beak FS sparse (.xshd) virtual hard disks.

mod kstl;
mod convert;

use anyhow::{Result, Context};
use clap::{Parser, Subcommand};
use log::info;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "kestrel",
    about = "Kestrel OS Package Manager & Beak FS Disk Tool",
    version = "0.1.0"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // ─── Package Commands ────────────────────────────────────────────────────
    
    /// Pack a directory into a .kstl file
    Pack {
        /// Source directory to pack
        #[arg(short, long)]
        source: PathBuf,
        /// Output .kstl file path
        #[arg(short, long)]
        output: PathBuf,
        /// Entry point path inside the package (e.g. /usr/bin/myapp)
        #[arg(short, long)]
        entry: String,
        /// Compression algorithm: xz or zstd
        #[arg(short, long, default_value = "zstd")]
        compression: String,
    },
    /// Unpack a .kstl file to a directory
    Unpack {
        /// Input .kstl file
        #[arg(short, long)]
        input: PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Show metadata of a .kstl file
    Info {
        /// Input .kstl file
        #[arg(short, long)]
        input: PathBuf,
    },
    /// Convert a foreign package to .kstl
    Convert {
        /// Input package file (.deb, .rpm, .pkg.tar.zst, .flatpak, .snap, .AppImage)
        #[arg(short, long)]
        input: PathBuf,
        /// Output .kstl file path
        #[arg(short, long)]
        output: PathBuf,
    },

    // ─── Disk Commands (Beak FS / .xshd) ─────────────────────────────────────
    
    /// Format a sparse virtual disk (.xshd) with Beak FS
    FormatDisk {
        /// Path to the virtual disk file
        #[arg(short, long)]
        disk: PathBuf,
        /// Total disk size in megabytes (MB)
        #[arg(short, long, default_value = "10")]
        size_mb: u64,
    },
    /// List contents of a directory on a Beak FS virtual disk
    DiskLs {
        /// Path to the virtual disk file
        #[arg(short, long)]
        disk: PathBuf,
        /// Path inside the virtual disk to list
        #[arg(short, long, default_value = "/")]
        path: String,
    },
    /// Create a new directory on a Beak FS virtual disk
    DiskMkdir {
        /// Path to the virtual disk file
        #[arg(short, long)]
        disk: PathBuf,
        /// Directory path to create (e.g. /data)
        #[arg(short, long)]
        path: String,
    },
    /// Copy a file from the host into a Beak FS virtual disk
    DiskAdd {
        /// Path to the virtual disk file
        #[arg(short, long)]
        disk: PathBuf,
        /// Path to the host file to add
        #[arg(short, long)]
        src: PathBuf,
        /// Path inside the virtual disk to save to (e.g. /config.json)
        #[arg(long)]
        dest: String,
    },
    /// Print contents of a file on a Beak FS virtual disk
    DiskCat {
        /// Path to the virtual disk file
        #[arg(short, long)]
        disk: PathBuf,
        /// File path inside the virtual disk to display
        #[arg(short, long)]
        path: String,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cli = Cli::parse();

    match cli.command {
        // ─── Package Commands ────────────────────────────────────────────────
        Commands::Pack { source, output, entry, compression } => {
            info!("Packing {:?} -> {:?} (entry: {})", source, output, entry);
            kstl::pack(&source, &output, &entry, &compression)?;
            println!("✔  Packed successfully: {:?}", output);
        }
        Commands::Unpack { input, output } => {
            info!("Unpacking {:?} -> {:?}", input, output);
            kstl::unpack(&input, &output)?;
            println!("✔  Unpacked successfully: {:?}", output);
        }
        Commands::Info { input } => {
            let meta = kstl::read_metadata(&input)?;
            println!("=== Kestrel Package Info ===");
            println!("  Name:       {}", meta.name);
            println!("  Version:    {}", meta.version);
            println!("  Entry:      {}", meta.entry_point);
            println!("  Arch:       {}", meta.architecture);
            println!("  Compression:{}", meta.compression);
            println!("  Size:       {} bytes", meta.payload_size);
        }
        Commands::Convert { input, output } => {
            info!("Converting {:?} -> {:?}", input, output);
            convert::convert_to_kstl(&input, &output)?;
            println!("✔  Converted successfully: {:?}", output);
        }

        // ─── Disk Commands (Beak FS / .xshd) ─────────────────────────────────
        Commands::FormatDisk { disk, size_mb } => {
            info!("Formatting virtual disk {:?} ({} MB)...", disk, size_mb);
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&disk)
                .context("Failed to open/create virtual disk file")?;

            beak_fs::BeakFs::format(file, size_mb * 1024 * 1024)?;
            println!("✔  Formatted successfully: {:?}", disk);
        }
        Commands::DiskLs { disk, path } => {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&disk)
                .context("Failed to open virtual disk")?;

            let mut fs = beak_fs::BeakFs::open(file)?;
            let inode = fs.resolve_path(&path)?;
            let entries = fs.list_dir(inode)?;

            println!("Contents of beak://{:?}:", path);
            for entry in entries {
                let type_str = if entry.file_type == 2 { "<DIR>" } else { "     " };
                println!("  {}   {}", type_str, entry.name);
            }
        }
        Commands::DiskMkdir { disk, path } => {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&disk)
                .context("Failed to open virtual disk")?;

            let mut fs = beak_fs::BeakFs::open(file)?;
            let (parent, name) = split_beak_path(&path)?;
            
            let parent_inode = fs.resolve_path(&parent)?;
            fs.create_file(parent_inode, &name, true)?;
            println!("✔  Directory created successfully: {:?}", path);
        }
        Commands::DiskAdd { disk, src, dest } => {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&disk)
                .context("Failed to open virtual disk")?;

            let mut fs = beak_fs::BeakFs::open(file)?;
            let data = fs::read(&src).context("Failed to read host file")?;
            
            let (parent, name) = split_beak_path(&dest)?;
            let parent_inode = fs.resolve_path(&parent)?;
            
            let file_inode = fs.create_file(parent_inode, &name, false)?;
            fs.write_file_data(file_inode, &data)?;
            println!("✔  Added file successfully: {:?} -> {:?}", src, dest);
        }
        Commands::DiskCat { disk, path } => {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&disk)
                .context("Failed to open virtual disk")?;

            let mut fs = beak_fs::BeakFs::open(file)?;
            let inode = fs.resolve_path(&path)?;
            let data = fs.read_file_data(inode)?;
            
            std::io::Write::write_all(&mut std::io::stdout(), &data)?;
        }
    }

    Ok(())
}

/// Split a Beak FS path into (parent_path, component_name)
fn split_beak_path(path: &str) -> Result<(String, String)> {
    let p = Path::new(path);
    let parent = p.parent()
        .map(|dir| dir.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    
    let parent = if parent.is_empty() { "/".to_string() } else { parent };
    
    let name = p.file_name()
        .context("Invalid target file name")?
        .to_string_lossy()
        .to_string();

    Ok((parent, name))
}
