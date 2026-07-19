//! Disassembly cache: an ordered model of the code region as display units.
//!
//! Task 1 scope only: the `Unit`/`DisasmCache` data model, code-region bounds,
//! and an empty-cache skeleton constructor. Routine discovery (populating
//! `units`/`routines`) lands in later tasks.

use crate::cpu::disasm::Unpack;
use crate::memory::Memory;

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
}
