//! Native implementations of the 13 well-known Glulx accelerated functions
//! (`@accelfunc`). See docs/superpowers/plans/2026-07-02-glulx-acceleration-algorithms.md
//! (authoritative) and the Glulx spec §2.17 / Glulxe accel.c.

use crate::exec::{Machine, R};

/// True iff `num` names an accelerated function this VM implements (1..=13).
pub(crate) fn accel_impl_supported(num: u32) -> bool {
    (1..=13).contains(&num)
}

impl Machine {
    /// Run accelerated function `num` (assumed 1..=13) with `args`, returning its
    /// value. Never builds a frame; a memory fault propagates as an interpreter error.
    pub(crate) fn accel_dispatch(&self, num: u32, args: &[u32]) -> R<u32> {
        match num {
            1 => self.accel_z_region(args),
            _ => Ok(0), // 2..=13 filled in by Task 3
        }
    }

    #[inline]
    fn accel_arg(args: &[u32], i: usize) -> u32 {
        args.get(i).copied().unwrap_or(0)
    }

    #[inline]
    #[allow(dead_code)] // used by Task 3's per-function implementations
    fn accel_param_or0(&self, i: u32) -> u32 {
        self.accel_param(i).unwrap_or(0)
    }

    /// Function 1 — Z__Region.
    fn accel_z_region(&self, args: &[u32]) -> R<u32> {
        let addr = Self::accel_arg(args, 0);
        if addr < 36 || addr >= self.mem.endmem() {
            return Ok(0);
        }
        let tb = self.m8(addr)?;
        Ok(if tb >= 0xE0 {
            3
        } else if tb >= 0xC0 {
            2
        } else if (0x70..=0x7F).contains(&tb) && addr >= self.mem.ramstart() {
            1
        } else {
            0
        })
    }

    // Task 3 adds: accel_cp_tab, accel_ra_pr, accel_rl_pr, accel_oc_cl,
    // accel_rv_pr, accel_op_pr, and the obj_in_class / get_prop / binsearch helpers.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm;
    use crate::glk::TestBackend;
    use crate::memory::Memory;

    /// A machine whose start function is empty, with `ram_bytes` of RAM
    /// (RAMSTART == 0x100, per `asm::assemble`'s tiny-code layout).
    fn accel_test_machine() -> (Machine, u32, u32, u32) {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");

        let obj_addr = 0x100;
        let routine_addr = 0x104;
        let string_addr = 0x108;
        m.mem.write8(obj_addr, 0x70).unwrap();
        m.mem.write8(routine_addr, 0xC0).unwrap();
        m.mem.write8(string_addr, 0xE0).unwrap();

        (m, obj_addr, routine_addr, string_addr)
    }

    #[test]
    fn z_region_classifies_addresses() {
        let (m, obj_addr, routine_addr, string_addr) = accel_test_machine();
        assert_eq!(m.accel_dispatch(1, &[10]).unwrap(), 0); // addr < 36
        assert_eq!(m.accel_dispatch(1, &[obj_addr]).unwrap(), 1); // object
        assert_eq!(m.accel_dispatch(1, &[routine_addr]).unwrap(), 2); // routine
        assert_eq!(m.accel_dispatch(1, &[string_addr]).unwrap(), 3); // string
        assert_eq!(m.accel_dispatch(1, &[]).unwrap(), 0); // no arg -> addr 0
    }
}
