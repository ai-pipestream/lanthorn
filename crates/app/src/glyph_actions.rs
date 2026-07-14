//! Glyph-picker modal action handling, peeled out of input.rs (SQ-0306).
//! `apply_action` delegates its glyph-picker arm cluster here; the arm bodies are
//! unchanged from their original in-place form.

use crate::input::{picker_block_range, set_zone_glyph, Action, GLYPH_BLOCKS};
use crate::state::AppState;

/// Dispatch a glyph-picker `Action`. Called by `apply_action` for its glyph-picker
/// arm cluster; only ever invoked with those variants.
pub(crate) fn apply_glyph_action(action: Action, state: &mut AppState) {
    match action {
        Action::StyleOpenGlyphPicker(zone) => {
            if let Some(ed) = &state.style_editor {
                let target_selector = ed.selectors[ed.active].to_string();
                // Picture-frame is a composite border; per-zone glyph overrides don't apply.
                let is_picture_frame = ed.doc.colors.selectors.get(&target_selector)
                    .and_then(|d| d.style.as_deref())
                    .unwrap_or("single") == "picture-frame";
                if !is_picture_frame {
                    let user_dir = state.config.user_dir.clone();
                    let mru = crate::style_mru::load_glyph_mru(&user_dir);
                    state.glyph_picker = Some(crate::state::GlyphPickerState {
                        target_selector,
                        target_zone: zone,
                        block: 0,
                        custom_start: None,
                        custom_focus: false,
                        custom_buf: String::new(),
                        cursor: 0,
                        pending: None,
                        mru,
                    });
                }
                // picture-frame: leave state.glyph_picker as None (no-op).
            }
        }

        Action::GlyphPickerNav(delta) => {
            if let Some(picker) = &mut state.glyph_picker {
                let (lo, hi) = picker_block_range(picker);
                let count = (hi - lo + 1) as usize;
                if count > 0 {
                    picker.cursor =
                        ((picker.cursor as i32 + delta).rem_euclid(count as i32)) as usize;
                }
            }
        }

        Action::GlyphPickerBlock(delta) => {
            if let Some(picker) = &mut state.glyph_picker {
                picker.custom_start = None; // return to curated blocks
                picker.custom_focus = false;
                picker.custom_buf.clear();
                let n = GLYPH_BLOCKS.len() as i32;
                picker.block = ((picker.block as i32 + delta).rem_euclid(n)) as usize;
                picker.cursor = 0;
                picker.pending = None;
            }
        }

        Action::GlyphPickerChar(c) => {
            if let Some(picker) = &mut state.glyph_picker {
                if !picker.custom_focus {
                    picker.pending = Some(c.to_string());
                }
            }
        }

        Action::GlyphPickerPick => {
            // Gather what we need before splitting borrows.
            let resolve_info = state.glyph_picker.as_ref().and_then(|picker| {
                let glyph = if let Some(s) = &picker.pending {
                    if crate::style_mru::is_valid_glyph(s) { Some(s.clone()) } else { None }
                } else {
                    picker_glyph_at_cursor(picker)
                };
                glyph.map(|g| (picker.target_selector.clone(), picker.target_zone, g))
            });

            if let Some((sel, zone, glyph)) = resolve_info {
                let user_dir = state.config.user_dir.clone();

                // Write glyph into the style doc.
                if let Some(ed) = &mut state.style_editor {
                    let decl = ed.doc.colors.selectors.entry(sel).or_default();
                    set_zone_glyph(decl, zone, Some(glyph.clone()));
                }

                // Push to glyph MRU and save.
                let saved_mru = if let Some(picker) = &mut state.glyph_picker {
                    crate::style_mru::push_glyph_mru(&mut picker.mru, &glyph);
                    picker.mru.clone()
                } else {
                    Vec::new()
                };
                let _ = crate::style_mru::save_glyph_mru(&user_dir, &saved_mru);

                // Close the picker.
                state.glyph_picker = None;

                // Recompute the preview.
                if let Some(ed) = &mut state.style_editor {
                    crate::style_actions::recompute_style_preview(ed, &user_dir);
                }
            }
            // If glyph was invalid / none, leave picker open.
        }

        Action::GlyphPickerClear => {
            let pick_info = state.glyph_picker.as_ref()
                .map(|p| (p.target_selector.clone(), p.target_zone));
            if let Some((sel, zone)) = pick_info {
                let user_dir = state.config.user_dir.clone();
                if let Some(ed) = &mut state.style_editor {
                    let decl = ed.doc.colors.selectors.entry(sel).or_default();
                    set_zone_glyph(decl, zone, None);
                    crate::style_actions::recompute_style_preview(ed, &user_dir);
                }
            }
            state.glyph_picker = None;
        }

        Action::GlyphPickerCancel => {
            state.glyph_picker = None;
        }

        Action::GlyphPickerCustomFocus => {
            if let Some(picker) = &mut state.glyph_picker {
                picker.custom_focus = true;
                picker.pending = None;
            }
        }

        Action::GlyphPickerCustomChar(c) => {
            if let Some(picker) = &mut state.glyph_picker {
                if c.is_ascii_hexdigit() && picker.custom_buf.len() < 6 {
                    picker.custom_buf.push(c.to_ascii_uppercase());
                    if let Ok(cp) = u32::from_str_radix(&picker.custom_buf, 16) {
                        picker.custom_start = Some(cp);
                        picker.cursor = 0;
                    }
                }
            }
        }

        Action::GlyphPickerCustomBackspace => {
            if let Some(picker) = &mut state.glyph_picker {
                picker.custom_buf.pop();
                picker.custom_start = if picker.custom_buf.is_empty() {
                    None
                } else {
                    u32::from_str_radix(&picker.custom_buf, 16).ok()
                };
            }
        }

        _ => unreachable!("apply_glyph_action called with a non-glyph-picker action"),
    }
}

/// Resolve the glyph at the picker's current `cursor` position, if it is single-width.
/// Returns `None` for empty ranges or non-single-width codepoints.
pub(crate) fn picker_glyph_at_cursor(picker: &crate::state::GlyphPickerState) -> Option<String> {
    let (lo, hi) = picker_block_range(picker);
    // Collect single-width glyphs in order.
    let mut idx = 0usize;
    for cp in lo..=hi {
        if let Some(c) = char::from_u32(cp) {
            let s = c.to_string();
            if crate::style_mru::is_valid_glyph(&s) {
                if idx == picker.cursor {
                    return Some(s);
                }
                idx += 1;
            }
        }
    }
    None
}
