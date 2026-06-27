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

// NOTE: Tasks 2–5 build the engine bottom-up; some methods are exercised only by
// unit tests until instruction dispatch wires them in (Task 6). The blanket
// allow is removed once `step()`/dispatch consumes them.
#![allow(dead_code)]

use crate::io::Output;
use crate::memory::Memory;

/// A recoverable runtime fault. Carries a human-readable diagnostic; the run
/// loop records it and Quits rather than panicking.
type R<T> = Result<T, String>;

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
    /// The output sink.
    pub(crate) out: Box<dyn Output>,
    /// Recorded runtime faults / deferred-feature notices.
    pub diagnostics: Vec<String>,
    /// Set once execution has ended (outer return or quit/fault).
    pub(crate) halted: bool,

    // Cached layout of the current frame (recomputed whenever `fp` changes).
    cur_frame_len: u32,
    cur_localspos: u32,
    /// `(offset_within_locals, size_bytes)` for each local of the current frame.
    cur_locals: Vec<(u32, u8)>,
}

fn align_up(v: u32, to: u32) -> u32 {
    (v + to - 1) / to * to
}

impl Machine {
    /// Build a machine over `mem`, entering the start function (no arguments).
    pub fn with_output(mem: Memory, out: Box<dyn Output>) -> Machine {
        let stack_len = (mem.stack_size().max(0x100)) as usize;
        let start = mem.start_func();
        let mut m = Machine {
            mem,
            stack: vec![0u8; stack_len],
            sp: 0,
            fp: 0,
            pc: 0,
            iosys_mode: 0,
            iosys_rock: 0,
            out,
            diagnostics: Vec::new(),
            halted: false,
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

    /// Write `v` to main memory at `addr`, mapping ROM/out-of-range faults to a
    /// diagnostic string.
    pub(crate) fn store_mem(&mut self, addr: u32, v: u32) -> R<()> {
        use crate::memory::WriteFault;
        match self.mem.write32(addr, v) {
            Ok(()) => Ok(()),
            Err(WriteFault::Rom) => {
                self.diagnostics.push(format!("ignored ROM write @{addr:#010x}"));
                Ok(())
            }
            Err(WriteFault::OutOfRange) => Err(format!("memory fault: write32 @{addr:#010x}")),
        }
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

    #[test]
    fn illegal_operand_mode_faults() {
        // Mode 0x4 is unused/illegal for load.
        let mut m = machine_with_body(&[], vec![0x04]);
        assert!(m.read_operands(1, 0).is_err());
        // Mode 0x1 (constant) is illegal as a store target.
        let mut m = machine_with_body(&[], vec![0x01, 0x00]);
        assert!(m.read_operands(0, 1).is_err());
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
}
