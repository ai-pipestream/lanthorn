//! Script-driven probe for how hint programs lay out across screen widths. (SQ-1809)
//!
//! InvisiClues hint programs (`*inv.z5`, `*izm.z5`) and the Solid Gold games'
//! built-in hints draw a menu in the upper window. SQ-1809 measured how those
//! screens lay out at different widths and whether the programs re-read the
//! screen-width header byte `$21` after boot. This is the headless driver for
//! that: it boots one story at a given width, feeds it a script of keys, lines
//! and resizes, and after every input the story waits for it prints the
//! `$21`-read flag (`Machine::take_header_width_read`, `Y`/`N` since the last
//! check), the upper window's size with header bytes `$20`/`$21`, the upper
//! window rows (`U|...|`) and the last lines of lower-window text (`L|...`).
//!
//! ```text
//! hint_probe FILE WIDTH TOKEN...
//! ```
//!
//! Tokens, one per input the story asks for:
//!
//! - `cX`    a key. A single character after the `c` is that character
//!   (`cn` is `n`, `c1` is `1`); anything longer is a decimal ZSCII code
//!   (`c13` is Enter, `c32` is space).
//! - `lTEXT` a line, submitted with Enter. `_` stands for a space; a bare `l`
//!   is an empty line (LGOP needs one first).
//! - `rW`    resize the screen to `W` columns now, before the next input. It
//!   consumes no input and may be repeated.
//!
//! A token of the wrong kind for what the story is waiting on stops the run, as
//! does running out of tokens. `QUIT`, `FAULT` and `HANG` (40M steps without
//! input) are printed if they happen.
//!
//! Env var: `Q=1` gives terse dumps (no upper rows, last 3 lower lines instead
//! of 8).
//!
//! Scripts used for SQ-1809:
//!
//! - `*inv.z5`:   `c32 cn c13 c13 ch ch cq cq`
//! - `*izm.z5`:   `c32 cn cn cn cn cn cn cn c13 c13 c13 c13 cq cq`
//! - Solid Gold built-in hints: `lhint lhint cn c13 c13 c13 ch cq cq cq llook`
//!
//! ```text
//! cargo run --release -q -p lanthorn-zvm --example hint_probe -- stories/zork1izm.z5 45 c32 cn c13 cq
//! ```

use std::any::Any;
use std::cell::RefCell;
use std::env;
use std::fs;
use std::rc::Rc;
use zvm::cpu::exec::{BootConfig, Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;
use zvm::text::input::ZsciiInput;

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
fn unwrap_blorb(b: Vec<u8>) -> Vec<u8> {
    if b.len() < 12 || &b[0..4] != b"FORM" {
        return b;
    }
    let mut p = 12;
    while p + 8 <= b.len() {
        let len = u32::from_be_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
        if &b[p..p + 4] == b"ZCOD" {
            return b[p + 8..(p + 8 + len).min(b.len())].to_vec();
        }
        p += 8 + len + (len & 1);
    }
    b
}
fn dump(m: &Machine, out: &Rc<RefCell<String>>, quiet: bool) {
    let up = &m.screen.upper;
    let cols = up.cols as usize;
    println!(
        "  [upper rows={} cols={} hdr20={} hdr21={}]",
        up.rows,
        up.cols,
        m.mem.read_byte(0x20),
        m.mem.read_byte(0x21)
    );
    if !quiet {
        for row in 0..up.rows as usize {
            let line: String = (0..cols).map(|c| up.cells[row * cols + c].ch).collect();
            println!("  U|{}|", line.trim_end());
        }
    }
    let t = out.borrow().clone();
    let t = t.trim_end();
    let lines: Vec<&str> = t.lines().collect();
    let from = lines.len().saturating_sub(if quiet { 3 } else { 8 });
    for l in &lines[from..] {
        println!("  L|{}", l);
    }
    out.borrow_mut().clear();
}
fn main() {
    let a: Vec<String> = env::args().skip(1).collect();
    let bytes = unwrap_blorb(fs::read(&a[0]).unwrap());
    let width: u8 = a[1].parse().unwrap();
    let quiet = env::var("Q").is_ok();
    let ver = bytes[0];
    println!(
        "== {} v{} r{} s{} width={}",
        a[0],
        ver,
        u16::from_be_bytes([bytes[2], bytes[3]]),
        String::from_utf8_lossy(&bytes[0x12..0x18]),
        width
    );
    let mem = Memory::new(bytes).unwrap();
    let out = Rc::new(RefCell::new(String::new()));
    let mut m = Machine::boot(
        mem,
        Box::new(Sink(out.clone())),
        BootConfig::new().with_screen_grid(25, width),
    );
    let mut script = a[2..].iter();
    let mut n = 0;
    let mut steps = 0u64;
    loop {
        steps += 1;
        if steps > 40_000_000 {
            println!("HANG");
            break;
        }
        let need = match m.step() {
            StepResult::Continue => continue,
            StepResult::Quit => {
                println!("QUIT");
                break;
            }
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
            StepResult::Fault => {
                println!("FAULT");
                break;
            }
            o => o,
        };
        steps = 0;
        let flag = m.take_header_width_read();
        let kind = match need {
            StepResult::NeedChar => "CHAR",
            StepResult::NeedLine { .. } => "LINE",
            _ => "?",
        };
        println!(
            "#{} wait={} $21read={}",
            n,
            kind,
            if flag { "Y" } else { "N" }
        );
        dump(&m, &out, quiet);
        n += 1;
        let mut tok = match script.next() {
            Some(t) => t.clone(),
            None => break,
        };
        while tok.starts_with('r') && tok[1..].parse::<u8>().is_ok() {
            let w: u8 = tok[1..].parse().unwrap();
            m.set_screen_dims(25, w);
            println!(">> resize to {w}");
            tok = match script.next() {
                Some(t) => t.clone(),
                None => return,
            };
        }
        println!(">> {}", tok);
        if let Some(r) = tok.strip_prefix('c') {
            let ch: u8 = if r.len() == 1 {
                r.as_bytes()[0]
            } else {
                r.parse().unwrap()
            };
            if !matches!(need, StepResult::NeedChar) {
                println!("(wanted line, got key)");
                break;
            }
            if let Some(z) = ZsciiInput::new(ch) {
                m.supply_char(z);
            }
        } else {
            let s = tok[1..].replace('_', " ");
            if !matches!(need, StepResult::NeedLine { .. }) {
                println!("(wanted key, got line)");
                break;
            }
            m.supply_line(&s, 13);
        }
    }
}
