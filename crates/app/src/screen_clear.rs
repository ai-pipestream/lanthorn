//! Screen-clear bookkeeping shared by the story pane and the hint panel.
//!
//! A game's `erase_window` (ZMSD 8.7.3) does not delete scrollback here: it pins
//! the post-clear output to the TOP of a fresh screen (the renderer's top-anchor,
//! `render::transcript::anchored_window_bounds`), with everything older reachable
//! by scrolling up. A menu that redraws itself on every keypress would pile up in
//! that scrollback, so a clear whose new content looks like the previous screen
//! redrawing COLLAPSES it first (SQ-0407, SQ-1654).
//!
//! `AppState` (story pane) and `HintSession` (hint panel) each own a transcript and
//! a `clear_anchor` / `top_anchor` pair; this module owns the two decisions both
//! make, so the panes cannot drift apart (SQ-1789). The caller does the truncation,
//! since each keeps its own parallel sidecar vectors.

/// Whether `new_content` looks like the screen in `old_lines` redrawing itself
/// (a menu reprint) rather than a different screen taking over.
pub(crate) fn is_screen_reprint(old_lines: &[String], new_content: &str) -> bool {
    let old_nonblank: Vec<&str> = old_lines.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let new_nonblank: Vec<&str> = new_content.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if old_nonblank.is_empty() || new_nonblank.is_empty() {
        return false;
    }
    let matches = new_nonblank.iter().filter(|l| old_nonblank.contains(l)).count();
    (matches as f64 / new_nonblank.len() as f64) >= 0.5
}

/// Where to truncate `transcript` before a clear whose new output is
/// `new_content`: back to the previous clear boundary when this is a reprint of
/// the screen since it, else `None` (keep it all as scrollback).
pub(crate) fn collapse_target(clear_anchor: Option<usize>, transcript: &[String], new_content: &str) -> Option<usize> {
    let anchor = clear_anchor?;
    is_screen_reprint(transcript.get(anchor..)?, new_content).then_some(anchor)
}

/// The value both `clear_anchor` and `top_anchor` take after a clear that leaves
/// the transcript `len` lines long (they move in lockstep, SQ-1660).
pub(crate) fn pin_after_clear(len: usize) -> Option<usize> {
    Some(len)
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn collapse_only_for_a_reprint_of_the_screen_since_the_anchor() {
        let t = lines(&["intro", "menu a", "menu b"]);
        assert_eq!(collapse_target(None, &t, "menu a\nmenu b"), None, "no anchor yet");
        assert_eq!(collapse_target(Some(1), &t, "menu a\nmenu b"), Some(1));
        assert_eq!(collapse_target(Some(1), &t, "something else"), None);
        assert_eq!(collapse_target(Some(9), &t, "menu a"), None, "stale anchor past the end");
    }
}
