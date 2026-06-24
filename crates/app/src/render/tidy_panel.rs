//! Overlay panel shown during tidy-animation playback.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Widget};

use crate::state::TidyFrame;

const PANEL_W: u16 = 62;
const PANEL_H: u16 = 4;

pub fn draw_tidy_panel(frame: &TidyFrame, area: Rect, buf: &mut Buffer) {
    if area.width < PANEL_W || area.height < PANEL_H {
        return;
    }
    let panel_area = Rect { x: area.x, y: area.y, width: PANEL_W, height: PANEL_H };
    let bg_style = Style::reset().bg(Color::DarkGray);

    for y in panel_area.y..panel_area.bottom() {
        for x in panel_area.x..panel_area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(bg_style);
                cell.set_symbol(" ");
            }
        }
    }

    let block = Block::default().borders(Borders::ALL).border_style(bg_style);
    let inner = block.inner(panel_area);
    block.render(panel_area, buf);

    let desc = truncate_str(&frame.description, inner.width as usize);
    if inner.height >= 1 {
        write_str(buf, inner.x, inner.y, &desc, bg_style);
    }

    if inner.height >= 2 {
        let s = &frame.stats;
        let stats_line = format!(
            "moved:{} overlaps:{} dropped:{} hints:{}",
            s.rooms_moved, s.overlaps_resolved, s.constraints_dropped, s.hints_repaired
        );
        let stats_line = truncate_str(&stats_line, inner.width as usize);
        write_str(buf, inner.x, inner.y + 1, &stats_line, bg_style);
    }
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

fn write_str(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style) {
    for (i, ch) in s.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((x + i as u16, y)) {
            cell.set_char(ch);
            cell.set_style(style);
        }
    }
}
