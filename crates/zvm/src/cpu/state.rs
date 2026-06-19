// Z-machine execution state — ZMSD §6.
//
// Manages the evaluation stack, call frames, local/global variables,
// and routine call/return mechanics.

use crate::memory::Memory;

/// A single call frame on the Z-machine call stack.
#[derive(Debug)]
pub struct Frame {
    /// PC to restore when this routine returns.
    pub return_pc: u32,
    /// Local variables for this routine (0-indexed: local 1 is locals[0]).
    pub locals: Vec<u16>,
    /// Base index into the shared eval_stack for this frame's region.
    pub eval_base: usize,
    /// Variable number to store the return value into, or None to discard.
    pub store_var: Option<u8>,
    /// Number of arguments passed to this routine.
    pub arg_count: u8,
}

/// Z-machine interpreter execution state.
#[derive(Debug)]
pub struct State {
    pub pc: u32,
    pub frames: Vec<Frame>,
    pub eval_stack: Vec<u16>,
}

impl State {
    pub fn new(pc: u32) -> State {
        State {
            pc,
            frames: Vec::new(),
            eval_stack: Vec::new(),
        }
    }
}

/// Read variable `var` from state/memory.
///
/// - var 0x00: pop from the current frame's eval stack region
/// - var 0x01–0x0F: local variable (1-based index into current frame's locals)
/// - var 0x10–0xFF: global variable from dynamic memory
pub fn read_var(state: &mut State, mem: &Memory, var: u8) -> u16 {
    match var {
        0x00 => {
            // Pop from current frame's eval stack region
            let base = state.frames.last().map(|f| f.eval_base).unwrap_or(0);
            assert!(state.eval_stack.len() > base, "eval stack underflow");
            state.eval_stack.pop().unwrap()
        }
        0x01..=0x0F => {
            let idx = (var - 1) as usize;
            let frame = state.frames.last().expect("no current frame");
            frame.locals[idx]
        }
        _ => {
            // Global variable: 0x10 is global 0, stored at global_vars + 2*(var-0x10)
            let base = mem.global_vars() as u32;
            let offset = (var - 0x10) as u32 * 2;
            mem.read_word(base + offset)
        }
    }
}

/// Write value `val` to variable `var` in state/memory.
///
/// - var 0x00: push onto the current frame's eval stack region
/// - var 0x01–0x0F: local variable (1-based index into current frame's locals)
/// - var 0x10–0xFF: global variable in dynamic memory
pub fn write_var(state: &mut State, mem: &mut Memory, var: u8, val: u16) {
    match var {
        0x00 => {
            state.eval_stack.push(val);
        }
        0x01..=0x0F => {
            let idx = (var - 1) as usize;
            let frame = state.frames.last_mut().expect("no current frame");
            frame.locals[idx] = val;
        }
        _ => {
            let base = mem.global_vars() as u32;
            let offset = (var - 0x10) as u32 * 2;
            mem.write_word(base + offset, val);
        }
    }
}

/// Call a routine at `packed_addr` with `args`, storing the return value into
/// `store_var` when the routine eventually returns (or None to discard).
///
/// Packed address 0 is special: do nothing and store 0 into `store_var`
/// immediately (ZMSD §6.4.3).
pub fn call_routine(
    state: &mut State,
    mem: &mut Memory,
    packed_addr: u16,
    args: &[u16],
    store_var: Option<u8>,
) {
    if packed_addr == 0 {
        // Special case: return false/0 immediately without pushing a frame
        if let Some(sv) = store_var {
            write_var(state, mem, sv, 0);
        }
        return;
    }

    let routine_addr = mem.unpack_routine(packed_addr);
    let local_count = mem.read_byte(routine_addr) as usize;
    assert!(local_count <= 15, "local count must be 0..=15");

    // Read initial local values (v1–4 only; v5+ locals initialise to 0)
    let mut locals: Vec<u16> = if mem.version() <= 4 {
        // Each local has a 2-byte initial value word in the routine header
        (0..local_count)
            .map(|i| mem.read_word(routine_addr + 1 + (i as u32) * 2))
            .collect()
    } else {
        vec![0u16; local_count]
    };

    // Copy call arguments over the first locals (extra args are discarded)
    for (i, &arg) in args.iter().enumerate() {
        if i < local_count {
            locals[i] = arg;
        }
    }

    // First instruction: after the count byte + initial-value words (v3/v4 only)
    let first_instruction = if mem.version() <= 4 {
        routine_addr + 1 + (local_count as u32) * 2
    } else {
        routine_addr + 1
    };

    let return_pc = state.pc;
    let eval_base = state.eval_stack.len();

    state.frames.push(Frame {
        return_pc,
        locals,
        eval_base,
        store_var,
        arg_count: args.len().min(255) as u8,
    });

    state.pc = first_instruction;
}

/// Return `val` from the current routine: pop the frame, truncate the eval
/// stack to the frame's base, store `val` into the frame's `store_var`, and
/// restore PC to the frame's `return_pc`.
pub fn return_value(state: &mut State, mem: &mut Memory, val: u16) {
    let frame = state.frames.pop().expect("return with no active frame");

    // Discard any eval stack entries belonging to this frame
    state.eval_stack.truncate(frame.eval_base);

    // Restore PC
    state.pc = frame.return_pc;

    // Store return value if the call requested it
    if let Some(sv) = frame.store_var {
        write_var(state, mem, sv, val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;

    // Test (a) from brief: locals and globals round-trip
    #[test]
    fn locals_and_globals_round_trip() {
        let mut m = Memory::new(sample_story(5)).unwrap();
        let mut st = State::new(0x0010);
        st.frames.push(Frame {
            return_pc: 0,
            locals: vec![0, 0, 0],
            eval_base: 0,
            store_var: None,
            arg_count: 0,
        });
        write_var(&mut st, &mut m, 0x02, 0xABCD); // local 2
        assert_eq!(read_var(&mut st, &m, 0x02), 0xABCD);
        write_var(&mut st, &mut m, 0x10, 0x1234); // global 0
        assert_eq!(read_var(&mut st, &m, 0x10), 0x1234);
    }

    // Test (b) from brief: stack push/pop via var 0
    #[test]
    fn stack_push_pop_via_var_zero() {
        let mut m = Memory::new(sample_story(5)).unwrap();
        let mut st = State::new(0x10);
        st.frames.push(Frame {
            return_pc: 0,
            locals: vec![],
            eval_base: 0,
            store_var: None,
            arg_count: 0,
        });
        write_var(&mut st, &mut m, 0x00, 0x0042);
        assert_eq!(read_var(&mut st, &m, 0x00), 0x0042);
    }

    // Test (c): call_routine on a v3 routine with initial-value locals
    #[test]
    fn call_routine_v3_initial_values() {
        // Build a v3 story with a routine at 0x0040 (packed addr = 0x0020 for v3)
        // routine header: 2 locals, initial values 0xAAAA and 0xBBBB
        let mut buf = sample_story(3);
        // Place routine at byte 0x0040
        buf[0x40] = 2; // local_count = 2
        buf[0x41] = 0xAA; buf[0x42] = 0xAA; // local 1 initial = 0xAAAA
        buf[0x43] = 0xBB; buf[0x44] = 0xBB; // local 2 initial = 0xBBBB
        // (first instruction would be at 0x45)

        let mut m = Memory::new(buf).unwrap();
        let mut st = State::new(0x1000); // arbitrary return PC

        // packed_addr 0x0020 → 2 * 0x0020 = 0x0040 for v3
        call_routine(&mut st, &mut m, 0x0020, &[0xDEAD], None);

        let frame = st.frames.last().unwrap();
        assert_eq!(frame.locals[0], 0xDEAD, "arg 1 overwrites local 1");
        assert_eq!(frame.locals[1], 0xBBBB, "local 2 keeps initial value");
        // PC should be at first instruction: 0x40 + 1 + 2*2 = 0x45
        assert_eq!(st.pc, 0x45);
    }

    // Test (d): v5 routine — locals are zero, no initial-value words
    #[test]
    fn call_routine_v5_zero_locals() {
        let mut buf = sample_story(5);
        // Place routine at 0x0040 (packed addr for v5: 4 * packed = addr → packed = 0x0010)
        buf[0x40] = 2; // local_count = 2 (no initial-value words for v5)
        // (first instruction at 0x41)

        let mut m = Memory::new(buf).unwrap();
        let mut st = State::new(0x2000);

        // packed_addr 0x0010 → 4 * 0x0010 = 0x0040 for v5
        call_routine(&mut st, &mut m, 0x0010, &[0x1234], None);

        let frame = st.frames.last().unwrap();
        assert_eq!(frame.locals[0], 0x1234, "arg 1 sets local 1");
        assert_eq!(frame.locals[1], 0x0000, "local 2 is zero-initialized");
        // PC at first instruction: 0x40 + 1 = 0x41
        assert_eq!(st.pc, 0x41);
    }

    // Test (e): return_value stores into caller's store_var, restores PC, truncates stack
    #[test]
    fn return_value_restores_state() {
        let mut m = Memory::new(sample_story(5)).unwrap();
        let mut st = State::new(0xFFFF);

        // Push a "caller" frame whose store_var is local 1 of a notional outer frame
        // For simplicity: use a global var (0x10) as store target
        st.frames.push(Frame {
            return_pc: 0x1234,
            locals: vec![],
            eval_base: 0,
            store_var: Some(0x10), // store into global 0
            arg_count: 0,
        });

        // Push some eval stack entries for this frame
        st.eval_stack.push(0xAAAA);
        st.eval_stack.push(0xBBBB);

        // Call into a "callee" at some PC
        st.pc = 0x5000;

        // Return 0x9999 from callee
        return_value(&mut st, &mut m, 0x9999);

        // PC restored to caller's return_pc
        assert_eq!(st.pc, 0x1234);
        // Eval stack truncated to eval_base (0)
        assert_eq!(st.eval_stack.len(), 0);
        // Return value stored into global 0
        assert_eq!(m.read_word(0x0300), 0x9999);
    }

    // Test (f): packed address 0 → stores 0, no frame pushed
    #[test]
    fn call_packed_addr_zero_returns_false() {
        let mut m = Memory::new(sample_story(5)).unwrap();
        let mut st = State::new(0x1000);
        let initial_pc = st.pc;

        // No frames — we need a frame for write_var to a local, so use a global store_var
        call_routine(&mut st, &mut m, 0, &[], Some(0x10)); // store result in global 0

        // No frame should have been pushed
        assert_eq!(st.frames.len(), 0);
        // PC unchanged
        assert_eq!(st.pc, initial_pc);
        // Global 0 should hold 0
        assert_eq!(m.read_word(0x0300), 0x0000);
    }

    // Additional test: multiple globals round-trip
    #[test]
    fn multiple_globals_round_trip() {
        let mut m = Memory::new(sample_story(5)).unwrap();
        let mut st = State::new(0x10);
        // We need a frame for write_var signature but globals don't need one
        // Just to be safe, push a dummy frame
        st.frames.push(Frame {
            return_pc: 0,
            locals: vec![],
            eval_base: 0,
            store_var: None,
            arg_count: 0,
        });
        write_var(&mut st, &mut m, 0x10, 0x0001); // global 0
        write_var(&mut st, &mut m, 0x11, 0x0002); // global 1
        // global_vars base = 0x0300, static_mem_base = 0x0400 → 0x80 words available
        // highest safe global: 0x10 + 0x7F = 0x8F
        write_var(&mut st, &mut m, 0x8F, 0x00EF); // global 0x7F (last safe in sample)
        assert_eq!(read_var(&mut st, &m, 0x10), 0x0001);
        assert_eq!(read_var(&mut st, &m, 0x11), 0x0002);
        assert_eq!(read_var(&mut st, &m, 0x8F), 0x00EF);
    }
}
