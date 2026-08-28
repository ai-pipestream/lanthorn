//! The theme-aware pane component: one `draw_panel` that owns pane chrome
//! (focus-aware border style + colour, border-derived terminator caps with user
//! overrides, the title/tab strip, optional body fill, and glyphs), composing the
//! theme-agnostic `paneframe` primitives with the resolved [`Theme`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::{
    draw_framed, draw_header_controls, draw_header_plain, draw_top_inset, header_controls_width,
    BorderStyle, HeaderControl, InsetCaps, InsetSegment, PaneGlyphs, PaneSides,
};
use crate::theme::resolve::{Provenance, Theme};

/// Resolve the title-strip terminator/divider caps for a border style, applying
/// user overrides. Start from [`InsetCaps::for_border`] (the style-derived
/// default); for each terminator selector, a user layer that actually set a
/// single glyph (provenance ≠ [`Provenance::Default`]) overrides the derived cap.
pub fn resolve_inset_caps(theme: &Theme, border: BorderStyle) -> InsetCaps {
    let mut caps = InsetCaps::for_border(border);
    for (sel, field) in [
        ("panel.terminator_left", &mut caps.left),
        ("panel.terminator_right", &mut caps.right),
        ("panel.tab_divider", &mut caps.divider),
    ] {
        let r = theme.get(sel);
        if r.provenance != Provenance::Default {
            if let Some(s) = r.glyph.and_then(|g| g.single) {
                *field = s;
            }
        }
    }
    caps
}

/// The title/tab strip drawn on a panel's header row. One segment = a plain
/// title; many = tabs. `base`/`active` are the inactive/active segment styles.
pub struct PanelStrip<'a> {
    pub segments: &'a [InsetSegment<'a>],
    pub base: Style,
    pub active: Style,
}

/// A request to draw a themed panel. The caller resolves focus into
/// `border_selector` (e.g. `"panel.border:active"` vs `"panel.border"`, or
/// `"dialog.border"`); `draw_panel` reads its `.border` (style) and `.style`
/// (colour, unless overridden by `border_color`).
pub struct PanelSpec<'a> {
    pub area: Rect,
    /// The border selector, already focus-resolved by the caller.
    pub border_selector: &'a str,
    /// Override the frame colour (pulse/resize); `None` uses the selector's style.
    pub border_color: Option<Style>,
    /// explicit border style; when None, resolved from `border_selector`.
    pub border_style: Option<crate::render::paneframe::BorderStyle>,
    pub glyphs: &'a PaneGlyphs,
    pub header_on: bool,
    /// The title/tab strip, or `None` for no strip.
    pub strip: Option<PanelStrip<'a>>,
    /// Fill the content area with this style before returning.
    pub body_fill: Option<Style>,
}

/// The result of drawing a panel: the content rect, the header rect (if any), and
/// per-strip-segment hit-rects (empty when there is no strip).
pub struct PanelFrame {
    pub content: Rect,
    pub header: Option<Rect>,
    pub tab_rects: Vec<Rect>,
}

/// Draw a themed panel: frame (style + colour from `border_selector`), the
/// border-derived terminator caps (with user overrides), the optional title/tab
/// strip, and an optional body fill. Returns the content/header rects and the
/// strip's per-segment hit-rects.
pub fn draw_panel(buf: &mut Buffer, spec: &PanelSpec, theme: &Theme) -> PanelFrame {
    draw_panel_with_controls(buf, spec, &[], theme).0
}

/// [`draw_panel`], plus a cluster of clickable toggle controls right-aligned in
/// the panel's top border row (SQ-1123).
///
/// The cluster's columns are taken OUT of the header rect before the title strip
/// is drawn, so a long title is centred in what is left and can never overwrite
/// a control — the alternative, drawing the controls over the finished strip,
/// silently eats the end of the title on a narrow pane. When the header is too
/// narrow to hold the cluster at all, nothing is drawn and the title keeps the
/// whole row.
///
/// Returns the panel frame and one hit-rect per control, in the order given.
pub fn draw_panel_with_controls(
    buf: &mut Buffer,
    spec: &PanelSpec,
    controls: &[HeaderControl],
    theme: &Theme,
) -> (PanelFrame, Vec<Rect>) {
    let r = theme.get(spec.border_selector);
    let border_style = spec.border_style.or(r.border).unwrap_or(BorderStyle::None);
    let color = spec.border_color.unwrap_or(r.style);

    let framed = draw_framed(
        buf,
        spec.area,
        PaneSides::all(border_style),
        spec.glyphs,
        color,
        spec.header_on,
    );

    let caps = resolve_inset_caps(theme, border_style);

    let mut control_rects = Vec::new();
    let mut tab_rects = Vec::new();
    if let Some(hrect) = framed.header {
        // The cluster's columns come out of the strip's FIRST, so an ordinary
        // title is centred in what is left and the two never meet. One blank
        // column is held back as well, so they never abut either.
        let want = if framed.header_bordered && !controls.is_empty() {
            header_controls_width(controls.len())
        } else {
            0
        };
        let mut strip_rect = hrect;
        let draw_controls = want > 0 && hrect.width > want + 1;
        if draw_controls {
            strip_rect.width -= want;
        }
        if let Some(strip) = &spec.strip {
            tab_rects = if framed.header_bordered {
                draw_top_inset(buf, strip_rect, strip.segments, strip.base, strip.active, &caps)
            } else {
                draw_header_plain(buf, strip_rect, strip.segments, strip.base, strip.active)
            };
        }
        // …and the cluster paints LAST, over the reserved columns. A single
        // segment wider than the whole strip is drawn by `render_overflow`,
        // which clips to the buffer rather than to the rect it was given — so a
        // very long title runs straight through the reservation. Painting the
        // controls afterwards means the worst that costs is the tail of a title
        // nobody could read anyway, instead of a control that is invisible and
        // still clickable.
        if draw_controls {
            control_rects = draw_header_controls(buf, hrect, controls, &caps, color);
        }
    }

    if let Some(fill) = spec.body_fill {
        let c = framed.content;
        for y in c.y..c.bottom() {
            for x in c.x..c.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(fill);
                }
            }
        }
    }

    (PanelFrame { content: framed.content, header: framed.header, tab_rects }, control_rects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::resolve::resolve_theme;
    use ratatui::style::{Color, Style};

    /// Build a `Theme` from the default scheme and the given style-toml text.
    fn theme_from(text: &str) -> Theme {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse(text).unwrap();
        resolve_theme(&scheme, &parsed)
    }

    #[test]
    fn draw_panel_double_border_and_caps() {
        let theme = theme_from("[panel]\nborder = { style = \"double\" }\n");
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let segs = [InsetSegment { text: "ZORK I", active: false }];
        let spec = PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip {
                segments: &segs,
                base: Style::default(),
                active: Style::default(),
            }),
            body_fill: None,
        };
        let frame = draw_panel(&mut buf, &spec, &theme);
        // Double top-left corner.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔");
        // Strip left cap is the double terminator ╡ (immediately left of segment).
        let r = frame.tab_rects[0];
        assert_eq!(buf.cell((r.x - 1, 0)).unwrap().symbol(), "╡");
    }

    #[test]
    fn draw_panel_focused_selector_picks_thick() {
        let theme = theme_from("[panel]\n\"border:active\" = { style = \"thick\" }\n");
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let spec = PanelSpec {
            area,
            border_selector: "panel.border:active",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: false,
            strip: None,
            body_fill: None,
        };
        draw_panel(&mut buf, &spec, &theme);
        // Thick top-left corner.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┏");
    }

    #[test]
    fn resolve_inset_caps_default_matches_for_border() {
        let theme = theme_from("");
        let caps = resolve_inset_caps(&theme, BorderStyle::Double);
        let base = InsetCaps::for_border(BorderStyle::Double);
        assert_eq!(caps.left, base.left);
        assert_eq!(caps.right, base.right);
        assert_eq!(caps.divider, base.divider);
    }

    #[test]
    fn resolve_inset_caps_user_override_wins() {
        let theme = theme_from("[panel]\nterminator_left = { glyph = \"X\" }\n");
        let caps = resolve_inset_caps(&theme, BorderStyle::Double);
        assert_eq!(caps.left, "X");
        // The untouched caps keep the derived double defaults.
        assert_eq!(caps.right, "╞");
    }

    #[test]
    fn draw_panel_body_fill_fills_content() {
        let theme = theme_from("");
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        let fill = Style::default().bg(Color::Blue);
        let spec = PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: false,
            strip: None,
            body_fill: Some(fill),
        };
        let frame = draw_panel(&mut buf, &spec, &theme);
        let c = frame.content;
        for y in c.y..c.bottom() {
            for x in c.x..c.right() {
                let cell = buf.cell((x, y)).unwrap();
                assert_eq!(cell.symbol(), " ");
                assert_eq!(cell.style().bg, Some(Color::Blue));
            }
        }
    }

    #[test]
    fn draw_panel_no_strip_returns_empty_tab_rects() {
        let theme = theme_from("");
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        let spec = PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: None,
            body_fill: None,
        };
        let frame = draw_panel(&mut buf, &spec, &theme);
        assert!(frame.tab_rects.is_empty());
        // The frame is still drawn (default single top-left corner).
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┌");
    }
}
