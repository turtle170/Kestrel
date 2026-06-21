//! kestrel-term: Instant terminal for the Kestrel OS Linux environment.
//!
//! This terminal connects to the Kestrel virtual serial port (COM1 pipe)
//! and provides a full interactive terminal session to the Kestrel Linux
//! guest OS. It automatically pops open when Kestrel boots.

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Stylize},
    terminal::{self, ClearType},
};
use log::{info, warn};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Named pipe path used for Kestrel serial console (COM1 bridge)
const KESTREL_PIPE: &str = r"\\.\pipe\KestrelSerial";

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    // Setup terminal
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();

    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        cursor::Show,
    )?;

    draw_banner(&mut stdout)?;

    // Try to connect to the Kestrel serial pipe
    let pipe_result = open_kestrel_pipe();

    let result = match pipe_result {
        Ok(pipe) => {
            info!("Connected to Kestrel serial console");
            run_terminal(pipe)
        }
        Err(e) => {
            // Not connected — show a friendly message and a stub shell
            warn!("Kestrel pipe not available: {}", e);
            run_stub_shell(&mut stdout)
        }
    };

    // Cleanup
    execute!(
        stdout,
        terminal::LeaveAlternateScreen,
    )?;
    terminal::disable_raw_mode()?;

    result
}

fn draw_banner(stdout: &mut impl Write) -> Result<()> {
    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
    )?;
    writeln!(stdout, "{}",
        "╔══════════════════════════════════════════════╗"
            .with(Color::Cyan)
    )?;
    writeln!(stdout, "{}",
        "║         Kestrel OS — Linux Terminal          ║"
            .with(Color::Cyan)
    )?;
    writeln!(stdout, "{}",
        "║  Connecting to Kestrel serial console...     ║"
            .with(Color::Cyan)
    )?;
    writeln!(stdout, "{}",
        "╚══════════════════════════════════════════════╝"
            .with(Color::Cyan)
    )?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_kestrel_pipe() -> Result<std::fs::File> {
    use std::fs::OpenOptions;
    // Wait up to 5 seconds for the pipe to become available
    for _ in 0..50 {
        let result = OpenOptions::new()
            .read(true)
            .write(true)
            .open(KESTREL_PIPE);
        match result {
            Ok(f) => return Ok(f),
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
    anyhow::bail!("Kestrel serial pipe not available after 5 seconds")
}

#[cfg(not(target_os = "windows"))]
fn open_kestrel_pipe() -> Result<std::fs::File> {
    // On Linux, connect to /dev/pts or a Unix socket
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/ttyS0")
        .context("Cannot open /dev/ttyS0")
}

fn run_terminal(mut pipe: std::fs::File) -> Result<()> {
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let stdout_clone = stdout.clone();
    let mut pipe_write = pipe.try_clone()?;

    #[cfg(target_os = "windows")]
    let raw_handle_val = {
        use std::os::windows::io::AsRawHandle;
        pipe.as_raw_handle() as isize
    };

    // Reader thread: pipe -> terminal display
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::System::Pipes::PeekNamedPipe;
                let handle = windows::Win32::Foundation::HANDLE(raw_handle_val as *mut std::ffi::c_void);
                let mut total_bytes_avail = 0u32;
                let res = unsafe {
                    PeekNamedPipe(
                        handle,
                        None,
                        0,
                        None,
                        Some(&mut total_bytes_avail),
                        None,
                    )
                };
                if res.is_err() {
                    break;
                }
                if total_bytes_avail == 0 {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            }

            use std::io::Read;
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut out = stdout_clone.lock().unwrap();
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
                Err(_) => break,
            }
        }
    });

    // Main thread: keyboard -> pipe
    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
                if kind == event::KeyEventKind::Release {
                    continue;
                }
                // Ctrl+Q to quit
                if code == KeyCode::Char('q') && modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                let bytes = key_to_bytes(code, modifiers);
                if !bytes.is_empty() {
                    pipe_write.write_all(&bytes)?;
                    pipe_write.flush()?;
                }
            }
        }
    }
    Ok(())
}

fn run_stub_shell(stdout: &mut impl Write) -> Result<()> {
    writeln!(stdout, "{}",
        " Kestrel is not running. Start kestrel.exe first."
            .with(Color::Yellow)
    )?;
    writeln!(stdout, "{}",
        " Press any key to exit."
            .with(Color::DarkGrey)
    )?;
    stdout.flush()?;
    // Wait for any key
    loop {
        if event::poll(Duration::from_millis(200))? {
            let _ = event::read()?;
            break;
        }
    }
    Ok(())
}

fn key_to_bytes(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
    match code {
        KeyCode::Char(c) => {
            let byte = c as u8;
            if modifiers.contains(KeyModifiers::CONTROL) && byte.is_ascii_alphabetic() {
                vec![byte.to_ascii_uppercase() - 64] // Ctrl+A = 0x01
            } else {
                vec![byte]
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7F],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1B],
        KeyCode::Up => vec![0x1B, b'[', b'A'],
        KeyCode::Down => vec![0x1B, b'[', b'B'],
        KeyCode::Right => vec![0x1B, b'[', b'C'],
        KeyCode::Left => vec![0x1B, b'[', b'D'],
        _ => vec![],
    }
}
