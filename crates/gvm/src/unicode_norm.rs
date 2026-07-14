//! Unicode canonical normalization (NFD / NFC) for the Glk buffer functions
//! `glk_buffer_canon_decompose_uni` and `glk_buffer_canon_normalize_uni`
//! (Glk spec §"Unicode String Normalization").
//!
//! Data tables (canonical decomposition map, combining-class map, primary
//! composites) are generated from the Unicode Character Database by
//! `crates/gvm/tables-gen` and checked in as [`unicode_norm_tables`] — gvm
//! itself stays zero-dependency with no build.rs. Hangul is handled
//! algorithmically here (UAX #15), so it needs no tables.
//!
//! The algorithms follow UAX #15: full recursive canonical decomposition +
//! canonical ordering (NFD); then canonical composition using the reference
//! Java sample from the Unicode normalization spec (NFC).

use crate::unicode_norm_tables as t;

/// The Unicode version backing the generated tables (provenance; asserted in
/// tests). Not read at runtime, hence the allow.
#[allow(dead_code)]
pub const UNICODE_VERSION: &str = t::UNICODE_VERSION;

// ── Hangul (UAX #15 §"Hangul") ───────────────────────────────────────────────
const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT; // 588
const S_COUNT: u32 = L_COUNT * N_COUNT; // 11172

fn is_hangul_syllable(cp: u32) -> bool {
    cp >= S_BASE && cp < S_BASE + S_COUNT
}

/// The Unicode titlecase mapping of `cp`, but only where it differs from the
/// (std `char::to_uppercase`) uppercase mapping — the digraphs, iota-subscript
/// Greek, Georgian, and the case ligatures. `None` means "titlecase == uppercase"
/// so the caller uppercases instead. Used by `glk_buffer_to_title_case_uni` for
/// the first character (Glk titlecases only buffer index 0).
pub fn titlecase(cp: u32) -> Option<&'static [u32]> {
    match t::TITLE_KEYS.binary_search(&cp) {
        Ok(i) => {
            let start = t::TITLE_OFF[i] as usize;
            let end = t::TITLE_OFF[i + 1] as usize;
            Some(&t::TITLE_DATA[start..end])
        }
        Err(_) => None,
    }
}

/// Canonical combining class of a code point (0 for a starter).
pub fn combining_class(cp: u32) -> u8 {
    match t::CCC_KEYS.binary_search(&cp) {
        Ok(i) => t::CCC_VALS[i],
        Err(_) => 0,
    }
}

/// Append the full canonical decomposition of `cp` to `out`. Handles Hangul
/// algorithmically; table code points expand to their fully-recursive form;
/// everything else is itself.
fn decompose_char(cp: u32, out: &mut Vec<u32>) {
    if is_hangul_syllable(cp) {
        let s_index = cp - S_BASE;
        let l = L_BASE + s_index / N_COUNT;
        let v = V_BASE + (s_index % N_COUNT) / T_COUNT;
        let tt = T_BASE + s_index % T_COUNT;
        out.push(l);
        out.push(v);
        if tt != T_BASE {
            out.push(tt);
        }
        return;
    }
    if let Ok(i) = t::DECOMP_KEYS.binary_search(&cp) {
        let start = t::DECOMP_OFF[i] as usize;
        let end = t::DECOMP_OFF[i + 1] as usize;
        out.extend_from_slice(&t::DECOMP_DATA[start..end]);
        return;
    }
    out.push(cp);
}

/// Canonical ordering: stably sort every maximal run of combining marks
/// (ccc != 0) by ascending combining class (UAX #15 §"Canonical Ordering").
fn canonical_order(buf: &mut [u32]) {
    let n = buf.len();
    let mut i = 0;
    while i < n {
        if combining_class(buf[i]) == 0 {
            i += 1;
            continue;
        }
        // find the end of this combining run
        let mut j = i;
        while j < n && combining_class(buf[j]) != 0 {
            j += 1;
        }
        // insertion sort keeps it stable (equal ccc preserve input order)
        for a in (i + 1)..j {
            let val = buf[a];
            let vc = combining_class(val);
            let mut b = a;
            while b > i && combining_class(buf[b - 1]) > vc {
                buf[b] = buf[b - 1];
                b -= 1;
            }
            buf[b] = val;
        }
        i = j;
    }
}

/// NFD: full canonical decomposition with canonical ordering.
pub fn canonical_decompose(input: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(input.len());
    for &cp in input {
        decompose_char(cp, &mut out);
    }
    canonical_order(&mut out);
    out
}

/// Look up the primary composite of a starter + following character, if one
/// exists and is not a composition exclusion. Hangul composes algorithmically.
fn compose_pair(a: u32, b: u32) -> Option<u32> {
    // Hangul L + V
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        let l_index = a - L_BASE;
        let v_index = b - V_BASE;
        return Some(S_BASE + (l_index * V_COUNT + v_index) * T_COUNT);
    }
    // Hangul LV + T
    if is_hangul_syllable(a) && (a - S_BASE) % T_COUNT == 0 {
        if (T_BASE + 1..T_BASE + T_COUNT).contains(&b) {
            return Some(a + (b - T_BASE));
        }
    }
    let key = ((a as u64) << 21) | b as u64;
    match t::COMPOSE_KEYS.binary_search(&key) {
        Ok(i) => Some(t::COMPOSE_VALS[i]),
        Err(_) => None,
    }
}

/// NFC: canonical decomposition followed by canonical composition. Implements
/// the reference algorithm from the Unicode normalization spec (the Java
/// `compose` sample also used by cheapglk's `cgunicod.c`).
pub fn canonical_compose(input: &[u32]) -> Vec<u32> {
    let mut buf = canonical_decompose(input);
    if buf.is_empty() {
        return buf;
    }
    let mut starter_pos = 0usize; // index in buf of the current starter
    let mut starter_ch = buf[0];
    let mut comp_pos = 1usize; // length of the composed prefix built so far
    // ccc of the last char written; a leading non-starter is treated as 256 so
    // nothing composes onto it.
    let mut last_class: i32 = match combining_class(starter_ch) {
        0 => 0,
        c => c as i32 + 256,
    };
    for decomp_pos in 1..buf.len() {
        let ch = buf[decomp_pos];
        let ch_class = combining_class(ch) as i32;
        let composite = compose_pair(starter_ch, ch);
        if let Some(comp) = composite {
            if last_class < ch_class || last_class == 0 {
                buf[starter_pos] = comp;
                starter_ch = comp;
                continue;
            }
        }
        if ch_class == 0 {
            starter_pos = comp_pos;
            starter_ch = ch;
        }
        last_class = ch_class;
        buf[comp_pos] = ch;
        comp_pos += 1;
    }
    buf.truncate(comp_pos);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_pinned() {
        assert_eq!(UNICODE_VERSION, "16.0.0");
    }

    // é: U+00E9 ↔ U+0065 U+0301, both directions.
    #[test]
    fn e_acute_round_trip() {
        assert_eq!(canonical_decompose(&[0x00E9]), vec![0x0065, 0x0301]);
        assert_eq!(canonical_compose(&[0x0065, 0x0301]), vec![0x00E9]);
        assert_eq!(canonical_compose(&[0x00E9]), vec![0x00E9]);
        assert_eq!(canonical_decompose(&[0x0065, 0x0301]), vec![0x0065, 0x0301]);
    }

    // Multiple combining marks reorder by combining class (ccc): dot-below
    // (0323, ccc 220) sorts before dot-above (0307, ccc 230), regardless of
    // input order, and both orders normalize identically.
    #[test]
    fn combining_marks_ordered_by_ccc() {
        let a = canonical_decompose(&[0x0064, 0x0323, 0x0307]);
        let b = canonical_decompose(&[0x0064, 0x0307, 0x0323]);
        assert_eq!(a, vec![0x0064, 0x0323, 0x0307]);
        assert_eq!(a, b);
        // Same for NFC.
        assert_eq!(
            canonical_compose(&[0x0064, 0x0307, 0x0323]),
            canonical_compose(&[0x0064, 0x0323, 0x0307])
        );
    }

    // Composition exclusion: U+0958 decomposes to U+0915 U+093C but must NOT
    // re-compose under NFC (it is on the script-specific exclusion list).
    #[test]
    fn composition_exclusion_does_not_recompose() {
        assert_eq!(canonical_decompose(&[0x0958]), vec![0x0915, 0x093C]);
        assert_eq!(canonical_compose(&[0x0958]), vec![0x0915, 0x093C]);
        assert_eq!(canonical_compose(&[0x0915, 0x093C]), vec![0x0915, 0x093C]);
    }

    // Hangul, handled algorithmically: LV syllable and LVT syllable both ways.
    #[test]
    fn hangul_algorithmic() {
        // U+AC00 (가) ↔ U+1100 U+1161 (LV)
        assert_eq!(canonical_decompose(&[0xAC00]), vec![0x1100, 0x1161]);
        assert_eq!(canonical_compose(&[0x1100, 0x1161]), vec![0xAC00]);
        // U+AC01 (각) ↔ U+1100 U+1161 U+11A8 (LVT)
        assert_eq!(canonical_decompose(&[0xAC01]), vec![0x1100, 0x1161, 0x11A8]);
        assert_eq!(canonical_compose(&[0x1100, 0x1161, 0x11A8]), vec![0xAC01]);
        // LV + T composes in two steps (L+V → LV, then LV+T → LVT).
        assert_eq!(canonical_compose(&[0xAC00, 0x11A8]), vec![0xAC01]);
    }

    // Singleton decomposition: U+212B (Å ANGSTROM SIGN) → NFC U+00C5.
    #[test]
    fn singleton_decomposition() {
        assert_eq!(canonical_decompose(&[0x212B]), vec![0x0041, 0x030A]);
        assert_eq!(canonical_compose(&[0x212B]), vec![0x00C5]);
        // U+2126 OHM SIGN → GREEK OMEGA (singleton, no recompose).
        assert_eq!(canonical_compose(&[0x2126]), vec![0x03A9]);
    }

    /// A spot subset of the official Unicode conformance data, embedded as
    /// literals (download at test time is not allowed). Each row is
    /// `source ; NFC ; NFD` (the first three columns of a line in
    /// `NormalizationTest.txt`, Unicode 16.0.0 —
    /// https://www.unicode.org/Public/16.0.0/ucd/NormalizationTest.txt).
    /// Chosen to exercise precomposed Latin/Greek/Cyrillic, combining-mark
    /// ordering, composition exclusions, singletons, and Hangul LV/LVT.
    const NORM_VECTORS: &[&str] = &[
        "1E0A;1E0A;0044 0307",
        "1E0A 0323;1E0C 0307;0044 0323 0307",
        "1E0C 0307;1E0C 0307;0044 0323 0307",
        "1E0A 031B;1E0A 031B;0044 031B 0307",
        "1E0A 031B 0323;1E0C 031B 0307;0044 031B 0323 0307",
        "00C0;00C0;0041 0300",
        "00C5;00C5;0041 030A",
        "00E9;00E9;0065 0301",
        "0100;0100;0041 0304",
        "0134;0134;004A 0302",
        "01FC;01FC;00C6 0301",
        "0344;0308 0301;0308 0301",
        "03D3;03D3;03D2 0301",
        "0407;0407;0406 0308",
        "04D0;04D0;0410 0306",
        "0958;0915 093C;0915 093C",
        "0F73;0F71 0F72;0F71 0F72",
        "0F75;0F71 0F74;0F71 0F74",
        "0F81;0F71 0F80;0F71 0F80",
        "1E9B;1E9B;017F 0307",
        "1EAC;1EAC;0041 0323 0302",
        "1FEE;0385;00A8 0301",
        "2126;03A9;03A9",
        "212B;00C5;0041 030A",
        "AC00;AC00;1100 1161",
        "AC01;AC01;1100 1161 11A8",
        "D4DB;D4DB;1111 1171 11B6",
        "FB1F;05F2 05B7;05F2 05B7",
        "00C5 0301;01FA;0041 030A 0301",
        "212B 0341;01FA;0041 030A 0301",
    ];

    fn parse(col: &str) -> Vec<u32> {
        col.split_whitespace()
            .map(|h| u32::from_str_radix(h, 16).unwrap())
            .collect()
    }

    #[test]
    fn normalization_test_subset() {
        for row in NORM_VECTORS {
            let cols: Vec<&str> = row.split(';').collect();
            let source = parse(cols[0]);
            let nfc = parse(cols[1]);
            let nfd = parse(cols[2]);
            assert_eq!(
                canonical_decompose(&source),
                nfd,
                "NFD mismatch for row {row:?}"
            );
            assert_eq!(
                canonical_compose(&source),
                nfc,
                "NFC mismatch for row {row:?}"
            );
        }
    }
}
