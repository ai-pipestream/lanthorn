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

/// Encode a function: a type byte (0xC0/0xC1), a locals-format list of
/// `(LocalType, LocalCount)` pairs terminated by `(0,0)`, then the body bytes.
pub fn func(type_byte: u8, locals: &[(u8, u8)], body: &[u8]) -> Vec<u8> {
    let mut f = vec![type_byte];
    for &(t, c) in locals {
        f.push(t);
        f.push(c);
    }
    f.push(0);
    f.push(0);
    f.extend_from_slice(body);
    f
}

/// One operand in a hand-assembled instruction, naming its addressing mode
/// explicitly so tests can exercise each mode (GLULX_NOTES §3).
#[derive(Clone, Copy)]
pub enum Op {
    /// Mode 0x0: constant zero (load) / discard (store).
    Zero,
    /// Mode 0x1: 1-byte signed constant.
    C8(i8),
    /// Mode 0x2: 2-byte signed constant.
    C16(i16),
    /// Mode 0x3: 4-byte constant.
    C32(u32),
    /// Mode 0x5: contents of / store to a 1-byte main-memory address.
    Mem8(u8),
    /// Mode 0x6: contents of / store to a 2-byte main-memory address.
    Mem16(u16),
    /// Mode 0x7: contents of / store to a 4-byte main-memory address.
    Mem32(u32),
    /// Mode 0x8: pop (load) / push (store).
    Stack,
    /// Mode 0x9: call-frame local at a 1-byte offset.
    Local8(u8),
    /// Mode 0xA: call-frame local at a 2-byte offset.
    Local16(u16),
    /// Mode 0xB: call-frame local at a 4-byte offset.
    Local32(u32),
    /// Mode 0xD: contents of / store to a 1-byte RAM-relative address.
    Ram8(u8),
    /// Mode 0xE: contents of / store to a 2-byte RAM-relative address.
    Ram16(u16),
    /// Mode 0xF: contents of / store to a 4-byte RAM-relative address.
    Ram32(u32),
}

impl Op {
    fn mode(self) -> u8 {
        match self {
            Op::Zero => 0x0,
            Op::C8(_) => 0x1,
            Op::C16(_) => 0x2,
            Op::C32(_) => 0x3,
            Op::Mem8(_) => 0x5,
            Op::Mem16(_) => 0x6,
            Op::Mem32(_) => 0x7,
            Op::Stack => 0x8,
            Op::Local8(_) => 0x9,
            Op::Local16(_) => 0xA,
            Op::Local32(_) => 0xB,
            Op::Ram8(_) => 0xD,
            Op::Ram16(_) => 0xE,
            Op::Ram32(_) => 0xF,
        }
    }
    fn data(self) -> Vec<u8> {
        match self {
            Op::Zero | Op::Stack => vec![],
            Op::C8(v) => vec![v as u8],
            Op::C16(v) => v.to_be_bytes().to_vec(),
            Op::C32(v) => v.to_be_bytes().to_vec(),
            Op::Mem8(v) | Op::Local8(v) | Op::Ram8(v) => vec![v],
            Op::Mem16(v) | Op::Local16(v) | Op::Ram16(v) => v.to_be_bytes().to_vec(),
            Op::Mem32(v) | Op::Local32(v) | Op::Ram32(v) => v.to_be_bytes().to_vec(),
        }
    }
}

/// Encode the variable-length opcode number (GLULX_NOTES §3).
pub fn opcode_bytes(opcode: u32) -> Vec<u8> {
    if opcode < 0x80 {
        vec![opcode as u8]
    } else if opcode < 0x4000 {
        ((opcode + 0x8000) as u16).to_be_bytes().to_vec()
    } else {
        (opcode + 0xC000_0000).to_be_bytes().to_vec()
    }
}

/// Encode just the operand block: packed mode nibbles (two per byte, earlier
/// operand in the low nibble) followed by the operand data in order.
pub fn operands(ops: &[Op]) -> Vec<u8> {
    let mut modes = vec![0u8; ops.len().div_ceil(2)];
    for (i, op) in ops.iter().enumerate() {
        let m = op.mode();
        if i % 2 == 0 {
            modes[i / 2] |= m;
        } else {
            modes[i / 2] |= m << 4;
        }
    }
    let mut out = modes;
    for op in ops {
        out.extend_from_slice(&op.data());
    }
    out
}

/// Assemble one instruction: opcode number, then the operand block.
pub fn ins(opcode: u32, ops: &[Op]) -> Vec<u8> {
    let mut out = opcode_bytes(opcode);
    out.extend_from_slice(&operands(ops));
    out
}

/// A built test image plus the load address of each supplied function.
pub struct Built {
    /// The complete Glulx image bytes.
    pub image: Vec<u8>,
    /// Load address of each function passed to [`assemble`], in order.
    pub addrs: Vec<u32>,
}

fn align_up(v: u32, to: u32) -> u32 {
    (v + to - 1) / to * to
}

/// Assemble `funcs` consecutively into ROM starting at offset 0x24 (just past
/// the header). `start_index` selects the header's start function. RAMSTART /
/// EXTSTART are 256-aligned just past the code; ENDMEM = RAMSTART + `ram_bytes`
/// (also 256-aligned). Stack size is 0x1000.
pub fn assemble(funcs: &[Vec<u8>], start_index: usize, ram_bytes: u32) -> Built {
    let mut rom = vec![0u8; 0x24];
    let mut addrs = Vec::with_capacity(funcs.len());
    for f in funcs {
        addrs.push(rom.len() as u32);
        rom.extend_from_slice(f);
    }
    let ramstart = align_up(rom.len() as u32, 256).max(0x100);
    let extstart = ramstart;
    let endmem = align_up(ramstart + ram_bytes.max(0x100), 256);
    let start = addrs[start_index];

    // Pad ROM up to EXTSTART (the stored initial memory ends at EXTSTART).
    rom.resize(extstart as usize, 0);
    rom[0..4].copy_from_slice(b"Glul");
    rom[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
    rom[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
    rom[0x0C..0x10].copy_from_slice(&extstart.to_be_bytes());
    rom[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
    rom[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
    rom[0x18..0x1C].copy_from_slice(&start.to_be_bytes());
    rom[0x1C..0x20].copy_from_slice(&0u32.to_be_bytes());
    Built { image: rom, addrs }
}
