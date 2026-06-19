//! Shared memory ring buffer implementation for Kestrel Bridge IPC.
//!
//! Layout of each ring buffer:
//!   [0]     Head index (u32, atomic)
//!   [4]     Tail index (u32, atomic)
//!   [8...]  Data region (capacity - 8 bytes)

use core::sync::atomic::{AtomicU32, Ordering};

pub struct SharedRingBuffer {
    base: *mut u8,
    capacity: usize,
}

unsafe impl Send for SharedRingBuffer {}
unsafe impl Sync for SharedRingBuffer {}

impl SharedRingBuffer {
    pub unsafe fn new(base: *mut u8, capacity: usize) -> Self {
        Self { base, capacity }
    }

    #[inline]
    fn head_ptr(&self) -> *mut AtomicU32 {
        self.base as *mut AtomicU32
    }

    #[inline]
    fn tail_ptr(&self) -> *mut AtomicU32 {
        unsafe { (self.base as *mut AtomicU32).add(1) }
    }

    #[inline]
    fn data_ptr(&self) -> *mut u8 {
        unsafe { self.base.add(8) }
    }

    #[inline]
    fn data_capacity(&self) -> usize {
        self.capacity - 8
    }

    /// Write data into the ring buffer. Returns false if full.
    pub fn write(&mut self, data: &[u8]) -> bool {
        let cap = self.data_capacity();
        let head = unsafe { (*self.head_ptr()).load(Ordering::Acquire) } as usize;
        let tail = unsafe { (*self.tail_ptr()).load(Ordering::Acquire) } as usize;

        // Need: 4 bytes for length prefix + data
        let needed = 4 + data.len();
        let used = if tail >= head { tail - head } else { cap - head + tail };
        if used + needed >= cap {
            return false; // full
        }

        // Write length prefix (u32 LE)
        let len = data.len() as u32;
        let len_bytes = len.to_le_bytes();
        self.write_bytes(&len_bytes, tail, cap);

        let tail2 = (tail + 4) % cap;
        self.write_bytes(data, tail2, cap);

        let new_tail = (tail2 + data.len()) % cap;
        unsafe {
            (*self.tail_ptr()).store(new_tail as u32, Ordering::Release);
        }
        true
    }

    /// Read data from the ring buffer into buf. Returns Some(len) or None if empty.
    pub fn read(&mut self, buf: &mut [u8]) -> Option<usize> {
        let cap = self.data_capacity();
        let head = unsafe { (*self.head_ptr()).load(Ordering::Acquire) } as usize;
        let tail = unsafe { (*self.tail_ptr()).load(Ordering::Acquire) } as usize;

        if head == tail {
            return None; // empty
        }

        // Read length prefix
        let mut len_bytes = [0u8; 4];
        self.read_bytes(&mut len_bytes, head, cap);
        let len = u32::from_le_bytes(len_bytes) as usize;

        if len > buf.len() {
            return None; // buffer too small
        }

        let head2 = (head + 4) % cap;
        self.read_bytes(&mut buf[..len], head2, cap);

        let new_head = (head2 + len) % cap;
        unsafe {
            (*self.head_ptr()).store(new_head as u32, Ordering::Release);
        }
        Some(len)
    }

    fn write_bytes(&self, data: &[u8], offset: usize, cap: usize) {
        let ptr = self.data_ptr();
        for (i, &b) in data.iter().enumerate() {
            let idx = (offset + i) % cap;
            unsafe { ptr.add(idx).write_volatile(b); }
        }
    }

    fn read_bytes(&self, buf: &mut [u8], offset: usize, cap: usize) {
        let ptr = self.data_ptr();
        for (i, slot) in buf.iter_mut().enumerate() {
            let idx = (offset + i) % cap;
            *slot = unsafe { ptr.add(idx).read_volatile() };
        }
    }
}
