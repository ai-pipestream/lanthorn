//! How big the things lanthorn opens actually are.
//!
//! One measurement, cited by every cap that has to admit a real game. It exists
//! because the same wrong belief was written down twice, independently, in two
//! files, and both times as a confident sentence rather than a number:
//!
//! * `ifdb_search::MAX_DOWNLOAD` was 16 MiB under *"story files are small; a
//!   real Z-code/Glulx/blorb rarely exceeds a few MiB"* (SQ-1086).
//! * `hints::MAX_ZIP_ENTRY` was 4 MiB under *"nothing lanthorn runs is this
//!   big"* (SQ-1085).
//!
//! Both refused real games — silently, reported as "no story here" rather than
//! as a limit — and neither could be caught by the other's tests. "A few MiB"
//! described the Infocom era and was never re-measured: a modern Glulx game
//! carries its artwork and its sound inside the blorb.
//!
//! **The numbers are NAMED here, not measured from `stories/`.** That directory
//! is gitignored commercial media (see CLAUDE.md), so a check that reads it
//! passes vacuously wherever the fixtures are absent — which is CI, and which is
//! exactly how a stale cap survives a change that makes it wrong.
//!
//! | file | bytes |
//! |---|---|
//! | `Kerkerkruip.gblorb` | 22,109,534 |
//! | `Kerkerkruip.b10.gblorb` | 14,261,770 |
//! | `InfocomMasterpieces.img` | 12,582,912 |
//! | `Never Gives Up Her Dead.gblorb` | 11,680,602 |
//! | `CounterfeitMonkey-11.gblorb` | 11,308,550 |
//! | `cragne.gblorb` | 8,869,096 |
//!
//! A cap belongs to whoever enforces it — one bounds a download, another bounds
//! one inflated ZIP entry, and they may reasonably differ. What must not differ
//! is the floor each is checked against, so every such cap states its own
//! `const _: () = assert!(…)` against these figures and the build fails if one
//! is raised past what its cap admits.

/// The largest single game file in this project's corpus.
pub const LARGEST_GAME: u64 = 22_109_534; // stories/Kerkerkruip.gblorb

/// The largest disk image in this project's corpus — a compilation CD, which the
/// zip classifier already recognises and so may one day be asked to unpack.
pub const LARGEST_DISC: u64 = 12_582_912; // stories/InfocomMasterpieces.img
