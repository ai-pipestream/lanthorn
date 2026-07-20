//! Build-time version string shared by the babelmap binaries.
//!
//! [`LONG`] is the crate (= workspace) version, with the short git commit hash
//! appended for builds that are not on an exact release tag — e.g.
//! `0.1.0 (23ab531)`, or `0.1.0 (23ab531-dirty)` with uncommitted changes. On a
//! release tag it is the clean version (`0.1.0`); with no git available (a source
//! tarball) it falls back to the plain crate version. Computed in `build.rs`.

/// The long version string (see the crate docs).
pub const LONG: &str = env!("BABELMAP_LONG_VERSION");
