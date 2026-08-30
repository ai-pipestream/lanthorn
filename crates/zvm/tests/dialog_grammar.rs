// SQ-1101: the two stories in the corpus that this reader refuses were compiled
// by **Dialog**, and Dialog emits no grammar table of any shape.
//
// ── How that was settled, so the next reader does not re-derive it ───────────
//
// The quest began as "modern Inform 7 stories are refused", which was wrong on
// both halves. `stories/frankenfingers_260330.z5` and `stories/ImpossibleStairs.z8`
// carry `Dia` in header bytes $39..$3B and their compiler's version in $3C..$3F
// — the slot Inform stamps `6.NN` into — and they say so themselves at the top
// of play: "Dialog compiler version 1a/01-dev" and "Dialog compiler version
// 0m/03. Library version 0.46."
//
// The signature is written unconditionally by `dialogc`'s `src/backend_z.c`
// (Dialog-IF/dialog), which also settles the substantive question:
//
//   * the string "grammar" does not appear anywhere in the compiler's sources.
//     Dialog's parser is library code — `(understand $ as $)` querying a
//     `(grammar entry $ $ $)` predicate defined in `stdlib.dg` — compiled to the
//     same predicate representation as any other rule, with no Z-machine table
//     to point at.
//   * static memory begins with the optimised alphabet table (when the story
//     uses one), then wordmaps, then data tables, then the dictionary. There is
//     no verb-pointer array at the base of static memory or anywhere else.
//
// So `Absent` is the correct answer for a Dialog story — not a coverage gap, and
// not the accident of a shape check tripping over a wordmap. `zvm::grammar`
// tests the signature FIRST for exactly that reason: the bytes these two happen
// to hold at static memory fail the shape checks, but the next Dialog story's
// need not, and a fabricated grammar is the one outcome this module exists to
// prevent.
//
// ── The specimens ────────────────────────────────────────────────────────────
//
// | fixture | release / serial | $38..$3F | Dialog |
// |---|---|---|---|
// | `stories/ImpossibleStairs.z8` | r3 / 241006 | `\0Dia0m03` | 0m/03 |
// | `stories/frankenfingers_260330.z5` | r1 / 260330 | `*Dia1a01` | 1a/01-dev |
//
// Byte $38 is `*` for a `-dev` build of the compiler and zero otherwise, which
// is why the two differ there and why frankenfingers' banner reads `-dev`.
//
// `stories/` is gitignored commercial media, so every case here skips vacuously
// without it. The synthetic half of this — that the SIGNATURE and not the bytes
// at static memory is what produces the refusal — is in `grammar.rs`'s own unit
// tests, which need no fixtures and run in CI.

use std::path::PathBuf;

use zvm::grammar::{Grammar, GrammarError};
use zvm::memory::Memory;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn story(name: &str) -> Option<Memory> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(Memory::new(bytes).expect("a story in stories/ is a valid image"))
}

/// The header signature, byte for byte, on both files — the fact every other
/// claim here rests on.
#[test]
fn both_dialog_stories_carry_the_compilers_signature() {
    for (name, dev, version) in [
        ("ImpossibleStairs.z8", false, "0m03"),
        ("frankenfingers_260330.z5", true, "1a01"),
    ] {
        let Some(mem) = story(name) else { continue };
        assert!(zvm::grammar::is_dialog(&mem), "{name} should be recognised as Dialog");

        let stamp: String = (0x3C..0x40).map(|a| mem.read_byte(a) as char).collect();
        assert_eq!(stamp, version, "{name} version stamp");

        // `*` marks a -dev build of the compiler; zero marks a release.
        let marker = mem.read_byte(0x38);
        assert_eq!(marker == b'*', dev, "{name} dev marker is {marker:#04x}");
    }
}

/// The answer this quest closes on: `Absent`, because there is nothing to read,
/// rather than `BadVerbTable`, which would claim a broken table exists.
#[test]
fn dialog_stories_report_absent_rather_than_a_broken_table() {
    for name in ["ImpossibleStairs.z8", "frankenfingers_260330.z5"] {
        let Some(mem) = story(name) else { continue };
        assert_eq!(
            Grammar::load(&mem).err(),
            Some(GrammarError::Absent),
            "{name} is a Dialog story and has no grammar table of any shape"
        );
    }
}

/// **The corpus census that makes the above durable.** Every Z-machine story on
/// disk is either Infocom's own (no stamp), Inform's (`6.NN` at $3C), or
/// Dialog's (`Dia` at $39) — and nothing else. Any story that is none of the
/// three is a producer nobody here has looked at, and is worth looking at before
/// trusting whatever `Grammar::load` said about it.
///
/// Deliberately not a count: an inventory of what `stories/` holds today rots,
/// where a rule that recomputes itself every run does not.
#[test]
fn every_z_machine_story_is_infocom_inform_or_dialog() {
    let Ok(entries) = std::fs::read_dir(stories_dir()) else {
        eprintln!("SKIP: stories/ absent");
        return;
    };
    let mut seen = 0;
    let mut dialog = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "z3" | "z4" | "z5" | "z6" | "z7" | "z8") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(mem) = Memory::new(bytes) else { continue };
        seen += 1;
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let is_dialog = zvm::grammar::is_dialog(&mem);
        let inform = mem.read_byte(0x3C) == b'6' && mem.read_byte(0x3D) == b'.';
        let infocom = (0x3C..0x40).all(|a| mem.read_byte(a) == 0);

        if is_dialog {
            dialog += 1;
            assert!(!inform, "{name} cannot be both Dialog and Inform");
            assert_eq!(
                Grammar::load(&mem).err(),
                Some(GrammarError::Absent),
                "{name} is Dialog, so it has no grammar table"
            );
        }
        assert!(
            is_dialog || inform || infocom,
            "{name} was produced by something this reader has never been shown: \
             $3C..$3F = {:02x?}",
            (0x3C..0x40).map(|a| mem.read_byte(a)).collect::<Vec<_>>()
        );
    }
    assert!(seen >= 10, "expected a corpus, saw {seen} stories");
    assert!(dialog >= 1, "expected the Dialog specimens among {seen} stories");
}
