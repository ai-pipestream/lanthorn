//! Shared fixture-path resolution (SQ-1015).
//!
//! 125 suites under `tests/suites/` used to each define their own private
//! `stories_dir()` pointing solely at the gitignored, commercial `stories/`
//! directory, so every one of them skipped vacuously on CI. A survey found 39
//! of those suites depend only on fixtures that are already freely
//! redistributable — `advent`, `scopa`, `sunburst`, the Mysterious Adventures,
//! `anchor.z8`, `photopia`, and several modern Glulx works — and moved them to
//! `tests/fixtures/stories/`, which `git ls-files` can see.
//!
//! [`fixture_path`] is the one place that duplication now goes through: it takes
//! the local `stories/` copy when there is one and the tracked copy otherwise, so
//! a developer's run is unchanged and CI — which has no `stories/` — still reaches
//! every tracked fixture. A suite that names a fixture never moved here behaves
//! exactly as it did before: a superset, never a narrowing.
use std::path::{Path, PathBuf};

/// Resolve `name` against the tracked fixtures directory first, then the
/// gitignored local `stories/`. Returns a path either way (possibly
/// non-existent) so callers keep their existing `std::fs::read(..).ok()?`
/// "skip if absent" pattern unchanged.
pub fn fixture_path(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Mini-Zork I (r34/s871124) already has a tracked, sha256-verified copy
    // at `crates/zvm/tests/fixtures/minizork.z3` (IF Archive demos/minizork.z3),
    // read by `pty_query_replies.rs` the same way. Route the several suites
    // that ask for it by its `stories/`-era filename there instead of
    // duplicating the binary.
    if name == "minizork-r34-s871124.z3" {
        return manifest.join("../zvm/tests/fixtures/minizork.z3");
    }

    // LOCAL FIRST, tracked as the fallback — because a story is not the only file
    // a story needs (SQ-1048).
    //
    // The tracked directory holds bare story files; their companion media stay in
    // `stories/`, which is where a release's own graphics, sound and disk resources
    // live. Preferring the tracked copy therefore hands back a story SEPARATED from
    // its resources: `mysterious01.z6` moved here while `Mysterious01.blb` did not,
    // so the title lost its pictures, stopped taking the hybrid ring, and rendered
    // as a full-frame image — which is exactly what `fmvpoker_is_the_only_title_this_moves`
    // exists to catch.
    //
    // Taking the local copy when there is one means a developer's run is bit-for-bit
    // what it was before any fixture moved, and CI — which has no `stories/` at all —
    // still reaches every tracked fixture. That is the "superset, never a narrowing"
    // this module promises; tracked-first quietly broke it for any fixture with a
    // sibling.
    let local = manifest.join("../../stories").join(name);
    if local.is_file() {
        return local;
    }
    let tracked = manifest.join("tests/fixtures/stories").join(name);
    if tracked.is_file() {
        return tracked;
    }
    // Neither has it — answer in `stories/`, which is where this always pointed and
    // what several callers actually want. `picture_override` asks for
    // `fixture_path("anything.z6")`, a name deliberately on no disk, purely to name
    // the DIRECTORY its real fixtures (`zork0.pic`, `zork0.eg1`) sit beside. Falling
    // back to the tracked directory instead moves that parent and the sidecars vanish.
    local
}
