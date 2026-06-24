use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::{BorderStyle, InsetSegment, draw_pane_frame, draw_top_inset};

// ── centered_rect ─────────────────────────────────────────────────────────────

/// Return a rect of size `w`x`h` centered within `area`, clamped to `area`.
pub fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect::new(x, y, w, h)
}

// ── Placement ─────────────────────────────────────────────────────────────────

pub enum Placement {
    Centered { w: u16, h: u16 },
    Positioned(Rect),
}

// ── ButtonId ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    Save,
    Cancel,
    Ok,
    Done,
    Close,
}

// ── DialogButton ──────────────────────────────────────────────────────────────

pub struct DialogButton {
    pub id: ButtonId,
    pub label: &'static str,
}

// ── DialogStyle ───────────────────────────────────────────────────────────────

pub struct DialogStyle {
    pub frame: Style,
    pub box_style: BorderStyle,
    pub title: Style,
    pub button: Style,
    pub button_active: Style,
    pub shadow: Style,
    pub shadow_on: bool,
}

// ── DialogSpec ────────────────────────────────────────────────────────────────

pub struct DialogSpec<'a> {
    pub title: &'a str,
    pub placement: Placement,
    pub buttons: &'a [DialogButton],
    pub show_close: bool,
}

// ── DialogRects ───────────────────────────────────────────────────────────────

pub struct DialogRects {
    pub area: Rect,
    pub content: Rect,
    pub close: Option<Rect>,
    pub buttons: Vec<(ButtonId, Rect)>,
}

// ── draw_dialog ───────────────────────────────────────────────────────────────

pub fn draw_dialog(buf: &mut Buffer, spec: &DialogSpec, st: &DialogStyle) -> DialogRects {
    // (1) Resolve area from placement
    let buf_area = *buf.area();
    let area = match spec.placement {
        Placement::Centered { w, h } => centered_rect(buf_area, w, h),
        Placement::Positioned(r) => r,
    };

    // (2) If shadow_on, paint shadow at +1/+1 offset (bottom+right), clamped to buffer
    if st.shadow_on {
        // Bottom row of shadow: area.y+1 .. area.bottom()+1, col area.right()
        let shadow_right = area.right();
        let shadow_bottom = area.bottom();

        // Right column shadow: rows area.y+1 .. area.bottom(), col area.right()
        if shadow_right < buf_area.right() {
            for row in (area.y + 1)..shadow_bottom.min(buf_area.bottom()) {
                if let Some(cell) = buf.cell_mut((shadow_right, row)) {
                    cell.set_style(st.shadow);
                }
            }
        }

        // Bottom row shadow: cols area.x+1 .. area.right()+1, row area.bottom()
        if shadow_bottom < buf_area.bottom() {
            for col in (area.x + 1)..=(shadow_right.min(buf_area.right().saturating_sub(1))) {
                if let Some(cell) = buf.cell_mut((col, shadow_bottom)) {
                    cell.set_style(st.shadow);
                }
            }
        }
    }

    // (3) Fill area OPAQUE with Style::reset().patch(st.frame)
    let fill_style = Style::reset().patch(st.frame);
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_symbol(" ");
                cell.set_style(fill_style);
            }
        }
    }

    // (4) draw_pane_frame for the border
    let pane = draw_pane_frame(buf, area, st.box_style, st.frame);

    // (5) Overlay the centered title via draw_top_inset
    let title_seg = InsetSegment { text: spec.title, active: false };
    draw_top_inset(buf, pane.top_inset, &[title_seg], st.title, st.title);

    // (6) If show_close, draw ✕ just inside the top-right border
    let close = if spec.show_close && area.width >= 3 && area.height >= 1 {
        // Just inside top-right: col = area.right()-2, row = area.y
        let cx = area.right().saturating_sub(2);
        let cy = area.y;
        if let Some(cell) = buf.cell_mut((cx, cy)) {
            cell.set_symbol("✕").set_style(st.frame);
        }
        Some(Rect::new(cx, cy, 1, 1))
    } else {
        None
    };

    // (7) If buttons non-empty, draw a right-aligned bottom button row
    let mut button_rects: Vec<(ButtonId, Rect)> = Vec::new();
    let content = if !spec.buttons.is_empty() && pane.content.height > 0 {
        // Reserve the last row of content for buttons
        let button_row_y = pane.content.bottom().saturating_sub(1);
        // Each button rendered as "[ Label ]"
        // Lay out right-to-left
        let mut col = pane.content.right();
        for btn in spec.buttons.iter().rev() {
            // "[ Label ]" = 4 + label_len chars
            let label_chars = btn.label.chars().count() as u16;
            let btn_width = 4 + label_chars; // "[ " + label + " ]"
            if col < btn_width || col.saturating_sub(btn_width) < pane.content.x {
                break;
            }
            col = col.saturating_sub(btn_width);
            let bx = col;

            // Draw "[ Label ]"
            let btn_str = format!("[ {} ]", btn.label);
            let mut draw_x = bx;
            for ch in btn_str.chars() {
                if draw_x < pane.content.right() {
                    if let Some(cell) = buf.cell_mut((draw_x, button_row_y)) {
                        let mut tmp = [0u8; 4];
                        cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(st.button);
                    }
                    draw_x += 1;
                }
            }

            button_rects.push((btn.id, Rect::new(bx, button_row_y, btn_width, 1)));
        }
        button_rects.reverse();

        // Content is frame content minus the button row
        if pane.content.height > 1 {
            Rect::new(
                pane.content.x,
                pane.content.y,
                pane.content.width,
                pane.content.height - 1,
            )
        } else {
            Rect::new(pane.content.x, pane.content.y, pane.content.width, 0)
        }
    } else {
        pane.content
    };

    DialogRects { area, content, close, buttons: button_rects }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_opaque_bg_covers_underlying_and_records_rects() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier, Color}};
        let full = Rect::new(0,0,40,12);
        let mut buf = Buffer::empty(full);
        // pre-fill a REVERSED cell where the dialog will sit
        buf.cell_mut((20,6)).unwrap().set_symbol("X").set_style(Style::new().add_modifier(Modifier::REVERSED));
        let st = DialogStyle{ frame: Style::new().bg(Color::Black), box_style: BorderStyle::Single, title: Style::default(), button: Style::default(), button_active: Style::default(), shadow: Style::default(), shadow_on:false };
        let spec = DialogSpec{ title:"Settings", placement: Placement::Centered{w:20,h:8}, buttons: &[DialogButton{id:ButtonId::Save,label:"Save"},DialogButton{id:ButtonId::Cancel,label:"Cancel"}], show_close:true };
        let r = draw_dialog(&mut buf, &spec, &st);
        // opaque: the covered cell no longer REVERSED
        assert!(!buf.cell((20,6)).unwrap().modifier.contains(Modifier::REVERSED));
        assert!(r.close.is_some());
        assert_eq!(r.buttons.len(), 2);
        assert!(r.content.width > 0 && r.content.height > 0);
    }

    #[test]
    fn centered_rect_centers_and_clamps() {
        use ratatui::layout::Rect;
        assert_eq!(centered_rect(Rect::new(0,0,40,12), 20, 8), Rect::new(10,2,20,8));
        let big = centered_rect(Rect::new(0,0,10,4), 20, 8); // clamps to area
        assert!(big.width <= 10 && big.height <= 4);
    }

    #[test]
    fn dialog_shadow_paints_offset_cells_when_on() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style,Color}};
        let mut buf = Buffer::empty(Rect::new(0,0,40,12));
        let st = DialogStyle{ frame: Style::new().bg(Color::Black), box_style: BorderStyle::Single, title:Style::default(), button:Style::default(), button_active:Style::default(), shadow: Style::new().bg(Color::DarkGray), shadow_on:true };
        let spec = DialogSpec{ title:"T", placement: Placement::Centered{w:10,h:5}, buttons:&[], show_close:false };
        let r = draw_dialog(&mut buf, &spec, &st);
        // a cell just below-right of the frame carries the shadow bg
        let sx = r.area.right(); let sy = r.area.bottom();
        if sx < 40 && sy < 12 { assert_eq!(buf.cell((sx, sy)).unwrap().style().bg, Some(Color::DarkGray)); }
    }
}
