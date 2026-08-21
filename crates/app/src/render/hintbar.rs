//! Bottom hint-bar view builder: turns the live keymap + hotkey layout into the
//! context-appropriate " key: label | … " string drawn along the bottom row.
//! Extracted verbatim from `main.rs` (SQ-0306) as a pure move — no behavior
//! change. A pure view builder, so it lives in `render/` alongside its siblings.

use crate::keymap::{Context, HotkeyLayout, KeyMap};

/// Priority-ordered command-string lists for the bottom hint bar.
/// Commands are included only when directly available in the current context.
/// `tidy-map` is intentionally excluded from all lists.
/// Story-pane hints while the debug inspector is open, so Tab has somewhere to
/// go.
pub const GAME_HINTS: &[&str] = &[
    "toggle-focus",
    "save-state",
    "restore-state",
];

/// The ordinary story-pane hints. `toggle-focus` is deliberately absent: with
/// the inspector closed the map is not a focus stop (SQ-0599), so Tab does
/// nothing and advertising it would promise a mode that no longer exists.
pub const GAME_HINTS_NO_INSPECTOR: &[&str] = &[
    "save-state",
    "restore-state",
];

pub const ANIM_HINTS: &[&str] = &[
    "anim-step forward",
    "anim-play",
    "anim-exit",
    "pan-map -1 0",
    "zoom-map in",
];

/// Debug-inspector hint bar: `(key, label)` pairs. Unlike the other hint
/// lists above, these bindings are internal to the debug panel (handled by
/// `DebugPanelState::handle_key`), not registered commands in the global
/// keymap — so they're rendered via [`literal_hint_bar`] instead of
/// [`hint_bar`]'s keymap-resolution path.
pub const DEBUG_HINTS: &[(&str, &str)] = &[
    ("Tab", "window"),
    ("\u{2190}\u{2192}", "section"),
    ("\u{2191}\u{2193}", "scroll"),
    ("hl", "pan"),
    ("g", "PC"),
    ("r", "raw"),
    ("Esc", "back"),
];

/// Build a hint bar string from a fixed literal `(key, label)` list. Mirrors
/// [`hint_bar`]'s join/truncate behavior, without the keymap-resolution gates
/// (there is no keymap entry to resolve — the bindings are hard-coded).
pub fn literal_hint_bar(hints: &[(&str, &str)], width: usize) -> String {
    let joined = hints.iter().map(|(k, l)| format!("{k}: {l}")).collect::<Vec<_>>().join(" | ");
    if width == 0 {
        return String::new();
    }
    let char_count = joined.chars().count();
    if char_count <= width {
        joined
    } else {
        let truncate_at = width.saturating_sub(1);
        let byte_pos = joined
            .char_indices()
            .nth(truncate_at)
            .map(|(i, _)| i)
            .unwrap_or(joined.len());
        format!("{}…", &joined[..byte_pos])
    }
}

/// Short hint-bar label for a command-string: "zoom-map in" -> "zoom map in".
fn hint_label(cmd_str: &str) -> String {
    cmd_str.replace('-', " ")
}

/// Build the hint bar string for the given context from the live keymap and layout.
///
/// For each command-string in `priority`, an entry is included only if all three hold:
/// 1. `layout.is_direct_name(cmd)` — the command is directly available, not dialog-only.
/// 2. `keymap.primary_key(name)` returns a KeySpec `k`.
/// 3. `keymap.lookup(&k, ctx) == Some(cmd)` — pressing `k` in `ctx` resolves to `cmd`.
///
/// Each surviving entry renders as "{k.label()}: {label}"; entries join with " | ".
/// If the joined string exceeds `width` characters, it is truncated and "…" appended.
pub fn hint_bar(
    keymap: &KeyMap,
    layout: &HotkeyLayout,
    ctx: Context,
    priority: &[&str],
    width: usize,
) -> String {
    let entries: Vec<String> = priority
        .iter()
        .filter_map(|&cmd| {
            // Gate 1: command must be directly available (not dialog-only).
            if !layout.is_direct_name(cmd) {
                return None;
            }
            // Gate 2: command must have a primary key binding.
            let name = cmd.split_whitespace().next().unwrap_or("");
            let k = keymap.primary_key(name)?;
            // Gate 3: pressing that key in this context must resolve back to this command.
            if keymap.lookup(&k, ctx) != Some(cmd) {
                return None;
            }
            let label = hint_label(cmd);
            Some(format!("{}: {}", k.label(), label))
        })
        .collect();

    let joined = entries.join(" | ");

    // Truncate to width (char-count aware), appending "…" if needed.
    if width == 0 {
        return String::new();
    }
    let char_count = joined.chars().count();
    if char_count <= width {
        joined
    } else {
        // Find the byte offset after (width - 1) chars to leave room for "…".
        let truncate_at = width.saturating_sub(1);
        let byte_pos = joined
            .char_indices()
            .nth(truncate_at)
            .map(|(i, _)| i)
            .unwrap_or(joined.len());
        format!("{}…", &joined[..byte_pos])
    }
}

#[cfg(test)]
mod tests {
    use super::{hint_bar, literal_hint_bar, ANIM_HINTS, DEBUG_HINTS, GAME_HINTS};
    use crate::keymap::{Context, HotkeyLayout, KeyMap};

    #[test]
    fn literal_hint_bar_joins_debug_hints() {
        let line = literal_hint_bar(DEBUG_HINTS, 200);
        assert_eq!(line, "Tab: window | \u{2190}\u{2192}: section | \u{2191}\u{2193}: scroll | hl: pan | g: PC | r: raw | Esc: back");
    }

    #[test]
    fn literal_hint_bar_truncates_at_width() {
        let line = literal_hint_bar(DEBUG_HINTS, 10);
        assert!(line.chars().count() <= 10);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn hint_line_anim_contains_zoom_with_plus_key() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        // With default keymap: zoom-map in primary key is '+'; short label is "zoom map in".
        assert!(line.contains("+: zoom"), "expected '+: zoom' in '{line}'");
    }

    #[test]
    fn hint_bar_excludes_dialog_only_commands() {
        // Regression (#11): gallery/inspector/layout moved to the leader dialog
        // after the leader-key change; the hint bar must NOT advertise their dead
        // direct keys. The is_direct filter excludes them (they are dialog-only).
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        assert!(!line.contains("gallery"), "must not advertise gallery (dialog-only): {line}");
        assert!(!line.contains("inspector"), "must not advertise inspector (dialog-only): {line}");
        assert!(!line.contains("hide the map"), "must not advertise toggle-map (dialog-only): {line}");
        // The working direct keys ARE present.
        assert!(line.contains("+: zoom"), "zoom present: {line}");
    }

    /// SQ-0599: the story hint bar only offers Tab when the inspector gives it
    /// somewhere to go. Advertising a focus toggle that does nothing is exactly
    /// the invisible-mode confusion this change removed.
    #[test]
    fn the_story_hint_bar_offers_tab_only_with_the_inspector_open() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let plain = hint_bar(&km, &layout, Context::Global, super::GAME_HINTS_NO_INSPECTOR, 200);
        assert!(!plain.contains("toggle focus"), "no focus toggle without the inspector: {plain}");
        assert!(plain.contains("save state"), "the real global keys are still shown: {plain}");
        let with_inspector = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        assert!(with_inspector.contains("Tab: toggle focus"), "{with_inspector}");
    }

    #[test]
    fn hint_line_game_contains_save_state() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        // Ctrl+S → save-state; short label is "save state".
        assert!(line.contains("Ctrl+S: save state"), "expected 'Ctrl+S: save state' in '{line}'");
        // toggle-map (formerly cycle-layout) was trimmed out of the always-active
        // set (SQ-0202); it's leader-only now and must not appear in the Game hint bar.
        assert!(!line.contains("hide the map"), "Game hint bar must not advertise toggle-map: '{line}'");
    }

    #[test]
    fn leader_hint_advertises_ctrl_p_menu() {
        // The bottom-bar default branch prepends "{prefix.label()}: menu" ahead
        // of the hint_bar output (SQ-0202). Pin the exact construction here since
        // the help-row assembly itself lives inline in the render loop. Prefix
        // moved from Ctrl+K to Ctrl+P (SQ-0447), freeing Ctrl+K for the story
        // prompt's readline delete-to-end shortcut.
        let layout = HotkeyLayout::default();
        let leader_hint = format!("{}: menu", layout.prefix.label());
        assert_eq!(leader_hint, "Ctrl+P: menu");
    }

    #[test]
    fn hint_bar_never_contains_tidy() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let anim_line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        let game_line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        assert!(
            !anim_line.to_lowercase().contains("tidy") && !anim_line.to_lowercase().contains("retidy"),
            "anim hint bar must not contain tidy/retidy; got: '{anim_line}'"
        );
        assert!(
            !game_line.to_lowercase().contains("tidy") && !game_line.to_lowercase().contains("retidy"),
            "game hint bar must not contain tidy/retidy; got: '{game_line}'"
        );
    }

    #[test]
    fn hint_bar_no_dead_keys_all_entries_resolve_back() {
        // Every entry shown must pass the round-trip check: lookup(primary_key(cmd), ctx) == Some(cmd).
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        for (ctx, hints) in [
            (Context::Global, GAME_HINTS),
            (Context::Global, super::GAME_HINTS_NO_INSPECTOR),
            (Context::Anim, ANIM_HINTS),
        ] {
            for &cmd in hints {
                if !layout.is_direct_name(cmd) {
                    continue;
                }
                let name = cmd.split_whitespace().next().unwrap_or("");
                if let Some(k) = km.primary_key(name) {
                    let resolved = km.lookup(&k, ctx);
                    if resolved == Some(cmd) {
                        // This entry would be shown — verify label format.
                        let entry = format!("{}: {}", k.label(), super::hint_label(cmd));
                        let bar = hint_bar(&km, &layout, ctx, hints, 200);
                        assert!(
                            bar.contains(&entry),
                            "bar for {ctx:?} should contain '{entry}'; got: '{bar}'"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hint_bar_drops_non_direct_command() {
        use std::collections::HashSet;
        // Build a layout where zoom-map in is NOT direct (dialog-only), but toggle-focus IS.
        let mut direct: HashSet<String> = HashSet::new();
        direct.insert("toggle-focus".into());
        direct.insert("anim-play".into());
        // zoom-map in intentionally NOT in direct set.
        let layout = HotkeyLayout {
            prefix: "ctrl+k".parse().unwrap(),
            direct,
            groups: vec![],
        };
        let km = KeyMap::default();
        let bar = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        assert!(
            !bar.contains("zoom"),
            "zoom-map in should be absent when not direct; got: '{bar}'"
        );
        // anim-play IS direct, so it should appear.
        assert!(
            bar.contains("anim play"),
            "anim-play should still appear when direct; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_truncates_at_width() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        // Use a very narrow width (10 chars) — the full bar is much longer.
        let bar = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 10);
        let char_count = bar.chars().count();
        assert!(
            char_count <= 10,
            "bar must not exceed width=10; got {char_count} chars: '{bar}'"
        );
        assert!(
            bar.ends_with('…'),
            "truncated bar must end with ellipsis; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_no_truncation_when_wide_enough() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        // Use a very generous width — no truncation expected.
        let bar = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 1000);
        assert!(
            !bar.ends_with('…'),
            "bar should not be truncated at width=1000; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_shows_short_registry_labels() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        // zoom-map in is direct and bound to '+' in Anim; its short label is "zoom map in".
        assert!(line.contains("zoom map in"), "hint bar should show the short label 'zoom map in', got: {line}");
        // The full description sentence must NOT appear.
        assert!(!line.contains("zoom the map in/out"), "hint bar must not show the long description");
    }
}
