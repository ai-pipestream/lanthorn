use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::{BorderStyle, InsetSegment, PaneGlyphs, draw_pane_frame, draw_top_inset};

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

// ── DialogPlacement ─────────────────────────────────────────────────────────────

/// Where a centered modal is anchored within the screen. `Center` (the default)
/// reproduces today's behavior exactly; the edges/corners anchor the modal to the
/// matching side(s) with a configurable margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogPlacement {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl DialogPlacement {
    /// Parse a placement token. Unknown tokens (and `center`) map to `Center`.
    pub fn from_token(s: &str) -> DialogPlacement {
        match s.trim().to_lowercase().as_str() {
            "top" => DialogPlacement::Top,
            "bottom" => DialogPlacement::Bottom,
            "left" => DialogPlacement::Left,
            "right" => DialogPlacement::Right,
            "top-left" => DialogPlacement::TopLeft,
            "top-right" => DialogPlacement::TopRight,
            "bottom-left" => DialogPlacement::BottomLeft,
            "bottom-right" => DialogPlacement::BottomRight,
            _ => DialogPlacement::Center,
        }
    }

    /// The canonical token for this placement (inverse of `from_token`).
    pub fn as_token(self) -> &'static str {
        match self {
            DialogPlacement::Center => "center",
            DialogPlacement::Top => "top",
            DialogPlacement::Bottom => "bottom",
            DialogPlacement::Left => "left",
            DialogPlacement::Right => "right",
            DialogPlacement::TopLeft => "top-left",
            DialogPlacement::TopRight => "top-right",
            DialogPlacement::BottomLeft => "bottom-left",
            DialogPlacement::BottomRight => "bottom-right",
        }
    }
}

// ── ButtonId ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    Save,
    SaveGame,
    Cancel,
    Ok,
    Done,
    Close,
    Reset,
    Resume,
    NewGame,
    Archive,
    Global,
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
    pub glyphs: PaneGlyphs,
    pub title: Style,
    pub button: Style,
    pub button_active: Style,
    pub shadow: Style,
    pub shadow_on: bool,
}

impl DialogStyle {
    /// Build the dialog chrome from a `ColorScheme`. This is the single seam that
    /// every modal uses, so cross-cutting dialog concerns (placement, later
    /// animation) live in one place instead of being duplicated per modal.
    pub fn from_colors(cs: &crate::colors::ColorScheme) -> DialogStyle {
        DialogStyle {
            frame: cs.dialog,
            box_style: cs.dialog_box_style,
            glyphs: cs.dialog_glyphs.clone(),
            title: cs.dialog_title,
            button: cs.dialog_button,
            button_active: cs.dialog_button_active,
            shadow: cs.dialog_shadow,
            shadow_on: cs.dialog_shadow_on,
        }
    }
}

// ── DialogSpec ────────────────────────────────────────────────────────────────

pub struct DialogSpec<'a> {
    pub title: &'a str,
    pub placement: Placement,
    pub buttons: &'a [DialogButton],
    pub show_close: bool,
    /// The confirm button: rendered underlined; Enter triggers it by default.
    pub default: Option<ButtonId>,
    /// Index into `buttons` to highlight with `button_active` (Tab focus).
    pub focus: Option<usize>,
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
    // Coerce BorderStyle::None to Single so every modal always has a visible border.
    let box_style = match st.box_style {
        BorderStyle::None => BorderStyle::Single,
        other => other,
    };

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
    let pane = draw_pane_frame(buf, area, box_style, &st.glyphs, st.frame);

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
        let n = spec.buttons.len();
        for (rev_i, btn) in spec.buttons.iter().rev().enumerate() {
            let orig_i = n - 1 - rev_i;
            // "[ Label ]" = 4 + label_len chars
            let label_chars = btn.label.chars().count() as u16;
            let btn_width = 4 + label_chars; // "[ " + label + " ]"
            if col < btn_width || col.saturating_sub(btn_width) < pane.content.x {
                break;
            }
            col = col.saturating_sub(btn_width);
            let bx = col;

            // Focused button uses button_active; default button is underlined.
            let mut style = if spec.focus == Some(orig_i) { st.button_active } else { st.button };
            if spec.default == Some(btn.id) {
                style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
            }

            let btn_str = format!("[ {} ]", btn.label);
            let mut draw_x = bx;
            for ch in btn_str.chars() {
                if draw_x < pane.content.right() {
                    if let Some(cell) = buf.cell_mut((draw_x, button_row_y)) {
                        let mut tmp = [0u8; 4];
                        cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
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
    use super::super::paneframe::PaneGlyphs;

    #[test]
    fn dialog_opaque_bg_covers_underlying_and_records_rects() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier, Color}};
        let full = Rect::new(0,0,40,12);
        let mut buf = Buffer::empty(full);
        // pre-fill a REVERSED cell where the dialog will sit
        buf.cell_mut((20,6)).unwrap().set_symbol("X").set_style(Style::new().add_modifier(Modifier::REVERSED));
        let st = DialogStyle{ frame: Style::new().bg(Color::Black), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(), title: Style::default(), button: Style::default(), button_active: Style::default(), shadow: Style::default(), shadow_on:false };
        let spec = DialogSpec{ title:"Settings", placement: Placement::Centered{w:20,h:8}, buttons: &[DialogButton{id:ButtonId::Save,label:"Save"},DialogButton{id:ButtonId::Cancel,label:"Cancel"}], show_close:true, default: None, focus: None };
        let r = draw_dialog(&mut buf, &spec, &st);
        // opaque: the covered cell no longer REVERSED
        assert!(!buf.cell((20,6)).unwrap().modifier.contains(Modifier::REVERSED));
        assert!(r.close.is_some());
        assert_eq!(r.buttons.len(), 2);
        assert!(r.content.width > 0 && r.content.height > 0);
    }

    #[test]
    fn dialog_placement_token_round_trips_and_defaults() {
        assert_eq!(DialogPlacement::from_token("center"), DialogPlacement::Center);
        assert_eq!(DialogPlacement::from_token("top"), DialogPlacement::Top);
        assert_eq!(DialogPlacement::from_token("bottom"), DialogPlacement::Bottom);
        assert_eq!(DialogPlacement::from_token("left"), DialogPlacement::Left);
        assert_eq!(DialogPlacement::from_token("right"), DialogPlacement::Right);
        assert_eq!(DialogPlacement::from_token("top-left"), DialogPlacement::TopLeft);
        assert_eq!(DialogPlacement::from_token("top-right"), DialogPlacement::TopRight);
        assert_eq!(DialogPlacement::from_token("bottom-left"), DialogPlacement::BottomLeft);
        assert_eq!(DialogPlacement::from_token("bottom-right"), DialogPlacement::BottomRight);
        // Unknown and case/whitespace handling.
        assert_eq!(DialogPlacement::from_token("nonsense"), DialogPlacement::Center);
        assert_eq!(DialogPlacement::from_token("  TOP-LEFT  "), DialogPlacement::TopLeft);
        // Default is Center.
        assert_eq!(DialogPlacement::default(), DialogPlacement::Center);
        // as_token is the inverse for every variant.
        for p in [
            DialogPlacement::Center, DialogPlacement::Top, DialogPlacement::Bottom,
            DialogPlacement::Left, DialogPlacement::Right, DialogPlacement::TopLeft,
            DialogPlacement::TopRight, DialogPlacement::BottomLeft, DialogPlacement::BottomRight,
        ] {
            assert_eq!(DialogPlacement::from_token(p.as_token()), p);
        }
    }

    #[test]
    fn from_colors_matches_inline_build() {
        // Guard: DialogStyle::from_colors must reproduce the previous per-modal
        // inline `DialogStyle { frame: cs.dialog, ... }` build byte-for-byte.
        let cs = crate::colors::ColorScheme::terminal_default();
        let ds = DialogStyle::from_colors(&cs);
        assert_eq!(ds.frame, cs.dialog);
        assert_eq!(ds.box_style, cs.dialog_box_style);
        assert_eq!(ds.glyphs, cs.dialog_glyphs);
        assert_eq!(ds.title, cs.dialog_title);
        assert_eq!(ds.button, cs.dialog_button);
        assert_eq!(ds.button_active, cs.dialog_button_active);
        assert_eq!(ds.shadow, cs.dialog_shadow);
        assert_eq!(ds.shadow_on, cs.dialog_shadow_on);
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
        let st = DialogStyle{ frame: Style::new().bg(Color::Black), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(), title:Style::default(), button:Style::default(), button_active:Style::default(), shadow: Style::new().bg(Color::DarkGray), shadow_on:true };
        let spec = DialogSpec{ title:"T", placement: Placement::Centered{w:10,h:5}, buttons:&[], show_close:false, default: None, focus: None };
        let r = draw_dialog(&mut buf, &spec, &st);
        // a cell just below-right of the frame carries the shadow bg
        let sx = r.area.right(); let sy = r.area.bottom();
        if sx < 40 && sy < 12 { assert_eq!(buf.cell((sx, sy)).unwrap().style().bg, Some(Color::DarkGray)); }
    }

    #[test]
    fn dialog_underlines_default_and_highlights_focus() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier}};
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            frame: Style::default(),
            box_style: BorderStyle::Single,
            glyphs: PaneGlyphs::default(),
            title: Style::default(),
            button: Style::default(),
            button_active: Style::default().add_modifier(Modifier::REVERSED),
            shadow: Style::default(),
            shadow_on: false,
        };
        let spec = DialogSpec {
            title: "T",
            placement: Placement::Centered { w: 30, h: 6 },
            buttons: &[
                DialogButton { id: ButtonId::Save, label: "Save" },
                DialogButton { id: ButtonId::Cancel, label: "Cancel" },
            ],
            show_close: true,
            default: Some(ButtonId::Save),
            focus: Some(1),
        };
        let rects = draw_dialog(&mut buf, &spec, &st);
        // The Save (default) button label cells carry UNDERLINED.
        let (_, save_rect) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).unwrap();
        let save_cell = buf.cell((save_rect.x + 2, save_rect.y)).unwrap(); // inside "[ "
        assert!(save_cell.style().add_modifier.contains(Modifier::UNDERLINED),
            "default button must be underlined");
        // The Cancel (focused idx 1) button cells carry REVERSED (button_active).
        let (_, cancel_rect) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).unwrap();
        let cancel_cell = buf.cell((cancel_rect.x + 2, cancel_rect.y)).unwrap();
        assert!(cancel_cell.style().add_modifier.contains(Modifier::REVERSED),
            "focused button must use button_active");
    }
}
