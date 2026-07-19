//! Disassembly cache: an ordered model of the code region as display units.
//!
//! Task 1 scope only: the `Unit`/`DisasmCache` data model, code-region bounds,
//! and an empty-cache skeleton constructor. Routine discovery (populating
//! `units`/`routines`) lands in later tasks.

use crate::cpu::decode::{decode, Operand, OperandCount};
use crate::cpu::disasm::Unpack;
use crate::memory::Memory;
use std::collections::{BTreeSet, HashSet};

/// A single displayable unit within the disassembled code region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    /// A decoded instruction spanning `[addr, next)`.
    Instr { addr: u32, next: u32 },
    /// A routine header spanning `[addr, first_instr)` (ZMSD §5.2: one byte of
    /// local count, then `nlocals` initial-value words in v1-4).
    RoutineHeader { addr: u32, nlocals: u8, first_instr: u32 },
    /// An opaque data run spanning `[addr, addr+len)` (not decoded as code).
    Data { addr: u32, len: u32 },
}

impl Unit {
    /// Start address of this unit.
    pub fn addr(&self) -> u32 {
        match *self {
            Unit::Instr { addr, .. } => addr,
            Unit::RoutineHeader { addr, .. } => addr,
            Unit::Data { addr, .. } => addr,
        }
    }

    /// One-past-the-end address (exclusive). `RoutineHeader` occupies
    /// `[addr, first_instr)`.
    pub fn end(&self) -> u32 {
        match *self {
            Unit::Instr { next, .. } => next,
            Unit::RoutineHeader { first_instr, .. } => first_instr,
            Unit::Data { addr, len } => addr + len,
        }
    }
}

/// Rendering detail level for a cached unit (mirrors `disasm`'s
/// full/basic/raw views).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CacheFmt {
    Full,
    Basic,
    Raw,
}

/// Ordered cache of the code region, tiled by `Unit`s with routine entry
/// points tracked separately for fast lookup.
pub struct DisasmCache {
    /// Sorted by `addr()`, tiling `[region_start, region_end)` with no gaps
    /// once discovery (later tasks) has populated it.
    units: Vec<Unit>,
    /// Routine ENTRY (header) addresses, populated by discovery.
    routines: std::collections::BTreeSet<u32>,
    region_start: u32,
    region_end: u32,
    version: u8,
    unpack: Unpack,
}

impl DisasmCache {
    /// Build an empty cache with region bounds computed but no units
    /// discovered yet. Later tasks populate `units`/`routines`.
    pub fn empty(mem: &Memory) -> DisasmCache {
        let (region_start, region_end) = code_region(mem);
        DisasmCache {
            units: Vec::new(),
            routines: Default::default(),
            region_start,
            region_end,
            version: mem.version(),
            unpack: Unpack::from_mem(mem),
        }
    }

    /// Code-region bounds this cache was built for: `(region_start, region_end)`.
    pub fn region(&self) -> (u32, u32) {
        (self.region_start, self.region_end)
    }

    /// The cached units, in address order. Empty until discovery (later tasks)
    /// populates it.
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// Routine entry (header) addresses discovered so far.
    pub fn routines(&self) -> &std::collections::BTreeSet<u32> {
        &self.routines
    }

    /// Z-machine version this cache was built for.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Packed-address unpacking context this cache was built for.
    pub fn unpack(&self) -> &Unpack {
        &self.unpack
    }
}

/// Code-region bounds: `(region_start, region_end)`.
///
/// `region_start` = `min(high_mem_base, initial_pc)`; `region_end` =
/// `mem.len()`. Permissive by design — discovery/validation in later tasks
/// reject non-code content within these bounds.
pub fn code_region(mem: &Memory) -> (u32, u32) {
    let high_mem_base = mem.read_word(0x04) as u32;
    let initial_pc = mem.read_word(0x06) as u32;
    let region_start = high_mem_base.min(initial_pc);
    let region_end = mem.len() as u32;
    (region_start, region_end)
}

/// First-instruction address of a routine whose header is at `entry`:
/// `entry + 1` (locals-count byte) `+ nlocals*2` initial-value words in v1-4
/// (ZMSD §5.2); v5+ routines carry no initial-value words.
///
/// Consumed by [`discover_rd`] and by tiling in Task 3.
pub(crate) fn routine_first_instr(mem: &Memory, entry: u32, version: u8) -> u32 {
    let nlocals = mem.read_byte(entry) as u32;
    entry + 1 + if version <= 4 { nlocals * 2 } else { 0 }
}

/// Is `(operand_count, opcode)` a `call*` opcode (whose routine operand is
/// operand index 0)? The v1-4 1OP:0x0F encoding is `not`, not `call_1n`, so
/// that case is gated on `version >= 5`.
fn is_call(count: OperandCount, opcode: u8, version: u8) -> bool {
    use OperandCount::*;
    match (count, opcode) {
        (One, 0x08) => true,          // call_1s
        (One, 0x0F) => version >= 5,  // call_1n (v5+); `not` in v1-4
        (Two, 0x19) => true,          // call_2s
        (Two, 0x1A) => true,          // call_2n
        (Var, 0x00) => true,          // call_vs
        (Var, 0x0C) => true,          // call_vs2
        (Var, 0x19) => true,          // call_vn
        (Var, 0x1A) => true,          // call_vn2
        _ => false,
    }
}

/// Does `(operand_count, opcode)` unconditionally end a routine's linear run?
fn is_terminator(count: OperandCount, opcode: u8) -> bool {
    use OperandCount::*;
    match (count, opcode) {
        (One, 0x0B) => true,  // ret
        (One, 0x0C) => true,  // jump (unconditional)
        (Zero, 0x00) => true, // rtrue
        (Zero, 0x01) => true, // rfalse
        (Zero, 0x03) => true, // print_ret
        (Zero, 0x08) => true, // ret_popped
        (Zero, 0x0A) => true, // quit
        _ => false,
    }
}

/// Recursive-descent routine discovery. Seeded from the initial-PC
/// first-instruction address, decode each routine forward and follow every
/// `call*` whose routine operand is a non-zero constant, to a fixpoint.
/// Returns the set of routine ENTRY (header) addresses discovered.
///
/// The initial "main" context has no header, so the seed is enqueued on the
/// first-instruction worklist directly and never added to the routine set.
///
/// Consumed by Task 3 (tiling/nav).
pub fn discover_rd(mem: &Memory, version: u8, unpack: &Unpack, region: (u32, u32)) -> BTreeSet<u32> {
    /// Safety cap on instructions decoded per routine run (guards a malformed
    /// decode loop that never reaches a terminator or the region end).
    const RUN_INSTR_CAP: u32 = 4096;

    let (rstart, rend) = region;
    let mut routines: BTreeSet<u32> = BTreeSet::new();
    let mut visited: HashSet<u32> = HashSet::new();

    let initial_pc = mem.read_word(0x06) as u32;
    let mut worklist: Vec<u32> = vec![initial_pc];

    while let Some(first) = worklist.pop() {
        if !visited.insert(first) {
            continue;
        }
        let mut pc = first;
        let mut steps = 0u32;
        loop {
            if pc >= rend || steps >= RUN_INSTR_CAP {
                break;
            }
            steps += 1;
            let instr = decode(mem, pc, version);

            if is_call(instr.operand_count.clone(), instr.opcode, version) {
                let target = match instr.operands.first() {
                    Some(Operand::Large(n)) => Some(*n),
                    Some(Operand::Small(n)) => Some(*n as u16),
                    _ => None,
                };
                if let Some(n) = target {
                    if n != 0 {
                        let entry = unpack.routine(n);
                        if entry >= rstart && entry < rend {
                            let fi = routine_first_instr(mem, entry, version);
                            if fi < rend && routines.insert(entry) {
                                worklist.push(fi);
                            }
                        }
                    }
                }
            }

            if is_terminator(instr.operand_count.clone(), instr.opcode) {
                break;
            }
            pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
        }
    }

    routines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_region_bounds_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let (start, end) = code_region(&mem);
        let initial_pc = mem.read_word(0x06) as u32;
        assert!(start <= initial_pc, "region_start {start:#x} > initial_pc {initial_pc:#x}");
        assert!(start < end, "region_start {start:#x} >= region_end {end:#x}");
        assert_eq!(end, mem.len() as u32);
    }

    #[test]
    fn code_region_bounds_on_synthetic_story() {
        // sample_story() header: high_mem_base=0x0400, initial_pc=0x0040, len=0x0400.
        let bytes = crate::header::tests_support::sample_story(5);
        let mem = Memory::new(bytes).unwrap();
        let (start, end) = code_region(&mem);
        let initial_pc = mem.read_word(0x06) as u32;
        assert!(start <= initial_pc);
        assert!(start < end);
        assert_eq!(end, mem.len() as u32);
    }

    #[test]
    fn empty_cache_reports_the_region_it_was_built_for() {
        let bytes = crate::header::tests_support::sample_story(5);
        let mem = Memory::new(bytes).unwrap();
        let expected = code_region(&mem);
        let cache = DisasmCache::empty(&mem);
        assert_eq!(cache.region(), expected);
        assert!(cache.units().is_empty());
        assert!(cache.routines().is_empty());
        assert_eq!(cache.version(), mem.version());
        assert_eq!(cache.unpack().version, mem.version());
    }

    #[test]
    fn unit_addr_and_end_for_instr() {
        let u = Unit::Instr { addr: 10, next: 14 };
        assert_eq!(u.addr(), 10);
        assert_eq!(u.end(), 14);
    }

    #[test]
    fn unit_addr_and_end_for_routine_header() {
        let u = Unit::RoutineHeader { addr: 20, nlocals: 3, first_instr: 27 };
        assert_eq!(u.addr(), 20);
        assert_eq!(u.end(), 27);
    }

    #[test]
    fn unit_addr_and_end_for_data() {
        let u = Unit::Data { addr: 30, len: 8 };
        assert_eq!(u.addr(), 30);
        assert_eq!(u.end(), 38);
    }

    #[test]
    fn is_call_classifies_call_opcodes() {
        use OperandCount::*;
        assert!(is_call(Var, 0x00, 3), "call_vs");
        assert!(!is_call(One, 0x0F, 3), "1OP:0x0F is `not` in v3");
        assert!(is_call(One, 0x0F, 5), "1OP:0x0F is call_1n in v5");
        assert!(is_call(Two, 0x1A, 5), "call_2n");
        assert!(is_call(One, 0x08, 5), "call_1s");
    }

    #[test]
    fn is_terminator_classifies_terminators() {
        use OperandCount::*;
        assert!(is_terminator(One, 0x0B), "ret");
        assert!(is_terminator(Zero, 0x0A), "quit");
        assert!(!is_terminator(Two, 0x14), "add is not a terminator");
    }

    #[test]
    fn routine_first_instr_v3_skips_local_words() {
        // v3 (<= 4): first instr = entry + 1 + nlocals*2.
        let bytes = crate::header::tests_support::sample_story(3);
        let mut mem = Memory::new(bytes).unwrap();
        let entry = 0x0080u32;
        let nlocals = 3u8;
        mem.write_byte(entry, nlocals);
        assert_eq!(
            routine_first_instr(&mem, entry, 3),
            entry + 1 + (nlocals as u32) * 2
        );
    }

    #[test]
    fn routine_first_instr_v5_has_no_local_words() {
        // v5+ carries no initial-value words: first instr = entry + 1.
        let bytes = crate::header::tests_support::sample_story(5);
        let mut mem = Memory::new(bytes).unwrap();
        let entry = 0x0080u32;
        mem.write_byte(entry, 7);
        assert_eq!(routine_first_instr(&mem, entry, 5), entry + 1);
    }

    #[test]
    fn discover_rd_finds_routines_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);

        let routines = discover_rd(&mem, version, &unpack, region);
        assert!(!routines.is_empty(), "RD found no routines");
        for &entry in &routines {
            assert!(
                entry >= region.0 && entry < region.1,
                "entry {entry:#x} outside region {region:#x?}"
            );
            let nlocals = mem.read_byte(entry);
            assert!(nlocals <= 15, "entry {entry:#x} locals byte {nlocals} > 15");
        }
    }

    #[test]
    fn discover_rd_follows_the_first_constant_call() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);

        // Independently decode forward from initial_pc until the first `call*`
        // with a constant routine operand, and unpack its target ourselves.
        let mut pc = mem.read_word(0x06) as u32;
        let mut expected: Option<u32> = None;
        for _ in 0..4096 {
            if pc >= region.1 {
                break;
            }
            let instr = decode(&mem, pc, version);
            if is_call(instr.operand_count.clone(), instr.opcode, version) {
                let n = match instr.operands.first() {
                    Some(Operand::Large(n)) => Some(*n),
                    Some(Operand::Small(n)) => Some(*n as u16),
                    _ => None,
                };
                if let Some(n) = n {
                    if n != 0 {
                        expected = Some(unpack.routine(n));
                        break;
                    }
                }
            }
            if is_terminator(instr.operand_count.clone(), instr.opcode) {
                break;
            }
            pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
        }

        let expected = expected.expect("no constant call reachable from initial_pc");
        let routines = discover_rd(&mem, version, &unpack, region);
        assert!(
            routines.contains(&expected),
            "RD did not include first-call target {expected:#x}"
        );
    }
}
