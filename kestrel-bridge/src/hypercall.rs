//! Kestrel hypercall interface.

pub use crate::KestrelHypercall;

/// Issue a raw hypercall by writing to the Kestrel I/O port.
/// In a real kernel module this would use inline assembly.
#[inline(always)]
pub fn hypercall(code: KestrelHypercall) {
    let _port = crate::KESTREL_IO_BASE;
    let _val = code as u16;
    // In the actual Linux kernel module this becomes:
    // unsafe { core::arch::asm!("outw %ax, %dx", in("dx") _port, in("ax") _val, options(att_syntax)); }
}
