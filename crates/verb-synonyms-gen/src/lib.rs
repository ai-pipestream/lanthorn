//! The offline generator behind `crates/verb-synonyms/src/player_verbs.tsv`.
//!
//! Three steps, each runnable on its own so that only the first needs a corpus
//! of story files:
//!
//! 1. [`harvest`] — read every verb the parsers in `stories/` and `unit_tests/`
//!    accept. Its output, `if_verbs.tsv`, is COMMITTED, so steps 2 and 3 are
//!    reproducible by anyone, including CI, with no game files at all.
//! 2. [`sources`] — read WordNet 3.0 and the 12dicts lemmatized frequency list.
//! 3. [`build`] — expand, invert, audit, and write the shipped table.
//!
//! See `README.md` in this crate for the exact source versions and the commands.

#![forbid(unsafe_code)]

pub mod build;
pub mod harvest;
pub mod sources;

#[cfg(test)]
mod tests;
