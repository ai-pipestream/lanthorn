//! Styling-role redesign (SQ-0309): the declarative selector registry and the
//! resolver/theme model built on top of it.
//!
//! [`registry`] is the single source of truth — one row per themeable selector
//! (name, section, kind, parent, default delta). Later tasks add the resolver
//! and TOML schema that consume it.

pub mod registry;
pub mod resolve;
pub mod template;
pub mod toml_schema;
