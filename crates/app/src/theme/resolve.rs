//! The single-layer resolver: flatten [`REGISTRY`] into a concrete theme map.
//!
//! Each registry row derives from a parent (a role, or another selector) via a
//! [`Delta`]. This pass starts from the parent's resolved [`Style`], layers the
//! row's `default_delta`, then applies any matching explicit override from
//! `Decls`. The result is a flat `name -> Resolved` map queried by [`Theme::get`].
//!
//! This is the **layered** resolver (SQ-0309 Task 0.3). It applies several
//! [`Decls`] layers in the spec's static build order (registry default → global
//! user → shipped garglk.ini → per-game overlay, per-game LAST) and stamps each
//! resolved selector with the [`Provenance`] of the highest layer that supplied a
//! value for it. The stamp is **per-selector** (which layer last wrote this
//! selector name), not per-channel — sufficient for the static build order; the
//! runtime per-cell lift lands in Wave 3.

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

    /// A mutable reference to a role's [`Style`] by name, for applying `[roles]`
    /// overrides in place. `None` for an unrecognised name.
    fn by_name_mut(&mut self, name: &str) -> Option<&mut Style> {
        Some(match name {
            "text" => &mut self.text,
            "chrome" => &mut self.chrome,
            "border" => &mut self.border,
            "accent" => &mut self.accent,
            "muted" => &mut self.muted,
            "alert" => &mut self.alert,
            "heading" => &mut self.heading,
            _ => return None,
        })
    }

    /// Derive the 7 role roots from a base colour scheme, matching today's
    /// `ColorScheme::from_ghostty` element→scheme mapping so the derived theme
    /// reproduces the current look.
    pub fn from_scheme(scheme: &crate::colors::GhosttyScheme) -> Roles {
        // No configured scheme: `resolve_base(None)` hands back a
        // `GhosttyScheme::default()` whose fg/bg/palette are all `Color::Reset`.
        // Deriving roles from that would paint every element terminal-default
        // monochrome (losing cyan borders/accents etc.). Fall back to the concrete
        // terminal-default role palette so the out-of-box look keeps its colours.
        if scheme.foreground == Color::Reset {
            return Roles::terminal_default();
        }
        let fg = scheme.foreground;
        let bg = scheme.background;
        Roles {
            text: Style::default().fg(fg),               // transcript = foreground
            chrome: Style::default().fg(fg).bg(bg),       // ink on a UI surface
            border: Style::default().fg(scheme.palette[6]), // cyan slot (focused_border/connector)
            accent: Style::default().fg(scheme.palette[6]), // highlight = cyan slot
            muted: Style::default().fg(scheme.palette[8]),  // suggestion = bright-black slot
            alert: Style::default().fg(scheme.palette[3]),  // yellow slot (room_selected)
            heading: Style::default().fg(fg).add_modifier(Modifier::BOLD),
        }
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

/// Which layer supplied a resolved selector's winning value (§5 static build
/// order). Ordered low→high: a later layer overrides an earlier one, so the
/// [`Provenance`] stamp records the *highest* layer that wrote the selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The registry default (no explicit layer touched this selector).
    Default,
    /// The global user `style.toml`.
    GlobalUser,
    /// The shipped/bundled `garglk.ini`.
    Garglk,
    /// The per-game overlay (the highest layer).
    PerGame,
}

/// A fully resolved selector: its concrete [`Style`], any glyph(s) it carries,
/// its border style (if any), and the [`Provenance`] of the layer that last set
/// its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub style: Style,
    pub glyph: Option<GlyphSet>,
    pub border: Option<crate::render::paneframe::BorderStyle>,
    pub provenance: Provenance,
}

/// Explicit per-selector overrides for a single layer, on top of the registry
/// default. [`resolve`] stacks several of these (global / garglk / per-game).
pub type Decls = HashMap<String, Delta>;

/// The flat resolved theme: `selector name -> Resolved`.
#[derive(Debug, Clone, PartialEq)]
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

/// Layer a [`Delta`]'s border-style onto the inherited one: a set preset name
/// overrides (parsed via the paneframe grammar); otherwise it carries through.
fn apply_border(
    inherited: Option<crate::render::paneframe::BorderStyle>,
    d: &Delta,
) -> Option<crate::render::paneframe::BorderStyle> {
    match d.border {
        Some(name) => Some(crate::render::paneframe::parse_border_style(name)),
        None => inherited,
    }
}

/// Resolve one row against a known parent `Resolved` (or a bare parent style when
/// the row has no parent): the registry default on the parent, then each explicit
/// `layers` override in build order (lowest→highest). Each layer that carries a
/// [`Delta`] for this selector overrides the running value AND advances the
/// [`Provenance`] stamp, so the stamp reflects the highest layer that wrote it.
fn resolve_row(row: &RegRow, parent: &Resolved, layers: &[(&Decls, Provenance)]) -> Resolved {
    // 1. registry default delta on the parent.
    let mut style = apply_style(parent.style, &row.default_delta);
    let mut glyph = apply_glyph(parent.glyph.clone(), &row.default_delta);
    let mut border = apply_border(parent.border, &row.default_delta);
    // 2. each explicit layer, lowest→highest; the last to touch wins the stamp.
    let mut provenance = Provenance::Default;
    for (decls, prov) in layers {
        if let Some(over) = decls.get(row.name) {
            style = apply_style(style, over);
            glyph = apply_glyph(glyph, over);
            border = apply_border(border, over);
            provenance = *prov;
        }
    }
    Resolved { style, glyph, border, provenance }
}

/// Compute the flat theme map from the registry via single-level parent fallback.
///
/// Roles resolve first (from `roles`); then each row resolves against its parent —
/// a role, or another already-resolved selector. A parent that is another selector
/// is handled generally: rows resolve in dependency order via a fixpoint loop, so a
/// parent is always resolved before its child (the registry currently only parents
/// roles, for which one pass suffices).
pub fn resolve(roles: &Roles, global: &Decls, garglk: &Decls, per_game: &Decls) -> Theme {
    // The layers in the spec's static build order (lowest → highest); per-game LAST.
    let layers: [(&Decls, Provenance); 3] = [
        (global, Provenance::GlobalUser),
        (garglk, Provenance::Garglk),
        (per_game, Provenance::PerGame),
    ];

    let mut map: HashMap<String, Resolved> = HashMap::new();

    // Roles first: their Resolved is the bare role style (no glyph, no delta).
    for name in ROLE_NAMES {
        let style = roles.by_name(name).expect("ROLE_NAMES entry has a role style");
        // A role row may still carry an explicit override.
        let base = Resolved { style, glyph: None, border: None, provenance: Provenance::Default };
        let row = REGISTRY.iter().find(|r| r.name == name);
        let resolved = match row {
            Some(r) => resolve_row(r, &Resolved { style, glyph: None, border: None, provenance: Provenance::Default }, &layers),
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
                None => Resolved { style: Style::default(), glyph: None, border: None, provenance: Provenance::Default },
                Some(p) => match map.get(p) {
                    Some(res) => res.clone(),
                    None => return true, // parent not resolved yet; keep pending.
                },
            };
            let resolved = resolve_row(row, &parent, &layers);
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
                let parent = Resolved { style: Style::default(), glyph: None, border: None, provenance: Provenance::Default };
                let resolved = resolve_row(row, &parent, &layers);
                map.insert(row.name.to_string(), resolved);
            }
            break;
        }
    }

    let fallback = map.get("text").cloned().expect("text role is always resolved");
    Theme { map, fallback }
}

/// Build the flat [`Theme`] from a base colour `scheme` and a parsed style document.
///
/// Roles start from [`Roles::from_scheme`] and are overridden by `parsed.roles`
/// (fg/bg only — roles carry no user modifiers today, aside from `heading`'s
/// already-baked-in bold). `parsed.decls` lower to a single GLOBAL [`Decls`]
/// layer (fg/bg + modifier flags). Colour strings resolve against `scheme` via
/// [`crate::colors::parse_color_value`].
///
/// DEFERRED (later waves): a decl's `parent` re-rooting and a decl's
/// `glyph`/`glyphs` overrides are NOT applied here — the registry [`Delta`]'s
/// glyph fields are `&'static`, and per-decl parent override needs resolver
/// support (Wave 5). Only fg/bg/modifiers flow now. Modifiers are additive only
/// (an unset/`false` flag is a no-op, matching [`apply_style`]) — a user cannot
/// yet CLEAR a default modifier.
pub fn resolve_theme(
    scheme: &crate::colors::GhosttyScheme,
    parsed: &super::toml_schema::ParsedStyle,
) -> Theme {
    resolve_theme_layered(scheme, parsed, &Decls::new(), &super::toml_schema::ParsedStyle::default())
}

/// Build the flat [`Theme`] from a base `scheme` and TWO parsed layers: the global
/// user `style.toml` and the per-game overlay (per-game wins, §5). Roles start from
/// [`Roles::from_scheme`], then global `[roles]` fg/bg overrides, then per-game
/// `[roles]` overrides. Each layer's `decls` lower to its own [`Decls`] (fg/bg +
/// modifiers) so [`resolve`] stamps [`Provenance`] per selector (GlobalUser / PerGame).
/// The `garglk` [`Decls`] layer (a discovered garglk.ini's colour overlay, built by
/// `garglk_ini::garglk_color_decls`) sits between global and per-game (§5 order).
///
/// DEFERRED (unchanged from Wave 1, marked `// Wave 5:`): decl `parent` re-rooting,
/// decl `glyph`/`glyphs` overrides, role modifiers, modifier-clearing.
pub fn resolve_theme_layered(
    scheme: &crate::colors::GhosttyScheme,
    global: &super::toml_schema::ParsedStyle,
    garglk: &Decls,
    per_game: &super::toml_schema::ParsedStyle,
) -> Theme {
    // 1. base roles from scheme, then [roles] fg/bg overrides (global, then per-game).
    let mut roles = Roles::from_scheme(scheme);
    let global_role_decls = lower_role_decls(&global.roles, scheme);
    let per_game_role_decls = lower_role_decls(&per_game.roles, scheme);
    apply_role_overrides(&mut roles, &global_role_decls);
    apply_role_overrides(&mut roles, &per_game_role_decls);

    // 2. lower each layer's decls -> its own Decls layer. The [roles] fg/bg
    //    overrides join the same layer (keyed by role name, which is itself a
    //    REGISTRY row name) so the role selector's own Provenance is stamped
    //    consistently with every other selector, via the same `resolve` pass.
    let mut global_decls = lower_decls(&global.decls, scheme);
    global_decls.extend(global_role_decls);
    let mut per_game_decls = lower_decls(&per_game.decls, scheme);
    per_game_decls.extend(per_game_role_decls);

    resolve(&roles, &global_decls, garglk, &per_game_decls)
}

/// Apply a layer's already-lowered `[roles]` fg/bg [`Decls`] onto `roles` in
/// place (so dependent selectors inherit the override via their parent role).
/// Unrecognised role names are ignored. Modifiers are not applied (Wave 5).
fn apply_role_overrides(roles: &mut Roles, role_decls: &Decls) {
    for (name, delta) in role_decls {
        let Some(style) = roles.by_name_mut(name) else { continue };
        *style = apply_style(*style, delta);
    }
}

/// Lower a layer's raw `[roles]` overrides to a [`Decls`] map of fg/bg-only
/// [`Delta`]s (role modifiers are Wave 5 — a role entry with neither fg nor bg
/// set is dropped). Shared by [`apply_role_overrides`] (role-derived selectors)
/// and [`resolve_theme_layered`]'s own decls layer (the role selector's stamp).
fn lower_role_decls(
    raw_roles: &std::collections::BTreeMap<String, super::toml_schema::RawDelta>,
    scheme: &crate::colors::GhosttyScheme,
) -> Decls {
    use crate::colors::parse_color_value;

    let mut decls = Decls::new();
    for (name, raw) in raw_roles {
        let fg = raw.fg.as_deref().and_then(|s| parse_color_value(s, scheme));
        let bg = raw.bg.as_deref().and_then(|s| parse_color_value(s, scheme));
        if fg.is_none() && bg.is_none() {
            continue;
        }
        decls.insert(name.clone(), Delta { fg, bg, ..Delta::EMPTY });
    }
    decls
}

/// Lower a layer's raw `decls` (fg/bg strings + modifier flags) to a [`Decls`]
/// map of resolved [`Delta`]s. Colour strings resolve against `scheme` via
/// [`crate::colors::parse_color_value`].
///
/// DEFERRED (later waves): a decl's `parent` re-rooting and a decl's
/// `glyph`/`glyphs` overrides are NOT applied here — the registry [`Delta`]'s
/// glyph fields are `&'static`, and per-decl parent override needs resolver
/// support (Wave 5). Only fg/bg/modifiers flow now. Modifiers are additive only
/// (an unset/`false` flag is a no-op, matching [`apply_style`]) — a user cannot
/// yet CLEAR a default modifier.
fn lower_decls(
    raw_decls: &std::collections::BTreeMap<String, super::toml_schema::RawDelta>,
    scheme: &crate::colors::GhosttyScheme,
) -> Decls {
    use crate::colors::parse_color_value;

    let mut decls = Decls::new();
    for (name, raw) in raw_decls {
        let delta = Delta {
            fg: raw.fg.as_deref().and_then(|s| parse_color_value(s, scheme)),
            bg: raw.bg.as_deref().and_then(|s| parse_color_value(s, scheme)),
            bold: raw.bold.unwrap_or(false),
            italic: raw.italic.unwrap_or(false),
            underline: raw.underline.unwrap_or(false),
            reversed: raw.reversed.unwrap_or(false),
            dim: raw.dim.unwrap_or(false),
            glyph: None, // Wave 5: glyph overrides not applied here.
            glyphs: &[], // Wave 5: glyph-slot overrides not applied here.
            border: None, // Wave 5: border-style overrides not applied here.
        };
        decls.insert(name.clone(), delta);
        // Wave 5: raw.parent re-rooting not applied here.
    }
    decls
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve with no explicit layers — the registry-default theme.
    fn resolve_default(roles: &Roles) -> Theme {
        resolve(roles, &Decls::new(), &Decls::new(), &Decls::new())
    }

    /// A one-entry [`Decls`] for `sel` carrying `delta`.
    fn one(sel: &str, delta: Delta) -> Decls {
        let mut d = Decls::new();
        d.insert(sel.to_string(), delta);
        d
    }

    #[test]
    fn unset_selector_inherits_its_parent_role() {
        let roles = Roles::terminal_default();
        let theme = resolve_default(&roles);

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
        let theme = resolve_default(&roles);

        let emph = theme.get("glk.buffer.emphasized").style;
        assert_eq!(emph.fg, roles.text.fg); // buffer base = text
        assert!(emph.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn explicit_decl_overrides_default() {
        let roles = Roles::terminal_default();

        // Without a decl, `transcript` is the text role (white fg).
        let plain = resolve_default(&roles);
        assert_eq!(plain.get("transcript").style.fg, Some(Color::White));

        // An explicit override wins over the registry default.
        let decls = one("transcript", Delta { fg: Some(Color::Red), ..Delta::EMPTY });
        let themed = resolve(&roles, &decls, &Decls::new(), &Decls::new());
        assert_eq!(themed.get("transcript").style.fg, Some(Color::Red));
    }

    #[test]
    fn glyph_carries_from_the_default_delta() {
        // A selector whose default delta carries a glyph exposes it in Resolved.
        let theme = resolve_default(&Roles::terminal_default());
        let meta = theme.get("transcript_meta");
        assert_eq!(meta.glyph.and_then(|g| g.single), Some("▏".to_string()));
    }

    #[test]
    fn unknown_selector_falls_back_to_text() {
        let roles = Roles::terminal_default();
        let theme = resolve_default(&roles);
        assert_eq!(theme.get("no.such.selector").style, roles.text);
    }

    #[test]
    fn panel_border_resolves_single() {
        let theme = resolve_default(&Roles::terminal_default());
        assert_eq!(
            theme.get("panel.border").border,
            Some(crate::render::paneframe::BorderStyle::Single)
        );
    }

    #[test]
    fn panel_border_active_is_single_and_bold() {
        let theme = resolve_default(&Roles::terminal_default());
        let active = theme.get("panel.border:active");
        assert_eq!(active.border, Some(crate::render::paneframe::BorderStyle::Single));
        assert!(active.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn non_border_selector_has_no_border() {
        let theme = resolve_default(&Roles::terminal_default());
        assert_eq!(theme.get("transcript").border, None);
    }

    // ── Task 0.3: layered decls + per-selector provenance ────────────────────

    #[test]
    fn per_game_layer_wins_and_is_stamped_pergame() {
        let roles = Roles::terminal_default();
        // Global and per-game both target `transcript`; per-game is the higher layer.
        let global = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let per_game = one("transcript", Delta { fg: Some(Color::Red), ..Delta::EMPTY });
        let theme = resolve(&roles, &global, &Decls::new(), &per_game);

        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Red)); // per-game value wins
        assert_eq!(r.provenance, Provenance::PerGame);
    }

    #[test]
    fn global_over_default_stamped_globaluser() {
        let roles = Roles::terminal_default();
        let global = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let theme = resolve(&roles, &global, &Decls::new(), &Decls::new());

        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Green));
        assert_eq!(r.provenance, Provenance::GlobalUser);
    }

    #[test]
    fn garglk_between_global_and_pergame() {
        let roles = Roles::terminal_default();
        // All three layers target the same selector; the order per-game > garglk >
        // global > default must hold for both the value and the stamp.
        let global = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let garglk = one("transcript", Delta { fg: Some(Color::Blue), ..Delta::EMPTY });

        // garglk beats global when per-game is absent.
        let t1 = resolve(&roles, &global, &garglk, &Decls::new());
        assert_eq!(t1.get("transcript").style.fg, Some(Color::Blue));
        assert_eq!(t1.get("transcript").provenance, Provenance::Garglk);

        // per-game still beats garglk.
        let per_game = one("transcript", Delta { fg: Some(Color::Red), ..Delta::EMPTY });
        let t2 = resolve(&roles, &global, &garglk, &per_game);
        assert_eq!(t2.get("transcript").style.fg, Some(Color::Red));
        assert_eq!(t2.get("transcript").provenance, Provenance::PerGame);
    }

    #[test]
    fn unset_stays_default() {
        let roles = Roles::terminal_default();
        // A selector no layer touches keeps Provenance::Default.
        let global = one("status_bar", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let theme = resolve(&roles, &global, &Decls::new(), &Decls::new());

        assert_eq!(theme.get("transcript").provenance, Provenance::Default);
        // And the touched selector is stamped, confirming Default isn't a blanket.
        assert_eq!(theme.get("status_bar").provenance, Provenance::GlobalUser);
    }

    // ── Task 1.2a: Roles::from_scheme + resolve_theme ─────────────────────────

    use super::super::toml_schema::ParsedStyle;
    use crate::colors::GhosttyScheme;

    /// A `GhosttyScheme` whose fg/bg/palette line up with
    /// `ColorScheme::terminal_default()` so `resolve_theme` should byte-match it.
    fn terminal_default_scheme() -> GhosttyScheme {
        let mut scheme = GhosttyScheme { foreground: Color::White, ..GhosttyScheme::default() };
        scheme.palette[3] = Color::Yellow; // alert slot (room_selected)
        scheme.palette[6] = Color::Cyan; // border/accent slot (focused_border/connector)
        scheme.palette[8] = Color::DarkGray; // muted slot (suggestion)
        scheme
    }

    #[test]
    fn no_scheme_falls_back_to_concrete_terminal_default_roles() {
        // Runtime regression guard: the startup `reload_style` builds the theme from
        // `resolve_base(None)` → `GhosttyScheme::default()`, whose fg/bg/palette are
        // all `Color::Reset`. Without the fallback, every role (hence every selector)
        // would resolve to `Reset` and the out-of-box look would go monochrome.
        let gs = crate::colors::GhosttyScheme::default();
        assert_eq!(gs.foreground, Color::Reset, "no-scheme base is all-Reset");
        assert_eq!(Roles::from_scheme(&gs), Roles::terminal_default());
        let theme = resolve_theme(&gs, &ParsedStyle::default());
        assert_eq!(theme.get("map.connector").style.fg, Some(Color::Cyan));
        assert_eq!(theme.get("panel.border").style.fg, Some(Color::Cyan));
        assert_eq!(theme.get("transcript").style.fg, Some(Color::White));
    }

    #[test]
    fn resolve_theme_reproduces_terminal_defaults() {
        let scheme = terminal_default_scheme();
        let theme = resolve_theme(&scheme, &ParsedStyle::default());

        assert_eq!(theme.get("transcript").style.fg, Some(Color::White));
        assert_eq!(theme.get("map.connector").style.fg, Some(Color::Cyan));
        assert_eq!(theme.get("map.connector_distorted").style.fg, Some(Color::Magenta));
        assert_eq!(theme.get("map.shared_path").style.fg, Some(Color::LightCyan));
        assert_eq!(theme.get("panel.border").style.fg, Some(Color::Cyan));

        let border_active = theme.get("panel.border:active").style;
        assert_eq!(border_active.fg, Some(Color::Cyan));
        assert!(border_active.add_modifier.contains(Modifier::BOLD));

        assert!(theme.get("status_bar").style.add_modifier.contains(Modifier::REVERSED));
        assert!(theme.get("help_bar").style.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(theme.get("suggestion").style.fg, Some(Color::DarkGray));
        assert!(theme.get("glk.buffer.header").style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn parsed_roles_and_decls_lower() {
        let scheme = terminal_default_scheme();

        // (a) a [roles] override on `accent` flows to a role-derived selector.
        let mut with_role = ParsedStyle::default();
        with_role.roles.insert(
            "accent".to_string(),
            super::super::toml_schema::RawDelta {
                fg: Some("red".to_string()),
                ..super::super::toml_schema::RawDelta::default()
            },
        );
        let theme = resolve_theme(&scheme, &with_role);
        assert_eq!(theme.get("map.room_current").style.fg, Some(Color::Red));

        // (b) a decl override on `transcript` wins over its default (text role).
        let mut with_decl = ParsedStyle::default();
        with_decl.decls.insert(
            "transcript".to_string(),
            super::super::toml_schema::RawDelta {
                fg: Some("green".to_string()),
                ..super::super::toml_schema::RawDelta::default()
            },
        );
        let theme = resolve_theme(&scheme, &with_decl);
        assert_eq!(theme.get("transcript").style.fg, Some(Color::Green));
    }

    // ── Task 3.1: resolve_theme_layered (global + per-game, provenance) ──────

    #[test]
    fn layered_per_game_beats_global_with_provenance() {
        use super::super::toml_schema::RawDelta;
        let scheme = terminal_default_scheme();

        let mut global = ParsedStyle::default();
        global.decls.insert(
            "transcript".to_string(),
            RawDelta { fg: Some("green".to_string()), ..RawDelta::default() },
        );

        // per_game overrides transcript to red.
        let mut per_game = ParsedStyle::default();
        per_game.decls.insert(
            "transcript".to_string(),
            RawDelta { fg: Some("red".to_string()), ..RawDelta::default() },
        );
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &per_game);
        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Red));
        assert_eq!(r.provenance, Provenance::PerGame);

        // With per_game default (no override), global wins.
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &ParsedStyle::default());
        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Green));
        assert_eq!(r.provenance, Provenance::GlobalUser);
    }

    #[test]
    fn layered_roles_override_applies() {
        use super::super::toml_schema::RawDelta;
        let scheme = terminal_default_scheme();

        let mut global = ParsedStyle::default();
        global.roles.insert(
            "accent".to_string(),
            RawDelta { fg: Some("green".to_string()), ..RawDelta::default() },
        );
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &ParsedStyle::default());
        assert_eq!(theme.get("accent").style.fg, Some(Color::Green));

        // A per-game [roles] override wins over the global one.
        let mut per_game = ParsedStyle::default();
        per_game.roles.insert(
            "accent".to_string(),
            RawDelta { fg: Some("red".to_string()), ..RawDelta::default() },
        );
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &per_game);
        assert_eq!(theme.get("accent").style.fg, Some(Color::Red));
    }
}
