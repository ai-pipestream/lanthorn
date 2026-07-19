//! Z-machine debug inspector (tiled pane) — panel state + navigation logic.
//! Pure over the `Debugger` trait (engine-neutral); the render code paints the
//! snapshot this holds. No `zvm::` calls here.

use crate::engine::Debugger;
use crossterm::event::KeyCode;

/// How many instructions / memory rows to pre-render for the address-windowed
/// panes (draw clips to the pane height; over-computing avoids threading height
/// into refresh).
pub const DISASM_WINDOW: usize = 256;
pub const MEM_WINDOW: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugView { Execution, WorldState }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugPane { Disasm, Locals, Stack, Globals, Objects, Dict, Memory }

impl DebugPane {
    /// Cycle order across both views (Tab walks this; rollover is implicit).
    const ORDER: [DebugPane; 7] = [
        DebugPane::Disasm, DebugPane::Locals, DebugPane::Stack,   // Execution
        DebugPane::Globals, DebugPane::Objects, DebugPane::Dict, DebugPane::Memory, // WorldState
    ];
    pub fn view(self) -> DebugView {
        match self {
            DebugPane::Disasm | DebugPane::Locals | DebugPane::Stack => DebugView::Execution,
            _ => DebugView::WorldState,
        }
    }
    fn cycle(self, dir: i32) -> DebugPane {
        let idx = Self::ORDER.iter().position(|&p| p == self).unwrap() as i32;
        let n = Self::ORDER.len() as i32;
        Self::ORDER[(idx + dir).rem_euclid(n) as usize]
    }
}

/// The formatted lines the render code paints, refreshed from the Debugger.
#[derive(Debug, Default, Clone)]
pub struct DebugSnapshot {
    pub disasm: Vec<String>,
    pub locals: Vec<String>,
    pub stack: Vec<String>,
    pub globals: Vec<String>,
    pub objects: Vec<String>,
    pub dict: Vec<String>,
    pub memory: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DebugPanelState {
    pub focus: DebugPane,
    pub disasm_addr: u32,
    pub disasm_history: Vec<u32>,
    pub mem_addr: u32,
    /// Scroll offset for the list panes (locals/stack/globals/objects/dict).
    pub list_scroll: usize,
    /// Focused-pane height captured by the last draw (for paging). 1 until drawn.
    pub viewport: usize,
    pub snapshot: DebugSnapshot,
}

/// Result of a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugKey { Consumed, Ignored, Close }

impl DebugPanelState {
    pub fn new(pc: u32) -> Self {
        DebugPanelState {
            focus: DebugPane::Disasm,
            disasm_addr: pc,
            disasm_history: Vec::new(),
            mem_addr: 0,
            list_scroll: 0,
            viewport: 1,
            snapshot: DebugSnapshot::default(),
        }
    }

    /// Recompute the snapshot for the current cursor positions.
    pub fn refresh(&mut self, dbg: &dyn Debugger) {
        self.snapshot.disasm = dbg.disassemble(self.disasm_addr, DISASM_WINDOW);
        self.snapshot.locals = dbg.locals_lines();
        self.snapshot.stack = dbg.stack_lines();
        self.snapshot.globals = dbg.globals_lines();
        self.snapshot.objects = dbg.object_tree_lines();
        self.snapshot.dict = dbg.dictionary_lines();
        self.snapshot.memory = dbg.memory_hex(self.mem_addr, MEM_WINDOW);
    }

    fn page(&self) -> usize { self.viewport.max(1) }

    pub fn handle_key(&mut self, code: KeyCode, dbg: &dyn Debugger) -> DebugKey {
        match code {
            KeyCode::Esc => return DebugKey::Close,
            KeyCode::Tab => { self.focus = self.focus.cycle(1); self.list_scroll = 0; }
            KeyCode::BackTab => { self.focus = self.focus.cycle(-1); self.list_scroll = 0; }
            KeyCode::Char('g') => {
                self.disasm_history.clear();
                self.disasm_addr = dbg.pc();
            }
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp
            | KeyCode::Home | KeyCode::End => {
                let step = matches!(code, KeyCode::PageDown | KeyCode::PageUp)
                    .then(|| self.page()).unwrap_or(1);
                let down = matches!(code, KeyCode::Down | KeyCode::PageDown | KeyCode::End);
                match self.focus {
                    DebugPane::Disasm => self.scroll_disasm(down, step, dbg),
                    DebugPane::Memory => self.scroll_memory(down, step, dbg),
                    _ => self.scroll_list(code),
                }
            }
            _ => return DebugKey::Ignored,
        }
        self.refresh(dbg);
        DebugKey::Consumed
    }

    fn scroll_disasm(&mut self, down: bool, step: usize, dbg: &dyn Debugger) {
        for _ in 0..step {
            if down {
                let next = dbg.next_instr(self.disasm_addr);
                if next > self.disasm_addr {
                    self.disasm_history.push(self.disasm_addr);
                    self.disasm_addr = next;
                }
            } else if let Some(prev) = self.disasm_history.pop() {
                self.disasm_addr = prev;
            }
        }
    }

    fn scroll_memory(&mut self, down: bool, step: usize, dbg: &dyn Debugger) {
        let delta = (16 * step) as u32;
        if down {
            let max = dbg.memory_len().saturating_sub(16);
            self.mem_addr = (self.mem_addr + delta).min(max);
        } else {
            self.mem_addr = self.mem_addr.saturating_sub(delta);
        }
    }

    fn scroll_list(&mut self, code: KeyCode) {
        let len = self.focused_list_len();
        let vp = self.page();
        let max = len.saturating_sub(1);
        self.list_scroll = match code {
            KeyCode::Down => (self.list_scroll + 1).min(max),
            KeyCode::Up => self.list_scroll.saturating_sub(1),
            KeyCode::PageDown => (self.list_scroll + vp).min(max),
            KeyCode::PageUp => self.list_scroll.saturating_sub(vp),
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => self.list_scroll,
        };
    }

    fn focused_list_len(&self) -> usize {
        match self.focus {
            DebugPane::Locals => self.snapshot.locals.len(),
            DebugPane::Stack => self.snapshot.stack.len(),
            DebugPane::Globals => self.snapshot.globals.len(),
            DebugPane::Objects => self.snapshot.objects.len(),
            DebugPane::Dict => self.snapshot.dict.len(),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    // Minimal mock: 4-byte fixed instructions, 0x10000 bytes of memory.
    struct MockDbg;
    impl crate::engine::Debugger for MockDbg {
        fn pc(&self) -> u32 { 0x1000 }
        fn disassemble(&self, addr: u32, n: usize) -> Vec<String> {
            (0..n).map(|i| format!("{:06x}  add", addr + i as u32 * 4)).collect()
        }
        fn next_instr(&self, a: u32) -> u32 { a + 4 }
        fn stack_lines(&self) -> Vec<String> { vec!["#0 main".into()] }
        fn locals_lines(&self) -> Vec<String> { vec!["(none)".into()] }
        fn globals_lines(&self) -> Vec<String> { (0..240).map(|i| format!("g{i:02x}")).collect() }
        fn object_tree_lines(&self) -> Vec<String> { vec!["[1] thing".into()] }
        fn dictionary_lines(&self) -> Vec<String> { vec!["word".into()] }
        fn memory_hex(&self, a: u32, r: usize) -> Vec<String> {
            (0..r).map(|i| format!("{:06x}", a + i as u32 * 16)).collect()
        }
        fn memory_len(&self) -> u32 { 0x10000 }
    }

    #[test]
    fn tab_cycles_focus_with_view_rollover_and_shift_tab_reverses() {
        let mut p = DebugPanelState::new(0x1000);
        assert_eq!(p.focus, DebugPane::Disasm);
        assert_eq!(p.focus.view(), DebugView::Execution);
        p.handle_key(KeyCode::Tab, &MockDbg); // -> Locals
        p.handle_key(KeyCode::Tab, &MockDbg); // -> Stack
        p.handle_key(KeyCode::Tab, &MockDbg); // -> Globals (rolls into WorldState)
        assert_eq!(p.focus, DebugPane::Globals);
        assert_eq!(p.focus.view(), DebugView::WorldState);
        p.handle_key(KeyCode::BackTab, &MockDbg); // back to Stack
        assert_eq!(p.focus, DebugPane::Stack);
    }

    #[test]
    fn disasm_scroll_advances_by_instruction_and_up_pops_history() {
        let mut p = DebugPanelState::new(0x1000);
        // focus is Disasm by default
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1008);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1000);
        p.handle_key(KeyCode::Up, &MockDbg); // history empty -> no-op
        assert_eq!(p.disasm_addr, 0x1000);
    }

    #[test]
    fn goto_pc_resets_disasm_and_esc_closes() {
        let mut p = DebugPanelState::new(0x1000);
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.handle_key(KeyCode::Char('g'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.disasm_addr, 0x1000);
        assert!(p.disasm_history.is_empty());
        assert_eq!(p.handle_key(KeyCode::Esc, &MockDbg), DebugKey::Close);
    }

    #[test]
    fn memory_scroll_clamps_at_memory_len() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = DebugPane::Memory;
        p.mem_addr = 0x10000 - 16;
        p.handle_key(KeyCode::Down, &MockDbg);
        assert!(p.mem_addr < 0x10000);
    }
}
