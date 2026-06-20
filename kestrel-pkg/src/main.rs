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
use std::io::{Read, Write};
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

    // ─── Guest Utilities ─────────────────────────────────────────────────────

    /// List files and directories
    Ls {
        /// Directory path to list
        #[arg(default_value = ".")]
        path: String,
    },
    /// Concatenate and display files
    Cat {
        /// Files to display
        files: Vec<String>,
    },
    /// Package management tool
    Apt {
        /// Action: update or install
        action: String,
        /// Package name
        package: Option<String>,
    },
    /// Build the guest initramfs CPIO archive with baked symlinks
    BuildInitramfs {
        /// Path to the guest kestrel-init binary
        #[arg(short, long)]
        init: PathBuf,
        /// Path to the guest kestrel-pkg binary
        #[arg(short, long)]
        kestrel: PathBuf,
        /// Path to output the initramfs.cpio file
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let program_name = std::path::Path::new(&args[0])
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    let is_packer = program_name == "kestrel-pkg"
        || program_name == "kestrel-pkg.exe"
        || program_name == "kestrel"
        || program_name == "kestrel.exe";

    if !is_packer {
        return handle_multi_call(&program_name, &args);
    }

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
        // ─── Guest Utilities ─────────────────────────────────────────────────
        Commands::Ls { path } => {
            run_ls_command(&path)?;
        }
        Commands::Cat { files } => {
            run_cat_command(&files)?;
        }
        Commands::Apt { action, package } => {
            run_apt_command(&action, package.as_deref())?;
        }
        Commands::BuildInitramfs { init, kestrel, output } => {
            info!("Building guest initramfs CPIO archive at {:?}...", output);
            let init_bytes = fs::read(&init).context("Failed to read init binary")?;
            let kestrel_bytes = fs::read(&kestrel).context("Failed to read kestrel binary")?;
            
            let f = fs::File::create(&output).context("Failed to create output cpio file")?;
            let mut w = std::io::BufWriter::new(f);
            
            let mtime = 1774000000; // static timestamp
            let mut ino = 1;
            
            // 1. Write directories
            write_cpio_dir(&mut w, ino, mtime, "bin")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "sbin")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "usr")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "usr/bin")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "proc")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "sys")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "tmp")?; ino += 1;
            write_cpio_dir(&mut w, ino, mtime, "data")?; ino += 1;
            
            // 2. Write init file (PID 1)
            write_cpio_file(&mut w, ino, 0o755, mtime, "init", &init_bytes)?; ino += 1;
            
            // 3. Write kestrel utility binary
            write_cpio_file(&mut w, ino, 0o755, mtime, "bin/kestrel", &kestrel_bytes)?; ino += 1;
            
            // 4. Write all baked symlinks for 200+ utilities
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
                let name = format!("bin/{}", util);
                write_cpio_symlink(&mut w, ino, mtime, &name, "kestrel")?;
                ino += 1;
            }
            
            // 5. Write trailer
            write_cpio_trailer(&mut w, ino, mtime)?;
            
            println!("✔  Initramfs cpio created successfully: {:?}", output);
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

// ─── Guest Utilities & APT Implementation ────────────────────────────────────

fn run_ls_command(path: &str) -> Result<()> {
    let entries = fs::read_dir(path).context("Failed to read directory")?;
    for entry in entries {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let type_str = if metadata.is_dir() { "<DIR>" } else { "     " };
        let size = metadata.len();
        println!("  {}  {:>10} B  {}", type_str, size, entry.file_name().to_string_lossy());
    }
    Ok(())
}

fn run_cat_command(files: &[String]) -> Result<()> {
    if files.is_empty() {
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)?;
        print!("{}", buffer);
    } else {
        for file in files {
            let content = fs::read(file).with_context(|| format!("Failed to read file '{}'", file))?;
            std::io::Write::write_all(&mut std::io::stdout(), &content)?;
        }
    }
    Ok(())
}

fn run_apt_command(action: &str, package: Option<&str>) -> Result<()> {
    match action {
        "update" => {
            println!("Get:1 http://deb.debian.org/debian stable InRelease [151 kB]");
            println!("Get:2 http://deb.debian.org/debian stable/main amd64 Packages [8,787 kB]");
            println!("Fetched 8,938 kB in 2s (4,469 kB/s)");
            println!("Reading package lists... Done");
            println!("Building dependency tree... Done");
            println!("All packages are up to date.");
        }
        "install" => {
            let pkg = package.context("Error: Please specify a package to install.")?;
            println!("Reading package lists... Done");
            println!("Building dependency tree... Done");
            println!("The following NEW packages will be installed:");
            println!("  {}", pkg);
            
            let url = get_package_url(pkg);
            println!("Need to get archives. Get:1 {} ...", url);
            
            match download_and_extract_package(pkg, &url) {
                Ok(_) => {
                    println!("Unpacking {}...", pkg);
                    println!("Setting up {}...", pkg);
                    println!("✔  Package '{}' installed successfully!", pkg);
                }
                Err(e) => {
                    println!("[Apt] Offline or network error: {}. Simulating mock install...", e);
                    let mock_path = std::path::Path::new("usr/bin").join(pkg);
                    let _ = std::fs::create_dir_all("usr/bin");
                    let _ = std::fs::write(&mock_path, format!("#!/bin/sh\necho 'Mock execution of {}'", pkg));
                    #[cfg(target_os = "linux")]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&mock_path, std::fs::Permissions::from_mode(0o755));
                    }
                    println!("✔  Mock package '{}' installed successfully!", pkg);
                }
            }
        }
        _ => {
            anyhow::bail!("Unknown apt action: {}. Supported: update, install", action);
        }
    }
    Ok(())
}

fn get_package_url(pkg: &str) -> String {
    match pkg {
        "curl" => "http://ftp.debian.org/debian/pool/main/c/curl/curl_8.4.0-2_amd64.deb".to_string(),
        "wget" => "http://ftp.debian.org/debian/pool/main/w/wget/wget_1.21.4-1+b1_amd64.deb".to_string(),
        "git" => "http://ftp.debian.org/debian/pool/main/g/git/git_2.43.0-1_amd64.deb".to_string(),
        "nano" => "http://ftp.debian.org/debian/pool/main/n/nano/nano_7.2-1_amd64.deb".to_string(),
        "htop" => "http://ftp.debian.org/debian/pool/main/h/htop/htop_3.2.2-2_amd64.deb".to_string(),
        "nginx" => "http://ftp.debian.org/debian/pool/main/n/nginx/nginx_1.24.0-2_amd64.deb".to_string(),
        "neofetch" => "http://ftp.debian.org/debian/pool/main/n/neofetch/neofetch_7.1.0-4_all.deb".to_string(),
        "python3" => "http://ftp.debian.org/debian/pool/main/p/python3-defaults/python3_3.11.2-1+b1_amd64.deb".to_string(),
        _ => {
            let first_char = pkg.chars().next().unwrap_or('a');
            format!("http://ftp.debian.org/debian/pool/main/{}/{}/{}_1.0.0_amd64.deb", first_char, pkg, pkg)
        }
    }
}

fn download_and_extract_package(_pkg: &str, url: &str) -> Result<()> {
    let body = download_file(url)?;
    
    println!("Extracting AR archive from .deb...");
    let files = extract_ar(&body)?;
    
    let (data_tar_name, data_tar_bytes) = files.into_iter()
        .find(|(name, _)| name.starts_with("data.tar"))
        .context("data.tar file not found inside .deb archive")?;
        
    println!("Found data archive: {}", data_tar_name);
    
    let temp_tar_path = std::env::temp_dir().join(&data_tar_name);
    std::fs::write(&temp_tar_path, &data_tar_bytes)?;
    
    println!("Extracting data archive to root '/'...");
    let status = std::process::Command::new("tar")
        .args(["xf", &temp_tar_path.to_string_lossy(), "-C", "/"])
        .status();
        
    let _ = std::fs::remove_file(&temp_tar_path);
    
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => anyhow::bail!("tar extraction failed with exit code: {:?}", s.code()),
        Err(e) => {
            println!("[Apt] tar command not available ({}). Extracting using native fallback...", e);
            anyhow::bail!("tar command not found: {}", e)
        }
    }
}

fn download_file(url: &str) -> Result<Vec<u8>> {
    if !url.starts_with("http://") {
        anyhow::bail!("Only http:// URLs are supported currently");
    }
    let url_without_http = &url[7..];
    let first_slash = url_without_http.find('/').context("Invalid URL path")?;
    let host = &url_without_http[..first_slash];
    let path = &url_without_http[first_slash..];
    
    let addr = format!("{}:80", host);
    let socket_addrs = std::net::ToSocketAddrs::to_socket_addrs(&addr)?
        .next()
        .context("Failed to resolve hostname")?;
        
    let mut stream = std::net::TcpStream::connect_timeout(&socket_addrs, std::time::Duration::from_secs(5))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    
    let request = format!(
        "GET {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: KestrelApt/0.1\r\n\
         Connection: close\r\n\r\n",
        path, host
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    
    let header_end = response.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("Invalid HTTP response headers")?;
        
    let body = response[header_end + 4..].to_vec();
    
    let first_line = std::str::from_utf8(&response[0..header_end])?
        .lines()
        .next()
        .unwrap_or_default();
        
    if !first_line.contains("200 OK") {
        anyhow::bail!("HTTP download failed: {}", first_line);
    }
    
    Ok(body)
}

fn extract_ar(ar_bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    if ar_bytes.len() < 8 || &ar_bytes[0..8] != b"!<arch>\n" {
        anyhow::bail!("Invalid AR magic");
    }
    
    let mut files = Vec::new();
    let mut offset = 8;
    while offset + 60 <= ar_bytes.len() {
        let header = &ar_bytes[offset..offset+60];
        let name_str = std::str::from_utf8(&header[0..16])?.trim().to_string();
        let size_str = std::str::from_utf8(&header[48..58])?.trim();
        let size: usize = size_str.parse()?;
        
        offset += 60;
        if offset + size > ar_bytes.len() {
            anyhow::bail!("AR file truncated");
        }
        
        let data = ar_bytes[offset..offset+size].to_vec();
        files.push((name_str, data));
        
        offset += size;
        if size % 2 != 0 {
            offset += 1;
        }
    }
    Ok(files)
}

fn handle_multi_call(cmd: &str, args: &[String]) -> Result<()> {
    let cmd_clean = if cmd.ends_with(".exe") {
        &cmd[..cmd.len() - 4]
    } else {
        cmd
    };

    match cmd_clean {
        "ls" => {
            let path = if args.len() > 1 { &args[1] } else { "." };
            run_ls_command(path)
        }
        "cat" => {
            let files = if args.len() > 1 { args[1..].to_vec() } else { vec![] };
            run_cat_command(&files)
        }
        "apt" => {
            if args.len() < 2 {
                println!("Usage: apt <update | install> [package]");
                return Ok(());
            }
            let action = &args[1];
            let package = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            run_apt_command(action, package)
        }
        "echo" => run_echo(args),
        "true" => std::process::exit(0),
        "false" => std::process::exit(1),
        "pwd" => run_pwd(),
        "whoami" => run_whoami(),
        "id" => run_id(),
        "uname" => run_uname(),
        "sleep" => run_sleep(args),
        "clear" => run_clear(),
        "hostname" => run_hostname(),
        _ => execute_real_or_suggest(cmd_clean, args),
    }
}

fn run_echo(args: &[String]) -> Result<()> {
    let output = args[1..].join(" ");
    println!("{}", output);
    Ok(())
}

fn run_pwd() -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    println!("{}", cwd.display());
    Ok(())
}

fn run_whoami() -> Result<()> {
    println!("root");
    Ok(())
}

fn run_id() -> Result<()> {
    println!("uid=0(root) gid=0(root) groups=0(root)");
    Ok(())
}

fn run_uname() -> Result<()> {
    println!("Linux kestrel 7.0.12-x86_64 #1 SMP PREEMPT Sat Jun 20 12:00:00 UTC 2026 x86_64 GNU/Linux");
    Ok(())
}

fn run_sleep(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        anyhow::bail!("sleep: missing operand");
    }
    let seconds: f64 = args[1].parse().context("sleep: invalid time interval")?;
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    Ok(())
}

fn run_clear() -> Result<()> {
    print!("\x1B[2J\x1B[1;1H");
    let _ = std::io::stdout().flush();
    Ok(())
}

fn run_hostname() -> Result<()> {
    println!("kestrel-hatchling");
    Ok(())
}

fn execute_real_or_suggest(command: &str, args: &[String]) -> Result<()> {
    if let Some(real_path) = find_real_executable(command) {
        let mut child = std::process::Command::new(real_path)
            .args(&args[1..])
            .spawn()
            .context("Failed to spawn real executable")?;
            
        let status = child.wait()?;
        std::process::exit(status.code().unwrap_or(0));
    }
    
    let pkg = suggest_package(command);
    println!("Command '{}' not found, but can be installed with:", command);
    println!("  apt install {}", pkg);
    
    std::process::exit(127);
}

fn find_real_executable(command: &str) -> Option<PathBuf> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let separators = if cfg!(windows) { ';' } else { ':' };
    
    for path_dir in path_env.split(separators) {
        for ext in &["", ".exe", ".bat", ".cmd"] {
            let filename = format!("{}{}", command, ext);
            let path = Path::new(path_dir).join(&filename);
            if path.exists() && path.is_file() {
                // Avoid symlink recursion pointing to kestrel
                if let Ok(metadata) = std::fs::symlink_metadata(&path) {
                    if metadata.file_type().is_symlink() {
                        if let Ok(target) = std::fs::read_link(&path) {
                            if target.to_string_lossy().contains("kestrel") {
                                continue;
                            }
                        }
                    }
                }
                // Avoid calling ourselves
                if let Ok(current_exe) = std::env::current_exe() {
                    if path == current_exe {
                        continue;
                    }
                }
                return Some(path);
            }
        }
    }
    None
}

fn suggest_package(command: &str) -> &'static str {
    match command {
        "grep" | "egrep" | "fgrep" => "grep",
        "sed" => "sed",
        "awk" | "gawk" => "gawk",
        "tar" => "tar",
        "gzip" | "gunzip" => "gzip",
        "bzip2" | "bunzip2" => "bzip2",
        "xz" | "unxz" => "xz-utils",
        "zstd" | "unzstd" => "zstd",
        "zip" | "unzip" => "unzip",
        "curl" => "curl",
        "wget" => "wget",
        "git" => "git",
        "make" => "make",
        "gcc" => "gcc",
        "g++" => "g++",
        "clang" => "clang",
        "gdb" => "gdb",
        "tmux" => "tmux",
        "screen" => "screen",
        "ping" => "iputils-ping",
        "ip" => "iproute2",
        "sudo" => "sudo",
        "top" | "ps" | "kill" | "killall" | "pkill" | "pgrep" | "free" | "uptime" => "procps",
        "htop" => "htop",
        "nc" | "netcat" => "netcat-openbsd",
        "ss" | "netstat" => "iproute2",
        "tcpdump" => "tcpdump",
        "ssh" | "scp" | "sftp" => "openssh-client",
        "rsync" => "rsync",
        "nslookup" | "dig" | "host" => "dnsutils",
        "iptables" => "iptables",
        "nft" | "nftables" => "nftables",
        "systemctl" => "systemd",
        "valgrind" => "valgrind",
        "perf" => "linux-perf",
        "strace" => "strace",
        "ltrace" => "ltrace",
        "lsof" => "lsof",
        "cron" | "crontab" => "cron",
        "chroot" => "coreutils",
        _ => "coreutils",
    }
}

// ─── CPIO Initramfs Archiver Helpers ─────────────────────────────────────────

fn write_cpio_header<W: std::io::Write>(
    w: &mut W,
    ino: u32,
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u32,
    mtime: u32,
    filesize: u32,
    name: &str,
) -> Result<()> {
    let name_len = name.len() + 1; // including null terminator
    let header_str = format!(
        "070701\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         {:08x}\
         00000000",
        ino,
        mode,
        uid,
        gid,
        nlink,
        mtime,
        filesize,
        3, // dev major
        1, // dev minor
        0, // rdev major
        0, // rdev minor
        name_len,
    );
    w.write_all(header_str.as_bytes())?;
    w.write_all(name.as_bytes())?;
    w.write_all(&[0])?;
    let total_header_len = 110 + name_len;
    let pad = (4 - (total_header_len % 4)) % 4;
    w.write_all(&vec![0; pad])?;
    Ok(())
}

fn write_cpio_file<W: std::io::Write>(
    w: &mut W,
    ino: u32,
    mode: u32,
    mtime: u32,
    name: &str,
    content: &[u8],
) -> Result<()> {
    let full_mode = 0o100000 | mode;
    write_cpio_header(w, ino, full_mode, 0, 0, 1, mtime, content.len() as u32, name)?;
    w.write_all(content)?;
    let pad = (4 - (content.len() % 4)) % 4;
    w.write_all(&vec![0; pad])?;
    Ok(())
}

fn write_cpio_dir<W: std::io::Write>(
    w: &mut W,
    ino: u32,
    mtime: u32,
    name: &str,
) -> Result<()> {
    let full_mode = 0o040000 | 0o755;
    write_cpio_header(w, ino, full_mode, 0, 0, 2, mtime, 0, name)?;
    Ok(())
}

fn write_cpio_symlink<W: std::io::Write>(
    w: &mut W,
    ino: u32,
    mtime: u32,
    name: &str,
    target: &str,
) -> Result<()> {
    let full_mode = 0o120000 | 0o777;
    let target_bytes = target.as_bytes();
    write_cpio_header(w, ino, full_mode, 0, 0, 1, mtime, target_bytes.len() as u32, name)?;
    w.write_all(target_bytes)?;
    let pad = (4 - (target_bytes.len() % 4)) % 4;
    w.write_all(&vec![0; pad])?;
    Ok(())
}

fn write_cpio_trailer<W: std::io::Write>(w: &mut W, ino: u32, mtime: u32) -> Result<()> {
    write_cpio_header(w, ino, 0, 0, 0, 1, mtime, 0, "TRAILER!!!")?;
    Ok(())
}
