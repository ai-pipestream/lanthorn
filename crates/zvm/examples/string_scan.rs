//! Which strings in a story lay themselves out by hand with leading spaces? (SQ-1810)
//!
//! A v4+ game told 80 columns (SQ-1809) sometimes pads or centres lower-window
//! text for 80 with leading spaces: AMFV's opening quotation is indented 22/23/39.
//! lanthorn reflows window 0 to the pane, so in a narrow pane that text breaks
//! up. This tool decodes every string it can find in a story and reports each
//! newline followed by `--min-indent` (default 4) or more spaces, and each string
//! that itself starts with that many spaces, so the real specimens can be seen
//! before a rule is written against them.
//!
//! A "hit" line is a line (delimited by `\n` inside one string) that starts with
//! at least `--min-indent` spaces and has non-space text after them. All-space
//! lines are padding, not layout, and are not counted. Consecutive hit lines of
//! one string form a "block"; blank lines do not end a block, a text line that
//! is not a hit does.
//!
//! COVERAGE (a heuristic, and what it can miss):
//! - Code: a linear sweep from the high-memory base, one routine after another
//!   (header, then instructions until a terminator past the furthest branch or
//!   jump target seen), collecting the text of `print` / `print_ret`.
//! - Strings: at each aligned address a routine is tried first; where the bytes do
//!   not parse as one (a local count over 15, an opcode with no name in this
//!   version, a decode past the end of the file) they are decoded as a packed
//!   string instead. Strings between the high-memory base and the routine holding
//!   the initial PC are decoded the same way.
//! - Misses: strings reached only through `print_addr` / table data in static or
//!   dynamic memory (not packed strings in high memory); anything after a desync
//!   in the sweep (the summary line prints where code stopped, how many routines
//!   and strings were seen, so a short count is visible); text assembled at run
//!   time (`print_char`, `print_num`, object names, text built in buffers).
//!   Abbreviations are expanded by the decoder, so they are not missed.
//!
//! Only stories whose Version is 4, 5, 7 or 8 are scanned; with the default
//! filter only those `zvm::width_probe::probe_status_width` classifies as CACHES
//! (the ones told 80 columns). `--all` scans every v4/v5/v7/v8 story. Hint
//! programs (`*izm.z5`, `*inv.z5`) are skipped, Blorb wrappers are opened, and the
//! walk is one directory deep and never follows a symlinked directory.
//!
//! Flags (the last non-flag argument is the stories directory, default `stories`):
//! - `--all`          scan adapting stories too
//! - `--min-indent N` leading spaces that count as hand layout (default 4)
//! - `--show N`       print at most N blocks per story (default 6; 0 = summary only)
//!
//! ```text
//! cargo run --release -q -p lanthorn-zvm --example string_scan -- [flags] stories
//! ```

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use zvm::cpu::boot::BootConfig;
use zvm::cpu::decode::{decode, Operand};
use zvm::cpu::disasm::mnemonic;
use zvm::memory::Memory;
use zvm::text::decode::decode_string;
use zvm::width_probe::{probe_status_width, WidthVerdict};

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

struct Found {
    addr: u32,
    inline: bool,
    text: String,
}

struct Scan {
    found: Vec<Found>,
    routines: usize,
    strings: usize,
    code_end: u32,
}

fn scan(mem: &Memory) -> Scan {
    let version = mem.version();
    let len = mem.len() as u32;
    let align: u32 = if version == 8 {
        8
    } else if version <= 3 {
        2
    } else {
        4
    };
    let up = |a: u32| a.div_ceil(align) * align;
    let mut s = Scan { found: Vec::new(), routines: 0, strings: 0, code_end: 0 };
    let base = up(mem.read_word(0x04) as u32);
    let pc0 = mem.read_word(0x06) as u32;
    // The routine holding the initial PC anchors the code: its header sits just
    // before the first instruction. Everything from `base` up to it is strings.
    let mut code_start = None;
    for n in 0..=15u32 {
        let (hdr, back) = if version < 5 { (n, 1 + 2 * n) } else { (0, 1) };
        if version >= 5 && n > 0 {
            break;
        }
        if let Some(c) = pc0.checked_sub(back) {
            if c % align == 0 && c >= base && mem.read_byte(c) as u32 == hdr {
                code_start = Some(c);
                break;
            }
        }
    }
    let code_start = code_start.unwrap_or(base);
    let mut a = base;
    while a + 2 <= code_start {
        let (t, end) = decode_string(mem, a);
        if end <= a || end > code_start {
            break;
        }
        s.found.push(Found { addr: a, inline: false, text: t });
        s.strings += 1;
        a = up(end);
    }
    a = code_start;
    // Phase 1: from the code start, walk the high memory at packed alignment.
    // At each address try a routine (header, then instructions to a clean
    // terminator); if that does not parse, take the address as a packed string.
    // Infocom and Inform both interleave the two.
    while a + 2 <= len {
        if let Some((end, texts)) = try_routine(mem, a, version, len) {
            s.found.extend(texts);
            s.routines += 1;
            a = up(end);
        } else {
            let (t, end) = decode_string(mem, a);
            if end <= a || end > len {
                break;
            }
            s.found.push(Found { addr: a, inline: false, text: t });
            s.strings += 1;
            a = up(end);
        }
    }
    s.code_end = a;
    s
}

/// One routine at `a`: `Some((address past it, its inline texts))`, or `None`
/// when the bytes do not parse as a routine (a local count over 15, an opcode
/// with no name in this version, or running off the end of the file).
fn try_routine(mem: &Memory, a: u32, version: u8, len: u32) -> Option<(u32, Vec<Found>)> {
    let nloc = mem.read_byte(a);
    if nloc > 15 {
        return None;
    }
    let mut pc = a + 1 + if version < 5 { 2 * nloc as u32 } else { 0 };
    let mut furthest = pc;
    let mut texts = Vec::new();
    loop {
        if pc >= len {
            return None;
        }
        let ins = decode(mem, pc, version);
        let name = mnemonic(&ins.operand_count, ins.opcode, version);
        if name.starts_with("op:") || ins.next_pc <= pc || ins.next_pc > len {
            return None;
        }
        if let Some((t, _)) = &ins.text {
            texts.push(Found { addr: pc, inline: true, text: t.clone() });
        }
        if let Some(b) = &ins.branch {
            if b.offset > 1 {
                furthest = furthest.max((ins.next_pc as i64 + b.offset as i64 - 2) as u32);
            }
        }
        let mut terminal = matches!(
            name,
            "ret" | "rtrue" | "rfalse" | "ret_popped" | "print_ret" | "quit" | "restart" | "throw"
        );
        if name == "jump" {
            terminal = true;
            if let Some(Operand::Large(o)) = ins.operands.first() {
                let off = *o as i16 as i64;
                if off > 0 {
                    furthest = furthest.max((ins.next_pc as i64 + off - 2) as u32);
                }
            }
        }
        pc = ins.next_pc;
        if terminal && pc > furthest {
            return Some((pc, texts));
        }
    }
}

struct Block {
    addr: u32,
    inline: bool,
    /// (indent, length including indent) per line
    lines: Vec<(usize, usize)>,
    excerpt: String,
}

fn blocks(f: &Found, min: usize) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;
    for line in f.text.split('\n') {
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let body = line.trim_end();
        let hit = indent >= min && !body.trim_start().is_empty();
        if hit {
            let b = cur.get_or_insert_with(|| Block {
                addr: f.addr,
                inline: f.inline,
                lines: Vec::new(),
                excerpt: String::new(),
            });
            b.lines.push((indent, body.chars().count()));
            b.excerpt.push_str(body);
            b.excerpt.push('\n');
        } else if !line.trim().is_empty() {
            if let Some(b) = cur.take() {
                out.push(b);
            }
        }
    }
    if let Some(b) = cur.take() {
        out.push(b);
    }
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut ents: Vec<_> = rd.flatten().collect();
    ents.sort_by_key(|e| e.file_name());
    for e in ents {
        let p = e.path();
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if md.is_dir() {
            if depth < 1 {
                walk(&p, depth + 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let mut all = false;
    let mut min = 4usize;
    let mut show = 6usize;
    let mut dir = "stories".to_string();
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--all" => all = true,
            "--min-indent" => min = args.next().and_then(|v| v.parse().ok()).unwrap_or(4),
            "--show" => show = args.next().and_then(|v| v.parse().ok()).unwrap_or(6),
            _ => dir = a,
        }
    }
    let mut files = Vec::new();
    walk(Path::new(&dir), 0, &mut files);
    let mut seen = HashSet::new();
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if lower.ends_with("izm.z5") || lower.ends_with("inv.z5") {
            continue;
        }
        let ext_ok = [".z4", ".z5", ".z7", ".z8", ".zblorb"].iter().any(|e| lower.ends_with(e));
        if !ext_ok {
            continue;
        }
        let Ok(canon) = fs::canonicalize(&p) else { continue };
        if !seen.insert(canon) {
            continue;
        }
        let Ok(raw) = fs::read(&p) else { continue };
        let Some(bytes) = unwrap_blorb(raw) else { continue };
        let Ok(mem) = Memory::new(bytes.clone()) else { continue };
        let v = mem.version();
        if !matches!(v, 4 | 5 | 7 | 8) {
            continue;
        }
        let verdict = probe_status_width(bytes, BootConfig::default()).map(|p| p.verdict);
        let caches = verdict == Some(WidthVerdict::Caches);
        if !all && !caches {
            continue;
        }
        let rel = mem.read_word(0x02);
        let serial: String = (0x12..0x18).map(|i| mem.read_byte(i) as char).collect();
        let sc = scan(&mem);
        let mut hits: Vec<Block> = Vec::new();
        for f in &sc.found {
            hits.extend(blocks(f, min));
        }
        let hit_lines: usize = hits.iter().map(|b| b.lines.len()).sum();
        println!(
            "{name}\tv{v} r{rel}/{serial}\t{}\tblocks={} hit-lines={}\t(walk: {} routines, {} strings, ended {:#x} of {:#x})",
            if caches { "caches" } else { "adapts/other" },
            hits.len(),
            hit_lines,
            sc.routines,
            sc.strings,
            sc.code_end,
            mem.len()
        );
        for b in hits.iter().take(show) {
            let lines: Vec<String> = b.lines.iter().map(|(i, l)| format!("{i}+{}", l - i)).collect();
            println!(
                "  {:#07x} {} lines indent+len: {}",
                b.addr,
                if b.inline { "inline" } else { "string" },
                lines.join(" ")
            );
            for l in b.excerpt.lines().take(6) {
                println!("    |{l}");
            }
        }
    }
}
