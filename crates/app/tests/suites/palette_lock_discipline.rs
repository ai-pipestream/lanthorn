//! Two rules, checked mechanically, either side of the same process-global palette:
//! a suite that SETS it must do so through a guard (SQ-0905/SQ-0987), and a suite that
//! READS it must state which palette it read (SQ-0958).
//!
//! **A mutex only excludes the parties that take it.** SQ-0904 gave every suite that
//! already declared a `PALETTE` mutex one shared lock, which fixed twenty-three suites
//! that were each locking their own private mutex — indistinguishable from one lock
//! under nextest, from none under `cargo test`. But three files set
//! `zvm::screen::set_palette` without acquiring anything at all, and those do not
//! merely go unprotected: they can flip the palette in the middle of a lock-holder's
//! case, and the lock-holder has no way to notice.
//!
//! **And a lock only says "not while I hold this", never "and afterwards".** SQ-0959
//! made the shared lock a guard that puts `Palette::Standard` back on drop; SQ-0987
//! made the guard the only way to reach either the lock or the setter. The lock is
//! private to `app`, so a suite cannot take it raw — that is a compile error rather
//! than a convention — and `app::v6_set_palette` panics unless the calling thread
//! holds a guard, which is what lets the thirty harnesses that resolve a profile deep
//! inside their own `boot()` keep setting the palette exactly where they always did.
//! What this file adds is the source-level half: it fails a suite that reaches
//! `zvm::screen::set_palette` DIRECTLY, which is the one spelling that would slip past
//! both the privacy and the panic.
//!
//! Every file under `tests/suites/` is compiled into one of the ~14 group binaries,
//! so any two of them may share a process and run on parallel threads. That is what
//! makes this a rule about the directory rather than about individual files.
//!
//! # Why this case exists at all rather than a one-off sweep
//!
//! The sweep is the easy half. The hard half is that the next suite to set a palette
//! will be written by someone who has no reason to know any of the above, and the
//! failure it causes is invisible to the gate — `cargo nextest run` gives every test
//! its own process, so the race is structurally unobservable there, and CI's
//! `cargo test` is the only command that can see it. A source-level assertion is the
//! cheapest thing that catches it at the moment the file is added.
//!
//! # What is deliberately NOT covered
//!
//! `crates/zvm/tests/amiga_palette.rs` sets the palette and takes no lock, correctly:
//! it is its own test binary, it holds three cases, and both of its writes plus every
//! palette-dependent assertion sit inside ONE of them — the other two use only the
//! pure `amiga_true_colour`/`zmsd_true_colour`, which do not read the global. Its
//! isolation argument is at the binary level and holds. `zvm` also takes zero
//! external dependencies, so it could not share this lock even if it wanted to.
//!
//! Production callers are exempt for the same kind of reason: `startup.rs`,
//! `reset.rs` and `zvm-cli` are single-threaded and set one machine's palette per
//! run. So are `crates/app/examples/ring_scout.rs` and `flank_shape.rs`, which are
//! hand-run diagnostics with one boot each and no second thread to race — they are
//! the only raw callers left outside production, and they are outside this directory
//! deliberately rather than by oversight.

use std::path::PathBuf;

fn suites_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/suites")
}

/// Every `.rs` under `tests/suites/`, as (file name, source).
fn suite_sources() -> Vec<(String, String)> {
    let dir = suites_dir();
    let entries = std::fs::read_dir(&dir).expect("the suites directory is part of the checkout");
    let mut out: Vec<(String, String)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rs"))
        .map(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let src = std::fs::read_to_string(&p).expect("a suite in the checkout is readable");
            (name, src)
        })
        .collect();
    out.sort();
    out
}

/// A suite says which palette it is using by taking a guard: `app::v6_palette(p)` when
/// it can name the table at the lock site, `app::v6_palette_at_boot()` when its own
/// `boot()` names it a few rows further down. Both restore `Standard` on drop.
fn states_a_palette(src: &str) -> bool {
    src.contains("v6_palette(") || src.contains("v6_palette_at_boot(")
}

/// Does `src` name `set_palette` anywhere except as `app::v6_set_palette`?
///
/// A plain substring scan, which is the right trade for a guard nobody will maintain —
/// but it cannot tell a CALL from a MENTION, so a suite that merely names the raw
/// function in a comment is reported too. If that happens, reword the comment: the
/// spelling is what the next author copies, and prose that shows the forbidden form is
/// how the form comes back. It catches an aliased import (`use zvm::screen::set_palette
/// as sp;`) for the same reason — the `use` line names it.
fn reaches_set_palette_raw(src: &str) -> bool {
    src.match_indices("set_palette").any(|(at, _)| !src[..at].ends_with("v6_"))
}

/// The ONLY route from a suite to the process-global palette is a guard that puts the
/// default back.
///
/// Two of the three locks on that route are not source checks at all, and this case
/// exists to catch the one spelling neither of them can see:
///
/// | lock | catches | when |
/// |---|---|---|
/// | `app::V6_PALETTE_LOCK` is private | taking the lock raw, then setting and restoring nothing | compile |
/// | `app::v6_set_palette` asserts a thread-local witness | setting with no guard held | run |
/// | this case | calling `zvm::screen::set_palette` directly, past both | source |
///
/// That third row is not hypothetical. A direct call compiles — `zvm` is a dependency
/// of the group binaries and every suite already names `zvm::screen::Palette` — and
/// its damage is invisible to `cargo nextest run`, where every test owns its process.
/// It would produce a green local gate and an intermittent failure in some OTHER suite
/// under CI's `cargo test`, which is SQ-0904 exactly. It is also the only one of the
/// three that fires without the suite RUNNING, which matters here: nearly every case
/// in this directory is gated behind a gitignored story and skips vacuously on CI, so
/// the panic in `v6_set_palette` may never get the chance.
///
/// Falsified by putting `zvm::screen::set_palette(...)` back into any migrated suite,
/// which names that file here. Done, for SQ-0987, with `v6_shogun_title_header.rs`.
#[test]
fn the_only_route_to_the_palette_is_through_a_guard() {
    let (mut setters, mut raw) = (0usize, Vec::new());
    for (name, src) in suite_sources() {
        // This file spells the forbidden form out, in its prose and in its own scan.
        if name == "palette_lock_discipline.rs" {
            continue;
        }
        if src.contains("v6_set_palette(") {
            setters += 1;
        }
        if reaches_set_palette_raw(&src) {
            raw.push(name);
        }
    }
    assert!(
        raw.is_empty(),
        "these suites reach zvm::screen::set_palette directly: {raw:?}\n\
         The palette is process-global and every file under tests/suites/ shares a group binary — \
         and therefore a process and a palette — with a dozen others, which cargo test runs on \
         parallel threads. A raw call both races its neighbours and leaves the table installed for \
         the rest of the run, so the next suite to look reads a machine it never booted \
         (SQ-0904/SQ-0958/SQ-0959). Open each case with a guard:\n\
         \x20   let _g = app::v6_palette(zvm::screen::Palette::Standard);   // palette known here\n\
         \x20   let _g = app::v6_palette_at_boot();                        // named inside boot()\n\
         held for the whole case, and set the table from inside the harness with\n\
         \x20   app::v6_set_palette(profile.palette());",
    );
    assert!(
        setters >= 25,
        "only {setters} suites appear to set the palette — this case is looking in the wrong place \
         or matching the wrong string, and would pass vacuously",
    );
}

/// The markers for "this suite resolves a colour number through the global palette".
///
/// Booting is one: `GameSession::new*` runs the story to its first input, and the
/// header defaults and window properties 17/18 are filled from the active table on
/// the way. Rendering is the other: `render_story_pane` turns every z-colour in the
/// screen model into an RGB the buffer carries.
const RESOLVES_A_COLOUR: [&str; 2] = ["GameSession::new", "render_story_pane"];

/// The markers for "and something here depends on the answer".
///
/// Two shapes, and the second was learned the hard way. A resolved colour reaches an
/// assertion as a literal triple — `Color::Rgb(..)` out of ratatui, `Rgba([..])` out
/// of the image pipeline — and that is the obvious half. The other half asserts no
/// literal at all: it takes a painted surface and compares it with ANOTHER painted
/// surface, which depends on the palette not moving between the two even though it
/// never names one. MEASURED: sweeping only the literal half left
/// `v6_restore_paint_ground` out, and it went from green to failing every run,
/// because it compares a fresh boot's ground with an earlier one while
/// `v6_hardware_palette` flips the table under it — 4080 pixels differing on a
/// premise assert that had nothing to do with colour. Widening the sweep is what
/// fixed it, so the marker has to be wide enough to have caught it.
///
/// A suite that boots or renders but asserts only geometry, text or window structure
/// takes no view on the palette and is deliberately not swept in.
const ASSERTS_A_COLOUR: [&str; 6] =
    ["Rgb(", "Rgba(", "RgbaImage", "paint_surface", "pictures_canvas", "to_rgba8"];

/// Every suite that asserts a resolved colour also states the palette it resolved
/// through — [`app::v6_palette`], or [`app::v6_palette_at_boot`] when its own harness
/// names the table.
///
/// # Why this cannot be a runtime check
///
/// The writer rule above catches a CALL. This one catches an ABSENCE, and an absence
/// has no runtime signal at all: under `cargo nextest run` every test owns its
/// process, the inherited palette is therefore always `Standard`, and a suite that
/// assumed `Standard` is right by construction — so the gate is green whether the
/// rule holds or not. Under `cargo test` it is right until a sibling in the same
/// group binary boots a machine press, and then it fails intermittently, in the
/// reader, for a change made in the writer. SQ-0958 is that failure: two cases of
/// `v6_shogun_gameplay` read EGA white because `v6_shogun_title_header` boots the
/// same story under `InterpreterProfile::IbmPc`.
///
/// # What it deliberately does not cover
///
/// Only suites that assert a colour, by either shape in [`ASSERTS_A_COLOUR`]. A suite
/// that renders and asserts geometry, text or window structure has a weaker exposure
/// — a writer can still flip the palette mid-case, but nothing it checks can notice —
/// and sweeping all 111 booting suites into one lock would serialise most of the
/// app's integration tests to guard assertions that do not exist.
///
/// Falsified by emptying the `standard_palette()` helper in any swept suite, which
/// names that file here.
#[test]
fn a_suite_that_asserts_a_resolved_colour_states_its_palette() {
    let (mut readers, mut unstated) = (0usize, Vec::new());
    for (name, src) in suite_sources() {
        // This file names every marker in its own prose, and asserts no colour.
        if name == "palette_lock_discipline.rs" {
            continue;
        }
        if !RESOLVES_A_COLOUR.iter().any(|m| src.contains(m)) || !ASSERTS_A_COLOUR.iter().any(|m| src.contains(m)) {
            continue;
        }
        readers += 1;
        if !states_a_palette(&src) {
            unstated.push(name);
        }
    }
    assert!(
        unstated.is_empty(),
        "these suites assert a resolved colour through a palette they never stated: {unstated:?}\n\
         `Palette::Standard` is as much an assumption as any other, and a suite that means it \
         still has to say so — otherwise it is asserting against whatever the last suite in its \
         group binary left behind (SQ-0958). Open each case that boots or renders with:\n\
         \x20   let _g = app::v6_palette(zvm::screen::Palette::Standard);\n\
         naming the palette the assertions were actually written against, and hold the guard \
         until the last of them has run.",
    );
    assert!(
        readers >= 25,
        "only {readers} suites appear to assert a resolved colour — this case is matching the \
         wrong strings and would pass vacuously",
    );
}
