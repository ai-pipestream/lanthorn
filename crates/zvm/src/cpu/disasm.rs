//! Public Z-machine disassembler: decoded `Instr` → human-readable text.
//! Zero-dependency. Verified against ZMSD §14 + this crate's `execute()` dispatch.

use crate::cpu::decode::{decode, Branch, Instr, Operand, OperandCount};
use crate::memory::Memory;

/// Mnemonic for a decoded instruction, version-aware. Hex fallback (never panics)
/// for opcodes with no assigned name in this version.
pub fn mnemonic(count: &OperandCount, opcode: u8, version: u8) -> &'static str {
    match count {
        OperandCount::Two => match opcode {
            0x01 => "je", 0x02 => "jl", 0x03 => "jg", 0x04 => "dec_chk",
            0x05 => "inc_chk", 0x06 => "jin", 0x07 => "test", 0x08 => "or",
            0x09 => "and", 0x0A => "test_attr", 0x0B => "set_attr",
            0x0C => "clear_attr", 0x0D => "store", 0x0E => "insert_obj",
            0x0F => "loadw", 0x10 => "loadb", 0x11 => "get_prop",
            0x12 => "get_prop_addr", 0x13 => "get_next_prop", 0x14 => "add",
            0x15 => "sub", 0x16 => "mul", 0x17 => "div", 0x18 => "mod",
            0x19 => "call_2s", 0x1A => "call_2n", 0x1B => "set_colour",
            0x1C => "throw",
            _ => "op:2op",
        },
        OperandCount::One => match opcode {
            0x00 => "jz", 0x01 => "get_sibling", 0x02 => "get_child",
            0x03 => "get_parent", 0x04 => "get_prop_len", 0x05 => "inc",
            0x06 => "dec", 0x07 => "print_addr", 0x08 => "call_1s",
            0x09 => "remove_obj", 0x0A => "print_obj", 0x0B => "ret",
            0x0C => "jump", 0x0D => "print_paddr", 0x0E => "load",
            0x0F => if version >= 5 { "call_1n" } else { "not" },
            _ => "op:1op",
        },
        OperandCount::Zero => match opcode {
            0x00 => "rtrue", 0x01 => "rfalse", 0x02 => "print",
            0x03 => "print_ret", 0x04 => "nop", 0x05 => "save",
            0x06 => "restore", 0x07 => "restart", 0x08 => "ret_popped",
            0x09 => if version >= 5 { "catch" } else { "pop" },
            0x0A => "quit", 0x0B => "new_line", 0x0C => "show_status",
            0x0D => "verify", 0x0E => "extended", 0x0F => "piracy",
            _ => "op:0op",
        },
        OperandCount::Var => match opcode {
            0x00 => "call_vs", 0x01 => "storew", 0x02 => "storeb",
            0x03 => "put_prop", 0x04 => if version >= 5 { "aread" } else { "sread" },
            0x05 => "print_char", 0x06 => "print_num", 0x07 => "random",
            0x08 => "push", 0x09 => "pull", 0x0A => "split_window",
            0x0B => "set_window", 0x0C => "call_vs2", 0x0D => "erase_window",
            0x0E => "erase_line", 0x0F => "set_cursor", 0x10 => "get_cursor",
            0x11 => "set_text_style", 0x12 => "buffer_mode", 0x13 => "output_stream",
            0x14 => "input_stream", 0x15 => "sound_effect", 0x16 => "read_char",
            0x17 => "scan_table", 0x18 => "not", 0x19 => "call_vn",
            0x1A => "call_vn2", 0x1B => "tokenise", 0x1C => "encode_text",
            0x1D => "copy_table", 0x1E => "print_table", 0x1F => "check_arg_count",
            _ => "op:var",
        },
        OperandCount::Ext => match opcode {
            0x00 => "save", 0x01 => "restore", 0x02 => "log_shift",
            0x03 => "art_shift", 0x04 => "set_font", 0x05 => "draw_picture",
            0x06 => "picture_data", 0x07 => "erase_picture", 0x08 => "set_margins",
            0x09 => "save_undo", 0x0A => "restore_undo", 0x0B => "print_unicode",
            0x0C => "check_unicode", 0x0D => "set_true_colour", 0x10 => "move_window",
            0x11 => "window_size", 0x12 => "window_style", 0x13 => "get_wind_prop",
            0x14 => "scroll_window", 0x15 => "pop_stack", 0x16 => "read_mouse",
            0x17 => "mouse_window", 0x18 => "push_stack", 0x19 => "put_wind_prop",
            0x1A => "print_form", 0x1B => "make_menu", 0x1C => "picture_table",
            0x1D => "buffer_screen",
            _ => "op:ext",
        },
    }
}

fn fmt_operand(op: &Operand) -> String {
    match op {
        Operand::Large(n) => format!("#{:04x}", n),
        Operand::Small(n) => format!("#{:02x}", n),
        Operand::Var(0) => "sp".to_string(),
        Operand::Var(n @ 1..=15) => format!("local{}", n - 1),
        Operand::Var(n) => format!("g{:02x}", n - 16),
    }
}

fn fmt_var(v: u8) -> String {
    match v {
        0 => "sp".to_string(),
        1..=15 => format!("local{}", v - 1),
        n => format!("g{:02x}", n - 16),
    }
}

fn fmt_branch(b: &Branch, next_pc: u32) -> String {
    let neg = if b.on_true { "" } else { "~" };
    let target = match b.offset {
        0 => "rfalse".to_string(),
        1 => "rtrue".to_string(),
        off => format!("0x{:06x}", (next_pc as i64 + off as i64 - 2) as u32),
    };
    format!(" ?{}{}", neg, target)
}

/// Format one decoded instruction as "mnemonic op, op -> store ?branch [\"text\"]".
pub fn format_instr(instr: &Instr, version: u8) -> String {
    let mut s = mnemonic(&instr.operand_count, instr.opcode, version).to_string();
    if !instr.operands.is_empty() {
        let ops: Vec<String> = instr.operands.iter().map(fmt_operand).collect();
        s.push(' ');
        s.push_str(&ops.join(", "));
    }
    if let Some(v) = instr.store {
        s.push_str(&format!(" -> {}", fmt_var(v)));
    }
    if let Some(b) = &instr.branch {
        s.push_str(&fmt_branch(b, instr.next_pc));
    }
    if let Some((text, _)) = &instr.text {
        s.push_str(&format!(" {:?}", text));
    }
    s
}

/// Disassemble `lines` instructions starting at `start`, each prefixed with its
/// address. Stops early at the end of memory (never reads out of range).
pub fn disassemble(mem: &Memory, start: u32, version: u8, lines: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines);
    let mut pc = start.min(mem.len() as u32);
    for _ in 0..lines {
        if pc >= mem.len() as u32 {
            break;
        }
        let instr = decode(mem, pc, version);
        out.push(format!("{:06x}  {}", pc, format_instr(&instr, version)));
        // Guard against a decoder that fails to advance (malformed bytes).
        pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
    }
    out
}

/// Address of the instruction following the one at `addr` (clamped to memory).
pub fn next_instr(mem: &Memory, addr: u32, version: u8) -> u32 {
    if addr >= mem.len() as u32 {
        return mem.len() as u32;
    }
    let n = decode(mem, addr, version).next_pc;
    n.min(mem.len() as u32).max(addr + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::decode::OperandCount;

    #[test]
    fn mnemonics_cover_each_class() {
        assert_eq!(mnemonic(&OperandCount::Two, 0x14, 5), "add");
        assert_eq!(mnemonic(&OperandCount::Two, 0x0F, 5), "loadw");
        assert_eq!(mnemonic(&OperandCount::One, 0x00, 5), "jz");
        assert_eq!(mnemonic(&OperandCount::One, 0x0B, 5), "ret");
        assert_eq!(mnemonic(&OperandCount::Zero, 0x00, 5), "rtrue");
        assert_eq!(mnemonic(&OperandCount::Zero, 0x02, 5), "print");
        assert_eq!(mnemonic(&OperandCount::Var, 0x00, 5), "call_vs");
        assert_eq!(mnemonic(&OperandCount::Var, 0x04, 5), "aread");
        assert_eq!(mnemonic(&OperandCount::Var, 0x04, 3), "sread");
        assert_eq!(mnemonic(&OperandCount::One, 0x0F, 5), "call_1n");
        assert_eq!(mnemonic(&OperandCount::One, 0x0F, 4), "not");
        assert_eq!(mnemonic(&OperandCount::Ext, 0x09, 5), "save_undo");
        // Unknown falls back to a hex label, never panics.
        assert!(mnemonic(&OperandCount::Two, 0x7F, 5).starts_with("op:"));
    }

    #[test]
    fn formats_operands_store_and_branch() {
        use crate::cpu::decode::Form;
        // add local1, #05 -> sp   (2OP:0x14, store to var 0)
        let instr = Instr {
            opcode: 0x14, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Var(1), Operand::Small(5)],
            store: Some(0), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&instr, 5);
        assert!(s.starts_with("add "), "got {s:?}");
        assert!(s.contains("local0"), "got {s:?}");
        assert!(s.contains("#05"), "got {s:?}");
        assert!(s.contains("-> sp"), "got {s:?}");
    }

    #[test]
    fn disassembles_a_real_routine_without_all_fallbacks() {
        // Real story fixture — sample_story() has all-zero code memory, which
        // would trivially fall back and defeat the point of this oracle test.
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let start = mem.initial_pc();
        let lines = disassemble(&mem, start, mem.version(), 8);
        assert!(!lines.is_empty());
        assert!(lines.iter().any(|l| !l.contains("op:")), "all fallbacks: {lines:?}");
    }
}
