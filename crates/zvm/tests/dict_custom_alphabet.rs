// Regression: dictionary lookup must encode player input with the story's
// CUSTOM alphabet table (header 0x34), including letters the game relocated
// into A1. Shogun (v6) moves the lowercase letters j/q/v/x/z out of A0 and into
// the front of A1, so any typed word containing one of them (save, quit, quill,
// japan, …) used to encode via the 10-bit escape and miss the game's own
// dictionary — «I don't know the word "save".» (SQ-0517).
//
// The encoder now searches A0, then A1 (shift-4), then A2 (shift-5), then the
// 10-bit escape, mirroring the decoder's alphabet rows.

use zvm::dictionary::load;
use zvm::memory::Memory;

/// Build a minimal but structurally valid v-`version` story buffer, mirroring
/// the crate-internal `header::tests_support::sample_story` (not reachable from
/// an integration test). Dynamic memory is 0x0000–0x03FF.
fn sample_story(version: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 0x400];
    buf[0x00] = version;
    buf[0x04] = 0x04; // high_mem_base = 0x0400
    buf[0x06] = 0x00;
    buf[0x07] = 0x40; // initial_pc = 0x0040
    buf[0x08] = 0x02; // dictionary = 0x0200
    buf[0x0A] = 0x01; // object_table = 0x0100
    buf[0x0C] = 0x03; // global_vars = 0x0300
    buf[0x0E] = 0x04; // static_mem_base = 0x0400
    buf[0x18] = 0x00;
    buf[0x19] = 0x40; // abbrev_table = 0x0040
    buf
}

// ---------------------------------------------------------------------------
// Synthetic mechanism test — no story asset required.
// ---------------------------------------------------------------------------

#[test]
fn custom_alphabet_relocated_and_shifted_letters_resolve() {
    // A v5 story with a custom alphabet table at 0x0200 that:
    //   * relocates a letter WITHIN A0 ('z' at A0[0], 'a' pushed to A0[1]), and
    //   * moves a lowercase letter into A1 ('q' at A1[0]).
    // A tiny 2-entry dictionary at 0x0250 holds keys for "zap" and "aqua",
    // encoded with that same table. Lookup of each must succeed; a made-up word
    // must miss.
    let tbl: usize = 0x0200;
    // A0 (26 glyphs): 'z' relocated to index 0, then a..p (indices 1..16),
    // then r..y (indices 17..24), '_' filler at 25. 'q' is absent from A0 and
    // lives only in A1 — exactly Shogun's shape in miniature. Index checks:
    // 'z'@0, 'a'@1, 'p'@16, 'u'@20 (used by the hand-packed keys below).
    let a0 = *b"zabcdefghijklmnoprstuvwxy_";
    // A1: 'q' at position 0, then the rest of the uppercase alphabet.
    let a1 = *b"qABCDEFGHIJKLMNOPRSTUVWXYZ";
    let a2 = *b"\x00\n0123456789.,!?_#'\"/\\-:()";

    let mut bytes = sample_story(5);
    bytes[0x34] = (tbl >> 8) as u8;
    bytes[0x35] = (tbl & 0xFF) as u8;
    for (i, &c) in a0.iter().enumerate() {
        bytes[tbl + i] = c;
    }
    for (i, &c) in a1.iter().enumerate() {
        bytes[tbl + 26 + i] = c;
    }
    for (i, &c) in a2.iter().enumerate() {
        bytes[tbl + 52 + i] = c;
    }

    // Encode the two keys the SAME way the game's compiler would, using this
    // table, by hand — proving the encoder must agree byte-for-byte.
    //   "zap": z=A0[0]→zc6, a=A0[1]→zc7, p=A0[16]→zc22.  9 zchars, pad 5.
    //   "aqua": a=A0[1]→zc7, q=A1[0]→shift4(zc4)+zc6, u=A0[20]→zc26, a=zc7.
    let zap = pack9(&[6, 7, 22]);
    let aqua = pack9(&[7, 4, 6, 26, 7]);

    // Dictionary at 0x0250: 0 separators, entry_length 6, count 2 (sorted).
    // Sort the two 6-byte keys so the binary search is valid.
    let mut keys = [zap, aqua];
    keys.sort();
    let dict: usize = 0x0250;
    bytes[0x08] = (dict >> 8) as u8;
    bytes[0x09] = (dict & 0xFF) as u8;
    bytes[dict] = 0; // 0 separators
    bytes[dict + 1] = 6; // entry_length
    bytes[dict + 2] = 0; // count hi
    bytes[dict + 3] = 2; // count lo = 2 (positive ⇒ sorted)
    for (i, key) in keys.iter().enumerate() {
        let base = dict + 4 + i * 6;
        bytes[base..base + 6].copy_from_slice(key);
    }

    let m = Memory::new(bytes).unwrap();
    let d = load(&m);

    assert_ne!(d.lookup(&m, "zap"), 0, "'zap' (A0-relocated 'z') must resolve");
    assert_ne!(
        d.lookup(&m, "aqua"),
        0,
        "'aqua' (A1-shifted 'q') must resolve — the pre-fix encoder missed this"
    );
    assert_eq!(d.lookup(&m, "xyzzy"), 0, "an absent word must still miss");
}

/// Pack up to 9 Z-chars (padding with Z-char 5) into a 6-byte v4+ key with the
/// terminator high bit on the final word.
fn pack9(zchars: &[u8]) -> [u8; 6] {
    let mut z = [5u8; 9];
    for (i, &c) in zchars.iter().take(9).enumerate() {
        z[i] = c;
    }
    let mut out = [0u8; 6];
    for w in 0..3 {
        let mut word = ((z[w * 3] as u16) << 10) | ((z[w * 3 + 1] as u16) << 5) | z[w * 3 + 2] as u16;
        if w == 2 {
            word |= 0x8000;
        }
        out[w * 2] = (word >> 8) as u8;
        out[w * 2 + 1] = (word & 0xFF) as u8;
    }
    out
}

// ---------------------------------------------------------------------------
// Real-Shogun test — skips cleanly if the story asset is absent.
// ---------------------------------------------------------------------------

fn load_shogun() -> Option<Vec<u8>> {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../stories/shogun-r322-s890706.z6");
    std::fs::read(&path).ok()
}

#[test]
fn shogun_custom_alphabet_words_resolve() {
    let Some(story) = load_shogun() else { return };
    let m = Memory::new(story).unwrap();
    let d = load(&m);

    // Words whose letters Shogun relocated into A1 (v/q/j) — every one used to
    // miss with «I don't know the word …».
    for word in ["save", "quit", "quill", "japan"] {
        let toks = d.tokenise(&m, word.as_bytes());
        assert_eq!(toks.len(), 1, "{word:?} should tokenise to one token");
        assert_ne!(
            toks[0].dict_addr, 0,
            "{word:?} must resolve in Shogun's dictionary (custom-alphabet A1 letter)"
        );
    }

    // Control: an all-A0 word already worked, and a genuine non-word still misses.
    assert_ne!(d.tokenise(&m, b"look")[0].dict_addr, 0, "'look' must resolve");
    assert_eq!(d.tokenise(&m, b"xyzzy")[0].dict_addr, 0, "'xyzzy' must miss");
}
