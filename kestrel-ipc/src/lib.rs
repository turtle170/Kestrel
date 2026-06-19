//! kestrel-ipc: High-performance IPC between Kestrel Linux guest and Windows host.
//!
//! Uses a Windows Named Pipe as the primary channel, carrying:
//!   - Raw Ethernet frames for networking
//!   - Serial console data for kestrel-term
//!   - Framed messages for graphics/block I/O
//!
//! On the Linux side (running inside the Kestrel guest), this module
//! provides a virtual network device that reads/writes to the shared
//! memory ring buffers exposed by kestrel-bridge.

use anyhow::{Result, Context, bail};
use crossbeam_channel::{bounded, Sender, Receiver};
use log::{info, warn, debug};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Named pipe for Kestrel serial console (kestrel-term).
pub const KESTREL_SERIAL_PIPE: &str = r"\\.\pipe\KestrelSerial";
/// Named pipe for Kestrel network packets.
pub const KESTREL_NET_PIPE: &str = r"\\.\pipe\KestrelNet";

/// A framed message over the IPC channel.
/// Frame format: [4 bytes type][4 bytes length][length bytes payload]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Serial  = 0x01,
    NetTx   = 0x02,
    NetRx   = 0x03,
    Graphics = 0x04,
    Control  = 0xFF,
}

#[derive(Debug, Clone)]
pub struct IpcFrame {
    pub frame_type: u32,
    pub payload: Vec<u8>,
}

/// Windows Named Pipe server for the Kestrel IPC bridge.
pub struct KestrelIpcServer {
    serial_pipe_path: String,
    net_pipe_path: String,
}

impl KestrelIpcServer {
    pub fn new() -> Self {
        Self {
            serial_pipe_path: KESTREL_SERIAL_PIPE.to_string(),
            net_pipe_path: KESTREL_NET_PIPE.to_string(),
        }
    }

    /// Start the IPC server. Spawns background threads for serial and network pipes.
    pub fn start(&self) -> Result<IpcHandle> {
        let (serial_tx, serial_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(256);
        let (net_tx, net_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(256);

        let serial_pipe = self.serial_pipe_path.clone();
        let net_pipe = self.net_pipe_path.clone();
        let stx = serial_tx.clone();
        let ntx = net_tx.clone();

        // Serial pipe server thread
        thread::spawn(move || {
            if let Err(e) = run_serial_server(&serial_pipe, stx) {
                warn!("[IPC] Serial server error: {}", e);
            }
        });

        // Network pipe server thread
        thread::spawn(move || {
            if let Err(e) = run_net_server(&net_pipe, ntx) {
                warn!("[IPC] Net server error: {}", e);
            }
        });

        info!("[IPC] Kestrel IPC server started");
        info!("[IPC]   Serial pipe: {}", KESTREL_SERIAL_PIPE);
        info!("[IPC]   Network pipe: {}", KESTREL_NET_PIPE);

        Ok(IpcHandle { serial_rx, net_rx })
    }
}

/// Handle returned by KestrelIpcServer::start() for receiving IPC messages.
pub struct IpcHandle {
    pub serial_rx: Receiver<Vec<u8>>,
    pub net_rx: Receiver<Vec<u8>>,
}

/// Run the serial console named pipe server.
fn run_serial_server(pipe_path: &str, tx: Sender<Vec<u8>>) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::Pipes::*;
        use windows::Win32::Foundation::*;
        use windows::Win32::Storage::FileSystem::*;
        use windows::core::HSTRING;

        info!("[IPC] Creating serial pipe: {}", pipe_path);
        let pipe_hstr = HSTRING::from(pipe_path);

        let handle = unsafe {
            CreateNamedPipeW(
                &pipe_hstr,
                FILE_FLAGS_AND_ATTRIBUTES(0x00000003), // PIPE_ACCESS_DUPLEX
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,      // max instances
                65536,  // out buffer size
                65536,  // in buffer size
                0,      // default timeout
                None,
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            bail!("CreateNamedPipeW failed for serial pipe");
        }

        loop {
            info!("[IPC] Waiting for kestrel-term to connect to serial pipe...");
            unsafe { ConnectNamedPipe(handle, None) }
                .context("ConnectNamedPipe failed")?;
            info!("[IPC] kestrel-term connected to serial console");

            let mut buf = vec![0u8; 1024];
            loop {
                let mut bytes_read = 0u32;
                let ok = unsafe {
                    ReadFile(
                        handle,
                        Some(buf.as_mut_slice()),
                        Some(&mut bytes_read),
                        None,
                    )
                };
                if ok.is_err() || bytes_read == 0 {
                    info!("[IPC] kestrel-term disconnected from serial pipe");
                    unsafe { DisconnectNamedPipe(handle) }.ok();
                    break;
                }
                let _ = tx.send(buf[..bytes_read as usize].to_vec());
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // On Linux: open /dev/ttyS0 for loopback testing
        use std::fs::OpenOptions;
        let mut dev = OpenOptions::new().read(true).write(true).open("/dev/ttyS0")
            .context("Cannot open /dev/ttyS0")?;
        let mut buf = vec![0u8; 256];
        loop {
            match dev.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => { let _ = tx.send(buf[..n].to_vec()); }
                Err(e) => { warn!("[IPC] Serial read error: {}", e); break; }
            }
        }
        Ok(())
    }
}

/// Run the network named pipe server.
fn run_net_server(pipe_path: &str, tx: Sender<Vec<u8>>) -> Result<()> {
    info!("[IPC] Network pipe stub (platform-specific implementation pending)");
    // Placeholder: In production this creates a TAP-like interface
    // that bridges packets between Linux TCP/IP stack and Windows WFP.
    // For now, use a TCP loopback socket for cross-platform dev testing.
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:9001")
        .context("Cannot bind net IPC port 9001")?;
    info!("[IPC] Network IPC listening on 127.0.0.1:9001");

    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                let tx2 = tx.clone();
                thread::spawn(move || {
                    let mut buf = vec![0u8; 1500]; // Ethernet MTU
                    loop {
                        match s.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => { let _ = tx2.send(buf[..n].to_vec()); }
                            Err(_) => break,
                        }
                    }
                });
            }
            Err(e) => warn!("[IPC] Net accept error: {}", e),
        }
    }
    Ok(())
}

impl Default for KestrelIpcServer {
    fn default() -> Self {
        Self::new()
    }
}
