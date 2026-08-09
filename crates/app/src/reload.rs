//! Live style reload: re-resolve style.toml from disk and swap the live
//! ColorScheme / SymbolSet, keeping the current look on a parse error.

use std::path::Path;

use crate::state::AppState;

/// Result of a reload attempt.
pub enum ReloadOutcome {
    /// Applied; carries any non-fatal resolve warnings — both `style::resolve`'s
    /// (legacy scheme-path warnings) and, appended after (SQ-0663), the
    /// registry theme's own [`crate::theme::resolve::Theme::warnings`]
    /// (formatted via [`crate::theme::resolve::describe_theme_warnings`]), so a
    /// `parent` typo or cycle in style.toml surfaces through the exact same
    /// notice path every other reload warning already uses.
    Reloaded { warnings: Vec<String> },
    /// Not applied (read/parse error); the current look is untouched.
    Failed { msg: String },
}

/// Resolve the `style` pointer to its on-disk path, if it names a real file.
/// Returns `None` for the built-in `"default"` or when `None` resolves to a
/// missing `user_dir/style.toml` (those parse the embedded default — no file).
pub fn resolved_style_path(style: Option<&str>, user_dir: &Path) -> Option<std::path::PathBuf> {
    match style {
        Some("default") => None,
        Some(p) => Some(crate::colors::expand_path(p, user_dir)),
        None => {
            let cand = user_dir.join("style.toml");
            if cand.is_file() { Some(cand) } else { None }
        }
    }
}

/// Re-read and apply `style.toml`. On a real-file read/parse error, the current
/// `state.colors`/`state.symbols` are left in place.
pub fn reload_style(state: &mut AppState) -> ReloadOutcome {
    let user_dir = state.config.user_dir.clone();
    let pointer = state.config.style.clone();

    // Build the StyleDoc: a real file parses directly (error → Failed); the
    // default/missing cases use the embedded default via load_style.
    let doc = match resolved_style_path(pointer.as_deref(), &user_dir) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => match crate::style::parse_style_toml(&text) {
                Ok(doc) => doc,
                Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", path.display(), e) },
            },
            Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", path.display(), e) },
        },
        None => {
            let (doc, _w) = crate::style::load_style(pointer.as_deref(), &user_dir);
            doc
        }
    };

    // Layer the per-game override (<game_dir>/style.toml) over the global.
    let doc = if !state.game_dir.as_os_str().is_empty() {
        let pg_path = crate::styles::per_game_style_path(&state.game_dir);
        if pg_path.is_file() {
            match std::fs::read_to_string(&pg_path) {
                Ok(text) => match crate::style::parse_style_toml(&text) {
                    Ok(over) => crate::style::merge(&doc, &over),
                    Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", pg_path.display(), e) },
                },
                Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", pg_path.display(), e) },
            }
        } else {
            doc
        }
    } else {
        doc
    };

    let (cs, set, warnings) = crate::style::resolve(&doc, &user_dir);
    state.colors = cs;
    state.symbols = set;
    // Re-apply the per-game garglk.ini overlay (SQ-0319): the resolve above
    // rebuilds `colors` from style.toml + <game_dir>/style.toml and would drop it,
    // so fold it back on so the imported look survives every reload path.
    if let Some(ov) = state.garglk_overlay.clone() {
        ov.apply(&mut state.colors);
    }
    // Honor-game-colours precedence (SQ-0318): the user's explicit per-game
    // override wins over the garglk.ini `stylehint` gate, which in turn wins over
    // the global config default. Recompute from disk each reload so `auto` (no
    // per-game override) falls back to garglk/global, and an explicit per-game
    // choice is never clobbered by the garglk gate.
    let per_game_honor = if state.game_dir.as_os_str().is_empty() {
        None
    } else {
        crate::styles::read_per_game_honor(&state.game_dir)
    };
    let garglk_honor = state.garglk_overlay.as_ref().and_then(|o| o.honor_game_colours);
    state.config.honor_game_colours =
        per_game_honor.or(garglk_honor).unwrap_or(state.honor_game_colours_base);

    // SQ-0309: rebuild the registry theme from the same global + per-game files, with
    // provenance. Render still reads the legacy fields above; the theme is consumed by
    // the Wave-5 render migration. New-schema sections drive it; an old-schema file
    // (only [colors]/[symbols]) parses to empty new-schema decls → registry defaults.
    {
        use crate::theme::{resolve::resolve_theme_layered, toml_schema};
        let parse_file = |p: Option<std::path::PathBuf>| -> toml_schema::ParsedStyle {
            p.and_then(|p| std::fs::read_to_string(&p).ok())
                .and_then(|t| toml_schema::parse(&t).ok())
                .unwrap_or_default()
        };
        let global = parse_file(resolved_style_path(pointer.as_deref(), &user_dir));
        let per_game = if state.game_dir.as_os_str().is_empty() {
            toml_schema::ParsedStyle::default()
        } else {
            parse_file(Some(crate::styles::per_game_style_path(&state.game_dir)))
        };
        let (_, gs, _) = crate::colors::resolve_base(global.scheme.as_deref(), &user_dir);
        // SQ-0510: with no scheme configured, `resolve_base` returns an all-Reset
        // scheme and `Roles::from_scheme` falls back to a hard-coded white-on-BLACK
        // `chrome` — a black band across every chrome-derived selector on a
        // terminal that is not black. Seed the scheme from the OSC 10/11 probe
        // taken at startup so the theme follows the real terminal. This is the ONE
        // place the theme is built (startup, `/reload-style`, the style watcher and
        // the per-game overlay all funnel through `reload_style`), so seeding here
        // reaches all of them. A terminal that did not answer both channels leaves
        // the scheme untouched and today's behaviour intact.
        let gs = crate::colors::seed_scheme_from_terminal(gs, &state.term_default_colors);
        // SQ-0309: a discovered garglk.ini's colours ride the theme's garglk layer
        // (between global and per-game) so they survive the migration off the legacy
        // ColorScheme fields (`ov.apply` still seeds the glk override array, which the
        // render glk path reads for per-style colours).
        let garglk_decls = state.garglk_overlay.as_ref().map(|o| o.color_decls()).unwrap_or_default();
        state.colors.theme = resolve_theme_layered(&gs, &global, &garglk_decls, &per_game);
    }

    // SQ-0700: the upper-window frame's STYLE still lives on `ColorScheme` (the
    // renderer frames and reserves from it), but the file it is written in is the
    // new schema, where the selector sits in `[elements]` and lands in the theme
    // just rebuilt above. Lower it back so `upper_window_border = { style = "none" }`
    // reaches the frame instead of dying in the theme.
    crate::style::lower_upper_window_border(&mut state.colors);

    // SQ-0663: append the freshly-rebuilt theme's own diagnostics (a `parent`
    // re-root naming an unknown selector, or a cycle) after style::resolve's.
    // Every consumer of `ReloadOutcome::Reloaded` already loops over `warnings`
    // and pushes each as a transcript Warning, so folding these in here — rather
    // than adding a second warnings channel — reuses that path for free and
    // keeps the "quiet when clean" property: a reload whose theme resolves
    // cleanly appends nothing, so nothing is re-announced.
    let mut warnings = warnings;
    warnings.extend(crate::theme::resolve::describe_theme_warnings(state.colors.theme.warnings()));

    ReloadOutcome::Reloaded { warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("babelmap-reload-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reload_applies_style_file_and_keeps_current_on_error() {
        // SQ-0309: the legacy `[colors]` selector table no longer patches
        // `ColorScheme` fields directly (that now flows through the theme —
        // see `reload_builds_theme_from_new_schema_files`), so this exercises
        // the new-schema `[elements]` table instead. The behaviour under test
        // — reload keeps the current look when the file fails to parse — is
        // unrelated to which schema drives the colour.
        use ratatui::style::Color;
        let dir = temp_dir("ok");
        let path = dir.join("style.toml");
        std::fs::write(&path, "[elements]\ntranscript = { fg = \"green\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(path.to_string_lossy().to_string());

        let outcome = reload_style(&mut state);
        assert!(matches!(outcome, ReloadOutcome::Reloaded { .. }));
        assert_eq!(state.colors.theme.get("transcript").style.fg, Some(Color::Green));

        // Now break the file: reload keeps the current (green) look and reports Failed.
        std::fs::write(&path, "this is not valid = = toml [[[").unwrap();
        let outcome2 = reload_style(&mut state);
        assert!(matches!(outcome2, ReloadOutcome::Failed { .. }));
        assert_eq!(
            state.colors.theme.get("transcript").style.fg, Some(Color::Green),
            "current look preserved on error"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A style.toml is required for reload_style's file path; seed a minimal one.
    fn seed_style(dir: &std::path::Path) -> std::path::PathBuf {
        let global = dir.join("style.toml");
        std::fs::write(&global, "[colors]\n\"transcript\" = { fg = \"white\" }\n").unwrap();
        global
    }

    #[test]
    fn per_game_honor_override_beats_global_and_auto_falls_back() {
        let dir = temp_dir("honor");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        // Global default (base) is honor = true.
        state.honor_game_colours_base = true;
        state.config.honor_game_colours = true;

        // A per-game override of false wins over the global true.
        crate::styles::write_per_game_honor(&game_dir, Some(false)).unwrap();
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "per-game false wins over global true");

        // `auto` (override cleared) falls back to the global base (true).
        crate::styles::write_per_game_honor(&game_dir, None).unwrap();
        reload_style(&mut state);
        assert!(state.config.honor_game_colours, "auto falls back to global base");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_honor_wins_over_garglk_and_auto_falls_back_to_garglk() {
        let dir = temp_dir("honor-garglk");
        let global = seed_style(&dir);
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();
        // Global base true; a garglk.ini stylehint set honor = true.
        state.honor_game_colours_base = true;
        state.garglk_overlay = Some(crate::garglk_ini::GarglkOverlay {
            honor_game_colours: Some(true),
            ..Default::default()
        });

        // A per-game override of false wins over the garglk-set true.
        crate::styles::write_per_game_honor(&game_dir, Some(false)).unwrap();
        reload_style(&mut state);
        assert!(!state.config.honor_game_colours, "per-game false wins over garglk true");

        // `auto` falls back to the garglk value (true), not just the base.
        crate::styles::write_per_game_honor(&game_dir, None).unwrap();
        reload_style(&mut state);
        assert!(state.config.honor_game_colours, "auto falls back to garglk stylehint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_builds_theme_from_new_schema_files() {
        use ratatui::style::Color;
        let dir = temp_dir("theme-newschema");
        let global = dir.join("style.toml");
        std::fs::write(&global, "[roles]\naccent = { fg = \"green\" }\n").unwrap();
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("style.toml"), "[roles]\naccent = { fg = \"red\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();

        let outcome = reload_style(&mut state);
        assert!(matches!(outcome, ReloadOutcome::Reloaded { .. }));
        let r = state.colors.theme.get("accent");
        assert_eq!(r.style.fg, Some(Color::Red), "per-game [roles] wins");
        assert_eq!(r.provenance, crate::theme::resolve::Provenance::PerGame);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_layers_per_game_map_glyph_presets_over_the_global() {
        // The `[map]` glyph knobs must layer through real files exactly like
        // colours do (SQ-0557/0558): the per-game file wins where it speaks, the
        // global stands where it is silent.
        let dir = temp_dir("map-presets");
        let global = dir.join("style.toml");
        std::fs::write(&global, "[map]\nbox_style = \"double\"\ndiagonal_corners = false\n").unwrap();
        let game_dir = dir.join("game.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("style.toml"), "[map]\nbox_style = \"ascii\"\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());
        state.game_dir = game_dir.clone();

        assert!(matches!(reload_style(&mut state), ReloadOutcome::Reloaded { .. }));
        assert_eq!(
            state.symbols.room_normal,
            crate::symbols::BoxStyle::preset("ascii").unwrap(),
            "per-game box_style wins"
        );
        assert!(!state.symbols.diagonal_corners, "global diagonal_corners stands");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0510: the probed terminal page reaches the theme's chrome roles ───

    /// The user's reported configuration exactly: no `style.toml`, no scheme —
    /// just an empty user dir and whatever the OSC 10/11 probe answered.
    fn state_with_probe(
        tag: &str,
        probe: crate::term_colors::TermDefaultColors,
    ) -> (AppState, std::path::PathBuf) {
        let dir = temp_dir(tag);
        let _ = std::fs::remove_file(dir.join("style.toml"));
        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = None; // no style.toml, no [colors] scheme
        state.term_default_colors = probe;
        match reload_style(&mut state) {
            ReloadOutcome::Reloaded { .. } => {}
            ReloadOutcome::Failed { msg } => panic!("the default look must load: {msg}"),
        }
        (state, dir)
    }

    fn rgba(r: u8, g: u8, b: u8) -> image::Rgba<u8> {
        image::Rgba([r, g, b, 255])
    }

    #[test]
    fn reload_seeds_the_chrome_roles_from_an_answered_terminal_probe() {
        // The bug: Anchorhead's status bar painted a BLACK band on a terminal
        // whose background is not black. The story sets no colours at all — the
        // black came from `Roles::terminal_default`'s hard-coded chrome, because
        // the OSC probe's answer was stored in AppState and never handed to the
        // theme. `reload_style` is the one place the theme is built (startup,
        // /reload-style, the style watcher and the per-game overlay all funnel
        // here), so the seeding has to land — and be observable — here.
        use ratatui::style::Color;
        let (state, dir) = state_with_probe(
            "probe-on",
            crate::term_colors::TermDefaultColors {
                fg: Some(rgba(0x58, 0x6e, 0x75)),
                bg: Some(rgba(0xfd, 0xf6, 0xe3)), // Solarized Light: emphatically not black
            },
        );

        let uw = state.colors.theme.get("upper_window").style;
        assert_ne!(uw.bg, Some(Color::Black), "no black band on a non-black terminal");
        assert_eq!(uw.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)), "the upper window follows the terminal page");
        assert_eq!(uw.fg, Some(Color::Rgb(0x58, 0x6e, 0x75)));
        assert_eq!(state.colors.theme.get("chrome").style.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
        assert_eq!(state.colors.theme.get("status_bar").style.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
        // The out-of-box accents are untouched — this is a page fix, not a repaint.
        assert_eq!(state.colors.theme.get("panel.border").style.fg, Some(Color::Cyan));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_keeps_todays_behaviour_when_the_terminal_answers_nothing_or_half() {
        // A terminal that never answers (or answers only one channel) must be
        // byte-for-byte what it is today: the concrete terminal-default roles,
        // chrome included. A probed ink is never mixed onto a guessed page.
        use ratatui::style::Color;
        let cases = [
            ("probe-off", crate::term_colors::TermDefaultColors::default()),
            (
                "probe-fg-only",
                crate::term_colors::TermDefaultColors { fg: Some(rgba(0x58, 0x6e, 0x75)), bg: None },
            ),
            (
                "probe-bg-only",
                crate::term_colors::TermDefaultColors { fg: None, bg: Some(rgba(0xfd, 0xf6, 0xe3)) },
            ),
        ];
        for (tag, probe) in cases {
            let (state, dir) = state_with_probe(tag, probe);
            let uw = state.colors.theme.get("upper_window").style;
            assert_eq!(uw.bg, Some(Color::Black), "{tag}: unchanged fallback");
            assert_eq!(uw.fg, Some(Color::White), "{tag}: unchanged fallback");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_configured_scheme_still_wins_over_the_probe() {
        // The probe only fills the gap left by "no scheme configured". A user who
        // picked a scheme gets that scheme's page, whatever the terminal says.
        use ratatui::style::Color;
        let dir = temp_dir("probe-scheme");
        let path = dir.join("style.toml");
        std::fs::write(&path, "version = 1\nscheme = \"tomorrow-night\"\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(path.to_string_lossy().to_string());
        state.term_default_colors = crate::term_colors::TermDefaultColors {
            fg: Some(rgba(0, 0, 0)),
            bg: Some(rgba(255, 255, 255)),
        };
        assert!(matches!(reload_style(&mut state), ReloadOutcome::Reloaded { .. }));

        let bg = state.colors.theme.get("upper_window").style.bg;
        assert_ne!(bg, Some(Color::Rgb(255, 255, 255)), "the probe must not overwrite a chosen scheme");
        assert_eq!(bg, Some(Color::Rgb(0x1d, 0x1f, 0x21)), "tomorrow-night's own page");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0663: theme resolution warnings ride the same reload notice path ──

    #[test]
    fn reload_surfaces_a_theme_warning_for_a_typo_d_parent() {
        // Falsification (a)/(b): with the SQ-0663 surfacing removed, this fails
        // because `warnings` never carries a line naming the typo.
        let dir = temp_dir("theme-warn");
        let global = dir.join("style.toml");
        std::fs::write(&global, "[panel]\ntitle = { parent = \"acent\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());

        match reload_style(&mut state) {
            ReloadOutcome::Reloaded { warnings } => {
                assert!(
                    warnings.iter().any(|w| w.contains("panel.title") && w.contains("acent")),
                    "theme warning must ride the reload's warnings: {:?}",
                    warnings
                );
            }
            ReloadOutcome::Failed { msg } => panic!("expected Reloaded, got Failed({msg})"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_stays_quiet_when_the_theme_resolves_cleanly() {
        // The other half of the falsification: a reload whose style.toml has no
        // `parent` issues must not manufacture a warning out of nothing, and a
        // SECOND clean reload must not re-announce anything either.
        let dir = temp_dir("theme-warn-clean");
        let global = seed_style(&dir);

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(global.to_string_lossy().to_string());

        for _ in 0..2 {
            match reload_style(&mut state) {
                ReloadOutcome::Reloaded { warnings } => {
                    assert!(warnings.is_empty(), "clean style.toml must carry no warnings: {:?}", warnings);
                }
                ReloadOutcome::Failed { msg } => panic!("expected Reloaded, got Failed({msg})"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
