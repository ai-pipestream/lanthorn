//! Z-machine debug inspector (tiled pane) — panel state + navigation logic.
//! Pure over the `Debugger` trait (engine-neutral); the render code paints the
//! snapshot this holds. No `zvm::` calls here.
//!
//! Model: three tabbed **windows** in a fixed screen layout (left full height;
//! right split top/bottom). `Tab`/`Shift-Tab` cycle which window is focused;
//! `Left`/`Right` switch the focused window's active tab. The disassembly
//! re-anchors to the live PC on every per-turn `refresh` ("PC-follow").

use ratatui::layout::Rect;

use crate::engine::Debugger;
use crossterm::event::KeyCode;

/// How many instructions / memory rows to pre-render for the address-windowed
/// sections (draw clips to the pane height; over-computing avoids threading
/// height into refresh).
pub const DISASM_WINDOW: usize = 256;
pub const MEM_WINDOW: usize = 256;

/// A displayable section (one tab's content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section { Disasm, Globals, Locals, Objects, Dict, Stack, Memory }

impl Section {
    /// Short label shown on its tab, and used as the on-screen hit target.
    pub fn label(self) -> &'static str {
        match self {
            Section::Disasm => "Disassembly",
            Section::Globals => "Globals",
            Section::Locals => "Locals",
            Section::Objects => "Objects",
            Section::Dict => "Dictionary",
            Section::Stack => "Stack",
            Section::Memory => "Memory",
        }
    }
}

/// Which tabs each window offers, in order. Window 0 = left (full height),
/// 1 = right-top, 2 = right-bottom.
pub const WINDOW_TABS: [&[Section]; 3] = [
    &[Section::Disasm, Section::Globals],
    &[Section::Locals, Section::Objects, Section::Dict],
    &[Section::Stack, Section::Memory],
];

/// The formatted lines the render code paints, refreshed from the Debugger.
#[derive(Debug, Default, Clone)]
pub struct DebugSnapshot {
    pub disasm: Vec<String>,
    pub globals: Vec<String>,
    pub locals: Vec<String>,
    pub objects: Vec<String>,
    pub dict: Vec<String>,
    pub stack: Vec<String>,
    pub memory: Vec<String>,
    /// Instruction start-PCs executed during the last command turn (execution-
    /// coverage marking — a `|` gutter is drawn beside these disasm lines).
    pub executed: std::collections::HashSet<u32>,
}

impl DebugSnapshot {
    /// The lines for one section, regardless of which window shows it.
    pub fn section(&self, s: Section) -> &[String] {
        match s {
            Section::Disasm => &self.disasm,
            Section::Globals => &self.globals,
            Section::Locals => &self.locals,
            Section::Objects => &self.objects,
            Section::Dict => &self.dict,
            Section::Stack => &self.stack,
            Section::Memory => &self.memory,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DebugPanelState {
    /// Focused window: 0 = left, 1 = right-top, 2 = right-bottom.
    pub focus: usize,
    /// Active tab index per window (into `WINDOW_TABS[window]`).
    pub tab: [usize; 3],
    /// List-content scroll offset per window (reset on tab change).
    pub scroll: [usize; 3],
    pub disasm_addr: u32,
    pub mem_addr: u32,
    /// Focused-window content height captured by the last draw (for paging).
    pub viewport: usize,
    /// Live PC (for disasm PC-follow + highlight).
    pub pc: u32,
    pub snapshot: DebugSnapshot,
}

/// Result of a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugKey { Consumed, Ignored, Close }

impl DebugPanelState {
    pub fn new(pc: u32) -> Self {
        DebugPanelState {
            focus: 0,
            tab: [0, 0, 0],
            scroll: [0, 0, 0],
            disasm_addr: pc,
            mem_addr: 0,
            viewport: 1,
            pc,
            snapshot: DebugSnapshot::default(),
        }
    }

    /// The section the given window is currently showing.
    pub fn active_section(&self, window: usize) -> Section {
        WINDOW_TABS[window][self.tab[window]]
    }

    /// Recompute the whole snapshot for the current cursor positions.
    /// **PC-follow:** re-anchors the disassembly to the live PC, so the
    /// executing instruction is always at the top of the Disassembly tab
    /// after a turn.
    pub fn refresh(&mut self, dbg: &dyn Debugger) {
        self.pc = dbg.pc();
        self.disasm_addr = self.pc;
        self.snapshot.disasm = dbg.disassemble(self.disasm_addr, DISASM_WINDOW);
        self.snapshot.globals = dbg.globals_lines();
        self.snapshot.locals = dbg.locals_lines();
        self.snapshot.objects = dbg.object_tree_lines();
        self.snapshot.dict = dbg.dictionary_lines();
        self.snapshot.stack = dbg.stack_lines();
        self.snapshot.memory = dbg.memory_hex(self.mem_addr, MEM_WINDOW);
        self.snapshot.executed = dbg.executed_pcs();
    }

    fn page(&self) -> usize { self.viewport.max(1) }

    /// `window`'s active tab index moves by `dir` (wrapping); its scroll resets.
    fn cycle_tab(&mut self, dir: i32) {
        let window = self.focus;
        let n = WINDOW_TABS[window].len() as i32;
        self.tab[window] = (self.tab[window] as i32 + dir).rem_euclid(n) as usize;
        self.scroll[window] = 0;
    }

    pub fn handle_key(&mut self, code: KeyCode, dbg: &dyn Debugger) -> DebugKey {
        match code {
            KeyCode::Tab => self.focus = (self.focus + 1) % 3,
            KeyCode::BackTab => self.focus = (self.focus + 2) % 3,
            KeyCode::Left => self.cycle_tab(-1),
            KeyCode::Right => self.cycle_tab(1),
            KeyCode::Char('g') => {
                self.disasm_addr = self.pc;
                self.snapshot.disasm = dbg.disassemble(self.disasm_addr, DISASM_WINDOW);
            }
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp
            | KeyCode::Home | KeyCode::End => {
                let window = self.focus;
                match self.active_section(window) {
                    Section::Disasm | Section::Memory => {
                        let step = matches!(code, KeyCode::PageDown | KeyCode::PageUp)
                            .then(|| self.page()).unwrap_or(1);
                        let down = matches!(code, KeyCode::Down | KeyCode::PageDown | KeyCode::End);
                        for _ in 0..step { self.scroll_active(window, down, dbg); }
                    }
                    section => self.scroll_list_key(window, section, code),
                }
            }
            _ => return DebugKey::Ignored,
        }
        DebugKey::Consumed
    }

    /// Scroll `window`'s active section by one step. Used by the key path
    /// (looped for PageUp/PageDown) and directly by the mouse wheel (any
    /// window, regardless of focus). Recomputes only the scrolled section's
    /// lines — never calls `refresh`, which would re-anchor the disassembly
    /// to the PC and fight a manual scroll within the turn.
    pub fn scroll_active(&mut self, window: usize, down: bool, dbg: &dyn Debugger) {
        match self.active_section(window) {
            Section::Disasm => self.step_disasm(down, dbg),
            Section::Memory => self.step_memory(down, dbg),
            section => self.scroll_list(window, section, down),
        }
    }

    fn step_disasm(&mut self, down: bool, dbg: &dyn Debugger) {
        if down {
            self.disasm_addr = dbg.next_instr(self.disasm_addr);
        } else {
            self.disasm_addr = dbg.prev_instr(self.disasm_addr);
        }
        self.snapshot.disasm = dbg.disassemble(self.disasm_addr, DISASM_WINDOW);
    }

    fn step_memory(&mut self, down: bool, dbg: &dyn Debugger) {
        let delta = 16u32;
        if down {
            let max = dbg.memory_len().saturating_sub(16);
            self.mem_addr = (self.mem_addr + delta).min(max);
        } else {
            self.mem_addr = self.mem_addr.saturating_sub(delta);
        }
        self.snapshot.memory = dbg.memory_hex(self.mem_addr, MEM_WINDOW);
    }

    fn scroll_list(&mut self, window: usize, section: Section, down: bool) {
        let max = self.snapshot.section(section).len().saturating_sub(1);
        self.scroll[window] = if down {
            (self.scroll[window] + 1).min(max)
        } else {
            self.scroll[window].saturating_sub(1)
        };
    }

    fn scroll_list_key(&mut self, window: usize, section: Section, code: KeyCode) {
        let len = self.snapshot.section(section).len();
        let vp = self.page();
        let max = len.saturating_sub(1);
        self.scroll[window] = match code {
            KeyCode::Down => (self.scroll[window] + 1).min(max),
            KeyCode::Up => self.scroll[window].saturating_sub(1),
            KeyCode::PageDown => (self.scroll[window] + vp).min(max),
            KeyCode::PageUp => self.scroll[window].saturating_sub(vp),
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => self.scroll[window],
        };
    }

    /// Mouse: focus `window` (click in its body).
    pub fn focus_window(&mut self, window: usize) {
        self.focus = window;
    }

    /// Mouse: activate `tab` in `window` and focus it (click on a tab label).
    pub fn activate_tab(&mut self, window: usize, tab: usize) {
        self.tab[window] = tab;
        self.scroll[window] = 0;
        self.focus = window;
    }
}

// ── Geometry (pure; shared by render and mouse hit-testing) ───────────────────

/// Tile `region` into the three window rects: left full-height, right column
/// split top/bottom. Must match exactly what `render/debug_panel.rs` draws.
pub fn window_rects(region: Rect) -> [Rect; 3] {
    let left_w = region.width / 2;
    let right_x = region.x + left_w;
    let right_w = region.width - left_w;
    let top_h = region.height / 2;
    let left = Rect::new(region.x, region.y, left_w, region.height);
    let r_top = Rect::new(right_x, region.y, right_w, top_h);
    let r_bot = Rect::new(right_x, region.y + top_h, right_w, region.height - top_h);
    [left, r_top, r_bot]
}

/// The on-screen rect of each tab label in `window_rect`'s header (its top
/// border row), one per entry in `sections`, in order. A tab that doesn't fit
/// the available header width gets a zero-width `Rect` (not clickable, and
/// the renderer draws nothing there either — both derive from this same
/// left-to-right walk, so a click on a visible label always resolves to the
/// right tab).
pub fn tab_hit_rects(window_rect: Rect, sections: &[Section]) -> Vec<Rect> {
    if window_rect.width < 3 {
        return sections.iter().map(|_| Rect::default()).collect();
    }
    let row = window_rect.y;
    let right = window_rect.right().saturating_sub(1); // leave room for the corner glyph
    let mut x = window_rect.x + 1; // just inside the left corner glyph
    let mut rects = Vec::with_capacity(sections.len());
    for s in sections {
        if x >= right {
            rects.push(Rect::default());
            continue;
        }
        let label_w = 1 + s.label().chars().count() as u16 + 1; // " Label "
        let w = label_w.min(right - x);
        rects.push(Rect::new(x, row, w, 1));
        x += label_w;
    }
    rects
}

/// Hit-test a click at `(col, row)` (absolute buffer coordinates) against the
/// tab strips of all three windows in `region`. Returns `(window, tab)`.
pub fn tab_at(region: Rect, _panel: &DebugPanelState, col: u16, row: u16) -> Option<(usize, usize)> {
    for (w, window_rect) in window_rects(region).iter().enumerate() {
        let sections = WINDOW_TABS[w];
        for (t, rect) in tab_hit_rects(*window_rect, sections).iter().enumerate() {
            if rect.width > 0 && col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom() {
                return Some((w, t));
            }
        }
    }
    None
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
        fn prev_instr(&self, a: u32) -> u32 { a.saturating_sub(4) }
        fn executed_pcs(&self) -> std::collections::HashSet<u32> { std::collections::HashSet::new() }
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
    fn tab_and_backtab_cycle_window_focus_with_wrap() {
        let mut p = DebugPanelState::new(0x1000);
        assert_eq!(p.focus, 0);
        p.handle_key(KeyCode::Tab, &MockDbg);
        assert_eq!(p.focus, 1);
        p.handle_key(KeyCode::Tab, &MockDbg);
        assert_eq!(p.focus, 2);
        p.handle_key(KeyCode::Tab, &MockDbg); // wraps
        assert_eq!(p.focus, 0);
        p.handle_key(KeyCode::BackTab, &MockDbg); // wraps the other way
        assert_eq!(p.focus, 2);
    }

    #[test]
    fn left_right_cycle_focused_tab_with_wrap_and_reset_scroll() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 1; // Locals | Objects | Dictionary
        p.scroll[1] = 5;
        p.handle_key(KeyCode::Right, &MockDbg);
        assert_eq!(p.tab[1], 1);
        assert_eq!(p.scroll[1], 0, "tab switch resets scroll");
        p.scroll[1] = 3;
        p.handle_key(KeyCode::Right, &MockDbg);
        assert_eq!(p.tab[1], 2);
        p.handle_key(KeyCode::Right, &MockDbg); // wraps
        assert_eq!(p.tab[1], 0);
        p.handle_key(KeyCode::Left, &MockDbg); // wraps the other way
        assert_eq!(p.tab[1], 2);
    }

    #[test]
    fn disasm_scroll_advances_and_retreats_by_instruction_symmetrically() {
        let mut p = DebugPanelState::new(0x1000);
        // focus 0 / tab 0 is Disasm by default. MockDbg's next_instr/prev_instr
        // are inverses (+4/-4), so scrolling down then up round-trips exactly —
        // no history buffer needed (unlike the old disasm_history model).
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1008);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1000);
        // Scrolling up before ever scrolling down still retreats — Feature B:
        // backward scroll is not gated on scroll-down history.
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x0ffc);
    }

    #[test]
    fn memory_scroll_clamps_at_memory_len() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 2;
        p.tab[2] = 1; // Memory
        p.mem_addr = 0x10000 - 16;
        p.handle_key(KeyCode::Down, &MockDbg);
        assert!(p.mem_addr < 0x10000);
    }

    #[test]
    fn refresh_re_anchors_disasm_to_pc() {
        let mut p = DebugPanelState::new(0x2000);
        p.disasm_addr = 0x3000;
        p.refresh(&MockDbg);
        assert_eq!(p.pc, 0x1000);
        assert_eq!(p.disasm_addr, 0x1000);
        assert!(!p.snapshot.disasm.is_empty());
    }

    #[test]
    fn active_section_mapping() {
        let mut p = DebugPanelState::new(0x1000);
        assert_eq!(p.active_section(0), Section::Disasm);
        assert_eq!(p.active_section(1), Section::Locals);
        assert_eq!(p.active_section(2), Section::Stack);
        p.tab[0] = 1;
        p.tab[1] = 2;
        p.tab[2] = 1;
        assert_eq!(p.active_section(0), Section::Globals);
        assert_eq!(p.active_section(1), Section::Dict);
        assert_eq!(p.active_section(2), Section::Memory);
    }

    #[test]
    fn window_rects_tiles_the_region_without_overlap() {
        let region = Rect::new(0, 0, 61, 40);
        let [left, top, bot] = window_rects(region);
        // Left is full height; right column split top/bottom, same x/width.
        assert_eq!(left.height, region.height);
        assert_eq!(top.x, bot.x);
        assert_eq!(top.width, bot.width);
        assert_eq!(top.y, region.y);
        assert_eq!(top.y + top.height, bot.y);
        assert_eq!(bot.y + bot.height, region.y + region.height);
        // Left + right widths cover the region with no gap or overlap.
        assert_eq!(left.width + top.width, region.width);
        assert_eq!(left.x, region.x);
        assert_eq!(top.x, left.x + left.width);
    }

    #[test]
    fn tab_at_resolves_a_click_on_a_visible_tab_label() {
        let region = Rect::new(0, 0, 61, 40);
        let p = DebugPanelState::new(0x1000);
        let [left, ..] = window_rects(region);
        // The first tab label starts at left.x + 1 (just inside the left border).
        let hit = tab_at(region, &p, left.x + 2, left.y);
        assert_eq!(hit, Some((0, 0)));
    }
}
