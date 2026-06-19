// Z-machine memory model — ZMSD §1.1, §1.2.
//
// Dynamic memory (0x0000 up to but not including static_mem_base) is readable
// and writable. Static memory and high memory are read-only. All multi-byte
// story values are big-endian.

use crate::error::ZError;
use crate::header::{parse_header, Header};

#[derive(Debug)]
pub struct Memory {
    bytes: Vec<u8>,
    header: Header,
}

impl Memory {
    /// Construct a `Memory` from raw story bytes.
    ///
    /// Parses the header and validates that `static_mem_base` does not exceed
    /// the buffer length (which would make the dynamic region nonsensical).
    pub fn new(bytes: Vec<u8>) -> Result<Memory, ZError> {
        let header = parse_header(&bytes)?;
        if header.static_mem_base as usize > bytes.len() {
            return Err(ZError::Truncated);
        }
        Ok(Memory { bytes, header })
    }

    /// Read a single byte at `addr`.
    pub fn read_byte(&self, addr: u32) -> u8 {
        self.bytes[addr as usize]
    }

    /// Write a single byte at `addr`. Only dynamic memory (addr < static_mem_base) is writable.
    pub fn write_byte(&mut self, addr: u32, v: u8) {
        debug_assert!(
            addr < self.header.static_mem_base as u32,
            "write to read-only memory at {:#06x} (static_mem_base = {:#06x})",
            addr,
            self.header.static_mem_base
        );
        self.bytes[addr as usize] = v;
    }

    /// Read a big-endian 16-bit word at `addr`.
    pub fn read_word(&self, addr: u32) -> u16 {
        let i = addr as usize;
        ((self.bytes[i] as u16) << 8) | self.bytes[i + 1] as u16
    }

    /// Write a big-endian 16-bit word at `addr`. Only dynamic memory is writable.
    pub fn write_word(&mut self, addr: u32, v: u16) {
        debug_assert!(
            addr < self.header.static_mem_base as u32,
            "write to read-only memory at {:#06x} (static_mem_base = {:#06x})",
            addr,
            self.header.static_mem_base
        );
        let i = addr as usize;
        self.bytes[i] = (v >> 8) as u8;
        self.bytes[i + 1] = (v & 0xFF) as u8;
    }

    /// Unpack a packed routine address (ZMSD §1.2.3).
    pub fn unpack_routine(&self, packed: u16) -> u32 {
        let p = packed as u32;
        match self.header.version {
            3 => 2 * p,
            4 | 5 => 4 * p,
            7 => 4 * p + 8 * self.header.routines_offset as u32,
            8 => 8 * p,
            _ => unreachable!("version validated by parse_header"),
        }
    }

    /// Z-machine version from the header.
    pub fn version(&self) -> u8 {
        self.header.version
    }

    /// Abbreviation table base address from the header.
    pub fn abbrev_table(&self) -> u16 {
        self.header.abbrev_table
    }

    /// Object table base address from the header.
    pub fn object_table(&self) -> u16 {
        self.header.object_table
    }

    /// Dictionary base address from the header.
    pub fn dictionary(&self) -> u16 {
        self.header.dictionary
    }

    /// Global variables table base address from the header.
    pub fn global_vars(&self) -> u16 {
        self.header.global_vars
    }

    /// Static memory base address (= end of dynamic memory region).
    pub fn static_mem_base(&self) -> u16 {
        self.header.static_mem_base
    }

    /// Raw read access to the underlying byte slice (for Quetzal CMem XOR).
    pub fn raw_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Unpack a packed string address (ZMSD §1.2.3).
    pub fn unpack_string(&self, packed: u16) -> u32 {
        let p = packed as u32;
        match self.header.version {
            3 => 2 * p,
            4 | 5 => 4 * p,
            7 => 4 * p + 8 * self.header.strings_offset as u32,
            8 => 8 * p,
            _ => unreachable!("version validated by parse_header"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;

    #[test]
    fn reads_and_writes_words_big_endian() {
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_word(0x40, 0xBEEF);
        assert_eq!(m.read_byte(0x40), 0xBE);
        assert_eq!(m.read_byte(0x41), 0xEF);
        assert_eq!(m.read_word(0x40), 0xBEEF);
    }

    #[test]
    fn unpacks_addresses_per_version() {
        assert_eq!(Memory::new(sample_story(3)).unwrap().unpack_routine(0x0100), 0x0200);
        assert_eq!(Memory::new(sample_story(5)).unwrap().unpack_routine(0x0100), 0x0400);
        assert_eq!(Memory::new(sample_story(8)).unwrap().unpack_routine(0x0100), 0x0800);
    }

    #[test]
    fn unpacks_string_addresses_per_version() {
        assert_eq!(Memory::new(sample_story(3)).unwrap().unpack_string(0x0100), 0x0200);
        assert_eq!(Memory::new(sample_story(5)).unwrap().unpack_string(0x0100), 0x0400);
        assert_eq!(Memory::new(sample_story(8)).unwrap().unpack_string(0x0100), 0x0800);
    }

    #[test]
    fn rejects_truncated_static_base() {
        let mut buf = crate::header::tests_support::sample_story(3);
        buf.truncate(64);
        let err = Memory::new(buf).unwrap_err();
        assert!(matches!(err, ZError::Truncated));
    }
}
