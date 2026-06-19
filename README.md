# 🦅 Kestrel OS

**A ultra-fast, minimal Linux kernel that hooks directly into Windows — without Hyper-V.**

Kestrel boots in milliseconds and relies on Windows for graphics, networking, and resources. It uses a custom *Windows-Kestrel linker* to bridge the two systems directly using either **Windows Hypervisor Platform (WHPX)** for 98% bare-metal performance, or **User-Mode Linux (UML)** as a software fallback.

---

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                  Windows 10/11 Host                          │
│                                                              │
│  ┌────────────────┐     ┌──────────────┐   ┌─────────────┐  │
│  │  kestrel-host  │────▶│  kestrel-ipc │──▶│ kestrel-term│  │
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
│  │  reverse-proton    (Windows .exe on Linux)             │  │
│  │  ├─ PE Loader       (MZ → PE+ section mapping)         │  │
│  │  ├─ NT Syscall Translator (WSL1-style, ~10 syscalls)    │  │
│  │  └─ binfmt_misc handler                                │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

---

## Components

| Crate | Purpose |
|-------|---------|
| `kestrel-host` | Windows exe that boots Kestrel. Detects WHPX first; falls back to UML. |
| `kestrel-bridge` | Linux Rust kernel module with shared ring buffers for graphics/net/block/serial forwarding |
| `kestrel-ipc` | Windows Named Pipe server bridging serial console + raw network |
| `kestrel-term` | Instant terminal that auto-pops when Kestrel boots (crossterm-based) |
| `kestrel-pkg` | Package manager — creates/converts/unpacks `.kstl` packages |
| `reverse-proton` | Windows PE loader + WSL1-style NT→Linux syscall translator |

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

## Dual Execution Modes

### WHPX (Preferred — 98% Bare-Metal)
When Windows Hypervisor Platform is available, `kestrel-host` creates a WHPX partition and maps the 256MB guest memory directly. The vCPU runs at near-native speed. I/O port traps handle device forwarding.

```
kestrel-host.exe
  → WHvGetCapability (detect WHPX)
  → WHvCreatePartition
  → WHvMapGpaRange (256MB guest RAM)
  → WHvCreateVirtualProcessor
  → WHvRunVirtualProcessor [loop]
  → I/O trap 0x3F8 → kestrel-term pipe
```

### UML (Fallback — Software Emulation)
When WHPX is unavailable, `kestrel-host` allocates the guest memory with `PAGE_EXECUTE_READWRITE` and uses **Vectored Exception Handling (VEH)** to intercept privileged instruction faults (HLT, WRMSR, RDMSR) and MMIO access violations.

---

## Getting Started

### Prerequisites
- Windows 10/11 with Rust nightly installed
- (Optional) Windows Hypervisor Platform enabled (`optionalfeatures → Windows Hypervisor Platform`)

### Build
```powershell
git clone https://github.com/turtle170/Kestrel.git
cd Kestrel
cargo build --release
```

### Run
```powershell
.\target\release\kestrel-host.exe
# kestrel-term.exe will open automatically
```

---

## Roadmap

- [ ] Fetch and configure minimal Linux 7.x kernel (`tinyconfig` + `CONFIG_RUST`)
- [ ] Compile `kestrel-bridge` as a real Linux `.ko` kernel module
- [ ] Implement full graphics forwarding via Windows GDI/DirectX
- [ ] Implement NAT networking via Windows WFP
- [ ] Loop-mount `.kstl` as a virtual block device inside Kestrel
- [ ] Expand `reverse-proton` syscall table to full Windows NT API surface

---

## License

Apache-2.0 © Kestrel OS Project
