//! Beak FS: A custom, ultra-fast, low-overhead filesystem for Kestrel OS.
//! Accompanied by the .xshd virtual hard disk (Sparse File) wrapper.

use std::fs::File;
use std::io::{Read, Write, Seek, SeekFrom, Result, Error, ErrorKind};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};

pub const BLOCK_SIZE: usize = 4096;
pub const BEAK_MAGIC: &[u8; 8] = b"BEAKFS\0\0";
pub const XSHD_MAGIC: &[u8; 8] = b"KSTLXSHD";

// ─── Sparse File Helper ──────────────────────────────────────────────────────

/// Marks a file as sparse under Windows. On Unix, this is a no-op.
#[allow(unused_variables)]
pub fn make_file_sparse(file: &File) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::io::AsRawHandle;
        
        type HANDLE = *mut std::ffi::c_void;
        type DWORD = u32;
        type BOOL = i32;
        type LPVOID = *mut std::ffi::c_void;
        type LPOVERLAPPED = *mut std::ffi::c_void;

        extern "system" {
            fn DeviceIoControl(
                hDevice: HANDLE,
                dwIoControlCode: DWORD,
                lpInBuffer: LPVOID,
                nInBufferSize: DWORD,
                lpOutBuffer: LPVOID,
                nOutBufferSize: DWORD,
                lpBytesReturned: *mut DWORD,
                lpOverlapped: LPOVERLAPPED,
            ) -> BOOL;
        }

        const FSCTL_SET_SPARSE: DWORD = 0x000900C4;

        let handle = file.as_raw_handle() as HANDLE;
        let mut bytes_returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_SPARSE,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(Error::last_os_error());
        }
    }
    Ok(())
}

// ─── Structures ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Superblock {
    pub magic: [u8; 8],
    pub block_size: u32,
    pub total_blocks: u64,
    pub inode_bitmap_block: u64,
    pub data_bitmap_block: u64,
    pub inode_table_block: u64,
    pub data_blocks_start: u64,
    pub inodes_count: u32,
    pub free_inodes_count: u32,
    pub free_blocks_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inode {
    pub mode: u16, // 0x4000 = Dir, 0x8000 = Regular File
    pub size: u64,
    pub mtime: u64,
    pub block_pointers: [u64; 12],
    pub indirect_pointer: u64,
}

impl Inode {
    pub fn new(mode: u16) -> Self {
        Self {
            mode,
            size: 0,
            mtime: 0,
            block_pointers: [0; 12],
            indirect_pointer: 0,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        buf.write_u16::<LE>(self.mode).unwrap();
        buf.write_u64::<LE>(self.size).unwrap();
        buf.write_u64::<LE>(self.mtime).unwrap();
        for &ptr in &self.block_pointers {
            buf.write_u64::<LE>(ptr).unwrap();
        }
        buf.write_u64::<LE>(self.indirect_pointer).unwrap();
        buf.resize(128, 0); // pad to 128 bytes
        buf
    }

    pub fn deserialize(mut data: &[u8]) -> Result<Self> {
        let mode = data.read_u16::<LE>()?;
        let size = data.read_u64::<LE>()?;
        let mtime = data.read_u64::<LE>()?;
        let mut block_pointers = [0u64; 12];
        for i in 0..12 {
            block_pointers[i] = data.read_u64::<LE>()?;
        }
        let indirect_pointer = data.read_u64::<LE>()?;
        Ok(Self {
            mode,
            size,
            mtime,
            block_pointers,
            indirect_pointer,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub inode: u32,
    pub file_type: u8, // 1 = file, 2 = dir
    pub name: String,
}

impl DirEntry {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32);
        buf.write_u32::<LE>(self.inode).unwrap();
        buf.write_u8(self.file_type).unwrap();
        let name_bytes = self.name.as_bytes();
        let name_len = name_bytes.len().min(26);
        buf.write_u8(name_len as u8).unwrap();
        buf.write_all(&name_bytes[..name_len]).unwrap();
        buf.resize(32, 0); // pad to exactly 32 bytes
        buf
    }

    pub fn deserialize(mut data: &[u8]) -> Result<Self> {
        let inode = data.read_u32::<LE>()?;
        let file_type = data.read_u8()?;
        let name_len = data.read_u8()? as usize;
        let mut name_buf = vec![0u8; name_len];
        data.read_exact(&mut name_buf)?;
        let name = String::from_utf8_lossy(&name_buf).into_owned();
        Ok(Self { inode, file_type, name })
    }
}

// ─── Filesystem Driver ──────────────────────────────────────────────────────

pub struct BeakFs {
    file: File,
    pub superblock: Superblock,
    disk_offset: u64, // Start offset of the filesystem inside the .xshd image (32 bytes header)
}

impl BeakFs {
    /// Format an image file as a sparse .xshd disk containing a Beak FS.
    pub fn format(mut file: File, total_size: u64) -> Result<Self> {
        if total_size < 1024 * 1024 {
            return Err(Error::new(ErrorKind::InvalidInput, "Disk size must be at least 1MB"));
        }

        // 1. Initialize sparse file on host
        make_file_sparse(&file)?;
        file.set_len(total_size)?;

        // 2. Write .xshd header (32 bytes)
        file.seek(SeekFrom::Start(0))?;
        file.write_all(XSHD_MAGIC)?;
        file.write_u16::<LE>(1)?; // version
        file.write_u64::<LE>(total_size)?; // disk size
        file.write_u32::<LE>(BLOCK_SIZE as u32)?; // block size
        file.write_all(&[0u8; 10])?; // padding

        let disk_offset = 32u64;

        // 3. Calculate blocks layout
        let blocks_count = (total_size - disk_offset) / BLOCK_SIZE as u64;
        let inodes_count = 128u32; // Fixed number of inodes for light VM use

        let inode_bitmap_block = 1u64; // Block 0 is superblock, Block 1 is inode bitmap
        let data_bitmap_block = 2u64;  // Block 2 is data block bitmap
        
        let inode_table_size_bytes = inodes_count as u64 * 128;
        let inode_table_blocks = (inode_table_size_bytes + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64;
        let inode_table_block = 3u64;

        let data_blocks_start = inode_table_block + inode_table_blocks;
        let free_blocks_count = blocks_count - data_blocks_start;

        let sb = Superblock {
            magic: *BEAK_MAGIC,
            block_size: BLOCK_SIZE as u32,
            total_blocks: blocks_count,
            inode_bitmap_block,
            data_bitmap_block,
            inode_table_block,
            data_blocks_start,
            inodes_count,
            free_inodes_count: inodes_count - 1, // inode 0 is root dir
            free_blocks_count,
        };

        let mut fs = Self {
            file,
            superblock: sb,
            disk_offset,
        };

        // Write superblock
        fs.write_superblock()?;

        // Clear bitmaps and inode table area with sparse blocks (writing zeros)
        fs.clear_block(fs.superblock.inode_bitmap_block)?;
        fs.clear_block(fs.superblock.data_bitmap_block)?;
        for i in 0..inode_table_blocks {
            fs.clear_block(fs.superblock.inode_table_block + i)?;
        }

        // Initialize root directory (inode 0)
        let root_inode = Inode::new(0x4000); // Directory
        fs.write_inode(0, &root_inode)?;

        // Allocate the root dir's first data block
        let root_dir_block = fs.allocate_block()?;
        let mut root_inode = fs.read_inode(0)?;
        root_inode.block_pointers[0] = root_dir_block;
        root_inode.size = 0; // Starts empty
        fs.write_inode(0, &root_inode)?;

        // Update inode bitmap to mark inode 0 as used
        fs.write_bitmap_bit(fs.superblock.inode_bitmap_block, 0, true)?;

        Ok(fs)
    }

    /// Open an existing .xshd file containing Beak FS.
    pub fn open(mut file: File) -> Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;
        if magic != *XSHD_MAGIC {
            return Err(Error::new(ErrorKind::InvalidData, "Not a valid .xshd disk image"));
        }

        let disk_offset = 32u64;
        file.seek(SeekFrom::Start(disk_offset))?;
        
        let mut sb_magic = [0u8; 8];
        file.read_exact(&mut sb_magic)?;
        if sb_magic != *BEAK_MAGIC {
            return Err(Error::new(ErrorKind::InvalidData, "Not a valid Beak FS filesystem"));
        }

        let block_size = file.read_u32::<LE>()?;
        let total_blocks = file.read_u64::<LE>()?;
        let inode_bitmap_block = file.read_u64::<LE>()?;
        let data_bitmap_block = file.read_u64::<LE>()?;
        let inode_table_block = file.read_u64::<LE>()?;
        let data_blocks_start = file.read_u64::<LE>()?;
        let inodes_count = file.read_u32::<LE>()?;
        let free_inodes_count = file.read_u32::<LE>()?;
        let free_blocks_count = file.read_u64::<LE>()?;

        let sb = Superblock {
            magic: sb_magic,
            block_size,
            total_blocks,
            inode_bitmap_block,
            data_bitmap_block,
            inode_table_block,
            data_blocks_start,
            inodes_count,
            free_inodes_count,
            free_blocks_count,
        };

        Ok(Self {
            file,
            superblock: sb,
            disk_offset,
        })
    }

    // ─── Direct Block I/O ────────────────────────────────────────────────────────

    pub fn read_block(&mut self, block_idx: u64, buf: &mut [u8]) -> Result<()> {
        let offset = self.disk_offset + block_idx * BLOCK_SIZE as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    pub fn write_block(&mut self, block_idx: u64, buf: &[u8]) -> Result<()> {
        let offset = self.disk_offset + block_idx * BLOCK_SIZE as u64;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(buf)?;
        Ok(())
    }

    fn clear_block(&mut self, block_idx: u64) -> Result<()> {
        let buf = [0u8; BLOCK_SIZE];
        self.write_block(block_idx, &buf)
    }

    // ─── Superblock & Metadata Helpers ──────────────────────────────────────────

    fn write_superblock(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(self.disk_offset))?;
        self.file.write_all(&self.superblock.magic)?;
        self.file.write_u32::<LE>(self.superblock.block_size)?;
        self.file.write_u64::<LE>(self.superblock.total_blocks)?;
        self.file.write_u64::<LE>(self.superblock.inode_bitmap_block)?;
        self.file.write_u64::<LE>(self.superblock.data_bitmap_block)?;
        self.file.write_u64::<LE>(self.superblock.inode_table_block)?;
        self.file.write_u64::<LE>(self.superblock.data_blocks_start)?;
        self.file.write_u32::<LE>(self.superblock.inodes_count)?;
        self.file.write_u32::<LE>(self.superblock.free_inodes_count)?;
        self.file.write_u64::<LE>(self.superblock.free_blocks_count)?;
        Ok(())
    }

    pub fn read_inode(&mut self, inode_idx: u32) -> Result<Inode> {
        if inode_idx >= self.superblock.inodes_count {
            return Err(Error::new(ErrorKind::InvalidInput, "Inode index out of range"));
        }
        let byte_offset = self.disk_offset 
            + self.superblock.inode_table_block * BLOCK_SIZE as u64 
            + (inode_idx as u64 * 128);

        self.file.seek(SeekFrom::Start(byte_offset))?;
        let mut buf = [0u8; 128];
        self.file.read_exact(&mut buf)?;
        Inode::deserialize(&buf)
    }

    pub fn write_inode(&mut self, inode_idx: u32, inode: &Inode) -> Result<()> {
        if inode_idx >= self.superblock.inodes_count {
            return Err(Error::new(ErrorKind::InvalidInput, "Inode index out of range"));
        }
        let byte_offset = self.disk_offset 
            + self.superblock.inode_table_block * BLOCK_SIZE as u64 
            + (inode_idx as u64 * 128);

        self.file.seek(SeekFrom::Start(byte_offset))?;
        let buf = inode.serialize();
        self.file.write_all(&buf)?;
        Ok(())
    }

    // ─── Bitmap Allocation ──────────────────────────────────────────────────────

    fn read_bitmap_bit(&mut self, bitmap_block: u64, bit_idx: u32) -> Result<bool> {
        let byte_offset = self.disk_offset + bitmap_block * BLOCK_SIZE as u64 + (bit_idx / 8) as u64;
        self.file.seek(SeekFrom::Start(byte_offset))?;
        let byte = self.file.read_u8()?;
        Ok((byte & (1 << (bit_idx % 8))) != 0)
    }

    fn write_bitmap_bit(&mut self, bitmap_block: u64, bit_idx: u32, val: bool) -> Result<()> {
        let byte_offset = self.disk_offset + bitmap_block * BLOCK_SIZE as u64 + (bit_idx / 8) as u64;
        self.file.seek(SeekFrom::Start(byte_offset))?;
        let mut byte = self.file.read_u8()?;
        if val {
            byte |= 1 << (bit_idx % 8);
        } else {
            byte &= !(1 << (bit_idx % 8));
        }
        self.file.seek(SeekFrom::Start(byte_offset))?;
        self.file.write_all(&[byte])?;
        Ok(())
    }

    pub fn allocate_block(&mut self) -> Result<u64> {
        let limit = self.superblock.total_blocks - self.superblock.data_blocks_start;
        for i in 0..limit {
            if !self.read_bitmap_bit(self.superblock.data_bitmap_block, i as u32)? {
                self.write_bitmap_bit(self.superblock.data_bitmap_block, i as u32, true)?;
                let allocated_block = self.superblock.data_blocks_start + i;
                self.clear_block(allocated_block)?;
                self.superblock.free_blocks_count -= 1;
                self.write_superblock()?;
                return Ok(allocated_block);
            }
        }
        Err(Error::new(ErrorKind::WriteZero, "No free blocks available on Beak FS"))
    }

    pub fn allocate_inode(&mut self) -> Result<u32> {
        for i in 0..self.superblock.inodes_count {
            if !self.read_bitmap_bit(self.superblock.inode_bitmap_block, i)? {
                self.write_bitmap_bit(self.superblock.inode_bitmap_block, i, true)?;
                self.superblock.free_inodes_count -= 1;
                self.write_superblock()?;
                return Ok(i);
            }
        }
        Err(Error::new(ErrorKind::WriteZero, "No free inodes available on Beak FS"))
    }

    // ─── Directory Operations ───────────────────────────────────────────────────

    pub fn lookup(&mut self, parent_inode_idx: u32, name: &str) -> Result<Option<u32>> {
        let entries = self.list_dir(parent_inode_idx)?;
        for entry in entries {
            if entry.name == name {
                return Ok(Some(entry.inode));
            }
        }
        Ok(None)
    }

    pub fn resolve_path(&mut self, path: &str) -> Result<u32> {
        let clean_path = path.trim_start_matches('/');
        if clean_path.is_empty() {
            return Ok(0); // Root inode
        }

        let mut current_inode = 0;
        for component in clean_path.split('/') {
            if component.is_empty() {
                continue;
            }
            match self.lookup(current_inode, component)? {
                Some(next) => current_inode = next,
                None => return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("Component '{}' not found in path '{}'", component, path),
                )),
            }
        }
        Ok(current_inode)
    }

    pub fn list_dir(&mut self, inode_idx: u32) -> Result<Vec<DirEntry>> {
        let inode = self.read_inode(inode_idx)?;
        if inode.mode & 0x4000 == 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "Not a directory inode"));
        }

        let mut entries = Vec::new();

        // Traverse direct blocks
        for i in 0..12 {
            let block_ptr = inode.block_pointers[i];
            if block_ptr == 0 {
                continue;
            }

            let mut block_buf = [0u8; BLOCK_SIZE];
            self.read_block(block_ptr, &mut block_buf)?;

            // Each block has BLOCK_SIZE / 32 entries = 128 entries
            for chunk in block_buf.chunks_exact(32) {
                if chunk[0..4] == [0, 0, 0, 0] {
                    continue; // Empty directory slot
                }
                entries.push(DirEntry::deserialize(chunk)?);
            }
        }

        // Traverse single indirect block if used
        if inode.indirect_pointer != 0 {
            let mut indirect_buf = [0u8; BLOCK_SIZE];
            self.read_block(inode.indirect_pointer, &mut indirect_buf)?;

            let mut cursor = std::io::Cursor::new(&indirect_buf[..]);
            for _ in 0..512 {
                let block_ptr = cursor.read_u64::<LE>()?;
                if block_ptr == 0 {
                    continue;
                }

                let mut block_buf = [0u8; BLOCK_SIZE];
                self.read_block(block_ptr, &mut block_buf)?;

                for chunk in block_buf.chunks_exact(32) {
                    if chunk[0..4] == [0, 0, 0, 0] {
                        continue;
                    }
                    entries.push(DirEntry::deserialize(chunk)?);
                }
            }
        }

        Ok(entries)
    }

    pub fn add_dir_entry(&mut self, parent_inode_idx: u32, name: &str, inode_num: u32, file_type: u8) -> Result<()> {
        let mut parent_inode = self.read_inode(parent_inode_idx)?;
        if parent_inode.mode & 0x4000 == 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "Not a directory"));
        }

        let new_entry = DirEntry {
            inode: inode_num,
            file_type,
            name: name.to_string(),
        };
        let entry_buf = new_entry.serialize();

        // 1. Try to find an empty slot in existing blocks
        for i in 0..12 {
            let mut block_ptr = parent_inode.block_pointers[i];
            if block_ptr == 0 {
                // Allocate a block for the directory
                block_ptr = self.allocate_block()?;
                parent_inode.block_pointers[i] = block_ptr;
            }

            let mut block_buf = [0u8; BLOCK_SIZE];
            self.read_block(block_ptr, &mut block_buf)?;

            for (idx, chunk) in block_buf.chunks_exact_mut(32).enumerate() {
                if chunk[0..4] == [0, 0, 0, 0] {
                    // Found empty slot! Copy entry in
                    chunk.copy_from_slice(&entry_buf);
                    self.write_block(block_ptr, &block_buf)?;

                    // Update parent directory size if we expanded it
                    let size_needed = (i as u64 * BLOCK_SIZE as u64) + (idx as u64 + 1) * 32;
                    if parent_inode.size < size_needed {
                        parent_inode.size = size_needed;
                    }
                    self.write_inode(parent_inode_idx, &parent_inode)?;
                    return Ok(());
                }
            }
        }

        // 2. Try single indirect block
        if parent_inode.indirect_pointer == 0 {
            parent_inode.indirect_pointer = self.allocate_block()?;
            self.write_inode(parent_inode_idx, &parent_inode)?;
        }

        let mut indirect_buf = [0u8; BLOCK_SIZE];
        self.read_block(parent_inode.indirect_pointer, &mut indirect_buf)?;

        let mut ptrs = vec![0u64; 512];
        let mut cursor = std::io::Cursor::new(&indirect_buf[..]);
        for i in 0..512 {
            ptrs[i] = cursor.read_u64::<LE>()?;
        }

        for i in 0..512 {
            let mut block_ptr = ptrs[i];
            if block_ptr == 0 {
                block_ptr = self.allocate_block()?;
                ptrs[i] = block_ptr;
                
                // Write back updated indirect pointers block
                let mut out_cursor = std::io::Cursor::new(&mut indirect_buf[..]);
                for &p in &ptrs {
                    out_cursor.write_u64::<LE>(p)?;
                }
                self.write_block(parent_inode.indirect_pointer, &indirect_buf)?;
            }

            let mut block_buf = [0u8; BLOCK_SIZE];
            self.read_block(block_ptr, &mut block_buf)?;

            for (idx, chunk) in block_buf.chunks_exact_mut(32).enumerate() {
                if chunk[0..4] == [0, 0, 0, 0] {
                    chunk.copy_from_slice(&entry_buf);
                    self.write_block(block_ptr, &block_buf)?;

                    let size_needed = 12 * BLOCK_SIZE as u64 + (i as u64 * BLOCK_SIZE as u64) + (idx as u64 + 1) * 32;
                    if parent_inode.size < size_needed {
                        parent_inode.size = size_needed;
                    }
                    self.write_inode(parent_inode_idx, &parent_inode)?;
                    return Ok(());
                }
            }
        }

        Err(Error::new(ErrorKind::StorageFull, "Directory size limit exceeded"))
    }

    // ─── File Operations ────────────────────────────────────────────────────────

    pub fn create_file(&mut self, parent_inode_idx: u32, name: &str, is_dir: bool) -> Result<u32> {
        if self.lookup(parent_inode_idx, name)?.is_some() {
            return Err(Error::new(ErrorKind::AlreadyExists, "File or directory already exists"));
        }

        let new_inode_num = self.allocate_inode()?;
        let mode = if is_dir { 0x4000 } else { 0x8000 };
        let mut new_inode = Inode::new(mode);
        new_inode.mtime = chrono::Utc::now().timestamp() as u64;

        if is_dir {
            // Allocate initial block for new directory
            let dir_block = self.allocate_block()?;
            new_inode.block_pointers[0] = dir_block;
        }

        self.write_inode(new_inode_num, &new_inode)?;

        // Add to parent directory entries
        let file_type = if is_dir { 2 } else { 1 };
        self.add_dir_entry(parent_inode_idx, name, new_inode_num, file_type)?;

        Ok(new_inode_num)
    }

    pub fn write_file_data(&mut self, inode_idx: u32, data: &[u8]) -> Result<()> {
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode & 0x8000 == 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "Cannot write to non-regular file"));
        }

        // Clean existing allocations for overwrite simplicity
        self.free_inode_blocks(&inode)?;
        inode.size = 0;
        inode.block_pointers = [0; 12];
        inode.indirect_pointer = 0;

        let total_blocks_needed = (data.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
        if total_blocks_needed > 12 + 512 {
            return Err(Error::new(ErrorKind::FileTooLarge, "File size exceeds Beak FS 2MB limit"));
        }

        // 1. Write direct blocks
        let mut bytes_written = 0;
        for i in 0..12.min(total_blocks_needed) {
            let block_ptr = self.allocate_block()?;
            inode.block_pointers[i] = block_ptr;

            let start = bytes_written;
            let end = (start + BLOCK_SIZE).min(data.len());
            let mut block_buf = [0u8; BLOCK_SIZE];
            block_buf[..end - start].copy_from_slice(&data[start..end]);
            
            self.write_block(block_ptr, &block_buf)?;
            bytes_written = end;
        }

        // 2. Write single indirect block if needed
        if total_blocks_needed > 12 {
            let indirect_block = self.allocate_block()?;
            inode.indirect_pointer = indirect_block;

            let mut ptrs = vec![0u64; 512];
            let indirect_blocks_needed = total_blocks_needed - 12;

            for i in 0..indirect_blocks_needed {
                let block_ptr = self.allocate_block()?;
                ptrs[i] = block_ptr;

                let start = bytes_written;
                let end = (start + BLOCK_SIZE).min(data.len());
                let mut block_buf = [0u8; BLOCK_SIZE];
                block_buf[..end - start].copy_from_slice(&data[start..end]);

                self.write_block(block_ptr, &block_buf)?;
                bytes_written = end;
            }

            // Write pointers array to indirect block
            let mut indirect_buf = [0u8; BLOCK_SIZE];
            let mut cursor = std::io::Cursor::new(&mut indirect_buf[..]);
            for &p in &ptrs {
                cursor.write_u64::<LE>(p)?;
            }
            self.write_block(indirect_block, &indirect_buf)?;
        }

        inode.size = data.len() as u64;
        inode.mtime = chrono::Utc::now().timestamp() as u64;
        self.write_inode(inode_idx, &inode)?;

        Ok(())
    }

    pub fn read_file_data(&mut self, inode_idx: u32) -> Result<Vec<u8>> {
        let inode = self.read_inode(inode_idx)?;
        if inode.mode & 0x8000 == 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "Cannot read from non-regular file"));
        }

        let mut data = Vec::with_capacity(inode.size as usize);
        let total_blocks = (inode.size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE;

        let mut bytes_read = 0;

        // 1. Read direct blocks
        for i in 0..12.min(total_blocks) {
            let block_ptr = inode.block_pointers[i];
            if block_ptr == 0 {
                break;
            }
            let mut block_buf = [0u8; BLOCK_SIZE];
            self.read_block(block_ptr, &mut block_buf)?;

            let start = bytes_read;
            let end = (start + BLOCK_SIZE).min(inode.size as usize);
            data.extend_from_slice(&block_buf[..end - start]);
            bytes_read = end;
        }

        // 2. Read single indirect block
        if total_blocks > 12 && inode.indirect_pointer != 0 {
            let mut indirect_buf = [0u8; BLOCK_SIZE];
            self.read_block(inode.indirect_pointer, &mut indirect_buf)?;

            let mut cursor = std::io::Cursor::new(&indirect_buf[..]);
            let indirect_blocks_count = total_blocks - 12;

            for _ in 0..indirect_blocks_count {
                let block_ptr = cursor.read_u64::<LE>()?;
                if block_ptr == 0 {
                    break;
                }
                let mut block_buf = [0u8; BLOCK_SIZE];
                self.read_block(block_ptr, &mut block_buf)?;

                let start = bytes_read;
                let end = (start + BLOCK_SIZE).min(inode.size as usize);
                data.extend_from_slice(&block_buf[..end - start]);
                bytes_read = end;
            }
        }

        Ok(data)
    }

    fn free_inode_blocks(&mut self, inode: &Inode) -> Result<()> {
        // Free direct blocks
        for &ptr in &inode.block_pointers {
            if ptr != 0 {
                let bit_idx = (ptr - self.superblock.data_blocks_start) as u32;
                self.write_bitmap_bit(self.superblock.data_bitmap_block, bit_idx, false)?;
                self.superblock.free_blocks_count += 1;
            }
        }

        // Free indirect blocks
        if inode.indirect_pointer != 0 {
            let mut indirect_buf = [0u8; BLOCK_SIZE];
            self.read_block(inode.indirect_pointer, &mut indirect_buf)?;

            let mut cursor = std::io::Cursor::new(&indirect_buf[..]);
            for _ in 0..512 {
                let block_ptr = cursor.read_u64::<LE>()?;
                if block_ptr == 0 {
                    continue;
                }
                let bit_idx = (block_ptr - self.superblock.data_blocks_start) as u32;
                self.write_bitmap_bit(self.superblock.data_bitmap_block, bit_idx, false)?;
                self.superblock.free_blocks_count += 1;
            }

            let bit_idx = (inode.indirect_pointer - self.superblock.data_blocks_start) as u32;
            self.write_bitmap_bit(self.superblock.data_bitmap_block, bit_idx, false)?;
            self.superblock.free_blocks_count += 1;
        }

        self.write_superblock()?;
        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_format_and_open() {
        let tmp = NamedTempFile::new().unwrap();
        let fs = BeakFs::format(tmp.reopen().unwrap(), 2 * 1024 * 1024).unwrap();
        assert_eq!(fs.superblock.block_size, BLOCK_SIZE as u32);
        assert!(fs.superblock.free_inodes_count > 0);

        let opened = BeakFs::open(tmp.reopen().unwrap()).unwrap();
        assert_eq!(opened.superblock.inode_bitmap_block, fs.superblock.inode_bitmap_block);
    }

    #[test]
    fn test_file_create_write_read() {
        let tmp = NamedTempFile::new().unwrap();
        let mut fs = BeakFs::format(tmp.reopen().unwrap(), 2 * 1024 * 1024).unwrap();

        // Create directory
        let dir_inode = fs.create_file(0, "test_dir", true).unwrap();
        let root_entries = fs.list_dir(0).unwrap();
        assert_eq!(root_entries.len(), 1);
        assert_eq!(root_entries[0].name, "test_dir");
        assert_eq!(root_entries[0].file_type, 2); // Dir

        // Create file inside test_dir
        let file_inode = fs.create_file(dir_inode, "hello.txt", false).unwrap();
        let data = b"Hello Kestrel Beak FS! This is an ultra-fast file system.";
        fs.write_file_data(file_inode, data).unwrap();

        // Read file data back
        let read_data = fs.read_file_data(file_inode).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_large_file_indirect_pointers() {
        let tmp = NamedTempFile::new().unwrap();
        // Allocate a 5MB virtual disk
        let mut fs = BeakFs::format(tmp.reopen().unwrap(), 5 * 1024 * 1024).unwrap();

        let file_inode = fs.create_file(0, "large.bin", false).unwrap();

        // Write a 200KB file (needs direct + indirect pointers)
        let data = vec![0x41; 200 * 1024];
        fs.write_file_data(file_inode, &data).unwrap();

        let read_data = fs.read_file_data(file_inode).unwrap();
        assert_eq!(read_data.len(), 200 * 1024);
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_path_resolution() {
        let tmp = NamedTempFile::new().unwrap();
        let mut fs = BeakFs::format(tmp.reopen().unwrap(), 2 * 1024 * 1024).unwrap();

        let dir = fs.create_file(0, "docs", true).unwrap();
        let file = fs.create_file(dir, "readme.txt", false).unwrap();

        assert_eq!(fs.resolve_path("/").unwrap(), 0);
        assert_eq!(fs.resolve_path("/docs").unwrap(), dir);
        assert_eq!(fs.resolve_path("/docs/readme.txt").unwrap(), file);
        assert!(fs.resolve_path("/docs/missing.txt").is_err());
    }
}
