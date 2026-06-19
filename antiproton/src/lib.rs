//! Antiproton: Windows PE binary loader and NT syscall translator for Kestrel.
//!
//! Architecture:
//!   1. `PeLoader` — Parses a Windows PE/PE+ (x64) executable, maps sections
//!      into the Kestrel guest memory space.
//!   2. `SyscallTranslator` — WSL1-style NT syscall -> Linux syscall translation.
//!   3. `binfmt` integration — Kestrel kernel calls `can_handle()` to detect
//!      PE binaries, then `load_and_exec()` to run them.

pub mod pe;
pub mod syscall;
pub mod binfmt;

pub use pe::PeLoader;
pub use syscall::SyscallTranslator;
pub use binfmt::KestrelBinfmt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pe_detection() {
        let not_pe = [0u8; 10];
        assert!(!PeLoader::is_pe(&not_pe));

        let mut mock_mz = [0u8; 64];
        mock_mz[0] = 0x4D;
        mock_mz[1] = 0x5A; // MZ
        assert!(PeLoader::is_pe(&mock_mz));
    }

    #[test]
    fn test_syscall_translation() {
        use crate::syscall::TranslationResult;

        // Test NtWriteFile (0x0008) on stdout (-11)
        let res = SyscallTranslator::translate(0x0008, -11i64 as u64, 0x1000, 100, 0, 0);
        match res {
            TranslationResult::Call { linux_syscall, arg1, arg2, arg3 } => {
                assert_eq!(linux_syscall, 1); // Linux Write
                assert_eq!(arg1, 1);          // stdout fd
                assert_eq!(arg2, 0x1000);
                assert_eq!(arg3, 100);
            }
            _ => panic!("Expected Call translation"),
        }

        // Test NtReadFile (0x0006) on stdin (-10)
        let res = SyscallTranslator::translate(0x0006, -10i64 as u64, 0x2000, 50, 0, 0);
        match res {
            TranslationResult::Call { linux_syscall, arg1, arg2, arg3 } => {
                assert_eq!(linux_syscall, 0); // Linux Read
                assert_eq!(arg1, 0);          // stdin fd
                assert_eq!(arg2, 0x2000);
                assert_eq!(arg3, 50);
            }
            _ => panic!("Expected Call translation"),
        }

        // Test NtTerminateProcess (0x002C)
        let res = SyscallTranslator::translate(0x002C, 0, 42, 0, 0, 0);
        match res {
            TranslationResult::Call { linux_syscall, arg1, .. } => {
                assert_eq!(linux_syscall, 60); // Linux Exit
                assert_eq!(arg1, 42);          // exit code
            }
            _ => panic!("Expected Call translation"),
        }
    }
}
