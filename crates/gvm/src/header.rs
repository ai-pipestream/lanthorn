// Glulx image header — GLULX_NOTES.md §1, §2.
//
// The first 36 bytes are nine big-endian 32-bit fields. We validate the magic,
// the version (major 2 or 3), and the memory-map invariants (256-byte aligned,
// RAMSTART ≤ EXTSTART ≤ ENDMEM).

use crate::error::GError;

/// The parsed Glulx header fields.
#[derive(Debug, Clone, Copy)]
pub struct Header {
    /// Raw 32-bit version word (major<<16 | minor<<8 | sub).
    pub version: u32,
    /// First writable address (end of ROM).
    pub ramstart: u32,
    /// End of the stored initial memory in the image file.
    pub extstart: u32,
    /// End of the memory map at startup.
    pub endmem: u32,
    /// Stack size the program requests, in bytes.
    pub stack_size: u32,
    /// Address of the first function to execute.
    pub start_func: u32,
    /// String-decoding-table address (0 = none; used in phase 2b).
    pub decode_table: u32,
    /// Whole-image checksum field as stored.
    pub checksum: u32,
}

fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Parse and validate the 36-byte Glulx header from `image`.
pub fn parse_header(image: &[u8]) -> Result<Header, GError> {
    if image.len() < 36 {
        return Err(GError::TooShort);
    }
    if &image[0..4] != b"Glul" {
        return Err(GError::BadMagic);
    }
    let version = be32(image, 0x04);
    let major = version >> 16;
    if major != 2 && major != 3 {
        return Err(GError::UnsupportedVersion(version));
    }
    let h = Header {
        version,
        ramstart: be32(image, 0x08),
        extstart: be32(image, 0x0C),
        endmem: be32(image, 0x10),
        stack_size: be32(image, 0x14),
        start_func: be32(image, 0x18),
        decode_table: be32(image, 0x1C),
        checksum: be32(image, 0x20),
    };

    // Memory-map invariants (GLULX_NOTES §2): all three boundaries are multiples
    // of 256 and ordered RAMSTART ≤ EXTSTART ≤ ENDMEM.
    let aligned = |v: u32| v.is_multiple_of(256);
    if !aligned(h.ramstart) || !aligned(h.extstart) || !aligned(h.endmem) {
        return Err(GError::BadMemoryMap);
    }
    if h.ramstart > h.extstart || h.extstart > h.endmem {
        return Err(GError::BadMemoryMap);
    }

    // The stored initial memory runs [0, EXTSTART); the file must be at least
    // that long.
    if (image.len() as u64) < h.extstart as u64 {
        return Err(GError::Truncated);
    }
    Ok(h)
}
