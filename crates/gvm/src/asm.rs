// Hand-assembler for Glulx test images — GLULX_NOTES.md §1, §3, §4.
//
// `#[cfg(test)]` only. Lets unit tests build tiny valid Glulx images: a header,
// a start function, and hand-encoded instructions. Grown task-by-task as more
// opcodes/modes are exercised.

#![cfg(test)]

/// Build a 36-byte-header image of length EXTSTART with the given memory map.
/// Bytes `[36, EXTSTART)` are zero. Useful for header/memory unit tests.
pub fn image_with_map(
    ramstart: u32,
    extstart: u32,
    endmem: u32,
    stack: u32,
    start: u32,
    decode: u32,
) -> Vec<u8> {
    let mut img = vec![0u8; extstart as usize];
    img[0..4].copy_from_slice(b"Glul");
    img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes()); // version 3.1.2
    img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
    img[0x0C..0x10].copy_from_slice(&extstart.to_be_bytes());
    img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
    img[0x14..0x18].copy_from_slice(&stack.to_be_bytes());
    img[0x18..0x1C].copy_from_slice(&start.to_be_bytes());
    img[0x1C..0x20].copy_from_slice(&decode.to_be_bytes());
    // checksum at 0x20 left zero (we don't verify it in 2a).
    img
}
