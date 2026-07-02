/// Configurable map symbols for the babelmap renderer.
///
/// All glyphs the map renderer uses are centralized here. The defaults reproduce
/// today's hardcoded literals exactly, so an absent `[symbols]` config changes nothing.

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
    /// In portal icon (⊙).
    pub in_: char,
    /// Out portal icon (⊗).
    pub out: char,
    /// Unknown portal icon (?).
    pub unknown: char,
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
    /// Gutter marker glyph for META transcript lines.
    pub meta_gutter: char,
    /// Gutter marker glyph for WARNING transcript lines.
    pub warning_gutter: char,
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
            },
            portal: PortalGlyphs {
                marker: '●',
                path: '┊',
                path_h: '┄',
                up: '↑',
                down: '↓',
                in_: '⊙',
                out: '⊗',
                unknown: '?',
            },
            meta_gutter: '▏',
            warning_gutter: '!',
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
    ///                   (heavy block frame with beveled inner corners)
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
    ///                  Cardinal: chevron-up (U+F0143) chevron-down (U+F0140)
    ///                            chevron-right (U+F0142) chevron-left (U+F0141)
    ///                  Diagonal: same as "line" (↗↖↘↙)
    /// - "nf-bold"    — MDI arrow-{up,down,left,right}-bold (F0737/F072E/F0731/F0734)
    ///                  Diagonal: Unicode fallback ↖↗↙↘ (no native MDI bold diagonals)
    /// - "nf-box"     — MDI arrow-{up,down,left,right}-bold-box (F0738/F072F/F0732/F0735)
    ///                  Diagonal: native MDI bold-box diagonals (F1968/F196A/F1964/F1966)
    /// - "nf-circle"  — MDI arrow-{up,down,left,right}-bold-circle (F005F/F0047/F004F/F0056)
    ///                  Diagonal: Unicode fallback ↖↗↙↘ (no native MDI circle diagonals)
    /// - "nf-outline" — MDI arrow-{up,down,left,right}-bold-outline (F09C7/F09BF/F09C0/F09C2)
    ///                  Diagonal: native MDI bold-outline diagonals (F09C3/F09C5/F09B7/F09B9)
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
            "light" => PathGlyphs {
                ew: '─', ns: '│', se: '┌', sw: '┐', ne: '└', nw: '┘',
                nse: '├', nsw: '┤', ews: '┬', ewn: '┴', nesw: '┼',
            },
            "heavy" => PathGlyphs {
                ew: '━', ns: '┃', se: '┏', sw: '┓', ne: '┗', nw: '┛',
                nse: '┣', nsw: '┫', ews: '┳', ewn: '┻', nesw: '╋',
            },
            "dotted" => PathGlyphs {
                // Quadruple-dash light for straights; turns fall back to light corners
                // since Unicode has no dotted corner glyphs.
                ew: '┄', ns: '┆', se: '┌', sw: '┐', ne: '└', nw: '┘',
                nse: '├', nsw: '┤', ews: '┬', ewn: '┴', nesw: '┼',
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

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "ascii"            — ASCII-compatible glyphs (default): ●/↑/↓/⊙/⊗/? with ┊┄ connectors
    /// - "nerdfont"         — Nerd Font single-width icon codepoints (requires patched font)
    ///                        nf-fa-circle (U+F111) for marker, nf-md-arrow_up_circle (U+F0B71) for up,
    ///                        nf-md-arrow_down_circle (U+F0B72) for down, nf-fa-sign_in (U+F090) for in,
    ///                        nf-fa-sign_out (U+F08B) for out, nf-fa-question_circle (U+F059) for unknown
    /// - "nerdfont-stairs"  — Nerd Font 4 distinct direction icons (requires patched font)
    ///                        up=mdi-stairs-up (U+F12BD), down=mdi-stairs-down (U+F12BE),
    ///                        in=mdi-location-enter (U+F0FC4), out=mdi-exit-run (U+F0A48)
    pub fn preset(name: &str) -> Option<PortalGlyphs> {
        Some(match name {
            "ascii" => PortalGlyphs {
                marker: '●', path: '┊', path_h: '┄',
                up: '↑', down: '↓', in_: '⊙', out: '⊗', unknown: '?',
            },
            "nerdfont" => PortalGlyphs {
                // nf-fa-circle U+F111, connectors keep the same box-drawing chars
                marker: '\u{F111}', path: '┊', path_h: '┄',
                // nf-md-arrow_up_circle U+F0B71, nf-md-arrow_down_circle U+F0B72
                up: '\u{F0B71}', down: '\u{F0B72}',
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
            meta_gutter: SymbolSet::default().meta_gutter,
            warning_gutter: SymbolSet::default().warning_gutter,
        };

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
            badge_zcode: crate::config::default_badge_zcode(),
            badge_glulx: crate::config::default_badge_glulx(),
            badge_blorb: crate::config::default_badge_blorb(),
            badge_save: crate::config::default_badge_save(),
            badge_hint: crate::config::default_badge_hint(),
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
        "portal.up"        => s.portal.up = ch,
        "portal.down"      => s.portal.down = ch,
        "portal.in"        => s.portal.in_ = ch,
        "portal.out"       => s.portal.out = ch,
        "portal.unknown"   => s.portal.unknown = ch,
        "portal.path"      => s.portal.path = ch,
        "portal.marker"    => s.portal.marker = ch,
        "gutter.meta"      => s.meta_gutter = ch,
        "gutter.warning"   => s.warning_gutter = ch,
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
            badge_zcode: crate::config::default_badge_zcode(),
            badge_glulx: crate::config::default_badge_glulx(),
            badge_blorb: crate::config::default_badge_blorb(),
            badge_save: crate::config::default_badge_save(),
            badge_hint: crate::config::default_badge_hint(),
            overrides: std::collections::BTreeMap::new(),
        };
        let expected = SymbolSet::resolve(&cfg);
        let got = SymbolSet::from_preset_names("ascii", "filled", "ascii", "light");
        assert_eq!(got, expected);
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
