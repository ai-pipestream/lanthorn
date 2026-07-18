//! `[more]` pager (SQ-0404).
//!
//! After a game command whose output overflows the story pane, instead of
//! auto-jumping to the newest line the view stops at the FIRST new screenful and
//! shows a `[more]` prompt; each keypress pages down one screen through the new
//! output until it catches up to the bottom, then the pager exits.
//!
//! The story pane's scroll offset (`AppState.transcript_scroll`) is measured in
//! rows-from-bottom (`0` = newest at bottom). Wrapped-row counts are only known
//! at render time, so engaging the pager is a two-phase arm/activate:
//!   1. **arm** — a qualifying command turn records the transcript's wrapped-row
//!      total *before* its output (from the last rendered frame).
//!   2. **activate** — the next render knows the *after* total and the viewport
//!      height, so [`activation_target`] decides whether to engage and where to
//!      park the scroll offset.

/// Pager runtime state, held on `AppState`.
#[derive(Debug, Default, Clone)]
pub struct Pager {
    /// True while the `[more]` prompt is showing and keypresses page the pane.
    pub active: bool,
    /// Set by a qualifying command turn: the transcript's wrapped-row total
    /// BEFORE this turn's output, awaiting the next render to decide whether to
    /// engage. `None` when not armed.
    pub pending_before_rows: Option<u16>,
}

impl Pager {
    /// Arm for the turn that just appended output: record the pre-turn wrapped-row
    /// count so the next render can measure how much was added.
    pub fn arm(&mut self, before_rows: u16) {
        self.pending_before_rows = Some(before_rows);
    }

    /// Clear any pending arm (e.g. the added output fit in one screen).
    pub fn disarm(&mut self) {
        self.pending_before_rows = None;
    }
}

/// Given the transcript's wrapped-row totals before/after a turn and the story
/// pane's viewport height, decide whether the pager should engage and, if so, the
/// initial rows-from-bottom scroll offset that shows the FIRST new screenful.
///
/// Returns `None` when the new output fits within one screen (no pager needed).
///
/// Derivation: the viewport shows absolute rows `[after - scroll - viewport ..
/// after - scroll]`. To put the first new row (`before`) at the viewport top:
/// `after - scroll - viewport = before` → `scroll = added - viewport`, where
/// `added = after - before`. That offset is always `<= max_scroll`
/// (`after - viewport`), so it never over-scrolls.
pub fn activation_target(before_rows: u16, after_rows: u16, viewport_rows: u16) -> Option<u16> {
    if viewport_rows == 0 {
        return None;
    }
    let added = after_rows.saturating_sub(before_rows);
    if added <= viewport_rows {
        return None; // fits in one screen — leave the view at the bottom
    }
    Some(added - viewport_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pager_when_output_fits_one_screen() {
        // 8 rows added, viewport 10 → fits, no pager.
        assert_eq!(activation_target(20, 28, 10), None);
        // Exactly one screen added is still "fits" (nothing past the fold).
        assert_eq!(activation_target(20, 30, 10), None);
        // Degenerate viewport never engages.
        assert_eq!(activation_target(0, 100, 0), None);
    }

    #[test]
    fn pager_parks_at_first_new_screenful() {
        // 25 rows added, viewport 10 → engage; first screenful sits 15 rows up.
        assert_eq!(activation_target(2, 27, 10), Some(15));
        // The target never exceeds max_scroll (= after - viewport = 17).
        let t = activation_target(2, 27, 10).unwrap();
        assert!(t <= 27u16.saturating_sub(10));
    }

    #[test]
    fn arm_disarm_roundtrip() {
        let mut p = Pager::default();
        assert!(p.pending_before_rows.is_none());
        p.arm(42);
        assert_eq!(p.pending_before_rows, Some(42));
        p.disarm();
        assert!(p.pending_before_rows.is_none());
        assert!(!p.active);
    }
}
