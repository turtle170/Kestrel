# 🦅 Kestrel OS Command Line Interface (CLI) Reference

This document provides a comprehensive guide to all command-line interfaces, options, and commands available in the Kestrel OS ecosystem (both on the Windows host and inside the Linux guest containers).

---

## Table of Contents
1. [Kestrel Host Orchestrator (`kestrel`)](#1-kestrel-host-orchestrator-kestrel)
2. [Kestrel Package & Disk Tool (`kestrel-pkg`)](#2-kestrel-package--disk-tool-kestrel-pkg)
3. [Kestrel Guest Package Managers (`apt`, `dnf`, `pacman`)](#3-kestrel-guest-package-managers-apt-dnf-pacman)
4. [Guest Multi-call Utilities](#4-guest-multi-call-utilities)
5. [Architecture & Deployment Integration](#5-architecture--deployment-integration)

---

## 1. Kestrel Host Orchestrator (`kestrel`)

The `kestrel` (or `kestrel.exe`) binary is the core execution driver running on the Windows host. It is responsible for booting the Kestrel Linux kernel (using WHPX or UML backend) and orchestrating container instances (hatchlings).

### Syntax
```powershell
kestrel.exe [OPTIONS] [SUBCOMMAND]
```

### Options
* `--state <PATH.kstl>`  
  Resume the virtual machine execution from a saved `.kstl` snapshot state instead of performing a cold kernel boot.
* `--save-state <PATH.kstl>`  
  Save the current guest physical memory (1GB capacity) and CPU registers into a `.kstl` snapshot file on graceful VM shutdown.
* `--daemon`  
  Run the Kestrel host orchestration control plane as a background system daemon. This spins up the named pipe listener `\\.\pipe\kestrel-control` and the TCP command bridge on port `9002` without launching the VM, allowing detached hatchling management.

### Subcommands

#### `hatch`
Spawns a new container (hatchling) within the currently running Kestrel virtual machine.
```powershell
kestrel hatch <APP.kstl> [OPTIONS]
```
* **Arguments:**
  * `<APP.kstl>`: The path to the `.kstl` application package to spawn.
* **Options:**
  * `-v, --volume <WIN:LIN>`: Bind-mounts a Windows host folder to a container path (e.g. `-v C:\Users\Public\Data:/data`). Can be specified multiple times.

---

## 2. Kestrel Package & Disk Tool (`kestrel-pkg`)

The `kestrel-pkg` (or `kestrel-pkg.exe`) tool is a Swiss Army knife for building packages, converting foreign linux archives, formatting Beak FS sparse virtual disks, and constructing the initramfs boot images.

### Syntax
```powershell
kestrel-pkg.exe <SUBCOMMAND> [OPTIONS]
```

### Package Commands

#### `pack`
Packs a target host directory into a compressed `.kstl` package file with metadata.
```powershell
kestrel-pkg pack -s <SOURCE_DIR> -o <OUTPUT.kstl> -e <ENTRY_POINT> [-c <xz|zstd>]
```
* `-s, --source <PATH>`: The source directory containing the files to bundle.
* `-o, --output <PATH>`: Destination path for the `.kstl` package.
* `-e, --entry <PATH>`: Entry point program to execute inside the container (e.g., `/usr/bin/python3`).
* `-c, --compression <xz|zstd>`: SquashFS compression algorithm (defaults to `zstd`).

#### `unpack`
Extracts a `.kstl` package back into a directory on the host.
```powershell
kestrel-pkg unpack -i <INPUT.kstl> -o <OUTPUT_DIR>
```

#### `info`
Reads and displays JSON metadata (e.g. name, version, entry point, architecture, compression, and payload size) from a `.kstl` package header without unpacking it.
```powershell
kestrel-pkg info -i <INPUT.kstl>
```

#### `convert`
Converts standard Linux package formats directly into Kestrel-compatible `.kstl` packages.
```powershell
kestrel-pkg convert -i <FOREIGN_PKG> -o <OUTPUT.kstl>
```
* **Supported input formats:**
  * Debian Packages (`.deb`)
  * Red Hat Packages (`.rpm`)
  * Arch Linux Packages (`.pkg.tar.zst`, `.pkg.tar.xz`)
  * Flatpak Bundles (`.flatpak`)
  * Snaps (`.snap`)
  * AppImages (`.AppImage`)

---

### Disk Commands (Beak FS / `.xshd`)

#### `format-disk`
Formats a sparse virtual disk file (`.xshd`) with the custom, page-aligned **Beak FS** layout.
```powershell
kestrel-pkg format-disk -d <DISK_PATH.xshd> [-s <SIZE_MB>]
```
* `-d, --disk <PATH>`: Path to the virtual disk file.
* `-s, --size-mb <SIZE>`: Total size capacity in MB (defaults to `10` MB). The file is allocated as a sparse file on NTFS, consuming 0MB physically until written.

#### `disk-ls`
Lists directory contents inside a Beak FS virtual disk.
```powershell
kestrel-pkg disk-ls -d <DISK_PATH.xshd> [-p <PATH>]
```

#### `disk-mkdir`
Creates a new directory inside a Beak FS virtual disk.
```powershell
kestrel-pkg disk-mkdir -d <DISK_PATH.xshd> -p <PATH>
```

#### `disk-add`
Copies a file from the Windows host into a specific path inside the Beak FS virtual disk.
```powershell
kestrel-pkg disk-add -d <DISK_PATH.xshd> -s <HOST_SRC_PATH> --dest <DISK_DEST_PATH>
```

#### `disk-cat`
Displays the text contents of a file stored inside the Beak FS virtual disk.
```powershell
kestrel-pkg disk-cat -d <DISK_PATH.xshd> -p <PATH>
```

---

### Deployment Commands

#### `build-initramfs`
Generates the standard guest `initramfs.cpio` file, pre-baking all 200+ multi-call shell utility symlinks directly inside the CPIO file table.
```powershell
kestrel-pkg build-initramfs -i <INIT_BIN> -k <KESTREL_BIN> -o <OUTPUT_CPIO>
```
* `-i, --init <PATH>`: Path to the compiled `kestrel-init` guest binary.
* `-k, --kestrel <PATH>`: Path to the compiled `kestrel-pkg` guest binary (which acts as the guest's multi-call backend, named `kestrel`).
* `-o, --output <PATH>`: Path to write the output `initramfs.cpio` archive.

---

## 3. Kestrel Guest Package Managers (`apt`, `dnf`, `pacman`)

Kestrel features built-in multi-call command routing inside guest containers. Depending on the command name called (`args[0]`), Kestrel routes execution to different package managers:

### APT (`apt`)
A zero-dependency Debian package manager compiled into the guest binary.
* `apt update`  
  Simulates repository synchronization and reads Debian repository metadata.
* `apt install <package>`  
  Natively downloads the `.deb` archive from official Debian mirrors over HTTP (port 80), extracts the AR wrapper, and unpacks the payload directly into the container's overlay writeable directory tree.

### DNF (`dnf`)
A guest Fedora package client.
* `dnf update` or `dnf makecache`  
  Caches repository metadata from Fedora mirrors.
* `dnf install <package>`  
  Downloads `.rpm` archives from Fedora mirrors and extracts them into the root file tables.

### Pacman (`pacman`)
A guest Arch Linux package client.
* `pacman -Sy`  
  Synchronizes the local package databases with Arch mirrors.
* `pacman -S <package>`  
  Downloads and extracts `.pkg.tar.zst` packages directly into the root filesystem.

> [!NOTE]
> All three guest package managers feature a robust **offline mock fallback** mechanism. If a mirror is unreachable or the host has no internet connection, they will automatically generate a mock execution stub in `/usr/bin/` to prevent scripts and builds from failing.

---

## 4. Guest Multi-call Utilities

The guest `kestrel` binary behaves similarly to BusyBox. By creating symlinks pointing to it (or renaming it), you access native lightweight implementations of core Linux tools:

### Built-in Commands (Executed inside `kestrel` guest)
* **`ls [path]`**: Displays a formatted listing of the specified directory.
* **`cat [files...]`**: Concatenates and prints file contents.
* **`echo [text]`**: Outputs strings to standard output.
* **`pwd`**: Prints the current working directory path.
* **`uname`**: Returns system details (configured as `Linux kestrel 7.0.12-x86_64`).
* **`whoami` / `id`**: Returns current guest user identification.
* **`sleep` / `clear` / `hostname` / `true` / `false`**: Standard terminal support utilities.

### Uninstalled Tool Redirection
If a command is invoked inside the guest container but is not natively supported or installed on `PATH`, the guest multi-call router intercepts the call, prints a unified multi-suggestion block, and exits with code `127`:
```bash
$ git clone https://github.com/...
Command 'git' not found, but can be installed with:
  apt install git
  dnf install git
  pacman -S git
```

---

## 5. Architecture & Deployment Integration

### Sparse Virtual Disks & Auto-Mounts
1. Place a `.xshd` virtual disk in your application folder.
2. Run `kestrel-pkg pack` to bundle your application files and the `.xshd` image.
3. Upon booting the `.kstl` package using `kestrel hatch`, `kestrel-init` will:
   * Copy the sparse disk to persistent storage.
   * Format a **4GB swapfile** on the disk (`/data/swapfile`) and mount it.
   * Enable aggressive swap swapping (`vm.swappiness = 100`) to free up Guest RAM.
   * Constrain physical host memory usage to a **1GB guest RAM ceiling**.
