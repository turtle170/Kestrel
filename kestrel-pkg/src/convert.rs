//! Conversion of foreign package formats to .kstl.

use anyhow::{Result, Context, bail};
use log::{info, warn};
use std::path::Path;
use std::fs;
use tempfile::TempDir;

use crate::kstl::{self, KstlMetadata};

/// Detect format and convert to .kstl.
pub fn convert_to_kstl(input: &Path, output: &Path) -> Result<()> {
    let ext = input
        .to_string_lossy()
        .to_lowercase();

    let stem = input
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    info!("Detecting format for {:?}...", input);

    if ext.ends_with(".deb") {
        convert_deb(input, output, &stem)
    } else if ext.ends_with(".rpm") {
        convert_rpm(input, output, &stem)
    } else if ext.ends_with(".pkg.tar.zst") || ext.ends_with(".pkg.tar.xz") {
        convert_arch_pkg(input, output, &stem)
    } else if ext.ends_with(".flatpak") {
        convert_flatpak(input, output, &stem)
    } else if ext.ends_with(".snap") {
        convert_snap(input, output, &stem)
    } else if ext.ends_with(".appimage") {
        convert_appimage(input, output, &stem)
    } else {
        bail!("Unknown package format: {:?}", input);
    }
}

fn convert_deb(input: &Path, output: &Path, name: &str) -> Result<()> {
    info!("[Convert] Extracting .deb archive...");
    let tmp = TempDir::new()?;
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir)?;

    // Use ar + tar to extract .deb (data.tar.*)
    let status = std::process::Command::new("ar")
        .args(["x", &input.to_string_lossy()])
        .current_dir(tmp.path())
        .status();

    match status {
        Ok(s) if s.success() => {
            // Find data.tar.*
            let data_tar = find_file(tmp.path(), "data.tar")?;
            extract_tar(&data_tar, &data_dir)?;
        }
        _ => {
            warn!("[Convert] 'ar' not found -- trying dpkg-deb");
            std::process::Command::new("dpkg-deb")
                .args(["--extract", &input.to_string_lossy(), &data_dir.to_string_lossy()])
                .status()
                .context("dpkg-deb also failed")?;
        }
    }

    // Find a plausible entry point
    let entry = find_entry_point(&data_dir, name);

    pack_dir_to_kstl(&data_dir, output, name, &entry)
}

fn convert_rpm(input: &Path, output: &Path, name: &str) -> Result<()> {
    info!("[Convert] Extracting .rpm archive...");
    let tmp = TempDir::new()?;
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir)?;

    // rpm2cpio | cpio -idmv
    let rpm2cpio = std::process::Command::new("rpm2cpio")
        .arg(input)
        .output();

    match rpm2cpio {
        Ok(r) if r.status.success() => {
            let mut cpio = std::process::Command::new("cpio")
                .args(["-idmv"])
                .current_dir(&data_dir)
                .stdin(std::process::Stdio::piped())
                .spawn()?;
            if let Some(mut stdin) = cpio.stdin.take() {
                use std::io::Write;
                stdin.write_all(&r.stdout)?;
            }
            cpio.wait()?;
        }
        _ => bail!("rpm2cpio is not available -- cannot convert .rpm"),
    }

    let entry = find_entry_point(&data_dir, name);
    pack_dir_to_kstl(&data_dir, output, name, &entry)
}

fn convert_arch_pkg(input: &Path, output: &Path, name: &str) -> Result<()> {
    info!("[Convert] Extracting .pkg.tar.zst archive...");
    let tmp = TempDir::new()?;
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir)?;

    extract_tar(input, &data_dir)?;

    let entry = find_entry_point(&data_dir, name);
    pack_dir_to_kstl(&data_dir, output, name, &entry)
}

fn convert_flatpak(input: &Path, output: &Path, name: &str) -> Result<()> {
    info!("[Convert] Extracting .flatpak (OCI/OSTree bundle)...");
    let tmp = TempDir::new()?;
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir)?;

    std::process::Command::new("unzip")
        .args(["-o", &input.to_string_lossy(), "-d", &data_dir.to_string_lossy()])
        .status()
        .context("unzip failed for flatpak")?;

    let entry = find_entry_point(&data_dir, name);
    pack_dir_to_kstl(&data_dir, output, name, &entry)
}

fn convert_snap(input: &Path, output: &Path, name: &str) -> Result<()> {
    info!("[Convert] Extracting .snap (SquashFS)...");
    // .snap files are SquashFS images -- just repackage them.
    let snap_data = fs::read(input)?;

    let meta = KstlMetadata {
        name: name.to_owned(),
        entry_point: format!("/usr/bin/{}", name),
        compression: "zstd".into(),
        payload_size: snap_data.len() as u64,
        ..Default::default()
    };

    // .snap is already a SquashFS - directly embed it
    kstl::write_kstl(output, &meta, &snap_data)?;
    info!("[Convert] .snap repackaged directly as .kstl");
    Ok(())
}

fn convert_appimage(input: &Path, output: &Path, name: &str) -> Result<()> {
    info!("[Convert] Extracting .AppImage (ELF + SquashFS)...");
    // AppImages have an ELF stub followed by a SquashFS at a fixed offset
    let data = fs::read(input)?;

    // Find SquashFS magic 0x73717368 after ELF header
    let sqfs_offset = data.windows(4)
        .position(|w| w == b"sqsh")
        .context("Cannot find SquashFS block in AppImage")?;

    let sqfs_data = &data[sqfs_offset..];

    let meta = KstlMetadata {
        name: name.to_owned(),
        entry_point: format!("/usr/bin/{}", name),
        compression: "zstd".into(),
        payload_size: sqfs_data.len() as u64,
        ..Default::default()
    };

    kstl::write_kstl(output, &meta, sqfs_data)?;
    info!("[Convert] AppImage SquashFS extracted and repackaged as .kstl");
    Ok(())
}

fn pack_dir_to_kstl(dir: &Path, output: &Path, _name: &str, entry: &str) -> Result<()> {
    info!("[Convert] Packing directory into .kstl...");
    kstl::pack(dir, output, entry, "zstd")
}

fn extract_tar(archive: &Path, dest: &Path) -> Result<()> {
    let status = std::process::Command::new("tar")
        .args(["xf", &archive.to_string_lossy(), "-C", &dest.to_string_lossy()])
        .status()
        .context("tar extraction failed")?;
    if !status.success() {
        bail!("tar exited with non-zero status: {:?}", status.code());
    }
    Ok(())
}

fn find_file(dir: &Path, prefix: &str) -> Result<std::path::PathBuf> {
    for entry in fs::read_dir(dir)? {
        let e = entry?;
        if e.file_name().to_string_lossy().starts_with(prefix) {
            return Ok(e.path());
        }
    }
    bail!("Cannot find file with prefix '{}' in {:?}", prefix, dir)
}

/// Try to find a plausible binary entry point inside the extracted package.
fn find_entry_point(dir: &Path, name: &str) -> String {
    let candidates = [
        format!("usr/bin/{}", name),
        format!("usr/local/bin/{}", name),
        format!("bin/{}", name),
        format!("usr/sbin/{}", name),
    ];
    for c in &candidates {
        if dir.join(c).exists() {
            return format!("/{}", c);
        }
    }
    format!("/usr/bin/{}", name)
}
