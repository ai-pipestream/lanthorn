// Glulx execution engine — GLULX_NOTES.md §4 (stack, call frames, calling
// convention). Instruction decode/dispatch and the run loop are layered on in
// later tasks.
//
// The stack is a single byte-addressed buffer (`stack`) sized to the header's
// stack size, with a stack pointer `sp` (bytes used) and a frame pointer `fp`.
// A call frame is laid out exactly as the spec's diagram: FrameLen, LocalsPos,
// the locals-format list, the locals (each at natural alignment), then the
// value-stack region. A four-word "call stub" sits just below each non-start
// frame so a return can restore the caller.

use crate::error::GError;
use crate::io::Output;
use crate::memory::Memory;

/// A recoverable runtime fault. Carries a human-readable diagnostic; the run
/// loop records it and Quits rather than panicking.
type R<T> = Result<T, String>;

/// The outcome of a single [`Machine::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Execution should continue with the next instruction.
    Continue,
    /// Execution has ended (`quit`, an outer return, or a recorded fault).
    Quit,
}

/// Where a produced value (a function's return value, or an opcode's store
/// operand) should go. Mirrors the spec's call-stub DestType encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dest {
    /// Throw the value away (DestType 0).
    Discard,
    /// Push onto the value stack (DestType 3).
    Push,
    /// Store to main memory at this address (DestType 1).
    Mem(u32),
    /// Store to a call-frame local at this byte offset (DestType 2).
    Local(u32),
}

impl Dest {
    fn to_stub(self) -> (u32, u32) {
        match self {
            Dest::Discard => (0, 0),
            Dest::Mem(a) => (1, a),
            Dest::Local(a) => (2, a),
            Dest::Push => (3, 0),
        }
    }
}

/// The Glulx virtual machine.
pub struct Machine {
    pub(crate) mem: Memory,
    /// Byte-addressed stack (length = header stack size).
    stack: Vec<u8>,
    /// Stack pointer: number of bytes currently in use.
    sp: usize,
    /// Frame pointer: byte offset of the current call frame.
    fp: usize,
    /// Program counter (address in main memory).
    pub(crate) pc: u32,
    /// Current I/O system mode (0 null, 1 filter, 2 Glk).
    pub(crate) iosys_mode: u32,
    /// Current I/O system rock.
    pub(crate) iosys_rock: u32,
    /// Current string-decoding-table address (0 = none). Initialized from the
    /// header's decode_table; overridable by `setstringtbl`.
    pub(crate) cur_stringtbl: u32,
    /// Allocation-heap start address (0 = heap inactive). Set on the first
    /// `malloc` to the memsize at that moment.
    pub(crate) heap_start: u32,
    /// Extant allocated blocks `(addr, size)`, kept sorted by address.
    heap_blocks: Vec<(u32, u32)>,
    /// The output sink.
    pub(crate) out: Box<dyn Output>,
    /// Recorded runtime faults / deferred-feature notices.
    pub diagnostics: Vec<String>,
    /// Set once execution has ended (outer return or quit/fault).
    pub(crate) halted: bool,
    /// Protected RAM range `(addr, len)` preserved across restore/restoreundo;
    /// `len == 0` means no protection (set by the `protect` opcode).
    protect: (u32, u32),

    // Cached layout of the current frame (recomputed whenever `fp` changes).
    cur_frame_len: u32,
    cur_localspos: u32,
    /// `(offset_within_locals, size_bytes)` for each local of the current frame.
    cur_locals: Vec<(u32, u8)>,
}

fn align_up(v: u32, to: u32) -> u32 {
    (v + to - 1) / to * to
}

/// Append one IFF chunk (`id`, 4-byte big-endian length, data, even-pad).
fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0); // IFF chunks are padded to an even length
    }
}

/// Parse a `FORM IFZS` container into its `(id, data)` chunks. Never panics;
/// returns a [`GError::BadSave`] on any structural problem.
fn parse_ifzs(data: &[u8]) -> Result<Vec<([u8; 4], &[u8])>, GError> {
    let bad = |m: &str| GError::BadSave(m.to_string());
    if data.len() < 12 || &data[0..4] != b"FORM" || &data[8..12] != b"IFZS" {
        return Err(bad("not a FORM IFZS container"));
    }
    let form_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let end = (8 + form_len).min(data.len());
    let mut chunks = Vec::new();
    let mut p = 12;
    while p + 8 <= end {
        let id = [data[p], data[p + 1], data[p + 2], data[p + 3]];
        let len = u32::from_be_bytes([data[p + 4], data[p + 5], data[p + 6], data[p + 7]]) as usize;
        let start = p + 8;
        if start + len > data.len() {
            return Err(bad("chunk length overruns the data"));
        }
        chunks.push((id, &data[start..start + len]));
        p = start + len + (len & 1); // skip the even-pad byte
    }
    Ok(chunks)
}

impl Machine {
    /// Ceiling on the memory-map size; `malloc` fails rather than grow past it.
    const MAX_MEMSIZE: u32 = 0x1000_0000; // 256 MiB

    /// Build a machine over `mem`, entering the start function (no arguments).
    pub fn with_output(mem: Memory, out: Box<dyn Output>) -> Machine {
        let stack_len = (mem.stack_size().max(0x100)) as usize;
        let start = mem.start_func();
        let decode_table = mem.decode_table();
        let mut m = Machine {
            mem,
            stack: vec![0u8; stack_len],
            sp: 0,
            fp: 0,
            pc: 0,
            iosys_mode: 0,
            iosys_rock: 0,
            cur_stringtbl: decode_table,
            heap_start: 0,
            heap_blocks: Vec::new(),
            out,
            diagnostics: Vec::new(),
            halted: false,
            protect: (0, 0),
            cur_frame_len: 0,
            cur_localspos: 0,
            cur_locals: Vec::new(),
        };
        // Enter the start function directly (no call stub beneath it; its fp is 0).
        if let Err(msg) = m.build_frame_and_enter(start, &[]) {
            m.diagnostics.push(msg);
            m.halted = true;
        }
        m
    }

    // ── main-memory read helpers (fault on out-of-range) ──────────────────────

    fn m8(&self, a: u32) -> R<u32> {
        self.mem.read8(a).ok_or_else(|| format!("memory fault: read8 @{a:#010x}"))
    }
    fn m16(&self, a: u32) -> R<u32> {
        self.mem.read16(a).ok_or_else(|| format!("memory fault: read16 @{a:#010x}"))
    }
    fn m32(&self, a: u32) -> R<u32> {
        self.mem.read32(a).ok_or_else(|| format!("memory fault: read32 @{a:#010x}"))
    }

    // ── instruction-stream readers (advance pc) ───────────────────────────────

    fn take8(&mut self) -> R<u32> {
        let v = self.m8(self.pc)?;
        self.pc += 1;
        Ok(v)
    }
    fn take16(&mut self) -> R<u32> {
        let v = self.m16(self.pc)?;
        self.pc += 2;
        Ok(v)
    }
    fn take32(&mut self) -> R<u32> {
        let v = self.m32(self.pc)?;
        self.pc += 4;
        Ok(v)
    }

    // ── opcode + operand decode (GLULX_NOTES §3) ──────────────────────────────

    /// Decode the variable-length opcode number at `pc`, advancing past it.
    pub(crate) fn decode_opcode(&mut self) -> R<u32> {
        let b0 = self.m8(self.pc)?;
        match b0 & 0xC0 {
            0xC0 => Ok(self.take32()? - 0xC000_0000), // top bits 11 → 4 bytes
            0x80 => Ok(self.take16()? - 0x8000),      // top bits 10 → 2 bytes
            _ => self.take8(),                        // top bit 0 → 1 byte
        }
    }

    /// Decode `n_load` load values then `n_store` store destinations from the
    /// packed mode nibbles + operand data following `pc`.
    pub(crate) fn read_operands(&mut self, n_load: usize, n_store: usize) -> R<(Vec<u32>, Vec<Dest>)> {
        let total = n_load + n_store;
        let mode_bytes = total.div_ceil(2);
        // Slurp the mode nibbles first; operand data follows them, in order.
        let mut modes = Vec::with_capacity(total);
        let start = self.pc;
        for i in 0..mode_bytes {
            let byte = self.m8(start + i as u32)?;
            modes.push((byte & 0x0F) as u8);
            modes.push((byte >> 4) as u8);
        }
        self.pc += mode_bytes as u32;

        let mut loads = Vec::with_capacity(n_load);
        let mut stores = Vec::with_capacity(n_store);
        for (i, &mode) in modes.iter().enumerate().take(total) {
            if i < n_load {
                loads.push(self.resolve_load(mode)?);
            } else {
                stores.push(self.resolve_store(mode)?);
            }
        }
        Ok((loads, stores))
    }

    /// Resolve one LOAD operand of the given addressing mode to its value.
    fn resolve_load(&mut self, mode: u8) -> R<u32> {
        let ramstart = self.mem.ramstart();
        Ok(match mode {
            0x0 => 0,
            0x1 => self.take8()? as u8 as i8 as i32 as u32, // sign-extend
            0x2 => self.take16()? as u16 as i16 as i32 as u32,
            0x3 => self.take32()?,
            0x5 => {
                let a = self.take8()?;
                self.m32(a)?
            }
            0x6 => {
                let a = self.take16()?;
                self.m32(a)?
            }
            0x7 => {
                let a = self.take32()?;
                self.m32(a)?
            }
            0x8 => self.pop32()?,
            0x9 => {
                let o = self.take8()?;
                self.local_load(o)?
            }
            0xA => {
                let o = self.take16()?;
                self.local_load(o)?
            }
            0xB => {
                let o = self.take32()?;
                self.local_load(o)?
            }
            0xD => {
                let a = self.take8()? + ramstart;
                self.m32(a)?
            }
            0xE => {
                let a = self.take16()? + ramstart;
                self.m32(a)?
            }
            0xF => {
                let a = self.take32()? + ramstart;
                self.m32(a)?
            }
            other => return Err(format!("illegal load operand mode {other:#x}")),
        })
    }

    /// Resolve one STORE operand of the given addressing mode to a destination.
    fn resolve_store(&mut self, mode: u8) -> R<Dest> {
        let ramstart = self.mem.ramstart();
        Ok(match mode {
            0x0 => Dest::Discard,
            0x8 => Dest::Push,
            0x5 => Dest::Mem(self.take8()?),
            0x6 => Dest::Mem(self.take16()?),
            0x7 => Dest::Mem(self.take32()?),
            0x9 => Dest::Local(self.take8()?),
            0xA => Dest::Local(self.take16()?),
            0xB => Dest::Local(self.take32()?),
            0xD => Dest::Mem(self.take8()? + ramstart),
            0xE => Dest::Mem(self.take16()? + ramstart),
            0xF => Dest::Mem(self.take32()? + ramstart),
            other => return Err(format!("illegal store operand mode {other:#x}")),
        })
    }

    /// Write `v` to a resolved destination.
    pub(crate) fn store(&mut self, dest: Dest, v: u32) -> R<()> {
        match dest {
            Dest::Discard => Ok(()),
            Dest::Push => self.push32(v),
            Dest::Mem(a) => self.store_mem(a, v),
            Dest::Local(a) => self.local_store(a, v),
        }
    }

    // ── single-instruction execution ──────────────────────────────────────────

    /// Decode and execute one instruction. Grown task-by-task; `step()`/`run()`
    /// (the public loop with fault handling) wrap this in Task 6.
    pub(crate) fn step_once(&mut self) -> R<()> {
        let opcode = self.decode_opcode()?;
        self.execute(opcode)
    }

    /// Dispatch a decoded opcode to its handler.
    fn execute(&mut self, opcode: u32) -> R<()> {
        match opcode {
            // Control / I/O system.
            0x00 => Ok(()), // nop
            0x120 => {
                self.halted = true;
                Ok(())
            }
            0x148 => {
                let (_, s) = self.read_operands(0, 2)?;
                let (mode, rock) = (self.iosys_mode, self.iosys_rock);
                self.store(s[0], mode)?;
                self.store(s[1], rock)
            }
            0x149 => {
                let (l, _) = self.read_operands(2, 0)?;
                self.iosys_mode = l[0];
                self.iosys_rock = l[1];
                if l[0] == 1 {
                    self.diagnostics.push("filter iosys deferred to a later phase".to_string());
                }
                Ok(())
            }
            // Stream output.
            0x70 => self.op_streamchar(),
            0x71 => self.op_streamnum(),
            0x72 => self.op_streamstr(),
            0x73 => self.op_streamunichar(),
            0x130 => self.op_glk(),
            // String-decoding table (get/set the current table address).
            0x140 => {
                let (_, s) = self.read_operands(0, 1)?;
                let v = self.cur_stringtbl;
                self.store(s[0], v)
            }
            0x141 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.cur_stringtbl = l[0];
                Ok(())
            }
            // Arithmetic (2 load, 1 store) — 32-bit two's complement.
            0x10 => self.binop(|a, b| a.wrapping_add(b)),
            0x11 => self.binop(|a, b| a.wrapping_sub(b)),
            0x12 => self.binop(|a, b| a.wrapping_mul(b)),
            0x13 => self.divop(false),
            0x14 => self.divop(true),
            0x15 => self.unop(|a| (a as i32).wrapping_neg() as u32),
            // Bitwise.
            0x18 => self.binop(|a, b| a & b),
            0x19 => self.binop(|a, b| a | b),
            0x1A => self.binop(|a, b| a ^ b),
            0x1B => self.unop(|a| !a),
            0x1C => self.binop(|a, b| a.checked_shl(b).unwrap_or(0)),
            0x1D => self.binop(|a, b| (a as i32).checked_shr(b).unwrap_or((a as i32) >> 31) as u32),
            0x1E => self.binop(|a, b| a.checked_shr(b).unwrap_or(0)),
            // Branches.
            0x20 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.branch(l[0], true)
            }
            0x22 => self.branch1(|v| v == 0),
            0x23 => self.branch1(|v| v != 0),
            0x24 => self.branch2(|a, b| a == b),
            0x25 => self.branch2(|a, b| a != b),
            0x26 => self.branch2(|a, b| (a as i32) < (b as i32)),
            0x27 => self.branch2(|a, b| (a as i32) >= (b as i32)),
            0x28 => self.branch2(|a, b| (a as i32) > (b as i32)),
            0x29 => self.branch2(|a, b| (a as i32) <= (b as i32)),
            0x2A => self.branch2(|a, b| a < b),
            0x2B => self.branch2(|a, b| a >= b),
            0x2C => self.branch2(|a, b| a > b),
            0x2D => self.branch2(|a, b| a <= b),
            // Memory-array load/store (L2 is a signed index).
            0x48 => self.op_aload(4),
            0x49 => self.op_aload(2),
            0x4A => self.op_aload(1),
            0x4B => self.op_aloadbit(),
            0x4C => self.op_astore(4),
            0x4D => self.op_astore(2),
            0x4E => self.op_astore(1),
            0x4F => self.op_astorebit(),
            // Block zero / copy.
            0x170 => self.op_mzero(),
            0x171 => self.op_mcopy(),
            // Search opcodes.
            0x150 => self.op_linearsearch(),
            0x151 => self.op_binarysearch(),
            0x152 => self.op_linkedsearch(),
            // Capability query / image verify.
            0x100 => {
                let (l, s) = self.read_operands(2, 1)?;
                let v = self.gestalt(l[0], l[1]);
                self.store(s[0], v)
            }
            0x121 => {
                let (_, s) = self.read_operands(0, 1)?;
                self.store(s[0], 0) // verify: report success
            }
            // Copy / sign-extend.
            0x40 => self.unop(|a| a),                                  // copy
            0x41 => self.copy_sized(2),                                // copys
            0x42 => self.copy_sized(1),                                // copyb
            0x44 => self.unop(|a| a as u16 as i16 as i32 as u32),      // sexs
            0x45 => self.unop(|a| a as u8 as i8 as i32 as u32),        // sexb
            // Stack manipulation.
            0x50 => {
                let (_, s) = self.read_operands(0, 1)?;
                let c = self.value_count();
                self.store(s[0], c)
            }
            0x51 => self.op_stkpeek(),
            0x52 => self.op_stkswap(),
            0x53 => self.op_stkroll(),
            0x54 => self.op_stkcopy(),
            // Memory size.
            0x102 => {
                let (_, s) = self.read_operands(0, 1)?;
                let v = self.mem.mem_size();
                self.store(s[0], v)
            }
            0x103 => {
                let (l, s) = self.read_operands(1, 1)?;
                let r = if self.heap_start != 0 {
                    self.diagnostics
                        .push("setmemsize while the heap is active is illegal".to_string());
                    1
                } else {
                    u32::from(self.mem.set_mem_size(l[0]).is_err())
                };
                self.store(s[0], r)
            }
            // Allocation heap.
            0x178 => {
                let (l, s) = self.read_operands(1, 1)?;
                let a = self.heap_malloc(l[0]);
                self.store(s[0], a)
            }
            0x179 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.heap_free(l[0])
            }
            // Calls / return.
            0x30 => self.op_call(),
            0x31 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.return_value(l[0])
            }
            0x34 => self.op_tailcall(),
            0x160 => self.op_callf(0),
            0x161 => self.op_callf(1),
            0x162 => self.op_callf(2),
            0x163 => self.op_callf(3),
            other => Err(format!("illegal/unimplemented opcode {other:#x}")),
        }
    }

    fn binop(&mut self, f: impl Fn(u32, u32) -> u32) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let v = f(l[0], l[1]);
        self.store(s[0], v)
    }

    fn unop(&mut self, f: impl Fn(u32) -> u32) -> R<()> {
        let (l, s) = self.read_operands(1, 1)?;
        let v = f(l[0]);
        self.store(s[0], v)
    }

    /// `div` (false) / `mod` (true): signed, truncating toward zero, faulting on
    /// a zero divisor.
    fn divop(&mut self, is_mod: bool) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (a, b) = (l[0] as i32, l[1] as i32);
        if b == 0 {
            return Err(format!("division by zero ({})", if is_mod { "mod" } else { "div" }));
        }
        let v = if is_mod { a.wrapping_rem(b) } else { a.wrapping_div(b) };
        self.store(s[0], v as u32)
    }

    /// A 1-value conditional branch (jz/jnz): value then offset.
    fn branch1(&mut self, cond: impl Fn(u32) -> bool) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let taken = cond(l[0]);
        self.branch(l[1], taken)
    }

    /// A 2-value conditional branch (jeq…jleu): two values then offset.
    fn branch2(&mut self, cond: impl Fn(u32, u32) -> bool) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let taken = cond(l[0], l[1]);
        self.branch(l[2], taken)
    }

    /// Apply the branch convention: offset 0 → return 0, 1 → return 1, else
    /// `pc = pc_after_operands + offset - 2`. `pc` is already past the operands.
    fn branch(&mut self, offset: u32, taken: bool) -> R<()> {
        if !taken {
            return Ok(());
        }
        match offset {
            0 => self.return_value(0),
            1 => self.return_value(1),
            _ => {
                self.pc = self.pc.wrapping_add(offset).wrapping_sub(2);
                Ok(())
            }
        }
    }

    // ── stack byte accessors (offsets are guaranteed in-range by callers) ─────

    fn st_w32(&mut self, off: usize, v: u32) {
        self.stack[off..off + 4].copy_from_slice(&v.to_be_bytes());
    }
    fn st_r32(&self, off: usize) -> u32 {
        u32::from_be_bytes([
            self.stack[off],
            self.stack[off + 1],
            self.stack[off + 2],
            self.stack[off + 3],
        ])
    }

    // ── value stack (operands above the current frame) ────────────────────────

    /// Byte offset where the current frame's value stack begins.
    fn value_base(&self) -> usize {
        self.fp + self.cur_frame_len as usize
    }

    /// Push a 32-bit value onto the value stack.
    pub(crate) fn push32(&mut self, v: u32) -> R<()> {
        if self.sp + 4 > self.stack.len() {
            return Err("stack overflow".to_string());
        }
        let off = self.sp;
        self.st_w32(off, v);
        self.sp += 4;
        Ok(())
    }

    /// Pop a 32-bit value off the value stack.
    pub(crate) fn pop32(&mut self) -> R<u32> {
        if self.sp < self.value_base() + 4 {
            return Err("stack underflow".to_string());
        }
        self.sp -= 4;
        Ok(self.st_r32(self.sp))
    }

    /// Number of 32-bit values currently on the frame's value stack.
    pub(crate) fn value_count(&self) -> u32 {
        ((self.sp - self.value_base()) / 4) as u32
    }

    // ── call-frame locals (size-aware, per the locals format) ─────────────────

    fn local_size(&self, offset: u32) -> R<u8> {
        self.cur_locals
            .iter()
            .find(|(o, _)| *o == offset)
            .map(|(_, s)| *s)
            .ok_or_else(|| format!("bad local offset {offset:#x}"))
    }

    /// Read a local at byte `offset` within the locals region (size from format).
    pub(crate) fn local_load(&self, offset: u32) -> R<u32> {
        let size = self.local_size(offset)?;
        let base = self.fp + self.cur_localspos as usize + offset as usize;
        Ok(match size {
            1 => self.stack[base] as u32,
            2 => ((self.stack[base] as u32) << 8) | self.stack[base + 1] as u32,
            _ => self.st_r32(base),
        })
    }

    /// Write a local at byte `offset` (truncated to the local's declared size).
    pub(crate) fn local_store(&mut self, offset: u32, v: u32) -> R<()> {
        let size = self.local_size(offset)?;
        let base = self.fp + self.cur_localspos as usize + offset as usize;
        match size {
            1 => self.stack[base] = v as u8,
            2 => {
                self.stack[base] = (v >> 8) as u8;
                self.stack[base + 1] = v as u8;
            }
            _ => self.st_w32(base, v),
        }
        Ok(())
    }

    // ── frame construction ────────────────────────────────────────────────────

    /// Recompute `cur_frame_len`/`cur_localspos`/`cur_locals` from the frame
    /// header bytes at `fp`.
    fn reload_frame_meta(&mut self) -> R<()> {
        let fp = self.fp;
        if fp + 8 > self.stack.len() {
            return Err("corrupt frame pointer".to_string());
        }
        self.cur_frame_len = self.st_r32(fp);
        self.cur_localspos = self.st_r32(fp + 4);
        // Parse the locals-format pairs that begin at fp+8.
        let mut locals = Vec::new();
        let mut p = fp + 8;
        let mut off = 0u32;
        loop {
            if p + 2 > self.stack.len() {
                return Err("corrupt locals format".to_string());
            }
            let t = self.stack[p];
            let c = self.stack[p + 1];
            p += 2;
            if t == 0 && c == 0 {
                break;
            }
            for _ in 0..c {
                off = align_up(off, t as u32);
                locals.push((off, t));
                off += t as u32;
            }
        }
        self.cur_locals = locals;
        Ok(())
    }

    /// Build a call frame for the function at `func_addr` at the current `sp`,
    /// install it as the current frame, copy/push `args` per the function type,
    /// and set `pc` to the first instruction. Does NOT push a call stub — the
    /// caller does that for non-start calls.
    fn build_frame_and_enter(&mut self, func_addr: u32, args: &[u32]) -> R<()> {
        let func_type = self.m8(func_addr)?;
        if func_type != 0xC0 && func_type != 0xC1 {
            return Err(format!("not a function: type byte {func_type:#x} @{func_addr:#x}"));
        }

        // Parse the locals format; compute the format byte length and the
        // per-local (offset, size) layout.
        let mut pairs: Vec<(u8, u8)> = Vec::new();
        let mut addr = func_addr + 1;
        loop {
            let t = self.m8(addr)? as u8;
            let c = self.m8(addr + 1)? as u8;
            addr += 2;
            if t == 0 && c == 0 {
                break;
            }
            if t != 1 && t != 2 && t != 4 {
                return Err(format!("bad local type {t} @{func_addr:#x}"));
            }
            pairs.push((t, c));
        }
        let format_len = (pairs.len() + 1) * 2; // includes the (0,0) terminator
        let localspos = align_up(8 + format_len as u32, 4);

        // Lay out the locals (each at its natural alignment).
        let mut locals_layout: Vec<(u32, u8)> = Vec::new();
        let mut off = 0u32;
        for &(t, c) in &pairs {
            for _ in 0..c {
                off = align_up(off, t as u32);
                locals_layout.push((off, t));
                off += t as u32;
            }
        }
        let locals_size = off;
        let frame_len = align_up(localspos + locals_size, 4);

        let frameptr = self.sp;
        if frameptr + frame_len as usize > self.stack.len() {
            return Err("stack overflow building call frame".to_string());
        }

        // Zero the whole frame region, then write the header + format list.
        for b in &mut self.stack[frameptr..frameptr + frame_len as usize] {
            *b = 0;
        }
        self.st_w32(frameptr, frame_len);
        self.st_w32(frameptr + 4, localspos);
        for (i, &(t, c)) in pairs.iter().enumerate() {
            self.stack[frameptr + 8 + 2 * i] = t;
            self.stack[frameptr + 8 + 2 * i + 1] = c;
        }
        // (terminator pair already zero from the wipe above)

        self.fp = frameptr;
        self.sp = frameptr + frame_len as usize;
        self.pc = addr;
        self.cur_frame_len = frame_len;
        self.cur_localspos = localspos;
        self.cur_locals = locals_layout;

        // Pass the arguments.
        if func_type == 0xC1 {
            // Copy args into locals in order, truncated to each local's size.
            let layout = self.cur_locals.clone();
            for (i, &(loff, _sz)) in layout.iter().enumerate() {
                if i >= args.len() {
                    break;
                }
                self.local_store(loff, args[i])?;
            }
        } else {
            // C0: push args (last first → first on top), then the count.
            for &a in args.iter().rev() {
                self.push32(a)?;
            }
            self.push32(args.len() as u32)?;
        }
        Ok(())
    }

    /// Call the function at `func_addr` with `args`, storing its eventual return
    /// value per `dest`. Pushes a call stub, then builds the callee frame.
    pub(crate) fn call_function(&mut self, func_addr: u32, args: &[u32], dest: Dest) -> R<()> {
        let (dtype, daddr) = dest.to_stub();
        let ret_pc = self.pc;
        let caller_fp = self.fp as u32;
        // Push the stub: DestType, DestAddr, PC, FramePtr (FramePtr ends on top).
        self.push32(dtype)?;
        self.push32(daddr)?;
        self.push32(ret_pc)?;
        self.push32(caller_fp)?;
        self.build_frame_and_enter(func_addr, args)
    }

    /// Return `v` from the current function. Pops the frame and its call stub,
    /// restores the caller, and stores `v` per the stub's destination. Returning
    /// from the start frame (`fp == 0`) halts the machine.
    pub(crate) fn return_value(&mut self, v: u32) -> R<()> {
        if self.fp == 0 {
            self.halted = true;
            return Ok(());
        }
        // Discard the current frame (and any pushed values).
        self.sp = self.fp;
        // Pop the stub: FramePtr, PC, DestAddr, DestType (reverse of the push).
        if self.sp < 16 {
            return Err("corrupt call stub on return".to_string());
        }
        self.sp -= 4;
        let caller_fp = self.st_r32(self.sp);
        self.sp -= 4;
        let ret_pc = self.st_r32(self.sp);
        self.sp -= 4;
        let daddr = self.st_r32(self.sp);
        self.sp -= 4;
        let dtype = self.st_r32(self.sp);

        self.fp = caller_fp as usize;
        self.pc = ret_pc;
        self.reload_frame_meta()?;

        match dtype {
            0 => {} // discard
            1 => self.store_mem(daddr, v)?,
            2 => self.local_store(daddr, v)?,
            3 => self.push32(v)?,
            other => return Err(format!("bad call-stub DestType {other}")),
        }
        Ok(())
    }

    /// Write a 32-bit `v` to main memory at `addr`.
    pub(crate) fn store_mem(&mut self, addr: u32, v: u32) -> R<()> {
        self.store_mem_sized(addr, v, 4)
    }

    /// Write the low `width` bytes of `v` to main memory at `addr`, mapping
    /// ROM/out-of-range faults to a diagnostic (ROM) or a fault (out of range).
    fn store_mem_sized(&mut self, addr: u32, v: u32, width: u32) -> R<()> {
        use crate::memory::WriteFault;
        let res = match width {
            1 => self.mem.write8(addr, v),
            2 => self.mem.write16(addr, v),
            _ => self.mem.write32(addr, v),
        };
        match res {
            Ok(()) => Ok(()),
            Err(WriteFault::Rom) => {
                self.diagnostics.push(format!("ignored ROM write @{addr:#010x}"));
                Ok(())
            }
            Err(WriteFault::OutOfRange) => Err(format!("memory fault: write @{addr:#010x}")),
        }
    }

    fn read_width(&self, addr: u32, width: u32) -> R<u32> {
        match width {
            1 => self.m8(addr),
            2 => self.m16(addr),
            _ => self.m32(addr),
        }
    }

    // ── copys/copyb (width-sized copies; no sign extension) ───────────────────

    /// `copys` (width 2) / `copyb` (width 1): copy a sub-word value. Memory
    /// operands access `width` bytes; stack operands stay 32-bit. (Local
    /// operands — deprecated — are read/written at the local's declared size.)
    fn copy_sized(&mut self, width: u32) -> R<()> {
        let mode_byte = self.take8()? as u8;
        let lmode = mode_byte & 0x0F;
        let smode = mode_byte >> 4;
        let v = self.resolve_load_sized(lmode, width)?;
        let dest = self.resolve_store(smode)?;
        match dest {
            Dest::Mem(a) => self.store_mem_sized(a, v, width),
            other => self.store(other, v),
        }
    }

    fn resolve_load_sized(&mut self, mode: u8, width: u32) -> R<u32> {
        let ramstart = self.mem.ramstart();
        let mask = if width >= 4 { u32::MAX } else { (1u32 << (width * 8)) - 1 };
        let v = match mode {
            0x0 => 0,
            0x1 => self.take8()?, // zero-extended (no sign extension for copys/copyb)
            0x2 => self.take16()?,
            0x3 => self.take32()?,
            0x5 => {
                let a = self.take8()?;
                self.read_width(a, width)?
            }
            0x6 => {
                let a = self.take16()?;
                self.read_width(a, width)?
            }
            0x7 => {
                let a = self.take32()?;
                self.read_width(a, width)?
            }
            0x8 => self.pop32()?,
            0x9 => {
                let o = self.take8()?;
                self.local_load(o)?
            }
            0xA => {
                let o = self.take16()?;
                self.local_load(o)?
            }
            0xB => {
                let o = self.take32()?;
                self.local_load(o)?
            }
            0xD => {
                let a = self.take8()? + ramstart;
                self.read_width(a, width)?
            }
            0xE => {
                let a = self.take16()? + ramstart;
                self.read_width(a, width)?
            }
            0xF => {
                let a = self.take32()? + ramstart;
                self.read_width(a, width)?
            }
            other => return Err(format!("illegal load operand mode {other:#x}")),
        };
        Ok(v & mask)
    }

    // ── memory-array opcodes (GLULX_NOTES; L2 is a signed index) ──────────────

    /// `aload`/`aloads`/`aloadb` (width 4/2/1): load from `L1 + width*L2`
    /// (L2 signed), zero-extended to 32 bits.
    fn op_aload(&mut self, width: u32) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let addr = Self::array_addr(l[0], l[1], width);
        let v = self.read_width(addr, width)?;
        self.store(s[0], v)
    }

    /// `astore`/`astores`/`astoreb` (width 4/2/1): store the low `width` bytes
    /// of L3 at `L1 + width*L2` (L2 signed).
    fn op_astore(&mut self, width: u32) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let addr = Self::array_addr(l[0], l[1], width);
        self.store_mem_sized(addr, l[2], width)
    }

    /// `aloadbit`: store bit `(L2 mod 8)` of byte `(L1 + L2/8)` (L2 signed,
    /// flooring division) as 0/1.
    fn op_aloadbit(&mut self) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (addr, bit) = Self::bit_addr(l[0], l[1]);
        let byte = self.m8(addr)?;
        self.store(s[0], (byte >> bit) & 1)
    }

    /// `astorebit`: set (L3 nonzero) or clear (L3 zero) bit `(L2 mod 8)` of byte
    /// `(L1 + L2/8)` (L2 signed).
    fn op_astorebit(&mut self) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let (addr, bit) = Self::bit_addr(l[0], l[1]);
        let byte = self.m8(addr)?;
        let v = if l[2] != 0 { byte | (1 << bit) } else { byte & !(1 << bit) };
        self.store_mem_sized(addr, v, 1)
    }

    /// Compute `base + scale*index` with `index` taken as signed.
    fn array_addr(base: u32, index: u32, scale: u32) -> u32 {
        (base as i64 + scale as i64 * (index as i32 as i64)) as u32
    }

    /// Compute `(byte_address, bit_number)` for the signed bit index `L2`.
    fn bit_addr(base: u32, index: u32) -> (u32, u32) {
        let idx = index as i32;
        let byte = (base as i64 + idx.div_euclid(8) as i64) as u32;
        (byte, idx.rem_euclid(8) as u32)
    }

    /// `mzero count addr`: zero `count` bytes starting at `addr`.
    fn op_mzero(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let (count, addr) = (l[0], l[1]);
        for i in 0..count {
            self.store_mem_sized(addr + i, 0, 1)?;
        }
        Ok(())
    }

    /// `mcopy count from to`: copy `count` bytes, choosing the copy direction so
    /// overlapping ranges move correctly (spec §2.6).
    fn op_mcopy(&mut self) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let (count, from, to) = (l[0], l[1], l[2]);
        if to < from {
            for i in 0..count {
                let b = self.m8(from + i)?;
                self.store_mem_sized(to + i, b, 1)?;
            }
        } else {
            for i in (0..count).rev() {
                let b = self.m8(from + i)?;
                self.store_mem_sized(to + i, b, 1)?;
            }
        }
        Ok(())
    }

    // ── gestalt (GLULX_NOTES §13) ─────────────────────────────────────────────

    /// Version of the Glulx spec this VM targets (3.1.2).
    const GLULX_VERSION: u32 = 0x0003_0102;
    /// This interpreter's own version (0.1.0).
    const TERP_VERSION: u32 = 0x0000_0100;

    /// Report the capability for gestalt selector `sel` (with argument `arg`).
    /// Returned values reflect what this VM actually implements; unimplemented
    /// selectors and deferred features return 0.
    fn gestalt(&self, sel: u32, arg: u32) -> u32 {
        match sel {
            0 => Self::GLULX_VERSION,                  // GlulxVersion
            1 => Self::TERP_VERSION,                   // TerpVersion
            2 => 1,                                    // ResizeMem
            3 => 0,                                    // Undo (2c)
            4 => u32::from(arg == 0 || arg == 2),      // IOSystem: null + Glk
            5 => 1,                                    // Unicode
            6 => 1,                                    // MemCopy
            7 => 1,                                    // MAlloc
            8 => self.heap_start,                      // MAllocHeap (0 if inactive)
            9 => 0,                                    // Acceleration (2c)
            10 => 0,                                   // AccelFunc (2c)
            11 => 0,                                   // Float
            _ => 0,
        }
    }

    // ── save / restore serialization core (GLULX_NOTES §14) ───────────────────

    /// Serialize the VM's mutable state as Glulx-Quetzal bytes (`FORM IFZS`:
    /// `IFhd` identity, `CMem` compressed RAM, `Stks` stack, `MAll` heap, plus a
    /// `GReg` register chunk). Round-trips exactly via [`Machine::restore_state`].
    pub fn save_state(&self) -> Vec<u8> {
        let mut body = Vec::new();

        // IFhd: the first 128 bytes of memory (identity).
        let mut ifhd = Vec::with_capacity(128);
        for a in 0..128 {
            ifhd.push(self.mem.read8(a).unwrap_or(0) as u8);
        }
        push_chunk(&mut body, b"IFhd", &ifhd);

        // CMem: current memsize, then the RLE-compressed diff against the
        // original image over [RAMSTART, memsize).
        push_chunk(&mut body, b"CMem", &self.compress_ram());

        // Stks: the live stack bytes [0, sp).
        push_chunk(&mut body, b"Stks", &self.stack[..self.sp]);

        // MAll: heap-start, block count, then (addr, len) per block.
        let mut mall = Vec::new();
        mall.extend_from_slice(&self.heap_start.to_be_bytes());
        mall.extend_from_slice(&(self.heap_blocks.len() as u32).to_be_bytes());
        for &(a, sz) in &self.heap_blocks {
            mall.extend_from_slice(&a.to_be_bytes());
            mall.extend_from_slice(&sz.to_be_bytes());
        }
        push_chunk(&mut body, b"MAll", &mall);

        // GReg: registers not derivable from the stack (sp, fp, pc, iosys,
        // string table, protect range).
        let mut greg = Vec::with_capacity(32);
        for v in [
            self.sp as u32,
            self.fp as u32,
            self.pc,
            self.iosys_mode,
            self.iosys_rock,
            self.cur_stringtbl,
            self.protect.0,
            self.protect.1,
        ] {
            greg.extend_from_slice(&v.to_be_bytes());
        }
        push_chunk(&mut body, b"GReg", &greg);

        let mut out = Vec::with_capacity(body.len() + 12);
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"IFZS");
        out.extend_from_slice(&body);
        out
    }

    /// RLE-compress the RAM diff `[RAMSTART, memsize)` against the original
    /// image (`CMem` body: 4-byte memsize then the compressed bytes). A run of
    /// 1..=256 zero bytes is `0x00` followed by `(count-1)`; non-zero diff bytes
    /// are literal (spec §1.8 / Quetzal).
    fn compress_ram(&self) -> Vec<u8> {
        let ramstart = self.mem.ramstart();
        let memsize = self.mem.mem_size();
        let mut diff = Vec::with_capacity((memsize - ramstart) as usize);
        for a in ramstart..memsize {
            let cur = self.mem.read8(a).unwrap_or(0) as u8;
            diff.push(cur ^ self.mem.orig_byte(a));
        }
        let mut out = Vec::new();
        out.extend_from_slice(&memsize.to_be_bytes());
        let mut i = 0;
        while i < diff.len() {
            if diff[i] == 0 {
                let mut run = 1usize;
                while i + run < diff.len() && diff[i + run] == 0 && run < 256 {
                    run += 1;
                }
                out.push(0);
                out.push((run - 1) as u8);
                i += run;
            } else {
                out.push(diff[i]);
                i += 1;
            }
        }
        out
    }

    /// Restore VM state from [`Machine::save_state`] bytes. Resets RAM to the
    /// original image, applies the saved diff, rebuilds the stack/heap/registers,
    /// preserves the protected range, and recomputes the frame cache. Corrupt or
    /// truncated data yields a [`GError`] (never a panic).
    pub fn restore_state(&mut self, data: &[u8]) -> Result<(), GError> {
        let chunks = parse_ifzs(data)?;
        let find = |id: &[u8; 4]| chunks.iter().find(|(cid, _)| cid == id).map(|(_, d)| *d);
        let cmem = find(b"CMem").ok_or_else(|| GError::BadSave("missing CMem chunk".into()))?;
        let stks = find(b"Stks").ok_or_else(|| GError::BadSave("missing Stks chunk".into()))?;
        let greg = find(b"GReg").ok_or_else(|| GError::BadSave("missing GReg chunk".into()))?;
        if greg.len() != 32 {
            return Err(GError::BadSave("GReg chunk has wrong length".into()));
        }
        let g = |i: usize| u32::from_be_bytes([greg[i], greg[i + 1], greg[i + 2], greg[i + 3]]);
        let (sp, fp, pc) = (g(0), g(4), g(8));
        let (iosys_mode, iosys_rock, stringtbl) = (g(12), g(16), g(20));
        let (paddr, plen) = (g(24), g(28));

        // Snapshot the currently-protected bytes (preserved across restore).
        let (cur_paddr, cur_plen) = self.protect;
        let mut protected: Vec<(u32, u8)> = Vec::new();
        for a in cur_paddr..cur_paddr.saturating_add(cur_plen) {
            if let Some(b) = self.mem.read8(a) {
                protected.push((a, b as u8));
            }
        }

        // Reset RAM to the original image and apply the saved CMem diff.
        self.decompress_ram(cmem)?;

        // Re-impose the protected bytes' pre-restore values.
        for (a, b) in protected {
            if a >= self.mem.ramstart() && a < self.mem.mem_size() {
                self.mem.write_byte_raw(a, b);
            }
        }

        // Stack: the Stks bytes must fit the buffer and match the saved sp.
        if stks.len() != sp as usize {
            return Err(GError::BadSave("Stks length disagrees with sp".into()));
        }
        if sp as usize > self.stack.len() {
            return Err(GError::BadSave("saved sp exceeds the stack size".into()));
        }
        self.stack[..sp as usize].copy_from_slice(stks);
        self.sp = sp as usize;
        self.fp = fp as usize;
        self.pc = pc;
        self.iosys_mode = iosys_mode;
        self.iosys_rock = iosys_rock;
        self.cur_stringtbl = stringtbl;
        self.protect = (paddr, plen);

        // Heap: rebuild from MAll (absent/empty → inactive).
        self.heap_start = 0;
        self.heap_blocks.clear();
        if let Some(mall) = find(b"MAll") {
            self.restore_heap(mall)?;
        }

        // Recompute the current-frame cache from the restored stack.
        self.reload_frame_meta().map_err(GError::BadSave)?;
        Ok(())
    }

    /// Decompress a `CMem` body into RAM: read the saved memsize, resize memory,
    /// and rebuild `[RAMSTART, memsize)` as `original XOR diff`. Faults on a
    /// truncated/over-long stream.
    fn decompress_ram(&mut self, cmem: &[u8]) -> Result<(), GError> {
        if cmem.len() < 4 {
            return Err(GError::BadSave("CMem chunk too short".into()));
        }
        let memsize = u32::from_be_bytes([cmem[0], cmem[1], cmem[2], cmem[3]]);
        let ramstart = self.mem.ramstart();
        if memsize < ramstart || memsize > Self::MAX_MEMSIZE {
            return Err(GError::BadSave("CMem memsize out of range".into()));
        }
        self.mem.set_raw_size(memsize);
        let mut addr = ramstart;
        let mut i = 4;
        while addr < memsize {
            if i >= cmem.len() {
                return Err(GError::BadSave("CMem data truncated".into()));
            }
            let b = cmem[i];
            i += 1;
            if b == 0 {
                if i >= cmem.len() {
                    return Err(GError::BadSave("CMem zero-run truncated".into()));
                }
                let run = cmem[i] as u32 + 1;
                i += 1;
                for _ in 0..run {
                    if addr >= memsize {
                        return Err(GError::BadSave("CMem data overruns memory".into()));
                    }
                    let base = self.mem.orig_byte(addr);
                    self.mem.write_byte_raw(addr, base);
                    addr += 1;
                }
            } else {
                let base = self.mem.orig_byte(addr);
                self.mem.write_byte_raw(addr, base ^ b);
                addr += 1;
            }
        }
        Ok(())
    }

    /// Rebuild the allocation heap from a `MAll` chunk body.
    fn restore_heap(&mut self, mall: &[u8]) -> Result<(), GError> {
        if mall.len() < 8 {
            if mall.is_empty() {
                return Ok(()); // omitted/empty heap → inactive
            }
            return Err(GError::BadSave("MAll chunk too short".into()));
        }
        let rd = |i: usize| u32::from_be_bytes([mall[i], mall[i + 1], mall[i + 2], mall[i + 3]]);
        let heap_start = rd(0);
        let nblocks = rd(4) as usize;
        if mall.len() != 8 + nblocks * 8 {
            return Err(GError::BadSave("MAll block count disagrees with length".into()));
        }
        if heap_start == 0 && nblocks == 0 {
            return Ok(()); // inactive
        }
        let mut blocks = Vec::with_capacity(nblocks);
        for k in 0..nblocks {
            let base = 8 + k * 8;
            blocks.push((rd(base), rd(base + 4)));
        }
        self.heap_start = heap_start;
        self.heap_blocks = blocks;
        Ok(())
    }

    // ── allocation heap (GLULX_NOTES §11) ─────────────────────────────────────

    /// Allocate `size` bytes in the heap, returning the block address or 0 on
    /// failure. The first allocation activates the heap at the current memsize.
    fn heap_malloc(&mut self, size: u32) -> u32 {
        if size == 0 {
            return 0; // malloc requires a positive size
        }
        let heap_start = if self.heap_start == 0 { self.mem.mem_size() } else { self.heap_start };

        // Walk the free gaps from heap_start, reusing the first that fits.
        let mut cursor = heap_start;
        let mut addr = None;
        for &(a, sz) in &self.heap_blocks {
            if a - cursor >= size {
                addr = Some(cursor);
                break;
            }
            cursor = a + sz;
        }
        // No internal gap fit: append at the end of the last block (or
        // heap_start if empty), reusing committed tail space and growing only
        // if necessary.
        let addr = addr.unwrap_or(cursor);

        let top = addr as u64 + size as u64;
        if top > Self::MAX_MEMSIZE as u64 {
            return 0; // would exceed the heap ceiling
        }
        let top = top as u32;
        if top > self.mem.mem_size() {
            self.mem.set_raw_size(top);
        }
        // Insert keeping the block list sorted by address.
        let pos = self.heap_blocks.partition_point(|&(a, _)| a < addr);
        self.heap_blocks.insert(pos, (addr, size));
        self.heap_start = heap_start;
        addr
    }

    /// Free the extant block at `addr`; faults if it is not a current block.
    /// Freeing the last block deactivates the heap and shrinks memory back to
    /// the heap-start address.
    fn heap_free(&mut self, addr: u32) -> R<()> {
        let pos = self
            .heap_blocks
            .iter()
            .position(|&(a, _)| a == addr)
            .ok_or_else(|| format!("mfree of non-extant block @{addr:#010x}"))?;
        self.heap_blocks.remove(pos);
        if self.heap_blocks.is_empty() {
            self.mem.set_raw_size(self.heap_start);
            self.heap_start = 0;
        }
        Ok(())
    }

    // ── search opcodes (GLULX_NOTES §12) ──────────────────────────────────────

    const KEY_INDIRECT: u32 = 0x01;
    const ZERO_KEY_TERMINATES: u32 = 0x02;
    const RETURN_INDEX: u32 = 0x04;

    /// Build the search key as a byte vector: either read indirectly from the
    /// key address, or take the low `size` bytes of the immediate value.
    fn search_key(&self, key: u32, size: u32, options: u32) -> R<Vec<u8>> {
        if options & Self::KEY_INDIRECT != 0 {
            self.read_bytes(key, size)
        } else {
            match size {
                1 | 2 | 4 => Ok(key.to_be_bytes()[(4 - size as usize)..].to_vec()),
                _ => Err(format!("search: KeySize {size} must be 1, 2, or 4 for a direct key")),
            }
        }
    }

    /// Read `n` bytes from main memory into a vector (bounds-checked).
    fn read_bytes(&self, addr: u32, n: u32) -> R<Vec<u8>> {
        let mut v = Vec::with_capacity(n as usize);
        for i in 0..n {
            v.push(self.m8(addr + i)? as u8);
        }
        Ok(v)
    }

    fn op_linearsearch(&mut self) -> R<()> {
        let (l, s) = self.read_operands(7, 1)?;
        let (key, key_size, start, struct_size, num, key_off, opts) =
            (l[0], l[1], l[2], l[3], l[4], l[5], l[6]);
        let want = self.search_key(key, key_size, opts)?;
        let mut found: Option<(u32, u32)> = None; // (addr, index)
        let mut index = 0u32;
        loop {
            if num != 0xFFFF_FFFF && index >= num {
                break;
            }
            let saddr = start.wrapping_add(index.wrapping_mul(struct_size));
            let have = self.read_bytes(saddr + key_off, key_size)?;
            if have == want {
                found = Some((saddr, index));
                break;
            }
            if opts & Self::ZERO_KEY_TERMINATES != 0 && have.iter().all(|&b| b == 0) {
                break;
            }
            index += 1;
        }
        let r = Self::search_result(found, opts);
        self.store(s[0], r)
    }

    fn op_binarysearch(&mut self) -> R<()> {
        let (l, s) = self.read_operands(7, 1)?;
        let (key, key_size, start, struct_size, num, key_off, opts) =
            (l[0], l[1], l[2], l[3], l[4], l[5], l[6]);
        let want = self.search_key(key, key_size, opts)?;
        let mut found: Option<(u32, u32)> = None;
        let (mut lo, mut hi) = (0u32, num);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let saddr = start.wrapping_add(mid.wrapping_mul(struct_size));
            let have = self.read_bytes(saddr + key_off, key_size)?;
            match have.cmp(&want) {
                std::cmp::Ordering::Equal => {
                    found = Some((saddr, mid));
                    break;
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        let r = Self::search_result(found, opts);
        self.store(s[0], r)
    }

    fn op_linkedsearch(&mut self) -> R<()> {
        let (l, s) = self.read_operands(6, 1)?;
        let (key, key_size, start, key_off, next_off, opts) =
            (l[0], l[1], l[2], l[3], l[4], l[5]);
        let want = self.search_key(key, key_size, opts)?;
        let mut node = start;
        let mut result = 0u32;
        while node != 0 {
            let have = self.read_bytes(node + key_off, key_size)?;
            if have == want {
                result = node;
                break;
            }
            if opts & Self::ZERO_KEY_TERMINATES != 0 && have.iter().all(|&b| b == 0) {
                break;
            }
            node = self.m32(node + next_off)?;
        }
        self.store(s[0], result)
    }

    /// Map a found `(addr, index)` (or `None`) to the result value per the
    /// ReturnIndex option.
    fn search_result(found: Option<(u32, u32)>, options: u32) -> u32 {
        match (found, options & Self::RETURN_INDEX != 0) {
            (Some((_, idx)), true) => idx,
            (Some((addr, _)), false) => addr,
            (None, true) => 0xFFFF_FFFF,
            (None, false) => 0,
        }
    }

    // ── stack-manipulation opcodes ────────────────────────────────────────────

    fn op_stkpeek(&mut self) -> R<()> {
        let (l, s) = self.read_operands(1, 1)?;
        let i = l[0];
        if i >= self.value_count() {
            return Err(format!("stkpeek index {i} out of range"));
        }
        let off = self.sp - 4 * (i as usize + 1);
        let v = self.st_r32(off);
        self.store(s[0], v)
    }

    fn op_stkswap(&mut self) -> R<()> {
        if self.value_count() < 2 {
            return Err("stkswap underflow".to_string());
        }
        let (a, b) = (self.sp - 4, self.sp - 8);
        let (va, vb) = (self.st_r32(a), self.st_r32(b));
        self.st_w32(a, vb);
        self.st_w32(b, va);
        Ok(())
    }

    fn op_stkcopy(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let n = l[0] as usize;
        if l[0] > self.value_count() {
            return Err("stkcopy out of range".to_string());
        }
        let base = self.sp;
        for k in 0..n {
            let v = self.st_r32(base - 4 * n + 4 * k);
            self.push32(v)?;
        }
        Ok(())
    }

    fn op_stkroll(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let count = l[0];
        let places = l[1] as i32;
        if count > self.value_count() {
            return Err("stkroll out of range".to_string());
        }
        if count == 0 {
            return Ok(());
        }
        let count = count as usize;
        // Positive places rotate "up" (topmost moves deeper) → rotate_right on the
        // bottom→top ordering.
        let eff = (((places % count as i32) + count as i32) % count as i32) as usize;
        let base = self.sp - 4 * count;
        let mut vals: Vec<u32> = (0..count).map(|k| self.st_r32(base + 4 * k)).collect();
        vals.rotate_right(eff);
        for (k, v) in vals.iter().enumerate() {
            self.st_w32(base + 4 * k, *v);
        }
        Ok(())
    }

    // ── call variants ─────────────────────────────────────────────────────────

    fn op_call(&mut self) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (func, argc) = (l[0], l[1]);
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop32()?); // first arg is topmost
        }
        self.call_function(func, &args, s[0])
    }

    fn op_callf(&mut self, nargs: usize) -> R<()> {
        let (l, s) = self.read_operands(1 + nargs, 1)?;
        let args = l[1..].to_vec();
        self.call_function(l[0], &args, s[0])
    }

    fn op_tailcall(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let (func, argc) = (l[0], l[1]);
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop32()?);
        }
        // Destroy the current frame but keep the call stub beneath it, so the new
        // function returns to the current function's caller.
        self.sp = self.fp;
        self.build_frame_and_enter(func, &args)
    }

    // ── stream output (GLULX_NOTES §7) ────────────────────────────────────────

    /// Route stream output to the sink, honoring the current I/O system: only
    /// the Glk system (mode 2) prints; the null system (and the deferred filter
    /// system) discard.
    fn emit(&mut self, s: &str) {
        if self.iosys_mode == 2 {
            self.out.print(s);
        }
    }

    fn op_streamchar(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let s = ((l[0] & 0xFF) as u8 as char).to_string();
        self.emit(&s);
        Ok(())
    }

    fn op_streamunichar(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let s = char::from_u32(l[0]).unwrap_or('\u{FFFD}').to_string();
        self.emit(&s);
        Ok(())
    }

    fn op_streamnum(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let s = (l[0] as i32).to_string();
        self.emit(&s);
        Ok(())
    }

    fn op_streamstr(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        self.print_object(l[0], &[], 0)
    }

    /// Print a "typable object" at `addr`: a string (E0/E1/E2) is decoded and
    /// streamed; a function (C0/C1) is called with `args` and its output
    /// streamed in its place. `depth` bounds indirect/recursive references.
    fn print_object(&mut self, addr: u32, args: &[u32], depth: u32) -> R<()> {
        if depth > 256 {
            return Err(format!("string decode recursion too deep @{addr:#010x}"));
        }
        match self.m8(addr)? {
            0xE0 => {
                let s = self.read_cstring(addr + 1)?;
                self.emit(&s);
                Ok(())
            }
            0xE2 => {
                let s = self.read_ustring(addr + 4)?;
                self.emit(&s);
                Ok(())
            }
            0xE1 => self.decode_compressed(addr + 1, depth),
            0xC0 | 0xC1 => self.run_call_to_return(addr, args),
            other => Err(format!("bad string/function type {other:#x} @{addr:#010x}")),
        }
    }

    /// Decode a compressed (E1) bit stream beginning at `start`, walking the
    /// current string-decoding table. Bits are read low-bit-first (GLULX_NOTES
    /// §9). Never panics: bad addresses/node types fault.
    fn decode_compressed(&mut self, start: u32, depth: u32) -> R<()> {
        let table = self.cur_stringtbl;
        if table == 0 {
            return Err("compressed string (E1) with no string-decoding table set".to_string());
        }
        let root = self.m32(table + 8)?;
        let mut node = root;
        let mut addr = start;
        let mut bit = 0u32;
        loop {
            match self.m8(node)? {
                0x00 => {
                    // Branch: read one bit, go left (0) or right (1).
                    let byte = self.m8(addr)?;
                    let b = (byte >> bit) & 1;
                    bit += 1;
                    if bit == 8 {
                        bit = 0;
                        addr += 1;
                    }
                    node = if b == 0 { self.m32(node + 1)? } else { self.m32(node + 5)? };
                }
                0x01 => return Ok(()), // string terminator
                0x02 => {
                    let c = self.m8(node + 1)?;
                    self.emit_latin1(c);
                    node = root;
                }
                0x03 => {
                    let s = self.read_cstring(node + 1)?;
                    self.emit(&s);
                    node = root;
                }
                0x04 => {
                    let cp = self.m32(node + 1)?;
                    self.emit_uni(cp);
                    node = root;
                }
                0x05 => {
                    // C-style Unicode string: 32-bit chars until a zero word.
                    let s = self.read_ustring(node + 1)?;
                    self.emit(&s);
                    node = root;
                }
                0x08 => {
                    let a = self.m32(node + 1)?;
                    self.print_object(a, &[], depth + 1)?;
                    node = root;
                }
                0x09 => {
                    let a = self.m32(self.m32(node + 1)?)?;
                    self.print_object(a, &[], depth + 1)?;
                    node = root;
                }
                0x0A => {
                    let a = self.m32(node + 1)?;
                    let args = self.read_node_args(node + 5)?;
                    self.print_object(a, &args, depth + 1)?;
                    node = root;
                }
                0x0B => {
                    let a = self.m32(self.m32(node + 1)?)?;
                    let args = self.read_node_args(node + 5)?;
                    self.print_object(a, &args, depth + 1)?;
                    node = root;
                }
                other => return Err(format!("bad string node type {other:#x} @{node:#010x}")),
            }
        }
    }

    /// Read an argument list for a 0x0A/0x0B node: a 32-bit count then that many
    /// 32-bit arguments.
    fn read_node_args(&self, at: u32) -> R<Vec<u32>> {
        let argc = self.m32(at)?;
        let mut args = Vec::with_capacity(argc as usize);
        for i in 0..argc {
            args.push(self.m32(at + 4 + 4 * i)?);
        }
        Ok(args)
    }

    /// Call the function at `func` with `args`, running the VM run-loop until
    /// that frame returns, then resume the caller. Used for string-embedded
    /// function nodes (GLULX_NOTES §9); the return value is discarded.
    fn run_call_to_return(&mut self, func: u32, args: &[u32]) -> R<()> {
        let resume_fp = self.fp;
        self.call_function(func, args, Dest::Discard)?;
        // call_function installed a deeper callee frame; run until it returns.
        while self.fp != resume_fp {
            if self.halted {
                return Err("function called within a string halted the machine".to_string());
            }
            self.step_once()?;
        }
        Ok(())
    }

    fn emit_latin1(&mut self, v: u32) {
        let s = ((v & 0xFF) as u8 as char).to_string();
        self.emit(&s);
    }
    fn emit_uni(&mut self, v: u32) {
        let s = char::from_u32(v).unwrap_or('\u{FFFD}').to_string();
        self.emit(&s);
    }

    /// Read a zero-terminated Latin-1 string from main memory.
    fn read_cstring(&self, mut addr: u32) -> R<String> {
        let mut s = String::new();
        loop {
            let b = self.m8(addr)?;
            if b == 0 {
                break;
            }
            s.push(b as u8 as char);
            addr += 1;
        }
        Ok(s)
    }

    /// Read a zero-terminated array of big-endian 32-bit Unicode code points.
    fn read_ustring(&self, mut addr: u32) -> R<String> {
        let mut s = String::new();
        loop {
            let cp = self.m32(addr)?;
            if cp == 0 {
                break;
            }
            s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
            addr += 4;
        }
        Ok(s)
    }

    // ── minimal @glk dispatch (GLULX_NOTES §8) ────────────────────────────────

    fn op_glk(&mut self) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (selector, argc) = (l[0], l[1]);
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop32()?); // first @glk arg is topmost
        }
        let result = self.glk_dispatch(selector, &args)?;
        self.store(s[0], result)
    }

    /// Handle the put-char/buffer/string family; pop-and-ignore anything else and
    /// return 0. Glk output prints regardless of the VM's I/O system.
    fn glk_dispatch(&mut self, selector: u32, args: &[u32]) -> R<u32> {
        let a = |i: usize| args.get(i).copied().unwrap_or(0);
        match selector {
            0x0080 => self.put_latin1(a(0)), // glk_put_char(ch)
            0x0081 => self.put_latin1(a(1)), // glk_put_char_stream(str, ch)
            0x0082 => {
                // glk_put_string(addr)
                let s = self.read_cstring(a(0))?;
                self.out.print(&s);
            }
            0x0084 => {
                // glk_put_buffer(addr, len)
                let (addr, len) = (a(0), a(1));
                let mut s = String::new();
                for i in 0..len {
                    s.push(self.m8(addr + i)? as u8 as char);
                }
                self.out.print(&s);
            }
            0x0128 => self.put_uni(a(0)), // glk_put_char_uni(ch)
            0x0129 => {
                // glk_put_string_uni(addr)
                let s = self.read_ustring(a(0))?;
                self.out.print(&s);
            }
            0x012A => {
                // glk_put_buffer_uni(addr, len)
                let (addr, len) = (a(0), a(1));
                let mut s = String::new();
                for i in 0..len {
                    let cp = self.m32(addr + i * 4)?;
                    s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                self.out.print(&s);
            }
            other => self
                .diagnostics
                .push(format!("unhandled @glk selector {other:#06x} (returning 0)")),
        }
        Ok(0)
    }

    fn put_latin1(&mut self, v: u32) {
        let s = ((v & 0xFF) as u8 as char).to_string();
        self.out.print(&s);
    }
    fn put_uni(&mut self, v: u32) {
        let s = char::from_u32(v).unwrap_or('\u{FFFD}').to_string();
        self.out.print(&s);
    }

    // ── the run loop ──────────────────────────────────────────────────────────

    /// Execute one instruction. Returns [`StepResult::Quit`] on `quit`, an outer
    /// return, or any fault (which is recorded in `diagnostics`); otherwise
    /// [`StepResult::Continue`]. Never panics.
    pub fn step(&mut self) -> StepResult {
        if self.halted {
            return StepResult::Quit;
        }
        match self.step_once() {
            Ok(()) if self.halted => StepResult::Quit,
            Ok(()) => StepResult::Continue,
            Err(msg) => {
                self.diagnostics.push(msg);
                self.halted = true;
                StepResult::Quit
            }
        }
    }

    /// Run until the machine quits.
    pub fn run(&mut self) {
        while self.step() == StepResult::Continue {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm;
    use crate::io::BufferOutput;

    fn machine(built: asm::Built) -> Machine {
        let mem = Memory::new(built.image).expect("valid image");
        Machine::with_output(mem, Box::new(BufferOutput::new()))
    }

    /// A machine whose start function has `locals` and whose body is `body`,
    /// with `pc` positioned at the first body byte. RAMSTART is 0x100 (tiny
    /// code), so tests can hardcode RAM addresses.
    fn machine_with_body(locals: &[(u8, u8)], body: Vec<u8>) -> Machine {
        let start = asm::func(0xC1, locals, &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let m = machine(built);
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");
        m
    }

    #[test]
    fn decode_opcode_1_2_4_byte_forms() {
        let mut m = machine_with_body(&[], asm::opcode_bytes(0x10));
        assert_eq!(m.decode_opcode().unwrap(), 0x10);
        let mut m = machine_with_body(&[], asm::opcode_bytes(0x130));
        assert_eq!(m.decode_opcode().unwrap(), 0x130);
        let mut m = machine_with_body(&[], asm::opcode_bytes(0x0100_0000));
        assert_eq!(m.decode_opcode().unwrap(), 0x0100_0000);
    }

    fn one_load(locals: &[(u8, u8)], op: asm::Op, setup: impl FnOnce(&mut Machine)) -> u32 {
        let mut m = machine_with_body(locals, asm::operands(&[op]));
        setup(&mut m);
        let (loads, _) = m.read_operands(1, 0).unwrap();
        loads[0]
    }

    #[test]
    fn load_mode_constants_sign_extend() {
        assert_eq!(one_load(&[], asm::Op::Zero, |_| {}), 0);
        assert_eq!(one_load(&[], asm::Op::C8(-5), |_| {}), (-5i32) as u32);
        assert_eq!(one_load(&[], asm::Op::C16(-1000), |_| {}), (-1000i32) as u32);
        assert_eq!(one_load(&[], asm::Op::C32(0xDEAD_BEEF), |_| {}), 0xDEAD_BEEF);
    }

    #[test]
    fn load_mode_contents_of_address() {
        // Address 0 holds the "Glul" magic.
        assert_eq!(one_load(&[], asm::Op::Mem8(0), |_| {}), 0x476C_756C);
        // 2-/4-byte address forms read a value we plant in RAM at 0x100.
        let plant = |m: &mut Machine| m.mem.write32(0x100, 0x0102_0304).unwrap();
        assert_eq!(one_load(&[], asm::Op::Mem16(0x0100), plant), 0x0102_0304);
        assert_eq!(one_load(&[], asm::Op::Mem32(0x0100), plant), 0x0102_0304);
    }

    #[test]
    fn load_mode_ram_relative() {
        let plant = |m: &mut Machine| m.mem.write32(0x100, 0x1111_2222).unwrap();
        assert_eq!(one_load(&[], asm::Op::Ram8(0), plant), 0x1111_2222);
        assert_eq!(one_load(&[], asm::Op::Ram16(0), plant), 0x1111_2222);
        assert_eq!(one_load(&[], asm::Op::Ram32(0), plant), 0x1111_2222);
    }

    #[test]
    fn load_mode_local_each_offset_size() {
        let set = |m: &mut Machine| m.local_store(4, 0x55AA).unwrap();
        assert_eq!(one_load(&[(4, 4)], asm::Op::Local8(4), set), 0x55AA);
        assert_eq!(one_load(&[(4, 4)], asm::Op::Local16(4), set), 0x55AA);
        assert_eq!(one_load(&[(4, 4)], asm::Op::Local32(4), set), 0x55AA);
    }

    #[test]
    fn load_mode_stack_pops() {
        let v = one_load(&[], asm::Op::Stack, |m| m.push32(0xFEED_FACE).unwrap());
        assert_eq!(v, 0xFEED_FACE);
    }

    #[test]
    fn store_mode_discard_and_push() {
        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Zero]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Discard);
        m.store(dests[0], 0x1234).unwrap(); // no-op

        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Stack]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Push);
        m.store(dests[0], 0x9).unwrap();
        assert_eq!(m.pop32().unwrap(), 0x9);
    }

    #[test]
    fn store_mode_memory_and_local() {
        // Memory (2-byte addr form) → RAM at 0x100.
        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Mem16(0x0100)]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Mem(0x100));
        m.store(dests[0], 0xABCD_1234).unwrap();
        assert_eq!(m.mem.read32(0x100), Some(0xABCD_1234));

        // RAM-relative store maps to the same address.
        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Ram8(0)]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Mem(0x100));

        // Local store.
        let mut m = machine_with_body(&[(4, 2)], asm::operands(&[asm::Op::Local8(4)]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Local(4));
        m.store(dests[0], 0x7777).unwrap();
        assert_eq!(m.local_load(4).unwrap(), 0x7777);
    }

    // ── Task 4: arithmetic / bitwise / branch ─────────────────────────────────

    /// Execute one binary-op instruction storing to RAM 0x100; return the result.
    fn arith2(op: u32, a: asm::Op, b: asm::Op) -> u32 {
        let body = asm::ins(op, &[a, b, asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        m.mem.read32(0x100).unwrap()
    }
    fn arith1(op: u32, a: asm::Op) -> u32 {
        let body = asm::ins(op, &[a, asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        m.mem.read32(0x100).unwrap()
    }

    #[test]
    fn arithmetic_core() {
        use asm::Op::{C16, C32, C8};
        assert_eq!(arith2(0x10, C8(5), C8(7)), 12); // add
        assert_eq!(arith2(0x11, C8(5), C8(7)), (-2i32) as u32); // sub
        assert_eq!(arith2(0x12, C16(1000), C16(1000)), 1_000_000); // mul
        assert_eq!(arith2(0x12, C32(0x10000), C32(0x10000)), 0); // mul truncates
        assert_eq!(arith2(0x13, C8(-7), C8(2)), (-3i32) as u32); // div trunc toward 0
        assert_eq!(arith2(0x14, C8(-7), C8(2)), (-1i32) as u32); // mod sign of dividend
        assert_eq!(arith1(0x15, C8(5)), (-5i32) as u32); // neg
        assert_eq!(arith1(0x15, C8(-128)), 128); // neg of -128
    }

    #[test]
    fn div_mod_by_zero_faults() {
        let body = asm::ins(0x13, &[asm::Op::C8(5), asm::Op::C8(0), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        assert!(m.step_once().is_err());
        let body = asm::ins(0x14, &[asm::Op::C8(5), asm::Op::C8(0), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        assert!(m.step_once().is_err());
    }

    #[test]
    fn bitwise_and_shifts() {
        use asm::Op::{C32, C8};
        assert_eq!(arith2(0x18, C32(0xF0F0), C32(0xFF00)), 0xF000); // bitand
        assert_eq!(arith2(0x19, C32(0xF0F0), C32(0x0F0F)), 0xFFFF); // bitor
        assert_eq!(arith2(0x1A, C32(0xFF00), C32(0x0FF0)), 0xF0F0); // bitxor
        assert_eq!(arith1(0x1B, C32(0x0000_FFFF)), 0xFFFF_0000); // bitnot
        assert_eq!(arith2(0x1C, C8(1), C8(4)), 0x10); // shiftl
        assert_eq!(arith2(0x1C, C8(1), C8(32)), 0); // shiftl >= 32 → 0
        assert_eq!(arith2(0x1E, C32(0x8000_0000), C8(4)), 0x0800_0000); // ushiftr
        assert_eq!(arith2(0x1E, C32(0x8000_0000), C8(40)), 0); // ushiftr >= 32 → 0
        assert_eq!(arith2(0x1D, C32(0x8000_0000), C8(4)), 0xF800_0000); // sshiftr arith
        assert_eq!(arith2(0x1D, C32(0x8000_0000), C8(40)), 0xFFFF_FFFF); // sshiftr >= 32 → sign
    }

    #[test]
    fn jump_offset_math() {
        // jump +5: [0x20, modes(0x01), data(0x05)] = 3 bytes → pc = pc0+3+5-2.
        let body = asm::ins(0x20, &[asm::Op::C8(5)]);
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        m.step_once().unwrap();
        assert_eq!(m.pc, pc0 + 6);
    }

    #[test]
    fn conditional_branch_taken_and_not() {
        // jz value=0 → taken (jumps forward).
        let body = asm::ins(0x22, &[asm::Op::C8(0), asm::Op::C8(20)]);
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        let instr_len = body_len(0x22, 2);
        m.step_once().unwrap();
        assert_eq!(m.pc, pc0 + instr_len + 20 - 2);

        // jz value=1 → not taken (falls through past the instruction).
        let body = asm::ins(0x22, &[asm::Op::C8(1), asm::Op::C8(20)]);
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        m.step_once().unwrap();
        assert_eq!(m.pc, pc0 + body_len(0x22, 2));
    }

    #[test]
    fn signed_vs_unsigned_compares() {
        // jlt(-1, 1) signed → taken; jltu(0xFFFFFFFF, 1) unsigned → not taken.
        let taken = |op: u32, a: asm::Op, b: asm::Op| -> bool {
            let body = asm::ins(op, &[a, b, asm::Op::C8(40)]);
            let blen = body.len() as u32;
            let mut m = machine_with_body(&[], body);
            let pc0 = m.pc;
            m.step_once().unwrap();
            // Not taken → pc falls through to pc0+blen; taken → pc differs.
            m.pc != pc0 + blen
        };
        assert!(taken(0x26, asm::Op::C8(-1), asm::Op::C8(1))); // jlt signed
        assert!(!taken(0x2A, asm::Op::C32(0xFFFF_FFFF), asm::Op::C8(1))); // jltu unsigned
        assert!(taken(0x2B, asm::Op::C32(0xFFFF_FFFF), asm::Op::C8(1))); // jgeu unsigned
    }

    #[test]
    fn branch_offset_0_and_1_return_convention() {
        // Inside a callee, `jump 0` returns 0 and `jump 1` returns 1 to the
        // caller's destination (here, the value stack).
        for (offset, expect) in [(0u32, 0u32), (1, 1)] {
            let start = asm::func(0xC1, &[], &[]);
            let off_op = if offset == 0 { asm::Op::Zero } else { asm::Op::C8(1) };
            let callee = asm::func(0xC1, &[], &asm::ins(0x20, &[off_op]));
            let built = asm::assemble(&[start, callee], 0, 0x100);
            let callee_addr = built.addrs[1];
            let mut m = machine(built);
            m.call_function(callee_addr, &[], Dest::Push).unwrap();
            m.step_once().unwrap(); // executes the jump → return
            assert_eq!(m.pop32().unwrap(), expect);
        }
    }

    /// Byte length of an instruction: 1-byte opcode (< 0x80) + ceil(n/2) mode
    /// bytes + n single-byte (C8) operands.
    fn body_len(_op: u32, n: u32) -> u32 {
        1 + n.div_ceil(2) + n
    }

    #[test]
    fn illegal_operand_mode_faults() {
        // Mode 0x4 is unused/illegal for load.
        let mut m = machine_with_body(&[], vec![0x04]);
        assert!(m.read_operands(1, 0).is_err());
        // Mode 0x1 (constant) is illegal as a store target.
        let mut m = machine_with_body(&[], vec![0x01, 0x00]);
        assert!(m.read_operands(0, 1).is_err());
    }

    // ── Task 5: stack ops, copy, memsize, call variants ───────────────────────

    #[test]
    fn copy_and_sign_extend() {
        // copy: full 32-bit through RAM.
        let body = asm::ins(0x40, &[asm::Op::C32(0xDEAD_BEEF), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF);

        // copys: write only 2 bytes to memory (no sign extension).
        let body = asm::ins(0x41, &[asm::Op::C32(0x1234_5678), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x100, 0xFFFF_FFFF).unwrap();
        m.step_once().unwrap();
        // Big-endian: the 2-byte write at 0x100 lands in the high-order bytes
        // (0x5678), leaving the low half (0x102..0x104) unchanged (FFFF).
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x5678_FFFF);

        // copyb: write only 1 byte (the top byte at 0x100).
        let body = asm::ins(0x42, &[asm::Op::C32(0x1234_5678), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x100, 0).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x7800_0000);

        // sexs / sexb sign-extend.
        let sx = |op: u32, v: u32| {
            let body = asm::ins(op, &[asm::Op::C32(v), asm::Op::Mem16(0x0100)]);
            let mut m = machine_with_body(&[], body);
            m.step_once().unwrap();
            m.mem.read32(0x100).unwrap()
        };
        assert_eq!(sx(0x44, 0x0000_8000), 0xFFFF_8000); // sexs negative
        assert_eq!(sx(0x44, 0x0000_7FFF), 0x0000_7FFF); // sexs positive
        assert_eq!(sx(0x45, 0x0000_0080), 0xFFFF_FF80); // sexb negative
        assert_eq!(sx(0x45, 0x0000_007F), 0x0000_007F); // sexb positive
    }

    #[test]
    fn stkcount_peek_swap() {
        // stkcount → RAM.
        let body = asm::ins(0x50, &[asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.push32(1).unwrap();
        m.push32(2).unwrap();
        m.push32(3).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 3);

        // stkpeek index 1 (does not pop).
        let body = asm::ins(0x51, &[asm::Op::C8(1), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.push32(0xAA).unwrap();
        m.push32(0xBB).unwrap();
        m.push32(0xCC).unwrap(); // top
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xBB);
        assert_eq!(m.value_count(), 3);

        // stkswap.
        let body = asm::ins(0x52, &[]);
        let mut m = machine_with_body(&[], body);
        m.push32(1).unwrap();
        m.push32(2).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.pop32().unwrap(), 1);
        assert_eq!(m.pop32().unwrap(), 2);
    }

    #[test]
    fn stkcopy_duplicates_top_n_in_order() {
        let body = asm::ins(0x54, &[asm::Op::C8(2)]);
        let mut m = machine_with_body(&[], body);
        m.push32(10).unwrap();
        m.push32(20).unwrap();
        m.push32(30).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.value_count(), 5);
        // ...10,20,30,20,30 (top).
        for expect in [30, 20, 30, 20, 10] {
            assert_eq!(m.pop32().unwrap(), expect);
        }
    }

    #[test]
    fn stkroll_rotates_per_spec_example() {
        // Spec example: 8 7 6 5 4 3 2 1 0<top>; stkroll 5 1 → 8 7 6 5 0 4 3 2 1<top>.
        let body = asm::ins(0x53, &[asm::Op::C8(5), asm::Op::C8(1)]);
        let mut m = machine_with_body(&[], body);
        for v in [8u32, 7, 6, 5, 4, 3, 2, 1, 0] {
            m.push32(v).unwrap();
        }
        m.step_once().unwrap();
        for expect in [1, 2, 3, 4, 0, 5, 6, 7, 8] {
            assert_eq!(m.pop32().unwrap(), expect);
        }
    }

    #[test]
    fn get_and_set_memsize() {
        let body = asm::ins(0x102, &[asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        let size = m.mem.mem_size();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), size);

        // setmemsize success → result 0, size grows.
        let body = asm::ins(0x103, &[asm::Op::C16(0x300), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0); // success
        assert_eq!(m.mem.mem_size(), 0x300);

        // setmemsize failure (unaligned) → result 1, size unchanged.
        let body = asm::ins(0x103, &[asm::Op::C16(0x250), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        let before = m.mem.mem_size();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 1); // failure
        assert_eq!(m.mem.mem_size(), before);
    }

    #[test]
    fn call_takes_args_from_stack_and_returns() {
        // callee (index 0, addr 0x24) returns the constant 0x123.
        let callee = asm::func(0xC1, &[], &asm::ins(0x31, &[asm::Op::C16(0x123)]));
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x30, &[asm::Op::C32(0x24), asm::Op::C8(0), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[callee, start], 1, 0x100);
        assert_eq!(built.addrs[0], 0x24);
        let mut m = machine(built);
        m.step_once().unwrap(); // call
        m.step_once().unwrap(); // return
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x123);
    }

    #[test]
    fn callfi_passes_one_arg() {
        // callee returns its single local (the argument).
        let callee = asm::func(0xC1, &[(4, 1)], &asm::ins(0x31, &[asm::Op::Local8(0)]));
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x161, &[asm::Op::C32(0x24), asm::Op::C16(0x99), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[callee, start], 1, 0x100);
        let mut m = machine(built);
        m.step_once().unwrap(); // callfi
        m.step_once().unwrap(); // return
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x99);
    }

    #[test]
    fn callfii_sums_two_args() {
        // callee: add local0 + local1 → stack, then return it.
        let mut body = asm::ins(0x10, &[asm::Op::Local8(0), asm::Op::Local8(4), asm::Op::Stack]);
        body.extend(asm::ins(0x31, &[asm::Op::Stack]));
        let callee = asm::func(0xC1, &[(4, 2)], &body);
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(
                0x162,
                &[asm::Op::C32(0x24), asm::Op::C8(10), asm::Op::C8(20), asm::Op::Mem16(0x0100)],
            ),
        );
        let built = asm::assemble(&[callee, start], 1, 0x100);
        let mut m = machine(built);
        m.step_once().unwrap(); // callfii
        m.step_once().unwrap(); // add
        m.step_once().unwrap(); // return
        assert_eq!(m.mem.read32(0x100).unwrap(), 30);
    }

    #[test]
    fn tailcall_reuses_caller_stub() {
        // f2 (addr 0x24) returns 0x77. f1 tailcalls f2. start calls f1 storing to
        // RAM 0x100. Because tailcall reuses f1's stub, 0x77 lands at 0x100.
        let f2 = asm::func(0xC1, &[], &asm::ins(0x31, &[asm::Op::C16(0x77)]));
        let f1_addr = 0x24 + f2.len() as u32;
        let f1 = asm::func(0xC1, &[], &asm::ins(0x34, &[asm::Op::C32(0x24), asm::Op::C8(0)]));
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x30, &[asm::Op::C32(f1_addr), asm::Op::C8(0), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[f2, f1, start], 2, 0x100);
        assert_eq!(built.addrs[1], f1_addr);
        let mut m = machine(built);
        m.step_once().unwrap(); // start: call f1
        m.step_once().unwrap(); // f1: tailcall f2
        m.step_once().unwrap(); // f2: return 0x77
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x77);
        assert!(!m.halted); // returned into start, not halted
    }

    // ── Task 6: output, iosys, @glk, run loop ─────────────────────────────────

    fn out_str(m: &Machine) -> String {
        m.out.as_any().downcast_ref::<BufferOutput>().unwrap().buf.clone()
    }

    /// Build + run a start function with body `body`; return its output.
    fn run_program(body: Vec<u8>) -> Machine {
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        m.run();
        m
    }

    #[test]
    fn streamnum_and_streamchar_under_glk_iosys() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]); // setiosys glk
        body.extend(asm::ins(0x71, &[asm::Op::C8(42)])); // streamnum 42
        body.extend(asm::ins(0x71, &[asm::Op::C8(-7)])); // streamnum -7
        body.extend(asm::ins(0x70, &[asm::Op::C8(65)])); // streamchar 'A'
        body.extend(asm::ins(0x120, &[])); // quit
        let m = run_program(body);
        assert_eq!(out_str(&m), "42-7A");
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn null_iosys_discards_output() {
        // Default iosys is null (0); streamchar produces nothing.
        let mut body = asm::ins(0x70, &[asm::Op::C8(88)]); // streamchar 'X'
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(out_str(&m), "");
    }

    #[test]
    fn glk_put_char_and_put_buffer() {
        // glk_put_char('B').
        let mut body = asm::ins(0x40, &[asm::Op::C8(66), asm::Op::Stack]); // push 'B'
        body.extend(asm::ins(0x130, &[asm::Op::C16(0x80), asm::Op::C8(1), asm::Op::Zero]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(out_str(&m), "B");

        // glk_put_buffer(addr=0x100, len=2) over "Hi" planted via copyb.
        let mut body = asm::ins(0x42, &[asm::Op::C8(b'H' as i8), asm::Op::Mem16(0x0100)]);
        body.extend(asm::ins(0x42, &[asm::Op::C8(b'i' as i8), asm::Op::Mem16(0x0101)]));
        body.extend(asm::ins(0x40, &[asm::Op::C8(2), asm::Op::Stack])); // push len (below)
        body.extend(asm::ins(0x40, &[asm::Op::C16(0x0100), asm::Op::Stack])); // push addr (top)
        body.extend(asm::ins(0x130, &[asm::Op::C16(0x84), asm::Op::C8(2), asm::Op::Zero]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(out_str(&m), "Hi");
    }

    #[test]
    fn streamstr_prints_e0_cstring() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0100)])); // streamstr @0x100
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        for (i, b) in [0xE0u8, b'H', b'i', b'!', 0].iter().enumerate() {
            m.mem.write8(0x100 + i as u32, *b as u32).unwrap();
        }
        m.run();
        assert_eq!(out_str(&m), "Hi!");
    }

    #[test]
    fn nop_and_quit() {
        let mut body = asm::ins(0x00, &[]); // nop
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        assert_eq!(m.step(), StepResult::Continue); // nop
        assert_eq!(m.step(), StepResult::Quit); // quit
        assert_eq!(m.step(), StepResult::Quit); // stays quit
    }

    #[test]
    fn unknown_opcode_records_diagnostic_and_quits() {
        let m = run_program(asm::ins(0x1FF, &[])); // 0x1FF unimplemented
        assert!(m.halted);
        assert!(!m.diagnostics.is_empty());
    }

    #[test]
    fn memory_fault_records_diagnostic_and_quits() {
        // Load contents of a wildly out-of-range address → fault, no panic.
        let m = run_program(asm::ins(0x40, &[asm::Op::Mem32(0xFFFF_FFF0), asm::Op::Zero]));
        assert!(m.halted);
        assert!(m.diagnostics.iter().any(|d| d.contains("memory fault")));
    }

    #[test]
    fn end_to_end_arith_call_output_branch_quit() {
        // double(arg) = arg + arg (function at 0x24).
        let mut dbody = asm::ins(0x10, &[asm::Op::Local8(0), asm::Op::Local8(0), asm::Op::Stack]);
        dbody.extend(asm::ins(0x31, &[asm::Op::Stack])); // return
        let double = asm::func(0xC1, &[(4, 1)], &dbody);

        // start: setiosys glk; push 3+4; call double; streamnum result; a taken
        // branch (jz 0, offset 1) returns from start (halts) before the poison.
        let mut sbody = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        sbody.extend(asm::ins(0x10, &[asm::Op::C8(3), asm::Op::C8(4), asm::Op::Stack])); // 7
        sbody.extend(asm::ins(0x30, &[asm::Op::C32(0x24), asm::Op::C8(1), asm::Op::Stack])); // 14
        sbody.extend(asm::ins(0x71, &[asm::Op::Stack])); // streamnum 14 → "14"
        sbody.extend(asm::ins(0x22, &[asm::Op::C8(0), asm::Op::C8(1)])); // jz 0,1 → return 1
        sbody.extend(asm::ins(0x70, &[asm::Op::C8(b'!' as i8)])); // poison, unreached
        let start = asm::func(0xC1, &[], &sbody);

        let built = asm::assemble(&[double, start], 1, 0x100);
        assert_eq!(built.addrs[0], 0x24);
        let mut m = machine(built);
        m.run();
        assert_eq!(out_str(&m), "14");
        assert!(m.halted);
    }

    #[test]
    fn start_frame_entered_at_fp_zero() {
        // A C1 start function with three 4-byte locals.
        let start = asm::func(0xC1, &[(4, 3)], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let m = machine(built);
        assert!(!m.halted);
        assert_eq!(m.fp, 0);
        // Locals are zero-initialized.
        assert_eq!(m.local_load(0).unwrap(), 0);
        assert_eq!(m.local_load(8).unwrap(), 0);
    }

    #[test]
    fn c1_call_copies_args_into_locals_extras_zeroed() {
        // start (C1, no locals) and a callee (C1, four 4-byte locals).
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC1, &[(4, 4)], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[11, 22, 33], Dest::Discard).unwrap();
        assert_eq!(m.local_load(0).unwrap(), 11);
        assert_eq!(m.local_load(4).unwrap(), 22);
        assert_eq!(m.local_load(8).unwrap(), 33);
        assert_eq!(m.local_load(12).unwrap(), 0); // extra local zeroed
    }

    #[test]
    fn c1_call_truncates_to_local_width() {
        let start = asm::func(0xC1, &[], &[]);
        // locals: one 1-byte, one 2-byte, one 4-byte.
        let callee = asm::func(0xC1, &[(1, 1), (2, 1), (4, 1)], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[0x1234_5678, 0xABCD, 0xDEAD_BEEF], Dest::Discard)
            .unwrap();
        assert_eq!(m.local_load(0).unwrap(), 0x78); // 1-byte truncation
        assert_eq!(m.local_load(2).unwrap(), 0xABCD); // 2-byte, aligned to even
        assert_eq!(m.local_load(4).unwrap(), 0xDEAD_BEEF); // 4-byte, aligned to 4
    }

    #[test]
    fn c0_call_pushes_args_and_count() {
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC0, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[100, 200, 300], Dest::Discard).unwrap();
        // Count on top, then first arg, then second, then third.
        assert_eq!(m.value_count(), 4);
        assert_eq!(m.pop32().unwrap(), 3); // count
        assert_eq!(m.pop32().unwrap(), 100); // first arg topmost
        assert_eq!(m.pop32().unwrap(), 200);
        assert_eq!(m.pop32().unwrap(), 300);
    }

    #[test]
    fn return_writes_value_to_local_dest() {
        let start = asm::func(0xC1, &[(4, 1)], &[]); // start has one local
        let callee = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        // Call storing the return value into start's local at offset 0.
        m.call_function(callee_addr, &[], Dest::Local(0)).unwrap();
        m.return_value(0x4242).unwrap();
        assert_eq!(m.fp, 0); // back in the start frame
        assert_eq!(m.local_load(0).unwrap(), 0x4242);
    }

    #[test]
    fn return_writes_value_to_stack_dest() {
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[], Dest::Push).unwrap();
        m.return_value(0x99).unwrap();
        assert_eq!(m.value_count(), 1);
        assert_eq!(m.pop32().unwrap(), 0x99);
    }

    #[test]
    fn return_writes_value_to_memory_dest() {
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let ramstart = {
            let mem = Memory::new(built.image.clone()).unwrap();
            mem.ramstart()
        };
        let mut m = machine(built);
        m.call_function(callee_addr, &[], Dest::Mem(ramstart)).unwrap();
        m.return_value(0xCAFE_F00D).unwrap();
        assert_eq!(m.mem.read32(ramstart), Some(0xCAFE_F00D));
    }

    #[test]
    fn returning_from_outermost_halts() {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        assert!(!m.halted);
        m.return_value(0).unwrap();
        assert!(m.halted);
    }

    #[test]
    fn nested_calls_restore_pc_and_fp() {
        let start = asm::func(0xC1, &[], &[]);
        let f1 = asm::func(0xC1, &[(4, 1)], &[]);
        let f2 = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, f1, f2], 0, 0x100);
        let (a1, a2) = (built.addrs[1], built.addrs[2]);
        let mut m = machine(built);
        let start_pc = m.pc;
        m.pc = 0xDEAD; // pretend the caller is mid-instruction
        m.call_function(a1, &[], Dest::Discard).unwrap();
        let f1_fp = m.fp;
        m.pc = 0xBEEF;
        m.call_function(a2, &[], Dest::Discard).unwrap();
        m.return_value(0).unwrap();
        assert_eq!(m.fp, f1_fp); // back to f1
        assert_eq!(m.pc, 0xBEEF); // f1's resume pc
        m.return_value(0).unwrap();
        assert_eq!(m.fp, 0); // back to start
        assert_eq!(m.pc, 0xDEAD);
        let _ = start_pc;
    }

    // ── Task 1 (2b): compressed-string decoding + full streamstr ──────────────

    /// Write consecutive bytes into memory starting at `addr` (RAM).
    fn poke(m: &mut Machine, addr: u32, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            m.mem.write8(addr + i as u32, b as u32).unwrap();
        }
    }

    /// Run a start-function body that has already had its RAM populated by
    /// `setup`; returns the machine. iosys is left to the body.
    fn run_with_ram(body: Vec<u8>, ram_bytes: u32, setup: impl FnOnce(&mut Machine)) -> Machine {
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, ram_bytes);
        let mut m = machine(built);
        setup(&mut m);
        m.run();
        m
    }

    /// A standard tiny decoding table at RAM base `t` that decodes the bit
    /// sequence 0,0,0,1,1 (packed byte 0x18) to "Hi": root branches
    /// left→inner / right→terminator; inner branches left→'H' / right→'i'.
    fn build_hi_table(m: &mut Machine, t: u32) {
        // Header: length, numnodes, root.
        let root = t + 12;
        let inner = root + 9;
        let h = inner + 9;
        let i = h + 2;
        let term = i + 2;
        let len = (term + 1) - t;
        poke(m, t, &len.to_be_bytes());
        poke(m, t + 4, &5u32.to_be_bytes());
        poke(m, t + 8, &root.to_be_bytes());
        // root: branch left=inner right=term
        poke(m, root, &[0x00]);
        poke(m, root + 1, &inner.to_be_bytes());
        poke(m, root + 5, &term.to_be_bytes());
        // inner: branch left='H' right='i'
        poke(m, inner, &[0x00]);
        poke(m, inner + 1, &h.to_be_bytes());
        poke(m, inner + 5, &i.to_be_bytes());
        poke(m, h, &[0x02, b'H']);
        poke(m, i, &[0x02, b'i']);
        poke(m, term, &[0x01]);
    }

    #[test]
    fn compressed_string_decodes_against_table() {
        // setstringtbl 0x100; streamstr 0x140; quit.
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]); // setiosys glk
        body.extend(asm::ins(0x141, &[asm::Op::C16(0x0100)])); // setstringtbl 0x100
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0140)])); // streamstr 0x140
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            build_hi_table(m, 0x100);
            poke(m, 0x140, &[0xE1, 0x18]); // compressed "Hi": bits 0,0,0,1,1
        });
        assert_eq!(out_str(&m), "Hi");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn getstringtbl_reports_current_table() {
        // setstringtbl 0x100; getstringtbl → RAM 0x160.
        let mut body = asm::ins(0x141, &[asm::Op::C16(0x0100)]);
        body.extend(asm::ins(0x140, &[asm::Op::Mem16(0x0160)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x160).unwrap(), 0x100);
    }

    #[test]
    fn compressed_string_indirect_node_prints_e0() {
        // root: left → 0x08 node (→ E0 "Ok"), right → terminator. bits 0,1 → 0x02.
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x141, &[asm::Op::C16(0x0100)]));
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0150)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            let t = 0x100u32;
            let root = t + 12;
            let indir = root + 9;
            let term = indir + 5;
            let estr = 0x170u32;
            let len = (term + 1) - t;
            poke(m, t, &len.to_be_bytes());
            poke(m, t + 4, &3u32.to_be_bytes());
            poke(m, t + 8, &root.to_be_bytes());
            poke(m, root, &[0x00]);
            poke(m, root + 1, &indir.to_be_bytes());
            poke(m, root + 5, &term.to_be_bytes());
            poke(m, indir, &[0x08]);
            poke(m, indir + 1, &estr.to_be_bytes());
            poke(m, term, &[0x01]);
            poke(m, estr, &[0xE0, b'O', b'k', 0]);
            poke(m, 0x150, &[0xE1, 0x02]); // bits 0,1
        });
        assert_eq!(out_str(&m), "Ok");
    }

    #[test]
    fn compressed_string_function_node_calls_function() {
        // A printer function (C1, no locals) that streams 'Z' and returns.
        let printer = asm::func(
            0xC1,
            &[],
            &{
                let mut b = asm::ins(0x70, &[asm::Op::C8(b'Z' as i8)]); // streamchar 'Z'
                b.extend(asm::ins(0x31, &[asm::Op::Zero])); // return 0
                b
            },
        );
        // start: setiosys glk; setstringtbl 0x100; streamstr 0x150; quit.
        let mut sbody = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        sbody.extend(asm::ins(0x141, &[asm::Op::C16(0x0100)]));
        sbody.extend(asm::ins(0x72, &[asm::Op::C16(0x0150)]));
        sbody.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &sbody);
        let built = asm::assemble(&[printer, start], 1, 0x200);
        let printer_addr = built.addrs[0];
        let mut m = machine(built);
        // root: left → 0x08 node (→ printer fn), right → terminator. bits 0,1.
        let t = 0x100u32;
        let root = t + 12;
        let fnode = root + 9;
        let term = fnode + 5;
        let len = (term + 1) - t;
        poke(&mut m, t, &len.to_be_bytes());
        poke(&mut m, t + 4, &3u32.to_be_bytes());
        poke(&mut m, t + 8, &root.to_be_bytes());
        poke(&mut m, root, &[0x00]);
        poke(&mut m, root + 1, &fnode.to_be_bytes());
        poke(&mut m, root + 5, &term.to_be_bytes());
        poke(&mut m, fnode, &[0x08]);
        poke(&mut m, fnode + 1, &printer_addr.to_be_bytes());
        poke(&mut m, term, &[0x01]);
        poke(&mut m, 0x150, &[0xE1, 0x02]); // bits 0,1 → call printer, then terminate
        m.run();
        assert_eq!(out_str(&m), "Z");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn streamstr_e2_unicode_still_works() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            // E2 + 3 pad bytes + 'A'(0x41) + 'λ'(0x3BB) + terminator word.
            poke(m, 0x100, &[0xE2, 0, 0, 0]);
            poke(m, 0x104, &0x41u32.to_be_bytes());
            poke(m, 0x108, &0x3BBu32.to_be_bytes());
            poke(m, 0x10C, &0u32.to_be_bytes());
        });
        assert_eq!(out_str(&m), "Aλ");
    }

    #[test]
    fn compressed_string_with_no_table_faults() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x100, &[0xE1, 0x00]));
        assert!(m.halted);
        assert!(m.diagnostics.iter().any(|d| d.contains("decoding table")));
    }

    // ── Task 2 (2b): memory-array opcodes + mzero/mcopy ───────────────────────

    #[test]
    fn aload_astore_32bit_roundtrip_signed_index() {
        use asm::Op::{C16, C32, C8, Mem16};
        // astore base=0x140 index=2 → 0x148; aload reads it back to 0x100.
        let mut body = asm::ins(0x4C, &[C16(0x0140), C8(2), C32(0xDEAD_BEEF)]);
        body.extend(asm::ins(0x48, &[C16(0x0140), C8(2), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x148).unwrap(), 0xDEAD_BEEF);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF);

        // Negative index: astore base=0x148 index=-1 → 0x144.
        let mut body = asm::ins(0x4C, &[C16(0x0148), C8(-1), C32(0x1234_5678)]);
        body.extend(asm::ins(0x48, &[C16(0x0148), C8(-1), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x144).unwrap(), 0x1234_5678);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x1234_5678);
    }

    #[test]
    fn aloads_astores_16bit_and_aloadb_astoreb_8bit() {
        use asm::Op::{C16, C8, Mem16};
        // 16-bit: astores base=0x140 index=3 → 0x146; loads expand WITHOUT sign.
        let mut body = asm::ins(0x4D, &[C16(0x0140), C8(3), C16(-2)]); // 0xFFFE truncated
        body.extend(asm::ins(0x49, &[C16(0x0140), C8(3), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read16(0x146).unwrap(), 0xFFFE);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_FFFE); // zero-extended

        // 8-bit at a negative index.
        let mut body = asm::ins(0x4E, &[C16(0x0150), C8(-1), C16(0xAB)]); // → 0x14F
        body.extend(asm::ins(0x4A, &[C16(0x0150), C8(-1), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read8(0x14F).unwrap(), 0xAB);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_00AB);
    }

    #[test]
    fn aloadbit_astorebit_signed_bit_index() {
        use asm::Op::{C16, C8, Mem16};
        // Set bit 0 of 0x142, bit 7 of 0x142, bit 0 of 0x143 (index 8), bit 7 of
        // 0x141 (index -1) — mirrors the spec's astorebit examples.
        let sets: &[(u16, i8)] = &[(0x142, 0), (0x142, 7), (0x142, 8), (0x142, -1)];
        let mut body = Vec::new();
        for &(base, idx) in sets {
            body.extend(asm::ins(0x4F, &[C16(base as i16), C8(idx), C8(1)]));
        }
        // Read them back: bit 7 of 0x141 should be 1; bit 0 of 0x142 should be 1.
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(0), Mem16(0x0100)]));
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(-1), Mem16(0x0104)])); // bit7 of 0x141
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(8), Mem16(0x0108)])); // bit0 of 0x143
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(1), Mem16(0x010C)])); // clear bit → 0
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read8(0x142).unwrap(), 0b1000_0001); // bits 0 and 7 set
        assert_eq!(m.mem.read8(0x143).unwrap(), 0b0000_0001); // bit 0 set
        assert_eq!(m.mem.read8(0x141).unwrap(), 0b1000_0000); // bit 7 set
        assert_eq!(m.mem.read32(0x100).unwrap(), 1);
        assert_eq!(m.mem.read32(0x104).unwrap(), 1);
        assert_eq!(m.mem.read32(0x108).unwrap(), 1);
        assert_eq!(m.mem.read32(0x10C).unwrap(), 0);
    }

    #[test]
    fn aload_out_of_range_faults() {
        use asm::Op::{C32, Zero};
        // base near the top of memory + a big index → out of range.
        let m = run_program(asm::ins(0x48, &[C32(0xFFFF_FFF0), Zero, Zero]));
        assert!(m.halted);
        assert!(m.diagnostics.iter().any(|d| d.contains("memory fault")));
    }

    #[test]
    fn mzero_clears_a_span() {
        use asm::Op::{C16, C32, C8};
        // Plant a value, then mzero 4 bytes over it.
        let mut body = asm::ins(0x4C, &[C16(0x0140), C8(0), C32(0xFFFF_FFFF)]);
        body.extend(asm::ins(0x170, &[asm::Op::C8(4), C16(0x0140)])); // mzero count=4 addr=0x140
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x140).unwrap(), 0);
    }

    #[test]
    fn mcopy_handles_forward_and_backward_overlap() {
        use asm::Op::{C16, C8};
        // Source bytes 1,2,3,4 at 0x140. mcopy 4 from 0x140 to 0x142 (to > from →
        // backward copy) must yield 1,2,1,2,3,4 across 0x140..0x146.
        let plant = |body: &mut Vec<u8>| {
            for (i, v) in [1u8, 2, 3, 4].iter().enumerate() {
                body.extend(asm::ins(0x4E, &[C16(0x0140), C8(i as i8), C8(*v as i8)]));
            }
        };
        let mut body = Vec::new();
        plant(&mut body);
        body.extend(asm::ins(0x171, &[C8(4), C16(0x0140), C16(0x0142)])); // count,from,to
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(
            [
                m.mem.read8(0x140).unwrap(),
                m.mem.read8(0x141).unwrap(),
                m.mem.read8(0x142).unwrap(),
                m.mem.read8(0x143).unwrap(),
                m.mem.read8(0x144).unwrap(),
                m.mem.read8(0x145).unwrap(),
            ],
            [1, 2, 1, 2, 3, 4]
        );

        // to < from → forward copy. 1,2,3,4 at 0x142; copy to 0x140 → 1,2,3,4.
        let mut body = Vec::new();
        for (i, v) in [1u8, 2, 3, 4].iter().enumerate() {
            body.extend(asm::ins(0x4E, &[C16(0x0142), C8(i as i8), C8(*v as i8)]));
        }
        body.extend(asm::ins(0x171, &[C8(4), C16(0x0142), C16(0x0140)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(
            [
                m.mem.read8(0x140).unwrap(),
                m.mem.read8(0x141).unwrap(),
                m.mem.read8(0x142).unwrap(),
                m.mem.read8(0x143).unwrap(),
            ],
            [1, 2, 3, 4]
        );
    }

    // ── Task 3 (2b): malloc / mfree heap ──────────────────────────────────────

    #[test]
    fn malloc_returns_distinct_in_range_blocks() {
        use asm::Op::{C8, Mem16};
        let mut body = asm::ins(0x178, &[C8(16), Mem16(0x0100)]); // malloc 16 → 0x100
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0104)])); // malloc 16 → 0x104
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        let a = m.mem.read32(0x100).unwrap();
        let b = m.mem.read32(0x104).unwrap();
        let floor = m.mem.endmem();
        assert!(a >= floor, "block a {a:#x} below endmem {floor:#x}");
        assert_ne!(a, b);
        assert_eq!(b, a + 16, "blocks must be adjacent and non-overlapping");
        assert!(a + 16 <= m.mem.mem_size() && b + 16 <= m.mem.mem_size());
    }

    #[test]
    fn mfree_then_malloc_reuses_freed_space() {
        use asm::Op::{C8, Mem16};
        // malloc a, malloc b, free a, malloc c → c reuses a's address.
        let mut body = asm::ins(0x178, &[C8(16), Mem16(0x0100)]); // a
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0104)])); // b
        body.extend(asm::ins(0x179, &[Mem16(0x0100)])); // mfree a (contents of 0x100)
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0108)])); // c
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        let a = m.mem.read32(0x100).unwrap();
        let c = m.mem.read32(0x108).unwrap();
        assert_eq!(c, a, "freed block should be reused");
    }

    #[test]
    fn setmemsize_while_heap_active_fails() {
        use asm::Op::{C32, C8, Mem16, Zero};
        let mut body = asm::ins(0x178, &[C8(16), Zero]); // malloc 16 (activate heap)
        body.extend(asm::ins(0x103, &[C32(0x1_0000), Mem16(0x0100)])); // setmemsize → result
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "setmemsize must fail");
        assert!(m.diagnostics.iter().any(|d| d.contains("heap")));
    }

    #[test]
    fn malloc_too_large_returns_zero() {
        use asm::Op::{C32, Mem16};
        // A request beyond the heap ceiling fails without allocating.
        let mut body = asm::ins(0x178, &[C32(0x2000_0000), Mem16(0x0100)]);
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0);
    }

    #[test]
    fn freeing_last_block_deactivates_and_shrinks() {
        use asm::Op::{C8, Mem16};
        let mut body = asm::ins(0x178, &[C8(32), Mem16(0x0100)]); // malloc → activates
        body.extend(asm::ins(0x179, &[Mem16(0x0100)])); // free the only block
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        let floor = m.mem.mem_size();
        m.run();
        assert_eq!(m.heap_start, 0, "heap should be inactive again");
        assert_eq!(m.mem.mem_size(), floor, "memory shrinks back to heap start");
    }

    // ── Task 4 (2b): search opcodes ───────────────────────────────────────────

    /// Run `linearsearch` with the given operands; return the stored result.
    fn linear(
        key: asm::Op,
        keysize: i8,
        start: u16,
        structsize: i8,
        numstructs: asm::Op,
        keyoffset: i8,
        options: i8,
        setup: impl FnOnce(&mut Machine),
    ) -> u32 {
        use asm::Op::{C16, C8, Mem16};
        let mut body = asm::ins(
            0x150,
            &[
                key,
                C8(keysize),
                C16(start as i16),
                C8(structsize),
                numstructs,
                C8(keyoffset),
                C8(options),
                Mem16(0x0100),
            ],
        );
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x300, setup);
        m.mem.read32(0x100).unwrap()
    }

    /// Three 8-byte structs at 0x140 with 4-byte keys at offset 0.
    fn three_structs(m: &mut Machine) {
        poke(m, 0x140, &0x1111u32.to_be_bytes());
        poke(m, 0x148, &0x2222u32.to_be_bytes());
        poke(m, 0x150, &0x3333u32.to_be_bytes());
    }

    #[test]
    fn linearsearch_finds_and_reports_absence() {
        use asm::Op::{C32, C8};
        // Present key → struct address.
        assert_eq!(
            linear(C32(0x2222), 4, 0x140, 8, C8(3), 0, 0, three_structs),
            0x148
        );
        // Absent key → 0.
        assert_eq!(linear(C32(0x9999), 4, 0x140, 8, C8(3), 0, 0, three_structs), 0);
        // ReturnIndex (0x04): present → index 2.
        assert_eq!(
            linear(C32(0x3333), 4, 0x140, 8, C8(3), 0, 0x04, three_structs),
            2
        );
        // ReturnIndex + absent → 0xFFFFFFFF.
        assert_eq!(
            linear(C32(0x9999), 4, 0x140, 8, C8(3), 0, 0x04, three_structs),
            0xFFFF_FFFF
        );
    }

    #[test]
    fn linearsearch_key_indirect_and_zero_terminates() {
        use asm::Op::{C32, C8};
        // KeyIndirect (0x01): key bytes live at 0x110.
        let r = linear(C32(0x0110), 4, 0x140, 8, C8(3), 0, 0x01, |m| {
            three_structs(m);
            poke(m, 0x110, &0x2222u32.to_be_bytes());
        });
        assert_eq!(r, 0x148);

        // ZeroKeyTerminates (0x02): a zero key at index 1 halts before 0x3333.
        // NumStructs -1 (no limit) so only the zero key stops it.
        let r = linear(C32(0x3333), 4, 0x140, 8, C8(-1), 0, 0x02, |m| {
            poke(m, 0x140, &0x1111u32.to_be_bytes());
            poke(m, 0x148, &0u32.to_be_bytes()); // zero key terminates here
            poke(m, 0x150, &0x3333u32.to_be_bytes());
        });
        assert_eq!(r, 0, "search should fail at the zero key");
    }

    #[test]
    fn binarysearch_on_sorted_table() {
        use asm::Op::{C16, C32, C8, Mem16};
        // Sorted keys 0x10,0x20,0x30,0x40 in 8-byte structs at 0x140.
        let setup = |m: &mut Machine| {
            for (i, k) in [0x10u32, 0x20, 0x30, 0x40].iter().enumerate() {
                poke(m, 0x140 + 8 * i as u32, &k.to_be_bytes());
            }
        };
        let run = |key: u32, options: i8| -> u32 {
            let mut body = asm::ins(
                0x151,
                &[
                    C32(key),
                    C8(4),
                    C16(0x0140),
                    C8(8),
                    C8(4),
                    C8(0),
                    C8(options),
                    Mem16(0x0100),
                ],
            );
            body.extend(asm::ins(0x120, &[]));
            run_with_ram(body, 0x300, setup).mem.read32(0x100).unwrap()
        };
        assert_eq!(run(0x30, 0), 0x150); // address of the 3rd struct
        assert_eq!(run(0x30, 0x04), 2); // ReturnIndex
        assert_eq!(run(0x25, 0), 0); // absent
        assert_eq!(run(0x10, 0x04), 0); // first element, index 0
        assert_eq!(run(0x40, 0x04), 3); // last element
        assert_eq!(run(0x50, 0x04), 0xFFFF_FFFF); // absent past end
    }

    #[test]
    fn linkedsearch_over_a_linked_list() {
        use asm::Op::{C16, C32, C8, Mem16};
        // Nodes: key at offset 0, next pointer at offset 4.
        let setup = |m: &mut Machine| {
            poke(m, 0x140, &0xAAAAu32.to_be_bytes());
            poke(m, 0x144, &0x0150u32.to_be_bytes());
            poke(m, 0x150, &0xBBBBu32.to_be_bytes());
            poke(m, 0x154, &0x0160u32.to_be_bytes());
            poke(m, 0x160, &0xCCCCu32.to_be_bytes());
            poke(m, 0x164, &0u32.to_be_bytes()); // end of list
        };
        let run = |key: u32| -> u32 {
            let mut body = asm::ins(
                0x152,
                &[C32(key), C8(4), C16(0x0140), C8(0), C8(4), C8(0), Mem16(0x0100)],
            );
            body.extend(asm::ins(0x120, &[]));
            run_with_ram(body, 0x300, setup).mem.read32(0x100).unwrap()
        };
        assert_eq!(run(0xBBBB), 0x150); // middle node
        assert_eq!(run(0xAAAA), 0x140); // head
        assert_eq!(run(0xCCCC), 0x160); // tail
        assert_eq!(run(0x9999), 0); // absent → 0
    }

    // ── Task 5 (2b): gestalt + verify ─────────────────────────────────────────

    #[test]
    fn gestalt_reports_capabilities() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.gestalt(0, 0), 0x0003_0102); // GlulxVersion 3.1.2
        assert_eq!(m.gestalt(1, 0), 0x0000_0100); // TerpVersion 0.1.0
        assert_eq!(m.gestalt(2, 0), 1); // ResizeMem
        assert_eq!(m.gestalt(3, 0), 0); // Undo (deferred)
        assert_eq!(m.gestalt(4, 0), 1); // IOSystem null
        assert_eq!(m.gestalt(4, 2), 1); // IOSystem Glk
        assert_eq!(m.gestalt(4, 1), 0); // IOSystem filter (not implemented)
        assert_eq!(m.gestalt(5, 0), 1); // Unicode
        assert_eq!(m.gestalt(6, 0), 1); // MemCopy
        assert_eq!(m.gestalt(7, 0), 1); // MAlloc
        assert_eq!(m.gestalt(8, 0), 0); // MAllocHeap inactive → 0
        assert_eq!(m.gestalt(9, 0), 0); // Acceleration (deferred)
        assert_eq!(m.gestalt(10, 0), 0); // AccelFunc (deferred)
        assert_eq!(m.gestalt(11, 0), 0); // Float (deferred)
        assert_eq!(m.gestalt(999, 0), 0); // unknown selector
    }

    #[test]
    fn gestalt_opcode_and_mallocheap() {
        use asm::Op::{C8, Mem16, Zero};
        // gestalt(0,0) via the opcode → 0x00030102.
        let mut body = asm::ins(0x100, &[Zero, Zero, Mem16(0x0100)]);
        // malloc 16 (activates heap), then gestalt(8) → heap-start address.
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0104)]));
        body.extend(asm::ins(0x100, &[C8(8), Zero, Mem16(0x0108)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0003_0102);
        let heap_start = m.mem.read32(0x108).unwrap();
        assert_eq!(heap_start, m.mem.read32(0x104).unwrap()); // == first block addr
        assert_eq!(heap_start, m.heap_start);
    }

    #[test]
    fn verify_returns_success() {
        let mut body = asm::ins(0x121, &[asm::Op::Mem16(0x0100)]);
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0);
    }

    // ── Task 1 (2c): save / restore serialization core ────────────────────────

    #[test]
    fn save_restore_roundtrips_ram_stack_heap_registers() {
        let mut m = machine_with_body(&[], vec![]);
        m.mem.write32(0x100, 0xCAFE_BABE).unwrap();
        m.push32(0x1111_2222).unwrap();
        m.push32(0x3333_4444).unwrap();
        m.iosys_mode = 2;
        m.iosys_rock = 0x99;
        m.cur_stringtbl = 0x1234;
        m.pc = 0x40;
        let blk = m.heap_malloc(16);
        assert_ne!(blk, 0);

        let snap = m.save_state();

        // Diverge from the saved state.
        m.mem.write32(0x100, 0).unwrap();
        m.pop32().unwrap();
        m.iosys_mode = 0;
        m.iosys_rock = 0;
        m.cur_stringtbl = 0;
        m.pc = 0;
        m.heap_free(blk).unwrap();

        m.restore_state(&snap).unwrap();

        assert_eq!(m.mem.read32(0x100).unwrap(), 0xCAFE_BABE);
        assert_eq!(m.iosys_mode, 2);
        assert_eq!(m.iosys_rock, 0x99);
        assert_eq!(m.cur_stringtbl, 0x1234);
        assert_eq!(m.pc, 0x40);
        assert_eq!(m.heap_start, blk);
        assert_eq!(m.heap_blocks, vec![(blk, 16)]);
        assert_eq!(m.value_count(), 2);
        assert_eq!(m.pop32().unwrap(), 0x3333_4444);
        assert_eq!(m.pop32().unwrap(), 0x1111_2222);
    }

    #[test]
    fn cmem_resets_to_original_then_applies_diff() {
        let mut m = machine_with_body(&[], vec![]);
        m.mem.write8(0x150, 0xAB).unwrap();
        let snap = m.save_state();
        // A byte changed after the save must be reset to the original image (0).
        m.mem.write8(0x150, 0x00).unwrap();
        m.mem.write8(0x108, 0xFF).unwrap();
        m.restore_state(&snap).unwrap();
        assert_eq!(m.mem.read8(0x150).unwrap(), 0xAB); // saved diff re-applied
        assert_eq!(m.mem.read8(0x108).unwrap(), 0x00); // not in the save → original
    }

    #[test]
    fn restore_rejects_corrupt_save_without_panic() {
        let mut m = machine_with_body(&[], vec![]);
        assert!(matches!(m.restore_state(b"not a save"), Err(GError::BadSave(_))));
        assert!(matches!(m.restore_state(&[]), Err(GError::BadSave(_))));
        let mut good = m.save_state();
        good.truncate(good.len() - 10); // sever a chunk
        assert!(matches!(m.restore_state(&good), Err(GError::BadSave(_))));
    }
}
