//! The single-layer resolver: flatten [`REGISTRY`] into a concrete theme map.
//!
//! Each registry row derives from a parent (a role, or another selector) via a
//! [`Delta`]. This pass starts from the parent's resolved [`Style`], layers the
//! row's `default_delta`, then applies any matching explicit override from
//! `Decls`. The result is a flat `name -> Resolved` map queried by [`Theme::get`].
//!
//! This is the **single-`Decls`** resolver (SQ-0309 Task 0.2). Provenance and the
//! layered (global / garglk / per-game) decls arrive in Task 0.3 and extend the
//! `resolve` signature; nothing here records where a value came from.

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};

use super::registry::{Delta, RegRow, REGISTRY, ROLE_NAMES};

/// The 7 resolved role roots (§1). Everything else derives from one of these.
///
/// Concrete colours come from the base scheme + `[roles]`; here we hold the
/// already-resolved [`Style`] per role. [`Roles::terminal_default`] provides the
/// spec's default (dark) role palette so callers/tests have a concrete input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roles {
    pub text: Style,
    pub chrome: Style,
    pub border: Style,
    pub accent: Style,
    pub muted: Style,
    pub alert: Style,
    pub heading: Style,
}

impl Roles {
    /// The spec's default (dark) role palette (design §1 / the `[roles]` example):
    /// text = white on terminal bg, chrome = white on black, border/accent = cyan,
    /// muted = dark-gray, alert = yellow, heading = white + bold.
    pub fn terminal_default() -> Roles {
        Roles {
            text: Style::default().fg(Color::White),
            chrome: Style::default().fg(Color::White).bg(Color::Black),
            border: Style::default().fg(Color::Cyan),
            accent: Style::default().fg(Color::Cyan),
            muted: Style::default().fg(Color::DarkGray),
            alert: Style::default().fg(Color::Yellow),
            heading: Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        }
    }

    /// Look up a role's [`Style`] by its name (one of [`ROLE_NAMES`]).
    fn by_name(&self, name: &str) -> Option<Style> {
        Some(match name {
            "text" => self.text,
            "chrome" => self.chrome,
            "border" => self.border,
            "accent" => self.accent,
            "muted" => self.muted,
            "alert" => self.alert,
            "heading" => self.heading,
            _ => return None,
        })
    }
}

/// The glyph(s) a resolved selector carries. Mirrors [`Delta`]'s two glyph slots:
/// `single` is one glyph (gutter marks, tab dividers, terminator caps, or a
/// [`Kind::Placement`](super::registry::Kind::Placement) preset name); `slots` is
/// a small named-glyph set (box/arrow slots). Owned so a [`Theme`] is self-contained.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphSet {
    pub single: Option<String>,
    pub slots: Vec<(String, String)>,
}

impl GlyphSet {
    fn is_empty(&self) -> bool {
        self.single.is_none() && self.slots.is_empty()
    }
}

/// A fully resolved selector: its concrete [`Style`] and any glyph(s) it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub style: Style,
    pub glyph: Option<GlyphSet>,
}

/// Explicit per-selector overrides, layered on top of the registry default. For
/// Task 0.2 this is a single flat map; Task 0.3 layers several of these.
pub type Decls = HashMap<String, Delta>;

/// The flat resolved theme: `selector name -> Resolved`.
#[derive(Debug, Clone)]
pub struct Theme {
    map: HashMap<String, Resolved>,
    /// Fallback for an unknown selector — the `text` role.
    fallback: Resolved,
}

impl Theme {
    /// Resolve a selector. An unknown selector falls back to the `text` role
    /// (the body-ink default) so a stray name never panics or renders unstyled.
    pub fn get(&self, sel: &str) -> Resolved {
        self.map.get(sel).cloned().unwrap_or_else(|| self.fallback.clone())
    }
}

/// Layer a [`Delta`] onto a base [`Style`]: override fg/bg only where the delta
/// sets them; add (never clear) the delta's modifier bits.
fn apply_style(base: Style, d: &Delta) -> Style {
    let mut s = base;
    if let Some(fg) = d.fg {
        s = s.fg(fg);
    }
    if let Some(bg) = d.bg {
        s = s.bg(bg);
    }
    let mut m = Modifier::empty();
    if d.bold {
        m |= Modifier::BOLD;
    }
    if d.italic {
        m |= Modifier::ITALIC;
    }
    if d.underline {
        m |= Modifier::UNDERLINED;
    }
    if d.reversed {
        m |= Modifier::REVERSED;
    }
    if d.dim {
        m |= Modifier::DIM;
    }
    if !m.is_empty() {
        s = s.add_modifier(m);
    }
    s
}

/// Layer a [`Delta`]'s glyph channels onto an inherited [`GlyphSet`]: a set glyph
/// or glyph-slot list overrides the inherited one; otherwise it carries through.
fn apply_glyph(inherited: Option<GlyphSet>, d: &Delta) -> Option<GlyphSet> {
    let mut g = inherited.unwrap_or_default();
    if let Some(single) = d.glyph {
        g.single = Some(single.to_string());
    }
    if !d.glyphs.is_empty() {
        g.slots = d.glyphs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    }
    if g.is_empty() {
        None
    } else {
        Some(g)
    }
}

/// Resolve one row against a known parent `Resolved` (or a bare parent style when
/// the row has no parent), layering the registry default then any `Decls` override.
fn resolve_row(row: &RegRow, parent: &Resolved, decls: &Decls) -> Resolved {
    // 1. registry default delta on the parent.
    let mut style = apply_style(parent.style, &row.default_delta);
    let mut glyph = apply_glyph(parent.glyph.clone(), &row.default_delta);
    // 2. explicit override, if any, on top.
    if let Some(over) = decls.get(row.name) {
        style = apply_style(style, over);
        glyph = apply_glyph(glyph, over);
    }
    Resolved { style, glyph }
}

/// Compute the flat theme map from the registry via single-level parent fallback.
///
/// Roles resolve first (from `roles`); then each row resolves against its parent —
/// a role, or another already-resolved selector. A parent that is another selector
/// is handled generally: rows resolve in dependency order via a fixpoint loop, so a
/// parent is always resolved before its child (the registry currently only parents
/// roles, for which one pass suffices).
pub fn resolve(roles: &Roles, decls: &Decls) -> Theme {
    let mut map: HashMap<String, Resolved> = HashMap::new();

    // Roles first: their Resolved is the bare role style (no glyph, no delta).
    for name in ROLE_NAMES {
        let style = roles.by_name(name).expect("ROLE_NAMES entry has a role style");
        // A role row may still carry an explicit override.
        let base = Resolved { style, glyph: None };
        let row = REGISTRY.iter().find(|r| r.name == name);
        let resolved = match row {
            Some(r) => resolve_row(r, &Resolved { style, glyph: None }, decls),
            None => base,
        };
        map.insert(name.to_string(), resolved);
    }

    // Everything else: resolve in dependency order. A row is resolvable once its
    // parent is a role (already in `map`), None (a bare root), or another selector
    // already resolved. Loop to a fixpoint so selector->selector parents work.
    let mut pending: Vec<&RegRow> =
        REGISTRY.iter().filter(|r| !ROLE_NAMES.contains(&r.name)).collect();

    loop {
        let before = pending.len();
        pending.retain(|row| {
            let parent = match row.parent {
                None => Resolved { style: Style::default(), glyph: None },
                Some(p) => match map.get(p) {
                    Some(res) => res.clone(),
                    None => return true, // parent not resolved yet; keep pending.
                },
            };
            let resolved = resolve_row(row, &parent, decls);
            map.insert(row.name.to_string(), resolved);
            false
        });
        if pending.is_empty() {
            break;
        }
        if pending.len() == before {
            // No progress: an unresolvable parent (cycle or dangling reference).
            // The registry test guarantees parents exist, so treat any remainder
            // as parentless roots rather than looping forever.
            for row in pending.drain(..) {
                let parent = Resolved { style: Style::default(), glyph: None };
                let resolved = resolve_row(row, &parent, decls);
                map.insert(row.name.to_string(), resolved);
            }
            break;
        }
    }

    let fallback = map.get("text").cloned().expect("text role is always resolved");
    Theme { map, fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_selector_inherits_its_parent_role() {
        let roles = Roles::terminal_default();
        let theme = resolve(&roles, &Decls::new());

        // §2: `transcript` has no delta, so it IS the `text` role.
        assert_eq!(theme.get("transcript").style, roles.text);

        // §3: `glk.buffer.header` = heading role + bold. Heading is already bold,
        // so fg matches heading and BOLD is set.
        let header = theme.get("glk.buffer.header").style;
        assert_eq!(header.fg, roles.heading.fg);
        assert!(header.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn glk_buffer_emphasized_is_italic() {
        // §3 canonical defaults: Emphasized = base role + italic.
        let roles = Roles::terminal_default();
        let theme = resolve(&roles, &Decls::new());

        let emph = theme.get("glk.buffer.emphasized").style;
        assert_eq!(emph.fg, roles.text.fg); // buffer base = text
        assert!(emph.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn explicit_decl_overrides_default() {
        let roles = Roles::terminal_default();

        // Without a decl, `transcript` is the text role (white fg).
        let plain = resolve(&roles, &Decls::new());
        assert_eq!(plain.get("transcript").style.fg, Some(Color::White));

        // An explicit override wins over the registry default.
        let mut decls = Decls::new();
        decls.insert(
            "transcript".to_string(),
            Delta { fg: Some(Color::Red), ..Delta::EMPTY },
        );
        let themed = resolve(&roles, &decls);
        assert_eq!(themed.get("transcript").style.fg, Some(Color::Red));
    }

    #[test]
    fn glyph_carries_from_the_default_delta() {
        // A selector whose default delta carries a glyph exposes it in Resolved.
        let theme = resolve(&Roles::terminal_default(), &Decls::new());
        let meta = theme.get("transcript_meta");
        assert_eq!(meta.glyph.and_then(|g| g.single), Some("▏".to_string()));
    }

    #[test]
    fn unknown_selector_falls_back_to_text() {
        let roles = Roles::terminal_default();
        let theme = resolve(&roles, &Decls::new());
        assert_eq!(theme.get("no.such.selector").style, roles.text);
    }
}
