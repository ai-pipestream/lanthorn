// Z-machine executor core — ZMSD §14, §15.
//
// Provides `Machine` (memory + CPU state) and `step()` (fetch-decode-execute).
// The pc-advance contract: step() sets state.pc = instr.next_pc BEFORE executing,
// so that call handlers find state.pc already pointing past the call instruction,
// making it the correct return_pc. Branch/jump offsets are relative to next_pc.
//
// Dispatch structure: match on operand_count then opcode number.
// Tasks 10–13 add arms to the same match without restructuring the core.

use crate::cpu::decode::{decode, Branch, Instr, Operand, OperandCount};
use crate::cpu::state::{call_routine, peek_stack, poke_stack, read_var, return_value, write_var, State};
use crate::io::{BufferOutput, Output};
use crate::memory::Memory;
use crate::objects;
use crate::text::decode::{decode_string, zscii_to_char};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of executing one instruction.
#[derive(Debug, PartialEq)]
pub enum StepResult {
    /// Normal execution: continue to next instruction.
    Continue,
    /// `quit` opcode — host should stop the run loop.
    Quit,
    /// `restart` opcode — host should reload and restart.
    Restart,
    /// `read` / `sread` — host must supply a line of input.
    NeedLine { text_buf: u32, parse_buf: u32 },
    /// `read_char` — host must supply a single keypress.
    NeedChar,
    /// `save` — host must write interpreter state to a file.
    SaveRequest,
    /// `restore` — host must read interpreter state from a file.
    RestoreRequest,
}

/// The Z-machine interpreter — ties memory and CPU state together.
/// Fields are `pub` so Tasks 11+ can attach I/O channels.
pub struct Machine {
    pub mem: Memory,
    pub state: State,
    /// Pluggable text output sink. Defaults to `BufferOutput` (Task 11).
    pub out: Box<dyn Output>,
}

impl Machine {
    /// Create a new `Machine` from story memory, using a `BufferOutput` sink.
    /// `state.pc` is set to the header's `initial_pc` field (direct instruction
    /// address for v3/4/5/7/8; v6 is not supported).
    pub fn new(mem: Memory) -> Machine {
        Machine::with_output(mem, Box::new(BufferOutput::new()))
    }

    /// Create a new `Machine` with a custom output sink.
    pub fn with_output(mem: Memory, out: Box<dyn Output>) -> Machine {
        let initial_pc = mem.initial_pc();
        Machine {
            state: State::new(initial_pc),
            mem,
            out,
        }
    }

    /// Borrow the default `BufferOutput` sink if that is what `out` holds, else `None`.
    pub fn buffer_output(&self) -> Option<&BufferOutput> {
        self.out.as_any().downcast_ref::<BufferOutput>()
    }

    /// Execute one instruction and return the result.
    ///
    /// Contract (per task spec):
    ///   1. Decode instruction at `state.pc`.
    ///   2. Advance `state.pc` to `instr.next_pc` BEFORE executing.
    ///   3. Execute the instruction.
    ///
    /// This ensures `state.pc` already points past the call site when
    /// `call_routine` is invoked, giving the correct `return_pc`. Branch and
    /// jump offsets are computed relative to `state.pc` (= next_pc) too.
    pub fn step(&mut self) -> StepResult {
        let version = self.mem.version();
        let instr = decode(&self.mem, self.state.pc, version);

        // CRITICAL: advance PC before executing so call/branch targets are correct.
        self.state.pc = instr.next_pc;

        self.execute(instr)
    }

    // -----------------------------------------------------------------------
    // Main dispatch
    // -----------------------------------------------------------------------

    fn execute(&mut self, instr: Instr) -> StepResult {
        // Resolve all operands left-to-right (Var operands can pop the stack).
        let ops: Vec<u16> = instr
            .operands
            .iter()
            .map(|op| self.resolve(op))
            .collect();

        match instr.operand_count {
            OperandCount::Two => self.exec_2op(instr.opcode, &ops, instr.store, instr.branch),
            OperandCount::One => self.exec_1op(instr.opcode, &ops, instr.store, instr.branch),
            OperandCount::Zero => self.exec_0op(instr.opcode, instr.store, instr.branch, instr.text),
            OperandCount::Var => self.exec_var(instr.opcode, &ops, instr.store, instr.branch),
            OperandCount::Ext => self.exec_ext(instr.opcode, &ops, instr.store),
        }
    }

    // -----------------------------------------------------------------------
    // 2OP opcodes
    // -----------------------------------------------------------------------

    fn exec_2op(
        &mut self,
        opcode: u8,
        ops: &[u16],
        store: Option<u8>,
        branch: Option<Branch>,
    ) -> StepResult {
        let a = ops.get(0).copied().unwrap_or(0);
        let b = ops.get(1).copied().unwrap_or(0);

        match opcode {
            // 0x01 je — branch if a equals ANY of ops[1..]
            // Variable form allows up to 4 operands (ZMSD §14).
            0x01 => {
                let cond = ops.len() > 1 && ops[1..].iter().any(|&x| x == a);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x02 jl — branch if a < b (signed)
            0x02 => {
                let cond = (a as i16) < (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x03 jg — branch if a > b (signed)
            0x03 => {
                let cond = (a as i16) > (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x04 dec_chk — decrement variable (ops[0] = var number), branch if new_val < b.
            // ZMSD §14: first operand is a "variable by reference" — its *value* is the
            // variable number to operate on. In the Long form this is a Small constant
            // (the var number); in the Var form it may be a Var (whose contents are
            // the var number). ops[0] already holds the variable number correctly in
            // both cases after normal operand resolution.
            0x04 => {
                let var = a as u8;
                let old = read_var(&mut self.state, &self.mem, var);
                let new_val = (old as i16).wrapping_sub(1) as u16;
                write_var(&mut self.state, &mut self.mem, var, new_val);
                let cond = (new_val as i16) < (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x05 inc_chk — increment variable (ops[0] = var number), branch if new_val > b.
            0x05 => {
                let var = a as u8;
                let old = read_var(&mut self.state, &self.mem, var);
                let new_val = (old as i16).wrapping_add(1) as u16;
                write_var(&mut self.state, &mut self.mem, var, new_val);
                let cond = (new_val as i16) > (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x06 jin — branch if obj a is a child of obj b (parent(a) == b)
            0x06 => {
                let parent = objects::get_parent(&self.mem, a);
                self.do_branch(branch, parent == b);
                StepResult::Continue
            }
            // 0x07 test — branch if all bits in b are set in a (bitmap test, ZMSD §15)
            0x07 => {
                self.do_branch(branch, a & b == b);
                StepResult::Continue
            }
            // 0x0A test_attr — branch if object a has attribute b set (ZMSD §14)
            0x0A => {
                let cond = objects::get_attr(&self.mem, a, b as u8);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x0B set_attr — set attribute b on object a (side effect only)
            0x0B => {
                objects::set_attr(&mut self.mem, a, b as u8);
                StepResult::Continue
            }
            // 0x0C clear_attr — clear attribute b on object a (side effect only)
            0x0C => {
                objects::clear_attr(&mut self.mem, a, b as u8);
                StepResult::Continue
            }
            // 0x0E insert_obj — make object a the first child of object b
            0x0E => {
                objects::insert_obj(&mut self.mem, a, b);
                StepResult::Continue
            }
            // 0x11 get_prop — store property b of object a (fallback to default)
            0x11 => {
                let val = objects::get_prop(&self.mem, a, b as u8);
                self.do_store(store, val);
                StepResult::Continue
            }
            // 0x12 get_prop_addr — store address of property b data in object a (0 if absent)
            0x12 => {
                let addr = objects::get_prop_addr(&self.mem, a, b as u8);
                self.do_store(store, addr);
                StepResult::Continue
            }
            // 0x13 get_next_prop — store next property number after b in object a (0=last/first)
            0x13 => {
                let next = objects::get_next_prop(&self.mem, a, b as u8);
                self.do_store(store, next as u16);
                StepResult::Continue
            }
            // 0x08 or — bitwise OR
            0x08 => {
                self.do_store(store, a | b);
                StepResult::Continue
            }
            // 0x09 and — bitwise AND
            0x09 => {
                self.do_store(store, a & b);
                StepResult::Continue
            }
            // 0x0D store — write value b into variable a (by reference).
            // ZMSD §6.3.4: if variable number == 0, REPLACE (do not push) the stack top.
            0x0D => {
                let var = a as u8;
                if var == 0 {
                    poke_stack(&mut self.state, b);
                } else {
                    write_var(&mut self.state, &mut self.mem, var, b);
                }
                StepResult::Continue
            }
            // 0x14 add (signed)
            0x14 => {
                let result = (a as i16).wrapping_add(b as i16) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x15 sub (signed)
            0x15 => {
                let result = (a as i16).wrapping_sub(b as i16) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x16 mul (signed)
            0x16 => {
                let result = (a as i16).wrapping_mul(b as i16) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x17 div (signed); division by zero → 0 (ZMSD §15 "interpreter may halt or trap")
            0x17 => {
                let result = if b == 0 { 0 } else { ((a as i16) / (b as i16)) as u16 };
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x18 mod (signed); mod by zero → 0
            0x18 => {
                let result = if b == 0 { 0 } else { ((a as i16) % (b as i16)) as u16 };
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x19 call_2s — call with one arg, store result (v4+)
            0x19 => {
                call_routine(&mut self.state, &mut self.mem, a, &[b], store);
                StepResult::Continue
            }
            // 0x1A call_2n — call with one arg, discard result (v5+)
            0x1A => {
                call_routine(&mut self.state, &mut self.mem, a, &[b], None);
                StepResult::Continue
            }
            // Unknown / unimplemented 2OP — no-op seam for Tasks 10+ (object/text ops)
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // 1OP opcodes
    // -----------------------------------------------------------------------

    fn exec_1op(
        &mut self,
        opcode: u8,
        ops: &[u16],
        store: Option<u8>,
        branch: Option<Branch>,
    ) -> StepResult {
        let a = ops.get(0).copied().unwrap_or(0);

        match opcode {
            // 0x00 jz — branch if a == 0
            0x00 => {
                self.do_branch(branch, a == 0);
                StepResult::Continue
            }
            // 0x01 get_sibling — store sibling of a AND branch if sibling != 0 (ZMSD §14)
            0x01 => {
                let sib = objects::get_sibling(&self.mem, a);
                self.do_store(store, sib);
                self.do_branch(branch, sib != 0);
                StepResult::Continue
            }
            // 0x02 get_child — store child of a AND branch if child != 0 (ZMSD §14)
            0x02 => {
                let child = objects::get_child(&self.mem, a);
                self.do_store(store, child);
                self.do_branch(branch, child != 0);
                StepResult::Continue
            }
            // 0x03 get_parent — store parent of a, no branch (ZMSD §14)
            0x03 => {
                let parent = objects::get_parent(&self.mem, a);
                self.do_store(store, parent);
                StepResult::Continue
            }
            // 0x04 get_prop_len — store length in bytes of property whose data address is a
            0x04 => {
                let len = objects::get_prop_len(&self.mem, a);
                self.do_store(store, len as u16);
                StepResult::Continue
            }
            // 0x07 print_addr — print string at byte address a
            0x07 => {
                let (s, _) = decode_string(&self.mem, a as u32);
                self.out.print(&s);
                StepResult::Continue
            }
            // 0x09 remove_obj — remove object a from its parent's child list
            0x09 => {
                objects::remove_obj(&mut self.mem, a);
                StepResult::Continue
            }
            // 0x0A print_obj — print the short name of object a via the output sink
            0x0A => {
                let name = objects::short_name(&self.mem, a);
                self.out.print(&name);
                StepResult::Continue
            }
            // 0x0D print_paddr — print string at packed address a
            0x0D => {
                let byte_addr = self.mem.unpack_string(a);
                let (s, _) = decode_string(&self.mem, byte_addr);
                self.out.print(&s);
                StepResult::Continue
            }
            // 0x05 inc — increment variable by reference (no store/branch)
            0x05 => {
                let var = a as u8;
                let v = read_var(&mut self.state, &self.mem, var);
                write_var(&mut self.state, &mut self.mem, var, v.wrapping_add(1));
                StepResult::Continue
            }
            // 0x06 dec — decrement variable by reference
            0x06 => {
                let var = a as u8;
                let v = read_var(&mut self.state, &self.mem, var);
                write_var(&mut self.state, &mut self.mem, var, v.wrapping_sub(1));
                StepResult::Continue
            }
            // 0x08 call_1s — call routine at packed addr a, no args, store result
            0x08 => {
                call_routine(&mut self.state, &mut self.mem, a, &[], store);
                StepResult::Continue
            }
            // 0x0B ret — return value a from current routine
            0x0B => {
                return_value(&mut self.state, &mut self.mem, a);
                StepResult::Continue
            }
            // 0x0C jump — unconditional; operand is signed i16 offset.
            // ZMSD §14: pc = pc + offset - 2 (where pc is already next_pc).
            0x0C => {
                let offset = a as i16;
                self.state.pc = (self.state.pc as i32 + offset as i32 - 2) as u32;
                StepResult::Continue
            }
            // 0x0E load — read value of variable a, store result.
            // ZMSD §6.3.4: if variable number == 0, PEEK (do not pop) the stack top.
            0x0E => {
                let var = a as u8;
                let val = if var == 0 {
                    peek_stack(&self.state)
                } else {
                    read_var(&mut self.state, &self.mem, var)
                };
                self.do_store(store, val);
                StepResult::Continue
            }
            // 0x0F not (v1–4, stores) / call_1n (v5+, no store)
            0x0F => {
                if self.mem.version() <= 4 {
                    self.do_store(store, !a);
                } else {
                    call_routine(&mut self.state, &mut self.mem, a, &[], None);
                }
                StepResult::Continue
            }
            // Unknown / unimplemented 1OP — no-op seam (object ops in Task 10)
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // 0OP opcodes
    // -----------------------------------------------------------------------

    // Branch and store are threaded through because verify/piracy need do_branch.
    fn exec_0op(
        &mut self,
        opcode: u8,
        store: Option<u8>,
        branch: Option<Branch>,
        text: Option<(String, u32)>,
    ) -> StepResult {
        match opcode {
            // 0x00 rtrue — return 1 from current routine
            0x00 => {
                return_value(&mut self.state, &mut self.mem, 1);
                StepResult::Continue
            }
            // 0x01 rfalse — return 0 from current routine
            0x01 => {
                return_value(&mut self.state, &mut self.mem, 0);
                StepResult::Continue
            }
            // 0x02 print — print the inline string (from Instr.text)
            0x02 => {
                if let Some((s, _)) = text {
                    self.out.print(&s);
                }
                StepResult::Continue
            }
            // 0x03 print_ret — print inline string + newline, then return true
            0x03 => {
                if let Some((s, _)) = text {
                    self.out.print(&s);
                }
                self.out.print("\n");
                return_value(&mut self.state, &mut self.mem, 1);
                StepResult::Continue
            }
            // 0x04 nop — no operation
            0x04 => StepResult::Continue,
            // 0x07 restart
            0x07 => StepResult::Restart,
            // 0x08 ret_popped — pop eval stack and return that value
            0x08 => {
                let val = read_var(&mut self.state, &self.mem, 0); // var 0 = pop
                return_value(&mut self.state, &mut self.mem, val);
                StepResult::Continue
            }
            // 0x09 pop (v1–4) / catch (v5+, stores frame depth)
            0x09 => {
                if self.mem.version() <= 4 {
                    // pop: discard top of eval stack
                    let _ = read_var(&mut self.state, &self.mem, 0);
                } else {
                    // catch: stores current call stack depth (frame count)
                    let depth = self.state.frames.len() as u16;
                    self.do_store(store, depth);
                }
                StepResult::Continue
            }
            // 0x0A quit
            0x0A => StepResult::Quit,
            // 0x0B new_line — print newline
            0x0B => {
                self.out.print("\n");
                StepResult::Continue
            }
            // 0x0D verify — branch always true (stub; real checksum in Task 16)
            0x0D => {
                self.do_branch(branch, true);
                StepResult::Continue
            }
            // 0x0F piracy — branch always true (ZMSD: treat as legit copy)
            0x0F => {
                self.do_branch(branch, true);
                StepResult::Continue
            }
            // 0x05 save — stub for Task 13
            0x05 => StepResult::SaveRequest,
            // 0x06 restore — stub for Task 13
            0x06 => StepResult::RestoreRequest,
            // Unknown / unimplemented 0OP — no-op
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // VAR opcodes
    // -----------------------------------------------------------------------

    fn exec_var(
        &mut self,
        opcode: u8,
        ops: &[u16],
        store: Option<u8>,
        branch: Option<Branch>,
    ) -> StepResult {
        match opcode {
            // 0x00 call / call_vs — call with up to 3 args, store result
            0x00 => {
                let packed = ops.get(0).copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, store);
                StepResult::Continue
            }
            // 0x03 put_prop — set property ops[1] of object ops[0] to value ops[2]
            0x03 => {
                let obj  = ops.get(0).copied().unwrap_or(0);
                let prop = ops.get(1).copied().unwrap_or(0) as u8;
                let val  = ops.get(2).copied().unwrap_or(0);
                objects::put_prop(&mut self.mem, obj, prop, val);
                StepResult::Continue
            }
            // 0x05 print_char — print a single ZSCII character
            0x05 => {
                let zscii = ops.get(0).copied().unwrap_or(0);
                let ch = zscii_to_char(zscii);
                let mut buf = [0u8; 4];
                self.out.print(ch.encode_utf8(&mut buf));
                StepResult::Continue
            }
            // 0x06 print_num — print operand as signed decimal
            0x06 => {
                let val = ops.get(0).copied().unwrap_or(0) as i16;
                let s = format!("{}", val);
                self.out.print(&s);
                StepResult::Continue
            }
            // 0x08 push — push value onto eval stack
            0x08 => {
                let val = ops.get(0).copied().unwrap_or(0);
                write_var(&mut self.state, &mut self.mem, 0, val); // var 0 = push
                StepResult::Continue
            }
            // 0x09 pull — pop from eval stack and store into variable ops[0]
            0x09 => {
                let var = ops.get(0).copied().unwrap_or(0) as u8;
                let val = read_var(&mut self.state, &self.mem, 0); // pop stack
                write_var(&mut self.state, &mut self.mem, var, val);
                StepResult::Continue
            }
            // 0x0C call_vs2 — like call_vs but with 2 type bytes, stores result
            0x0C => {
                let packed = ops.get(0).copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, store);
                StepResult::Continue
            }
            // 0x04 sread/read — stub for Task 12
            0x04 => {
                let text_buf = ops.get(0).copied().unwrap_or(0) as u32;
                let parse_buf = ops.get(1).copied().unwrap_or(0) as u32;
                StepResult::NeedLine { text_buf, parse_buf }
            }
            // 0x16 read_char — stub for Task 12
            0x16 => StepResult::NeedChar,
            // 0x18 not (VAR form, v5+) — bitwise complement
            0x18 => {
                let val = ops.get(0).copied().unwrap_or(0);
                self.do_store(store, !val);
                StepResult::Continue
            }
            // 0x19 call_vn — call with up to 3 args, discard result (v5+)
            0x19 => {
                let packed = ops.get(0).copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, None);
                StepResult::Continue
            }
            // 0x1A call_vn2 — like call_vn but with 2 type bytes
            0x1A => {
                let packed = ops.get(0).copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, None);
                StepResult::Continue
            }
            // 0x1F check_arg_count (v5+) — branch if arg_count >= ops[0]
            0x1F => {
                let n = ops.get(0).copied().unwrap_or(0);
                let arg_count = self.state.frames.last().map(|f| f.arg_count as u16).unwrap_or(0);
                self.do_branch(branch, arg_count >= n);
                StepResult::Continue
            }
            // Unknown / unimplemented VAR — no-op seam (screen/text ops in Tasks 11–12)
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // EXT opcodes (v5+)
    // -----------------------------------------------------------------------

    fn exec_ext(&mut self, opcode: u8, ops: &[u16], store: Option<u8>) -> StepResult {
        // No EXT opcodes needed in this task group. No-op seam for Tasks 10+.
        let _ = (opcode, ops, store);
        StepResult::Continue
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Resolve an `Operand` to a u16 value.
    /// NOTE: `Var` resolution may pop the eval stack — call left-to-right exactly once.
    fn resolve(&mut self, op: &Operand) -> u16 {
        match op {
            Operand::Large(v) => *v,
            Operand::Small(v) => *v as u16,
            Operand::Var(n) => read_var(&mut self.state, &self.mem, *n),
        }
    }

    /// Execute a branch: if condition matches `branch.on_true`, take the branch.
    ///
    /// Offset 0 → return false (0) from current routine.
    /// Offset 1 → return true (1) from current routine.
    /// Else → pc += offset - 2  (offset is relative to next_pc already in state.pc).
    pub fn do_branch(&mut self, branch: Option<Branch>, cond: bool) {
        let br = match branch {
            Some(b) => b,
            None => return,
        };
        if cond == br.on_true {
            match br.offset {
                0 => return_value(&mut self.state, &mut self.mem, 0),
                1 => return_value(&mut self.state, &mut self.mem, 1),
                off => {
                    self.state.pc = (self.state.pc as i32 + off as i32 - 2) as u32;
                }
            }
        }
    }

    /// Store `val` into variable `var` if `var` is Some.
    pub fn do_store(&mut self, var: Option<u8>, val: u16) {
        if let Some(v) = var {
            write_var(&mut self.state, &mut self.mem, v, val);
        }
    }

    /// Read global variable N (0-based). Convenience for tests and Tasks 11+.
    pub fn global(&self, n: u8) -> u16 {
        let base = self.mem.global_vars() as u32;
        self.mem.read_word(base + n as u32 * 2)
    }
}

// ---------------------------------------------------------------------------
// Memory accessor
// ---------------------------------------------------------------------------

impl Memory {
    /// Initial PC from the story header (direct instruction address for v3–v8).
    pub fn initial_pc(&self) -> u32 {
        self.read_word(0x06) as u32
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;

    // -----------------------------------------------------------------------
    // Tiny assembler for test programs
    //
    // `Asm` describes Z-machine instructions; `assemble()` emits bytes.
    // Targets v5 (no local-initial-value words in routine headers).
    // Designed to be extended by Tasks 10–13 executor tests.
    //
    // Usage:
    //   let mut m = build_test_machine(&[Asm::Add(C(2), C(3), DG(0)), Asm::Quit]);
    //   run_until_quit(&mut m);
    //   assert_eq!(m.global(0), 5);
    //
    // Supported forms: Long-form 2OP (small/var operands), Short-form 1OP,
    // Variable-form call_vs (large packed addr + small/var args), 0OP.
    // For Large-operand instructions, write raw bytes directly (see jump_negative_offset test).
    // -----------------------------------------------------------------------

    /// An operand value for test assembly.
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    pub(crate) enum Op {
        /// Small constant (0..=255).
        Const(u8),
        /// Global variable reference (var number 0x10 + n).
        Global(u8),
        /// Local variable reference (var number n, 1-based).
        Local(u8),
    }

    pub(crate) use Op::Const as C;
    pub(crate) use Op::Global as G;

    /// Store destination for test assembly.
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    pub(crate) enum Dest {
        /// Store into global variable N (var number = 0x10 + N).
        Global(u8),
        /// Store into local variable N (var number = N).
        Local(u8),
    }

    pub(crate) use Dest::Global as DG;

    /// Assembler instructions (subset used by Tasks 9–13 executor tests).
    #[allow(dead_code)]
    pub(crate) enum Asm {
        /// add a, b -> dest  (signed)
        Add(Op, Op, Dest),
        /// mul a, b -> dest  (signed)
        Mul(Op, Op, Dest),
        /// sub a, b -> dest  (signed)
        Sub(Op, Op, Dest),
        /// je a, b — branch if a == b, taken branch skips next instruction (offset=2)
        JeTrue(Op, Op),
        /// je a, b — fall through if a == b; branch (skip) if a != b
        JeNot(Op, Op),
        /// jz a — branch if a == 0, taken → skip next instruction (offset=2)
        JzTrue(Op),
        /// jump offset (signed i16; applied as: pc = pc + offset - 2)
        Jump(i16),
        /// inc_chk var_num, val — increment var, branch (skip next) if new_val > val
        IncChk(u8, Op),
        /// dec_chk var_num, val — decrement var, branch (skip next) if new_val < val
        DecChk(u8, Op),
        /// call_vs packed_addr, args, dest — VAR:0x00 with Large packed addr
        CallVs(u16, Vec<Op>, Dest),
        /// ret val — return value from current routine
        Ret(Op),
        /// rtrue — return 1
        Rtrue,
        /// rfalse — return 0
        Rfalse,
        /// quit — halt interpreter
        Quit,
        /// nop — no operation
        Nop,
        /// push val — push onto eval stack
        Push(Op),
    }

    /// Operand type code used in Variable-form type bytes (2-bit ZMSD encoding):
    ///   0b01 = small constant, 0b10 = variable reference.
    fn op_type(op: Op) -> u8 {
        match op {
            Op::Const(_) => 0b01,
            Op::Global(_) | Op::Local(_) => 0b10,
        }
    }

    /// Long-form operand bit: 0 = small constant, 1 = variable reference.
    /// (Long form encodes operand types as single bits, not 2-bit type codes.)
    fn op_long_bit(op: Op) -> u8 {
        match op {
            Op::Const(_) => 0,
            Op::Global(_) | Op::Local(_) => 1,
        }
    }

    /// The byte value to emit for an operand (constant value or variable number).
    fn op_byte(op: Op) -> u8 {
        match op {
            Op::Const(v) => v,
            Op::Global(n) => 0x10 + n,
            Op::Local(n) => n,
        }
    }

    fn dest_var(d: Dest) -> u8 {
        match d {
            Dest::Global(n) => 0x10 + n,
            Dest::Local(n) => n,
        }
    }

    /// Emit a long-form 2OP instruction.
    /// Bits 6 and 5 of the opcode byte encode operand types (0=small const, 1=variable).
    fn emit_long2op(out: &mut Vec<u8>, opcode: u8, a: Op, b: Op, store: Option<u8>, branch: Option<(bool, i16)>) {
        let t1 = op_long_bit(a); // 0=small const, 1=variable
        let t2 = op_long_bit(b);
        let ob = (t1 << 6) | (t2 << 5) | (opcode & 0x1F);
        out.push(ob);
        out.push(op_byte(a));
        out.push(op_byte(b));
        if let Some(sv) = store { out.push(sv); }
        if let Some((on_true, offset)) = branch { emit_branch(out, on_true, offset); }
    }

    /// Emit branch data (single-byte for 0..=63, two-byte otherwise).
    fn emit_branch(out: &mut Vec<u8>, on_true: bool, offset: i16) {
        if offset >= 0 && offset <= 63 {
            // Single-byte: bit7=on_true, bit6=1 (short form), bits5-0=offset
            out.push(if on_true { 0x80 } else { 0x00 } | 0x40 | (offset as u8 & 0x3F));
        } else {
            // Two-byte: 14-bit signed (bits 13..0 of raw, sign-extended)
            let raw = (offset as u16) & 0x3FFF;
            let high6 = ((raw >> 8) & 0x3F) as u8;
            let low8 = (raw & 0xFF) as u8;
            out.push(if on_true { 0x80 } else { 0x00 } | high6);
            out.push(low8);
        }
    }

    /// Emit VAR-form type byte and operand bytes (up to 4 operands).
    fn emit_var_ops(out: &mut Vec<u8>, ops: &[Op]) {
        // Type byte: MSB pair = first operand type; 0b11 = omitted
        let mut type_byte: u8 = 0xFF;
        for (i, op) in ops.iter().enumerate().take(4) {
            let t = op_type(*op);
            let shift = 6u8.saturating_sub(2 * i as u8);
            type_byte &= !(0b11 << shift);
            type_byte |= (t & 0b11) << shift;
        }
        out.push(type_byte);
        for op in ops.iter().take(4) { out.push(op_byte(*op)); }
    }

    /// Assemble `Asm` instructions into a byte vector.
    pub(crate) fn assemble(instrs: &[Asm]) -> Vec<u8> {
        let mut out = Vec::new();
        for instr in instrs {
            match instr {
                Asm::Add(a, b, d) => emit_long2op(&mut out, 0x14, *a, *b, Some(dest_var(*d)), None),
                Asm::Mul(a, b, d) => emit_long2op(&mut out, 0x16, *a, *b, Some(dest_var(*d)), None),
                Asm::Sub(a, b, d) => emit_long2op(&mut out, 0x15, *a, *b, Some(dest_var(*d)), None),
                Asm::JeTrue(a, b) => emit_long2op(&mut out, 0x01, *a, *b, None, Some((true, 2))),
                Asm::JeNot(a, b)  => emit_long2op(&mut out, 0x01, *a, *b, None, Some((false, 2))),
                Asm::JzTrue(a) => {
                    // Short form 1OP jz (opcode=0) with small constant: 0x90
                    out.push(0x90);
                    out.push(op_byte(*a));
                    emit_branch(&mut out, true, 2);
                }
                Asm::Jump(offset) => {
                    // Short form 1OP jump (0x0C) with large constant: 0x8C + 2 bytes
                    out.push(0x8C);
                    let v = *offset as u16;
                    out.push((v >> 8) as u8);
                    out.push((v & 0xFF) as u8);
                }
                Asm::IncChk(var, b) => {
                    // Long form 2OP inc_chk (0x05), var number as small const, branch taken=skip
                    emit_long2op(&mut out, 0x05, Op::Const(*var), *b, None, Some((true, 2)));
                }
                Asm::DecChk(var, b) => {
                    // Long form 2OP dec_chk (0x04), var number as small const, branch taken=skip
                    emit_long2op(&mut out, 0x04, Op::Const(*var), *b, None, Some((true, 2)));
                }
                Asm::CallVs(packed, args, d) => {
                    // VAR form call_vs (0x00) with Large first operand (packed addr)
                    out.push(0xE0); // 11 1 00000 = VAR class, opcode 0
                    // Type byte: first = large const (0b00), rest from args
                    let mut type_byte: u8 = 0xFF;
                    type_byte &= !(0b11 << 6); // first = large (0b00)
                    for (i, arg) in args.iter().enumerate().take(3) {
                        let t = op_type(*arg);
                        let shift = 4u8.saturating_sub(2 * i as u8);
                        type_byte &= !(0b11 << shift);
                        type_byte |= (t & 0b11) << shift;
                    }
                    out.push(type_byte);
                    out.push((*packed >> 8) as u8);
                    out.push((*packed & 0xFF) as u8);
                    for arg in args.iter().take(3) { out.push(op_byte(*arg)); }
                    out.push(dest_var(*d));
                }
                Asm::Ret(a) => {
                    // Short form 1OP ret (opcode=0x0B) with small constant: 0x9B
                    out.push(0x9B);
                    out.push(op_byte(*a));
                }
                Asm::Rtrue  => out.push(0xB0),
                Asm::Rfalse => out.push(0xB1),
                Asm::Quit   => out.push(0xBA),
                Asm::Nop    => out.push(0xB4),
                Asm::Push(a) => {
                    // VAR form push (0xE8): 11 1 01000
                    out.push(0xE8);
                    emit_var_ops(&mut out, &[*a]);
                }
            }
        }
        out
    }

    /// Build a `Machine` with `instrs` placed at 0x10 in a v5 story.
    /// Overrides `state.pc` to 0x10 (the header's initial_pc is 0x40).
    pub(crate) fn build_test_machine(instrs: &[Asm]) -> Machine {
        let bytes = assemble(instrs);
        let mut buf = sample_story(5);
        for (i, &b) in bytes.iter().enumerate() {
            buf[0x10 + i] = b;
        }
        let mem = Memory::new(buf).unwrap();
        let mut machine = Machine::new(mem);
        machine.state.pc = 0x10;
        machine
    }

    /// Run `step()` until `Quit` (safety limit: 10 000 steps).
    pub(crate) fn run_until_quit(machine: &mut Machine) -> u32 {
        for i in 0..10_000u32 {
            if matches!(machine.step(), StepResult::Quit) {
                return i + 1;
            }
        }
        panic!("step limit exceeded without Quit");
    }

    // -----------------------------------------------------------------------
    // Test (a): (2 + 3) * 4 = 20 stored in global 0
    // -----------------------------------------------------------------------

    #[test]
    fn executes_add_mul_into_global() {
        let mut m = build_test_machine(&[
            Asm::Add(C(2), C(3), DG(0)),  // G0 = 2 + 3 = 5
            Asm::Mul(G(0), C(4), DG(0)),  // G0 = G0 * 4 = 20
            Asm::Quit,
        ]);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 20);
    }

    // -----------------------------------------------------------------------
    // Test (b): je branch taken vs not taken
    //
    // ZMSD §4.7: branch offset is relative to the instruction AFTER the branch
    // bytes (= next_pc). offset=2 → no movement (fall-through). To skip an
    // N-byte instruction, use offset = N + 2.
    //
    // We hand-assemble to control the exact offsets.
    // -----------------------------------------------------------------------

    #[test]
    fn je_branch_taken_and_not_taken() {
        // Layout at 0x10 (hand-assembled bytes):
        //
        //   [TAKEN path]
        //   0x10: je 5, 5  (long form, both small const, opcode=0x01, branch)
        //         Long-form opcode: bit6=t1(small=0), bit5=t2(small=0), opcode=0x01 → 0x01
        //         bytes: 0x01, 0x05, 0x05, branch_byte
        //         branch: on_true=1, skip Add(1,0,G0) which is 4 bytes → offset = 4+2 = 6
        //         branch_byte (single): 0x80 | 0x40 | 6 = 0xC6
        //         → 4 bytes total (0x10–0x13), next_pc = 0x14
        //         branch taken: pc = 0x14 + 6 - 2 = 0x18 (skips Add)
        //   0x14: add 1, 0 → G0  (4 bytes, skipped when branch taken)
        //   0x18: add 0, 7 → G0  (4 bytes: 0x14→G0=1; then 0x18→G0=7)
        //   0x1C: quit (1 byte)

        let mut buf = sample_story(5);
        // 0x10: je 5, 5: opcode=0x01, both small (bits6=0,5=0), branch on_true offset=6
        buf[0x10] = 0x01; // je, small+small
        buf[0x11] = 5;    // a=5
        buf[0x12] = 5;    // b=5
        buf[0x13] = 0xC6; // branch: on_true=1, short form, offset=6
        // 0x14: add 1, 0 → G0 (long form, small+small: opcode=0x14, bit6=0, bit5=0)
        buf[0x14] = 0x14;
        buf[0x15] = 1;
        buf[0x16] = 0;
        buf[0x17] = 0x10; // store → G0
        // 0x18: add 0, 7 → G0
        buf[0x18] = 0x14;
        buf[0x19] = 0;
        buf[0x1A] = 7;
        buf[0x1B] = 0x10;
        // 0x1C: quit
        buf[0x1C] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 7, "je taken: Add(1,0) skipped, G0=7 from Add(0,7)");

        // NOT taken: je 5, 6 → falls through to Add(1,0,G0) → G0=1, then quit
        let mut buf2 = sample_story(5);
        buf2[0x10] = 0x01; // je, small+small
        buf2[0x11] = 5;
        buf2[0x12] = 6;    // b=6 ≠ a, branch NOT taken
        buf2[0x13] = 0xC6; // branch: on_true, offset=6 (irrelevant — branch not taken)
        // fall-through → Add(1,0→G0)
        buf2[0x14] = 0x14;
        buf2[0x15] = 1;
        buf2[0x16] = 0;
        buf2[0x17] = 0x10;
        // quit
        buf2[0x18] = 0xBA;

        let mem2 = Memory::new(buf2).unwrap();
        let mut m2 = Machine::new(mem2);
        m2.state.pc = 0x10;
        run_until_quit(&mut m2);
        assert_eq!(m2.global(0), 1, "je not taken: falls through, G0=1");
    }

    // -----------------------------------------------------------------------
    // Test (c): call/return — routine returns 42 into global 0
    // -----------------------------------------------------------------------

    #[test]
    fn call_and_return_value() {
        // Routine at byte 0x80 (v5 packed addr = 0x80 / 4 = 0x20).
        // Routine header: 0 locals (1 byte), then: ret 42 (2 bytes).
        // Main at 0x10: call_vs packed=0x0020 → G0; quit.
        let mut buf = sample_story(5);

        // Routine header + body at 0x80
        buf[0x80] = 0;    // local count = 0 (v5)
        buf[0x81] = 0x9B; // ret, small const
        buf[0x82] = 42;

        // Main: call_vs 0x0020 → G0 (0x10); quit
        buf[0x10] = 0xE0;               // call_vs (VAR:0x00)
        buf[0x11] = 0b00_11_11_11;      // type byte: large, omit, omit, omit
        buf[0x12] = 0x00;               // packed addr high = 0x0020
        buf[0x13] = 0x20;
        buf[0x14] = 0x10;               // store → global 0 (var 0x10)
        buf[0x15] = 0xBA;               // quit

        let mem = Memory::new(buf).unwrap();
        let mut machine = Machine::new(mem);
        machine.state.pc = 0x10;
        run_until_quit(&mut machine);
        assert_eq!(machine.global(0), 42);
    }

    // -----------------------------------------------------------------------
    // Test (d): jump with negative offset loops correctly
    // -----------------------------------------------------------------------

    #[test]
    fn jump_negative_offset() {
        // Program: increment G0 each iteration, exit when G0 > 3.
        //
        // Byte layout (hand-assembled):
        //   0x10: add G0,1 → G0   (long form, var+small) — 4 bytes, next_pc=0x14
        //   0x14: jg G0,3         (long form, var+small, + branch byte) — 4 bytes, next_pc=0x18
        //         branch on_true, offset=5: target = 0x18 + 5 - 2 = 0x1B (skip 3-byte jump)
        //   0x18: jump -9         (short 1OP large const) — 3 bytes, next_pc=0x1B
        //         offset = 0x10 - 0x1B + 2 = -9 → pc = 0x1B + (-9) - 2 = 0x10 ✓
        //   0x1B: quit

        let mut buf = sample_story(5);

        // 0x10: add G0(var=0x10), 1 → G0; long form var+small
        // t1=var(bit6=1), t2=small(bit5=0), opcode=0x14 → 0b0_1_0_10100 = 0x54
        buf[0x10] = 0x54; // add, var+small
        buf[0x11] = 0x10; // G0
        buf[0x12] = 1;
        buf[0x13] = 0x10; // store → G0

        // 0x14: jg G0, 3; long form var+small, opcode=0x03
        // t1=var(bit6=1), t2=small(bit5=0), opcode=0x03 → 0b0_1_0_00011 = 0x43
        buf[0x14] = 0x43; // jg, var+small
        buf[0x15] = 0x10; // G0
        buf[0x16] = 3;    // const 3
        // branch: on_true=1, single-byte, offset=5 → 0x80|0x40|5 = 0xC5
        // next_pc of jg = 0x18; branch target = 0x18 + 5 - 2 = 0x1B (past the jump) ✓
        buf[0x17] = 0xC5;

        // 0x18: jump -9 (large const); next_pc = 0x1B
        // offset = target(0x10) - next_pc(0x1B) + 2 = 0x10 - 0x1B + 2 = -9
        buf[0x18] = 0x8C;
        let jmp_off: i16 = 0x10i16 - 0x1Bi16 + 2; // = -9
        buf[0x19] = (jmp_off as u16 >> 8) as u8;
        buf[0x1A] = (jmp_off as u16 & 0xFF) as u8;

        // 0x1B: quit
        buf[0x1B] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut machine = Machine::new(mem);
        machine.state.pc = 0x10;
        run_until_quit(&mut machine);
        // Iterations: G0: 0→1(jg F), 1→2(jg F), 2→3(jg F), 3→4(jg 4>3=T→branch→quit)
        assert_eq!(machine.global(0), 4);
    }

    // -----------------------------------------------------------------------
    // Test (e): inc_chk and dec_chk branch behavior
    //
    // We hand-assemble to control branch offsets precisely.
    // ZMSD §4.7: branch offset relative to next_pc; offset N → target = next_pc + N - 2.
    // To skip a 4-byte instruction (long-form add with store): offset = 4 + 2 = 6.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Test: load sp peeks without popping (ZMSD §6.3.4)
    // -----------------------------------------------------------------------

    #[test]
    fn load_sp_peeks_not_pops() {
        // Layout at 0x10:
        //   push 0xAAAA              (VAR:0x08, 3 bytes: E8 type_byte val)
        //   push 0xBEEF              (VAR:0x08, 3 bytes)
        //   load sp -> G0            (1OP:0x0E, small const 0x00, store G0)
        //     Short form 1OP with small const: 0x9E, operand=0x00, store=0x10
        //     3 bytes total
        //   quit
        //
        // After load: G0 == 0xBEEF (top value), stack depth still 2.
        let mut buf = sample_story(5);
        let mut pos = 0x10usize;

        // push 0xAAAA: 0xE8 type_byte(large=0b00...) value_hi value_lo
        // VAR:push uses emit_var_ops but for a large constant we need 2 bytes.
        // Actually looking at the Asm::Push handler: it uses emit_var_ops which emits
        // a 1-byte type byte + 1-byte operand (small const). For a 16-bit value we
        // need a different approach. Use raw bytes: write directly.
        // push 0xBEEF needs a large constant. Emit as VAR:0x08 with large operand:
        //   0xE8 (VAR push), type byte: large=0b00 for first op → 0b00_11_11_11 = 0x3F
        //   then 2-byte value: hi, lo
        buf[pos] = 0xE8; pos += 1;   // VAR push
        buf[pos] = 0x3F; pos += 1;   // type: first=large(0b00), rest=omit(0b11)
        buf[pos] = 0xAA; pos += 1;   // 0xAAAA hi
        buf[pos] = 0xAA; pos += 1;   // 0xAAAA lo

        buf[pos] = 0xE8; pos += 1;   // VAR push
        buf[pos] = 0x3F; pos += 1;
        buf[pos] = 0xBE; pos += 1;   // 0xBEEF hi
        buf[pos] = 0xEF; pos += 1;   // 0xBEEF lo

        // load sp (var 0) -> G0: short 1OP small const, opcode=0x0E → 0x9E
        buf[pos] = 0x9E; pos += 1;   // load, small const
        buf[pos] = 0x00; pos += 1;   // operand = variable number 0 (sp)
        buf[pos] = 0x10; pos += 1;   // store -> G0 (var 0x10)

        buf[pos] = 0xBA;              // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        assert_eq!(m.global(0), 0xBEEF, "load sp: G0 should be top of stack (0xBEEF)");
        assert_eq!(m.state.eval_stack.len(), 2, "load sp: stack depth must be unchanged (peek, not pop)");
        assert_eq!(m.state.eval_stack[1], 0xBEEF, "load sp: top value still on stack");
    }

    // -----------------------------------------------------------------------
    // Test: store sp replaces top without pushing (ZMSD §6.3.4)
    // -----------------------------------------------------------------------

    #[test]
    fn store_sp_replaces_top() {
        // Layout at 0x10:
        //   push 0x1234              (VAR:push, large const)
        //   store sp, 0x56           (2OP:0x0D, a=small const 0x00, b=small const 0x56)
        //     Long form small+small: 0x0D, a=0x00, b=0x56
        //     3 bytes total
        //   quit
        //
        // After store: stack depth still 1, top == 0x0056.
        let mut buf = sample_story(5);
        let mut pos = 0x10usize;

        // push 0x1234 (large const)
        buf[pos] = 0xE8; pos += 1;
        buf[pos] = 0x3F; pos += 1;
        buf[pos] = 0x12; pos += 1;
        buf[pos] = 0x34; pos += 1;

        // store sp, 0x56: 2OP:0x0D long form, both small const
        // Long-form opcode: t1=small(0), t2=small(0), opcode=0x0D → 0x0D
        buf[pos] = 0x0D; pos += 1;   // store, small+small
        buf[pos] = 0x00; pos += 1;   // a = variable number 0 (sp)
        buf[pos] = 0x56; pos += 1;   // b = new value 0x56

        buf[pos] = 0xBA;              // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        assert_eq!(m.state.eval_stack.len(), 1, "store sp: stack depth must be unchanged (replace, not push)");
        assert_eq!(m.state.eval_stack[0], 0x0056, "store sp: top value must be new value");
    }

    // -----------------------------------------------------------------------
    // Test: emit_branch two-byte form encodes correctly (ZMSD §4.7)
    // -----------------------------------------------------------------------

    #[test]
    fn emit_branch_two_byte_form() {
        // ZMSD §4.7: when |offset| >= 64, branch is two bytes.
        // Byte 0: bit7=on_true, bit6=0 (long form), bits5-0 = high 6 bits of 14-bit offset
        // Byte 1: low 8 bits of 14-bit offset
        // Test with offset=100 (>= 64), on_true=true.
        let mut out = Vec::new();
        emit_branch(&mut out, true, 100);
        assert_eq!(out.len(), 2, "long branch must emit exactly 2 bytes");

        // offset=100 = 0x0064; 14-bit raw = 0x0064
        // high6 = (0x0064 >> 8) & 0x3F = 0x00
        // low8  = 0x0064 & 0xFF        = 0x64
        // byte0 = on_true(1<<7) | high6 = 0x80 | 0x00 = 0x80
        // byte1 = 0x64
        assert_eq!(out[0], 0x80, "byte0: on_true bit set, bit6 clear, high6=0");
        assert_eq!(out[1], 0x64, "byte1: low8 of offset 100");

        // Also verify: on_true=false with offset=200 (0x00C8)
        // high6 = (0x00C8 >> 8) & 0x3F = 0x00
        // low8  = 0x00C8 & 0xFF        = 0xC8
        // byte0 = 0x00 | 0x00 = 0x00 (bit7=0, bit6=0)
        let mut out2 = Vec::new();
        emit_branch(&mut out2, false, 200);
        assert_eq!(out2[0], 0x00, "byte0: on_true=false, high6=0");
        assert_eq!(out2[1], 0xC8, "byte1: low8 of offset 200");

        // And offset=64 (boundary: just over single-byte limit)
        // high6 = 0x00, low8 = 0x40
        // byte0 = 0x80 (on_true=true)
        let mut out3 = Vec::new();
        emit_branch(&mut out3, true, 64);
        assert_eq!(out3.len(), 2, "offset=64 uses two-byte form");
        assert_eq!(out3[0], 0x80);
        assert_eq!(out3[1], 0x40);
    }

    #[test]
    fn inc_chk_and_dec_chk() {
        // inc_chk test:
        //   0x10: inc_chk 0x10, 0  (dec_chk: opcode=0x05, both small const, branch)
        //         Long-form: opcode=0x05 (inc_chk), t1=small(0), t2=small(0) → 0x05
        //         operand 1 = 0x10 (var number for G0), operand 2 = 0 (threshold)
        //         branch: on_true, offset=6 to skip 4-byte Add: 0x80|0x40|6 = 0xC6
        //         next_pc = 0x14; branch target = 0x14 + 6 - 2 = 0x18
        //   0x14: add 99, 0 → G0  (4 bytes, 0x14–0x17, skipped when branch taken)
        //   0x18: quit
        let mut buf = sample_story(5);
        // inc_chk 0x10, 0: opcode=0x05, both small
        buf[0x10] = 0x05; // inc_chk, small+small
        buf[0x11] = 0x10; // var number = global 0
        buf[0x12] = 0;    // threshold = 0
        buf[0x13] = 0xC6; // branch on_true, short form, offset=6
        // add 99, 0 → G0 (long form, small+small)
        buf[0x14] = 0x14;
        buf[0x15] = 99;
        buf[0x16] = 0;
        buf[0x17] = 0x10; // store → G0
        // quit
        buf[0x18] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 1, "inc_chk: G0 should be 1 (Add(99,0) skipped)");

        // dec_chk test:
        //   0x10: dec_chk 0x10, 0  (opcode=0x04, both small const)
        //         G0 starts at 0 → decrements to 0xFFFF (-1 signed)
        //         -1 < 0 → branch taken, offset=6 → skip Add → quit
        //   0x14: add 99, 0 → G0  (skipped)
        //   0x18: quit
        let mut buf2 = sample_story(5);
        buf2[0x10] = 0x04; // dec_chk, small+small
        buf2[0x11] = 0x10; // var number = G0
        buf2[0x12] = 0;    // threshold = 0
        buf2[0x13] = 0xC6; // branch on_true, short, offset=6
        buf2[0x14] = 0x14; // add
        buf2[0x15] = 99;
        buf2[0x16] = 0;
        buf2[0x17] = 0x10;
        buf2[0x18] = 0xBA; // quit

        let mem2 = Memory::new(buf2).unwrap();
        let mut m2 = Machine::new(mem2);
        m2.state.pc = 0x10;
        run_until_quit(&mut m2);
        assert_eq!(m2.global(0), 0xFFFF, "dec_chk: G0 should be 0xFFFF (-1 as u16)");
    }

    // -----------------------------------------------------------------------
    // Object / property opcode tests (Task 10)
    //
    // Object table layout (v3, sample_story(3)):
    //   object_table = 0x0100 (set by sample_story)
    //   v3 property-defaults: 31 words = 62 bytes → entries at 0x013E
    //   Each v3 entry = 9 bytes: [0..3] attrs, [4] parent, [5] sibling, [6] child, [7..8] prop_tbl
    //
    //   obj1 at 0x013E: parent=0, sibling=0, child=2, attr0 set, prop_tbl=0x0200
    //   obj2 at 0x0147: parent=1, sibling=3, child=0, attr7+8 set, prop_tbl=0x0220
    //   obj3 at 0x0150: parent=1, sibling=0, child=0, prop_tbl=0x0230
    //
    // Property table for obj1 (at 0x0200):
    //   name: 0 words (empty)
    //   prop 10: 2 bytes 0xABCD → size byte 0x2A, data 0xABCD
    //   prop 5:  1 byte  0x42  → size byte 0x05, data 0x42
    //   sentinel 0x00
    //
    // Property table for obj2/obj3: name 0 words, sentinel 0x00 only.
    //
    // Test programs are placed at 0x10 (pc=0x10) in v3 story buffers.
    // -----------------------------------------------------------------------

    /// Build a v3 story buffer with a small 3-object tree for executor tests.
    fn build_obj_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        const OBJ_TABLE: usize = 0x0100;
        const ENTRIES: usize   = OBJ_TABLE + 31 * 2; // 0x013E
        const OBJ1: usize      = ENTRIES;             // 0x013E
        const OBJ2: usize      = ENTRIES + 9;         // 0x0147
        const OBJ3: usize      = ENTRIES + 18;        // 0x0150

        const PROP1: u16 = 0x0200;
        const PROP2: u16 = 0x0220;
        const PROP3: u16 = 0x0230;

        // obj1: attr0 set, parent=0, sibling=0, child=2
        buf[OBJ1]   = 0x80; // attr0
        buf[OBJ1+1] = 0; buf[OBJ1+2] = 0; buf[OBJ1+3] = 0;
        buf[OBJ1+4] = 0; // parent
        buf[OBJ1+5] = 0; // sibling
        buf[OBJ1+6] = 2; // child
        buf[OBJ1+7] = (PROP1 >> 8) as u8; buf[OBJ1+8] = (PROP1 & 0xFF) as u8;

        // obj2: attr7+attr8 set, parent=1, sibling=3, child=0
        buf[OBJ2]   = 0x01; // attr7
        buf[OBJ2+1] = 0x80; // attr8
        buf[OBJ2+2] = 0; buf[OBJ2+3] = 0;
        buf[OBJ2+4] = 1; // parent
        buf[OBJ2+5] = 3; // sibling
        buf[OBJ2+6] = 0; // child
        buf[OBJ2+7] = (PROP2 >> 8) as u8; buf[OBJ2+8] = (PROP2 & 0xFF) as u8;

        // obj3: no attrs, parent=1, sibling=0, child=0
        buf[OBJ3]   = 0; buf[OBJ3+1] = 0; buf[OBJ3+2] = 0; buf[OBJ3+3] = 0;
        buf[OBJ3+4] = 1; // parent
        buf[OBJ3+5] = 0; // sibling
        buf[OBJ3+6] = 0; // child
        buf[OBJ3+7] = (PROP3 >> 8) as u8; buf[OBJ3+8] = (PROP3 & 0xFF) as u8;

        // prop table obj1: name=0 words, prop10(2B)=0xABCD, prop5(1B)=0x42, sentinel
        let p1 = PROP1 as usize;
        buf[p1]   = 0;    // 0 name words
        buf[p1+1] = 0x2A; // size: (2-1)<<5 | 10 = 0b001_01010
        buf[p1+2] = 0xAB; buf[p1+3] = 0xCD;
        buf[p1+4] = 0x05; // size: (1-1)<<5 | 5  = 0b000_00101
        buf[p1+5] = 0x42;
        buf[p1+6] = 0x00; // sentinel

        // prop table obj2: name=0 words, no props
        let p2 = PROP2 as usize;
        buf[p2] = 0; buf[p2+1] = 0x00; // sentinel

        // prop table obj3: name=0 words, no props
        let p3 = PROP3 as usize;
        buf[p3] = 0; buf[p3+1] = 0x00; // sentinel

        // property default for prop10 = 0x5678
        let def10 = OBJ_TABLE + (10 - 1) * 2;
        buf[def10]   = 0x56;
        buf[def10+1] = 0x78;

        buf
    }

    /// Build a `Machine` from a pre-built story buffer, program at 0x10.
    fn build_obj_machine_raw(buf: Vec<u8>, prog: &[u8]) -> Machine {
        let mut buf = buf;
        for (i, &b) in prog.iter().enumerate() {
            buf[0x10 + i] = b;
        }
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m
    }

    // -----------------------------------------------------------------------
    // (a) jin — branch if parent(obj1)==obj2
    // test_attr — branch if attribute set
    // -----------------------------------------------------------------------

    #[test]
    fn obj_jin_branch_taken() {
        // jin obj2, obj1 → branch taken (parent(2)==1)
        // Long-form 2OP 0x06, both small const: opcode byte = 0x06
        // branch taken → skip 1 byte nop, reaching quit
        // Layout: [jin obj2,obj1 + branch(taken, offset=3)][nop][add 99,0→G0][quit]
        //
        // jin (3 + 1 branch byte = 4 bytes, next_pc=0x14)
        // branch: on_true=1, offset=5 → skip 4-byte add, land on quit
        // nop is 1 byte, add is 4 bytes: to skip both → offset = 5 + 2 = 7?
        // Actually place nop+add at 0x14..0x18, quit at 0x19.
        // From next_pc=0x14: to skip to quit at 0x19 → offset = 0x19-0x14+2 = 7.
        // branch byte: on_true, short, offset=7 → 0x80|0x40|7 = 0xC7
        //
        // After branch skips to quit without running add, G0 stays 0.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x06, 2, 1, 0xC7, // jin obj2,obj1 → branch on_true, offset=7 → skip to quit
            0xB4,              // nop (1 byte)
            0x14, 99, 0, 0x10, // add 99,0 → G0 (4 bytes, skipped)
            0xBA,              // quit
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0, "jin taken: add skipped, G0 remains 0");
    }

    #[test]
    fn obj_jin_branch_not_taken() {
        // jin obj1, obj2 → NOT taken (parent(1)==0, not 2)
        // G0 gets set to 99.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x06, 1, 2, 0xC7, // jin obj1,obj2 → branch on_true, offset=7 (but not taken)
            0xB4,              // nop
            0x14, 99, 0, 0x10, // add 99,0 → G0
            0xBA,              // quit
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 99, "jin not taken: falls through, G0=99");
    }

    #[test]
    fn obj_test_attr_branch_taken() {
        // test_attr obj1, attr0 → taken (attr0 is set on obj1)
        // Long-form 2OP 0x0A, both small: opcode byte = 0x0A
        // branch taken → skip 4-byte add, G0 stays 0
        // next_pc=0x14 after [0x0A,1,0,branch_byte]
        // to skip 4-byte add at 0x14 and land at 0x18 (quit): offset=0x18-0x14+2=6
        // branch: on_true=1, short, offset=6 → 0xC6
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0A, 1, 0, 0xC6, // test_attr obj1,attr0 → branch taken, offset=6
            0x14, 99, 0, 0x10, // add 99,0 → G0 (skipped)
            0xBA,              // quit
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0, "test_attr taken: add skipped");
    }

    #[test]
    fn obj_test_attr_branch_not_taken() {
        // test_attr obj1, attr1 → NOT taken (attr1 is clear)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0A, 1, 1, 0xC6, // test_attr obj1,attr1 → not taken
            0x14, 99, 0, 0x10, // add 99,0 → G0 (runs)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 99, "test_attr not taken: G0=99");
    }

    // -----------------------------------------------------------------------
    // (b) set_attr / clear_attr — verify via get_attr after step()
    // -----------------------------------------------------------------------

    #[test]
    fn obj_set_attr_and_clear_attr() {
        // set_attr obj1, attr3 → then clear_attr obj1, attr3
        // Long-form 2OP: set_attr=0x0B, clear_attr=0x0C
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0B, 1, 3, // set_attr obj1, attr3 (3 bytes)
            0xBA,        // quit
        ];
        let mut m = build_obj_machine_raw(buf.clone(), prog);
        run_until_quit(&mut m);
        assert!(objects::get_attr(&m.mem, 1, 3), "attr3 should be set after set_attr");
        assert!(objects::get_attr(&m.mem, 1, 0), "attr0 still set");

        // Now clear it
        let prog2: &[u8] = &[
            0x0B, 1, 3, // set_attr obj1, attr3
            0x0C, 1, 3, // clear_attr obj1, attr3
            0xBA,
        ];
        let mut m2 = build_obj_machine_raw(buf, prog2);
        run_until_quit(&mut m2);
        assert!(!objects::get_attr(&m2.mem, 1, 3), "attr3 should be clear after clear_attr");
        assert!(objects::get_attr(&m2.mem, 1, 0), "attr0 still set");
    }

    // -----------------------------------------------------------------------
    // (c) insert_obj → get_parent / get_child reflect the change
    // -----------------------------------------------------------------------

    #[test]
    fn obj_insert_obj_updates_tree() {
        // insert_obj obj2, obj3 (move obj2 to be child of obj3)
        // Long-form 2OP: insert_obj=0x0E
        // After: parent(obj2)==3, child(obj3)==2
        // Verify by reading tree directly after running.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0E, 2, 3, // insert_obj obj2, obj3
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(objects::get_parent(&m.mem, 2), 3, "obj2 parent should be 3");
        assert_eq!(objects::get_child(&m.mem, 3), 2, "obj3 child should be 2");
    }

    // -----------------------------------------------------------------------
    // (d) get_parent — store parent, no branch
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_parent_stores() {
        // get_parent obj2 → G0 (parent of 2 is 1)
        // Short form 1OP: 0x93, operand=2 (small const), store byte = G0 (0x10)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x93, 2, 0x10, // get_parent obj2, store → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 1, "get_parent(obj2) should be 1");
    }

    // -----------------------------------------------------------------------
    // (e) get_sibling — store AND branch on result != 0
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_sibling_stores_and_branches() {
        // get_sibling obj2 → G0 (sibling of 2 is 3); branch taken (3 != 0)
        // Short form 1OP: 0x91, operand=2, store=G0(0x10), branch data
        //
        // Instruction: 0x91, op=2, store=0x10, branch: on_true, skip 4-byte add
        // next_pc = 0x10 + 1+1+1+1_branch = 0x15 (5 bytes: opcode+op+store+1_branch_byte)
        // branch short, on_true=1, offset=6 → skip 4-byte add at 0x15 → land at 0x19
        // branch byte: 0x80|0x40|6 = 0xC6
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x91, 2, 0x10, 0xC6, // get_sibling obj2, store→G0, branch on_true offset=6
            0x14, 0, 0, 0x10,    // add 0,0 → G0 (would set G0=0, skipped)
            0xBA,                 // quit
        ];
        let mut m = build_obj_machine_raw(buf.clone(), prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 3, "get_sibling(2) should store 3");

        // get_sibling obj3 → G0 (sibling of 3 is 0); branch NOT taken
        // Not taken means the add runs, overwriting G0 with 99.
        let prog2: &[u8] = &[
            0x91, 3, 0x10, 0xC6, // get_sibling obj3, store→G0, branch on_true offset=6
            0x14, 99, 0, 0x10,   // add 99,0 → G0 (runs because branch not taken)
            0xBA,
        ];
        let mut m2 = build_obj_machine_raw(buf, prog2);
        run_until_quit(&mut m2);
        // G0 was set to 0 (sibling=0), then overwritten to 99 by add
        assert_eq!(m2.global(0), 99, "get_sibling(3) not taken: add runs, G0=99");
    }

    // -----------------------------------------------------------------------
    // (f) get_child — store AND branch on result != 0
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_child_stores_and_branches() {
        // get_child obj1 → G0 = 2, branch taken
        // Short form 1OP: 0x92, op=1, store=0x10, branch: on_true, offset=6
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x92, 1, 0x10, 0xC6, // get_child obj1 → G0, branch taken (child=2 ≠ 0)
            0x14, 0, 0, 0x10,    // add (skipped)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 2, "get_child(obj1) should store 2 and branch");
    }

    // -----------------------------------------------------------------------
    // (g) get_prop — stores the property value, fallback to default
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_prop_stores_value() {
        // get_prop obj1, prop10 → G0 = 0xABCD
        // Long-form 2OP 0x11, both small const, + store byte
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x11, 1, 10, 0x10, // get_prop obj1,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        // Note: prop10 has 2 bytes: value = 0xABCD; low byte only fits in G0? No, G0 is u16 = 0xABCD.
        assert_eq!(m.global(0), 0xABCD, "get_prop(obj1,10) should be 0xABCD");
    }

    #[test]
    fn obj_get_prop_defaults_fallback() {
        // get_prop obj2, prop10 → G0 = 0x5678 (default, obj2 has no props)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x11, 2, 10, 0x10, // get_prop obj2,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0x5678, "get_prop fallback to default");
    }

    // -----------------------------------------------------------------------
    // (h) get_prop_addr — store the address of property data
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_prop_addr_stores() {
        // get_prop_addr obj1, prop10 → G0 = non-zero address
        // Long-form 2OP 0x12
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x12, 1, 10, 0x10, // get_prop_addr obj1,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        let addr = m.global(0);
        assert_ne!(addr, 0, "get_prop_addr should be non-zero");
        // The data at that address should be 0xAB (high byte of 0xABCD)
        assert_eq!(m.mem.read_byte(addr as u32), 0xAB, "prop data at addr");
    }

    // -----------------------------------------------------------------------
    // (i) get_next_prop — iterate properties
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_next_prop_iterates() {
        // get_next_prop obj1, prop=0 → G0 = first prop (10)
        // Long-form 2OP 0x13
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x13, 1, 0, 0x10, // get_next_prop obj1,0 → G0 (first prop = 10)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 10, "first prop should be 10");
    }

    // -----------------------------------------------------------------------
    // (j) put_prop then get_prop — round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn obj_put_prop_round_trip() {
        // VAR:put_prop obj1, prop10, 0x1234 → then get_prop obj1,10 → G0
        // put_prop: VAR form 0x03 → opcode byte 0b11_1_00011 = 0xE3
        //   type byte: all small const (0b01_01_01_11) = 0x57
        //   operands: obj=1, prop=10, val_hi=0x12, val_lo=0x34
        // Wait: put_prop takes 3 operands and val is u16. Since small const max is 255,
        // can't encode 0x1234 as small. Use large const for val → need Var form.
        // Alternative: use a smaller value that fits in u8 for simplicity: val=0xAA.
        //   type byte: obj=small(01), prop=small(01), val=small(01), omit(11) → 0b01_01_01_11 = 0x57
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0xE3, 0x57, 1, 10, 0xAA, // put_prop obj1,prop10,0xAA (1-byte val, but prop is 2 bytes)
            // Actually put_prop on a 2-byte property writes 2 bytes; 0xAA goes in low byte.
            // Actually the put_prop implementation: len=2 → write_word → writes 0x00AA.
            // Let's just check with get_prop that the value is 0x00AA.
            0x11, 1, 10, 0x10, // get_prop obj1,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0x00AA, "put_prop/get_prop round-trip: 0x00AA");
    }

    // -----------------------------------------------------------------------
    // (k) get_prop_len — store length of property data
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_prop_len_stores() {
        // First get_prop_addr for obj1 prop10 → G0 (addr)
        // Then get_prop_len G0 → G1 (should be 2)
        // Short form 1OP get_prop_len: 0x94, operand = G0 (var 0x10), store = G1 (0x11)
        //   Short form with variable operand: 0b10_10_0100 = 0xA4
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x12, 1, 10, 0x10, // get_prop_addr obj1,prop10 → G0 (4 bytes)
            0xA4, 0x10, 0x11,  // get_prop_len G0 → G1 (3 bytes: 0xA4=short/var, var_num, store)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        let len = m.mem.read_word(m.mem.global_vars() as u32 + 1 * 2);
        assert_eq!(len, 2, "get_prop_len for prop10 (2 bytes) should be 2");
    }

    // -----------------------------------------------------------------------
    // (l) remove_obj — unlinks object from parent
    // -----------------------------------------------------------------------

    #[test]
    fn obj_remove_obj_unlinks() {
        // remove_obj obj2 → obj2's parent becomes 0
        // Short form 1OP: 0x99, op=2 (small const)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x99, 2, // remove_obj obj2
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(objects::get_parent(&m.mem, 2), 0, "obj2 parent should be 0 after remove_obj");
        assert_eq!(objects::get_child(&m.mem, 1), 3, "obj1 child should now be 3 (obj2 removed)");
    }

    // -----------------------------------------------------------------------
    // (m) print_obj — writes short name to the output sink
    // -----------------------------------------------------------------------

    #[test]
    fn obj_print_obj_writes_to_output() {
        // print_obj needs an object. Use build_obj_story() which has objects with
        // zero name words (empty name). The important thing is the opcode is wired up
        // and routes to self.out rather than the removed out_buf.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x9A, 1, // print_obj obj1
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        // short_name of obj1 with 0 name words returns "" — output was called (just empty).
        // We verify no panic and that the sink is accessible.
        let out = m.buffer_output().expect("default sink is BufferOutput");
        // empty name string was printed — buf is "" (valid)
        let _ = &out.buf;
    }

    // -----------------------------------------------------------------------
    // Task 11: text output opcode tests
    // -----------------------------------------------------------------------

    // Helper: build a test machine from raw bytes placed at 0x10.
    fn build_raw_machine(buf: Vec<u8>) -> Machine {
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m
    }

    /// Test: `print` (inline) + `new_line` + `print_num -7` → sink receives "Hello\n-7".
    ///
    /// Z-encoding "Hello" (A0/A1 alphabets, 3 words):
    ///   H: shift-A1 (Z4) + Z13 (A1[7]='H', index = zchar-6 = 13-6 = 7)
    ///   e: Z10 (A0: a=Z6,b=Z7,...,e=Z10)
    ///   l: Z17, l: Z17, o: Z20; pad Z5,Z5 to fill word 2.
    ///   word0: Z4,Z13,Z10  = (4<<10)|(13<<5)|10
    ///   word1: Z17,Z17,Z20 = (17<<10)|(17<<5)|20
    ///   word2 (last): Z5,Z5,Z5 = 0x8000|(5<<10)|(5<<5)|5
    ///
    /// print_num -7: use Large constant 0xFFF9 (small constants are 0-255, no sign).
    #[test]
    fn text_print_newline_print_num() {
        let mut buf = sample_story(5);

        let w0: u16 = (4u16 << 10) | (13u16 << 5) | 10u16;
        let w1: u16 = (17u16 << 10) | (17u16 << 5) | 20u16;
        let w2: u16 = 0x8000 | (5u16 << 10) | (5u16 << 5) | 5u16;

        buf[0x10] = 0xB2;  // 0OP print
        buf[0x11] = (w0 >> 8) as u8; buf[0x12] = (w0 & 0xFF) as u8;
        buf[0x13] = (w1 >> 8) as u8; buf[0x14] = (w1 & 0xFF) as u8;
        buf[0x15] = (w2 >> 8) as u8; buf[0x16] = (w2 & 0xFF) as u8;

        buf[0x17] = 0xBB;  // 0OP new_line

        // VAR:0x06 print_num with Large const -7 (0xFFF9)
        buf[0x18] = 0xE6;
        buf[0x19] = 0x3F;  // type: large first, rest omit
        buf[0x1A] = 0xFF;
        buf[0x1B] = 0xF9;

        buf[0x1C] = 0xBA;  // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "Hello\n-7");
    }

    /// Test: `print_char` for ZSCII 65 ('A') prints 'A'.
    #[test]
    fn text_print_char_known_zscii() {
        let mut buf = sample_story(5);
        // VAR:0x05 print_char, operand=65 (ZSCII 'A')
        // 0xE5 = 0b11_1_00101 (VAR form, opcode 5)
        // type byte: first=small const(01), rest=omit → 0x7F
        buf[0x10] = 0xE5;
        buf[0x11] = 0x7F;
        buf[0x12] = 65u8; // 'A'
        buf[0x13] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "A");
    }

    /// Test: `print_addr` decodes a string at a byte address and prints it.
    #[test]
    fn text_print_addr_decodes_string() {
        let mut buf = sample_story(5);
        // Z-encode "abc": a=Z6, b=Z7, c=Z8
        // word = 0x8000|(6<<10)|(7<<5)|8 = 0x8000|0x1800|0x00E0|0x08 = 0x98E8
        let abc_word: u16 = 0x8000 | (6u16 << 10) | (7u16 << 5) | 8u16;
        buf[0x0200] = (abc_word >> 8) as u8;
        buf[0x0201] = (abc_word & 0xFF) as u8;

        // 1OP:0x07 print_addr, Large operand 0x0200
        // Short form 1OP large const: 0b10_00_0111 = 0x87
        buf[0x10] = 0x87;
        buf[0x11] = 0x02;
        buf[0x12] = 0x00;
        buf[0x13] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "abc");
    }

    /// Test: `print_paddr` unpacks a packed address and prints the string.
    #[test]
    fn text_print_paddr_decodes_string() {
        let mut buf = sample_story(5);
        // v5: unpack_string(packed) = packed * 4. Use packed=0x0050 → byte 0x0140.
        // sample_story(5) is 1024 bytes (0x400); 0x0140 is within bounds.
        // Z-encode "de": d=Z9, e=Z10, pad=Z5
        // word = 0x8000|(9<<10)|(10<<5)|5
        let de_word: u16 = 0x8000 | (9u16 << 10) | (10u16 << 5) | 5u16;
        buf[0x0140] = (de_word >> 8) as u8;
        buf[0x0141] = (de_word & 0xFF) as u8;

        // 1OP:0x0D print_paddr, Large operand 0x0050
        // Short form 1OP large const: 0b10_00_1101 = 0x8D
        buf[0x10] = 0x8D;
        buf[0x11] = 0x00;
        buf[0x12] = 0x50;
        buf[0x13] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "de");
    }

    /// Test: `print_ret` prints inline string + newline + returns true (store var gets 1).
    ///
    /// To test print_ret properly we need a routine that calls another routine
    /// containing print_ret. print_ret returns 1 to the caller; the caller stores
    /// the return value in G0. We verify G0=1 and output ends with "\n".
    #[test]
    fn text_print_ret_returns_true() {
        // Routine at 0x80 (v5 packed addr 0x80/4 = 0x20):
        //   0x80: local_count=0
        //   0x81: print_ret "hi"
        //     0xB3 (0OP opcode 0x03 = print_ret)
        //     Z-encode "hi": h=A1-idx(2)=Z4+Z8+Z13... actually h in A1: A1="ABCDEFGHIJKLMNOPQRSTUVWXYZ^0123456789._,!?_#'"
        //     Let me use a simple 3-char encodable string instead: use Z-char padding.
        //     Simpler: Z-chars for "hi": shift-A1(4), h(A1-idx=7=Z13)... actually A1 index 7 = 'H'.
        //     Even simpler: use 3 pad Z-chars (all 5=shift) → empty string output, but test structure.
        //     Let me use "ab": a(A0,Z6), b(A0,Z7), pad(Z5) → word = 0x8000|(6<<10)|(7<<5)|5 = 0x99C5
        let mut buf = sample_story(5);

        // Routine at 0x80
        buf[0x80] = 0; // local count
        // print_ret: 0xB3 + inline text "ab" (Z-chars 6,7,5)
        buf[0x81] = 0xB3;
        let ab_word: u16 = 0x8000 | (6u16 << 10) | (7u16 << 5) | 5u16;
        buf[0x82] = (ab_word >> 8) as u8;
        buf[0x83] = (ab_word & 0xFF) as u8;
        // No explicit quit needed — print_ret returns to caller

        // Main at 0x10: call_vs packed=0x0020 → G0, then quit
        buf[0x10] = 0xE0;
        buf[0x11] = 0b00_11_11_11; // type: large, omit, omit, omit
        buf[0x12] = 0x00;
        buf[0x13] = 0x20;
        buf[0x14] = 0x10; // store → G0
        buf[0x15] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);

        assert_eq!(m.global(0), 1, "print_ret should return true (1) to caller");
        let out = m.buffer_output().expect("default sink");
        assert!(out.buf.ends_with('\n'), "print_ret output must end with newline");
        assert!(out.buf.starts_with("ab"), "print_ret output starts with the inline string");
    }
}
