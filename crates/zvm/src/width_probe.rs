//! Does a Version 4+ story re-read the screen width, or lay itself out once?
//! (SQ-1809)
//!
//! Header byte `$21` tells a story how wide the screen is, and the story is the
//! only party that knows what it does with the answer. Two kinds exist:
//!
//! * **Adapts** — the story reads `$21` again as it plays (Inform's library does
//!   on every turn) and lays its status line out from whatever it finds. A host
//!   may give such a story the real window width and keep giving it the new one
//!   as the window changes.
//! * **Caches** — the story works out its status layout once, at boot or at its
//!   first screen, and from then on only `set_cursor`s to the columns it derived
//!   then (Infocom's own v4/v5 games). Narrow the screen under it and the moves
//!   fall outside the window; widen it and the layout stays put.
//!
//! The two cannot be told apart by the story's version, its compiler or its
//! release, and a story caches whatever it is told at boot — so the answer has
//! to exist BEFORE the real boot. [`probe_status_width`] gets it by booting a
//! throwaway machine from the same bytes and watching
//! [`Machine::take_header_width_read`] (SQ-1604) while it plays a few turns.
//!
//! # The rules (measured over every v4+ story in the author's corpus)
//!
//! 1. Only reads AFTER the first line-input prompt has been answered count. A
//!    read during the keypress turn right after boot says nothing: Trinity reads
//!    `$21` once there and never again.
//! 2. Any read within the next [`LINE_TURNS`] line turns (or [`MAX_TURNS`] turns
//!    of any kind) means ADAPTS. Otherwise CACHES.
//! 3. Keypress prompts are answered with Enter, line prompts with `look`. A
//!    prompt that comes back with the screen unchanged is retried with a
//!    rotation of answers (`1`, `begin`, `n`, `y`, a bare Enter), and until a
//!    status line has been drawn the rotation simply walks on with every input —
//!    enough for Border Zone's chapter menu and Beyond Zork's start screen
//!    without naming either. Only turns played with a status line on screen
//!    count against the budget, and a retry is never a turn: it spends the
//!    repeat limit instead.
//! 4. A step cap per input, an input cap and a repeated-identical-prompt limit
//!    all end the probe with CACHES, the safe outcome (a story told 80 columns
//!    behaves exactly as it always did on an 80-column screen).
//!
//! Two behaviours worth knowing, both measured rather than assumed. A timed
//! `read_char` (an interrupt interval and routine) is answered like any other
//! keypress: the key we send ends the read, so the interrupt routine never needs
//! running. And a story that is driven entirely by keypresses and never asks for
//! a line (Inform's `solpoker`, whose game loop is a 0.1 s timed `read_char`) is
//! never "armed" by rule 1, so it ends on the input cap as CACHES. That is the
//! safe outcome, and the only one the evidence would not contradict.
//!
//! The probe never opens a story's own hint system, saves or restores: a save
//! request is declined and a restore request fails.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use crate::cpu::boot::BootConfig;
use crate::cpu::exec::{Machine, StepResult};
use crate::io::Output;
use crate::memory::Memory;
use crate::text::input::ZsciiInput;

/// What the probe concluded about a story.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthVerdict {
    /// The story re-reads `$21` as it plays and lays out from the answer.
    Adapts,
    /// The story computes its layout once and never looks again.
    Caches,
}

/// Why the probe stopped, for tools and tests that want the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthEvidence {
    /// A `$21` read was seen after the first line input was answered.
    ReadAfterInput,
    /// The turn budget ran out with no such read.
    NoReadInBudget,
    /// The story quit, faulted, or stopped asking for input.
    StoppedEarly,
    /// A step, input or repeated-prompt cap was hit.
    CapHit,
}

/// The probe's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidthProbe {
    /// The verdict.
    pub verdict: WidthVerdict,
    /// Why.
    pub evidence: WidthEvidence,
    /// Inputs the probe fed the throwaway machine.
    pub inputs: usize,
    /// Whether the upper window ever held text (a status line was drawn). A
    /// story that never drew one is classified CACHES by default.
    pub status_drawn: bool,
}

/// Line turns inspected after the first line input before concluding CACHES.
pub const LINE_TURNS: usize = 5;
/// Turns of any kind (line or keypress) inspected before concluding CACHES, for
/// a story whose early play is keypress-driven (Bureaucracy's form).
pub const MAX_TURNS: usize = 24;
/// Hard cap on inputs fed, retries included.
pub const MAX_INPUTS: usize = 60;
/// Consecutive identical prompts tolerated before giving up.
pub const MAX_REPEATS: usize = 7;
/// Instructions one input may run before the probe calls it a hang.
pub const STEPS_PER_INPUT: u64 = 20_000_000;

const LINE_ANSWERS: [&str; 6] = ["look", "1", "begin", "n", "y", ""];
const KEY_ANSWERS: [u8; 5] = [13, b' ', b'1', b'y', b'n'];

struct Sink(Rc<RefCell<String>>);

impl Output for Sink {
    fn print(&mut self, s: &str) {
        self.0.borrow_mut().push_str(s);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The upper window's text, row by row, for the unchanged-screen test.
fn upper_text(m: &Machine) -> String {
    let up = &m.screen.upper;
    let cols = up.cols as usize;
    let mut s = String::new();
    for row in 0..up.rows as usize {
        s.extend((0..cols).map(|c| up.cells[row * cols + c].ch));
        s.push('\n');
    }
    s
}

/// Classify a Version 4/5/7/8 story by playing a throwaway copy of it.
///
/// `story` is the story file's bytes and `config` the boot configuration the
/// real boot would use (the pane's rows and columns are the natural screen to
/// give it; the verdict does not depend on them). Returns `None` for a story
/// this question does not apply to — Versions 1–3 draw their status line in the
/// interpreter, Version 6 has its own pixel screen — or one that will not load.
///
/// Costs a few thousand to a few million instructions, once per boot.
pub fn probe_status_width(story: Vec<u8>, config: BootConfig) -> Option<WidthProbe> {
    let mem = Memory::new(story).ok()?;
    let version = mem.version();
    if !(4..=8).contains(&version) || version == 6 {
        return None;
    }
    let out = Rc::new(RefCell::new(String::new()));
    let mut m = Machine::boot(mem, Box::new(Sink(out.clone())), config);

    let finish = |verdict, evidence, inputs, status_drawn| WidthProbe { verdict, evidence, inputs, status_drawn };
    let mut inputs = 0usize;
    let mut status_drawn = false;
    // Armed once the first LINE prompt has been answered (rule 1).
    let mut armed = false;
    let mut line_turns = 0usize;
    let mut turns = 0usize;
    let mut last: Option<(bool, String)> = None;
    let mut repeats = 0usize;
    let mut steps = 0u64;
    loop {
        steps += 1;
        if steps > STEPS_PER_INPUT {
            return Some(finish(WidthVerdict::Caches, WidthEvidence::CapHit, inputs, status_drawn));
        }
        let is_line = match m.step() {
            StepResult::Continue => continue,
            StepResult::Restart => {
                m.restart();
                continue;
            }
            StepResult::SaveRequest => {
                m.complete_save(false);
                continue;
            }
            StepResult::RestoreRequest => {
                m.complete_restore_failure();
                continue;
            }
            StepResult::Quit | StepResult::Fault => {
                let _ = m.take_fault_trace();
                return Some(finish(WidthVerdict::Caches, WidthEvidence::StoppedEarly, inputs, status_drawn));
            }
            StepResult::NeedLine { .. } => true,
            StepResult::NeedChar => false,
            #[allow(unreachable_patterns)]
            _ => return Some(finish(WidthVerdict::Caches, WidthEvidence::StoppedEarly, inputs, status_drawn)),
        };
        steps = 0;
        let read = m.take_header_width_read();
        let upper = upper_text(&m);
        status_drawn |= upper.chars().any(|c| !c.is_whitespace());
        if armed && read {
            return Some(finish(WidthVerdict::Adapts, WidthEvidence::ReadAfterInput, inputs, status_drawn));
        }
        // The prompt's identity: kind, the tail of the story text, the grid.
        let text = out.borrow();
        let tail: String = {
            let n = text.chars().count();
            text.chars().skip(n.saturating_sub(200)).collect()
        };
        drop(text);
        let key = (is_line, format!("{tail}\u{1}{upper}"));
        let same = last.as_ref() == Some(&key);
        last = Some(key);
        if same {
            repeats += 1;
            if repeats > MAX_REPEATS {
                return Some(finish(WidthVerdict::Caches, WidthEvidence::CapHit, inputs, status_drawn));
            }
        } else {
            repeats = 0;
            // Only turns played with a status line on screen count against the
            // budget: until then the story is still in its title or menu.
            if armed && status_drawn {
                turns += 1;
                if is_line {
                    line_turns += 1;
                }
                if line_turns >= LINE_TURNS || turns >= MAX_TURNS {
                    return Some(finish(WidthVerdict::Caches, WidthEvidence::NoReadInBudget, inputs, status_drawn));
                }
            }
        }
        if inputs >= MAX_INPUTS {
            return Some(finish(WidthVerdict::Caches, WidthEvidence::CapHit, inputs, status_drawn));
        }
        inputs += 1;
        // Before a status line exists the story may be sitting in a menu, so walk
        // the answers by input count; once it is playing, `look` and only a
        // repeated prompt moves on.
        let pick = if status_drawn { repeats } else { inputs - 1 + repeats };
        if is_line {
            m.supply_line(LINE_ANSWERS[pick % LINE_ANSWERS.len()], 13);
            armed = true;
        } else {
            let k = KEY_ANSWERS[pick % KEY_ANSWERS.len()];
            if let Some(z) = ZsciiInput::new(k) {
                m.supply_char(z);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal v5 story: `loop { [loadb 0 0x21 -> G2;] inc G1; print_num G1;
    /// new_line; read buf 0 -> G0; jump loop }`. With `reads_width` the loop
    /// reads header byte $21 every turn (an ADAPTing story); without, never (a
    /// CACHing one). The counter makes every prompt's text differ, so each is a
    /// real turn rather than a repeat.
    fn story(reads_width: bool) -> Vec<u8> {
        let mut b = vec![0u8; 0x0800];
        b[0x00] = 5;
        b[0x04] = 0x04;
        b[0x06] = 0x00;
        b[0x07] = 0x40; // initial pc
        b[0x08] = 0x00;
        b[0x09] = 0x80; // dictionary (empty)
        b[0x0081] = 4;
        b[0x0A] = 0x01; // object table
        b[0x0C] = 0x03; // globals
        b[0x0E] = 0x04; // static memory base
        b[0x18] = 0x00;
        b[0x19] = 0x60;
        b[0x0200] = 20; // text buffer: max length
        let mut code: Vec<u8> = Vec::new();
        if reads_width {
            code.extend([0x10, 0x00, 0x21, 0x12]); // loadb 0 0x21 -> G2
        }
        code.extend([0xA5, 0x11]); // inc G1
        code.extend([0xE6, 0xBF, 0x11]); // print_num G1
        code.extend([0xBB]); // new_line
        code.extend([0xE4, 0x0F, 0x02, 0x00, 0x00, 0x00, 0x10]); // read 0x200 0 -> G0
        let here = 0x40 + code.len() as i32 + 3;
        let off = (0x40 - here + 2) as i16;
        code.extend([0x8C, (off >> 8) as u8, off as u8]); // jump to the top
        b[0x40..0x40 + code.len()].copy_from_slice(&code);
        b
    }

    fn cfg() -> BootConfig {
        BootConfig::new().with_screen_grid(25, 80)
    }

    #[test]
    fn a_story_that_rereads_the_width_adapts() {
        let p = probe_status_width(story(true), cfg()).expect("a v5 story");
        assert_eq!(p.verdict, WidthVerdict::Adapts);
        assert_eq!(p.evidence, WidthEvidence::ReadAfterInput);
    }

    #[test]
    fn a_story_that_never_rereads_the_width_caches() {
        let p = probe_status_width(story(false), cfg()).expect("a v5 story");
        assert_eq!(p.verdict, WidthVerdict::Caches);
        // The synthetic story draws no status line, so no turn counts against
        // the budget (rule 3) and the probe falls back on the input cap.
        assert!(!p.status_drawn);
        assert_eq!(p.evidence, WidthEvidence::CapHit);
        assert_eq!(p.inputs, MAX_INPUTS);
    }

    #[test]
    fn versions_this_does_not_apply_to_are_declined() {
        let mut v3 = story(true);
        v3[0] = 3;
        assert!(probe_status_width(v3, cfg()).is_none());
        let mut v6 = story(true);
        v6[0] = 6;
        assert!(probe_status_width(v6, cfg()).is_none());
    }

    #[test]
    fn a_repeated_identical_prompt_ends_the_probe_as_caches() {
        // Same loop without the counter: every prompt is identical.
        let mut b = story(false);
        // Overwrite `inc G1; print_num G1` (5 bytes at 0x40) with `new_line`s.
        b[0x40..0x45].copy_from_slice(&[0xBB, 0xBB, 0xBB, 0xBB, 0xBB]);
        let p = probe_status_width(b, cfg()).expect("a v5 story");
        assert_eq!(p.verdict, WidthVerdict::Caches);
    }
}
