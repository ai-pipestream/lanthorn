//! The save/restore prompt lists what exists, and a number picks it (SQ-0918).
//!
//! The helpers behind this are unit-tested in `cli_host::storage`; what these cases
//! cover is the WIRING, which is the half that can be wrong while every helper is
//! right — the list printed at the wrong prompt, the number resolved for a save
//! instead of a restore, or the whole thing simply not called.
//!
//! Driven through the real binary with piped stdin, on `disk_image.rs`'s pattern.
//! A synthetic story is enough: this is about the prompt, not about any game.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A scratch directory of this test's own, so the saves it writes cannot be seen by
/// another case. `cargo test` shares one process across these, so the thread id is
/// what keeps two apart.
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "zvm-cli-saveprompt-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn story() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3");
    p.is_file().then_some(p).or_else(|| {
        let alt = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Zork I - The Great Underground Empire.adf");
        alt.is_file().then_some(alt)
    })
}

fn run(story: &Path, dir: &Path, script: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zvm-cli"))
        .arg(story)
        .arg("--screen-reader")
        .arg("--data-dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zvm-cli spawns");
    child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    child.wait_with_output().expect("zvm-cli runs")
}

/// **The first save shows no list, the second shows the first, and a number
/// restores it.**
///
/// One session rather than three, because the interesting part is the sequence: a
/// list that appears only once something exists, and an index that means what the
/// list said it meant.
#[test]
fn the_prompt_lists_existing_saves_and_a_number_restores_one() {
    let Some(story) = story() else {
        eprintln!("SKIP: gitignored Zork I fixture absent");
        return;
    };
    let dir = scratch("listing");
    let out = run(
        &story,
        &dir,
        "\n\nsave\ncellar\nsave\ntroll\nrestore\n1\nquit\ny\n",
    );
    let text = String::from_utf8_lossy(&out.stdout);

    // Split on the prompts so each one's preceding list can be judged separately.
    let saves: Vec<&str> = text.match_indices("Save to file:").map(|(i, _)| &text[..i]).collect();
    assert_eq!(saves.len(), 2, "two save prompts in this script:\n{text}");
    assert!(!saves[0].contains("saves:"), "the FIRST save has nothing to list:\n{}", saves[0]);
    assert!(
        saves[1].contains("saves: 1 cellar"),
        "the second lists the first:\n{}",
        &saves[1][saves[0].len()..],
    );

    let before_restore = text.split("Restore from file:").next().unwrap_or("");
    assert!(
        before_restore.contains("saves: 1 cellar   2 troll"),
        "the restore prompt lists both, numbered and sorted:\n{before_restore}",
    );
    // `1` resolved to `cellar` rather than being taken as a filename — a filename
    // `1` does not exist, so the restore would have failed instead.
    assert!(
        !text.contains("Restore failed") && !text.contains("restore failed"),
        "typing 1 should restore `cellar`, not look for a file called 1:\n{text}",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A number at the SAVE prompt is a filename, not an overwrite.**
///
/// The asymmetry is deliberate: silently clobbering a save the player named is the
/// defect SQ-0648 fixed in the TUI, so an overwrite here has to be spelled out.
/// Falsified by routing `pick_save` through the save arm, which writes `cellar.qzl`
/// a second time and leaves no `1.qzl` behind.
#[test]
fn a_number_at_the_save_prompt_is_a_filename() {
    let Some(story) = story() else {
        eprintln!("SKIP: gitignored Zork I fixture absent");
        return;
    };
    let dir = scratch("nooverwrite");
    let _ = run(&story, &dir, "\n\nsave\ncellar\nsave\n1\nquit\ny\n");

    let mut found: Vec<String> = walk(&dir);
    found.sort();
    assert!(found.iter().any(|f| f == "1.qzl"), "a save called 1 was written: {found:?}");
    assert!(found.iter().any(|f| f == "cellar.qzl"), "and cellar survived it: {found:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every `.qzl` under `dir`, at any depth — the game directory is nested under the
/// data dir by a key this test has no reason to know.
fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("qzl")) {
            out.push(p.file_name().unwrap_or_default().to_string_lossy().into_owned());
        }
    }
    out
}
