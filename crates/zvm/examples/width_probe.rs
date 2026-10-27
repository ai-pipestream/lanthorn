//! Which v4+ stories re-read the screen-width header byte after boot? (SQ-1809)
//!
//! A story is told the screen width once, in header byte `$21`. Some games
//! re-lay their status line for whatever width they are told on every input
//! (ADAPTS); others bake the layout at boot and never look again (CACHES), so a
//! host resize leaves their status line at the old width. This probe tells the
//! two apart by asking the machine, after each input, whether the game read
//! `$21` since the last check (`Machine::take_header_width_read`, SQ-1604).
//!
//! For every v4+ story found under the directory (Version 3 is counted and
//! skipped, Version 6 is skipped silently), it boots at `--width`, answers the
//! first y/n-looking prompt with `n`, types `begin` at a "type begin" menu,
//! feeds the chosen command to every other line prompt and the chosen key to
//! every keypress prompt, and records the read flag at boot and after each of
//! the first `--inputs` inputs.
//!
//! Verdict rule: a game ADAPTS if the flag is `Y` after any input following
//! boot, and CACHES if it is `N` after all of them (reads at boot only, or
//! never). With `--resize` the probe also changes the screen to `--rwidth`
//! columns once the inputs are done, runs two more inputs, and records the
//! status line's width before the resize (`before80=` in the note column) next
//! to the final `rightmost=`, which shows whether the status line followed.
//!
//! Flags (the last non-flag argument is the stories directory, default
//! `stories`):
//!
//! - `--width N`   screen width to boot at (default 80)
//! - `--cmd C`     what to type at line prompts: `look` (default), `z`, or
//!   `empty`
//! - `--inputs N`  how many inputs to feed after boot (default 6)
//! - `--key N`     ZSCII code to send at keypress prompts (default 32, space)
//! - `--resize`    resize mid-run, see above
//! - `--rwidth W`  the width `--resize` switches to (default 60)
//! - `--dump`      append the last 300 characters of output to the note column
//! - `--layout`    accepted for compatibility; currently has no effect
//!
//! Output is one tab-separated row per story: file name, `v<version>`,
//! `r<release>/<serial>`, `boot=Y|N`, the per-input read flags (space
//! separated), `fed=` the inputs sent, `rightmost=` the width of the widest
//! non-blank upper-window row at the end, a notes column (`keypress;`,
//! `yn-question;`, `menu-begin;`, `HANG;`, `FAULT;`, `quit;`, ...) and the
//! first non-blank upper-window row. A final `# v3 files seen` line closes it.
//!
//! The walk goes one directory deep and never follows a symlinked directory,
//! so the `stories/stories` self-symlink (SQ-1708) cannot loop it; files
//! reached twice are de-duplicated by canonical path. Hint programs
//! (`*izm.z5` InvisiClues files and `*inv.z5`) are not games and are left out
//! of the verdicts.
//!
//! ```text
//! cargo run --release -q -p lanthorn-zvm --example width_probe -- [flags] stories
//! ```

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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

fn unwrap_blorb(b: Vec<u8>) -> Option<Vec<u8>> {
    if b.len() < 12 || &b[0..4] != b"FORM" || &b[8..12] != b"IFRS" {
        return Some(b);
    }
    let mut p = 12;
    while p + 8 <= b.len() {
        let id = &b[p..p + 4];
        let len = u32::from_be_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
        if id == b"ZCOD" {
            return Some(b[p + 8..(p + 8 + len).min(b.len())].to_vec());
        }
        p += 8 + len + (len & 1);
    }
    None
}

struct Res {
    boot: bool,
    reads: Vec<bool>,
    fed: Vec<String>,
    note: String,
    rightmost: usize,
    row1: String,
}

fn measure(m: &Machine) -> (usize, String) {
    let up = &m.screen.upper;
    let cols = up.cols as usize;
    let mut rm = 0;
    let mut row1 = String::new();
    for row in 0..up.rows as usize {
        let line: String = (0..cols).map(|c| up.cells[row * cols + c].ch).collect();
        let t = line.trim_end();
        if !t.is_empty() {
            rm = rm.max(t.chars().count());
            if row1.is_empty() {
                row1 = t.trim_start().to_string();
            }
        }
    }
    (rm, row1)
}

#[allow(clippy::too_many_arguments)]
fn run(
    bytes: Vec<u8>,
    width: u8,
    cmd: &str,
    ninputs: usize,
    _layout: bool,
    resize: bool,
    dump: bool,
    key: u8,
    rw: u8,
) -> Res {
    let mut ninputs = ninputs;
    let mut resized = false;
    let mut before = (0usize, String::new());
    let mem = Memory::new(bytes).unwrap();
    let out = Rc::new(RefCell::new(String::new()));
    let cfg = BootConfig::new().with_screen_grid(25, width);
    let mut m = Machine::boot(mem, Box::new(Sink(out.clone())), cfg);
    let mut r = Res {
        boot: false,
        reads: vec![],
        fed: vec![],
        note: String::new(),
        rightmost: 0,
        row1: String::new(),
    };
    let mut answered_n = false;
    let mut first = true;
    let mut mark = 0usize;
    let mut steps: u64 = 0;
    let cap: u64 = 30_000_000;
    loop {
        steps += 1;
        if steps > cap {
            r.note.push_str("HANG;");
            break;
        }
        let res = m.step();
        let need = match res {
            StepResult::Continue => continue,
            StepResult::Quit => {
                r.note.push_str("quit;");
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
                r.note.push_str("FAULT;");
                let _ = m.take_fault_trace();
                break;
            }
            other => other,
        };
        let flag = m.take_header_width_read();
        if first {
            r.boot = flag;
            first = false;
        } else {
            r.reads.push(flag);
            if r.reads.len() >= ninputs {
                if resize && !resized {
                    resized = true;
                    before = measure(&m);
                    m.set_screen_dims(25, rw);
                    ninputs += 2;
                } else {
                    break;
                }
            }
        }
        steps = 0;
        let text = out.borrow().clone();
        let n = text.chars().count();
        let tail: String = text
            .chars()
            .skip(n.saturating_sub(300).max(mark.min(n)))
            .collect();
        let tl = tail.to_lowercase();
        let tt = tl.trim_end();
        match need {
            StepResult::NeedChar => {
                let ch = if tt.contains("\"1\"") { b'1' } else { key };
                r.fed.push(format!("<key {}>", ch));
                if let Some(z) = ZsciiInput::new(ch) {
                    m.supply_char(z);
                }
                r.note.push_str("keypress;");
            }
            StepResult::NeedLine { .. } => {
                let yn = tt.contains("(y/n)")
                    || tt.contains("[y/n]")
                    || tt.contains("restore")
                    || tt.contains("yes or no")
                    || tt.contains("(y or n)")
                    || (tt
                        .rsplit('\n')
                        .nth(1)
                        .map(|l| l.trim_end().ends_with('?'))
                        .unwrap_or(false)
                        && !tt.ends_with('>'));
                let s = if tt.contains("type begin") {
                    r.note.push_str("menu-begin;");
                    "begin".to_string()
                } else if yn && !answered_n {
                    answered_n = true;
                    r.note.push_str("yn-question;");
                    "n".to_string()
                } else if cmd == "empty" {
                    String::new()
                } else {
                    cmd.to_string()
                };
                r.fed.push(format!("{:?}", s));
                m.supply_line(&s, 13);
            }
            _ => break,
        }
        mark = out.borrow().chars().count();
    }
    let (rm, row1) = measure(&m);
    r.rightmost = rm;
    r.row1 = row1;
    if resize {
        r.note.push_str(&format!(
            "before80={}|{};",
            before.0,
            before.1.chars().take(60).collect::<String>()
        ));
    }
    if dump {
        let t = out.borrow();
        let n = t.chars().count();
        let tail: String = t.chars().skip(n.saturating_sub(300)).collect();
        r.note.push_str(&format!("TAIL={:?};", tail));
    }
    r
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut v: Vec<_> = rd.flatten().collect();
    v.sort_by_key(|e| e.file_name());
    for e in v {
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else {
            continue;
        };
        let is_link = md.file_type().is_symlink();
        let target_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
        if target_dir {
            if !is_link && depth < 1 {
                walk(&p, depth + 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let mut width = 80u8;
    let mut cmd = "look".to_string();
    let mut ninputs = 6usize;
    let mut layout = false;
    let mut resize = false;
    let mut dump = false;
    let mut key = b' ';
    let mut rw = 60u8;
    let mut dir = "stories".to_string();
    let mut a = env::args().skip(1);
    while let Some(x) = a.next() {
        match x.as_str() {
            "--width" => width = a.next().unwrap().parse().unwrap(),
            "--cmd" => cmd = a.next().unwrap(),
            "--inputs" => ninputs = a.next().unwrap().parse().unwrap(),
            "--layout" => layout = true,
            "--resize" => resize = true,
            "--dump" => dump = true,
            "--rwidth" => rw = a.next().unwrap().parse().unwrap(),
            "--key" => key = a.next().unwrap().parse().unwrap(),
            _ => dir = x,
        }
    }
    let mut files = vec![];
    walk(Path::new(&dir), 0, &mut files);
    let mut seen = HashSet::new();
    let mut v3 = 0;
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let lname = name.to_lowercase();
        let stem = lname.rsplit_once('.').map_or(lname.as_str(), |(s, _)| s);
        if lname.contains("izm") || stem.ends_with("inv") {
            continue;
        }
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !["z3", "z4", "z5", "z7", "z8", "zblorb", "zlb"].contains(&ext.as_str()) {
            continue;
        }
        let Ok(canon) = fs::canonicalize(&p) else {
            continue;
        };
        if !seen.insert(canon) {
            continue;
        }
        let Ok(raw) = fs::read(&p) else { continue };
        let Some(bytes) = unwrap_blorb(raw) else {
            println!("{name}\t?\t?\tno ZCOD");
            continue;
        };
        if bytes.len() < 0x20 {
            continue;
        }
        let ver = bytes[0];
        if ver == 3 {
            v3 += 1;
            continue;
        }
        if ver < 4 || ver == 6 || ver > 8 {
            if ver != 6 {
                println!("{name}\tv{ver}\t-\tskipped (version)");
            }
            continue;
        }
        let rel = u16::from_be_bytes([bytes[2], bytes[3]]);
        let ser = String::from_utf8_lossy(&bytes[0x12..0x18]).to_string();
        let b2 = bytes.clone();
        let (cmd2, w2) = (cmd.clone(), width);
        let r = std::panic::catch_unwind(move || {
            run(b2, w2, &cmd2, ninputs, layout, resize, dump, key, rw)
        });
        match r {
            Ok(r) => {
                let yn = |b: &bool| if *b { "Y" } else { "N" };
                println!(
                    "{name}\tv{ver}\tr{rel}/{ser}\tboot={}\t{}\tfed={}\trightmost={}\t{}\t{}",
                    yn(&r.boot),
                    r.reads.iter().map(yn).collect::<Vec<_>>().join(" "),
                    r.fed.join(","),
                    r.rightmost,
                    r.note,
                    r.row1
                );
            }
            Err(_) => println!("{name}\tv{ver}\tr{rel}/{ser}\tPANIC"),
        }
    }
    println!("# v3 files seen (skipped): {v3}");
}
