//! Windows PE/PE+ (x64) loader for Kestrel.

use byteorder::{LE, ReadBytesExt};
use std::io::{Cursor, Read, Seek, SeekFrom};
use log::{info, debug, warn};

const MZ_MAGIC: u16 = 0x5A4D; // 'MZ'
const PE_SIGNATURE: u32 = 0x00004550; // 'PE\0\0'
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x020B;

/// A mapped PE section
#[derive(Debug, Clone)]
pub struct PeSection {
    pub name: [u8; 8],
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

/// Result of loading a PE binary
#[derive(Debug)]
pub struct PeImage {
    pub image_base: u64,
    pub entry_point_rva: u32,
    pub sections: Vec<PeSection>,
    pub image_size: u32,
}

pub struct PeLoader;

impl PeLoader {
    /// Check if the given bytes are a valid PE binary.
    pub fn is_pe(data: &[u8]) -> bool {
        if data.len() < 2 { return false; }
        let magic = u16::from_le_bytes([data[0], data[1]]);
        magic == MZ_MAGIC
    }

    /// Parse the PE headers and return a PeImage descriptor.
    pub fn parse(data: &[u8]) -> Result<PeImage, &'static str> {
        let mut c = Cursor::new(data);

        // DOS header
        let mz = c.read_u16::<LE>().map_err(|_| "Cannot read MZ magic")?;
        if mz != MZ_MAGIC {
            return Err("Not a PE binary: bad MZ magic");
        }

        // e_lfanew at offset 0x3C
        c.seek(SeekFrom::Start(0x3C)).map_err(|_| "Seek to e_lfanew failed")?;
        let e_lfanew = c.read_u32::<LE>().map_err(|_| "Cannot read e_lfanew")?;

        // PE signature
        c.seek(SeekFrom::Start(e_lfanew as u64)).map_err(|_| "Seek to PE sig failed")?;
        let pe_sig = c.read_u32::<LE>().map_err(|_| "Cannot read PE signature")?;
        if pe_sig != PE_SIGNATURE {
            return Err("Not a PE binary: bad PE signature");
        }

        // COFF header (after PE sig)
        let machine = c.read_u16::<LE>().map_err(|_| "Cannot read machine")?;
        let num_sections = c.read_u16::<LE>().map_err(|_| "Cannot read section count")?;
        c.seek(SeekFrom::Current(12)).map_err(|_| "Seek past COFF fields")?;
        let opt_header_size = c.read_u16::<LE>().map_err(|_| "Cannot read opt header size")?;
        c.seek(SeekFrom::Current(2)).map_err(|_| "Seek past characteristics")?; // characteristics

        debug!("PE: machine=0x{:04x} sections={} opt_header={}", machine, num_sections, opt_header_size);

        // Optional header
        let opt_magic = c.read_u16::<LE>().map_err(|_| "Cannot read opt magic")?;
        if opt_magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
            return Err("Only PE32+ (x64) binaries are supported");
        }

        // Skip MajorLinkerVersion, MinorLinkerVersion, SizeOfCode, ...
        c.seek(SeekFrom::Current(22)).map_err(|_| "Seek in opt header")?;
        let entry_point_rva = c.read_u32::<LE>().map_err(|_| "Cannot read AddressOfEntryPoint")?;
        c.seek(SeekFrom::Current(4)).map_err(|_| "Seek past BaseOfCode")?;
        let image_base = c.read_u64::<LE>().map_err(|_| "Cannot read ImageBase")?;
        c.seek(SeekFrom::Current(4)).map_err(|_| "Seek past SectionAlignment")?; // SectionAlignment
        c.seek(SeekFrom::Current(4)).map_err(|_| "Seek past FileAlignment")?;    // FileAlignment
        c.seek(SeekFrom::Current(16)).map_err(|_| "Seek past OS/subsystem versions")?;
        c.seek(SeekFrom::Current(4)).map_err(|_| "Seek past Win32VersionValue")?;
        let size_of_image = c.read_u32::<LE>().map_err(|_| "Cannot read SizeOfImage")?;

        // Jump past rest of opt header to section table
        let opt_header_start = e_lfanew + 4 + 20; // PE sig + COFF header
        let section_table_offset = opt_header_start + opt_header_size as u32;
        c.seek(SeekFrom::Start(section_table_offset as u64))
            .map_err(|_| "Seek to section table failed")?;

        // Parse sections
        let mut sections = Vec::with_capacity(num_sections as usize);
        for _ in 0..num_sections {
            let mut name = [0u8; 8];
            c.read_exact(&mut name).map_err(|_| "Cannot read section name")?;
            let virtual_size = c.read_u32::<LE>().map_err(|_| "Cannot read VirtualSize")?;
            let virtual_address = c.read_u32::<LE>().map_err(|_| "Cannot read VirtualAddress")?;
            let raw_size = c.read_u32::<LE>().map_err(|_| "Cannot read SizeOfRawData")?;
            let raw_offset = c.read_u32::<LE>().map_err(|_| "Cannot read PointerToRawData")?;
            c.seek(SeekFrom::Current(16)).map_err(|_| "Seek past section extras")?;
            let characteristics = c.read_u32::<LE>().map_err(|_| "Cannot read Characteristics")?;

            debug!("  Section: {} VA=0x{:08x} size={}",
                std::str::from_utf8(&name).unwrap_or("?"), virtual_address, virtual_size);

            sections.push(PeSection {
                name, virtual_address, virtual_size,
                raw_offset, raw_size, characteristics,
            });
        }

        info!("PE parsed: entry=0x{:08x} image_base=0x{:016x} size={}",
            entry_point_rva, image_base, size_of_image);

        Ok(PeImage {
            image_base,
            entry_point_rva,
            sections,
            image_size: size_of_image,
        })
    }

    /// Map PE sections into a flat memory buffer (guest address space).
    /// Returns the absolute entry point address.
    pub fn map_into_memory(
        pe: &PeImage,
        file_data: &[u8],
        guest_mem: &mut [u8],
        load_base: usize,
    ) -> Result<usize, &'static str> {
        // Zero the image area
        let end = load_base + pe.image_size as usize;
        if end > guest_mem.len() {
            return Err("PE image too large for guest memory");
        }
        guest_mem[load_base..end].fill(0);

        // Map each section
        for section in &pe.sections {
            let src_start = section.raw_offset as usize;
            let src_end = src_start + section.raw_size as usize;
            let dst_start = load_base + section.virtual_address as usize;
            let dst_end = dst_start + section.raw_size.min(section.virtual_size) as usize;

            if src_end > file_data.len() || dst_end > guest_mem.len() {
                warn!("Section out of bounds, skipping");
                continue;
            }

            let copy_len = section.raw_size.min(section.virtual_size) as usize;
            guest_mem[dst_start..dst_start + copy_len]
                .copy_from_slice(&file_data[src_start..src_start + copy_len]);

            debug!("  Mapped section at guest 0x{:x}", dst_start);
        }

        let entry = load_base + pe.entry_point_rva as usize;
        info!("PE mapped at load_base=0x{:x}, entry=0x{:x}", load_base, entry);
        Ok(entry)
    }
}
