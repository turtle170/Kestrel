//! kestrel-pkg: Package manager for the Kestrel OS .kstl format.
//!
//! .kstl format specification:
//! [4 bytes]  Magic: b"KSTL"
//! [4 bytes]  Metadata length (u32 LE)
//! [N bytes]  Metadata JSON (UTF-8)
//! [rest]     SquashFS block (XZ or ZSTD compressed)

mod kstl;
mod convert;

use anyhow::Result;
use clap::{Parser, Subcommand};
use log::info;

#[derive(Parser)]
#[command(
    name = "kestrel-pkg",
    about = "Kestrel OS Package Manager -- Pack, unpack and convert application packages to .kstl",
    version = "0.1.0"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Pack a directory into a .kstl file
    Pack {
        /// Source directory to pack
        #[arg(short, long)]
        source: std::path::PathBuf,
        /// Output .kstl file path
        #[arg(short, long)]
        output: std::path::PathBuf,
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
        input: std::path::PathBuf,
        /// Output directory
        #[arg(short, long)]
        output: std::path::PathBuf,
    },
    /// Show metadata of a .kstl file
    Info {
        /// Input .kstl file
        #[arg(short, long)]
        input: std::path::PathBuf,
    },
    /// Convert a foreign package to .kstl
    Convert {
        /// Input package file (.deb, .rpm, .pkg.tar.zst, .flatpak, .snap, .AppImage)
        #[arg(short, long)]
        input: std::path::PathBuf,
        /// Output .kstl file path
        #[arg(short, long)]
        output: std::path::PathBuf,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Pack { source, output, entry, compression } => {
            info!("Packing {:?} -> {:?} (entry: {})", source, output, entry);
            kstl::pack(&source, &output, &entry, &compression)?;
            println!("?  Packed successfully: {:?}", output);
        }
        Commands::Unpack { input, output } => {
            info!("Unpacking {:?} -> {:?}", input, output);
            kstl::unpack(&input, &output)?;
            println!("?  Unpacked successfully: {:?}", output);
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
            println!("?  Converted successfully: {:?}", output);
        }
    }

    Ok(())
}
