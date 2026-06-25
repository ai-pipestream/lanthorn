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
use crate::dictionary;
use crate::io::{BufferOutput, Output};
use crate::memory::Memory;
use crate::objects;
use crate::screen::{init_header_caps, ScreenState, StreamState};
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

/// State saved while waiting for player input (`read` / `read_char`).
///
/// When `step()` returns `NeedLine` or `NeedChar` the machine suspends; the
/// host calls `supply_line` / `supply_char` with the input, which uses these
/// fields to complete the operation (write buffers, store result) before the
/// next `step()` call.
struct PendingInput {
    /// Destination variable for the result (store var of the read/read_char
    /// instruction; `None` if the instruction has no store — v3 `read` has none).
    store_var: Option<u8>,
    /// Address of the text buffer (for `supply_line`).
    text_buf: u32,
    /// Address of the parse buffer (for `supply_line`; 0 in v5+ means skip).
    parse_buf: u32,
}

/// The Z-machine interpreter — ties memory and CPU state together.
/// Fields are `pub` so Tasks 11+ can attach I/O channels.
pub struct Machine {
    pub mem: Memory,
    pub state: State,
    /// Pluggable text output sink. Defaults to `BufferOutput` (Task 11).
    pub out: Box<dyn Output>,
    /// Non-None while the machine is suspended waiting for player input.
    pending_input: Option<PendingInput>,
    /// Screen model: window layout, cursor, text style.
    pub screen: ScreenState,
    /// Output stream routing: streams 1/2/3/4 state.
    pub streams: StreamState,
    /// Snapshot of the original dynamic memory (bytes 0..static_mem_base) taken
    /// at construction time.  Used by Quetzal CMem encoding (XOR diff).
    /// Story files are small (< 256 KB) so the memory cost is acceptable.
    pub original_dynamic: Vec<u8>,
    /// Pending save-request context: branch/store info from the save opcode so
    /// complete_save() can deliver the version-appropriate result.
    pending_save: Option<PendingSave>,
    /// Store variable captured from the restore opcode (v4+), used by
    /// complete_restore_failure() to store 0 into the correct variable.
    pending_restore_store: Option<u8>,
    /// PRNG state for the `random` opcode (xorshift32).
    /// Initialised to a fixed nonzero constant; seeded by `random` with negative arg.
    rng_state: u32,
    /// VAR opcodes that have hit the unimplemented fallthrough (warned once each).
    pub(crate) warned_var_opcodes: std::collections::HashSet<u8>,
}

/// Context captured when the `save` opcode fires, needed by `complete_save`.
struct PendingSave {
    /// v3: the branch descriptor; v4+: the store variable number.
    result_dest: SaveDest,
}

enum SaveDest {
    Branch(crate::cpu::decode::Branch),
    Store(u8),
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
        // Capture original dynamic memory for Quetzal CMem XOR diff.
        let dyn_len = mem.static_mem_base() as usize;
        let original_dynamic = mem.raw_bytes()[..dyn_len].to_vec();
        Machine {
            state: State::new(initial_pc),
            mem,
            out,
            pending_input: None,
            screen: ScreenState::default(),
            streams: StreamState::new(),
            original_dynamic,
            pending_save: None,
            pending_restore_store: None,
            rng_state: 0x12345678, // fixed nonzero seed
            warned_var_opcodes: std::collections::HashSet::new(),
        }
    }

    /// Set interpreter capability bits in the story header (ZMSD §11.1).
    ///
    /// Call this once after loading a real story file and before running.
    /// Not called automatically by `new`/`with_output` because the writes
    /// overlap with address 0x10 (Flags2) which test programs occupy at that
    /// same address — real story files have static programs above 0x40.
    ///
    /// # Caller
    /// Call this from the host after loading a real story file, before the first
    /// `step()`. Not needed for test harnesses built from `sample_story` (whose
    /// buffers may overlap header bytes).
    pub fn init_caps(&mut self) {
        init_header_caps(&mut self.mem);
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
        let a = ops.first().copied().unwrap_or(0);
        let b = ops.get(1).copied().unwrap_or(0);

        match opcode {
            // 0x01 je — branch if a equals ANY of ops[1..]
            // Variable form allows up to 4 operands (ZMSD §14).
            0x01 => {
                let cond = ops.len() > 1 && ops[1..].contains(&a);
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
            // 0x0F loadw — load word from array: result = mem[a + 2*b]
            0x0F => {
                let addr = (a as u32).wrapping_add(2u32.wrapping_mul(b as u32));
                let result = self.mem.read_word(addr);
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x10 loadb — load byte from array: result = mem[a + b]
            0x10 => {
                let addr = (a as u32).wrapping_add(b as u32);
                let result = self.mem.read_byte(addr) as u16;
                self.do_store(store, result);
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
        let a = ops.first().copied().unwrap_or(0);

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
                self.print_text(&s);
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
                self.print_text(&name);
                StepResult::Continue
            }
            // 0x0D print_paddr — print string at packed address a
            0x0D => {
                let byte_addr = self.mem.unpack_string(a);
                let (s, _) = decode_string(&self.mem, byte_addr);
                self.print_text(&s);
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
                    self.print_text(&s);
                }
                StepResult::Continue
            }
            // 0x03 print_ret — print inline string + newline, then return true
            0x03 => {
                if let Some((s, _)) = text {
                    self.print_text(&s);
                }
                self.print_text("\n");
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
                self.print_text("\n");
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
            // 0x05 save — suspend and let the host serialise state (Task 14).
            // v3: branch on success; v4+: store result (1=ok, 0=fail).
            // PC has already advanced past the instruction (standard step() contract),
            // so state.pc at this point is the correct resume address.
            0x05 => {
                let dest = if self.mem.version() <= 3 {
                    // v3: save is a branch instruction; branch is present
                    match branch {
                        Some(b) => SaveDest::Branch(b),
                        None => SaveDest::Store(0), // shouldn't happen; safe fallback
                    }
                } else {
                    // v4+: save is a store instruction; store is present
                    match store {
                        Some(sv) => SaveDest::Store(sv),
                        None => SaveDest::Store(0),
                    }
                };
                self.pending_save = Some(PendingSave { result_dest: dest });
                StepResult::SaveRequest
            }
            // 0x06 restore — suspend and let the host supply bytes (Task 14).
            // v3: branch on success; v4+: store result (2 = restored from save,
            // 0 = failure). The store byte is decoded by the decoder and passed
            // here; capture it so complete_restore_failure() can use it without
            // reading from state.pc (which has already advanced past the store byte).
            0x06 => {
                if self.mem.version() >= 4 {
                    self.pending_restore_store = store;
                }
                StepResult::RestoreRequest
            }
            // 0x0C show_status (v3 only) — signal host to redraw the status line
            0x0C => {
                self.screen.show_status_requested = true;
                StepResult::Continue
            }
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
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, store);
                StepResult::Continue
            }
            // 0x01 storew — store word: mem[ops[0] + 2*ops[1]] = ops[2]
            0x01 => {
                let array = ops.first().copied().unwrap_or(0) as u32;
                let index = ops.get(1).copied().unwrap_or(0) as u32;
                let val   = ops.get(2).copied().unwrap_or(0);
                self.mem.write_word(array.wrapping_add(2u32.wrapping_mul(index)), val);
                StepResult::Continue
            }
            // 0x02 storeb — store byte: mem[ops[0] + ops[1]] = ops[2] & 0xFF
            0x02 => {
                let array = ops.first().copied().unwrap_or(0) as u32;
                let index = ops.get(1).copied().unwrap_or(0) as u32;
                let val   = (ops.get(2).copied().unwrap_or(0) & 0xFF) as u8;
                self.mem.write_byte(array.wrapping_add(index), val);
                StepResult::Continue
            }
            // 0x03 put_prop — set property ops[1] of object ops[0] to value ops[2]
            0x03 => {
                let obj  = ops.first().copied().unwrap_or(0);
                let prop = ops.get(1).copied().unwrap_or(0) as u8;
                let val  = ops.get(2).copied().unwrap_or(0);
                objects::put_prop(&mut self.mem, obj, prop, val);
                StepResult::Continue
            }
            // 0x05 print_char — print a single ZSCII character
            0x05 => {
                let zscii = ops.first().copied().unwrap_or(0);
                let ch = zscii_to_char(zscii);
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                self.print_text(s);
                StepResult::Continue
            }
            // 0x06 print_num — print operand as signed decimal
            0x06 => {
                let val = ops.first().copied().unwrap_or(0) as i16;
                let s = format!("{}", val);
                self.print_text(&s);
                StepResult::Continue
            }
            // 0x07 random — ZMSD §15: random number generator
            //   range > 0 → uniform random in 1..=range
            //   range == 0 → reseed from entropy (we use a fixed step; return 0)
            //   range < 0 → seed with |range| (predictable mode); return 0
            0x07 => {
                let range = ops.first().copied().unwrap_or(0) as i16;
                let result = if range > 0 {
                    // xorshift32 step
                    let mut s = self.rng_state;
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    self.rng_state = s;
                    // Map to 1..=range
                    (s % (range as u32) + 1) as u16
                } else if range < 0 {
                    // Predictable seed: use |range| as the new state (nonzero guard)
                    let seed = (-range) as u32;
                    self.rng_state = if seed == 0 { 1 } else { seed };
                    0
                } else {
                    // range == 0: re-randomise (use a fixed increment so no OS calls)
                    self.rng_state = self.rng_state.wrapping_add(0x9E3779B9);
                    if self.rng_state == 0 { self.rng_state = 1; }
                    0
                };
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x08 push — push value onto eval stack
            0x08 => {
                let val = ops.first().copied().unwrap_or(0);
                write_var(&mut self.state, &mut self.mem, 0, val); // var 0 = push
                StepResult::Continue
            }
            // 0x09 pull — pop from eval stack and store into variable ops[0].
            // ZMSD §14 / frotz semantics: when destination var == 0 (sp),
            // pop the top value, then OVERWRITE the new top with that value
            // (rather than pushing it back). This is the "pull to sp" effect:
            // stack [a, b, TOP] → pop TOP → stack [a, b], then overwrite b
            // → stack [a, TOP]. Net: removes the second-from-top element.
            0x09 => {
                let var = ops.first().copied().unwrap_or(0) as u8;
                let val = read_var(&mut self.state, &self.mem, 0); // pop stack
                if var == 0 {
                    // Destination is sp: overwrite new top (not push-back)
                    poke_stack(&mut self.state, val);
                } else {
                    write_var(&mut self.state, &mut self.mem, var, val);
                }
                StepResult::Continue
            }
            // 0x0C call_vs2 — like call_vs but with 2 type bytes, stores result
            0x0C => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, store);
                StepResult::Continue
            }
            // 0x04 sread/aread/read — pause execution and wait for a line of input.
            // v3: no store var. v4+: has a store var (terminating character).
            // Operands: text_buf, parse_buf (+ optional time/routine in v4+ — ignored).
            0x04 => {
                let text_buf = ops.first().copied().unwrap_or(0) as u32;
                let parse_buf = ops.get(1).copied().unwrap_or(0) as u32;
                self.pending_input = Some(PendingInput { store_var: store, text_buf, parse_buf });
                StepResult::NeedLine { text_buf, parse_buf }
            }
            // 0x16 read_char — pause execution and wait for a single keypress (v4+).
            // Has a store var for the ZSCII code.
            0x16 => {
                self.pending_input = Some(PendingInput { store_var: store, text_buf: 0, parse_buf: 0 });
                StepResult::NeedChar
            }
            // 0x18 not (VAR form, v5+) — bitwise complement
            0x18 => {
                let val = ops.first().copied().unwrap_or(0);
                self.do_store(store, !val);
                StepResult::Continue
            }
            // 0x19 call_vn — call with up to 3 args, discard result (v5+)
            0x19 => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, None);
                StepResult::Continue
            }
            // 0x1A call_vn2 — like call_vn but with 2 type bytes
            0x1A => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, None);
                StepResult::Continue
            }
            // 0x1F check_arg_count (v5+) — branch if arg_count >= ops[0]
            0x1F => {
                let n = ops.first().copied().unwrap_or(0);
                let arg_count = self.state.frames.last().map(|f| f.arg_count as u16).unwrap_or(0);
                self.do_branch(branch, arg_count >= n);
                StepResult::Continue
            }
            // ── Screen / stream opcodes (Task 13) ────────────────────────────

            // 0x0A split_window — set upper window to N rows (v3+)
            0x0A => {
                let rows = ops.first().copied().unwrap_or(0);
                self.screen.upper_window_rows = rows;
                StepResult::Continue
            }
            // 0x0B set_window — select window 0 (lower) or 1 (upper) (v3+)
            0x0B => {
                let win = ops.first().copied().unwrap_or(0) as u8;
                self.screen.current_window = win;
                StepResult::Continue
            }
            // 0x0D erase_window — clear window (state-tracking only; no render)
            0x0D => {
                // Erase window: -1 = all windows + unsplit, -2 = all without unsplit,
                // 0 = lower, 1 = upper. We just update upper_window_rows if -1.
                let win = ops.first().copied().unwrap_or(0) as i16;
                if win == -1 {
                    self.screen.upper_window_rows = 0;
                }
                StepResult::Continue
            }
            // 0x0F set_cursor — update cursor position (row, col) in upper window
            0x0F => {
                let row = ops.first().copied().unwrap_or(1);
                let col = ops.get(1).copied().unwrap_or(1);
                self.screen.cursor_row = row;
                self.screen.cursor_col = col;
                StepResult::Continue
            }
            // 0x11 set_text_style — update text style bitmask (v4+)
            0x11 => {
                let style = ops.first().copied().unwrap_or(0) as u8;
                self.screen.text_style = style;
                StepResult::Continue
            }
            // 0x12 buffer_mode — toggle output buffering (v4+)
            0x12 => {
                let mode = ops.first().copied().unwrap_or(0);
                self.screen.buffer_mode = mode != 0;
                StepResult::Continue
            }
            // 0x13 output_stream — select/deselect output streams (ZMSD §7.1.2.5)
            //   +1/-1: stream 1 (screen) on/off
            //   +2/-2: stream 2 (transcript) on/off
            //   +3:    stream 3 on — second operand is table address
            //   -3:    stream 3 off — finalise table, restore routing
            //   +4/-4: stream 4 (commands) on/off
            0x13 => {
                let stream = ops.first().copied().unwrap_or(0) as i16;
                match stream {
                    1  => { self.streams.stream1 = true; }
                    -1 => { self.streams.stream1 = false; }
                    2  => { self.streams.stream2 = true; }
                    -2 => { self.streams.stream2 = false; }
                    3  => {
                        let table = ops.get(1).copied().unwrap_or(0) as u32;
                        self.streams.push_stream3(table);
                    }
                    -3 => {
                        self.streams.pop_stream3(&mut self.mem);
                    }
                    4  => { self.streams.stream4 = true; }
                    -4 => { self.streams.stream4 = false; }
                    _  => {}
                }
                StepResult::Continue
            }
            // VAR:0x10 get_cursor — write (row, col) of the upper-window cursor into a 2-word array.
            0x10 => {
                let array = ops.first().copied().unwrap_or(0) as u32;
                self.mem.write_word(array, self.screen.cursor_row);
                self.mem.write_word(array + 2, self.screen.cursor_col);
                StepResult::Continue
            }
            // VAR:0x17 scan_table — search a table for x; store match address (0 if none), branch if found.
            0x17 => {
                let x = ops.first().copied().unwrap_or(0);
                let table = ops.get(1).copied().unwrap_or(0) as u32;
                let len = ops.get(2).copied().unwrap_or(0);
                let form = ops.get(3).copied().unwrap_or(0x82);
                let is_word = form & 0x80 != 0;
                let step = ((form & 0x7F) as u32).max(1);
                let mut found: u16 = 0;
                for i in 0..len as u32 {
                    let addr = table + i * step;
                    let val = if is_word { self.mem.read_word(addr) } else { self.mem.read_byte(addr) as u16 };
                    let target = if is_word { x } else { x & 0xFF };
                    if val == target {
                        found = addr as u16;
                        break;
                    }
                }
                self.do_store(store, found);
                self.do_branch(branch, found != 0);
                StepResult::Continue
            }
            // VAR:0x1D copy_table — copy/zero a memory region (ZMSD §15).
            0x1D => {
                let first = ops.first().copied().unwrap_or(0) as u32;
                let second = ops.get(1).copied().unwrap_or(0) as u32;
                let size = ops.get(2).copied().unwrap_or(0) as i16;
                if second == 0 {
                    for i in 0..size.unsigned_abs() as u32 {
                        self.mem.write_byte(first + i, 0);
                    }
                } else if size < 0 {
                    // forced forward copy; overlap corruption is intentional
                    let n = size.unsigned_abs() as u32;
                    for i in 0..n {
                        let b = self.mem.read_byte(first + i);
                        self.mem.write_byte(second + i, b);
                    }
                } else {
                    // positive: copy avoiding corruption — snapshot the source first
                    let n = size as u32;
                    let src: Vec<u8> = (0..n).map(|i| self.mem.read_byte(first + i)).collect();
                    for (i, &b) in src.iter().enumerate() {
                        self.mem.write_byte(second + i as u32, b);
                    }
                }
                StepResult::Continue
            }
            // VAR:0x1E print_table — print a rectangle of ZSCII text from the current cursor (ZMSD §15).
            0x1E => {
                let mut addr = ops.first().copied().unwrap_or(0) as u32;
                let width = ops.get(1).copied().unwrap_or(0);
                let height = ops.get(2).copied().unwrap_or(1).max(1);
                let skip = ops.get(3).copied().unwrap_or(0) as u32;
                let start_col = self.screen.cursor_col;
                let start_row = self.screen.cursor_row;
                for row in 0..height {
                    // Position each row at the starting column, one line down (correct once the grid exists).
                    self.screen.cursor_row = start_row + row;
                    self.screen.cursor_col = start_col;
                    for _ in 0..width {
                        let ch = zscii_to_char(self.mem.read_byte(addr) as u16);
                        let mut buf = [0u8; 4];
                        self.print_text(ch.encode_utf8(&mut buf));
                        addr += 1;
                    }
                    addr += skip;
                }
                StepResult::Continue
            }
            // Unknown / unimplemented VAR opcode: warn once, then ignore.
            _ => {
                if self.warned_var_opcodes.insert(opcode) {
                    eprintln!("zvm: warning: unimplemented VAR opcode 0x{opcode:02X} (ignored)");
                }
                StepResult::Continue
            }
        }
    }

    // -----------------------------------------------------------------------
    // EXT opcodes (v5+)
    // -----------------------------------------------------------------------

    fn exec_ext(&mut self, opcode: u8, ops: &[u16], store: Option<u8>) -> StepResult {
        match opcode {
            // EXT:0x00 save — like 0OP save but store-only (v5+)
            0x00 => {
                let dest = match store {
                    Some(sv) => SaveDest::Store(sv),
                    None => SaveDest::Store(0),
                };
                self.pending_save = Some(PendingSave { result_dest: dest });
                StepResult::SaveRequest
            }
            // EXT:0x01 restore — like 0OP restore but store-only (v5+)
            0x01 => {
                self.pending_restore_store = store;
                StepResult::RestoreRequest
            }
            // EXT:0x02 log_shift — logical (unsigned) shift
            // places > 0 → left shift; places < 0 → right shift (zero-fill)
            0x02 => {
                let n = ops.first().copied().unwrap_or(0);
                let places = ops.get(1).copied().unwrap_or(0) as i16;
                let result = if places >= 16 || places <= -16 {
                    0u16
                } else if places > 0 {
                    n << (places as u16)
                } else if places < 0 {
                    n >> ((-places) as u16)
                } else {
                    n
                };
                self.do_store(store, result);
                StepResult::Continue
            }
            // EXT:0x03 art_shift — arithmetic (signed) shift
            // places > 0 → left shift; places < 0 → arithmetic right shift
            0x03 => {
                let n = ops.first().copied().unwrap_or(0) as i16;
                let places = ops.get(1).copied().unwrap_or(0) as i16;
                let result: i16 = if places >= 16 || places <= -16 {
                    if n < 0 { -1 } else { 0 }
                } else if places > 0 {
                    n << (places as u16)
                } else if places < 0 {
                    n >> ((-places) as u16)
                } else {
                    n
                };
                self.do_store(store, result as u16);
                StepResult::Continue
            }
            // EXT:0x04 set_font — return 0 (font change unsupported)
            0x04 => {
                self.do_store(store, 0);
                StepResult::Continue
            }
            // EXT:0x09 save_undo — unsupported; store -1 (0xFFFF)
            0x09 => {
                self.do_store(store, 0xFFFF);
                StepResult::Continue
            }
            // EXT:0x0A restore_undo — unsupported; store -1 (0xFFFF)
            0x0A => {
                self.do_store(store, 0xFFFF);
                StepResult::Continue
            }
            // Other EXT opcodes: no-op
            _ => StepResult::Continue,
        }
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

    /// Route text through the output-stream state.
    ///
    /// If stream 3 is active, text goes to the memory table buffer (NOT the
    /// screen).  Otherwise it goes to `self.out` (subject to stream 1 being
    /// active).
    pub fn print_text(&mut self, s: &str) {
        // ZMSD 7.1.2.5: when stream 3 is selected it is the ONLY output stream —
        // any future stream-2/4 transcript sink MUST be added below this early
        // return, never above it.
        if self.streams.stream3_active() {
            self.streams.write_stream3(s);
            return;
        }
        // Stream 3 is inactive; streams 1/2/4 apply.
        if self.streams.stream1 {
            self.out.print(s);
        }
    }

    /// Compute the v3 status line from memory globals.
    pub fn status_line(&self) -> crate::screen::StatusLine {
        crate::screen::compute_status_line(&self.mem)
    }

    /// Read global variable N (0-based). Convenience for tests and Tasks 11+.
    pub fn global(&self, n: u8) -> u16 {
        let base = self.mem.global_vars() as u32;
        self.mem.read_word(base + n as u32 * 2)
    }

    /// Complete a suspended `read` instruction by supplying a line of input.
    ///
    /// This is the natural hook for the future automapper to observe the player's
    /// command — the host calls this method with whatever the player typed, and
    /// could record `input` before forwarding to this function.
    ///
    /// Text-buffer layout (ZMSD §15):
    ///   v1–4: byte 0 = max chars; text starts at byte 1 (lower-cased, 0-terminated).
    ///   v5+:  byte 0 = max chars; byte 1 = actual char count; text starts at byte 2
    ///         (lower-cased, NOT zero-terminated).
    ///
    /// Parse-buffer layout (ZMSD §15):
    ///   byte 0 = max tokens (set by game); byte 1 = token count (we write this);
    ///   then for each token: 2-byte dict addr, 1-byte len, 1-byte text-buf position.
    ///   The text-buf position is 1-based from the start of the text buffer, i.e.
    ///   `text_data_start + token.text_pos` where text_data_start = 1 (v1–4) or 2 (v5+).
    ///
    /// For v5+: stores the terminating character (13 = Enter) into the store variable.
    /// Skips tokenisation when parse_buf == 0 (v5+ only).
    pub fn supply_line(&mut self, input: &str) {
        let pending = match self.pending_input.take() {
            Some(p) => p,
            None => return, // no pending read — ignore
        };

        let version = self.mem.version();
        let text_buf = pending.text_buf;
        let parse_buf = pending.parse_buf;

        // Read the max-length cap written by the game (byte 0 of text buffer).
        let max_len = self.mem.read_byte(text_buf) as usize;

        // Lower-case the input and truncate to max_len.
        let lowered: String = input.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
        let text: &str = if lowered.len() > max_len { &lowered[..max_len] } else { &lowered };

        // Write the text into the buffer and set the count/terminator bytes.
        if version <= 4 {
            // v1–4: text starts at byte 1, terminated by a 0 byte.
            let text_data_start: u32 = 1;
            for (i, b) in text.bytes().enumerate() {
                self.mem.write_byte(text_buf + text_data_start + i as u32, b);
            }
            // Null-terminate.
            self.mem.write_byte(text_buf + text_data_start + text.len() as u32, 0);
        } else {
            // v5+: byte 1 = char count; text starts at byte 2, no null terminator.
            let text_data_start: u32 = 2;
            self.mem.write_byte(text_buf + 1, text.len() as u8);
            for (i, b) in text.bytes().enumerate() {
                self.mem.write_byte(text_buf + text_data_start + i as u32, b);
            }
        }

        // Tokenise and fill the parse buffer (skip if parse_buf == 0 in v5+).
        let should_parse = parse_buf != 0 || version <= 4;
        if should_parse && parse_buf != 0 {
            let text_data_start: u8 = if version <= 4 { 1 } else { 2 };
            let max_tokens = self.mem.read_byte(parse_buf) as usize;

            let dict = dictionary::load(&self.mem);
            let tokens = dict.tokenise(&self.mem, text);

            let count = tokens.len().min(max_tokens);
            self.mem.write_byte(parse_buf + 1, count as u8);

            for (i, tok) in tokens.iter().take(max_tokens).enumerate() {
                let entry = parse_buf + 2 + i as u32 * 4;
                // 2-byte dict address.
                self.mem.write_word(entry, tok.dict_addr);
                // 1-byte token length.
                self.mem.write_byte(entry + 2, tok.len);
                // 1-byte text-buffer position: convert 0-based text_pos to
                // 1-based index from start of text buffer.
                let buf_pos = text_data_start + tok.text_pos;
                self.mem.write_byte(entry + 3, buf_pos);
            }
        }

        // v5+: store the terminating character (13 = newline/Enter).
        if version >= 5 {
            self.do_store(pending.store_var, 13);
        }
    }

    /// Complete a suspended `read_char` instruction by supplying a single keystroke.
    ///
    /// `ch` is the ZSCII code of the key pressed (e.g. 65 = 'A').
    /// The value is written into the instruction's store variable.
    pub fn supply_char(&mut self, ch: u8) {
        let pending = match self.pending_input.take() {
            Some(p) => p,
            None => return,
        };
        self.do_store(pending.store_var, ch as u16);
    }

    // -----------------------------------------------------------------------
    // Quetzal save / restore (Task 14)
    // -----------------------------------------------------------------------

    /// Serialise the current machine state to a Quetzal IFF byte buffer.
    ///
    /// The host (CLI / app) should call this after receiving `StepResult::SaveRequest`,
    /// then write the returned bytes to a file (or wherever), then call `complete_save`.
    pub fn save_quetzal(&self) -> Vec<u8> {
        crate::quetzal::save_quetzal(self)
    }

    /// Deliver the result of a save operation back to the machine.
    ///
    /// `ok = true`  → save succeeded (v3: branch taken; v4+: store 1).
    /// `ok = false` → save failed    (v3: fall through; v4+: store 0).
    ///
    /// Must be called after `StepResult::SaveRequest` before the next `step()`.
    pub fn complete_save(&mut self, ok: bool) {
        let pending = match self.pending_save.take() {
            Some(p) => p,
            None => return,
        };
        match pending.result_dest {
            SaveDest::Branch(br) => self.do_branch(Some(br), ok),
            SaveDest::Store(sv) => self.do_store(Some(sv), if ok { 1 } else { 0 }),
        }
    }

    /// Restore machine state from a Quetzal byte buffer supplied by the host.
    ///
    /// On success the machine state (dynamic memory, frames, eval stack, PC) is
    /// replaced with the saved state and `Ok(())` is returned.  On failure the
    /// machine is untouched and an error is returned; the host should then call
    /// `complete_restore_failure()` to set the failure result.
    ///
    /// On a successful restore execution continues from the saved PC — the saved
    /// state already contains the correct resume address (the instruction after
    /// the save opcode) so no additional store/branch is needed.
    pub fn restore_quetzal(&mut self, data: &[u8]) -> Result<(), crate::error::ZError> {
        crate::quetzal::restore_quetzal(self, data)
    }

    /// Signal that a restore operation failed (no data / invalid data).
    ///
    /// v3: fall through (no branch taken); v4+: store 0 into the restore's
    /// store variable.  The store variable was captured into `pending_restore_store`
    /// when the restore opcode fired, so state.pc is already correct (pointing to
    /// the instruction after restore) and must not be modified here.
    pub fn complete_restore_failure(&mut self) {
        if self.mem.version() <= 3 {
            // v3 restore is a branch instruction; on failure just fall through
            // (no state change needed — execution continues at state.pc which
            // is already past the restore instruction).
        } else {
            // v4+: use the store variable captured when the restore opcode fired.
            if let Some(sv) = self.pending_restore_store.take() {
                self.do_store(Some(sv), 0);
            }
        }
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

    // -----------------------------------------------------------------------
    // Task 12: input opcode tests
    //
    // Dictionary layout shared by these tests:
    //   Same hand-built v3/v5 dict used in dictionary module tests.
    //   We embed a minimal dictionary containing "north", "open", "mailbox"
    //   at memory address 0x0200 (where sample_story points `dictionary`).
    //
    // Text/parse buffers are placed in dynamic memory away from code:
    //   text_buf  at 0x0300 (before global_vars which starts at 0x0300 —
    //   BUT global_vars are at 0x0300 in sample_story! Use 0x0280 instead.)
    //   parse_buf at 0x02C0
    // -----------------------------------------------------------------------

    /// Build a story buffer with a hand-crafted dictionary at 0x0200.
    /// Entries: "north", "open", "mailbox" (sorted by encoded key, 4-byte keys, v3).
    /// Returns (buf, addr_north, addr_open, addr_mailbox).
    fn build_input_story(version: u8) -> (Vec<u8>, u16, u16, u16) {
        use crate::text::encode::encode_word;

        let mut buf = sample_story(version);

        // We use 4-byte keys for v3 and 6-byte keys for v5.
        let key_len: usize = if version <= 3 { 4 } else { 6 };

        // encode_word takes the story version (not syllable count).
        let key_north   = encode_word("north",   version);
        let key_open    = encode_word("open",    version);
        let key_mailbox = encode_word("mailbox", version);

        // Sort by key bytes for binary search.
        let mut entries: Vec<(&str, Vec<u8>)> = vec![
            ("north",   key_north.clone()),
            ("open",    key_open.clone()),
            ("mailbox", key_mailbox.clone()),
        ];
        entries.sort_by(|a, b| a.1.cmp(&b.1));

        let entry_length: usize = key_len + 2; // key + 2 bytes game data (total ≥ 4 v3, ≥ 6 v4+)

        // Write dictionary header at 0x0200.
        buf[0x0200] = 1;    // 1 separator
        buf[0x0201] = b'.'; // separator = '.'
        buf[0x0202] = entry_length as u8;
        buf[0x0203] = 0;
        buf[0x0204] = 3;    // count = 3

        // Entries start at 0x0205.
        let entries_base: usize = 0x0205;
        for (i, (_word, key)) in entries.iter().enumerate() {
            let base = entries_base + i * entry_length;
            buf[base..base + key_len].copy_from_slice(&key[..key_len]);
        }

        // Compute addresses for each word in sorted order.
        let addr_for = |word: &str| -> u16 {
            for (i, (w, _)) in entries.iter().enumerate() {
                if *w == word {
                    return (entries_base + i * entry_length) as u16;
                }
            }
            panic!("word not found: {}", word);
        };

        let addr_north   = addr_for("north");
        let addr_open    = addr_for("open");
        let addr_mailbox = addr_for("mailbox");

        (buf, addr_north, addr_open, addr_mailbox)
    }

    /// Build a VAR-form `read` instruction (opcode 0x04) at `buf[offset]`.
    /// Operands: two Large constants (text_buf addr, parse_buf addr).
    /// v5+ includes a store byte; v3 does not.
    /// Returns the number of bytes emitted.
    fn emit_read(buf: &mut [u8], offset: usize, text_buf: u16, parse_buf: u16, version: u8, store_var: Option<u8>) -> usize {
        // VAR-form opcode for read: 0b11_1_00100 = 0xE4
        buf[offset] = 0xE4;
        // Type byte: first two = large const (0b00), rest = omit (0b11).
        // 0b00_00_11_11 = 0x0F
        buf[offset + 1] = 0x0F;
        // text_buf (large const, 2 bytes)
        buf[offset + 2] = (text_buf >> 8) as u8;
        buf[offset + 3] = (text_buf & 0xFF) as u8;
        // parse_buf (large const, 2 bytes)
        buf[offset + 4] = (parse_buf >> 8) as u8;
        buf[offset + 5] = (parse_buf & 0xFF) as u8;
        let mut len = 6;
        // v5+ has store byte
        if version >= 5 {
            if let Some(sv) = store_var {
                buf[offset + len] = sv;
                len += 1;
            }
        }
        len
    }

    // -----------------------------------------------------------------------
    // Test (a-v3): v3 read → NeedLine → supply_line("north") → check text/parse buf
    // -----------------------------------------------------------------------
    #[test]
    fn read_v3_need_line_supply_north() {
        let (mut buf, addr_north, _addr_open, _addr_mailbox) = build_input_story(3);

        // Text buffer at 0x0250: byte0=max_len=10, rest zero.
        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 10; // max 10 chars

        // Parse buffer at 0x0260: byte0=max_tokens=8, rest zero.
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8; // max 8 tokens

        // Instruction at 0x0010: read text_buf, parse_buf; quit
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        // Step 1: step() returns NeedLine with correct addresses.
        let result = m.step();
        assert!(
            matches!(result, StepResult::NeedLine { text_buf: tb, parse_buf: pb } if tb == text_buf as u32 && pb == parse_buf as u32),
            "expected NeedLine{{text_buf={:#x}, parse_buf={:#x}}}, got {:?}", text_buf, parse_buf, result
        );

        // Step 2: supply_line("north") and check text buffer (v3 layout).
        m.supply_line("north");

        // v3: byte 0 = max (untouched), text at byte 1, null-terminated.
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'n', "text[1] = 'n'");
        assert_eq!(m.mem.read_byte(tb + 2), b'o', "text[2] = 'o'");
        assert_eq!(m.mem.read_byte(tb + 3), b'r', "text[3] = 'r'");
        assert_eq!(m.mem.read_byte(tb + 4), b't', "text[4] = 't'");
        assert_eq!(m.mem.read_byte(tb + 5), b'h', "text[5] = 'h'");
        assert_eq!(m.mem.read_byte(tb + 6), 0,    "null terminator at text[6]");

        // Parse buffer: token count = 1, first token has correct fields.
        let pb = parse_buf as u32;
        assert_eq!(m.mem.read_byte(pb + 1), 1, "parse buf: 1 token");
        // Token 0: dict_addr (2 bytes), len (1 byte), text_buf_pos (1 byte).
        let tok_dict = m.mem.read_word(pb + 2);
        let tok_len  = m.mem.read_byte(pb + 4);
        let tok_pos  = m.mem.read_byte(pb + 5);
        assert_eq!(tok_dict, addr_north, "token dict addr = addr_north ({:#x})", addr_north);
        assert_eq!(tok_len,  5,          "token len = 5 ('north')");
        assert_eq!(tok_pos,  1,          "token pos = 1 (v3: text starts at byte 1, 'north' at pos 0 in input → buf pos = 1+0 = 1)");

        // Machine continues normally after supply_line.
        let r2 = m.step();
        assert_eq!(r2, StepResult::Quit, "next step is quit");
    }

    // -----------------------------------------------------------------------
    // Test (a-v5): v5 read → NeedLine → supply_line("north") → check text/parse buf
    // -----------------------------------------------------------------------
    #[test]
    fn read_v5_need_line_supply_north() {
        let (mut buf, addr_north, _addr_open, _addr_mailbox) = build_input_story(5);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 10; // max 10 chars

        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        // v5: read has a store var (terminator char).
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 5, Some(0x10)); // store→G0
        buf[0x0010 + n] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        let result = m.step();
        assert!(
            matches!(result, StepResult::NeedLine { text_buf: tb, parse_buf: pb } if tb == text_buf as u32 && pb == parse_buf as u32),
            "expected NeedLine, got {:?}", result
        );

        m.supply_line("north");

        // v5: byte 0 = max (untouched), byte 1 = char count, text at byte 2.
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), 5,    "v5: char count = 5");
        assert_eq!(m.mem.read_byte(tb + 2), b'n', "text[2] = 'n'");
        assert_eq!(m.mem.read_byte(tb + 6), b'h', "text[6] = 'h'");

        // Parse buffer: 1 token, correct position (text_data_start=2 for v5).
        let pb = parse_buf as u32;
        assert_eq!(m.mem.read_byte(pb + 1), 1, "1 token");
        let tok_dict = m.mem.read_word(pb + 2);
        let tok_len  = m.mem.read_byte(pb + 4);
        let tok_pos  = m.mem.read_byte(pb + 5);
        assert_eq!(tok_dict, addr_north, "v5 token dict addr = addr_north");
        assert_eq!(tok_len,  5,          "v5 token len = 5");
        assert_eq!(tok_pos,  2,          "v5 token pos = 2 (text_data_start=2, text_pos=0 → 2+0=2)");

        // v5: terminator (13 = Enter) stored in G0.
        assert_eq!(m.global(0), 13, "v5 read stores terminator 13 in G0");
    }

    // -----------------------------------------------------------------------
    // Test (b): two-word input "open mailbox" → 2 tokens, correct positions
    // -----------------------------------------------------------------------
    #[test]
    fn read_v3_two_word_input_open_mailbox() {
        let (mut buf, _addr_north, addr_open, addr_mailbox) = build_input_story(3);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 20; // max 20 chars

        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        let result = m.step();
        assert!(matches!(result, StepResult::NeedLine { .. }));

        m.supply_line("open mailbox");

        // v3 layout: text at offset 1.
        // "open mailbox" → 12 chars; null at byte 1+12=13.
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'o', "starts with 'o'");
        assert_eq!(m.mem.read_byte(tb + 13), 0, "null terminator");

        let pb = parse_buf as u32;
        assert_eq!(m.mem.read_byte(pb + 1), 2, "2 tokens");

        // Token 0: "open" at text_pos=0 → buf_pos=1.
        let tok0_dict = m.mem.read_word(pb + 2);
        let tok0_len  = m.mem.read_byte(pb + 4);
        let tok0_pos  = m.mem.read_byte(pb + 5);
        assert_eq!(tok0_dict, addr_open, "tok0 = 'open'");
        assert_eq!(tok0_len,  4,         "tok0 len = 4");
        assert_eq!(tok0_pos,  1,         "tok0 buf_pos = 1 (text_data_start=1, text_pos=0)");

        // Token 1: "mailbox" at text_pos=5 → buf_pos=6.
        let tok1_dict = m.mem.read_word(pb + 6);
        let tok1_len  = m.mem.read_byte(pb + 8);
        let tok1_pos  = m.mem.read_byte(pb + 9);
        assert_eq!(tok1_dict, addr_mailbox, "tok1 = 'mailbox'");
        assert_eq!(tok1_len,  7,            "tok1 len = 7");
        assert_eq!(tok1_pos,  6,            "tok1 buf_pos = 6 (text_data_start=1, text_pos=5 → 1+5=6)");
    }

    // -----------------------------------------------------------------------
    // Test (c): v5 read stores terminator char (already covered in v5 test,
    //   but explicit test for completeness)
    // -----------------------------------------------------------------------
    #[test]
    fn read_v5_stores_terminator() {
        let (mut buf, _addr_north, _addr_open, _addr_mailbox) = build_input_story(5);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 20;

        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        // Store into G1 (var 0x11)
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 5, Some(0x11));
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        m.step(); // returns NeedLine
        m.supply_line("hello");

        // G1 should have been set to 13 (Enter).
        let g1 = m.mem.read_word(m.mem.global_vars() as u32 + 1 * 2);
        assert_eq!(g1, 13, "v5 read terminator stored in G1 = 13");
    }

    // -----------------------------------------------------------------------
    // Test (d): read_char → NeedChar → supply_char(65) stores 65 in store var
    // -----------------------------------------------------------------------
    #[test]
    fn read_char_need_char_supply_char() {
        let mut buf = sample_story(5);

        // VAR-form read_char: opcode 0x16 → 0b11_1_10110 = 0xF6
        // Operands: first arg = 1 (keyboard, required). Type byte: small const(01), rest omit(11).
        // Store byte → G0 (0x10).
        buf[0x0010] = 0xF6; // VAR read_char
        buf[0x0011] = 0x7F; // type: small(01), omit, omit, omit → 0b01_11_11_11 = 0x7F
        buf[0x0012] = 1;    // operand: device=1 (keyboard)
        buf[0x0013] = 0x10; // store → G0
        buf[0x0014] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        let result = m.step();
        assert_eq!(result, StepResult::NeedChar, "read_char returns NeedChar");

        m.supply_char(65); // ZSCII 'A'

        assert_eq!(m.global(0), 65, "supply_char(65) stored in G0");

        let r2 = m.step();
        assert_eq!(r2, StepResult::Quit, "machine resumes after supply_char");
    }

    // -----------------------------------------------------------------------
    // Test: input is lower-cased before writing to text buffer
    // -----------------------------------------------------------------------
    #[test]
    fn read_lower_cases_input() {
        let (mut buf, _addr_north, _addr_open, _addr_mailbox) = build_input_story(3);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 20;
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;
        m.step();
        m.supply_line("NORTH");

        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'n', "upper N lower-cased to 'n'");
        assert_eq!(m.mem.read_byte(tb + 5), b'h', "upper H lower-cased to 'h'");
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

    // -----------------------------------------------------------------------
    // Task 13: screen model, output stream 3, window opcodes
    // -----------------------------------------------------------------------

    /// Helper: emit a VAR-form instruction with up to 4 small-const operands.
    /// `opcode` is the VAR opcode number (0x00–0x1F).
    fn emit_var_instr(buf: &mut Vec<u8>, opcode: u8, ops: &[u8]) {
        // VAR form: 0b11_1_xxxxx
        buf.push(0b1110_0000 | (opcode & 0x1F));
        // Type byte: each op = small-const (0b01), unused = omit (0b11).
        let mut type_byte: u8 = 0xFF;
        for (i, _) in ops.iter().enumerate().take(4) {
            let shift = 6u8.saturating_sub(2 * i as u8);
            type_byte &= !(0b11 << shift);
            type_byte |= 0b01 << shift; // small const
        }
        buf.push(type_byte);
        for &op in ops.iter().take(4) {
            buf.push(op);
        }
    }

    /// Emit VAR output_stream (0x13) with a large-const signed stream number.
    /// Z-machine signed stream numbers (e.g. -1, -3) require 16-bit large constants.
    fn emit_output_stream_large(buf: &mut Vec<u8>, stream_val: i16) {
        let v = stream_val as u16;
        buf.push(0b1110_0000 | 0x13);  // VAR:0x13
        // Type byte: first=large(0b00), rest=omit(0b11) → 0b00_11_11_11 = 0x3F
        buf.push(0x3F);
        buf.push((v >> 8) as u8);
        buf.push((v & 0xFF) as u8);
    }

    /// Emit output_stream +3 with a large-const table address.
    /// stream=3 (small const), table_addr (large const).
    /// type_byte: first=small(01), second=large(00), rest=omit(11) → 0b01_00_11_11 = 0x4F
    fn emit_output_stream3_on(buf: &mut Vec<u8>, table_addr: u16) {
        buf.push(0b1110_0000 | 0x13); // VAR:0x13
        // Type byte: op0=small(01), op1=large(00), rest=omit(11)
        buf.push(0b01_00_11_11);      // 0x4F
        buf.push(3u8);                // stream number = 3 (small const)
        buf.push((table_addr >> 8) as u8);
        buf.push((table_addr & 0xFF) as u8);
    }

    // ── (a) set_text_style and split_window update ScreenState ───────────────

    #[test]
    fn screen_set_text_style_and_split_window() {
        // Program at 0x10 (v5):
        //   set_text_style 1  (bold)   → screen.text_style = 1
        //   split_window  3           → screen.upper_window_rows = 3
        //   set_window    1           → screen.current_window = 1
        //   quit
        //
        // set_text_style = VAR:0x11
        // split_window   = VAR:0x0A
        // set_window     = VAR:0x0B
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;

        // set_text_style 1: VAR:0x11, small 1
        let instr = {let mut v = vec![]; emit_var_instr(&mut v, 0x11, &[1]); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // split_window 3: VAR:0x0A, small 3
        let instr = {let mut v = vec![]; emit_var_instr(&mut v, 0x0A, &[3]); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // set_window 1: VAR:0x0B, small 1
        let instr = {let mut v = vec![]; emit_var_instr(&mut v, 0x0B, &[1]); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        assert_eq!(m.screen.text_style, 1, "set_text_style(1) → text_style=1");
        assert_eq!(m.screen.upper_window_rows, 3, "split_window(3) → upper_window_rows=3");
        assert_eq!(m.screen.current_window, 1, "set_window(1) → current_window=1");
    }

    // ── (b) show_status (v3 0OP:0x0C) sets the flag ─────────────────────────

    #[test]
    fn screen_show_status_v3_sets_flag() {
        // v3 program: show_status (0xBC), quit
        let mut buf = sample_story(3);
        buf[0x10] = 0xBC; // 0OP:0x0C show_status
        buf[0x11] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert!(m.screen.show_status_requested, "show_status should set the flag");
    }

    // ── (c) output_stream 3: text goes to memory table, NOT screen ───────────

    #[test]
    fn output_stream3_redirects_text_to_table() {
        // Program at 0x10 (v5):
        //   output_stream +3 table_addr    → select stream 3
        //   print "ab"                     → goes to table, NOT screen
        //   output_stream -3               → deselect stream 3
        //   print "cd"                     → goes to screen (stream 1)
        //   quit
        //
        // Table at 0x0060 (inside dynamic memory, safely below 0x0400).
        // "ab" Z-encoded: a=Z6, b=Z7, pad=Z5 → word = 0x8000|(6<<10)|(7<<5)|5 = 0x99C5
        // "cd" Z-encoded: c=Z8, d=Z9, pad=Z5 → word = 0x8000|(8<<10)|(9<<5)|5

        let table_addr: u16 = 0x0060;
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;

        // output_stream +3, table_addr
        let instr = {let mut v = vec![]; emit_output_stream3_on(&mut v, table_addr); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print "ab" (0OP:0x02 inline)
        let ab_word: u16 = 0x8000 | (6u16 << 10) | (7u16 << 5) | 5u16;
        buf[pos] = 0xB2; pos += 1; // 0OP print
        buf[pos] = (ab_word >> 8) as u8; pos += 1;
        buf[pos] = (ab_word & 0xFF) as u8; pos += 1;

        // output_stream -3 (deselect stream 3)
        let instr = {let mut v = vec![]; emit_output_stream_large(&mut v, -3); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print "cd" (0OP:0x02 inline)
        let cd_word: u16 = 0x8000 | (8u16 << 10) | (9u16 << 5) | 5u16;
        buf[pos] = 0xB2; pos += 1;
        buf[pos] = (cd_word >> 8) as u8; pos += 1;
        buf[pos] = (cd_word & 0xFF) as u8; pos += 1;

        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        // Screen should only have "cd" (stream 1).
        let screen_text = &m.buffer_output().expect("BufferOutput").buf;
        assert_eq!(screen_text.as_str(), "cd", "screen should only receive 'cd' (not 'ab')");

        // Table at 0x0060: word=length=2, then 'a','b'
        assert_eq!(m.mem.read_word(table_addr as u32), 2, "table length word = 2");
        assert_eq!(m.mem.read_byte(table_addr as u32 + 2), b'a', "table[0] = 'a'");
        assert_eq!(m.mem.read_byte(table_addr as u32 + 3), b'b', "table[1] = 'b'");
    }

    // ── (d) stream 1 off: screen receives nothing ─────────────────────────────

    #[test]
    fn output_stream1_off_suppresses_screen() {
        // output_stream -1 (disable screen), print "x", output_stream +1, print "y", quit
        // Screen should only have "y".
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;

        // output_stream -1
        let instr = {let mut v = vec![]; emit_output_stream_large(&mut v, -1); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print "x": Z-encode x (A0: z=Z31; x is... wait let's use print_char)
        // print_char ZSCII 120 = 'x': VAR:0x05
        buf[pos] = 0xE5; pos += 1;     // VAR print_char
        buf[pos] = 0x7F; pos += 1;     // type: small, omit, omit, omit
        buf[pos] = 120u8; pos += 1;    // 'x' = ZSCII 120

        // output_stream +1
        let instr = {let mut v = vec![]; emit_output_stream_large(&mut v, 1); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print_char 'y' = 121
        buf[pos] = 0xE5; pos += 1;
        buf[pos] = 0x7F; pos += 1;
        buf[pos] = 121u8; pos += 1;    // 'y'

        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        let out = m.buffer_output().expect("BufferOutput");
        assert_eq!(out.buf, "y", "only 'y' reaches screen (stream 1 was off for 'x')");
    }

    // ── (e) Machine::init_caps sets header bits correctly ────────────────────

    #[test]
    fn machine_init_caps_sets_header_bits() {
        // Build a machine on a story where the initial_pc is past 0x40
        // (so there's no program at 0x10 that conflicts with Flags2).
        // sample_story sets initial_pc = 0x0040 and programs at 0x40+.
        // But we need programs at a safe location. Let's use 0x80 as initial_pc.
        let mut buf = sample_story(5);
        // Place quit at 0x80 so the machine doesn't crash.
        buf[0x80] = 0xBA;
        // Override initial_pc to 0x0080 in the header.
        buf[0x06] = 0x00;
        buf[0x07] = 0x80;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.init_caps();

        // Check that Flags1 has fixed-space font bit set (bit 4 for v5).
        let f1 = m.mem.read_byte(0x01);
        assert_ne!(f1 & (1 << 4), 0, "Flags1 bit 4 (fixed-space font) should be set");

        // Interpreter number and version.
        assert_eq!(m.mem.read_byte(0x1E), 6, "interpreter number = 6");
        assert_eq!(m.mem.read_byte(0x1F), b'A', "interpreter version = 'A'");
    }

    // -----------------------------------------------------------------------
    // Test: v4+ restore-failure stores 0 into the correct store variable
    // and does not corrupt state.pc.
    //
    // Program layout (v5 story at 0x10):
    //   0x10: 0OP restore (0x06), store byte = 0x10 (global 0)
    //         Encoded as short 0OP: 0xB6, then store byte 0x10
    //         → step() decodes this, captures store=G0, sets state.pc=0x12,
    //           then returns RestoreRequest.
    //   0x12: quit (0xBA)
    //
    // After complete_restore_failure():
    //   global(0) == 0  (failure result stored into G0)
    //   state.pc  == 0x12 (unchanged — points to quit)
    // -----------------------------------------------------------------------

    #[test]
    fn restore_failure_stores_zero_into_correct_var_and_pc_unchanged() {
        let mut buf = sample_story(5);
        // restore opcode: short 0OP form = 0xB6, followed by store byte
        buf[0x10] = 0xB6; // 0OP:0x06 restore
        buf[0x11] = 0x10; // store → global 0 (var 0x10)
        buf[0x12] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;

        // Pre-condition: global 0 is 0 (default), but set it to a non-zero sentinel
        // so we can prove it was written by complete_restore_failure.
        let base = m.mem.global_vars() as u32;
        m.mem.write_word(base, 0xABCD); // G0 = 0xABCD (sentinel)

        // Execute the restore instruction.
        let result = m.step();
        assert_eq!(result, StepResult::RestoreRequest, "restore opcode must return RestoreRequest");

        // After step(): pc must be 0x12 (past the store byte).
        assert_eq!(m.state.pc, 0x12, "state.pc must point to instruction after restore (0x12)");

        // Simulate restore failure (no save data).
        m.complete_restore_failure();

        // G0 must now be 0 (failure result).
        assert_eq!(m.global(0), 0, "restore failure must store 0 into the store variable (G0)");

        // state.pc must still be 0x12 — complete_restore_failure must not advance pc.
        assert_eq!(m.state.pc, 0x12, "state.pc must not be corrupted by complete_restore_failure");
    }

    // -----------------------------------------------------------------------
    // loadw / loadb / storew / storeb round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn loadw_storew_round_trip() {
        // storew G0 #0 G1  — store G1 at dynamic_mem[G0 + 0]
        // loadw G0 #0 → G2  — read back from same address
        //
        // Hand-assembled bytes:
        //   storew: VAR:01, type_byte=[Var,Small,Var,omit], G0, 0, G1
        //     0xC1 (VAR bit5=0=2OP? No, VAR:01 with bit5=1=Var)
        //     Wait: VAR:01 opcode byte = 0b11_1_00001 = 0xE1, type_byte=0b10_01_10_11=0xAB
        //
        // Actually easier to set base address to a known dynamic address (e.g. 0x40)
        // and use raw bytes.
        //
        // storew: 0xE1 (VAR:01), type=0xAB([Var,Small,Var,omit]), G0, 0, G1
        // loadw:  VAR:0F = 0b11_1_01111 = 0xEF, type=0b10_01_11_11=0x9F, G0, 0, store=G2
        let mut buf = sample_story(5);
        // Set up globals: G0=0x40 (base address in dynamic mem), G1=0xBEEF (value to store)
        let gbase = {
            let tmp = Memory::new(buf.clone()).unwrap();
            tmp.global_vars() as usize
        };
        // G0 = 0x40
        buf[gbase]     = 0x00;
        buf[gbase + 1] = 0x40;
        // G1 = 0xBEEF
        buf[gbase + 2] = 0xBE;
        buf[gbase + 3] = 0xEF;

        // storew G0 #0 G1:  E1 AB 10 00 11
        buf[0x10] = 0xE1; // VAR:01 storew
        buf[0x11] = 0xAB; // type: [Var=10, Small=01, Var=10, omit=11]
        buf[0x12] = 0x10; // G0 (var 0x10)
        buf[0x13] = 0x00; // index 0
        buf[0x14] = 0x11; // G1 (var 0x11)
        // loadw G0 #0 → G2:  EF 9F 10 00 12
        buf[0x15] = 0xEF; // VAR:0F (but 0xEF with bit5=1=Var, opcode=0x0F=15)
        // Wait: 0xEF = 0b11_1_01111: VAR form, bit5=1→Var, opcode=0x0F=loadw
        // but loadw is 2OP:0x0F. In VAR form with bit5=0→Two, 0xCF would be loadw.
        // 0xEF has bit5=1→Var, so that's VAR:0x0F (not 2OP). But loadw is 2OP!
        // Use Long form instead: 0x4F (bit6=1=Var, bit5=0=Small, op=0x0F=loadw)
        //   Long: 0b01_0_01111 = 0x4F, G0, 0, store=G2
        buf[0x15] = 0x4F; // long form: Var, Small, opcode=0x0F=loadw
        buf[0x16] = 0x10; // G0
        buf[0x17] = 0x00; // index 0
        buf[0x18] = 0x12; // store → G2
        buf[0x19] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(2), 0xBEEF, "loadw round-trip: should read back 0xBEEF");
    }

    #[test]
    fn loadb_storeb_round_trip() {
        // storeb base #0 val  — write byte at base+0
        // loadb base #0 → G2  — read back the byte
        let mut buf = sample_story(5);
        let gbase = {
            let tmp = Memory::new(buf.clone()).unwrap();
            tmp.global_vars() as usize
        };
        // G0 = 0x40 (base)
        buf[gbase]     = 0x00;
        buf[gbase + 1] = 0x40;
        // G1 = 0x42 (byte value to store)
        buf[gbase + 2] = 0x00;
        buf[gbase + 3] = 0x42;

        // storeb G0 #0 G1:  E2 AB 10 00 11  (VAR:02)
        buf[0x10] = 0xE2; // VAR:02 storeb
        buf[0x11] = 0xAB; // [Var, Small, Var, omit]
        buf[0x12] = 0x10;
        buf[0x13] = 0x00;
        buf[0x14] = 0x11;
        // loadb G0 #0 → G2:  Long form 0x50, Var G0, Small 0, store G2
        //   long: bit6=1(var), bit5=0(small), opcode=0x10=loadb → 0b01_0_10000=0x50
        buf[0x15] = 0x50;
        buf[0x16] = 0x10;
        buf[0x17] = 0x00;
        buf[0x18] = 0x12;
        buf[0x19] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(2), 0x42, "loadb round-trip: should read back 0x42");
    }

    // -----------------------------------------------------------------------
    // random — result must be in [1, range] for positive range
    // -----------------------------------------------------------------------

    #[test]
    fn random_in_range() {
        // random #10 → G0; quit
        // VAR:07 random: 0xE7 (bit5=1→Var, op=7), type_byte=0x7F([Small,omit,omit,omit]),
        //   operand=10, store=G0(0x10)
        let mut buf = sample_story(5);
        buf[0x10] = 0xE7; // VAR:07 random
        buf[0x11] = 0x7F; // type: [Small, omit, omit, omit]
        buf[0x12] = 10;   // range = 10
        buf[0x13] = 0x10; // store → G0
        buf[0x14] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        let result = m.global(0);
        assert!(result >= 1 && result <= 10,
            "random(10) must be in [1,10], got {result}");
    }

    // -----------------------------------------------------------------------
    // log_shift / art_shift (EXT opcodes)
    // -----------------------------------------------------------------------

    #[test]
    fn log_shift_left_and_right() {
        // log_shift value places → store
        // EXT:02 format: 0xBE 0x02 type_byte [operands] store
        //
        // Type byte encoding (2-bit fields, MSB-first):
        //   00=Large(16-bit), 01=Small(8-bit), 10=Variable, 11=omit
        // For [Small, Small, omit, omit]: 0b01_01_11_11 = 0x5F
        // For [Small, Large, omit, omit]: 0b01_00_11_11 = 0x4F (Large places)
        //
        // Test 1: log_shift 8 places=2 → G0  (8u << 2 = 32)
        // Test 2: log_shift 8 places=-1 → G1 (8u >> 1 = 4, unsigned shift)
        //   places=-1 requires Large constant 0xFFFF (Small only covers 0..255)
        let mut buf = sample_story(5);
        let mut pc = 0x10usize;

        // EXT:02, [Small value=8, Small places=2, omit, omit], store=G0
        buf[pc] = 0xBE; buf[pc+1] = 0x02; buf[pc+2] = 0x5F; // type: [S,S,_,_]
        buf[pc+3] = 8; buf[pc+4] = 2; buf[pc+5] = 0x10; pc += 6;

        // EXT:02, [Small value=8, Large places=0xFFFF(-1), omit, omit], store=G1
        buf[pc] = 0xBE; buf[pc+1] = 0x02; buf[pc+2] = 0x4F; // type: [S,L,_,_]
        buf[pc+3] = 8; buf[pc+4] = 0xFF; buf[pc+5] = 0xFF; buf[pc+6] = 0x11; pc += 7;

        buf[pc] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 32, "log_shift 8 << 2 = 32");
        assert_eq!(m.global(1), 4, "log_shift 8 >> 1 = 4 (unsigned)");
    }

    #[test]
    fn art_shift_preserves_sign() {
        // art_shift value places → store
        // EXT:03 format: 0xBE 0x03 type_byte [operands] store
        //
        // Test 1: art_shift 0xFFF8 places=-1 → G0  (-8 >> 1 = -4 = 0xFFFC, sign-extended)
        // Test 2: art_shift 0xFFF8 places=2  → G1  (-8 << 2 = -32 = 0xFFE0)
        // Both need Large(0xFFF8) and either Large(0xFFFF=-1) or Small(2).
        // Type [Large, Large, omit, omit]: 0b00_00_11_11 = 0x0F
        // Type [Large, Small, omit, omit]: 0b00_01_11_11 = 0x1F
        let mut buf = sample_story(5);
        let mut pc = 0x10usize;

        // EXT:03, [Large value=0xFFF8, Large places=0xFFFF(-1), omit, omit], store=G0
        buf[pc] = 0xBE; buf[pc+1] = 0x03; buf[pc+2] = 0x0F; // type: [L,L,_,_]
        buf[pc+3] = 0xFF; buf[pc+4] = 0xF8; // value = 0xFFF8
        buf[pc+5] = 0xFF; buf[pc+6] = 0xFF; // places = 0xFFFF = -1
        buf[pc+7] = 0x10; // store → G0
        pc += 8;

        // EXT:03, [Large value=0xFFF8, Small places=2, omit, omit], store=G1
        buf[pc] = 0xBE; buf[pc+1] = 0x03; buf[pc+2] = 0x1F; // type: [L,S,_,_]
        buf[pc+3] = 0xFF; buf[pc+4] = 0xF8; // value = 0xFFF8
        buf[pc+5] = 0x02; // places = 2
        buf[pc+6] = 0x11; // store → G1
        pc += 7;

        buf[pc] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        // 0xFFF8 = -8 signed. arithmetic right-shift by 1 = -4 = 0xFFFC
        assert_eq!(m.global(0), 0xFFFC, "art_shift(-8, -1) = -4 = 0xFFFC");
        // arithmetic left-shift by 2: -8 << 2 = -32 = 0xFFE0
        assert_eq!(m.global(1), 0xFFE0, "art_shift(-8, 2) = -32 = 0xFFE0");
    }

    // -----------------------------------------------------------------------
    // pull sp — overwrite-new-top semantics (frotz §z_pull)
    // -----------------------------------------------------------------------

    #[test]
    fn pull_sp_overwrites_new_top() {
        // Stack before: [10, 20, 30] (10=bottom, 30=top)
        // pull #0 (pull Small(0) = destination is sp):
        //   pop 30 (value), stack=[10, 20], poke_stack(30) → stack=[10, 30]
        // pull #0 again:
        //   pop 30, stack=[10], poke_stack(30) → stack=[30]
        // pull Small(G0) = pop 30 into G0.
        // Then G0 should be 30, and the stack should have one item (30).
        //
        // We push 10, 20, 30 using push opcodes, then do two pull-sp, then
        // do a normal pull into G0, then quit.
        //
        // push: VAR:08 = 0xE8, type_byte=[Small,omit,omit,omit]=0x7F, value
        // pull Small(0): VAR:09 = 0xE9, type_byte=[Small,omit,omit,omit]=0x7F, 0
        // pull Small(G0_var=0x10): 0xE9, 0x7F, 0x10
        let mut buf = sample_story(5);
        let prog: &[u8] = &[
            0xE8, 0x7F, 10,   // push 10
            0xE8, 0x7F, 20,   // push 20
            0xE8, 0x7F, 30,   // push 30  — stack = [10, 20, 30]
            0xE9, 0x7F, 0x00, // pull #0 (sp)  — pops 30, pokes 30 over 20 → [10, 30]
            0xE9, 0x7F, 0x00, // pull #0 (sp)  — pops 30, pokes 30 over 10 → [30]
            0xE9, 0x7F, 0x10, // pull Small(G0=var 0x10)  — pops 30 into G0
            0xBA,             // quit
        ];
        for (i, &b) in prog.iter().enumerate() {
            buf[0x10 + i] = b;
        }
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 30, "pull sp: final value pulled into G0 should be 30");
    }

    // -----------------------------------------------------------------------
    // scan_table (VAR:0x17) — search table for value, store address, branch if found
    // -----------------------------------------------------------------------

    #[test]
    fn scan_table_word_finds_and_stores_address() {
        let mut m = build_test_machine(&[]);
        // Word table at 0x0200: [0x1111, 0x2222, 0x3333]
        m.mem.write_word(0x0200, 0x1111);
        m.mem.write_word(0x0202, 0x2222);
        m.mem.write_word(0x0204, 0x3333);
        // scan_table 0x2222, table=0x0200, len=3, form=0x82 (word, step 2) -> G0
        m.exec_var(0x17, &[0x2222, 0x0200, 3, 0x82], Some(16), None);
        assert_eq!(m.global(0), 0x0202, "address of the matching word entry");
    }

    #[test]
    fn scan_table_not_found_stores_zero() {
        let mut m = build_test_machine(&[]);
        m.mem.write_word(0x0200, 0x1111);
        m.exec_var(0x17, &[0x9999, 0x0200, 1, 0x82], Some(16), None);
        assert_eq!(m.global(0), 0, "no match -> store 0");
    }

    #[test]
    fn scan_table_byte_form_compares_low_byte() {
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x0200, 0x05);
        m.mem.write_byte(0x0201, 0x07);
        // form=0x01 -> byte entries, step 1
        m.exec_var(0x17, &[0x0007, 0x0200, 2, 0x01], Some(16), None);
        assert_eq!(m.global(0), 0x0201, "byte form matches low byte at the second entry");
    }

    // -----------------------------------------------------------------------
    // copy_table (VAR:0x1D) — copy/zero memory region
    // -----------------------------------------------------------------------

    #[test]
    fn copy_table_copies_forward() {
        let mut m = build_test_machine(&[]);
        for i in 0..4u32 { m.mem.write_byte(0x0200 + i, (i + 1) as u8); } // 1,2,3,4
        m.exec_var(0x1D, &[0x0200, 0x0300, 4], None, None);
        for i in 0..4u32 { assert_eq!(m.mem.read_byte(0x0300 + i), (i + 1) as u8); }
    }

    #[test]
    fn copy_table_zeroes_when_second_is_zero() {
        let mut m = build_test_machine(&[]);
        for i in 0..3u32 { m.mem.write_byte(0x0200 + i, 0xFF); }
        m.exec_var(0x1D, &[0x0200, 0, 3], None, None);
        for i in 0..3u32 { assert_eq!(m.mem.read_byte(0x0200 + i), 0); }
    }

    #[test]
    fn copy_table_positive_size_overlap_is_noncorrupting() {
        let mut m = build_test_machine(&[]);
        for i in 0..4u32 { m.mem.write_byte(0x0200 + i, (i + 1) as u8); } // 1,2,3,4
        // Overlapping forward copy by 1 (dest > src). Positive size must NOT corrupt:
        // result at 0x0201..=0x0204 should be the ORIGINAL 1,2,3,4.
        m.exec_var(0x1D, &[0x0200, 0x0201, 4], None, None);
        assert_eq!(m.mem.read_byte(0x0201), 1);
        assert_eq!(m.mem.read_byte(0x0202), 2);
        assert_eq!(m.mem.read_byte(0x0203), 3);
        assert_eq!(m.mem.read_byte(0x0204), 4);
    }

    #[test]
    fn get_cursor_writes_row_and_col() {
        let mut m = build_test_machine(&[]);
        m.screen.cursor_row = 3;
        m.screen.cursor_col = 7;
        m.exec_var(0x10, &[0x0200], None, None); // array at 0x0200
        assert_eq!(m.mem.read_word(0x0200), 3, "word 0 = row");
        assert_eq!(m.mem.read_word(0x0202), 7, "word 1 = col");
    }

    // print_table (VAR:0x1E)
    fn captured_output(m: &Machine) -> String {
        m.buffer_output().expect("default sink is BufferOutput").buf.clone()
    }

    #[test]
    fn print_table_emits_each_row_chars() {
        let mut m = build_test_machine(&[]);
        // 2x2 region of ASCII at 0x0200: "AB" / "CD"
        m.mem.write_byte(0x0200, b'A');
        m.mem.write_byte(0x0201, b'B');
        m.mem.write_byte(0x0202, b'C');
        m.mem.write_byte(0x0203, b'D');
        m.exec_var(0x1E, &[0x0200, 2, 2, 0], None, None); // width 2, height 2, skip 0
        let out = captured_output(&m);
        assert!(out.contains('A') && out.contains('B') && out.contains('C') && out.contains('D'),
            "all rectangle characters are printed");
    }

    #[test]
    fn unimplemented_var_opcode_is_warned_once() {
        let mut m = build_test_machine(&[]);
        // 0x15 sound_effect is intentionally unimplemented -> hits the fallthrough.
        assert!(m.warned_var_opcodes.is_empty());
        m.exec_var(0x15, &[], None, None);
        assert!(m.warned_var_opcodes.contains(&0x15), "fallthrough records the opcode");
        m.exec_var(0x15, &[], None, None); // second call must not duplicate
        assert_eq!(m.warned_var_opcodes.len(), 1, "warned at most once per opcode");
    }
}
