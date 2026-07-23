//! Glulx disassembler + lazy discovery cache for the debug inspector (SQ-0465).
//!
//! Built on the shared [`crate::decode`] primitives (same opcode-number decode
//! the interpreter uses). Discovery is cheap and eager (function headers + call
//! graph + a type-validated linear scan of ROM); *rendering* disassembly text is
//! lazy and windowed — an I7 image is multi-MB, so we never format the whole
//! thing, only the address window the inspector asks for.
//!
//! Confidence tiers ([`Tier`], mirroring zvm's `Provenance`):
//!  * `Rd`   — reached from the start function via the constant call graph.
//!  * `Soft` — found only by the type-validated linear scan (an unverified guess).
//!  * `Data` — string/table bytes (E0/E1/E2 objects and gaps), never code.
//!
//! RAM-resident code (legal but rare) is not scanned statically; it is surfaced
//! only when the app seeds executed PCs (see [`DisasmCache::seed_executed`] /
//! [`confirm_pc`](DisasmCache::confirm_pc)), matching zvm's execution-as-truth.

use crate::decode::{decode_opcode, operand_data_len};
use crate::exec::R;
use crate::memory::Memory;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Static confidence tier of a byte range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Reached from the start function / constant call graph (hard).
    Rd,
    /// Found only by the type-validated linear scan (soft guess).
    Soft,
    /// String/table/data bytes — never code.
    Data,
}

// ─── opcode metadata table ────────────────────────────────────────────────────

/// Role-shaping of an opcode's operand list, for target/annotation resolution.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Special {
    /// Plain: loads first, then stores; no branch/call target.
    None,
    /// Conditional/unconditional relative branch: the last load is the offset.
    Branch,
    /// A call: operand 0 (a load) is the function target.
    Call,
    /// `jumpabs`: operand 0 is an absolute target (no `+off-2` bias).
    JumpAbs,
    /// `glk`: operand 0 is the selector (named when constant).
    Glk,
    /// `streamstr`: operand 0 is a string/callable address (previewed).
    StreamStr,
    /// `catch S1 L1`: operand 0 is the *store*, operand 1 the branch offset.
    Catch,
}

/// Static shape of one opcode: mnemonic, load/store operand counts, role.
#[derive(Clone, Copy)]
struct OpInfo {
    mnemonic: &'static str,
    n_load: u8,
    n_store: u8,
    special: Special,
}

impl OpInfo {
    #[inline]
    fn total(&self) -> usize {
        (self.n_load + self.n_store) as usize
    }
    /// Is operand `i` (encoding order) a store destination?
    #[inline]
    fn is_store(&self, i: usize) -> bool {
        match self.special {
            Special::Catch => i == 0, // catch S1 L1: store first
            _ => i >= self.n_load as usize,
        }
    }
    /// Index of the branch-offset operand, if this opcode branches.
    #[inline]
    fn branch_index(&self) -> Option<usize> {
        match self.special {
            Special::Branch => Some(self.n_load as usize - 1), // last load
            Special::Catch => Some(1),
            _ => None,
        }
    }
}

macro_rules! op {
    ($mn:literal, $l:literal, $s:literal) => {
        OpInfo { mnemonic: $mn, n_load: $l, n_store: $s, special: Special::None }
    };
    ($mn:literal, $l:literal, $s:literal, $sp:expr) => {
        OpInfo { mnemonic: $mn, n_load: $l, n_store: $s, special: $sp }
    };
}

/// The Glulx opcode table, transcribed from `exec::Machine::execute` (the
/// authority). Operand counts are the `read_operands(n_load, n_store)` shapes;
/// `special` marks call/branch/glk/string roles for annotation.
fn opcode_info(opcode: u32) -> Option<OpInfo> {
    use Special::*;
    Some(match opcode {
        0x00 => op!("nop", 0, 0),
        // integer arithmetic
        0x10 => op!("add", 2, 1),
        0x11 => op!("sub", 2, 1),
        0x12 => op!("mul", 2, 1),
        0x13 => op!("div", 2, 1),
        0x14 => op!("mod", 2, 1),
        0x15 => op!("neg", 1, 1),
        0x18 => op!("bitand", 2, 1),
        0x19 => op!("bitor", 2, 1),
        0x1A => op!("bitxor", 2, 1),
        0x1B => op!("bitnot", 1, 1),
        0x1C => op!("shiftl", 2, 1),
        0x1D => op!("sshiftr", 2, 1),
        0x1E => op!("ushiftr", 2, 1),
        // branches
        0x20 => op!("jump", 1, 0, Branch),
        0x22 => op!("jz", 2, 0, Branch),
        0x23 => op!("jnz", 2, 0, Branch),
        0x24 => op!("jeq", 3, 0, Branch),
        0x25 => op!("jne", 3, 0, Branch),
        0x26 => op!("jlt", 3, 0, Branch),
        0x27 => op!("jge", 3, 0, Branch),
        0x28 => op!("jgt", 3, 0, Branch),
        0x29 => op!("jle", 3, 0, Branch),
        0x2A => op!("jltu", 3, 0, Branch),
        0x2B => op!("jgeu", 3, 0, Branch),
        0x2C => op!("jgtu", 3, 0, Branch),
        0x2D => op!("jleu", 3, 0, Branch),
        // calls / return
        0x30 => op!("call", 2, 1, Call),
        0x31 => op!("return", 1, 0),
        0x32 => op!("catch", 1, 1, Catch),
        0x33 => op!("throw", 2, 0),
        0x34 => op!("tailcall", 2, 0, Call),
        // copy / sign-extend
        0x40 => op!("copy", 1, 1),
        0x41 => op!("copys", 1, 1),
        0x42 => op!("copyb", 1, 1),
        0x44 => op!("sexs", 1, 1),
        0x45 => op!("sexb", 1, 1),
        // memory-array load/store
        0x48 => op!("aload", 2, 1),
        0x49 => op!("aloads", 2, 1),
        0x4A => op!("aloadb", 2, 1),
        0x4B => op!("aloadbit", 2, 1),
        0x4C => op!("astore", 3, 0),
        0x4D => op!("astores", 3, 0),
        0x4E => op!("astoreb", 3, 0),
        0x4F => op!("astorebit", 3, 0),
        // stack
        0x50 => op!("stkcount", 0, 1),
        0x51 => op!("stkpeek", 1, 1),
        0x52 => op!("stkswap", 0, 0),
        0x53 => op!("stkroll", 2, 0),
        0x54 => op!("stkcopy", 1, 0),
        // stream output
        0x70 => op!("streamchar", 1, 0),
        0x71 => op!("streamnum", 1, 0),
        0x72 => op!("streamstr", 1, 0, StreamStr),
        0x73 => op!("streamunichar", 1, 0),
        // machine / memory
        0x100 => op!("gestalt", 2, 1),
        0x101 => op!("debugtrap", 1, 0),
        0x102 => op!("getmemsize", 0, 1),
        0x103 => op!("setmemsize", 1, 1),
        0x104 => op!("jumpabs", 1, 0, JumpAbs),
        0x110 => op!("random", 1, 1),
        0x111 => op!("setrandom", 1, 0),
        0x120 => op!("quit", 0, 0),
        0x121 => op!("verify", 0, 1),
        0x122 => op!("restart", 0, 0),
        0x123 => op!("save", 1, 1),
        0x124 => op!("restore", 1, 1),
        0x125 => op!("saveundo", 0, 1),
        0x126 => op!("restoreundo", 0, 1),
        0x127 => op!("protect", 2, 0),
        0x128 => op!("hasundo", 0, 1),
        0x129 => op!("discardundo", 0, 0),
        0x130 => op!("glk", 2, 1, Glk),
        0x140 => op!("getstringtbl", 0, 1),
        0x141 => op!("setstringtbl", 1, 0),
        0x148 => op!("getiosys", 0, 2),
        0x149 => op!("setiosys", 2, 0),
        0x150 => op!("linearsearch", 7, 1),
        0x151 => op!("binarysearch", 7, 1),
        0x152 => op!("linkedsearch", 6, 1),
        // call-with-fixed-args
        0x160 => op!("callf", 1, 1, Call),
        0x161 => op!("callfi", 2, 1, Call),
        0x162 => op!("callfii", 3, 1, Call),
        0x163 => op!("callfiii", 4, 1, Call),
        // block copy / heap / accel
        0x170 => op!("mzero", 2, 0),
        0x171 => op!("mcopy", 3, 0),
        0x178 => op!("malloc", 1, 1),
        0x179 => op!("mfree", 1, 0),
        0x180 => op!("accelfunc", 2, 0),
        0x181 => op!("accelparam", 2, 0),
        // single-precision float
        0x190 => op!("numtof", 1, 1),
        0x191 => op!("ftonumz", 1, 1),
        0x192 => op!("ftonumn", 1, 1),
        0x198 => op!("ceil", 1, 1),
        0x199 => op!("floor", 1, 1),
        0x1A0 => op!("fadd", 2, 1),
        0x1A1 => op!("fsub", 2, 1),
        0x1A2 => op!("fmul", 2, 1),
        0x1A3 => op!("fdiv", 2, 1),
        0x1A4 => op!("fmod", 2, 2),
        0x1A8 => op!("sqrt", 1, 1),
        0x1A9 => op!("exp", 1, 1),
        0x1AA => op!("log", 1, 1),
        0x1AB => op!("pow", 2, 1),
        0x1B0 => op!("sin", 1, 1),
        0x1B1 => op!("cos", 1, 1),
        0x1B2 => op!("tan", 1, 1),
        0x1B3 => op!("asin", 1, 1),
        0x1B4 => op!("acos", 1, 1),
        0x1B5 => op!("atan", 1, 1),
        0x1B6 => op!("atan2", 2, 1),
        0x1C0 => op!("jfeq", 4, 0, Branch),
        0x1C1 => op!("jfne", 4, 0, Branch),
        0x1C2 => op!("jflt", 3, 0, Branch),
        0x1C3 => op!("jfle", 3, 0, Branch),
        0x1C4 => op!("jfgt", 3, 0, Branch),
        0x1C5 => op!("jfge", 3, 0, Branch),
        0x1C8 => op!("jisnan", 2, 0, Branch),
        0x1C9 => op!("jisinf", 2, 0, Branch),
        // double-precision float
        0x200 => op!("numtod", 1, 2),
        0x201 => op!("dtonumz", 2, 1),
        0x202 => op!("dtonumn", 2, 1),
        0x203 => op!("ftod", 1, 2),
        0x204 => op!("dtof", 2, 1),
        0x208 => op!("dceil", 2, 2),
        0x209 => op!("dfloor", 2, 2),
        0x210 => op!("dadd", 4, 2),
        0x211 => op!("dsub", 4, 2),
        0x212 => op!("dmul", 4, 2),
        0x213 => op!("ddiv", 4, 2),
        0x214 => op!("dmodr", 4, 2),
        0x215 => op!("dmodq", 4, 2),
        0x218 => op!("dsqrt", 2, 2),
        0x219 => op!("dexp", 2, 2),
        0x21A => op!("dlog", 2, 2),
        0x21B => op!("dpow", 4, 2),
        0x220 => op!("dsin", 2, 2),
        0x221 => op!("dcos", 2, 2),
        0x222 => op!("dtan", 2, 2),
        0x223 => op!("dasin", 2, 2),
        0x224 => op!("dacos", 2, 2),
        0x225 => op!("datan", 2, 2),
        0x226 => op!("datan2", 4, 2),
        0x230 => op!("jdeq", 7, 0, Branch),
        0x231 => op!("jdne", 7, 0, Branch),
        0x232 => op!("jdlt", 5, 0, Branch),
        0x233 => op!("jdle", 5, 0, Branch),
        0x234 => op!("jdgt", 5, 0, Branch),
        0x235 => op!("jdge", 5, 0, Branch),
        0x238 => op!("jdisnan", 3, 0, Branch),
        0x239 => op!("jdisinf", 3, 0, Branch),
        _ => return Option::None,
    })
}

/// One-to-two line description of an opcode for a hover tip. Covers the common
/// opcodes; exotic ones fall back to `None` (the caller shows "no description").
pub fn opcode_help(opcode: u32) -> Option<&'static str> {
    Some(match opcode {
        0x00 => "nop — do nothing.",
        0x10 => "add L1 L2 S1 — S1 = L1 + L2 (32-bit wraparound).",
        0x11 => "sub L1 L2 S1 — S1 = L1 - L2.",
        0x12 => "mul L1 L2 S1 — S1 = L1 * L2 (low 32 bits).",
        0x13 => "div L1 L2 S1 — S1 = L1 / L2 (signed, toward zero).",
        0x14 => "mod L1 L2 S1 — S1 = L1 mod L2 (signed remainder).",
        0x15 => "neg L1 S1 — S1 = -L1.",
        0x18 => "bitand L1 L2 S1 — S1 = L1 & L2.",
        0x19 => "bitor L1 L2 S1 — S1 = L1 | L2.",
        0x1A => "bitxor L1 L2 S1 — S1 = L1 ^ L2.",
        0x1B => "bitnot L1 S1 — S1 = ~L1.",
        0x1C => "shiftl L1 L2 S1 — S1 = L1 << L2 (0 if L2>=32).",
        0x1D => "sshiftr L1 L2 S1 — arithmetic right shift.",
        0x1E => "ushiftr L1 L2 S1 — logical right shift.",
        0x20 => "jump L1 — branch by offset L1.",
        0x22 => "jz L1 off — branch if L1 == 0.",
        0x23 => "jnz L1 off — branch if L1 != 0.",
        0x24 => "jeq L1 L2 off — branch if L1 == L2.",
        0x25 => "jne L1 L2 off — branch if L1 != L2.",
        0x26 => "jlt L1 L2 off — branch if L1 < L2 (signed).",
        0x27 => "jge L1 L2 off — branch if L1 >= L2 (signed).",
        0x28 => "jgt L1 L2 off — branch if L1 > L2 (signed).",
        0x29 => "jle L1 L2 off — branch if L1 <= L2 (signed).",
        0x2A => "jltu L1 L2 off — branch if L1 < L2 (unsigned).",
        0x2B => "jgeu L1 L2 off — branch if L1 >= L2 (unsigned).",
        0x2C => "jgtu L1 L2 off — branch if L1 > L2 (unsigned).",
        0x2D => "jleu L1 L2 off — branch if L1 <= L2 (unsigned).",
        0x30 => "call L1 L2 S1 — call fn L1 with L2 args popped from stack; result to S1.",
        0x31 => "return L1 — return value L1 from the current function.",
        0x32 => "catch S1 off — push a catch token to S1, then branch.",
        0x33 => "throw L1 L2 — throw value L1 to catch token L2.",
        0x34 => "tailcall L1 L2 — replace this frame with a call to fn L1 (L2 args).",
        0x40 => "copy L1 S1 — S1 = L1 (32-bit).",
        0x41 => "copys L1 S1 — copy the low 16 bits.",
        0x42 => "copyb L1 S1 — copy the low 8 bits.",
        0x44 => "sexs L1 S1 — sign-extend from 16 bits.",
        0x45 => "sexb L1 S1 — sign-extend from 8 bits.",
        0x48 => "aload L1 L2 S1 — S1 = memory word at L1 + 4*L2.",
        0x49 => "aloads L1 L2 S1 — S1 = 16-bit value at L1 + 2*L2.",
        0x4A => "aloadb L1 L2 S1 — S1 = byte at L1 + L2.",
        0x4B => "aloadbit L1 L2 S1 — S1 = bit L2 of the bitstring at L1.",
        0x4C => "astore L1 L2 L3 — store word L3 at L1 + 4*L2.",
        0x4D => "astores L1 L2 L3 — store 16-bit L3 at L1 + 2*L2.",
        0x4E => "astoreb L1 L2 L3 — store byte L3 at L1 + L2.",
        0x4F => "astorebit L1 L2 L3 — set/clear bit L2 of the bitstring at L1.",
        0x50 => "stkcount S1 — S1 = number of values on the stack.",
        0x51 => "stkpeek L1 S1 — S1 = the L1'th value from the top (0 = top).",
        0x52 => "stkswap — swap the top two stack values.",
        0x53 => "stkroll L1 L2 — rotate the top L1 values by L2 places.",
        0x54 => "stkcopy L1 — duplicate the top L1 values.",
        0x70 => "streamchar L1 — output the low byte of L1 as a Latin-1 char.",
        0x71 => "streamnum L1 — output L1 as a signed decimal number.",
        0x72 => "streamstr L1 — print the string/function object at L1.",
        0x73 => "streamunichar L1 — output L1 as a Unicode char.",
        0x100 => "gestalt L1 L2 S1 — query interpreter capability L1 (arg L2).",
        0x101 => "debugtrap L1 — signal a debug trap with value L1.",
        0x102 => "getmemsize S1 — S1 = current memory size in bytes.",
        0x103 => "setmemsize L1 S1 — resize memory to L1; S1 = 0 on success.",
        0x104 => "jumpabs L1 — jump to absolute address L1.",
        0x110 => "random L1 S1 — S1 = random number in [0,L1) (or full range if 0).",
        0x111 => "setrandom L1 — seed the RNG with L1 (0 = unpredictable).",
        0x120 => "quit — stop the program.",
        0x121 => "verify S1 — S1 = 0 if the image checksum is intact.",
        0x122 => "restart — reset the program to its initial state.",
        0x123 => "save L1 S1 — save game state to stream L1; S1 = result.",
        0x124 => "restore L1 S1 — restore game state from stream L1; S1 = result.",
        0x125 => "saveundo S1 — snapshot state for undo; S1 = result.",
        0x126 => "restoreundo S1 — restore the last undo snapshot; S1 = result.",
        0x127 => "protect L1 L2 — protect L2 bytes at L1 across restore.",
        0x128 => "hasundo S1 — S1 = 0 if an undo state is available.",
        0x129 => "discardundo — drop the most recent undo snapshot.",
        0x130 => "glk L1 L2 S1 — call Glk selector L1 with L2 args; result to S1.",
        0x140 => "getstringtbl S1 — S1 = the current string-decoding table.",
        0x141 => "setstringtbl L1 — set the string-decoding table to L1.",
        0x148 => "getiosys S1 S2 — S1 = I/O system mode, S2 = rock.",
        0x149 => "setiosys L1 L2 — set the I/O system to mode L1, rock L2.",
        0x150 => "linearsearch — linear search of a key array (7 args, S1 result).",
        0x151 => "binarysearch — binary search of a sorted key array (7 args).",
        0x152 => "linkedsearch — search a linked list by key (6 args).",
        0x160 => "callf L1 S1 — call fn L1 with no args; result to S1.",
        0x161 => "callfi L1 L2 S1 — call fn L1 with 1 arg L2.",
        0x162 => "callfii L1 L2 L3 S1 — call fn L1 with 2 args.",
        0x163 => "callfiii L1 L2 L3 L4 S1 — call fn L1 with 3 args.",
        0x170 => "mzero L1 L2 — zero L1 bytes starting at L2.",
        0x171 => "mcopy L1 L2 L3 — copy L1 bytes from L2 to L3.",
        0x178 => "malloc L1 S1 — allocate L1 bytes; S1 = address (0 = fail).",
        0x179 => "mfree L1 — free the block at L1.",
        0x180 => "accelfunc L1 L2 — accelerate fn at L2 with accel #L1.",
        0x181 => "accelparam L1 L2 — set acceleration parameter L1 = L2.",
        0x190 => "numtof L1 S1 — S1 = L1 converted to float.",
        0x191 => "ftonumz L1 S1 — float L1 to int, toward zero.",
        0x192 => "ftonumn L1 S1 — float L1 to int, rounded.",
        0x1A0 => "fadd L1 L2 S1 — float add.",
        0x1A1 => "fsub L1 L2 S1 — float subtract.",
        0x1A2 => "fmul L1 L2 S1 — float multiply.",
        0x1A3 => "fdiv L1 L2 S1 — float divide.",
        _ => return None,
    })
}

// ─── one-instruction decode (cold path; disassembler only) ────────────────────

/// One decoded operand: its addressing `mode`, the raw immediate/address/offset
/// `value` (zero-extended), whether it is a store destination, and the address
/// its data occupies in the stream.
#[derive(Clone, Copy, Debug)]
pub struct Operand {
    pub mode: u8,
    pub value: u32,
    pub is_store: bool,
    pub data_addr: u32,
}

/// A fully decoded instruction spanning `[addr, next)`.
#[derive(Clone, Debug)]
pub struct Instr {
    pub addr: u32,
    pub opcode: u32,
    pub next: u32,
    pub operands: Vec<Operand>,
}

fn fault(kind: &str, a: u32) -> String {
    format!("memory fault: {kind} @{a:#010x}")
}

/// Decode the instruction at `addr` into opcode + operands. Errors on an
/// unknown opcode or a read past the mapped image (so the disassembler never
/// walks off into undecodable bytes).
pub fn decode_instr(mem: &Memory, addr: u32) -> R<Instr> {
    let (opcode, mut pc) = decode_opcode(mem, addr)?;
    let info = opcode_info(opcode).ok_or_else(|| format!("unknown opcode {opcode:#x} @{addr:#010x}"))?;
    let total = info.total();
    let mode_bytes = total.div_ceil(2);
    let mut modes = Vec::with_capacity(total);
    for i in 0..mode_bytes {
        let byte = mem.read8(pc + i as u32).ok_or_else(|| fault("read8", pc + i as u32))?;
        modes.push((byte & 0x0F) as u8);
        modes.push((byte >> 4) as u8);
    }
    pc += mode_bytes as u32;
    let mut operands = Vec::with_capacity(total);
    for (i, &mode) in modes.iter().enumerate().take(total) {
        let dl = operand_data_len(mode);
        let value = match dl {
            0 => 0,
            1 => mem.read8(pc).ok_or_else(|| fault("read8", pc))?,
            2 => mem.read16(pc).ok_or_else(|| fault("read16", pc))?,
            _ => mem.read32(pc).ok_or_else(|| fault("read32", pc))?,
        };
        operands.push(Operand { mode, value, is_store: info.is_store(i), data_addr: pc });
        pc += dl;
    }
    Ok(Instr { addr, opcode, next: pc, operands })
}

/// Length-only decode: the `next_pc` after the instruction at `addr`, without
/// allocating an operand list. Used by the discovery linear scan / tiling.
fn instr_len(mem: &Memory, addr: u32) -> R<u32> {
    let (opcode, mut pc) = decode_opcode(mem, addr)?;
    let info = opcode_info(opcode).ok_or_else(|| format!("unknown opcode {opcode:#x}"))?;
    let total = info.total();
    let mode_bytes = total.div_ceil(2) as u32;
    let mut modes = Vec::with_capacity(total);
    for i in 0..mode_bytes {
        let byte = mem.read8(pc + i).ok_or_else(|| fault("read8", pc + i))?;
        modes.push((byte & 0x0F) as u8);
        modes.push((byte >> 4) as u8);
    }
    pc += mode_bytes;
    for &mode in modes.iter().take(total) {
        pc += operand_data_len(mode);
    }
    Ok(pc)
}

/// Sign-extend an operand `value` per its addressing `mode` (constant modes
/// 0x1/0x2 are signed in the executor; 0x3 and addresses are used as-is).
fn signed_const(mode: u8, value: u32) -> i64 {
    match mode & 0x0F {
        0x1 => value as u8 as i8 as i64,
        0x2 => value as u16 as i16 as i64,
        _ => value as i32 as i64,
    }
}

// ─── function headers & strings ───────────────────────────────────────────────

/// A validated function header.
struct FuncHeader {
    type_byte: u8,
    locals_count: u32,
    first_instr: u32,
}

/// Validate a function object at `addr`: a `0xC0`/`0xC1` type byte followed by a
/// well-formed locals descriptor (`(type∈{1,2,4}, count)` pairs ending in
/// `(0,0)`). Returns `None` for anything that is not a well-formed function.
fn parse_func_header(mem: &Memory, addr: u32) -> Option<FuncHeader> {
    let tb = mem.read8(addr)?;
    if tb != 0xC0 && tb != 0xC1 {
        return None;
    }
    let mut a = addr + 1;
    let mut count = 0u32;
    let mut pairs = 0u32;
    loop {
        let t = mem.read8(a)?;
        let c = mem.read8(a + 1)?;
        a += 2;
        if t == 0 && c == 0 {
            break;
        }
        if t != 1 && t != 2 && t != 4 {
            return None;
        }
        count += c;
        pairs += 1;
        if pairs > 255 {
            return None; // runaway: not a real descriptor
        }
    }
    Some(FuncHeader { type_byte: tb as u8, locals_count: count, first_instr: a })
}

/// If `addr` begins a string object, return `(type_byte, end)` where `end` is
/// one past the last string byte. E1 (compressed) extents are found by walking
/// the Huffman bitstream against the header decode table.
fn string_extent(mem: &Memory, decode_table: u32, addr: u32) -> Option<(u8, u32)> {
    match mem.read8(addr)? {
        0xE0 => {
            let mut a = addr + 1;
            loop {
                let b = mem.read8(a)?;
                a += 1;
                if b == 0 {
                    break;
                }
            }
            Some((0xE0, a))
        }
        0xE2 => {
            let mut a = addr + 4; // type byte + 3 pad, then 32-bit chars
            loop {
                let w = mem.read32(a)?;
                a += 4;
                if w == 0 {
                    break;
                }
            }
            Some((0xE2, a))
        }
        0xE1 => walk_e1(mem, decode_table, addr + 1, None).map(|(end, _)| (0xE1, end)),
        _ => None,
    }
}

/// Walk a compressed (E1) bit stream from `start` against the decode table at
/// `table` (root at `table+8`), returning `(end_byte, text)`. Bits are read
/// low-bit-first (matching `exec::decode_compressed`). When `cap` is `Some(n)`,
/// at most `n` chars of preview text are collected (indirect/complex leaf nodes
/// are elided as `…`); when `None`, no text is built (extent-only). Bounded so a
/// malformed stream can't loop forever.
fn walk_e1(mem: &Memory, table: u32, start: u32, cap: Option<usize>) -> Option<(u32, String)> {
    if table == 0 {
        return None;
    }
    let root = mem.read32(table + 8)?;
    let mut node = root;
    let mut addr = start;
    let mut bit = 0u32;
    let mut text = String::new();
    let mut steps = 0u32;
    loop {
        steps += 1;
        if steps > 1_000_000 {
            return None; // runaway guard
        }
        match mem.read8(node)? {
            0x00 => {
                // branch: consume one bit
                let byte = mem.read8(addr)?;
                let b = (byte >> bit) & 1;
                bit += 1;
                if bit == 8 {
                    bit = 0;
                    addr += 1;
                }
                node = if b == 0 { mem.read32(node + 1)? } else { mem.read32(node + 5)? };
            }
            0x01 => {
                // terminator: end is the byte after the last consumed one
                let end = if bit == 0 { addr } else { addr + 1 };
                return Some((end, text));
            }
            0x02 => {
                if let Some(n) = cap {
                    if text.chars().count() < n {
                        text.push(mem.read8(node + 1)? as u8 as char);
                    }
                }
                node = root;
            }
            0x03 => {
                if let Some(n) = cap {
                    let mut a = node + 1;
                    while text.chars().count() < n {
                        let b = mem.read8(a)?;
                        if b == 0 {
                            break;
                        }
                        text.push(b as u8 as char);
                        a += 1;
                    }
                }
                node = root;
            }
            // Complex/indirect leaves (unichar, unicode string, indirect refs):
            // not previewed — elide, but keep walking so the extent stays correct.
            0x04 | 0x05 | 0x08 | 0x09 | 0x0A | 0x0B => {
                if let Some(n) = cap {
                    if text.chars().count() < n {
                        text.push('…');
                    }
                }
                node = root;
            }
            _ => return None, // bad node type
        }
    }
}

// ─── discovered units ─────────────────────────────────────────────────────────

/// A discovered function.
#[derive(Clone, Copy)]
struct FuncUnit {
    addr: u32,
    end: u32,
    type_byte: u8,
    locals_count: u32,
    first_instr: u32,
    tier: Tier, // Rd or Soft
}

/// A tiled unit of the code region.
#[derive(Clone, Copy)]
enum Unit {
    Func(FuncUnit),
    /// A string object `[addr, end)` with its E0/E1/E2 type byte.
    Str { addr: u32, end: u32, type_byte: u8 },
    /// An opaque data/padding run `[addr, end)`.
    Data { addr: u32, end: u32 },
}

impl Unit {
    fn addr(&self) -> u32 {
        match self {
            Unit::Func(f) => f.addr,
            Unit::Str { addr, .. } | Unit::Data { addr, .. } => *addr,
        }
    }
    fn end(&self) -> u32 {
        match self {
            Unit::Func(f) => f.end,
            Unit::Str { end, .. } | Unit::Data { end, .. } => *end,
        }
    }
}

/// Public per-string summary for the inspector's Strings list.
#[derive(Clone, Debug)]
pub struct StringInfo {
    pub addr: u32,
    /// `0xE0` (C string), `0xE1` (compressed), or `0xE2` (Unicode).
    pub type_byte: u8,
    /// A short, lossy decoded preview of the string's text.
    pub preview: String,
}

/// Public per-function summary for the inspector's function list.
#[derive(Clone, Debug)]
pub struct FuncInfo {
    pub addr: u32,
    /// `0xC0` (stack-args) or `0xC1` (locals-args).
    pub type_byte: u8,
    pub locals_count: u32,
    pub tier: Tier,
    /// Accelerated-function number if the VM has assigned one, else `None`.
    pub accel: Option<u32>,
}

// ─── the cache ────────────────────────────────────────────────────────────────

/// Lazy disassembly + discovery cache. Build it once (cheap discovery), then
/// render address windows on demand.
pub struct DisasmCache {
    /// Sorted, gapless tiling of `[region_start, region_end)`.
    units: Vec<Unit>,
    region_start: u32,
    region_end: u32,
    decode_table: u32,
    ramstart: u32,
    /// Executed instruction-start PCs seeded from the VM (ground-truth code;
    /// upgrades tier and enables RAM-code rendering). See `seed_executed`.
    executed: HashSet<u32>,
    /// Memoized per-function instruction-start boundaries (ROM is immutable, so
    /// these are stable). Keyed by function entry address.
    boundaries: RefCell<HashMap<u32, Vec<u32>>>,
}

impl DisasmCache {
    /// Discover functions and tile the ROM code region. Cheap: headers + a
    /// constant call graph + a type-validated linear scan. Does NOT render text.
    pub fn build(mem: &Memory) -> DisasmCache {
        let region_start = 36; // just past the 36-byte header
        let ramstart = mem.ramstart();
        let region_end = ramstart;
        let decode_table = mem.decode_table();

        // Pass 1 — RD: BFS the constant call graph from the start function.
        let rd = discover_rd(mem, mem.start_func(), region_start, region_end);

        // Pass 2 — linear tiling: walk [region_start, region_end), classifying
        // each address as a function, a string, or opaque data.
        let mut units: Vec<Unit> = Vec::new();
        let mut addr = region_start;
        while addr < region_end {
            if let Some(fh) = parse_func_header(mem, addr) {
                let end = scan_func_end(mem, decode_table, fh.first_instr, region_end);
                let tier = if rd.contains(&addr) { Tier::Rd } else { Tier::Soft };
                units.push(Unit::Func(FuncUnit {
                    addr,
                    end,
                    type_byte: fh.type_byte,
                    locals_count: fh.locals_count,
                    first_instr: fh.first_instr,
                    tier,
                }));
                addr = end;
            } else if let Some((ty, end)) = string_extent(mem, decode_table, addr) {
                units.push(Unit::Str { addr, end, type_byte: ty });
                addr = end.max(addr + 1);
            } else {
                // Data/padding: accumulate to the next func/string start.
                let start = addr;
                addr += 1;
                while addr < region_end
                    && parse_func_header(mem, addr).is_none()
                    && string_extent(mem, decode_table, addr).is_none()
                {
                    addr += 1;
                }
                units.push(Unit::Data { addr: start, end: addr });
            }
        }

        DisasmCache {
            units,
            region_start,
            region_end,
            decode_table,
            ramstart,
            executed: HashSet::new(),
            boundaries: RefCell::new(HashMap::new()),
        }
    }

    /// The `[start, end)` code region this cache tiles (ROM: header end → RAMSTART).
    pub fn region(&self) -> (u32, u32) {
        (self.region_start, self.region_end)
    }

    /// Seed the cumulative executed-PC set (ground truth: these bytes are code).
    /// Upgrades tier rendering and lets RAM-resident code be disassembled.
    pub fn seed_executed(&mut self, pcs: impl IntoIterator<Item = u32>) {
        self.executed.extend(pcs);
    }

    /// Record a single executed instruction-start PC (dynamic discovery). Returns
    /// true iff it was newly added. Mirrors zvm's `confirm_pc` in intent:
    /// execution is ground truth about where instructions start.
    pub fn confirm_pc(&mut self, pc: u32) -> bool {
        self.executed.insert(pc)
    }

    /// Every discovered function, sorted by address, tagged with its accel number
    /// (looked up in `accel`, the VM's function→accel-number map).
    pub fn functions(&self, accel: &HashMap<u32, u32>) -> Vec<FuncInfo> {
        self.units
            .iter()
            .filter_map(|u| match u {
                Unit::Func(f) => Some(FuncInfo {
                    addr: f.addr,
                    type_byte: f.type_byte,
                    locals_count: f.locals_count,
                    tier: f.tier,
                    accel: accel.get(&f.addr).copied(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Count of discovered string objects (cheap: the static tiling, no preview
    /// decoding). Lets a caller size/paginate its Strings list without paying to
    /// decode every preview.
    pub fn string_count(&self) -> usize {
        self.units.iter().filter(|u| matches!(u, Unit::Str { .. })).count()
    }

    /// Up to `limit` discovered string objects, in address order, each with a
    /// short preview — the inspector's Strings list. Discovery is static (the ROM
    /// tiling); previews are decoded here, only for the returned entries, so the
    /// cost is bounded by `limit` (a huge I7 image can hold tens of thousands of
    /// strings, and preview decoding an E1 stream is not free). Cold: only ever
    /// called while the inspector is open.
    pub fn strings(&self, mem: &Memory, limit: usize) -> Vec<StringInfo> {
        self.units
            .iter()
            .filter_map(|u| match u {
                Unit::Str { addr, type_byte, .. } => Some((*addr, *type_byte)),
                _ => None,
            })
            .take(limit)
            .map(|(addr, type_byte)| StringInfo {
                addr,
                type_byte,
                preview: self.string_preview(mem, addr).unwrap_or_default(),
            })
            .collect()
    }

    /// A short decoded preview of the string object at `addr` (E0/E1/E2), capped
    /// and lossy. `None` if `addr` is not a string object.
    pub fn string_preview(&self, mem: &Memory, addr: u32) -> Option<String> {
        const CAP: usize = 40;
        match mem.read8(addr)? {
            0xE0 => {
                let mut s = String::new();
                let mut a = addr + 1;
                while s.chars().count() < CAP {
                    let b = mem.read8(a)?;
                    if b == 0 {
                        break;
                    }
                    s.push(b as u8 as char);
                    a += 1;
                }
                Some(s)
            }
            0xE2 => {
                let mut s = String::new();
                let mut a = addr + 4;
                while s.chars().count() < CAP {
                    let w = mem.read32(a)?;
                    if w == 0 {
                        break;
                    }
                    s.push(char::from_u32(w).unwrap_or('\u{FFFD}'));
                    a += 4;
                }
                Some(s)
            }
            0xE1 => walk_e1(mem, self.decode_table, addr + 1, Some(CAP)).map(|(_, t)| t),
            _ => None,
        }
    }

    /// A hover/help description for the instruction at `addr`: the opcode's
    /// `opcode_help` text, or a "no description" fallback naming the mnemonic.
    pub fn describe(&self, mem: &Memory, addr: u32) -> Option<String> {
        let ins = decode_instr(mem, addr).ok()?;
        Some(match opcode_help(ins.opcode) {
            Some(h) => h.to_string(),
            None => {
                let mn = opcode_info(ins.opcode).map(|i| i.mnemonic).unwrap_or("?");
                format!("{mn} ({:#x}) — no description available.", ins.opcode)
            }
        })
    }

    /// Start address of the instruction after the one at `addr`.
    pub fn next_instr(&self, mem: &Memory, addr: u32) -> u32 {
        instr_len(mem, addr).unwrap_or(addr.saturating_add(1))
    }

    /// Start address of the instruction before `addr`. Found by re-scanning
    /// forward from the enclosing function's first instruction (a known
    /// boundary) — never by byte-guessing backward.
    pub fn prev_instr(&self, mem: &Memory, addr: u32) -> u32 {
        if let Some(f) = self.enclosing_func(addr) {
            let bounds = self.func_boundaries(mem, &f);
            let mut prev = f.addr; // fall back to the function entry
            for &b in &bounds {
                if b >= addr {
                    break;
                }
                prev = b;
            }
            return prev;
        }
        addr.saturating_sub(1)
    }

    /// Disassemble up to `lines` rows starting at `addr` (text only).
    pub fn disassemble(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        addr: u32,
        lines: usize,
    ) -> Vec<String> {
        self.disassemble_tiered(mem, accel, addr, lines)
            .into_iter()
            .map(|(row, _)| row)
            .collect()
    }

    /// Disassemble up to `lines` rows starting at `addr`, each tagged with its
    /// [`Tier`]. Rendering is lazy: only this window is formatted. An executed
    /// PC (seeded) always renders as an instruction tier `Rd`, overriding a
    /// static Data/Soft classification.
    pub fn disassemble_tiered(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        addr: u32,
        lines: usize,
    ) -> Vec<(String, Tier)> {
        let mut out = Vec::with_capacity(lines);
        if lines == 0 {
            return out;
        }
        // Inside a known function: render header (if at entry) + instructions.
        if let Some(f) = self.enclosing_func(addr) {
            self.render_from_func(mem, accel, &f, addr, lines, &mut out);
            // Continue into following units if room remains.
            let mut cursor = f.end;
            while out.len() < lines && cursor < self.region_end {
                self.render_unit_at(mem, accel, cursor, lines, &mut out);
                cursor = self.next_unit_start(cursor);
            }
            return out;
        }
        // Outside functions: walk units (string/data), or decode executed RAM code.
        let mut cursor = addr;
        while out.len() < lines {
            if cursor >= self.region_end && !self.executed.contains(&cursor) {
                break;
            }
            let advanced = self.render_unit_at(mem, accel, cursor, lines, &mut out);
            cursor = advanced;
            if cursor <= addr && out.len() >= lines {
                break;
            }
        }
        out
    }

    // ── internal rendering helpers ────────────────────────────────────────────

    /// Function whose `[addr, end)` contains `addr`.
    fn enclosing_func(&self, addr: u32) -> Option<FuncUnit> {
        // units are sorted; find the last unit starting at/<= addr.
        let idx = self.units.partition_point(|u| u.addr() <= addr);
        if idx == 0 {
            return None;
        }
        if let Unit::Func(f) = self.units[idx - 1] {
            if addr >= f.addr && addr < f.end {
                return Some(f);
            }
        }
        None
    }

    /// Start of the unit at/after `at` (clamped to region_end).
    fn next_unit_start(&self, at: u32) -> u32 {
        let idx = self.units.partition_point(|u| u.addr() <= at);
        self.units.get(idx).map(|u| u.addr()).unwrap_or(self.region_end)
    }

    /// Memoized instruction-start boundaries of a function (first_instr..end).
    fn func_boundaries(&self, mem: &Memory, f: &FuncUnit) -> Vec<u32> {
        if let Some(b) = self.boundaries.borrow().get(&f.addr) {
            return b.clone();
        }
        let mut bounds = Vec::new();
        let mut pc = f.first_instr;
        while pc < f.end {
            bounds.push(pc);
            match instr_len(mem, pc) {
                Ok(next) if next > pc => pc = next,
                _ => break,
            }
        }
        self.boundaries.borrow_mut().insert(f.addr, bounds.clone());
        bounds
    }

    /// Render a function starting at `from` (its header if `from == f.addr`,
    /// else its instructions from the boundary at/after `from`).
    fn render_from_func(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        f: &FuncUnit,
        from: u32,
        lines: usize,
        out: &mut Vec<(String, Tier)>,
    ) {
        if from <= f.addr {
            out.push((self.func_header_line(f, accel), f.tier));
            if out.len() >= lines {
                return;
            }
        }
        for b in self.func_boundaries(mem, f) {
            if b < from {
                continue;
            }
            if out.len() >= lines {
                return;
            }
            let tier = if self.executed.contains(&b) { Tier::Rd } else { f.tier };
            out.push((self.instr_line(mem, accel, b), tier));
        }
    }

    /// Render whatever unit starts at `at` (or an executed RAM instruction),
    /// returning the address just past what was rendered.
    fn render_unit_at(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        at: u32,
        lines: usize,
        out: &mut Vec<(String, Tier)>,
    ) -> u32 {
        // A known unit?
        let idx = self.units.partition_point(|u| u.addr() <= at);
        if idx > 0 {
            let u = self.units[idx - 1];
            if at >= u.addr() && at < u.end() {
                return match u {
                    Unit::Func(f) => {
                        self.render_from_func(mem, accel, &f, at, lines, out);
                        f.end
                    }
                    Unit::Str { addr, end, type_byte } => {
                        let preview = self.string_preview(mem, addr).unwrap_or_default();
                        let kind = match type_byte {
                            0xE0 => "string",
                            0xE1 => "string(E1)",
                            _ => "string(uni)",
                        };
                        out.push((format!("{addr:06x}  ; {kind} \"{}\"", escape(&preview)), Tier::Data));
                        end
                    }
                    Unit::Data { addr, end } => {
                        self.render_data(mem, addr, end, lines, out);
                        end
                    }
                };
            }
        }
        // Not in any unit: an executed RAM instruction, or unknown → decode/byte.
        if self.executed.contains(&at) || at >= self.ramstart {
            if let Ok(ins) = decode_instr(mem, at) {
                let tier = if self.executed.contains(&at) { Tier::Rd } else { Tier::Soft };
                out.push((self.instr_line(mem, accel, at), tier));
                return ins.next;
            }
        }
        let b = mem.read8(at).unwrap_or(0);
        out.push((format!("{at:06x}  .byte {b:#04x}"), Tier::Data));
        at + 1
    }

    fn render_data(&self, mem: &Memory, addr: u32, end: u32, lines: usize, out: &mut Vec<(String, Tier)>) {
        let mut row = addr;
        while row < end && out.len() < lines {
            let row_end = (row + 16).min(end);
            let hex = (row..row_end)
                .map(|a| format!("{:02x}", mem.read8(a).unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(" ");
            out.push((format!("{row:06x}  .byte {hex}"), Tier::Data));
            row += 16;
        }
    }

    fn func_header_line(&self, f: &FuncUnit, accel: &HashMap<u32, u32>) -> String {
        let kind = if f.type_byte == 0xC0 { "C0/stack" } else { "C1/locals" };
        let mut line = format!(
            "{:06x}  ; function {kind}, {} local{}",
            f.addr,
            f.locals_count,
            if f.locals_count == 1 { "" } else { "s" }
        );
        if let Some(&num) = accel.get(&f.addr) {
            line.push_str(&format!("  [accel: {}]", crate::accel::accel_name(num)));
        }
        line
    }

    /// Render one instruction row: `addr  mnemonic  ops   ; annotations`.
    fn instr_line(&self, mem: &Memory, accel: &HashMap<u32, u32>, addr: u32) -> String {
        let ins = match decode_instr(mem, addr) {
            Ok(i) => i,
            Err(_) => {
                let b = mem.read8(addr).unwrap_or(0);
                return format!("{addr:06x}  .byte {b:#04x}");
            }
        };
        let info = opcode_info(ins.opcode);
        let mnemonic = info.map(|i| i.mnemonic).unwrap_or("?");
        let ops = ins
            .operands
            .iter()
            .map(|o| self.format_operand(o))
            .collect::<Vec<_>>()
            .join(", ");
        let mut line = if ops.is_empty() {
            format!("{addr:06x}  {mnemonic}")
        } else {
            format!("{addr:06x}  {mnemonic} {ops}")
        };
        if let Some(ann) = self.annotate(mem, accel, &ins, info) {
            line.push_str(&format!("   ; {ann}"));
        }
        line
    }

    /// Format one operand mode-aware: `#const`, `Ln`, `sp`, `@addr`, `_`.
    fn format_operand(&self, o: &Operand) -> String {
        match o.mode & 0x0F {
            0x0 if o.is_store => "_".to_string(),
            0x0 => "#0".to_string(),
            0x8 => "sp".to_string(),
            0x1..=0x3 => format!("#{:#x}", o.value),
            0x5..=0x7 => format!("@{:#x}", o.value),
            0x9..=0xB => format!("L{}", o.value),
            0xD..=0xF => format!("@{:#x}", self.ramstart.wrapping_add(o.value)),
            _ => format!("?{:#x}", o.value),
        }
    }

    /// Build the trailing `; …` annotation for call/branch/glk/string sites.
    fn annotate(
        &self,
        mem: &Memory,
        _accel: &HashMap<u32, u32>,
        ins: &Instr,
        info: Option<OpInfo>,
    ) -> Option<String> {
        let info = info?;
        let const_op = |o: &Operand| -> Option<u32> {
            matches!(o.mode & 0x0F, 0x1..=0x3).then_some(o.value)
        };
        match info.special {
            Special::Call => {
                let t = const_op(ins.operands.first()?)?;
                Some(format!("-> fn@{t:#x}"))
            }
            Special::JumpAbs => {
                let t = const_op(ins.operands.first()?)?;
                Some(format!("-> {t:#x}"))
            }
            Special::Glk => {
                let sel = const_op(ins.operands.first()?)?;
                Some(crate::exec::glk_selector_name(sel))
            }
            Special::StreamStr => {
                let a = const_op(ins.operands.first()?)?;
                self.string_preview(mem, a).map(|p| format!("\"{}\"", escape(&p)))
            }
            Special::Branch | Special::Catch => {
                let idx = info.branch_index()?;
                let o = ins.operands.get(idx)?;
                // Only a constant offset resolves to a fixed target.
                if !matches!(o.mode & 0x0F, 0x1..=0x3) {
                    return None;
                }
                let off = signed_const(o.mode, o.value);
                match off {
                    0 => Some("-> return 0".to_string()),
                    1 => Some("-> return 1".to_string()),
                    // Branch bias (exec::branch): pc = next_pc + offset - 2.
                    _ => Some(format!("-> {:#x}", (ins.next as i64 + off - 2) as u32)),
                }
            }
            Special::None => None,
        }
    }
}

/// Escape control chars in a preview for single-line display.
fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

/// RD discovery: BFS the constant call graph from `start_func`, returning every
/// validated function entry reached. Each function's body is scanned linearly
/// (stopping at the next header / a decode error / the region end) to harvest
/// constant call targets.
fn discover_rd(mem: &Memory, start_func: u32, region_start: u32, region_end: u32) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    let mut work = vec![start_func];
    while let Some(a) = work.pop() {
        if a < region_start || a >= region_end || found.contains(&a) {
            continue;
        }
        let Some(fh) = parse_func_header(mem, a) else { continue };
        found.insert(a);
        // Scan this function's body for constant call targets.
        let mut pc = fh.first_instr;
        while pc < region_end {
            if pc != fh.first_instr && parse_func_header(mem, pc).is_some() {
                break; // reached the next function
            }
            let Ok(ins) = decode_instr(mem, pc) else { break };
            if let Some(info) = opcode_info(ins.opcode) {
                if info.special == Special::Call {
                    if let Some(o) = ins.operands.first() {
                        if matches!(o.mode & 0x0F, 0x1..=0x3) {
                            work.push(o.value);
                        }
                    }
                }
            }
            pc = ins.next;
        }
    }
    found
}

/// Scan a function body from `first_instr`, returning the end address (one past
/// the last instruction) — where the next function/string starts, a decode
/// error occurs, or the region ends.
fn scan_func_end(mem: &Memory, decode_table: u32, first_instr: u32, region_end: u32) -> u32 {
    let mut pc = first_instr;
    while pc < region_end {
        if pc != first_instr
            && (parse_func_header(mem, pc).is_some()
                || string_extent(mem, decode_table, pc).is_some())
        {
            break;
        }
        match instr_len(mem, pc) {
            Ok(next) if next > pc => pc = next,
            _ => {
                pc += 1;
                break;
            }
        }
    }
    pc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{self, Op::*};
    use crate::exec::{Machine, StepResult};
    use crate::glk::TestBackend;
    use crate::memory::Memory;

    fn machine(funcs: &[Vec<u8>], start: usize) -> Machine {
        let built = asm::assemble(funcs, start, 0x100);
        Machine::with_glk(Memory::new(built.image).unwrap(), Box::new(TestBackend::new()))
    }

    fn mem_of(funcs: &[Vec<u8>], start: usize) -> (Memory, Vec<u32>) {
        let built = asm::assemble(funcs, start, 0x100);
        (Memory::new(built.image).unwrap(), built.addrs)
    }

    /// A straight-line function: copy #5→L0, add L0 #3→L0, sub L0 #1→L0, return L0.
    fn straight_line() -> Vec<u8> {
        let body = [
            asm::ins(0x40, &[C32(5), Local8(0)]),
            asm::ins(0x10, &[Local8(0), C8(3), Local8(0)]),
            asm::ins(0x11, &[Local8(0), C8(1), Local8(0)]),
            asm::ins(0x31, &[Local8(0)]),
        ]
        .concat();
        asm::func(0xC1, &[(4, 1)], &body)
    }

    #[test]
    fn decode_round_trips_with_executor() {
        // The executor is the oracle: every PC it starts an instruction at must
        // decode to an instruction the disassembler agrees ends at the next PC.
        let mut m = machine(&[straight_line()], 0);
        let mut pcs = Vec::new();
        loop {
            pcs.push(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        assert_eq!(pcs.len(), 4, "copy, add, sub, return");
        for w in pcs.windows(2) {
            let ins = decode_instr(m.mem(), w[0]).expect("decodes");
            assert_eq!(ins.next, w[1], "disasm next must match executor advance");
        }
        assert!(decode_instr(m.mem(), pcs[3]).is_ok(), "return decodes");
    }

    #[test]
    fn discovers_functions_and_assigns_tiers() {
        // helper@0x24 (called by main), main (start), orphan (only scan-found).
        let helper = asm::func(0xC1, &[], &asm::ins(0x31, &[C8(7)]));
        let main = asm::func(
            0xC1,
            &[],
            &[asm::ins(0x160, &[C32(0x24), Zero]), asm::ins(0x31, &[C8(0)])].concat(),
        );
        let orphan = asm::func(0xC1, &[], &asm::ins(0x31, &[C8(9)]));
        let (mem, addrs) = mem_of(&[helper, main, orphan], 1);
        let cache = DisasmCache::build(&mem);
        let funcs = cache.functions(&HashMap::new());
        assert_eq!(funcs.len(), 3, "all three discovered");
        let tier = |a: u32| funcs.iter().find(|f| f.addr == a).unwrap().tier;
        assert_eq!(tier(addrs[0]), Tier::Rd, "helper is called → Rd");
        assert_eq!(tier(addrs[1]), Tier::Rd, "main is the start func → Rd");
        assert_eq!(tier(addrs[2]), Tier::Soft, "orphan only scan-found → Soft");
        // type byte + locals surfaced
        let helper_info = funcs.iter().find(|f| f.addr == addrs[0]).unwrap();
        assert_eq!(helper_info.type_byte, 0xC1);
        assert_eq!(helper_info.locals_count, 0);
    }

    #[test]
    fn annotates_glk_selector_name() {
        // glk #0x2A #0 -> _   (0x2A = glk_window_clear); then return #0.
        let body = [
            asm::ins(0x130, &[C16(0x2A), C8(0), Zero]),
            asm::ins(0x31, &[C8(0)]),
        ]
        .concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[], &body)], 0);
        let cache = DisasmCache::build(&mem);
        let text = cache.disassemble(&mem, &HashMap::new(), addrs[0], 8).join("\n");
        assert!(text.contains("glk_window_clear"), "got:\n{text}");
    }

    #[test]
    fn badges_accelerated_functions() {
        let (mem, addrs) = mem_of(&[straight_line()], 0);
        let cache = DisasmCache::build(&mem);
        let mut accel = HashMap::new();
        accel.insert(addrs[0], 1u32); // Z__Region
        let header = cache.disassemble(&mem, &accel, addrs[0], 1).remove(0);
        assert!(header.contains("[accel: Z__Region]"), "got: {header}");
    }

    #[test]
    fn string_preview_e0_e1_absent_and_present() {
        // Write an E0 C-string into writable RAM and preview it.
        let mut m = machine(&[straight_line()], 0);
        let base = m.mem().ramstart();
        m.mem.write8(base, 0xE0).unwrap();
        for (i, b) in b"Hi!".iter().enumerate() {
            m.mem.write8(base + 1 + i as u32, *b as u32).unwrap();
        }
        m.mem.write8(base + 4, 0).unwrap();
        let cache = DisasmCache::build(m.mem());
        assert_eq!(cache.string_preview(m.mem(), base).as_deref(), Some("Hi!"));
        // A non-string address previews as None.
        assert_eq!(cache.string_preview(m.mem(), base + 1), None);
    }

    #[test]
    fn lazy_window_renders_only_requested_lines() {
        let (mem, addrs) = mem_of(&[straight_line()], 0);
        let cache = DisasmCache::build(&mem);
        // From the function entry: header + N instruction rows, capped at `lines`.
        assert_eq!(cache.disassemble(&mem, &HashMap::new(), addrs[0], 1).len(), 1);
        assert_eq!(cache.disassemble(&mem, &HashMap::new(), addrs[0], 2).len(), 2);
        let three = cache.disassemble(&mem, &HashMap::new(), addrs[0], 3);
        assert!(three[0].contains("; function"), "first row is the header: {}", three[0]);
    }

    #[test]
    fn next_and_prev_instr_round_trip() {
        let mut m = machine(&[straight_line()], 0);
        let mut pcs = Vec::new();
        loop {
            pcs.push(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        let cache = DisasmCache::build(m.mem());
        for w in pcs.windows(2) {
            assert_eq!(cache.next_instr(m.mem(), w[0]), w[1]);
            assert_eq!(cache.prev_instr(m.mem(), w[1]), w[0]);
        }
    }

    #[test]
    fn describe_reports_opcode_help() {
        let (mem, addrs) = mem_of(&[straight_line()], 0);
        let cache = DisasmCache::build(&mem);
        // first_instr = header end; decode from the function entry's boundaries.
        let first = cache.next_instr(&mem, addrs[0]); // header byte 0 → first instr? No.
        // Walk to the first instruction (skip the header via a known boundary).
        let _ = first;
        // The copy is the first instruction; find it by disassembling.
        let mut m = machine(&[straight_line()], 0);
        let copy_pc = m.pc;
        m.step(); // execute copy
        let add_pc = m.pc;
        let d_copy = cache.describe(&mem, copy_pc).unwrap();
        assert!(d_copy.starts_with("copy"), "got: {d_copy}");
        let d_add = cache.describe(&mem, add_pc).unwrap();
        assert!(d_add.starts_with("add"), "got: {d_add}");
    }

    #[test]
    fn branch_target_resolves() {
        // jnz L0, back-by-9 → target = next_pc - 9 - 2. Build a self-loop body.
        let body = [
            asm::ins(0x40, &[C32(3), Local8(0)]),          // copy #3 -> L0  (7 bytes)
            asm::ins(0x11, &[Local8(0), C8(1), Local8(0)]), // sub           (6 bytes)
            asm::ins(0x23, &[Local8(0), C16(-9)]),         // jnz L0, -9
            asm::ins(0x31, &[C8(0)]),
        ]
        .concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[(4, 1)], &body)], 0);
        let cache = DisasmCache::build(&mem);
        let text = cache.disassemble(&mem, &HashMap::new(), addrs[0], 8).join("\n");
        // The sub sits at first_instr+7; jnz should resolve back to it.
        // Header = type byte + one (4,1) locals pair + (0,0) terminator = 5 bytes.
        let first_instr = addrs[0] + 1 + 2 + 2;
        let sub_addr = first_instr + 7;
        assert!(
            text.contains(&format!("-> {sub_addr:#x}")),
            "jnz target should resolve to the sub at {sub_addr:#x}\n{text}"
        );
    }

    #[test]
    fn real_image_decode_round_trip() {
        // Skip cleanly when the fixture isn't present.
        let path = "../gvm-cli/tests/fixtures/glulxercise.ulx";
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skip: {path} not found");
            return;
        };
        let mem = Memory::new(bytes).expect("valid Glulx image");
        let start_func = mem.start_func();
        let cache = DisasmCache::build(&mem);
        let funcs = cache.functions(&HashMap::new());
        assert!(!funcs.is_empty(), "discovery found functions");
        assert!(
            funcs.iter().any(|f| f.addr == start_func && f.tier == Tier::Rd),
            "start function is discovered and Rd"
        );
        // Executor as oracle: every PC it starts must decode as a valid start.
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        let mut pcs = std::collections::HashSet::new();
        for _ in 0..300_000 {
            pcs.insert(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        assert!(pcs.len() > 50, "exercised a meaningful number of instructions");
        let bad: Vec<u32> = pcs
            .iter()
            .copied()
            .filter(|&pc| decode_instr(m.mem(), pc).is_err())
            .collect();
        assert!(
            bad.is_empty(),
            "{} executor PCs failed to decode (first: {:?})",
            bad.len(),
            bad.first().map(|a| format!("{a:#x}"))
        );
    }
}
