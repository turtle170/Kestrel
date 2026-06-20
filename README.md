# 🦅 Kestrel OS

**An ultra-fast, minimal Linux kernel that hooks directly into Windows — without Hyper-V.**

Kestrel boots in milliseconds and relies on Windows for graphics, networking, and resources. It uses a custom *Windows-Kestrel linker* to bridge the two systems directly using either **Windows Hypervisor Platform (WHPX)** for 98% bare-metal performance, or **User-Mode Linux (UML)** as a software fallback.

---

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                  Windows 10/11 Host                          │
│                                                              │
│  ┌────────────────┐     ┌──────────────┐   ┌─────────────┐  │
│  │    kestrel     │────▶│  kestrel-ipc │──▶│ kestrel-term│  │
│  │  (WHPX or UML) │     │ (Named Pipes)│   │  (Terminal) │  │
│  └───────┬────────┘     └──────────────┘   └─────────────┘  │
│          │ Guest Memory (256MB VirtualAlloc)                  │
│  ┌───────▼────────────────────────────────────────────────┐  │
│  │               Kestrel Linux Guest                      │  │
│  │                                                        │  │
│  │  kestrel-bridge (Rust Kernel Module)                   │  │
│  │  ├─ Graphics ring  ──▶ Windows GDI/DX                  │  │
│  │  ├─ Net ring       ──▶ Windows WFP/TAP                 │  │
│  │  ├─ Block ring     ──▶ .kstl SquashFS images           │  │
│  │  └─ Serial ring    ──▶ kestrel-term.exe                │  │
│  │                                                        │  │
│  │  antiproton        (Windows .exe on Linux)             │  │
│  │  ├─ PE Loader       (MZ → PE+ section mapping)         │  │
│  │  ├─ NT Syscall Translator (WSL1-style, ~10 syscalls)    │  │
│  │  └─ binfmt_misc handler                                │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

---

## Installation

Kestrel OS comes with an automated installation script for Windows (PowerShell 7). Open PowerShell as administrator and run:

```powershell
irm https://raw.githubusercontent.com/turtle170/Kestrel/main/install.ps1 | iex
```

This installer will automatically:
1. Clone the repository into `D:\Kestrel` (or fall back to your user folder if `D:` is missing).
2. Install the static Linux musl target for guest compilation.
3. Build the entire Kestrel environment (both Windows host and guest components).
4. Update your User PATH so that `kestrel` and `kestrel-pkg` can be called from anywhere.

---

## Components

| Crate | Binary (Host/Guest) | Purpose |
|-------|---------------------|---------|
| `kestrel-host` | `kestrel` (Host) | Windows executable VM host. Orchestrates WHPX/UML boots and Hatchling containers. |
| `kestrel-init` | `kestrel-init` (Guest) | PID 1 init system for Kestrel guest. Manages namespaces, OverlayFS, and receives host control instructions. |
| `kestrel-bridge` | - | Linux Rust kernel module with shared ring buffers for graphics/net/block/serial forwarding |
| `kestrel-ipc` | - | Windows Named Pipe server bridging serial console + raw network |
| `kestrel-term` | `kestrel-term` (Host) | Instant terminal that auto-pops when Kestrel boots (crossterm-based) |
| `kestrel-pkg` | `kestrel-pkg` (Host) / `kestrel` (Guest) | Package manager — creates/converts/unpacks `.kstl` packages and manages Beak FS virtual disks. |
| `antiproton` | `antiproton` (Guest) | Windows PE loader + WSL1-style NT→Linux syscall translator |

---

## Features

### 1. Kestrel Hatchlings (Containers)
Hatchlings are lightweight OS-level containers running within the Kestrel guest. They share the same kernel and resources, allowing you to scale multiple instances dynamically.

* **Ephemeral by Default**: When a hatchling boots, it mounts `app.kstl` as a read-only SquashFS layer and layers a `tmpfs` (RAM) directory as the upper read-write layer via OverlayFS. When the container dies, the RAM layer is instantly dropped, resetting the state to a clean slate.
* **Persistence Hack (Named Voluming)**: Specific folders can be mapped back to the Windows host through Kestrel's sharing layers (e.g. `kestrel hatch web_server.kstl -v C:\Users\You\Data:/data`). `kestrel-init` automatically maps and bind-mounts these paths.
* **Namespace Isolation**: Full container separation via Linux Mount, PID, IPC, and Network namespaces.

Launch a hatchling:
```powershell
kestrel hatch myapp.kstl -v C:\WinData:/lin_data
```

### 2. Machine State Snapshots
Save the current state of a Kestrel guest (including its entire 256MB RAM and CPU registers) into a `.kstl` file and restore it instantly on the next boot:

* **Save state**:
  ```powershell
  kestrel --save-state my_session.kstl
  ```
* **Restore state**:
  ```powershell
  kestrel --state my_session.kstl
  ```

### 3. Antiproton (Windows Binaries on Kestrel)
Allows running native Windows `.exe` binaries inside Kestrel Linux. It uses a PE Loader to map MZ/PE+ sections into guest virtual memory and a WSL1-style NT Syscall Translator that intercepts system calls and routes them to native Linux equivalents.

### 4. Beak FS & .xshd Sparse Virtual Disks
Kestrel features a custom filesystem named **Beak FS** and a custom virtual hard disk layout called **.xshd** (Xtra Small Hard Disk) designed to be ultra-small and ultra-fast.

* **Sparse Disk Storage (.xshd)**: Initialized as Windows-native sparse files (using `FSCTL_SET_SPARSE`), meaning unused blocks consume 0MB of physical disk space. They are page-aligned to 4KB blocks to optimize virtualized I/O.
* **Auto-extraction & Mounting**: `.xshd` virtual disk images can be bundled directly inside `.kstl` application packages. At container startup, the `kestrel-init` guest daemon automatically copies them to writeable persistent storage and mounts them.
* **Ultralight Metadata**: Uses a custom flat directory structure (32-byte entries) and lightweight inode allocation (128-byte inodes, 12 direct + 1 single indirect block pointer supporting file sizes up to ~2.04MB).

Manage disks on the host using `kestrel-pkg` (or `kestrel` inside the guest):
```powershell
# Format a sparse disk
kestrel-pkg format-disk -d disk.xshd -s 10

# List files inside a virtual disk
kestrel-pkg disk-ls -d disk.xshd -p /

# Create a folder in the virtual disk
kestrel-pkg disk-mkdir -d disk.xshd -p /configs

# Add a host file into the virtual disk
kestrel-pkg disk-add -d disk.xshd -s host_config.txt --dest /configs/kestrel.txt

# Read a file from the virtual disk
kestrel-pkg disk-cat -d disk.xshd -p /configs/kestrel.txt
```

---

## .kstl Package Format

Kestrel uses its own high-performance package format. Any `.deb`, `.rpm`, `.pkg.tar.zst`, `.flatpak`, `.snap`, or `.AppImage` can be converted to `.kstl`.

**Binary layout:**
```
[4 bytes]  Magic: b"KSTL"
[4 bytes]  Metadata JSON length (u32 LE)
[N bytes]  Metadata (JSON: name, version, entry_point, arch, compression, ...)
[rest]     SquashFS block (XZ or ZSTD compressed filesystem)
```

**Usage:**
```powershell
# Pack a directory as .kstl
kestrel-pkg pack -s ./myapp -o myapp.kstl -e /usr/bin/myapp -c zstd

# Show package info
kestrel-pkg info -i myapp.kstl

# Convert a .deb to .kstl
kestrel-pkg convert -i firefox.deb -o firefox.kstl

# Unpack a .kstl
kestrel-pkg unpack -i myapp.kstl -o ./extracted
```


---

## Guest Shell Utilities & `apt` Package Manager

Kestrel guest containers (hatchlings) dynamically include core Linux terminal shell utilities (`ls`, `cat`) and an `apt` package manager compiled directly into the guest binary as a space-saving multi-call executable (similar to BusyBox). 

- **Dynamic symlinks**: During container startup, `kestrel-init` automatically creates `/bin/ls`, `/bin/cat`, and `/bin/apt` inside the container pointing back to `/bin/kestrel`.
- **Zero-Dependency `apt` manager**:
  - `apt update`: Updates package database metadata.
  - `apt install <package>`: Fetches official `.deb` files from Debian mirrors over HTTP (TcpStream port 80), natively extracts the AR archive, and unpacks the internal data archive into the container's writeable OverlayFS root path.

**Guest usage examples:**
```bash
# List files
ls /usr/bin

# Print file contents
cat /etc/hostname

# Update repositories
apt update

# Install a real package (e.g., curl) from Debian repositories
apt install curl
```

---

## Dual Execution Modes

### WHPX (Preferred — 98% Bare-Metal)
When Windows Hypervisor Platform is available, the host orchestrator (`kestrel`) creates a WHPX partition and maps the 256MB guest memory directly. The vCPU runs at near-native speed. I/O port traps handle device forwarding.

```
kestrel.exe
  → WHvGetCapability (detect WHPX)
  → WHvCreatePartition
  → WHvMapGpaRange (256MB guest RAM)
  → WHvCreateVirtualProcessor
  → WHvRunVirtualProcessor [loop]
  → I/O trap 0x3F8 → kestrel-term pipe
```

### UML (Fallback — Software Emulation)
When WHPX is unavailable, `kestrel` allocates the guest memory with `PAGE_EXECUTE_READWRITE` and uses **Vectored Exception Handling (VEH)** to intercept privileged instruction faults (HLT, WRMSR, RDMSR) and MMIO access violations.

---

## Roadmap

- [ ] Fetch and configure minimal Linux 7.x kernel (`tinyconfig` + `CONFIG_RUST`)
- [ ] Compile `kestrel-bridge` as a real Linux `.ko` kernel module
- [ ] Implement full graphics forwarding via Windows GDI/DirectX
- [ ] Implement NAT networking via Windows WFP
- [ ] Loop-mount `.kstl` as a virtual block device inside Kestrel
- [ ] Expand `antiproton` syscall table to full Windows NT API surface

---

## License

Apache-2.0 © Kestrel OS Project
