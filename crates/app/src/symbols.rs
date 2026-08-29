// /// Configurable map symbols for the lanthorn renderer.
// ///
// /// All glyphs the map renderer uses are centralized here. The defaults reproduce
// /// today's hardcoded literals exactly, so an absent `[symbols]` config changes nothing.

// ── Sub-structs ───────────────────────────────────────────────────────────────

/// Six glyphs that form one room-outline style (box-drawing corners + lines).
/// Tuple field order: (tl, tr, bl, br, h, v).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxStyle {
    pub tl: char,
    pub tr: char,
    pub bl: char,
    pub br: char,
    pub h: char,
    pub v: char,
}

/// Cardinal + diagonal arrow glyphs for connector arrowheads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrows {
    pub north: char,
    pub south: char,
    pub east: char,
    pub west: char,
    pub ne: char,
    pub nw: char,
    pub se: char,
    pub sw: char,
}

/// Box-drawing glyphs for the path line-art table (what glyph_for returns per mask).
/// Field names match the direction-bit combinations: ew=east-west straight, ns=north-south,
/// se=southeast corner (coming from south, going east), etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathGlyphs {
    pub ew: char,
    pub ns: char,
    pub se: char,
    pub sw: char,
    pub ne: char,
    pub nw: char,
    pub nse: char,
    pub nsw: char,
    pub ews: char,
    pub ewn: char,
    pub nesw: char,
    // ── Diagonal corner-exit stubs (SQ-0314) ─────────────────────────────────
    // Half-diagonals from the Legacy Computing block, named for their two
    // endpoints (matching their Unicode names). Unlike ╱/╲ (U+2571/2572), which
    // run corner-to-corner, EVERY endpoint here is an edge MIDPOINT — the same
    // points ─ attaches at (middle left/right) and │ attaches at (upper/lower
    // centre). That is what lets a diagonal stub hand off to an orthogonal path
    // cleanly. Used only when `diagonal_corners` is on.
    /// Upper-centre ↔ middle-left (U+1FBA0 🮠).
    pub diag_ul: char,
    /// Upper-centre ↔ middle-right (U+1FBA1 🮡).
    pub diag_ur: char,
    /// Middle-left ↔ lower-centre (U+1FBA2 🮢).
    pub diag_ll: char,
    /// Middle-right ↔ lower-centre (U+1FBA3 🮣).
    pub diag_lr: char,
}

/// Portal icon glyphs: directional markers + connector path char.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalGlyphs {
    /// Marker drawn in the notes/icon column for a room with notes (●).
    pub marker: char,
    /// Dotted vertical connector for Up/Down portal links (┊).
    pub path: char,
    /// Dotted horizontal connector for Up/Down portal links (┄).
    pub path_h: char,
    /// Up portal icon (↑).
    pub up: char,
    /// Down portal icon (↓).
    pub down: char,
    /// In portal icon (◉).
    pub in_: char,
    /// Out portal icon (◎).
    pub out: char,
    /// Unknown portal icon (?).
    pub unknown: char,
}

/// The glyphs on the pane border's clickable toggle controls (SQ-1123).
///
/// Every slot is a STATE, not a control: a toggle draws one of two glyphs
/// depending on which way it would move things, so the icon says what is on
/// before the colour does. The panel toggles are arrows pointing the way the
/// panel would go — the map lives to the right of the story pane and the verb
/// panel below it, so `map_hide` points right (click and the map leaves that
/// way) and `band_show` points up (click and the band rises into view).
///
/// Defaults come from Geometric Shapes (U+25xx) for the same reason
/// [`PortalGlyphs`]' do: it is the block an ordinary monospace face already has
/// to carry for the map's ● ▲ ▼ ◀ ▶, so the controls draw on a stock terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlGlyphs {
    /// Map hidden — click and it slides in from the right (◀).
    pub map_show: char,
    /// Map shown — click and it leaves to the right (▶).
    pub map_hide: char,
    /// Verb panel closed — click and it rises from the bottom (▲).
    pub band_show: char,
    /// Verb panel open — click and it drops back down (▼).
    pub band_hide: char,
    /// Lanthorn's Guiding Light is on (●; the lamp itself in a patched font).
    pub guidance_on: char,
    /// The Guiding Light is off (○).
    pub guidance_off: char,
    /// v6 render mode `hybrid` — half text, half art (◧).
    pub render_hybrid: char,
    /// v6 render mode `raster` — the whole frame is a picture (■).
    pub render_raster: char,
    /// v6 render mode `extended` (▦).
    pub render_extended: char,
    /// v6 pixel lock engaged — art pinned to whole device pixels (▣).
    pub lock_on: char,
    /// v6 pixel lock off (□).
    pub lock_off: char,
    /// The return probe (SQ-0785) — a footprint, in one state rather than two.
    ///
    /// **The only control here with a single glyph, and deliberately.** Every
    /// other one names two modes and draws the mode it is in: shown/hidden,
    /// open/closed, locked/unlocked. This one has no opposite mode — it is either
    /// looking for the way back or it is not — and it is also the only control
    /// whose off state is the DEFAULT, which is the state a player has to notice
    /// in order to ever turn it on. So the mark is always the same and the colour
    /// carries the state: muted when off, lit through `panel.control:lit` when
    /// on. A shape that changed here would be saying "the other mode is engaged"
    /// about a thing with no other mode.
    pub return_probe: char,
}

// ── Top-level set ─────────────────────────────────────────────────────────────

/// All map glyphs used by the renderer, resolved from config at startup.
///
/// `Default` returns the exact set of glyphs that were hardcoded before this
/// abstraction was introduced — back-compat is guaranteed by that contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSet {
    pub room_normal: BoxStyle,
    pub room_current: BoxStyle,
    pub room_portal: BoxStyle,
    /// Selected room outline. Defaults to normal (selection is color-only today).
    pub room_selected: BoxStyle,
    pub arrows: Arrows,
    pub path: PathGlyphs,
    pub portal: PortalGlyphs,
    /// Glyphs for the pane border's clickable toggle controls (SQ-1123).
    pub controls: ControlGlyphs,
    /// Gutter marker glyph for META transcript lines.
    pub meta_gutter: char,
    /// Gutter marker glyph for WARNING transcript lines.
    pub warning_gutter: char,
    /// The mark of Lanthorn's Guiding Light, drawn in the gutter of every ASSIST
    /// transcript line (SQ-1045). It is not a bar beside an icon — it **is** the
    /// icon, and the only thing that identifies an assist on screen, since the
    /// lines themselves carry no marker in their text.
    ///
    /// `●` (U+25CF) by default, chosen by scanning the cmaps of eight text faces
    /// on a working machine: every glyph that actually *depicts* a light misses
    /// too many of them (`☼` 4/8, `★` 2/8, the dingbat stars 2/8), while the
    /// filled circle reaches 6/8. It is a mark, not a picture, and a mark that
    /// draws everywhere beats a lamp that draws in three fonts.
    ///
    /// A patched font has the lamp itself: set `[symbols.overrides]
    /// "gutter.assist" = "\u{F1A60}"` — Nerd Fonts' `md-post_lamp`, verified
    /// against the font's own `post` table rather than a cheat sheet — which
    /// reaches the same 6/8, missing only the unpatched system faces. SQ-1104
    /// will pick it automatically when a first-run font check can see it.
    ///
    /// **Not `*`**: Infocom games spend asterisks on footnotes, and a footnote
    /// marker in the margin of an interpreter's own line is exactly the
    /// impersonation this register exists to avoid.
    pub assist_gutter: char,
    /// Header marker for the room dock while it FOLLOWS the player.
    ///
    /// Hollow against [`Self::dock_pinned`]'s filled, the same reading the portal
    /// icons use: hollow is the moving state, filled is the fixed one. Both were
    /// hard-coded in `render::room_dock` until SQ-0989's follow-up, as `U+2316`
    /// POSITION INDICATOR and `U+2299` CIRCLED DOT — the second being the very
    /// glyph that quest removed from the map for being undrawable, and neither is
    /// in Fira Code (`fc-list ":charset=2316"` and `":charset=2299"` match no
    /// FiraCode face; `25C6`/`25C7` match 13 each).
    pub dock_following: char,
    /// Header marker for the room dock while it is PINNED to a selected room.
    ///
    /// A BMP glyph, not the design sketch's emoji: an emoji is double-width in a
    /// cell grid and the header is drawn cell-by-cell like every other line there.
    pub dock_pinned: char,
    /// Draw ne/nw/se/sw connectors as a chain of half-diagonals out of the room corner, using the
    /// `path.diag_*` glyphs (SQ-0314). On by default.
    ///
    /// Turn it off for a terminal/font without Unicode 13 Legacy Computing coverage: the connector
    /// still leaves and arrives on the same CORNERS — that part is the router's doing, not this
    /// setting's — but walks between them orthogonally instead.
    pub diagonal_corners: bool,
}

impl Default for SymbolSet {
    fn default() -> Self {
        let room_normal = BoxStyle { tl: '╭', tr: '╮', bl: '╰', br: '╯', h: '─', v: '│' };
        Self {
            room_normal,
            room_current: BoxStyle { tl: '┏', tr: '┓', bl: '┗', br: '┛', h: '━', v: '┃' },
            room_portal: BoxStyle { tl: '╔', tr: '╗', bl: '╚', br: '╝', h: '═', v: '║' },
            room_selected: room_normal, // color-only selection today
            arrows: Arrows {
                north: '▲',
                south: '▼',
                east: '▶',
                west: '◀',
                ne: '↗',
                nw: '↖',
                se: '↘',
                sw: '↙',
            },
            path: PathGlyphs {
                ew: '─',
                ns: '│',
                se: '┌',
                sw: '┐',
                ne: '└',
                nw: '┘',
                nse: '├',
                nsw: '┤',
                ews: '┬',
                ewn: '┴',
                nesw: '┼',
                diag_ul: '🮠',
                diag_ur: '🮡',
                diag_ll: '🮢',
                diag_lr: '🮣',
            },
            portal: PortalGlyphs {
                marker: '●',
                path: '┊',
                path_h: '┄',
                up: '↑',
                down: '↓',
                // ◉/◎, not ⊙/⊗ — see `PortalGlyphs::preset`'s "ascii" arm.
                in_: '◉',
                out: '◎',
                unknown: '?',
            },
            controls: ControlGlyphs {
                map_show: '◀',
                map_hide: '▶',
                band_show: '▲',
                band_hide: '▼',
                guidance_on: '●',
                guidance_off: '○',
                render_hybrid: '◧',
                render_raster: '■',
                render_extended: '▦',
                lock_on: '▣',
                lock_off: '□',
                // ◌ (U+25CC), the only mark in Geometric Shapes that reads as a
                // TRACE rather than as a state — the print left by something that
                // walked through and is not there any more, which is exactly what
                // the shadow leaves behind. Everything else in the block is a
                // filled/hollow pair saying which of two modes is in force.
                return_probe: '◌',
            },
            dock_following: '◇',
            dock_pinned: '◆',
            meta_gutter: '▏',
            assist_gutter: '●',
            warning_gutter: '!',
            diagonal_corners: true,
        }
    }
}

// ── Presets ───────────────────────────────────────────────────────────────────

impl BoxStyle {
    /// All known preset names for BoxStyle, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["rounded", "thick", "double", "solid", "super-thick", "ascii", "borderless"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "rounded"    — rounded corners (default, matches `SymbolSet::default().room_normal`)
    /// - "thick"      — heavy box-drawing (matches `room_current`)
    /// - "double"     — double-line box-drawing (matches `room_portal`)
    /// - "solid"      — full-block walls: every edge/corner is `█` (single-width)
    /// - "super-thick" — full-block edges `█` with quadrant-block corners `▛▜▙▟`
    ///   (heavy block frame with beveled inner corners)
    /// - "ascii"      — ASCII-only: corners `+`, horizontal `-`, vertical `|`
    /// - "borderless" — all spaces (invisible walls)
    pub fn preset(name: &str) -> Option<BoxStyle> {
        Some(match name {
            "rounded" => BoxStyle { tl: '╭', tr: '╮', bl: '╰', br: '╯', h: '─', v: '│' },
            "thick" => BoxStyle { tl: '┏', tr: '┓', bl: '┗', br: '┛', h: '━', v: '┃' },
            "double" => BoxStyle { tl: '╔', tr: '╗', bl: '╚', br: '╝', h: '═', v: '║' },
            "solid" => BoxStyle { tl: '█', tr: '█', bl: '█', br: '█', h: '█', v: '█' },
            "super-thick" => BoxStyle { tl: '▛', tr: '▜', bl: '▙', br: '▟', h: '█', v: '█' },
            "ascii" => BoxStyle { tl: '+', tr: '+', bl: '+', br: '+', h: '-', v: '|' },
            "borderless" => BoxStyle { tl: ' ', tr: ' ', bl: ' ', br: ' ', h: ' ', v: ' ' },
            _ => return None,
        })
    }
}

impl Arrows {
    /// All known preset names for Arrows, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["filled", "line", "nerdfont", "nf-bold", "nf-box", "nf-circle", "nf-outline"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "filled"     — filled triangle glyphs ▲▼▶◀ + diagonal arrows ↗↖↘↙ (default)
    /// - "line"       — thin Unicode arrows ↑↓→← + diagonal ↗↖↘↙
    /// - "nerdfont"   — Nerd Font single-width chevron codepoints (requires patched font)
    ///   Cardinal: chevron-up (U+F0143) chevron-down (U+F0140)
    ///   chevron-right (U+F0142) chevron-left (U+F0141)
    ///   Diagonal: same as "line" (↗↖↘↙)
    /// - "nf-bold"    — MDI arrow-{up,down,left,right}-bold (F0737/F072E/F0731/F0734)
    ///   Diagonal: Unicode fallback ↖↗↙↘ (no native MDI bold diagonals)
    /// - "nf-box"     — MDI arrow-{up,down,left,right}-bold-box (F0738/F072F/F0732/F0735)
    ///   Diagonal: native MDI bold-box diagonals (F1968/F196A/F1964/F1966)
    /// - "nf-circle"  — MDI arrow-{up,down,left,right}-bold-circle (F005F/F0047/F004F/F0056)
    ///   Diagonal: Unicode fallback ↖↗↙↘ (no native MDI circle diagonals)
    /// - "nf-outline" — MDI arrow-{up,down,left,right}-bold-outline (F09C7/F09BF/F09C0/F09C2)
    ///   Diagonal: native MDI bold-outline diagonals (F09C3/F09C5/F09B7/F09B9)
    pub fn preset(name: &str) -> Option<Arrows> {
        Some(match name {
            "filled" => Arrows {
                north: '▲', south: '▼', east: '▶', west: '◀',
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "line" => Arrows {
                north: '↑', south: '↓', east: '→', west: '←',
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nerdfont" => Arrows {
                // MDI chevron glyphs (single-width in patched fonts):
                // chevron-up F0143, chevron-down F0140, chevron-right F0142, chevron-left F0141.
                north: '\u{F0143}', south: '\u{F0140}',
                east: '\u{F0142}', west: '\u{F0141}',
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nf-bold" => Arrows {
                // MDI arrow-up-bold F0737, arrow-down-bold F072E,
                // arrow-left-bold F0731, arrow-right-bold F0734
                north: '\u{F0737}', south: '\u{F072E}',
                east: '\u{F0734}', west: '\u{F0731}',
                // No native MDI plain-bold diagonal arrows; use Unicode fallback
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nf-box" => Arrows {
                // MDI arrow-up-bold-box F0738, arrow-down-bold-box F072F,
                // arrow-left-bold-box F0732, arrow-right-bold-box F0735
                north: '\u{F0738}', south: '\u{F072F}',
                east: '\u{F0735}', west: '\u{F0732}',
                // Native MDI bold-box diagonal arrows (verified)
                // arrow-top-left-bold-box F1968, arrow-top-right-bold-box F196A,
                // arrow-bottom-left-bold-box F1964, arrow-bottom-right-bold-box F1966
                nw: '\u{F1968}', ne: '\u{F196A}',
                sw: '\u{F1964}', se: '\u{F1966}',
            },
            "nf-circle" => Arrows {
                // MDI arrow-up-bold-circle F005F, arrow-down-bold-circle F0047,
                // arrow-left-bold-circle F004F, arrow-right-bold-circle F0056
                north: '\u{F005F}', south: '\u{F0047}',
                east: '\u{F0056}', west: '\u{F004F}',
                // No native MDI circle diagonal arrows; use Unicode fallback
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nf-outline" => Arrows {
                // MDI arrow-up-bold-outline F09C7, arrow-down-bold-outline F09BF,
                // arrow-left-bold-outline F09C0, arrow-right-bold-outline F09C2
                north: '\u{F09C7}', south: '\u{F09BF}',
                east: '\u{F09C2}', west: '\u{F09C0}',
                // Native MDI bold-outline diagonal arrows (verified from MDI CSS)
                // arrow-top-left-bold-outline F09C3, arrow-top-right-bold-outline F09C5,
                // arrow-bottom-left-bold-outline F09B7, arrow-bottom-right-bold-outline F09B9
                nw: '\u{F09C3}', ne: '\u{F09C5}',
                sw: '\u{F09B7}', se: '\u{F09B9}',
            },
            _ => return None,
        })
    }
}

impl PathGlyphs {
    /// All known preset names for PathGlyphs, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["light", "heavy", "dotted"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "light"  — light box-drawing lines ─│┌┐└┘├┤┬┴┼ (default)
    /// - "heavy"  — heavy box-drawing lines ━┃┏┓┗┛┣┫┳┻╋
    /// - "dotted" — dotted/dashed box-drawing lines ╌╎┄┆ with fallbacks
    pub fn preset(name: &str) -> Option<PathGlyphs> {
        Some(match name {
            // The four diag_* slots are identical across every preset: the Legacy
            // Computing block has only LIGHT half-diagonals — no heavy or dotted
            // variants exist — so they fall back the same way "dotted" already
            // falls back to light corners for its turns. (SQ-0314)
            "light" => PathGlyphs {
                ew: '─', ns: '│', se: '┌', sw: '┐', ne: '└', nw: '┘',
                nse: '├', nsw: '┤', ews: '┬', ewn: '┴', nesw: '┼',
                diag_ul: '🮠', diag_ur: '🮡',
                diag_ll: '🮢', diag_lr: '🮣',
            },
            "heavy" => PathGlyphs {
                ew: '━', ns: '┃', se: '┏', sw: '┓', ne: '┗', nw: '┛',
                nse: '┣', nsw: '┫', ews: '┳', ewn: '┻', nesw: '╋',
                diag_ul: '🮠', diag_ur: '🮡',
                diag_ll: '🮢', diag_lr: '🮣',
            },
            "dotted" => PathGlyphs {
                // Quadruple-dash light for straights; turns fall back to light corners
                // since Unicode has no dotted corner glyphs.
                ew: '┄', ns: '┆', se: '┌', sw: '┐', ne: '└', nw: '┘',
                nse: '├', nsw: '┤', ews: '┬', ewn: '┴', nesw: '┼',
                diag_ul: '🮠', diag_ur: '🮡',
                diag_ll: '🮢', diag_lr: '🮣',
            },
            _ => return None,
        })
    }
}

impl PortalGlyphs {
    /// All known preset names for PortalGlyphs, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["ascii", "nerdfont", "nerdfont-stairs"]
    }

    /// The `(vertical, horizontal)` connector pair for a named portal-path
    /// preset, or `None` for an unknown name. Chosen by `portal_path_style`
    /// independently of the icon set, so the up/down/in/out links can be styled
    /// apart from the cardinal paths (`path_style`).
    ///
    /// - "light"  — │ / ─
    /// - "heavy"  — ┃ / ━
    /// - "dotted" — ┊ / ┄ (the default: the connectors the map has always drawn)
    pub fn path_preset(name: &str) -> Option<(char, char)> {
        Some(match name {
            "light" => ('│', '─'),
            "heavy" => ('┃', '━'),
            "dotted" => ('┊', '┄'),
            _ => return None,
        })
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "ascii"            — ASCII-compatible glyphs (default): ●/↑/↓/◉/◎/? with ┊┄ connectors
    /// - "nerdfont"         — Nerd Font single-width icon codepoints (requires patched font)
    ///   nf-fa-circle (U+F111) for marker, nf-md-arrow_up_circle (U+F0CE1) for up,
    ///   nf-md-arrow_down_circle (U+F0CDB) for down, nf-fa-sign_in (U+F090) for in,
    ///   nf-fa-sign_out (U+F08B) for out, nf-fa-question_circle (U+F059) for unknown
    /// - "nerdfont-stairs"  — Nerd Font 4 distinct direction icons (requires patched font)
    ///   up=mdi-stairs-up (U+F12BD), down=mdi-stairs-down (U+F12BE),
    ///   in=mdi-location-enter (U+F0FC4), out=mdi-exit-run (U+F0A48)
    pub fn preset(name: &str) -> Option<PortalGlyphs> {
        Some(match name {
            // In/Out are ◉ FISHEYE (U+25C9) and ◎ BULLSEYE (U+25CE), not the ⊙ (U+2299) and
            // ⊗ (U+2297) they were until SQ-0989. The old pair sits in Miscellaneous
            // Mathematical Operators, which monospace faces routinely skip: Fira Code — the
            // face `pty_stream::gallery::FONT_CANDIDATES` leads with, and a common terminal
            // font — carries neither (checked with `fc-list ":charset=2299"`, and pinned
            // against its cmap by SQ-0963), so the default map drew tofu or borrowed a
            // fallback face with the wrong metrics. Geometric Shapes is the block the map
            // already depends on for ● ▲ ▼ ◀ ▶, and every monospace face measured that has
            // ⊙/⊗ also has ◉/◎ — the swap costs no coverage and buys Fira Code's.
            // Same reading as before (a circle with something in it, non-directional):
            // ◉ a filled way in, ◎ a hollow way out. A user who prefers the old pair keeps
            // it with `portal.in`/`portal.out` overrides in `style.toml`.
            "ascii" => PortalGlyphs {
                marker: '●', path: '┊', path_h: '┄',
                up: '↑', down: '↓', in_: '◉', out: '◎', unknown: '?',
            },
            "nerdfont" => PortalGlyphs {
                // nf-fa-circle U+F111, connectors keep the same box-drawing chars
                marker: '\u{F111}', path: '┊', path_h: '┄',
                // md-arrow_up_circle U+F0CE1, md-arrow_down_circle U+F0CDB — resolved by NAME
                // from the Nerd Fonts `glyphnames.json` (v3.5.1). They used to read F0B71 and
                // F0B72, which that file calls md-card_bulleted_off{,_outline}: patched faces
                // do carry those codepoints, so the preset drew a crisp, confident icon of the
                // wrong thing rather than a missing glyph anyone would notice (SQ-0989).
                up: '\u{F0CE1}', down: '\u{F0CDB}',
                // nf-fa-sign_in U+F090, nf-fa-sign_out U+F08B
                in_: '\u{F090}', out: '\u{F08B}',
                // nf-fa-question_circle U+F059
                unknown: '\u{F059}',
            },
            "nerdfont-stairs" => PortalGlyphs {
                // Reuse nf-fa-circle U+F111 for marker, nf-fa-question_circle U+F059 for unknown
                marker: '\u{F111}', path: '┊', path_h: '┄',
                // Four DISTINCT direction icons (resolved from MDI webfont CSS by name):
                // mdi-stairs-up U+F12BD
                up: '\u{F12BD}',
                // mdi-stairs-down U+F12BE
                down: '\u{F12BE}',
                // mdi-location-enter U+F0FC4
                in_: '\u{F0FC4}',
                // mdi-exit-run U+F0A48
                out: '\u{F0A48}',
                unknown: '\u{F059}',
            },
            _ => return None,
        })
    }
}

impl ControlGlyphs {
    /// All known preset names for [`ControlGlyphs`], in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["plain", "nerdfont"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "plain"    — Geometric Shapes only (default): ◀▶▲▼ ●○ ◧■▦ ▣□
    /// - "nerdfont" — a named icon for every one of the eleven states.
    ///
    /// **Every nerdfont codepoint below was read from the font's own `post`
    /// table**, not inferred from a name. SQ-0989 is what a guessed codepoint
    /// costs: a patched face draws the wrong icon crisply and confidently and
    /// nobody notices, because there is nothing on our side that can see it.
    /// Two of the names originally proposed for this set do not exist in the
    /// font at all (`cod-layout_panel_dock`, `md-post_map`), which is exactly
    /// the failure the reading catches. `nerdfont_control_glyphs_are_the_names_
    /// that_were_read_from_the_font` pins them.
    ///
    /// **Each control's two states come from ONE icon family** — `fa-` for the
    /// map, `cod-` for the verb panel, `md-` for the Guiding Light, the render
    /// mode and the pixel lock. Codicons, Font Awesome and Material Design carry
    /// different stroke weights and cap heights, so a control whose states came
    /// from different families appeared to JUMP on toggle, independently of the
    /// shape change that was meant to be the signal.
    pub fn preset(name: &str) -> Option<ControlGlyphs> {
        let plain = SymbolSet::default().controls;
        Some(match name {
            "plain" => plain,
            "nerdfont" => ControlGlyphs {
                // fa-map_location / fa-map_location_dot — the dot reads as
                // "you are here", which is what an automap is for.
                map_show: '\u{0EE68}',
                map_hide: '\u{0EE69}',
                // cod-layout_panel_off / cod-layout_panel — a purpose-built
                // off/on pair rather than two icons pressed into service.
                band_show: '\u{0EC01}',
                band_hide: '\u{0EBF2}',
                // md-post_lamp — the Guiding Light's own mark, the same glyph
                // `font_check_dialog::ASSIST_LAMP` draws in the gutter — and
                // md-help for the light that is out.
                guidance_on: '\u{F1A60}',
                guidance_off: '\u{F02D6}',
                // md-monitor / md-monitor_shimmer / md-monitor_star: one screen
                // per way of drawing the screen.
                render_hybrid: '\u{F0379}',
                render_raster: '\u{F1104}',
                render_extended: '\u{F0DDC}',
                // md-lock / md-lock_open.
                lock_on: '\u{F033E}',
                lock_off: '\u{F033F}',
                // md-shoe_print — a footprint, for the search that walks the way
                // back and leaves nothing behind but the knowledge that it does.
                return_probe: '\u{F0DFA}',
            },
            _ => return None,
        })
    }
}

// ── resolve ───────────────────────────────────────────────────────────────────

impl SymbolSet {
    /// Build a `SymbolSet` from a `SymbolConfig`:
    /// 1. Start from each category's named preset (unknown name → category default).
    /// 2. Apply per-slot overrides from `cfg.overrides`.
    ///
    /// Override validation: the value must be exactly one `char` (checked via
    /// `chars().count() == 1`). We do not add a `unicode-width` dependency; we
    /// instead reject any char with a code point in the known CJK/wide ranges
    /// (U+1100..=U+FFEF broad block that covers fullwidth and wide CJK) plus any
    /// char above U+FFFF that terminals commonly render as double-wide (emoji etc.).
    /// Single-byte ASCII and the entire BMP box-drawing block are always accepted.
    /// Invalid values (empty, multi-char, wide estimate) → keep the preset glyph.
    pub fn resolve(cfg: &crate::config::SymbolConfig) -> SymbolSet {
        let mut s = SymbolSet {
            room_normal: BoxStyle::preset(&cfg.box_style).unwrap_or_else(|| SymbolSet::default().room_normal),
            room_current: SymbolSet::default().room_current,
            room_portal: SymbolSet::default().room_portal,
            room_selected: BoxStyle::preset(&cfg.box_style).unwrap_or_else(|| SymbolSet::default().room_selected),
            arrows: Arrows::preset(&cfg.arrow_set).unwrap_or_else(|| SymbolSet::default().arrows),
            path: PathGlyphs::preset(&cfg.path_style).unwrap_or_else(|| SymbolSet::default().path),
            portal: PortalGlyphs::preset(&cfg.portal_icons).unwrap_or_else(|| SymbolSet::default().portal),
            controls: ControlGlyphs::preset(&cfg.control_icons).unwrap_or_else(|| SymbolSet::default().controls),
            meta_gutter: SymbolSet::default().meta_gutter,
            warning_gutter: SymbolSet::default().warning_gutter,
            assist_gutter: SymbolSet::default().assist_gutter,
            dock_following: SymbolSet::default().dock_following,
            dock_pinned: SymbolSet::default().dock_pinned,
            diagonal_corners: cfg.diagonal_corners,
        };

        // The portal connectors are a preset of their own, layered on the icon
        // set: every icon preset ships the same ┊/┄ pair, so `portal_path_style`
        // is what actually chooses them (unknown name → keep the icon set's).
        if let Some((v, h)) = PortalGlyphs::path_preset(&cfg.portal_path_style) {
            s.portal.path = v;
            s.portal.path_h = h;
        }

        for (key, val) in &cfg.overrides {
            // Validate: exactly one char, estimated single display width.
            let mut chars = val.chars();
            let Some(ch) = chars.next() else { continue }; // empty
            if chars.next().is_some() { continue; } // multi-char
            if is_wide_estimate(ch) { continue; } // likely wide

            apply_override(&mut s, key, ch);
        }

        s
    }

    /// Build a `SymbolSet` from four named presets (box, arrow, portal, path).
    /// Unknown preset names fall back to the category default (same as `resolve`).
    pub fn from_preset_names(box_: &str, arrow: &str, portal: &str, path: &str) -> SymbolSet {
        let cfg = crate::config::SymbolConfig {
            box_style: box_.to_owned(),
            arrow_set: arrow.to_owned(),
            portal_icons: portal.to_owned(),
            path_style: path.to_owned(),
            portal_path_style: crate::config::default_portal_path_style(),
            control_icons: crate::config::default_control_icons(),
            badge_zcode: crate::config::default_badge_zcode(),
            badge_glulx: crate::config::default_badge_glulx(),
            badge_blorb: crate::config::default_badge_blorb(),
            badge_save: crate::config::default_badge_save(),
            badge_hint: crate::config::default_badge_hint(),
            badge_hint_available: crate::config::default_badge_hint_available(),
            diagonal_corners: crate::config::default_diagonal_corners(),
            overrides: std::collections::BTreeMap::new(),
        };
        SymbolSet::resolve(&cfg)
    }
}

/// Conservative "likely wide" estimate without unicode-width dependency.
/// Rejects chars in the CJK/fullwidth/emoji-heavy ranges. The box-drawing
/// block (U+2500..=U+257F), arrows (U+2190..=U+21FF), and BMP geometric
/// shapes are always accepted.
pub(crate) fn is_wide_estimate(c: char) -> bool {
    let cp = c as u32;
    // U+1FBA0..=U+1FBAF are the Legacy Computing box-drawing half-diagonals: narrow
    // line-art, not emoji, despite sitting inside the blanket 0x1F000..=0x1FFFF
    // reject below. Carve them out or `path.diag_*` overrides are silently dropped
    // and the slots stop being themeable. (SQ-0314)
    if (0x1FBA0..=0x1FBAF).contains(&cp) {
        return false;
    }
    matches!(cp,
        0x1100..=0x115F  // Hangul Jamo
        | 0x2E80..=0x2EFF  // CJK Radicals
        | 0x2F00..=0x2FDF  // Kangxi Radicals
        | 0x2FF0..=0x303F  // CJK Symbols
        | 0x3040..=0x309F  // Hiragana
        | 0x30A0..=0x30FF  // Katakana
        | 0x3100..=0x312F  // Bopomofo
        | 0x3130..=0x318F  // Hangul Compatibility
        | 0x3190..=0x319F  // Kanbun
        | 0x31A0..=0x31BF  // Bopomofo Extended
        | 0x31F0..=0x31FF  // Katakana Phonetic
        | 0x3200..=0x32FF  // Enclosed CJK
        | 0x3300..=0x33FF  // CJK Compatibility
        | 0x3400..=0x4DBF  // CJK Extension A
        | 0x4E00..=0x9FFF  // CJK Unified Ideographs
        | 0xA000..=0xA48F  // Yi Syllables
        | 0xA490..=0xA4CF  // Yi Radicals
        | 0xAC00..=0xD7AF  // Hangul Syllables
        | 0xF900..=0xFAFF  // CJK Compatibility Ideographs
        | 0xFE10..=0xFE1F  // Vertical Forms
        | 0xFE30..=0xFE4F  // CJK Compatibility Forms
        | 0xFE50..=0xFE6F  // Small Form Variants
        | 0xFF00..=0xFFEF  // Halfwidth/Fullwidth Forms
        | 0x1F000..=0x1FFFF // Emoji, Mahjong, etc.
        | 0x20000..=0x2A6DF // CJK Extension B
        | 0x2A700..=0x2CEAF // CJK Extension C/D/E
    )
}

/// Apply one validated override char to the matching slot in `s`.
/// Unknown slot keys are ignored.
fn apply_override(s: &mut SymbolSet, key: &str, ch: char) {
    match key {
        "room.normal.tl"   => s.room_normal.tl = ch,
        "room.normal.tr"   => s.room_normal.tr = ch,
        "room.normal.bl"   => s.room_normal.bl = ch,
        "room.normal.br"   => s.room_normal.br = ch,
        "room.normal.h"    => s.room_normal.h = ch,
        "room.normal.v"    => s.room_normal.v = ch,
        "room.current.tl"  => s.room_current.tl = ch,
        "room.current.tr"  => s.room_current.tr = ch,
        "room.current.bl"  => s.room_current.bl = ch,
        "room.current.br"  => s.room_current.br = ch,
        "room.current.h"   => s.room_current.h = ch,
        "room.current.v"   => s.room_current.v = ch,
        "room.portal.tl"   => s.room_portal.tl = ch,
        "room.portal.tr"   => s.room_portal.tr = ch,
        "room.portal.bl"   => s.room_portal.bl = ch,
        "room.portal.br"   => s.room_portal.br = ch,
        "room.portal.h"    => s.room_portal.h = ch,
        "room.portal.v"    => s.room_portal.v = ch,
        "room.selected.tl" => s.room_selected.tl = ch,
        "room.selected.tr" => s.room_selected.tr = ch,
        "room.selected.bl" => s.room_selected.bl = ch,
        "room.selected.br" => s.room_selected.br = ch,
        "room.selected.h"  => s.room_selected.h = ch,
        "room.selected.v"  => s.room_selected.v = ch,
        "arrow.north"      => s.arrows.north = ch,
        "arrow.south"      => s.arrows.south = ch,
        "arrow.east"       => s.arrows.east = ch,
        "arrow.west"       => s.arrows.west = ch,
        "arrow.ne"         => s.arrows.ne = ch,
        "arrow.nw"         => s.arrows.nw = ch,
        "arrow.se"         => s.arrows.se = ch,
        "arrow.sw"         => s.arrows.sw = ch,
        "path.ew"          => s.path.ew = ch,
        "path.ns"          => s.path.ns = ch,
        "path.se"          => s.path.se = ch,
        "path.sw"          => s.path.sw = ch,
        "path.ne"          => s.path.ne = ch,
        "path.nw"          => s.path.nw = ch,
        "path.nse"         => s.path.nse = ch,
        "path.nsw"         => s.path.nsw = ch,
        "path.ews"         => s.path.ews = ch,
        "path.ewn"         => s.path.ewn = ch,
        "path.cross"       => s.path.nesw = ch,
        "path.diag_ul"     => s.path.diag_ul = ch,
        "path.diag_ur"     => s.path.diag_ur = ch,
        "path.diag_ll"     => s.path.diag_ll = ch,
        "path.diag_lr"     => s.path.diag_lr = ch,
        "portal.up"        => s.portal.up = ch,
        "portal.down"      => s.portal.down = ch,
        "portal.in"        => s.portal.in_ = ch,
        "portal.out"       => s.portal.out = ch,
        "portal.unknown"   => s.portal.unknown = ch,
        "portal.path"      => s.portal.path = ch,
        "portal.marker"    => s.portal.marker = ch,
        "control.map_show"       => s.controls.map_show = ch,
        "control.map_hide"       => s.controls.map_hide = ch,
        "control.band_show"      => s.controls.band_show = ch,
        "control.band_hide"      => s.controls.band_hide = ch,
        "control.guidance_on"    => s.controls.guidance_on = ch,
        "control.guidance_off"   => s.controls.guidance_off = ch,
        "control.render_hybrid"  => s.controls.render_hybrid = ch,
        "control.render_raster"  => s.controls.render_raster = ch,
        "control.render_extended" => s.controls.render_extended = ch,
        "control.lock_on"        => s.controls.lock_on = ch,
        "control.lock_off"       => s.controls.lock_off = ch,
        "control.return_probe"   => s.controls.return_probe = ch,
        "gutter.meta"      => s.meta_gutter = ch,
        "gutter.warning"   => s.warning_gutter = ch,
        "gutter.assist"    => s.assist_gutter = ch,
        "dock.following"   => s.dock_following = ch,
        "dock.pinned"      => s.dock_pinned = ch,
        _ => {} // unknown key — ignored
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_glyph_defaults_and_overrides() {
        let s = SymbolSet::default();
        assert_eq!(s.meta_gutter, '▏');
        assert_eq!(s.warning_gutter, '!');
        // resolve(default) keeps defaults.
        assert_eq!(SymbolSet::resolve(&crate::config::SymbolConfig::default()), SymbolSet::default());
        // overrides apply.
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.overrides.insert("gutter.meta".into(), "|".into());
        cfg.overrides.insert("gutter.warning".into(), "*".into());
        let r = SymbolSet::resolve(&cfg);
        assert_eq!(r.meta_gutter, '|');
        assert_eq!(r.warning_gutter, '*');
    }

    #[test]
    fn default_matches_todays_glyphs() {
        let s = SymbolSet::default();
        assert_eq!((s.room_normal.tl, s.room_normal.br, s.room_normal.h), ('╭', '╯', '─'));
        assert_eq!((s.room_current.tl, s.room_current.v), ('┏', '┃'));
        assert_eq!((s.room_portal.tl, s.room_portal.v), ('╔', '║'));
        // selected defaults to the normal set (color-only selection today)
        assert_eq!((s.room_selected.tl, s.room_selected.v), (s.room_normal.tl, s.room_normal.v));
        assert_eq!((s.arrows.north, s.arrows.east, s.arrows.ne), ('▲', '▶', '↗'));
        assert_eq!((s.path.ew, s.path.nesw, s.path.se), ('─', '┼', '┌'));
        assert_eq!(s.portal.marker, '●');
    }

    #[test]
    fn presets_resolve_and_default_names_match_default_set() {
        assert_eq!(BoxStyle::preset("rounded"), Some(SymbolSet::default().room_normal));
        let ascii = BoxStyle::preset("ascii").unwrap();
        assert_eq!((ascii.tl, ascii.h, ascii.v), ('+', '-', '|'));
        let borderless = BoxStyle::preset("borderless").unwrap();
        assert_eq!(borderless.h, ' ');
        assert_eq!(Arrows::preset("filled"), Some(SymbolSet::default().arrows));
        assert_eq!(PathGlyphs::preset("light"), Some(SymbolSet::default().path));
        assert!(BoxStyle::preset("nonsense").is_none());
    }

    #[test]
    fn resolve_default_config_equals_default_set() {
        let cfg = crate::config::SymbolConfig::default();
        assert_eq!(SymbolSet::resolve(&cfg), SymbolSet::default());
    }

    #[test]
    fn resolve_applies_preset_then_override() {
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.box_style = "ascii".into();
        cfg.overrides.insert("room.normal.tl".into(), "#".into());
        let s = SymbolSet::resolve(&cfg);
        assert_eq!(s.room_normal.tl, '#');   // override beats preset
        assert_eq!(s.room_normal.h, '-');    // rest from ascii preset
    }

    #[test]
    fn resolve_rejects_bad_width_override() {
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.overrides.insert("arrow.north".into(), "ab".into());  // multi-char
        cfg.overrides.insert("arrow.south".into(), "".into());    // empty
        let s = SymbolSet::resolve(&cfg);
        assert_eq!(s.arrows.north, SymbolSet::default().arrows.north); // unchanged
        assert_eq!(s.arrows.south, SymbolSet::default().arrows.south);
    }

    #[test]
    fn legacy_computing_diagonals_are_not_rejected_as_wide() {
        // SQ-0314: U+1FBA0..=U+1FBAF sit inside the blanket 0x1F000..=0x1FFFF
        // "emoji" reject, but they are NARROW box-drawing half-diagonals. Without
        // the carve-out every `path.diag_*` override is silently dropped and the
        // slots stop being themeable.
        for cp in 0x1FBA0..=0x1FBAFu32 {
            let ch = char::from_u32(cp).unwrap();
            assert!(!is_wide_estimate(ch), "U+{cp:05X} {ch:?} must be accepted as narrow");
        }
        // The guard is surgical: a neighbouring real emoji is still rejected.
        assert!(is_wide_estimate('\u{1F600}'), "emoji outside the carve-out stay rejected");
        assert!(is_wide_estimate('\u{1FB00}'), "0x1FB00 is below the carve-out and stays rejected");
    }

    #[test]
    fn diagonal_slots_default_to_legacy_computing_and_accept_overrides() {
        // Defaults are the four half-diagonals, and every preset carries them
        // (Legacy Computing has no heavy/dotted variants, so all presets share them).
        let d = SymbolSet::default().path;
        assert_eq!((d.diag_ul, d.diag_ur, d.diag_ll, d.diag_lr),
                   ('🮠', '🮡', '🮢', '🮣'));
        for name in PathGlyphs::preset_names() {
            let p = PathGlyphs::preset(name).unwrap();
            assert_eq!((p.diag_ul, p.diag_ur, p.diag_ll, p.diag_lr),
                       ('🮠', '🮡', '🮢', '🮣'), "preset {name}");
        }
        // And they are themeable: an override reaches the slot (proving the
        // is_wide_estimate carve-out and the apply_override key are both wired).
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.overrides.insert("path.diag_ul".into(), "🮣".into());
        cfg.overrides.insert("path.diag_lr".into(), "/".into());
        let s = SymbolSet::resolve(&cfg);
        assert_eq!(s.path.diag_ul, '🮣', "a Legacy Computing override is accepted");
        assert_eq!(s.path.diag_lr, '/', "an ASCII override is accepted");
    }

    #[test]
    fn diagonal_corners_defaults_on_and_follows_config() {
        assert!(SymbolSet::default().diagonal_corners, "diagonals are on out of the box");
        assert!(SymbolSet::resolve(&crate::config::SymbolConfig::default()).diagonal_corners);
        // Turning it off is what a font without Unicode 13 coverage does.
        let cfg = crate::config::SymbolConfig { diagonal_corners: false, ..Default::default() };
        assert!(!SymbolSet::resolve(&cfg).diagonal_corners);
    }

    #[test]
    fn preset_names_cover_all_known_presets() {
        assert!(BoxStyle::preset_names().contains(&"ascii"));
        assert!(BoxStyle::preset_names().contains(&"rounded"));
        assert!(Arrows::preset_names().contains(&"filled"));
        assert!(PathGlyphs::preset_names().contains(&"light"));
        assert!(PortalGlyphs::preset_names().contains(&"ascii"));
    }

    #[test]
    fn from_preset_names_matches_resolve() {
        let cfg = crate::config::SymbolConfig {
            box_style: "ascii".into(),
            arrow_set: "filled".into(),
            portal_icons: "ascii".into(),
            path_style: "light".into(),
            portal_path_style: crate::config::default_portal_path_style(),
            control_icons: crate::config::default_control_icons(),
            badge_zcode: crate::config::default_badge_zcode(),
            badge_glulx: crate::config::default_badge_glulx(),
            badge_blorb: crate::config::default_badge_blorb(),
            badge_save: crate::config::default_badge_save(),
            badge_hint: crate::config::default_badge_hint(),
            badge_hint_available: crate::config::default_badge_hint_available(),
            diagonal_corners: crate::config::default_diagonal_corners(),
            overrides: std::collections::BTreeMap::new(),
        };
        let expected = SymbolSet::resolve(&cfg);
        let got = SymbolSet::from_preset_names("ascii", "filled", "ascii", "light");
        assert_eq!(got, expected);
    }

    /// The DEFAULT icons must be drawable by an ordinary monospace face, which the
    /// ⊙/⊗ they used to be were not (SQ-0989): Fira Code — the face the gallery
    /// rasterises with — carries no Miscellaneous Mathematical Operators at all.
    /// Geometric Shapes is the block the map already requires for ● ▲ ▼ ◀ ▶, so
    /// pin the default in/out there and a future swap back to a maths operator
    /// fails here instead of on somebody's screen.
    #[test]
    fn default_portal_in_out_come_from_geometric_shapes() {
        let p = PortalGlyphs::preset("ascii").expect("default preset");
        let shapes = 0x25A0..=0x25FF;
        for (slot, ch) in [("in", p.in_), ("out", p.out)] {
            assert!(shapes.contains(&(ch as u32)), "portal.{slot} = {ch:?} is outside Geometric Shapes");
            assert!(!is_wide_estimate(ch), "portal.{slot} = {ch:?} estimates as double-width");
        }
        assert_ne!(p.in_, p.out, "in and out must be told apart");
        assert_ne!(p.in_, p.marker, "in must not read as the notes marker");
        assert_ne!(p.out, p.marker, "out must not read as the notes marker");
        // And the pair is the same one `SymbolSet::default()` hands the renderer.
        assert_eq!((SymbolSet::default().portal.in_, SymbolSet::default().portal.out), (p.in_, p.out));
    }

    /// Codepoints resolved by NAME from the Nerd Fonts `glyphnames.json` (v3.5.1),
    /// not from memory: up/down here were F0B71/F0B72 — `md-card_bulleted_off` and
    /// its outline — for as long as the preset existed, and a patched face draws
    /// those happily, so nothing looked broken (SQ-0989).
    #[test]
    fn nerdfont_portal_icons_are_the_named_codepoints() {
        let p = PortalGlyphs::preset("nerdfont").expect("preset");
        assert_eq!(p.up, '\u{F0CE1}', "md-arrow_up_circle");
        assert_eq!(p.down, '\u{F0CDB}', "md-arrow_down_circle");
        assert_eq!(p.in_, '\u{F090}', "fa-sign_in");
        assert_eq!(p.out, '\u{F08B}', "fa-sign_out");
        assert_eq!(p.marker, '\u{F111}', "fa-circle");
        assert_eq!(p.unknown, '\u{F059}', "fa-question_circle");
    }

    #[test]
    fn nerdfont_stairs_portal_has_four_distinct_single_width_icons() {
        assert!(PortalGlyphs::preset_names().contains(&"nerdfont-stairs"));
        let p = PortalGlyphs::preset("nerdfont-stairs").unwrap();
        // four DISTINCT direction icons
        let four = [p.up, p.down, p.in_, p.out];
        for ch in four { assert!(!is_wide_estimate(ch)); }
        assert_eq!(four.iter().collect::<std::collections::HashSet<_>>().len(), 4, "up/down/in/out must differ");
    }

    /// The border controls' nerdfont set is ELEVEN named icons, each codepoint
    /// read from the font's own `post` table rather than inferred from a name.
    ///
    /// This pins the numbers, because nothing else can: SQ-0989 is what a
    /// guessed codepoint costs — a patched face draws the wrong icon crisply and
    /// confidently, and there is no assertion on our side of the terminal that
    /// could notice. Two of the names first proposed for this set turned out not
    /// to exist in the font (`cod-layout_panel_dock`, `md-post_map`), which is
    /// the same failure caught one step earlier.
    #[test]
    fn nerdfont_control_glyphs_are_the_names_that_were_read_from_the_font() {
        let c = ControlGlyphs::preset("nerdfont").expect("preset");
        for (name, got, want) in [
            ("fa-map_location", c.map_show, '\u{0EE68}'),
            ("fa-map_location_dot", c.map_hide, '\u{0EE69}'),
            ("cod-layout_panel_off", c.band_show, '\u{0EC01}'),
            ("cod-layout_panel", c.band_hide, '\u{0EBF2}'),
            ("md-help", c.guidance_off, '\u{F02D6}'),
            ("md-post_lamp", c.guidance_on, '\u{F1A60}'),
            ("md-monitor", c.render_hybrid, '\u{F0379}'),
            ("md-monitor_shimmer", c.render_raster, '\u{F1104}'),
            ("md-monitor_star", c.render_extended, '\u{F0DDC}'),
            ("md-lock_open", c.lock_off, '\u{F033F}'),
            ("md-lock", c.lock_on, '\u{F033E}'),
            ("md-shoe_print", c.return_probe, '\u{F0DFA}'),
        ] {
            assert_eq!(got, want, "{name} moved: U+{:05X} is not U+{:05X}", got as u32, want as u32);
        }
        // The Guiding Light's lit mark is the SAME glyph the gutter draws, not a
        // second lamp that could drift away from it.
        assert_eq!(c.guidance_on, crate::render::font_check_dialog::ASSIST_LAMP);
        // Each toggle's two states must still differ, and each pair must stay
        // inside ONE icon family — mixed families have different stroke weights
        // and cap heights, so the control appears to jump on toggle.
        for (slot, off, on) in [
            ("map", c.map_show, c.map_hide),
            ("band", c.band_show, c.band_hide),
            ("guidance", c.guidance_off, c.guidance_on),
            ("lock", c.lock_off, c.lock_on),
        ] {
            assert_ne!(off, on, "control.{slot}'s two states are the same glyph");
        }
        for (slot, a, b) in [
            ("map", c.map_show as u32, c.map_hide as u32),
            ("band", c.band_show as u32, c.band_hide as u32),
            ("guidance", c.guidance_off as u32, c.guidance_on as u32),
            ("lock", c.lock_off as u32, c.lock_on as u32),
        ] {
            // `fa-`/`cod-` live in the 0xE000 private-use block, `md-` above
            // 0xF0000; a pair straddling that line is two families.
            assert_eq!(
                a >= 0xF_0000, b >= 0xF_0000,
                "control.{slot}'s two states come from different icon families",
            );
        }
        // …and the render mode's three are one family too.
        for ch in [c.render_hybrid, c.render_raster, c.render_extended] {
            assert!(ch as u32 >= 0xF_0000, "the render icons are all Material Design");
        }
        for (slot, ch) in [
            ("map_show", c.map_show), ("map_hide", c.map_hide),
            ("band_show", c.band_show), ("band_hide", c.band_hide),
            ("guidance_on", c.guidance_on), ("guidance_off", c.guidance_off),
            ("render_hybrid", c.render_hybrid), ("render_raster", c.render_raster),
            ("render_extended", c.render_extended),
            ("lock_on", c.lock_on), ("lock_off", c.lock_off),
            ("return_probe", c.return_probe),
        ] {
            assert!(!is_wide_estimate(ch), "control.{slot} = {ch:?} estimates as double-width");
        }
        // The return probe is Material Design like the rest of the `md-` set, and
        // it is exempt from the two-states rule above because it HAS one state:
        // its off-reading is the muted colour, not a second glyph (SQ-0785).
        assert!(c.return_probe as u32 >= 0xF_0000, "md-shoe_print is Material Design");
    }

    /// The PLAIN defaults must be drawable by an ordinary monospace face, so
    /// every one of them comes out of Geometric Shapes — the block the map
    /// already requires (see `default_portal_in_out_come_from_geometric_shapes`).
    /// And each toggle's two states must actually differ, or the icon says
    /// nothing and only the colour is left carrying it.
    #[test]
    fn plain_control_glyphs_are_geometric_shapes_and_tell_their_states_apart() {
        let c = ControlGlyphs::preset("plain").expect("the default preset");
        assert_eq!(c, SymbolSet::default().controls);
        let shapes = 0x25A0..=0x25FF;
        for (slot, ch) in [
            ("map_show", c.map_show), ("map_hide", c.map_hide),
            ("band_show", c.band_show), ("band_hide", c.band_hide),
            ("guidance_on", c.guidance_on), ("guidance_off", c.guidance_off),
            ("render_hybrid", c.render_hybrid), ("render_raster", c.render_raster),
            ("render_extended", c.render_extended),
            ("lock_on", c.lock_on), ("lock_off", c.lock_off),
            ("return_probe", c.return_probe),
        ] {
            assert!(shapes.contains(&(ch as u32)), "control.{slot} = {ch:?} is outside Geometric Shapes");
            assert!(!is_wide_estimate(ch), "control.{slot} = {ch:?} estimates as double-width");
        }
        assert_ne!(c.map_show, c.map_hide);
        assert_ne!(c.band_show, c.band_hide);
        assert_ne!(c.guidance_on, c.guidance_off);
        assert_ne!(c.lock_on, c.lock_off);
        // Three render modes, three distinct glyphs.
        let modes = [c.render_hybrid, c.render_raster, c.render_extended];
        assert_eq!(modes.iter().collect::<std::collections::HashSet<_>>().len(), 3);
        // …and the return probe's single mark is not any of the others, so it is
        // still legible as its own control on a border that draws several
        // (SQ-0785). It has no second state by design — the colour carries that.
        let all = [
            c.map_show, c.map_hide, c.band_show, c.band_hide, c.guidance_on, c.guidance_off,
            c.render_hybrid, c.render_raster, c.render_extended, c.lock_on, c.lock_off,
        ];
        assert!(!all.contains(&c.return_probe), "the footprint is its own mark");
    }

    /// Every control slot is themeable one glyph at a time, the way every other
    /// family is: a key `apply_override` silently ignores is a knob that does
    /// nothing (SQ-0558).
    #[test]
    fn every_control_slot_accepts_an_override() {
        let baseline = SymbolSet::resolve(&crate::config::SymbolConfig::default());
        for key in [
            "control.map_show", "control.map_hide", "control.band_show", "control.band_hide",
            "control.guidance_on", "control.guidance_off", "control.render_hybrid",
            "control.render_raster", "control.render_extended", "control.lock_on",
            "control.lock_off",
        ] {
            let mut cfg = crate::config::SymbolConfig::default();
            cfg.overrides.insert(key.into(), "#".into());
            assert_ne!(SymbolSet::resolve(&cfg), baseline, "override {key} changed nothing");
        }
    }

    #[test]
    fn nf_arrow_presets_exist_and_are_single_width() {
        for name in ["nf-bold","nf-box","nf-circle","nf-outline"] {
            assert!(Arrows::preset_names().contains(&name), "{name} missing");
            let a = Arrows::preset(name).expect("preset");
            for ch in [a.north,a.south,a.east,a.west,a.ne,a.nw,a.se,a.sw] {
                assert!(!is_wide_estimate(ch), "{name}: wide char {:?}", ch);
            }
        }
        // verified cardinal codepoints for nf-bold:
        let b = Arrows::preset("nf-bold").unwrap();
        assert_eq!(b.north, '\u{F0737}');
        assert_eq!(b.south, '\u{F072E}');
        assert_eq!(b.east,  '\u{F0734}');
        assert_eq!(b.west,  '\u{F0731}');
        // nf-box native diagonals:
        let bx = Arrows::preset("nf-box").unwrap();
        assert_eq!(bx.ne, '\u{F196A}');
        assert_eq!(bx.nw, '\u{F1968}');
        assert_eq!(bx.se, '\u{F1966}');
        assert_eq!(bx.sw, '\u{F1964}');
    }
}
