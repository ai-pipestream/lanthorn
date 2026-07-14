//! Generator for `crates/gvm/src/unicode_norm_tables.rs`.
//!
//! Parses the Unicode Character Database (UCD) and emits compact, sorted,
//! binary-searchable tables that back gvm's NFD/NFC implementation
//! (`glk_buffer_canon_decompose_uni` / `glk_buffer_canon_normalize_uni`) and
//! the titlecase mapping used by `glk_buffer_to_title_case_uni` (the only case
//! operation Rust's std `char` methods do not cover). Hangul is handled
//! algorithmically at runtime, so it needs no tables here.
//!
//! Usage:
//!   1. Download the three pinned UCD files (see UNICODE_VERSION below):
//!        https://www.unicode.org/Public/<VER>/ucd/UnicodeData.txt
//!        https://www.unicode.org/Public/<VER>/ucd/CompositionExclusions.txt
//!        https://www.unicode.org/Public/<VER>/ucd/SpecialCasing.txt
//!      into some directory <UCD_DIR>. (The raw files are NOT committed.)
//!   2. cargo run --manifest-path crates/gvm/tables-gen/Cargo.toml -- \
//!          <UCD_DIR> crates/gvm/src/unicode_norm_tables.rs
//!
//! The emitted file carries its own header with the version + this command.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

const UNICODE_VERSION: &str = "16.0.0";

// Hangul algorithmic range (UAX #15). Kept in sync with the runtime module.
const S_BASE: u32 = 0xAC00;
const S_COUNT: u32 = 11172;

fn is_hangul_syllable(cp: u32) -> bool {
    cp >= S_BASE && cp < S_BASE + S_COUNT
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: {} <UCD_DIR> <OUT.rs>", args[0]);
        std::process::exit(2);
    }
    let ucd_dir = Path::new(&args[1]);
    let out_path = Path::new(&args[2]);

    let unicode_data = fs::read_to_string(ucd_dir.join("UnicodeData.txt"))
        .expect("read UnicodeData.txt");
    let comp_excl = fs::read_to_string(ucd_dir.join("CompositionExclusions.txt"))
        .expect("read CompositionExclusions.txt");
    let special_casing = fs::read_to_string(ucd_dir.join("SpecialCasing.txt"))
        .expect("read SpecialCasing.txt");

    // ── Parse UnicodeData.txt ────────────────────────────────────────────────
    // canonical single-level decomposition map + combining-class map, plus the
    // simple (1:1) uppercase (field 12) and titlecase (field 14) mappings used
    // to derive the titlecase table below.
    let mut raw_decomp: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut ccc: BTreeMap<u32, u8> = BTreeMap::new();
    let mut simple_upper: BTreeMap<u32, u32> = BTreeMap::new();
    let mut simple_title: BTreeMap<u32, u32> = BTreeMap::new();

    for line in unicode_data.lines() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(';').collect();
        if f.len() < 6 {
            continue;
        }
        let cp = u32::from_str_radix(f[0], 16).expect("cp hex");
        // Simple case mappings (fields 12/14): recorded before the Hangul skip
        // is irrelevant (Hangul has none), but only for non-Hangul we reach here.
        if f.len() > 12 && !f[12].is_empty() {
            simple_upper.insert(cp, u32::from_str_radix(f[12], 16).expect("upper hex"));
        }
        if f.len() > 14 && !f[14].is_empty() {
            simple_title.insert(cp, u32::from_str_radix(f[14], 16).expect("title hex"));
        }
        // Skip the algorithmic Hangul block — decomposed/composed at runtime.
        if is_hangul_syllable(cp) {
            continue;
        }
        // Combining class (field 3, decimal).
        let cls: u32 = f[3].parse().expect("ccc");
        if cls != 0 {
            ccc.insert(cp, cls as u8);
        }
        // Decomposition mapping (field 5). A leading '<tag>' marks a
        // COMPATIBILITY mapping — excluded from canonical (NFD/NFC).
        let dec = f[5].trim();
        if dec.is_empty() || dec.starts_with('<') {
            continue;
        }
        let seq: Vec<u32> = dec
            .split_whitespace()
            .map(|h| u32::from_str_radix(h, 16).expect("decomp hex"))
            .collect();
        raw_decomp.insert(cp, seq);
    }

    // ── Full (recursive) canonical decomposition ─────────────────────────────
    // Each canonical decomposition element may itself decompose; expand fully.
    // `memo` caches leaf/intermediate expansions; `full_decomp` keeps ONLY the
    // characters that actually have a canonical decomposition (raw_decomp keys).
    let mut memo: HashMap<u32, Vec<u32>> = HashMap::new();
    fn expand(
        cp: u32,
        raw: &BTreeMap<u32, Vec<u32>>,
        memo: &mut HashMap<u32, Vec<u32>>,
    ) -> Vec<u32> {
        if let Some(v) = memo.get(&cp) {
            return v.clone();
        }
        let out = match raw.get(&cp) {
            None => vec![cp],
            Some(seq) => {
                let mut acc = Vec::new();
                for &c in seq {
                    acc.extend(expand(c, raw, memo));
                }
                acc
            }
        };
        memo.insert(cp, out.clone());
        out
    }
    let mut full_decomp: HashMap<u32, Vec<u32>> = HashMap::new();
    for &cp in raw_decomp.keys() {
        let d = expand(cp, &raw_decomp, &mut memo);
        full_decomp.insert(cp, d);
    }

    // ── Full composition exclusions ──────────────────────────────────────────
    // Full_Composition_Exclusion = script-specific exclusions (from file)
    //   ∪ singletons (canonical decomp of length 1)
    //   ∪ non-starter decompositions (first char of decomp has ccc != 0).
    let mut excluded: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for line in comp_excl.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Entries are single code points (possibly with trailing whitespace).
        if let Some(tok) = line.split_whitespace().next() {
            if let Ok(cp) = u32::from_str_radix(tok, 16) {
                excluded.insert(cp);
            }
        }
    }
    for (&cp, seq) in &raw_decomp {
        // singleton
        if seq.len() == 1 {
            excluded.insert(cp);
        }
        // non-starter decomposition: first element carries a combining class
        if let Some(&first) = seq.first() {
            if ccc.get(&first).copied().unwrap_or(0) != 0 {
                excluded.insert(cp);
            }
        }
    }

    // ── Primary composites (for NFC) ─────────────────────────────────────────
    // Invert canonical decompositions of exactly two chars that are not
    // fully excluded: (a, b) -> composite.
    let mut compose: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (&cp, seq) in &raw_decomp {
        if seq.len() == 2 && !excluded.contains(&cp) {
            compose.insert((seq[0], seq[1]), cp);
        }
    }

    // ── Full case mappings (SpecialCasing.txt) → titlecase table ─────────────
    // SpecialCasing gives the full (possibly multi-char) lower/title/upper
    // mappings. We only want *unconditional* entries (no locale/context in the
    // condition field); locale-tailored casing is out of scope, exactly as
    // Rust's std `char::to_uppercase` ignores it.
    let mut full_upper: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut full_title: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let parse_seq = |s: &str| -> Vec<u32> {
        s.split_whitespace()
            .map(|h| u32::from_str_radix(h, 16).expect("case hex"))
            .collect()
    };
    for line in special_casing.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(';').map(str::trim).collect();
        // <code>; <lower>; <title>; <upper>; (<condition>;)?
        if parts.len() < 4 {
            continue;
        }
        // A non-empty condition field marks a locale/context-tailored entry.
        if parts.get(4).map(|c| !c.is_empty()).unwrap_or(false) {
            continue;
        }
        let cp = u32::from_str_radix(parts[0], 16).expect("sc cp hex");
        full_title.insert(cp, parse_seq(parts[2]));
        full_upper.insert(cp, parse_seq(parts[3]));
    }

    // The correct full uppercase of a code point (what std `char::to_uppercase`
    // produces): SpecialCasing full mapping if any, else the simple field-12
    // mapping, else the code point itself.
    let upper_of = |cp: u32| -> Vec<u32> {
        if let Some(u) = full_upper.get(&cp) {
            u.clone()
        } else if let Some(&u) = simple_upper.get(&cp) {
            vec![u]
        } else {
            vec![cp]
        }
    };
    // The correct full titlecase: SpecialCasing full title if any, else the
    // simple field-14 titlecase, else the uppercase mapping (Unicode default).
    let title_of = |cp: u32| -> Vec<u32> {
        if let Some(t) = full_title.get(&cp) {
            t.clone()
        } else if let Some(&t) = simple_title.get(&cp) {
            vec![t]
        } else {
            upper_of(cp)
        }
    };
    // Titlecase table = code points whose titlecase differs from their (std)
    // uppercase mapping. For everything else, runtime falls back to
    // `char::to_uppercase`, which is the Unicode-default titlecase.
    let mut title_candidates: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    title_candidates.extend(simple_title.keys().copied());
    title_candidates.extend(full_title.keys().copied());
    let mut title_map: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &cp in &title_candidates {
        let t = title_of(cp);
        if t != upper_of(cp) {
            title_map.insert(cp, t);
        }
    }

    // ── Emit ─────────────────────────────────────────────────────────────────
    let mut out = String::new();
    let cmd = "cargo run --manifest-path crates/gvm/tables-gen/Cargo.toml -- <UCD_DIR> crates/gvm/src/unicode_norm_tables.rs";
    writeln!(out, "// @generated by crates/gvm/tables-gen — DO NOT EDIT BY HAND.").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// Unicode canonical normalization tables (NFD/NFC data) + titlecase mapping.").unwrap();
    writeln!(out, "// Unicode version: {UNICODE_VERSION}").unwrap();
    writeln!(out, "// Source files (re-downloadable; NOT committed):").unwrap();
    writeln!(out, "//   https://www.unicode.org/Public/{UNICODE_VERSION}/ucd/UnicodeData.txt").unwrap();
    writeln!(out, "//   https://www.unicode.org/Public/{UNICODE_VERSION}/ucd/CompositionExclusions.txt").unwrap();
    writeln!(out, "//   https://www.unicode.org/Public/{UNICODE_VERSION}/ucd/SpecialCasing.txt").unwrap();
    writeln!(out, "// Regenerate with:").unwrap();
    writeln!(out, "//   {cmd}").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// Hangul is handled algorithmically in unicode_norm.rs (no tables).").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// The Unicode version these tables were generated from.").unwrap();
    writeln!(out, "pub const UNICODE_VERSION: &str = {UNICODE_VERSION:?};").unwrap();
    writeln!(out).unwrap();

    // Fully-recursive canonical decompositions, flattened.
    // DECOMP_KEYS[i] decomposes to DECOMP_DATA[DECOMP_OFF[i]..DECOMP_OFF[i+1]].
    let decomp_keys: Vec<u32> = {
        let mut k: Vec<u32> = full_decomp.keys().copied().collect();
        k.sort_unstable();
        k
    };
    let mut decomp_data: Vec<u32> = Vec::new();
    let mut decomp_off: Vec<u32> = Vec::with_capacity(decomp_keys.len() + 1);
    decomp_off.push(0);
    for &cp in &decomp_keys {
        decomp_data.extend_from_slice(&full_decomp[&cp]);
        decomp_off.push(decomp_data.len() as u32);
    }

    writeln!(out, "/// Code points with a canonical decomposition, sorted for binary search.").unwrap();
    emit_u32_slice(&mut out, "DECOMP_KEYS", &decomp_keys);
    writeln!(out, "/// Half-open [start, end) offsets into `DECOMP_DATA`, one pair per key.").unwrap();
    emit_u32_slice(&mut out, "DECOMP_OFF", &decomp_off);
    writeln!(out, "/// Flattened fully-recursive canonical decompositions.").unwrap();
    emit_u32_slice(&mut out, "DECOMP_DATA", &decomp_data);

    // Combining classes.
    let ccc_keys: Vec<u32> = ccc.keys().copied().collect();
    let ccc_vals: Vec<u8> = ccc_keys.iter().map(|k| ccc[k]).collect();
    writeln!(out, "/// Code points with a non-zero canonical combining class, sorted.").unwrap();
    emit_u32_slice(&mut out, "CCC_KEYS", &ccc_keys);
    writeln!(out, "/// Combining class for each `CCC_KEYS` entry.").unwrap();
    emit_u8_slice(&mut out, "CCC_VALS", &ccc_vals);

    // Primary composites: packed (a << 21 | b) keys, sorted, with values.
    let mut comp_pairs: Vec<(u64, u32)> = compose
        .iter()
        .map(|(&(a, b), &c)| (((a as u64) << 21) | b as u64, c))
        .collect();
    comp_pairs.sort_unstable_by_key(|&(k, _)| k);
    let comp_keys: Vec<u64> = comp_pairs.iter().map(|&(k, _)| k).collect();
    let comp_vals: Vec<u32> = comp_pairs.iter().map(|&(_, v)| v).collect();
    writeln!(out, "/// Primary composites, keyed by `(first << 21) | second`, sorted.").unwrap();
    emit_u64_slice(&mut out, "COMPOSE_KEYS", &comp_keys);
    writeln!(out, "/// The composite code point for each `COMPOSE_KEYS` entry.").unwrap();
    emit_u32_slice(&mut out, "COMPOSE_VALS", &comp_vals);

    // Titlecase mapping: TITLE_KEYS[i] titlecases to
    // TITLE_DATA[TITLE_OFF[i]..TITLE_OFF[i+1]] (only entries whose titlecase
    // differs from the uppercase mapping; all others fall back to uppercase).
    let title_keys: Vec<u32> = title_map.keys().copied().collect();
    let mut title_data: Vec<u32> = Vec::new();
    let mut title_off: Vec<u32> = Vec::with_capacity(title_keys.len() + 1);
    title_off.push(0);
    for &cp in &title_keys {
        title_data.extend_from_slice(&title_map[&cp]);
        title_off.push(title_data.len() as u32);
    }
    writeln!(out, "/// Code points whose titlecase differs from their uppercase mapping, sorted.").unwrap();
    emit_u32_slice(&mut out, "TITLE_KEYS", &title_keys);
    writeln!(out, "/// Half-open [start, end) offsets into `TITLE_DATA`, one pair per key.").unwrap();
    emit_u32_slice(&mut out, "TITLE_OFF", &title_off);
    writeln!(out, "/// Flattened titlecase mappings (may be multi-character).").unwrap();
    emit_u32_slice(&mut out, "TITLE_DATA", &title_data);

    fs::write(out_path, out).expect("write output");
    eprintln!(
        "wrote {} (Unicode {UNICODE_VERSION}): {} decomps ({} data words), {} ccc, {} composites, {} titlecase",
        out_path.display(),
        decomp_keys.len(),
        decomp_data.len(),
        ccc_keys.len(),
        comp_keys.len(),
        title_keys.len(),
    );
}

fn emit_u32_slice(out: &mut String, name: &str, data: &[u32]) {
    writeln!(out, "pub static {name}: [u32; {}] = [", data.len()).unwrap();
    emit_wrapped(out, data.iter().map(|v| format!("0x{v:X}")));
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
}

fn emit_u64_slice(out: &mut String, name: &str, data: &[u64]) {
    writeln!(out, "pub static {name}: [u64; {}] = [", data.len()).unwrap();
    emit_wrapped(out, data.iter().map(|v| format!("0x{v:X}")));
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
}

fn emit_u8_slice(out: &mut String, name: &str, data: &[u8]) {
    writeln!(out, "pub static {name}: [u8; {}] = [", data.len()).unwrap();
    emit_wrapped(out, data.iter().map(|v| v.to_string()));
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
}

fn emit_wrapped(out: &mut String, items: impl Iterator<Item = String>) {
    let mut col = 0;
    out.push_str("    ");
    for item in items {
        write!(out, "{item}, ").unwrap();
        col += 1;
        if col == 12 {
            out.push_str("\n    ");
            col = 0;
        }
    }
    if col != 0 {
        out.push('\n');
    }
}
