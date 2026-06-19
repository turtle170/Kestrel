//! WSL1-style Windows NT syscall -> Linux syscall translation table.
//!
//! When a Windows .exe running under Kestrel makes a syscall, the Kestrel
//! kernel traps it (via a custom int 0x2E / syscall handler) and dispatches
//! it here. We translate the NT syscall number and arguments to the nearest
//! Linux equivalent and invoke it.

use log::{debug, warn};

/// NT syscall numbers (x64)
#[allow(non_camel_case_types)]
#[repr(u32)]
pub enum NtSyscall {
    NtReadFile         = 0x0006,
    NtWriteFile        = 0x0008,
    NtClose            = 0x000F,
    NtCreateFile       = 0x0055,
    NtQuerySystemTime  = 0x005A,
    NtAllocateVirtualMemory = 0x0018,
    NtFreeVirtualMemory     = 0x001E,
    NtCreateProcess    = 0x004B,
    NtTerminateProcess = 0x002C,
    NtWaitForSingleObject   = 0x0004,
    NtQueryInformationProcess = 0x0019,
}

/// Linux syscall numbers (x86_64)
#[allow(non_camel_case_types)]
#[repr(u64)]
pub enum LinuxSyscall {
    Read       = 0,
    Write      = 1,
    Open       = 2,
    Close      = 3,
    Mmap       = 9,
    Munmap     = 11,
    Brk        = 12,
    Getpid     = 39,
    Fork       = 57,
    Execve     = 59,
    Exit       = 60,
    Wait4      = 61,
    ClockGettime = 228,
}

/// Result of syscall translation
#[derive(Debug)]
pub enum TranslationResult {
    /// Call the given Linux syscall number with translated args
    Call { linux_syscall: u64, arg1: u64, arg2: u64, arg3: u64 },
    /// Return a constant value (no Linux syscall needed)
    ReturnConst(u64),
    /// Syscall not yet implemented
    Unimplemented { nt_syscall: u32 },
}

pub struct SyscallTranslator;

impl SyscallTranslator {
    /// Translate an NT syscall into the appropriate Linux action.
    pub fn translate(
        nt_syscall: u32,
        arg1: u64, arg2: u64, arg3: u64, arg4: u64, _arg5: u64,
    ) -> TranslationResult {
        debug!("[NT->Linux] syscall 0x{:04x} ({}, {}, {})", nt_syscall, arg1, arg2, arg3);

        match nt_syscall {
            // NtReadFile -> read(fd, buf, count)
            0x0006 => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Read as u64,
                arg1: Self::handle_to_fd(arg1),
                arg2, arg3,
            },

            // NtWriteFile -> write(fd, buf, count)
            0x0008 => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Write as u64,
                arg1: Self::handle_to_fd(arg1),
                arg2, arg3,
            },

            // NtClose -> close(fd)
            0x000F => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Close as u64,
                arg1: Self::handle_to_fd(arg1),
                arg2: 0, arg3: 0,
            },

            // NtCreateFile -> open(path, flags, mode)
            0x0055 => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Open as u64,
                arg1, arg2: Self::translate_access_flags(arg2), arg3: 0o644,
            },

            // NtAllocateVirtualMemory -> mmap(addr, size, ...)
            0x0018 => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Mmap as u64,
                arg1: arg2, // BaseAddress
                arg2: arg4, // RegionSize
                arg3: 3,    // PROT_READ | PROT_WRITE
            },

            // NtFreeVirtualMemory -> munmap(addr, size)
            0x001E => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Munmap as u64,
                arg1: arg2, // BaseAddress
                arg2: arg4, // RegionSize
                arg3: 0,
            },

            // NtTerminateProcess -> exit(code)
            0x002C => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Exit as u64,
                arg1: arg2, // ExitStatus
                arg2: 0, arg3: 0,
            },

            // NtQuerySystemTime -> clock_gettime(CLOCK_REALTIME, tp)
            0x005A => TranslationResult::Call {
                linux_syscall: LinuxSyscall::ClockGettime as u64,
                arg1: 0, // CLOCK_REALTIME
                arg2, arg3: 0,
            },

            // NtWaitForSingleObject -> wait4(pid, ...)
            0x0004 => TranslationResult::Call {
                linux_syscall: LinuxSyscall::Wait4 as u64,
                arg1, arg2, arg3,
            },

            other => {
                warn!("[NT->Linux] Unimplemented NT syscall 0x{:04x}", other);
                TranslationResult::Unimplemented { nt_syscall: other }
            }
        }
    }

    /// Convert a Windows HANDLE to a Linux file descriptor.
    /// Windows pseudo-handles: STD_INPUT=-10, STD_OUTPUT=-11, STD_ERROR=-12
    fn handle_to_fd(handle: u64) -> u64 {
        match handle as i64 {
            -10 => 0, // stdin
            -11 => 1, // stdout
            -12 => 2, // stderr
            h => h as u64, // Pass-through for real handles mapped to FDs
        }
    }

    /// Translate Windows access flags to Linux open(2) flags.
    fn translate_access_flags(access: u64) -> u64 {
        const GENERIC_READ:    u64 = 0x80000000;
        const GENERIC_WRITE:   u64 = 0x40000000;
        let mut flags: u64 = 0;
        if (access & GENERIC_READ) != 0 && (access & GENERIC_WRITE) != 0 {
            flags |= 2; // O_RDWR
        } else if (access & GENERIC_WRITE) != 0 {
            flags |= 1; // O_WRONLY
        } else {
            flags |= 0; // O_RDONLY
        }
        flags
    }
}
