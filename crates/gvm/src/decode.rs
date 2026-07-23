//! Shared Glulx instruction-decoding primitives (GLULX_NOTES §3).
//!
//! These pure, `&Memory`-only helpers are the single source of truth for the
//! instruction *layout* — the variable-length opcode number and the byte width
//! of each addressing mode's operand data. Both the interpreter ([`exec`]) and
//! the disassembler ([`disasm`]) decode against them.
//!
//! Boundary (SQ-0465): to keep the interpreter hot loop zero-cost, only the
//! opcode-number decode is *code-shared* — [`exec::Machine::decode_opcode`]
//! delegates here. The executor keeps its own fused operand fetch
//! (`resolve_load`/`resolve_store`), whose take-widths are LAYOUT-coupled to
//! [`operand_data_len`] and verified identical by a decode round-trip test
//! (disasm walks each executed PC and must land on the same instruction
//! boundaries the executor advanced through).
//!
//! [`exec`]: crate::exec
//! [`disasm`]: crate::disasm

use crate::exec::R;
use crate::memory::Memory;

/// Decode the variable-length opcode number at `pc`, returning
/// `(opcode, next_pc)` where `next_pc` is the first operand-mode byte.
///
/// The top two bits of the first byte select the encoding width (GLULX_NOTES
/// §3): `0b11…` → 4 bytes (bias `0xC000_0000`), `0b10…` → 2 bytes (bias
/// `0x8000`), otherwise 1 byte. Error strings match the executor's memory-fault
/// wording so a decode fault reads identically wherever it surfaces.
#[inline]
pub(crate) fn decode_opcode(mem: &Memory, pc: u32) -> R<(u32, u32)> {
    let b0 = mem.read8(pc).ok_or_else(|| format!("memory fault: read8 @{pc:#010x}"))?;
    match b0 & 0xC0 {
        0xC0 => {
            let v = mem.read32(pc).ok_or_else(|| format!("memory fault: read32 @{pc:#010x}"))?;
            Ok((v - 0xC000_0000, pc + 4))
        }
        0x80 => {
            let v = mem.read16(pc).ok_or_else(|| format!("memory fault: read16 @{pc:#010x}"))?;
            Ok((v - 0x8000, pc + 2))
        }
        _ => Ok((b0, pc + 1)),
    }
}

/// Bytes of operand *data* that follow the mode nibble for addressing `mode`
/// (GLULX_NOTES §3). Constants and addresses carry 1/2/4 inline bytes; the
/// zero, stack, and (illegal-for-load, but same-width) modes carry none.
///
/// This is the layout truth the disassembler walks with; the executor's
/// `resolve_load`/`resolve_store` encode the same widths inline via
/// `take8/16/32` and are held to it by the decode round-trip test.
#[inline]
pub(crate) fn operand_data_len(mode: u8) -> u32 {
    match mode & 0x0F {
        0x0 | 0x8 => 0,             // constant zero / discard, stack pop / push
        0x1 | 0x5 | 0x9 | 0xD => 1, // 1-byte const / addr / local / RAM-rel
        0x2 | 0x6 | 0xA | 0xE => 2, // 2-byte …
        _ => 4,                     // 0x3/0x7/0xB/0xF (and 0x4/0xC, unused) 4-byte …
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm;

    fn mem_of(bytes: &[u8]) -> Memory {
        // A minimal valid image with `bytes` placed at offset 0x24 (just past
        // the header), so opcode decoding can be exercised at a known address.
        let built = asm::assemble(&[asm::func(0xC1, &[], bytes)], 0, 0x100);
        Memory::new(built.image).unwrap()
    }

    #[test]
    fn opcode_widths_and_bias() {
        // 1-byte: 0x30 (call) at 0x24 → func header type byte is 0xC1, so build a
        // raw byte image instead via a body whose first bytes are the opcode.
        let m = mem_of(&[0x30]); // body starts after the (0xC1,(0,0)) header
        // body offset within the func: header is C1 + (0,0) terminator = 3 bytes,
        // so the body byte 0x30 sits at 0x24 + 3.
        let base = 0x24 + 3;
        assert_eq!(decode_opcode(&m, base).unwrap(), (0x30, base + 1));

        // 2-byte: 0x8000 | 0x0130 = 0x8130 encodes opcode 0x130 (glk).
        let m = mem_of(&[0x81, 0x30]);
        assert_eq!(decode_opcode(&m, base).unwrap(), (0x130, base + 2));

        // 4-byte: 0xC000_0000 | 0x104 = 0xC0000104 encodes opcode 0x104 (jumpabs).
        let m = mem_of(&[0xC0, 0x00, 0x01, 0x04]);
        assert_eq!(decode_opcode(&m, base).unwrap(), (0x104, base + 4));
    }

    #[test]
    fn operand_data_len_by_mode() {
        for (mode, len) in [
            (0x0, 0), (0x8, 0),
            (0x1, 1), (0x5, 1), (0x9, 1), (0xD, 1),
            (0x2, 2), (0x6, 2), (0xA, 2), (0xE, 2),
            (0x3, 4), (0x7, 4), (0xB, 4), (0xF, 4),
        ] {
            assert_eq!(operand_data_len(mode), len, "mode {mode:#x}");
        }
    }
}
