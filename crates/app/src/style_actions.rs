//! Style-editor action handling and its state helpers, peeled out of input.rs
//! (SQ-0306). `apply_action` delegates its style-editor arm cluster here; the arm
//! bodies are unchanged from their original in-place form.

use crate::input::{border_zone_from_index, is_bordered_selector, set_zone_glyph, Action, AttrKind};
use crate::state::AppState;

/// Number of footer buttons in the style editor (Save, Save Game, Cancel).
const STYLE_BUTTON_COUNT: usize = 3;

/// Dispatch a style-editor `Action`. Called by `apply_action` for its style-editor
/// arm cluster; only ever invoked with those variants.
pub(crate) fn apply_style_action(action: Action, state: &mut AppState) {
    match action {
        Action::OpenStyleEditor => {
            open_style_editor(state);
        }

        Action::StyleEditorCancel => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &state.overlays.style_editor {
                let _ = crate::style_mru::save_mru(&dir, &ed.mru);
            }
            state.overlays.style_editor = None;
        }

        Action::StyleNav(d) => {
            if let Some(ed) = &mut state.overlays.style_editor {
                let n = ed.selectors.len() as i32;
                ed.active = ((ed.active as i32 + d).rem_euclid(n.max(1))) as usize;
                if ed.focus == crate::state::StyleFocus::Border
                    && !is_bordered_selector(ed.selectors[ed.active])
                {
                    ed.focus = crate::state::StyleFocus::Board;
                }
            }
        }

        Action::StyleToggleAttr(kind) => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.overlays.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                let slot = match kind {
                    AttrKind::Bold      => &mut decl.bold,
                    AttrKind::Italic    => &mut decl.italic,
                    AttrKind::Underline => &mut decl.underline,
                    AttrKind::Dim       => &mut decl.dim,
                    AttrKind::Reversed  => &mut decl.reversed,
                };
                *slot = Some(!slot.unwrap_or(false));
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleFocusCycle(d) => {
            use crate::state::StyleFocus;
            // The tab order is the body focus ring followed by each footer button
            // as its own stop, so Tab/Shift-Tab step through Save, Save Game and
            // Cancel individually before leaving the button row. Body stops keep
            // their original indices, so multi-step deltas still land correctly.
            enum Stop {
                Body(StyleFocus),
                Button(usize),
            }
            let ed_info = state
                .overlays.style_editor
                .as_ref()
                .map(|ed| (is_bordered_selector(ed.selectors[ed.active]), ed.focus));
            if let Some((bordered, cur_focus)) = ed_info {
                let mut stops = vec![
                    Stop::Body(StyleFocus::Board),
                    Stop::Body(StyleFocus::Fg),
                    Stop::Body(StyleFocus::Bg),
                    Stop::Body(StyleFocus::Custom),
                    Stop::Body(StyleFocus::Attrs),
                ];
                if bordered {
                    stops.push(Stop::Body(StyleFocus::Border));
                }
                for b in 0..STYLE_BUTTON_COUNT {
                    stops.push(Stop::Button(b));
                }
                let cur = stops
                    .iter()
                    .position(|s| match s {
                        Stop::Body(f) => *f == cur_focus,
                        Stop::Button(b) => cur_focus == StyleFocus::Buttons && *b == state.overlays.dialog_focus,
                    })
                    .unwrap_or(0) as i32;
                let n = stops.len() as i32;
                match &stops[((cur + d).rem_euclid(n)) as usize] {
                    Stop::Body(f) => {
                        if let Some(ed) = &mut state.overlays.style_editor {
                            ed.focus = *f;
                            match f {
                                StyleFocus::Fg => ed.color_target = false,
                                StyleFocus::Bg => ed.color_target = true,
                                StyleFocus::Custom
                                    if ed.custom_buf.is_empty() => {
                                        ed.custom_buf = "#".to_string();
                                    }
                                _ => {}
                            }
                        }
                    }
                    Stop::Button(b) => {
                        if let Some(ed) = &mut state.overlays.style_editor {
                            ed.focus = StyleFocus::Buttons;
                        }
                        state.overlays.dialog_focus = *b;
                    }
                }
            }
        }

        Action::StyleAttrChipNav(d) => {
            if let Some(ed) = &mut state.overlays.style_editor {
                let n = 5i32;
                ed.attr_cursor = ((ed.attr_cursor as i32 + d).rem_euclid(n)) as usize;
            }
        }

        Action::StyleSetColor { is_bg, value } => {
            let dir = state.config.user_dir.clone();
            apply_style_set_color(state, is_bg, value, &dir);
        }

        Action::StyleCommitCustom => {
            if let Some(ed) = &state.overlays.style_editor {
                if crate::style_mru::is_valid_color_token(&ed.custom_buf) {
                    let is_bg = ed.color_target;
                    let value = if ed.custom_buf == "default" { Some("reset".to_string()) } else { Some(ed.custom_buf.clone()) };
                    let dir = state.config.user_dir.clone();
                    apply_style_set_color(state, is_bg, value, &dir);
                    if let Some(ed) = &mut state.overlays.style_editor { ed.custom_buf.clear(); }
                }
            }
        }

        Action::StyleSwatchNav(d) => {
            if let Some(ed) = &mut state.overlays.style_editor {
                let n = crate::style_mru::ANSI_NAMES.len() as i32 + 1; // +1 for default cell
                ed.swatch_cursor = ((ed.swatch_cursor as i32 + d).rem_euclid(n)) as usize;
            }
        }

        Action::StyleSwatchPick => {
            if let Some(ed) = &state.overlays.style_editor {
                let is_bg = ed.color_target;
                let cur = ed.swatch_cursor;
                let value = if cur == crate::style_mru::ANSI_NAMES.len() {
                    Some("reset".to_string())
                } else {
                    crate::style_mru::ANSI_NAMES.get(cur).map(|s| s.to_string())
                };
                let dir = state.config.user_dir.clone();
                apply_style_set_color(state, is_bg, value, &dir);
            }
        }

        Action::StyleCustomChar(c) => {
            if let Some(ed) = &mut state.overlays.style_editor {
                ed.custom_buf.push(c);
            }
        }

        Action::StyleCustomBackspace => {
            if let Some(ed) = &mut state.overlays.style_editor {
                if ed.custom_buf.len() > 1 {
                    ed.custom_buf.pop();
                }
            }
        }

        Action::StyleSave => {
            if let Some(ed) = state.overlays.style_editor.take() {
                let dir = state.config.user_dir.clone();
                let _ = crate::style_mru::save_mru(&dir, &ed.mru);
                let (cs, set, _w) = crate::style::resolve(&ed.doc, &dir);
                state.colors = cs;
                state.symbols = set;
            }
        }

        Action::StyleSaveGame => {
            if state.ifid.is_empty() {
                state.set_status("no game loaded");
            } else if let Some(ed) = state.overlays.style_editor.take() {
                let dir = state.config.user_dir.clone();
                let _ = crate::style_mru::save_mru(&dir, &ed.mru);
                let (cs, set, _w) = crate::style::resolve(&ed.doc, &dir);
                state.colors = cs;
                state.symbols = set;
            }
        }

        Action::StyleReset => {
            if let Some(ed) = &mut state.overlays.style_editor {
                let default_doc = crate::style::parse_style_toml(crate::style::DEFAULT_STYLE_TOML)
                    .expect("DEFAULT_STYLE_TOML is always valid");
                let sel = ed.selectors[ed.active].to_string();
                match default_doc.colors.selectors.get(&sel) {
                    Some(d) => { ed.doc.colors.selectors.insert(sel, d.clone()); }
                    None => { ed.doc.colors.selectors.remove(&sel); }
                }
                let dir = state.config.user_dir.clone();
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderTypeCycle(d) => {
            const STYLES: &[&str] = &["none", "single", "double", "rounded", "thick", "picture-frame"];
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.overlays.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                let cur_name = decl.style.as_deref().unwrap_or("single");
                let cur_idx = STYLES.iter().position(|s| *s == cur_name).unwrap_or(1) as i32;
                let n = STYLES.len() as i32;
                let new_idx = ((cur_idx + d).rem_euclid(n)) as usize;
                decl.style = Some(STYLES[new_idx].to_string());
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderZoneNav(d) => {
            if let Some(ed) = &mut state.overlays.style_editor {
                let n = 8i32;
                ed.border_zone = ((ed.border_zone as i32 + d).rem_euclid(n)) as usize;
            }
        }

        Action::StyleBorderClearZone => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.overlays.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let zone = border_zone_from_index(ed.border_zone);
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                set_zone_glyph(decl, zone, None);
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderToggleHeader => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.overlays.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                decl.header = Some(!decl.header.unwrap_or(false));
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderToggleShadow => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.overlays.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                decl.shadow = Some(!decl.shadow.unwrap_or(false));
                recompute_style_preview(ed, &dir);
            }
        }

        _ => unreachable!("apply_style_action called with a non-style-editor action"),
    }
}

/// Open the live style editor: load the current style doc, resolve a preview
/// ColorScheme, and seed the StyleEditorState on `state.overlays.style_editor`.
///
/// Does not touch `state.colors` — the live theme is untouched until Save.
pub fn open_style_editor(state: &mut AppState) {
    let user_dir = state.config.user_dir.clone();
    let (global, _warnings) = crate::style::load_style(state.config.style.as_deref(), &user_dir);
    // Layer the per-game override (<game_dir>/style.toml) over the global so
    // the editor opens showing the live look. A missing or unparseable per-game
    // file falls back to the global doc.
    let doc = if !state.game_dir.as_os_str().is_empty() {
        let pg_path = crate::styles::per_game_style_path(&state.game_dir);
        match std::fs::read_to_string(&pg_path) {
            Ok(text) => match crate::style::parse_style_toml(&text) {
                Ok(over) => crate::style::merge(&global, &over),
                Err(_) => global,
            },
            Err(_) => global,
        }
    } else {
        global
    };
    let (preview, _set, _w2) = crate::style::resolve(&doc, &user_dir);
    let selectors: Vec<&'static str> =
        crate::style::SELECTOR_GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
    state.overlays.style_editor = Some(crate::state::StyleEditorState {
        doc,
        preview,
        selectors,
        active: 0,
        focus: crate::state::StyleFocus::Board,
        custom_buf: String::new(),
        mru: crate::style_mru::load_mru(&user_dir),
        attr_cursor: 0,
        color_target: false,
        swatch_cursor: 0,
        border_zone: 0,
    });
    state.overlays.dialog_focus = 0;
}

/// Re-resolve `ed.preview` from the current `ed.doc` + `user_dir`.
///
/// Called from edit handlers (Tasks 4-6) whenever the doc changes.
/// Nav doesn't change the doc, so it skips this call.
pub fn recompute_style_preview(ed: &mut crate::state::StyleEditorState, user_dir: &std::path::Path) {
    let (cs, _set, _w) = crate::style::resolve(&ed.doc, user_dir);
    ed.preview = cs;
}

/// Set the fg or bg color for the active selector, push hex to MRU, recompute preview.
///
/// Shared by `StyleSetColor`, `StyleCommitCustom`, and `StyleSwatchPick`.
pub(crate) fn apply_style_set_color(
    state: &mut AppState,
    is_bg: bool,
    value: Option<String>,
    user_dir: &std::path::Path,
) {
    if let Some(ed) = &mut state.overlays.style_editor {
        let sel = ed.selectors[ed.active].to_string();
        let decl = ed.doc.colors.selectors.entry(sel).or_default();
        let slot = if is_bg { &mut decl.bg } else { &mut decl.fg };
        *slot = value.clone();
        if let Some(v) = &value {
            if v.starts_with('#') {
                crate::style_mru::push_mru(&mut ed.mru, v);
            }
        }
        recompute_style_preview(ed, user_dir);
    }
}
