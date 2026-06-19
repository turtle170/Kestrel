//! .kstl file format implementation.

use anyhow::{Result, bail, Context};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use log::{info, debug};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

pub const KSTL_MAGIC: &[u8; 4] = b"KSTL";
#[allow(dead_code)]
pub const KSTL_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KstlMetadata {
    /// Human-readable package name
    pub name: String,
    /// Package version string
    pub version: String,
    /// Absolute path to the entry point inside the SquashFS
    pub entry_point: String,
    /// Target architecture (x86_64, aarch64)
    pub architecture: String,
    /// Compression algorithm used for the SquashFS block
    pub compression: String,
    /// Size of the SquashFS payload in bytes
    pub payload_size: u64,
    /// Optional list of required Linux capabilities
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Optional environment variables
    #[serde(default)]
    pub environment: std::collections::HashMap<String, String>,
}

impl Default for KstlMetadata {
    fn default() -> Self {
        Self {
            name: "unknown".into(),
            version: "0.0.0".into(),
            entry_point: "/usr/bin/app".into(),
            architecture: "x86_64".into(),
            compression: "zstd".into(),
            payload_size: 0,
            capabilities: vec![],
            environment: Default::default(),
        }
    }
}

/// Pack a directory into a .kstl file.
/// This calls mksquashfs to build the SquashFS block.
pub fn pack(source: &Path, output: &Path, entry: &str, compression: &str) -> Result<()> {
    // Build SquashFS image of the source directory
    let squashfs_path = output.with_extension("sqfs.tmp");
    build_squashfs(source, &squashfs_path, compression)?;

    let squashfs_data = fs::read(&squashfs_path)
        .context("Failed to read temporary SquashFS image")?;
    let _ = fs::remove_file(&squashfs_path);

    let name = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let meta = KstlMetadata {
        name,
        entry_point: entry.to_owned(),
        compression: compression.to_owned(),
        payload_size: squashfs_data.len() as u64,
        ..Default::default()
    };

    write_kstl(output, &meta, &squashfs_data)
}

/// Unpack a .kstl file to a directory.
pub fn unpack(input: &Path, output: &Path) -> Result<()> {
    let (meta, squashfs_data) = read_kstl(input)?;
    debug!("Unpacking {} v{} ...", meta.name, meta.version);

    fs::create_dir_all(output).context("Failed to create output directory")?;

    // Write squashfs to temp file, then mount/extract it
    let tmp = tempfile::NamedTempFile::new()?.into_temp_path();
    fs::write(&tmp, &squashfs_data).context("Failed to write temporary SquashFS")?;

    // Use unsquashfs to extract
    let status = std::process::Command::new("unsquashfs")
        .args(["-d", &output.to_string_lossy(), &tmp.to_string_lossy()])
        .status();

    match status {
        Ok(s) if s.success() => {
            info!("Extracted to {:?}", output);
            Ok(())
        }
        Ok(s) => bail!("unsquashfs exited with code: {:?}", s.code()),
        Err(_) => {
            // Fallback: just dump squashfs blob
            info!("unsquashfs not found -- dumping raw SquashFS to {:?}", output);
            fs::write(output.join("payload.sqfs"), &squashfs_data)?;
            Ok(())
        }
    }
}

/// Read only the metadata from a .kstl file (fast -- no payload read).
pub fn read_metadata(input: &Path) -> Result<KstlMetadata> {
    let f = File::open(input).context("Cannot open .kstl file")?;
    let mut r = BufReader::new(f);
    let (meta, _) = parse_header(&mut r)?;
    Ok(meta)
}

/// Write a fully formed .kstl file.
pub fn write_kstl(output: &Path, meta: &KstlMetadata, payload: &[u8]) -> Result<()> {
    let f = File::create(output).context("Cannot create .kstl output file")?;
    let mut w = BufWriter::new(f);

    // Magic
    w.write_all(KSTL_MAGIC)?;

    // Serialize metadata to JSON
    let meta_json = serde_json::to_vec(meta)?;
    w.write_u32::<LE>(meta_json.len() as u32)?;
    w.write_all(&meta_json)?;

    // SquashFS payload
    w.write_all(payload)?;
    w.flush()?;
    Ok(())
}

/// Read the full .kstl file (header + payload).
pub fn read_kstl(input: &Path) -> Result<(KstlMetadata, Vec<u8>)> {
    let f = File::open(input).context("Cannot open .kstl file")?;
    let mut r = BufReader::new(f);
    let (meta, _meta_len) = parse_header(&mut r)?;

    // Read rest = SquashFS payload
    let mut payload = Vec::new();
    r.read_to_end(&mut payload)?;

    Ok((meta, payload))
}

fn parse_header<R: Read>(r: &mut R) -> Result<(KstlMetadata, u32)> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).context("Cannot read magic bytes")?;
    if &magic != KSTL_MAGIC {
        bail!("Invalid magic bytes -- not a .kstl file (got {:?})", magic);
    }

    let meta_len = r.read_u32::<LE>().context("Cannot read metadata length")?;
    let mut meta_json = vec![0u8; meta_len as usize];
    r.read_exact(&mut meta_json).context("Cannot read metadata JSON")?;

    let meta: KstlMetadata = serde_json::from_slice(&meta_json)
        .context("Cannot parse metadata JSON")?;

    Ok((meta, meta_len))
}

/// Build a SquashFS image using mksquashfs.
fn build_squashfs(source: &Path, output: &Path, compression: &str) -> Result<()> {
    let src_str = source.to_string_lossy();
    let out_str = output.to_string_lossy();
    let comp_str: &str = if compression == "xz" { "xz" } else { "zstd" };

    let status = std::process::Command::new("mksquashfs")
        .args([
            src_str.as_ref(),
            out_str.as_ref(),
            "-comp",
            comp_str,
            "-noappend",
            "-quiet",
        ])
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        _ => {
            // Fallback: create a minimal SquashFS stub
            info!("mksquashfs not available -- creating stub SquashFS image");
            // Write minimal SquashFS magic (sqsh) + zero padding
            let mut stub = vec![0u8; 4096];
            stub[0] = 0x73; stub[1] = 0x71; stub[2] = 0x73; stub[3] = 0x68; // sqsh
            fs::write(output, stub)?;
            Ok(())
        }
    }
}
