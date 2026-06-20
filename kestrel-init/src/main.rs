//! kestrel-init: PID 1 init system and container orchestration daemon for Kestrel OS.
//!
//! Listens for orchestration commands from the Windows host (via TCP control channel)
//! and spawns isolated container instances ("hatchlings") using Linux namespaces,
//! OverlayFS (SquashFS lower layer + tmpfs upper layer), and bind mounts.
//!
//! When compiled on Windows, it runs in a simulated mock mode to ensure workspace
//! compile-safety and testability.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HatchRequest {
    pub app_path: PathBuf,
    pub volumes: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KstlMetadata {
    pub name: String,
    pub version: String,
    pub entry_point: String,
    pub compression: String,
    pub payload_size: u64,
}

fn main() {
    println!("=============================================");
    println!("     Kestrel OS Init Daemon (PID 1) v0.1     ");
    println!("=============================================");

    #[cfg(target_os = "linux")]
    {
        println!("[Init] Running in native Linux mode.");
        // Mount initial filesystems if running as PID 1
        let _ = fs::create_dir_all("/proc");
        let _ = fs::create_dir_all("/sys");
        let _ = fs::create_dir_all("/tmp");
        
        // Suppress errors as they might already be mounted
        let _ = nix::mount::mount(
            Some("proc"),
            "/proc",
            Some("proc"),
            nix::mount::MsFlags::empty(),
            None::<&str>,
        );
        let _ = nix::mount::mount(
            Some("sysfs"),
            "/sys",
            Some("sysfs"),
            nix::mount::MsFlags::empty(),
            None::<&str>,
        );
        let _ = nix::mount::mount(
            Some("tmpfs"),
            "/tmp",
            Some("tmpfs"),
            nix::mount::MsFlags::empty(),
            None::<&str>,
        );
    }

    #[cfg(not(target_os = "linux"))]
    {
        println!("[Init] Running in mock Windows mode.");
    }

    // Start connection loop to host control server
    let mut retry_count = 0;
    loop {
        // Try connecting to 127.0.0.1 (WSL/localhost) or 10.0.2.2 (UML/hypervisor gateway)
        let host_addr = if retry_count % 2 == 0 { "127.0.0.1:9002" } else { "10.0.2.2:9002" };
        println!("[Init] Connecting to host control bridge at {}...", host_addr);

        match TcpStream::connect(host_addr) {
            Ok(mut stream) => {
                println!("[Init] Connected to host orchestration channel!");
                retry_count = 0;
                if let Err(e) = handle_control_channel(&mut stream) {
                    println!("[Init] Control channel error: {}. Reconnecting...", e);
                }
            }
            Err(_) => {
                retry_count += 1;
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Handles the persistence connection loop with the host.
fn handle_control_channel(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            println!("[Init] Host closed connection.");
            break;
        }

        let msg = String::from_utf8_lossy(&buf[..n]);
        println!("[Init] Received control command: {}", msg);

        if let Ok(req) = serde_json::from_str::<HatchRequest>(&msg) {
            let req_clone = req.clone();
            thread::spawn(move || {
                if let Err(e) = spawn_hatchling(req_clone) {
                    println!("[Hatchling] Failed to spawn container: {}", e);
                }
            });
            
            // Send acknowledgement
            let ack = serde_json::to_string(&serde_json::json!({
                "status": "received",
                "app": req.app_path
            }))?;
            stream.write_all(ack.as_bytes())?;
            stream.flush()?;
        } else {
            println!("[Init] Failed to parse HatchRequest from host.");
        }
    }
    Ok(())
}

/// Parse the .kstl file and extract metadata + SquashFS payload.
#[cfg(target_os = "linux")]
fn extract_kstl(kstl_path: &Path, hatch_dir: &Path) -> Result<(KstlMetadata, PathBuf), Box<dyn std::error::Error>> {
    let mut file = fs::File::open(kstl_path)?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"KSTL" {
        return Err("Invalid KSTL magic header".into());
    }

    let mut meta_len_bytes = [0u8; 4];
    file.read_exact(&mut meta_len_bytes)?;
    let meta_len = u32::from_le_bytes(meta_len_bytes) as usize;

    let mut meta_json = vec![0u8; meta_len];
    file.read_exact(&mut meta_json)?;
    let meta: KstlMetadata = serde_json::from_slice(&meta_json)?;

    let sqfs_path = hatch_dir.join("payload.sqfs");
    let mut sqfs_file = fs::File::create(&sqfs_path)?;
    
    // Copy remaining bytes (SquashFS payload) to the local temp file
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        sqfs_file.write_all(&buffer[..bytes_read])?;
    }
    
    Ok((meta, sqfs_path))
}

/// Spawns a container instance.
fn spawn_hatchling(req: HatchRequest) -> Result<(), Box<dyn std::error::Error>> {
    let app_name = req.app_path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
        
    let hatch_id = format!("{}-{}", app_name, std::process::id());
    println!("[Hatchling] Orchestrating container '{}'...", hatch_id);

    #[cfg(target_os = "linux")]
    {
        // 1. Create directory structures
        let base_dir = PathBuf::from(format!("/tmp/hatchlings/{}", hatch_id));
        let lower_dir = base_dir.join("lower");
        let upper_tmpfs = base_dir.join("upper_tmpfs");
        let upper_dir = upper_tmpfs.join("upper");
        let work_dir = upper_tmpfs.join("work");
        let merged_dir = base_dir.join("merged");

        fs::create_dir_all(&lower_dir)?;
        fs::create_dir_all(&upper_dir)?;
        fs::create_dir_all(&work_dir)?;
        fs::create_dir_all(&merged_dir)?;

        // Translate the kstl path from Windows path if needed (e.g. C:\path -> /mnt/c/path)
        let guest_kstl_path = translate_path(&req.app_path);
        println!("[Hatchling] Reading package from {:?}", guest_kstl_path);

        // 2. Extract SquashFS payload from the .kstl file
        let (meta, sqfs_path) = extract_kstl(&guest_kstl_path, &base_dir)?;
        println!("[Hatchling] Extracted package metadata: {:?}", meta);

        // 3. Mount SquashFS lower layer
        println!("[Hatchling] Mounting SquashFS lower layer...");
        let mount_status = std::process::Command::new("mount")
            .args(&[
                "-o", "loop",
                "-t", "squashfs",
                &sqfs_path.to_string_lossy(),
                &lower_dir.to_string_lossy()
            ])
            .status()?;
        if !mount_status.success() {
            return Err("Failed to loop-mount SquashFS".into());
        }

        // 4. Mount tmpfs for OverlayFS upper/work layers
        println!("[Hatchling] Mounting tmpfs upper layer...");
        nix::mount::mount(
            Some("tmpfs"),
            &upper_tmpfs,
            Some("tmpfs"),
            nix::mount::MsFlags::empty(),
            None::<&str>,
        )?;
        
        // Re-create upper and work inside the mounted tmpfs
        fs::create_dir_all(&upper_dir)?;
        fs::create_dir_all(&work_dir)?;

        // 5. Mount OverlayFS
        println!("[Hatchling] Mounting OverlayFS...");
        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower_dir.to_string_lossy(),
            upper_dir.to_string_lossy(),
            work_dir.to_string_lossy()
        );
        nix::mount::mount(
            Some("overlay"),
            &merged_dir,
            Some("overlay"),
            nix::mount::MsFlags::empty(),
            Some(options.as_str()),
        )?;

        // 5.2 Populate container bin directory with core utilities and apt symlinks
        let bin_dir = merged_dir.join("bin");
        let _ = fs::create_dir_all(&bin_dir);
        let container_kestrel_path = bin_dir.join("kestrel");
        
        let guest_kestrel_path = Path::new("/bin/kestrel");
        if guest_kestrel_path.exists() {
            let _ = fs::copy(guest_kestrel_path, &container_kestrel_path);
        } else if let Ok(current_exe) = std::env::current_exe() {
            let _ = fs::copy(current_exe, &container_kestrel_path);
        }
        
        let utilities = &[
            "ls", "cat", "grep", "sed", "awk", "cut", "paste", "join", "sort", "uniq", "wc", "head", 
            "tail", "tee", "xargs", "tr", "diff", "patch", "cp", "mv", "rm", "mkdir", "rmdir", "touch", 
            "ln", "pwd", "stat", "chmod", "chown", "chgrp", "dd", "df", "du", "mount", "umount", "fdisk", 
            "gdisk", "parted", "mkfs", "fsck", "lsblk", "blkid", "uname", "dmesg", "uptime", "hostname", 
            "lshw", "lspci", "lsusb", "lsmod", "modprobe", "insmod", "rmmod", "sysctl", "ps", "top", 
            "htop", "atop", "kill", "killall", "pkill", "pgrep", "nice", "renice", "nohup", "bg", "fg", 
            "jobs", "lsof", "strace", "ltrace", "time", "taskset", "chrt", "unshare", "nsenter", "ip", 
            "ping", "traceroute", "tracepath", "netstat", "ss", "tcpdump", "nc", "curl", "wget", "iptables", 
            "nftables", "route", "dig", "nslookup", "arp", "sudo", "su", "whoami", "id", "useradd", 
            "userdel", "usermod", "groupadd", "groupdel", "passwd", "chage", "groups", "last", "lastb", 
            "tar", "gzip", "gunzip", "bzip2", "bunzip2", "xz", "unxz", "zstd", "unzstd", "zip", "unzip", 
            "cpio", "init", "systemctl", "echo", "printf", "less", "more", "clear", "env", "export", 
            "alias", "unalias", "history", "man", "info", "help", "which", "whereis", "whatis", "file", 
            "ldd", "nm", "objdump", "readelf", "size", "strip", "make", "gcc", "g++", "clang", "as", 
            "ld", "gdb", "valgrind", "perf", "git", "ssh", "scp", "rsync", "sftp", "telnet", "ftp", 
            "screen", "tmux", "watch", "sleep", "date", "cal", "bc", "expr", "seq", "basename", 
            "dirname", "realpath", "md5sum", "sha1sum", "sha256sum", "base64", "od", "hexdump", "xxd", 
            "ddrescue", "sync", "chroot", "pivot_root", "capsh", "getcap", "setcap", "getfacl", "setfacl", 
            "logger", "logrotate", "cron", "crontab", "at", "batch", "wall", "write", "mesg", "talk", 
            "finger", "w", "who", "users", "tty", "stty", "tput", "reset", "factor", "units", "look", 
            "fold", "fmt", "pr", "nl", "comm", "tsort", "ptx", "m4", "yes", "true", "false", "test", "["
        ];

        for util in utilities {
            let link_path = bin_dir.join(util);
            #[cfg(target_os = "linux")]
            {
                let _ = std::os::unix::fs::symlink("/bin/kestrel", &link_path);
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = fs::write(&link_path, format!("MOCK_SYM_TO: /bin/kestrel"));
            }
        }
        println!("[Hatchling] Populated container symlinks: 200+ utilities -> /bin/kestrel");

        // 5.5 Detect and extract bundled .xshd disks inside the SquashFS
        if let Ok(entries) = fs::read_dir(&lower_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |ext| ext == "xshd") {
                    let disk_name = path.file_name().unwrap().to_string_lossy().to_string();
                    println!("[Hatchling] Bundled .xshd disk detected: {}", disk_name);
                    
                    let dest_disk_path = merged_dir.join(&disk_name);
                    println!("[Hatchling] Extracting bundled disk to {:?}", dest_disk_path);
                    if let Err(e) = fs::copy(&path, &dest_disk_path) {
                        println!("[Hatchling] Failed to copy .xshd: {}", e);
                    } else {
                        // Open and verify Beak FS superblock
                        if let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&dest_disk_path) {
                            if let Ok(beak_fs) = beak_fs::BeakFs::open(file) {
                                println!(
                                    "[Hatchling] Mounted beak://{} successfully. Free blocks: {}",
                                    disk_name, beak_fs.superblock.free_blocks_count
                                );
                            }
                        }
                    }
                }
            }
        }

        // 6. Map persistent named volumes
        for vol_map in &req.volumes {
            let parts: Vec<&str> = vol_map.split(':').collect();
            if parts.len() == 2 {
                let win_src = PathBuf::from(parts[0]);
                let lin_dest_rel = parts[1].trim_start_matches('/');
                let guest_src = translate_path(&win_src);
                let container_dest = merged_dir.join(lin_dest_rel);

                println!(
                    "[Hatchling] Mapping volume: {:?} -> {:?}",
                    guest_src, container_dest
                );

                fs::create_dir_all(&container_dest)?;
                
                nix::mount::mount(
                    Some(guest_src.to_str().unwrap()),
                    &container_dest,
                    None::<&str>,
                    nix::mount::MsFlags::MS_BIND,
                    None::<&str>,
                )?;
            }
        }

        // 7. Double-Fork and Namespace Isolation
        println!("[Hatchling] Performing namespace isolation...");
        match unsafe { nix::unistd::fork() } {
            Ok(nix::unistd::ForkResult::Parent { child }) => {
                // Parent monitors child process
                println!("[Hatchling] Monitor process active for child PID {}", child);
                let _ = nix::sys::wait::waitpid(child, None);
                println!("[Hatchling] Container exited. Initiating cleanup...");
                
                // Cleanup mounts in reverse order
                for vol_map in &req.volumes {
                    let parts: Vec<&str> = vol_map.split(':').collect();
                    if parts.len() == 2 {
                        let lin_dest_rel = parts[1].trim_start_matches('/');
                        let container_dest = merged_dir.join(lin_dest_rel);
                        let _ = nix::mount::umount(&container_dest);
                    }
                }
                let _ = nix::mount::umount(&merged_dir);
                let _ = nix::mount::umount(&upper_tmpfs);
                let _ = std::process::Command::new("umount").arg(&lower_dir).status();
                let _ = fs::remove_dir_all(&base_dir);
                println!("[Hatchling] Cleanup completed cleanly.");
            }
            Ok(nix::unistd::ForkResult::Child) => {
                // In child: unshare namespaces
                nix::sched::unshare(
                    nix::sched::CloneFlags::CLONE_NEWNS
                        | nix::sched::CloneFlags::CLONE_NEWPID
                        | nix::sched::CloneFlags::CLONE_NEWIPC
                        | nix::sched::CloneFlags::CLONE_NEWNET,
                ).expect("Failed to unshare namespaces");

                // Fork again to enter the new PID namespace
                match unsafe { nix::unistd::fork() } {
                    Ok(nix::unistd::ForkResult::Parent { child: _grandchild }) => {
                        // Intermediate process exits
                        std::process::exit(0);
                    }
                    Ok(nix::unistd::ForkResult::Child) => {
                        // Grandchild runs as PID 1 in the new PID namespace
                        println!("[Container] Chrooting into OverlayFS root...");
                        nix::unistd::chroot(&merged_dir).expect("chroot failed");
                        nix::unistd::chdir("/").expect("chdir failed");

                        // Mount a fresh /proc inside the namespace
                        fs::create_dir_all("/proc").ok();
                        let _ = nix::mount::mount(
                            Some("proc"),
                            "/proc",
                            Some("proc"),
                            nix::mount::MsFlags::empty(),
                            None::<&str>,
                        );

                        println!("[Container] Executing entry point: {}", meta.entry_point);
                        
                        // Exec application
                        let entry_path = std::ffi::CString::new(meta.entry_point.as_str()).unwrap();
                        let args = [entry_path.clone()];
                        let envs = [std::ffi::CString::new("PATH=/usr/bin:/bin").unwrap()];
                        
                        let _ = nix::unistd::execve(&entry_path, &args, &envs);
                        
                        // Fallback if execve fails
                        println!("[Container] Error: entry point execution failed.");
                        std::process::exit(1);
                    }
                    Err(_) => std::process::exit(1),
                }
            }
            Err(e) => {
                println!("[Hatchling] Fork failed: {}", e);
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Mock execution on Windows
        println!("[Hatchling] [Mock] Volume mappings:");
        for vol_map in &req.volumes {
            println!("[Hatchling] [Mock]   - Mount: {}", vol_map);
        }
        println!("[Hatchling] [Mock] Setup overlay filesystem: SquashFS lower + tmpfs upper");
        println!("[Hatchling] [Mock] Populated container symlinks: 200+ utilities -> /bin/kestrel");
        println!("[Hatchling] [Mock] Detecting bundled .xshd disks...");
        println!("[Hatchling] Bundled .xshd disk detected: data.xshd");
        println!("[Hatchling] Mounted beak://data.xshd successfully. Free blocks: 2500");
        println!("[Hatchling] [Mock] Isolation namespaces: CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWIPC | CLONE_NEWNET");
        println!("[Hatchling] [Mock] Simulating container execution (runs for 5 seconds)...");
        thread::sleep(Duration::from_secs(5));
        println!("[Hatchling] [Mock] Container completed. Tmpfs upper layer dropped. Slate reset.");
    }

    Ok(())
}

/// Translates a Windows file path to a guest Linux file path.
/// E.g. C:\Users\Account_2\Kestrel -> /mnt/c/Users/Account_2/Kestrel
#[cfg(target_os = "linux")]
fn translate_path(win_path: &Path) -> PathBuf {
    let win_str = win_path.to_string_lossy().to_string();
    if win_str.len() >= 2 && win_str.as_bytes()[1] == b':' {
        let drive = win_str.chars().next().unwrap().to_lowercase();
        let relative = &win_str[2..].replace('\\', "/");
        PathBuf::from(format!("/mnt/{}{}", drive, relative))
    } else {
        win_path.to_path_buf()
    }
}
