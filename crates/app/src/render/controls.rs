//! The pane border's clickable toggle controls (SQ-1123).
//!
//! Guidance, the verb panel and the two v6 render switches were reachable only
//! by slash command, key or the settings screen — nothing on screen said they
//! existed, let alone whether they were on. A player who turned guidance on and
//! saw nothing had every reason to conclude it was broken. These are the answer:
//! a short cluster of icons at the right-hand end of the story pane's top
//! border, each one saying what state it is in and switching that state when
//! clicked.
//!
//! Three rules shape everything here.
//!
//! **A click runs the command.** Each control names an existing entry in
//! `slash::COMMANDS` and the event loop puts that command string through the
//! ordinary slash pipeline, so clicking is byte-for-byte what typing it does —
//! including whatever the command persists. There is no second implementation of
//! any toggle beside the one the registry already owns.
//!
//! **The glyph carries the state, not just the colour.** The panel toggles are
//! arrows pointing the way the panel would move (the map lives right of the
//! story pane, the verb panel below it), the Guiding Light is filled when lit
//! and hollow when not, and the two v6 controls draw a distinct glyph per mode.
//! Colour then reinforces it — `panel.control:lit` is the `alert` role, the same
//! yellow slot `transcript_assist` lights up in, so the light and its mark are
//! one colour.
//!
//! **The v6 pair does not exist off v6.** They are absent from the cluster
//! entirely rather than drawn disabled, so the border of a Zork I never shows a
//! switch that would do nothing.

use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::HeaderControl;
use crate::config::V6RenderMode;
use crate::state::{AppState, Layout};

/// One border toggle, identified by what it switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderControl {
    /// Show / hide the map pane.
    Map,
    /// Lanthorn's Guiding Light.
    Guidance,
    /// The verb panel (the command band).
    VerbPanel,
    /// The v6 render mode — a three-way cycle, not a toggle. v6 only.
    V6Render,
    /// The v6 pixel lock. v6 only.
    V6PixelLock,
}

impl BorderControl {
    /// The `slash::COMMANDS` entry a click runs, bare (every one of them toggles
    /// or cycles when given no argument).
    pub fn command(self) -> &'static str {
        match self {
            BorderControl::Map => "toggle-map",
            BorderControl::Guidance => "set-guidance",
            BorderControl::VerbPanel => "open-command-band",
            BorderControl::V6Render => "set-v6-render",
            BorderControl::V6PixelLock => "set-v6-pixel-lock",
        }
    }
}

/// One control resolved against the live state: which toggle it is, the glyph
/// for the state it is in, the style that state resolves to, and the hover hint.
pub struct ControlView {
    pub id: BorderControl,
    pub glyph: char,
    pub style: Style,
    /// The floating hint's lines: what this is and what a click would do, then
    /// the command or key that does the same thing from the keyboard.
    pub hint: Vec<String>,
}

impl ControlView {
    /// The paint-only half, for `panel::draw_panel_with_controls`.
    pub fn as_header_control(&self) -> HeaderControl {
        HeaderControl { glyph: self.glyph, style: self.style }
    }
}

/// Resolve the theme selector for a control's state. `hover` wins over
/// everything, so whatever the pointer is on always reads as reachable.
fn style_for(state: &AppState, id: BorderControl, on: bool, lit: bool) -> Style {
    let sel = if state.control_hover == Some(id) {
        "panel.control:hover"
    } else if lit {
        "panel.control:lit"
    } else if on {
        "panel.control:active"
    } else {
        "panel.control"
    };
    state.colors.theme.get(sel).style
}

/// The controls to draw in the story pane's border, left to right.
///
/// Always the three that apply to every story; the two v6 ones only when the
/// story really is v6 (header version 6, as `startup` recorded it), so they
/// appear and vanish with the game rather than being greyed out.
pub fn controls_for(state: &AppState) -> Vec<ControlView> {
    let g = &state.symbols.controls;
    let mut out = Vec::with_capacity(5);

    // ── Map ──────────────────────────────────────────────────────────────────
    let map_on = state.layout == Layout::Split;
    out.push(ControlView {
        id: BorderControl::Map,
        glyph: if map_on { g.map_hide } else { g.map_show },
        style: style_for(state, BorderControl::Map, map_on, false),
        hint: vec![
            if map_on { "Map: shown — click to hide" } else { "Map: hidden — click to show" }
                .to_string(),
            "/toggle-map".to_string(),
        ],
    });

    // ── Guidance ─────────────────────────────────────────────────────────────
    let guide_on = state.config.guidance;
    out.push(ControlView {
        id: BorderControl::Guidance,
        glyph: if guide_on { g.guidance_on } else { g.guidance_off },
        // `lit`, not merely `active`: the Guiding Light gets the yellow slot.
        style: style_for(state, BorderControl::Guidance, guide_on, guide_on),
        hint: vec![
            if guide_on {
                "Guiding Light: on — click to put it out"
            } else {
                "Guiding Light: off — click to light it"
            }
            .to_string(),
            "/set-guidance".to_string(),
        ],
    });

    // ── Verb panel ───────────────────────────────────────────────────────────
    let band_on = state.command_band_visible();
    out.push(ControlView {
        id: BorderControl::VerbPanel,
        glyph: if band_on { g.band_hide } else { g.band_show },
        style: style_for(state, BorderControl::VerbPanel, band_on, false),
        hint: vec![
            if band_on {
                "Verb panel: open — click to close"
            } else {
                "Verb panel: closed — click to open"
            }
            .to_string(),
            "F2".to_string(),
        ],
    });

    if state.story_zversion != Some(6) {
        return out;
    }

    // ── v6 render mode (a cycle, so the hint names what is next) ─────────────
    let (mode_glyph, mode_name, next) = match state.config.v6_render {
        V6RenderMode::Hybrid => (g.render_hybrid, "hybrid", "raster"),
        V6RenderMode::Raster => (g.render_raster, "raster", "extended"),
        V6RenderMode::Extended => (g.render_extended, "extended", "hybrid"),
    };
    out.push(ControlView {
        id: BorderControl::V6Render,
        glyph: mode_glyph,
        // `hybrid` is how the game arrives, so it reads as the idle state and the
        // other two as a choice the player made.
        style: style_for(
            state,
            BorderControl::V6Render,
            state.config.v6_render != V6RenderMode::Hybrid,
            false,
        ),
        hint: vec![
            format!("Render: {mode_name} — click for {next}"),
            "/set-v6-render".to_string(),
        ],
    });

    // ── v6 pixel lock ────────────────────────────────────────────────────────
    let lock_on = state.config.v6_pixel_lock;
    out.push(ControlView {
        id: BorderControl::V6PixelLock,
        glyph: if lock_on { g.lock_on } else { g.lock_off },
        style: style_for(state, BorderControl::V6PixelLock, lock_on, false),
        hint: vec![
            if lock_on {
                "Pixel lock: on — click to unlock"
            } else {
                "Pixel lock: off — click to lock"
            }
            .to_string(),
            "/set-v6-pixel-lock".to_string(),
        ],
    });

    out
}

/// Draw the hover hint for whichever control the pointer is on, if any.
///
/// `hits` are this frame's control rects (the same ones a click resolves
/// against, so hint and click can never disagree about what is under the
/// pointer); `state.control_hover` is what the last `Moved` event resolved.
/// The box hangs one row BELOW the control — the controls sit in a top border,
/// so it drops into the pane and never covers the icon being pointed at — and
/// `tooltip::draw_tip` slides it left of the right edge and flips it above the
/// bottom one. It paints and nothing else: no focus, no keyboard, no event.
pub fn draw_control_hint(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    state: &AppState,
    views: &[ControlView],
    hits: &[(BorderControl, Rect)],
) -> Option<Rect> {
    // A modal owning the screen also owns the pointer: the hint is drawn after
    // the overlay ladder, so without this it would float on top of a dialog if
    // one opened while the pointer sat on a control and never moved after.
    if state.any_modal_overlay_open() {
        return None;
    }
    let id = state.control_hover?;
    let (_, rect) = hits.iter().find(|(i, _)| *i == id)?;
    let view = views.iter().find(|v| v.id == id)?;
    super::tooltip::draw_tip(buf, area, rect.x, rect.y, &view.hint, &state.colors.theme)
}

/// Resolve a pointer position against this frame's control rects.
///
/// One function for both the click path and the `Moved` hover path, so the two
/// always agree; returns `None` over anything else, including while a modal
/// overlay owns the screen.
pub fn control_at(
    state: &AppState,
    hits: &[(BorderControl, Rect)],
    col: u16,
    row: u16,
) -> Option<BorderControl> {
    if state.any_modal_overlay_open() {
        return None;
    }
    hits.iter()
        .find(|(_, r)| {
            r.width > 0 && r.height > 0 && col >= r.x && col < r.right() && row >= r.y
                && row < r.bottom()
        })
        .map(|(id, _)| *id)
}
