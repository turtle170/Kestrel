//! Reverse-Proton: Windows PE binary loader and NT syscall translator for Kestrel.
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
