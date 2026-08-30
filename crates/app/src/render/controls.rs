//! The pane border's clickable toggle controls (SQ-1123).
//!
//! Guidance, the verb panel and the two v6 render switches were reachable only
//! by slash command, key or the settings screen — nothing on screen said they
//! existed, let alone whether they were on. A player who turned guidance on and
//! saw nothing had every reason to conclude it was broken. These are the answer:
//! icons riding the story pane's own border, each one saying what state it is in
//! and switching that state when clicked.
//!
//! Four rules shape everything here — and one exception, which arrived last and
//! is stated where it applies: [`BorderControl::Reveal`] is a TRIGGER, not a
//! switch. It has no state to report, remembers nothing, and lights only while
//! the thing it started is still happening.
//!
//! Four rules shape everything here.
//!
//! **A control sits where the thing it governs is, or where it would appear.**
//! The command band opens BELOW the story pane, so its toggle rides the bottom
//! border; the map lives to the RIGHT, so its toggle takes the bottom border's
//! right-hand end, nearest the pane it summons; guidance and the word reveal have
//! no direction of their own — the reveal acts on the story pane's own prose,
//! right there — so they join the band in the centred group; and the two v6 controls
//! govern how the story pane ITSELF is drawn, so they keep that pane's own top
//! border. See [`ControlPlacement`].
//!
//! **The one place that rule was wrong was the return probe** (SQ-0785), which
//! rode the MAP pane's border because the map is what it changes. But the search
//! keeps running when the map is hidden — hiding a view must not degrade the data
//! behind it — and a pane that disappears cannot carry the only switch for
//! something that does not: you could not turn off a feature that was still going.
//! So it sits on the story pane beside the map toggle, immediately inboard of it,
//! and every control lanthorn draws is now on one border of one pane (SQ-1107).
//!
//! **A click runs the command.** Each control names an existing entry in
//! `slash::COMMANDS` and the event loop puts that command string through the
//! ordinary slash pipeline, so clicking is byte-for-byte what typing it does —
//! including whatever the command persists. There is no second implementation of
//! any toggle beside the one the registry already owns.
//!
//! **The state is carried TWICE: by the glyph and by the colour.** The panel
//! toggles are arrows pointing the way the panel would move (the map lives right
//! of the story pane, the verb panel below it), the Guiding Light is filled when
//! lit and hollow when not, and the two v6 controls draw a distinct glyph per
//! mode — and on top of that, **every control that is ON is lit yellow**,
//! through `panel.control:lit`, which is the `alert` role and so the same slot
//! `transcript_assist` lights up in. The doubling is deliberate: a player who
//! cannot tell the two colours apart still has the shape, and the shape change
//! is legible at a glance without reading the colour.
//!
//! The render mode is a three-way cycle rather than a switch, so "on" needs a
//! reading: **`hybrid` is how the game arrives and is NOT lit; `raster` and
//! `extended` both are**, because either is a choice the player made.
//!
//! **The v6 pair does not exist off v6.** They are absent from the cluster
//! entirely rather than drawn disabled, so the border of a Zork I never shows a
//! switch that would do nothing — and since they are the only two controls on
//! the top border, a non-v6 story has no top cluster at all and its title strip
//! gets the whole row back.
//!
//! **What a click switches, it also remembers.** Every SWITCH here writes the
//! per-game `config.toml` sidecar, so a preference chosen for one story stays
//! with that story and no other. That is the commands' behaviour, not a second
//! implementation layered under the buttons: a click IS the command. The reveal
//! is exempt and says so through [`BorderControl::persists`], because a light
//! that was on for four seconds has nothing to remember — the `border_controls`
//! suite reads that method rather than a list, so the next control added without
//! a thought about persistence still fails the guard.

use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::{ControlPlacement, HeaderControl};
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
    /// The return probe (SQ-0785).
    ReturnProbe,
    /// The momentary word reveal (SQ-1107).
    ///
    /// **The first TRIGGER in this cluster, and the only one.** Every other
    /// control here reports a state you can read off it at a glance and flips
    /// that state when clicked. This one has no state to report: it makes
    /// something happen on the story pane and is over a few seconds later. Two
    /// consequences, both deliberate — its tooltip carries more weight than its
    /// neighbours', since the glyph alone cannot say what a press does; and it
    /// still LIGHTS for the duration of the reveal, not to report a state but so
    /// that a click visibly did something, because a press that happened to light
    /// no words would otherwise be indistinguishable from a broken button.
    Reveal,
}

impl BorderControl {
    /// Which of the pane's three border clusters this control belongs to.
    ///
    /// The whole placement rule, in one match: the two panel toggles point at
    /// panels that live below and to the right, and the two v6 switches act on
    /// the story pane itself.
    pub fn placement(self) -> ControlPlacement {
        match self {
            BorderControl::Map => ControlPlacement::BottomRight,
            // Guidance, the verb panel and the reveal have no direction of their
            // own — the reveal acts on the story pane's own prose, right there —
            // so they ride the bottom border together, centred.
            BorderControl::VerbPanel | BorderControl::Guidance | BorderControl::Reveal => {
                ControlPlacement::BottomCentre
            }
            BorderControl::V6Render | BorderControl::V6PixelLock => ControlPlacement::TopRight,
            // Beside the map toggle, on the STORY pane, immediately inboard of
            // it. It rode the map pane's own bottom border until SQ-1107, which
            // was the placement rule applied to the wrong half of the feature:
            // the search keeps running when the map is hidden — hiding a view
            // must not degrade the data behind it — so its only switch cannot
            // live on a pane that disappears. You could not turn off something
            // that was still running.
            BorderControl::ReturnProbe => ControlPlacement::BottomRight,
        }
    }

    /// The `slash::COMMANDS` entry a click runs, bare — which toggles or cycles
    /// for every switch here, and simply HAPPENS for [`BorderControl::Reveal`],
    /// the one trigger.
    pub fn command(self) -> &'static str {
        match self {
            BorderControl::Map => "toggle-map",
            BorderControl::Guidance => "set-guidance",
            BorderControl::VerbPanel => "open-command-band",
            BorderControl::V6Render => "set-v6-render",
            BorderControl::V6PixelLock => "set-v6-pixel-lock",
            BorderControl::ReturnProbe => "set-return-probe",
            BorderControl::Reveal => "reveal-words",
        }
    }

    /// Does this control REMEMBER what it switched, in the per-game sidecar?
    ///
    /// True of every switch here and false of the one trigger, which has nothing
    /// to remember. Stated rather than inferred, because the property it exists
    /// to guard is exactly "a control whose command does not persist" — see the
    /// `border_controls` suite, which walks this and asserts the registry's own
    /// description matches.
    pub fn persists(self) -> bool {
        !matches!(self, BorderControl::Reveal)
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
        HeaderControl { glyph: self.glyph, style: self.style, placement: self.id.placement() }
    }
}

/// Resolve the theme selector for a control's state: lit when it is on, quiet
/// when it is not, and `hover` over everything — so whatever the pointer is on
/// always reads as reachable, on or off.
///
/// Two selectors, not three. There was a `panel.control:active` beside `:lit`
/// while "on" and "lit" were different states; every on-state is lit now, so
/// nothing could ever resolve to it, and a selector a themer can set and never
/// see is worse than one that does not exist.
fn style_for(state: &AppState, id: BorderControl, lit: bool) -> Style {
    let sel = if state.control_hover == Some(id) {
        "panel.control:hover"
    } else if lit {
        "panel.control:lit"
    } else {
        "panel.control"
    };
    state.colors.theme.get(sel).style
}

/// The controls to draw in the story pane's border, left to right.
///
/// Always the five that apply to every story; the two v6 ones only when the
/// story really is v6 (header version 6, as `startup` recorded it), so they
/// appear and vanish with the game rather than being greyed out.
///
/// **Order is placement, within a group.** The groups are filtered out of this
/// one list in index order, so the probe standing ahead of the map toggle is what
/// puts it inboard — and what makes it the one that goes first when the pane
/// narrows, since an anchored group sheds from its left.
pub fn controls_for(state: &AppState) -> Vec<ControlView> {
    let g = &state.symbols.controls;
    let mut out = Vec::with_capacity(7);

    // ── Return probe ─────────────────────────────────────────────────────────
    // First, so it takes the INBOARD slot of the right-hand pair and the map
    // toggle keeps the corner. Within that pair the probe gives way first as the
    // pane narrows: the map toggle moves a whole pane and is the only way back to
    // a hidden map, so it survives longest (SQ-1107).
    //
    // **Drawn in both states, never hidden.** Every other switch here governs
    // something already on by default or already visible, so it is discovered by
    // being used. This one is off out of the box, and a switch nobody has ever
    // seen lit is a switch nobody finds: muted through the plain `panel.control`
    // when off, lit yellow when on, same glyph either way (see
    // [`crate::symbols::ControlGlyphs::return_probe`]).
    let probe_on = state.config.return_probe;
    out.push(ControlView {
        id: BorderControl::ReturnProbe,
        glyph: g.return_probe,
        style: style_for(state, BorderControl::ReturnProbe, probe_on),
        hint: vec![
            if probe_on {
                "Return probe: on — click to stop looking for the way back"
            } else {
                "Return probe: off — click to look for the way back after a move"
            }
            .to_string(),
            "/set-return-probe".to_string(),
        ],
    });

    // ── Map ──────────────────────────────────────────────────────────────────
    let map_on = state.layout == Layout::Split;
    out.push(ControlView {
        id: BorderControl::Map,
        glyph: if map_on { g.map_hide } else { g.map_show },
        style: style_for(state, BorderControl::Map, map_on),
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
        style: style_for(state, BorderControl::Guidance, guide_on),
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
        style: style_for(state, BorderControl::VerbPanel, band_on),
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

    // ── The reveal (a trigger, not a switch) ─────────────────────────────────
    // Lit while a reveal is up — which is not a state report, since there is no
    // state: it is the click's own acknowledgement, so a press that lights no
    // words still reads as a press that worked.
    //
    // Its hint does more work than the others'. Theirs need only name a state and
    // its opposite, because the glyph has already said which one is in force; a
    // lamp on a border says nothing at all about WHAT it lights, so this one has
    // to say it — and, when the Guiding Light is out, has to say why a press will
    // do nothing rather than leaving the player to conclude it is broken.
    let lit = state.reveal.as_ref().is_some_and(|r| r.is_lit());
    out.push(ControlView {
        id: BorderControl::Reveal,
        glyph: g.reveal,
        style: style_for(state, BorderControl::Reveal, lit),
        hint: vec![
            "Reveal: light the words on screen the parser knows".to_string(),
            if state.config.guidance {
                "click for a moment — it goes out on your next key"
            } else {
                "needs the Guiding Light, which is out — the lamp beside this one"
            }
            .to_string(),
            // Both, unlike its neighbours: F2's toggle can be found by clicking
            // the control it names, and this one cannot be found at all without
            // being told the key.
            "F4 · /reveal-words".to_string(),
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
        // other two as a choice the player made — which is what "on" means for a
        // cycle, and so what is lit.
        style: style_for(
            state,
            BorderControl::V6Render,
            state.config.v6_render != V6RenderMode::Hybrid,
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
        style: style_for(state, BorderControl::V6PixelLock, lock_on),
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
///
/// The box goes INTO the pane, whichever border its control rides: down from the
/// top one, up from the bottom one. It never covers the icon being pointed at,
/// and `tooltip::draw_tip_on` slides it left of the right edge and flips it to
/// the other side of the anchor rather than letting it run off. It paints and
/// nothing else: no focus, no keyboard, no event.
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
    // A group the pane was too narrow to draw leaves a zero-area rect; there is
    // no icon on screen to explain, so there is no hint either.
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let view = views.iter().find(|v| v.id == id)?;
    let side = match id.placement() {
        ControlPlacement::TopRight => super::tooltip::TipSide::Below,
        ControlPlacement::BottomCentre | ControlPlacement::BottomRight => {
            super::tooltip::TipSide::Above
        }
    };
    super::tooltip::draw_tip_on(
        buf,
        area,
        rect.x,
        rect.y,
        &view.hint,
        &state.colors.theme,
        &state.symbols,
        side,
    )
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
