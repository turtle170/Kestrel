//! IPC bridge between Kestrel Linux and the Windows host.
//! Handles I/O port interception from WHPX, MMIO fault handling from UML,
//! and terminal process spawning.

use anyhow::{Result, Context};
use log::{debug, info, warn};
use std::process::Command;
use std::path::PathBuf;
use std::thread;
use std::io::Write;
use std::fs::OpenOptions;

#[cfg(target_os = "windows")]
use windows::Win32::System::Hypervisor::WHV_X64_IO_PORT_ACCESS_CONTEXT;

// ─── Terminal ────────────────────────────────────────────────────────────────

/// Attempt to launch `kestrel-term.exe` from the same directory as this executable.
pub fn spawn_terminal() {
    let result = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("kestrel-term.exe")));

    match result {
        Some(path) if path.exists() => {
            info!("[IPC] Spawning Kestrel Terminal: {:?}", path);
            match Command::new(&path).spawn() {
                Ok(_) => info!("[IPC] kestrel-term.exe launched successfully"),
                Err(e) => warn!("[IPC] Failed to launch kestrel-term.exe: {}", e),
            }
        }
        Some(path) => {
            warn!(
                "[IPC] kestrel-term.exe not found at {:?} — terminal will not auto-open",
                path
            );
        }
        None => {
            warn!("[IPC] Could not determine executable path for terminal launch");
        }
    }
}

// ─── WHPX I/O Port Handler ───────────────────────────────────────────────────

/// Handle an I/O port access exit delivered by the WHPX virtual processor.
#[cfg(target_os = "windows")]
pub fn handle_io_port(ctx: WHV_X64_IO_PORT_ACCESS_CONTEXT) -> Option<u32> {
    let port = ctx.PortNumber;

    match port {
        // COM1 UART data register
        0x3F8 => {
            let is_write = (unsafe { ctx.AccessInfo.AsUINT32 } & 0x01) != 0;
            if is_write {
                let byte = (ctx.Rax & 0xFF) as u8;
                info!("[IPC] COM1 TX: '{}' ({})", byte as char, byte);
                let mut written = false;
                if let Ok(mut guard) = SERIAL_PIPE_STREAM.lock() {
                    if let Some(ref mut file) = *guard {
                        if file.write_all(&[byte]).is_ok() {
                            written = true;
                        }
                    }
                }
                
                // Throttle guest execution
                if !written {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                None
            } else {
                // Read from serial buffer
                let mut byte = 0u8;
                if let Ok(mut queue) = SERIAL_INPUT_QUEUE.lock() {
                    if !queue.is_empty() {
                        byte = queue.remove(0);
                    }
                }
                Some(byte as u32)
            }
        }
        // COM1 Line Status Register (LSR)
        0x3FD => {
            let is_write = (unsafe { ctx.AccessInfo.AsUINT32 } & 0x01) != 0;
            if !is_write {
                let mut status = 0x60u8; // Transmitter empty (ready to write)
                if let Ok(queue) = SERIAL_INPUT_QUEUE.lock() {
                    if !queue.is_empty() {
                        status |= 0x01; // Data Ready (ready to read)
                    }
                }
                Some(status as u32)
            } else {
                None
            }
        }
        // ISA debug port — safe to ignore
        0x0080 => { None }
        _ => {
            debug!("[IPC] Unhandled I/O port 0x{:04x}", port);
            None
        }
    }
}

// Non-Windows stub so the module compiles on all targets
#[cfg(not(target_os = "windows"))]
pub fn handle_io_port(_ctx: ()) -> Option<u32> { None }

// ─── UML MMIO Handler ────────────────────────────────────────────────────────

/// Handle an MMIO access fault intercepted by the UML VEH.
/// `gpa` is the guest physical address that triggered the fault.
pub fn handle_mmio_access(gpa: u64) {
    debug!("[IPC] MMIO access at GPA 0x{:016x}", gpa);

    match gpa {
        // Legacy VGA text/graphics framebuffer
        0x000A_0000..=0x000B_FFFF => {
            debug!("[IPC] VGA framebuffer write at 0x{:016x} — forwarding to Windows GDI (stub)", gpa);
            // TODO: translate VGA writes to Win32 GDI / DirectX surface updates
        }
        // VGA BIOS ROM
        0x000C_0000..=0x000C_7FFF => {
            debug!("[IPC] VGA BIOS ROM access (read-only, ignored)");
        }
        // ACPI MMIO range (placeholder)
        0xFEC0_0000..=0xFECF_FFFF => {
            debug!("[IPC] ACPI MMIO at 0x{:016x} (stub)", gpa);
        }
        // Local APIC (xAPIC default base)
        0xFEE0_0000..=0xFEEF_FFFF => {
            debug!("[IPC] Local APIC MMIO at 0x{:016x} (stub)", gpa);
        }
        _ => {
            warn!("[IPC] Unhandled MMIO at GPA 0x{:016x}", gpa);
        }
    }
}

// ─── Hatchling Orchestration ─────────────────────────────────────────────────

const CONTROL_PIPE: &str = r"\\.\pipe\kestrel-control";
const CONTROL_PORT: &str = "0.0.0.0:9002";

static GUEST_STREAM: std::sync::OnceLock<std::sync::Mutex<Option<std::net::TcpStream>>> = std::sync::OnceLock::new();

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct HatchRequest {
    pub app_path: PathBuf,
    pub volumes: Vec<String>,
}

/// Run by `kestrel-host hatch ...` to send the command to the background VM daemon.
pub fn send_hatch_request(app: PathBuf, volumes: Vec<String>) -> Result<()> {
    // Convert relative app_path to absolute
    let app_abs = if app.is_absolute() {
        app
    } else {
        std::env::current_dir()?.join(app)
    };

    let req = HatchRequest {
        app_path: app_abs,
        volumes,
    };
    
    let json = serde_json::to_string(&req)?;
    
    let mut pipe = OpenOptions::new()
        .write(true)
        .open(CONTROL_PIPE)
        .context("Failed to connect to Kestrel host daemon. Is the Kestrel VM running?")?;
    
    pipe.write_all(json.as_bytes())?;
    pipe.flush()?;
    Ok(())
}

/// Run by the main `kestrel-host` daemon to listen for Hatchling commands from the CLI.
pub fn start_control_listener() {
    start_serial_server();
    // 1. Spawn TCP control socket server for Kestrel guest (kestrel-init)
    thread::spawn(|| {
        info!("[IPC] Guest control TCP server starting on {}", CONTROL_PORT);
        let listener = match std::net::TcpListener::bind(CONTROL_PORT) {
            Ok(l) => l,
            Err(e) => {
                warn!("[IPC] Failed to bind TCP control port 9002: {}", e);
                return;
            }
        };

        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    if let Ok(addr) = s.peer_addr() {
                        info!("[IPC] Guest control connection established from {}", addr);
                    }
                    
                    let mutex = GUEST_STREAM.get_or_init(|| std::sync::Mutex::new(None));
                    if let Ok(mut guard) = mutex.lock() {
                        if let Ok(clone_stream) = s.try_clone() {
                            *guard = Some(clone_stream);
                        }
                    }

                    // Read replies or status updates from guest in a separate loop
                    let mutex_ref = GUEST_STREAM.get().unwrap();
                    let mut read_buf = [0u8; 1024];
                    loop {
                        use std::io::Read;
                        match s.read(&mut read_buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let reply = String::from_utf8_lossy(&read_buf[..n]);
                                info!("[Orchestration] Received status update from guest: {}", reply.trim());
                            }
                            Err(_) => break,
                        }
                    }

                    info!("[IPC] Guest control connection disconnected.");
                    if let Ok(mut guard) = mutex_ref.lock() {
                        *guard = None;
                    }
                }
                Err(e) => warn!("[IPC] TCP accept error: {}", e),
            }
        }
    });

    // 2. Spawn Windows Named Pipe server for CLI orchestration command forwarding
    thread::spawn(|| {
        info!("[IPC] Named pipe control listener starting on {}", CONTROL_PIPE);

        #[cfg(target_os = "windows")]
        {
            use windows::core::HSTRING;
            use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_READMODE_BYTE, PIPE_WAIT, ConnectNamedPipe, DisconnectNamedPipe};
            use windows::Win32::Storage::FileSystem::{ReadFile, PIPE_ACCESS_DUPLEX};
            use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_PIPE_CONNECTED};
            
            unsafe {
                let pipe_name = HSTRING::from(CONTROL_PIPE);
                loop {
                    let pipe = CreateNamedPipeW(
                        &pipe_name,
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        1, // Max instances
                        4096,
                        4096,
                        0,
                        None,
                    );
                    
                    if pipe.is_invalid() {
                        warn!("[IPC] Failed to create control named pipe.");
                        return;
                    }
                    
                    let connected = ConnectNamedPipe(pipe, None);
                    if connected.is_ok() || GetLastError() == ERROR_PIPE_CONNECTED {
                        let mut buf = [0u8; 4096];
                        let mut bytes_read = 0;
                        if ReadFile(pipe, Some(&mut buf), Some(&mut bytes_read), None).is_ok() {
                            if bytes_read > 0 {
                                let msg = String::from_utf8_lossy(&buf[..bytes_read as usize]);
                                if let Ok(req) = serde_json::from_str::<HatchRequest>(&msg) {
                                    info!("[Orchestration] Received hatch request for app: {:?}", req.app_path);
                                    info!("[Orchestration] Volume maps: {:?}", req.volumes);
                                    
                                    // Forward to guest via TCP stream
                                    let mut forwarded = false;
                                    if let Some(mutex) = GUEST_STREAM.get() {
                                        if let Ok(mut guard) = mutex.lock() {
                                            if let Some(ref mut stream) = *guard {
                                                if stream.write_all(msg.as_bytes()).is_ok() && stream.flush().is_ok() {
                                                    forwarded = true;
                                                }
                                            }
                                        }
                                    }
                                    
                                    if forwarded {
                                        info!("[Orchestration] Forwarded hatch command to guest.");
                                    } else {
                                        warn!("[Orchestration] Guest is not connected. Hatch request could not be forwarded.");
                                    }
                                } else {
                                    warn!("[Orchestration] Invalid hatch request received.");
                                }
                            }
                        }
                    }
                    let _ = DisconnectNamedPipe(pipe);
                    let _ = CloseHandle(pipe);
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            warn!("[IPC] Control listener not supported on non-Windows targets.");
        }
    });
}

// ─── Serial Named Pipe Server ────────────────────────────────────────────────

static SERIAL_PIPE_STREAM: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
static SERIAL_INPUT_QUEUE: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

#[cfg(target_os = "windows")]
pub fn start_serial_server() {
    thread::spawn(|| {
        use windows::core::HSTRING;
        use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_READMODE_BYTE, PIPE_WAIT, ConnectNamedPipe, DisconnectNamedPipe, PeekNamedPipe};
        use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED};
        use std::os::windows::io::FromRawHandle;

        let pipe_name = HSTRING::from(r"\\.\pipe\KestrelSerial");
        unsafe {
            loop {
                let pipe = CreateNamedPipeW(
                    &pipe_name,
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    1, // Max instances
                    4096,
                    4096,
                    0,
                    None,
                );
                
                if pipe.is_invalid() {
                    thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
                
                let connected = ConnectNamedPipe(pipe, None);
                if connected.is_ok() || GetLastError() == ERROR_PIPE_CONNECTED {
                    info!("[IPC] Kestrel terminal connected to serial pipe");
                    
                    let file = std::fs::File::from_raw_handle(pipe.0 as *mut std::ffi::c_void);
                    
                    if let Ok(mut guard) = SERIAL_PIPE_STREAM.lock() {
                        *guard = Some(file.try_clone().unwrap());
                    }
                    
                    let mut file_read = file;
                    let mut buf = [0u8; 1024];
                    loop {
                        let mut total_bytes_avail = 0u32;
                        let res = PeekNamedPipe(
                            pipe,
                            None,
                            0,
                            None,
                            Some(&mut total_bytes_avail),
                            None,
                        );
                        if res.is_err() {
                            break;
                        }
                        if total_bytes_avail == 0 {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                        
                        use std::io::Read;
                        match file_read.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if let Ok(mut queue) = SERIAL_INPUT_QUEUE.lock() {
                                    queue.extend_from_slice(&buf[..n]);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    
                    info!("[IPC] Kestrel terminal disconnected from serial pipe");
                }
                
                if let Ok(mut guard) = SERIAL_PIPE_STREAM.lock() {
                    *guard = None;
                }
                let _ = DisconnectNamedPipe(pipe);
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn start_serial_server() {}
