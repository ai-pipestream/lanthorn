//! Live style reload: re-resolve style.toml from disk and swap the live
//! ColorScheme / SymbolSet, keeping the current look on a parse error.

use std::path::Path;

use crate::state::AppState;

/// Result of a reload attempt.
pub enum ReloadOutcome {
    /// Applied; carries any non-fatal resolve warnings.
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
        // SQ-0309: a discovered garglk.ini's colours ride the theme's garglk layer
        // (between global and per-game) so they survive the migration off the legacy
        // ColorScheme fields (`ov.apply` still seeds the glk override array, which the
        // render glk path reads for per-style colours).
        let garglk_decls = state.garglk_overlay.as_ref().map(|o| o.color_decls()).unwrap_or_default();
        state.colors.theme = resolve_theme_layered(&gs, &global, &garglk_decls, &per_game);
    }

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
}
