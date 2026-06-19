//! kestrel-host: Machine state snapshot save and restore.
//!
//! A snapshot .kstl has `kstl_type = "snapshot"` in its metadata.
//! The payload is a raw dump of the 256MB guest physical memory.
//! CPU registers are serialized into the JSON metadata itself
//! (so restoring is: write payload back to VirtualAlloc'd region,
//!  then set vCPU registers from metadata before the first run).

use anyhow::{Context, Result, bail};
use log::{info, warn};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// Number of registers we save per vCPU.
/// (RIP, RFLAGS, RSP, RAX, RBX, RCX, RDX, RSI, RDI, R8-R15, CR0, CR3, CR4, EFER)
pub const SAVED_REG_COUNT: usize = 22;

/// KSTL magic bytes.
const KSTL_MAGIC: &[u8; 4] = b"KSTL";

/// CPU register snapshot (x86-64). All values in hex strings for JSON portability.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CpuSnapshot {
    pub rip:    u64,
    pub rflags: u64,
    pub rsp:    u64,
    pub rax:    u64,
    pub rbx:    u64,
    pub rcx:    u64,
    pub rdx:    u64,
    pub rsi:    u64,
    pub rdi:    u64,
    pub r8:     u64,
    pub r9:     u64,
    pub r10:    u64,
    pub r11:    u64,
    pub r12:    u64,
    pub r13:    u64,
    pub r14:    u64,
    pub r15:    u64,
    pub cr0:    u64,
    pub cr3:    u64,
    pub cr4:    u64,
    pub efer:   u64,
    pub cs_base: u64,
}

/// Metadata block embedded in every .kstl snapshot file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMetadata {
    /// Always "snapshot" to distinguish from package .kstl files.
    pub kstl_type:   String,
    /// Human label, e.g. "my-ubuntu-session-2026-06-19"
    pub label:       String,
    /// ISO-8601 timestamp of when the snapshot was taken.
    pub created_at:  String,
    /// Size of the raw guest memory payload in bytes.
    pub memory_size: u64,
    /// Saved vCPU registers.
    pub cpu:         CpuSnapshot,
    /// Which execution backend was running (whpx | uml).
    pub backend:     String,
}

/// Save the current machine state to a .kstl snapshot file.
///
/// `guest_mem` — pointer to the VirtualAlloc'd guest memory.
/// `cpu`       — current vCPU register values.
/// `backend`   — "whpx" or "uml".
pub fn save_snapshot(
    path:      &Path,
    guest_mem: *const u8,
    cpu:       CpuSnapshot,
    backend:   &str,
) -> Result<()> {
    let label = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let now = chrono::Utc::now().to_rfc3339();

    let meta = SnapshotMetadata {
        kstl_type:   "snapshot".to_owned(),
        label,
        created_at:  now,
        memory_size: crate::GUEST_MEMORY_SIZE as u64,
        cpu,
        backend:     backend.to_owned(),
    };

    let meta_json = serde_json::to_vec(&meta)
        .context("Failed to serialize snapshot metadata")?;

    info!("[Snapshot] Saving {} MB guest memory to {:?}...",
          crate::GUEST_MEMORY_SIZE / 1024 / 1024, path);

    let mut f = fs::File::create(path)
        .with_context(|| format!("Cannot create snapshot file {:?}", path))?;

    // Write KSTL magic
    f.write_all(KSTL_MAGIC)?;

    // Write metadata length (u32 LE) + metadata JSON
    let meta_len = meta_json.len() as u32;
    f.write_all(&meta_len.to_le_bytes())?;
    f.write_all(&meta_json)?;

    // Write raw guest memory dump
    let mem_slice = unsafe {
        std::slice::from_raw_parts(guest_mem, crate::GUEST_MEMORY_SIZE)
    };
    f.write_all(mem_slice)
        .context("Failed to write guest memory to snapshot")?;

    f.flush()?;
    info!("[Snapshot] Saved to {:?} ({} MB + metadata)",
          path, crate::GUEST_MEMORY_SIZE / 1024 / 1024);
    Ok(())
}

/// Restore a machine state from a .kstl snapshot file.
///
/// Returns `(SnapshotMetadata, guest_memory_bytes)`.
/// The caller is responsible for mapping the bytes back into a
/// `VirtualAlloc`-ed region and restoring the CPU registers.
pub fn load_snapshot(path: &Path) -> Result<(SnapshotMetadata, Vec<u8>)> {
    info!("[Snapshot] Loading snapshot from {:?}...", path);

    let mut f = fs::File::open(path)
        .with_context(|| format!("Cannot open snapshot file {:?}", path))?;

    // Verify magic
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).context("Cannot read KSTL magic")?;
    if &magic != KSTL_MAGIC {
        bail!("Not a valid .kstl file (bad magic: {:?})", magic);
    }

    // Read metadata
    let mut meta_len_bytes = [0u8; 4];
    f.read_exact(&mut meta_len_bytes).context("Cannot read metadata length")?;
    let meta_len = u32::from_le_bytes(meta_len_bytes) as usize;

    let mut meta_json = vec![0u8; meta_len];
    f.read_exact(&mut meta_json).context("Cannot read metadata JSON")?;

    let meta: SnapshotMetadata = serde_json::from_slice(&meta_json)
        .context("Cannot parse snapshot metadata JSON")?;

    if meta.kstl_type != "snapshot" {
        bail!(
            "This .kstl file is a package (type='{}'), not a snapshot. \
             Use --state only with snapshot .kstl files.",
            meta.kstl_type
        );
    }

    // Read raw memory payload
    let expected_size = meta.memory_size as usize;
    let mut memory = Vec::with_capacity(expected_size);
    f.read_to_end(&mut memory)
        .context("Failed to read guest memory from snapshot")?;

    if memory.len() != expected_size {
        bail!(
            "Snapshot memory size mismatch: expected {} bytes, got {}",
            expected_size, memory.len()
        );
    }

    info!("[Snapshot] Loaded: '{}' ({}), backend={}, {} MB",
          meta.label, meta.created_at, meta.backend,
          meta.memory_size / 1024 / 1024);

    Ok((meta, memory))
}
