//! Overlay panel shown during tidy-animation playback.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::render::draw_str_clipped;
use crate::state::TidyFrame;

const PANEL_W: u16 = 62;
const PANEL_H: u16 = 4;

/// Render the tidy-animation panel into the top-left corner of `area`.
///
/// Returns `Some(DialogRects)` when the panel was drawn (for mouse hit-testing).
pub fn draw_tidy_panel(frame: &TidyFrame, area: Rect, buf: &mut Buffer, dialog_style: &DialogStyle) -> Option<DialogRects> {
    if area.width < PANEL_W || area.height < PANEL_H {
        return None;
    }
    let panel_area = Rect { x: area.x, y: area.y, width: PANEL_W, height: PANEL_H };

    // Render via shared dialog chrome (positioned at the computed rect).
    let spec = DialogSpec {
        title: " Tidy ",
        placement: Placement::Positioned(panel_area),
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
    };
    let dr = draw_dialog(buf, &spec, dialog_style);

    let content = dr.content;

    let desc = truncate_str(&frame.description, content.width as usize);
    if content.height >= 1 {
        draw_str_clipped(buf, content.x, content.y, &desc, dialog_style.frame, content);
    }

    if content.height >= 2 {
        let s = &frame.stats;
        let stats_line = format!(
            "moved:{} overlaps:{} dropped:{} hints:{}",
            s.rooms_moved, s.overlaps_resolved, s.constraints_dropped, s.hints_repaired
        );
        let stats_line = truncate_str(&stats_line, content.width as usize);
        draw_str_clipped(buf, content.x, content.y + 1, &stats_line, dialog_style.frame, content);
    }

    Some(dr)
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}
