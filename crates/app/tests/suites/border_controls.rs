//! The pane border's clickable toggle controls (SQ-1123).
//!
//! Guidance, the verb panel and the two v6 render switches had no presence on
//! screen at all: a player who turned guidance on saw nothing and could only
//! conclude it was broken. These assert the three things that closes the gap —
//! the controls are THERE, they SHOW their state, and the v6-only pair appears
//! only on a v6 story.
//!
//! Everything here renders into a buffer and reads cells back, because that is
//! the only evidence about a screen that is worth anything.

use app::render::controls::{control_at, controls_for, draw_control_hint, BorderControl};
use app::render::panel::{draw_panel_with_controls, PanelSpec, PanelStrip};
use app::render::paneframe::{header_controls_width, InsetSegment, PaneGlyphs};
use app::state::{AppState, CommandBandState, Layout};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// Draw the story panel the way `main::draw_story_panel` does, into a fresh
/// buffer, and hand back the buffer plus the control hit-rects.
fn draw(state: &AppState, w: u16, h: u16) -> (Buffer, Vec<(BorderControl, Rect)>) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let views = controls_for(state);
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let title_style = state.colors.theme.get("story_title").style;
    let segs = [InsetSegment { text: &state.pane_title, active: false }];
    let (_, rects) = draw_panel_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip { segments: &segs, base: title_style, active: title_style }),
            body_fill: None,
        },
        &ctls,
        &state.colors.theme,
    );
    let hits = views.iter().map(|v| v.id).zip(rects).collect();
    (buf, hits)
}

/// One buffer row as a string.
fn row(buf: &Buffer, y: u16) -> String {
    (buf.area.x..buf.area.right())
        .map(|x| buf.cell((x, y)).unwrap().symbol().to_owned())
        .collect()
}

/// A state with a title, a known z-version and every toggle explicitly OFF, so
/// the border has something to centre, the v6 gate has something to read, and no
/// case here silently depends on what `AppState::default()` happens to switch on
/// (it opens with the map shown and the Guiding Light lit).
fn story(zversion: Option<u8>) -> AppState {
    let mut st = AppState::default();
    st.pane_title = "ZORK I".into();
    st.story_zversion = zversion;
    st.layout = Layout::TranscriptFull;
    st.config.guidance = false;
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.v6_pixel_lock = false;
    st
}

fn open_band(state: &mut AppState) {
    state.overlays.command_band = Some(CommandBandState::new(
        app::render::command_band::default_verbs(),
        app::render::command_band::default_quick(),
    ));
    state.band_dock.toggle_to(true, true);
}

// ── The border, drawn ────────────────────────────────────────────────────────

/// A non-v6 story gets three controls; a v6 story gets five. Both rows are
/// printed so the shape is on the record and not merely asserted about.
#[test]
fn the_border_carries_three_controls_off_v6_and_five_on_it() {
    let plain = story(Some(3));
    let (buf, hits) = draw(&plain, 44, 6);
    let plain_row = row(&buf, 0);
    println!("z3   idle: {plain_row}");
    assert_eq!(hits.len(), 3, "off v6: map, guidance, verb panel");
    assert!(plain_row.contains("┤◀ ○ ▲├"), "z3 border row: {plain_row:?}");
    assert!(plain_row.contains("ZORK I"), "the title still fits: {plain_row:?}");
    // The v6 pair is ABSENT, not disabled: none of its glyphs is anywhere.
    for g in ['◧', '■', '▦', '▣', '□'] {
        assert!(!plain_row.contains(g), "z3 must not draw {g:?}: {plain_row:?}");
    }

    let v6 = story(Some(6));
    let (buf, hits) = draw(&v6, 44, 6);
    let v6_row = row(&buf, 0);
    println!("z6   idle: {v6_row}");
    assert_eq!(hits.len(), 5, "on v6: the render mode and the pixel lock join");
    assert!(v6_row.contains("┤◀ ○ ▲ ◧ □├"), "z6 border row: {v6_row:?}");
}

/// Every control draws a DIFFERENT glyph in its other state — a control that
/// looks the same on and off is half a control. Both rows are printed.
#[test]
fn every_control_changes_glyph_with_its_state() {
    let mut on = story(Some(6));
    on.layout = Layout::Split;
    on.config.guidance = true;
    open_band(&mut on);
    on.config.v6_render = app::config::V6RenderMode::Raster;
    on.config.v6_pixel_lock = true;

    let off = story(Some(6));

    let (on_buf, _) = draw(&on, 44, 6);
    let (off_buf, _) = draw(&off, 44, 6);
    let on_row = row(&on_buf, 0);
    let off_row = row(&off_buf, 0);
    println!("z6     on: {on_row}");
    println!("z6    off: {off_row}");

    // Map shown → ▶ (click and it leaves to the right); hidden → ◀.
    // Guidance lit → ●, out → ○. Band open → ▼ (click and it drops), closed → ▲.
    // Raster → ■ / hybrid → ◧. Lock on → ▣ / off → □.
    assert!(on_row.contains("┤▶ ● ▼ ■ ▣├"), "every-on row: {on_row:?}");
    assert!(off_row.contains("┤◀ ○ ▲ ◧ □├"), "every-off row: {off_row:?}");

    // …and the third render mode is a third glyph, not a repeat of either.
    let mut ext = story(Some(6));
    ext.config.v6_render = app::config::V6RenderMode::Extended;
    let (ext_buf, _) = draw(&ext, 44, 6);
    let ext_row = row(&ext_buf, 0);
    println!("z6    ext: {ext_row}");
    assert!(ext_row.contains("┤◀ ○ ▲ ▦ □├"), "extended row: {ext_row:?}");
}

/// The Guiding Light's control is YELLOW when lit, and it gets that yellow from
/// the theme's `alert` role — the same slot `transcript_assist` uses — not from
/// a hard-coded colour. Restyling the role must move the control with it.
#[test]
fn a_lit_guidance_control_takes_the_alert_role() {
    let mut st = story(Some(3));
    st.config.guidance = true;
    let (buf, hits) = draw(&st, 44, 6);
    let (_, r) = hits.iter().find(|(id, _)| *id == BorderControl::Guidance).unwrap();
    let lit = buf.cell((r.x, r.y)).unwrap();
    assert_eq!(lit.symbol(), "●", "lit guidance draws the filled mark");
    assert_eq!(
        lit.fg,
        st.colors.theme.get("alert").style.fg.unwrap(),
        "a lit Guiding Light is the alert role's yellow, not a literal",
    );
    assert!(lit.modifier.contains(Modifier::BOLD), "…and bold, per panel.control:lit");

    // Unlit it and the same cell falls back to the quiet `panel.control`.
    st.config.guidance = false;
    let (buf, _) = draw(&st, 44, 6);
    let out = buf.cell((r.x, r.y)).unwrap();
    assert_eq!(out.symbol(), "○");
    assert_eq!(
        out.fg,
        st.colors.theme.get("muted").style.fg.unwrap(),
        "an unlit control is muted",
    );
}

/// A hovered control takes `panel.control:hover`, so whatever the pointer is on
/// always reads as reachable — even the ones that are otherwise idle.
#[test]
fn the_hovered_control_is_highlighted() {
    let mut st = story(Some(3));
    let (_, hits) = draw(&st, 44, 6);
    let (_, r) = hits.iter().find(|(id, _)| *id == BorderControl::VerbPanel).unwrap();
    let (r_x, r_y) = (r.x, r.y);

    st.control_hover = Some(BorderControl::VerbPanel);
    let (buf, _) = draw(&st, 44, 6);
    let cell = buf.cell((r_x, r_y)).unwrap();
    assert!(
        cell.modifier.contains(Modifier::REVERSED),
        "panel.control:hover is reversed by default",
    );
}

// ── The hint ─────────────────────────────────────────────────────────────────

/// The hover hint says what the control is, what a click does, and how to do the
/// same from the keyboard — and it hangs BELOW the control, so it never covers
/// the icon being pointed at.
#[test]
fn the_hint_names_the_control_and_sits_below_it() {
    let mut st = story(Some(3));
    st.config.guidance = false;
    st.control_hover = Some(BorderControl::Guidance);

    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    let views = controls_for(&st);
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let segs = [InsetSegment { text: "ZORK I", active: false }];
    let title = st.colors.theme.get("story_title").style;
    let (_, rects) = draw_panel_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip { segments: &segs, base: title, active: title }),
            body_fill: None,
        },
        &ctls,
        &st.colors.theme,
    );
    let hits: Vec<_> = views.iter().map(|v| v.id).zip(rects).collect();
    let anchor = hits.iter().find(|(id, _)| *id == BorderControl::Guidance).unwrap().1;

    let tip = draw_control_hint(&mut buf, area, &st, &views, &hits).expect("the hint is drawn");
    assert!(tip.y > anchor.y, "the hint hangs below the control, never over it");
    assert_eq!(row(&buf, anchor.y).chars().nth(anchor.x as usize), Some('○'),
               "…and the control itself is still visible");

    let text: String = (tip.y..tip.bottom()).map(|y| row(&buf, y)).collect();
    println!("hint: {}", (tip.y..tip.bottom()).map(|y| row(&buf, y)).collect::<Vec<_>>().join(" / "));
    assert!(text.contains("Guiding Light: off"), "the hint states the state: {text:?}");
    assert!(text.contains("click to light it"), "…and what a click does: {text:?}");
    assert!(text.contains("/set-guidance"), "…and the command that does the same: {text:?}");
}

/// Near the right edge the hint slides LEFT rather than off the screen, and near
/// the bottom it flips ABOVE the control. Neither may panic, and neither may
/// draw outside the pane.
#[test]
fn the_hint_stays_inside_the_pane_at_both_edges() {
    let mut st = story(Some(6));
    st.control_hover = Some(BorderControl::V6PixelLock); // the right-most control

    for (w, h) in [(44u16, 8u16), (30, 4), (26, 3)] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let views = controls_for(&st);
        let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
        let (_, rects) = draw_panel_with_controls(
            &mut buf,
            &PanelSpec {
                area,
                border_selector: "panel.border",
                border_color: None,
                border_style: None,
                glyphs: &PaneGlyphs::default(),
                header_on: true,
                strip: None,
                body_fill: None,
            },
            &ctls,
            &st.colors.theme,
        );
        let hits: Vec<_> = views.iter().map(|v| v.id).zip(rects).collect();
        if let Some(tip) = draw_control_hint(&mut buf, area, &st, &views, &hits) {
            assert!(tip.right() <= area.right(), "{w}x{h}: hint ran off the right edge");
            assert!(tip.bottom() <= area.bottom(), "{w}x{h}: hint ran off the bottom");
            assert!(tip.x >= area.x && tip.y >= area.y, "{w}x{h}: hint ran off the top-left");
        }
    }
}

// ── Hit-testing and dispatch ─────────────────────────────────────────────────

/// The click path and the hover path resolve through ONE function against ONE
/// list of rects, so they can never disagree about what is under the pointer.
#[test]
fn a_click_and_a_hover_resolve_to_the_same_control() {
    let st = story(Some(6));
    let (_, hits) = draw(&st, 44, 6);
    for (id, r) in &hits {
        assert_eq!(control_at(&st, &hits, r.x, r.y), Some(*id));
    }
    // A cell one row down (inside the pane) is not a control.
    let (_, first) = hits[0];
    assert_eq!(control_at(&st, &hits, first.x, first.y + 1), None);
    // …nor is the separator column between two controls.
    assert_eq!(control_at(&st, &hits, first.x + 1, first.y), None);
}

/// Every control drives an existing `slash::COMMANDS` entry, bare. A control
/// that named a command the registry does not have would silently do nothing;
/// this is the guard, because nothing structural stops the string drifting.
#[test]
fn every_control_names_a_real_bare_slash_command() {
    for id in [
        BorderControl::Map,
        BorderControl::Guidance,
        BorderControl::VerbPanel,
        BorderControl::V6Render,
        BorderControl::V6PixelLock,
    ] {
        let name = id.command();
        let spec = app::slash::COMMANDS
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{id:?} names {name:?}, which is not in slash::COMMANDS"));
        // …and it must actually do something when given no argument, which is
        // all a click ever gives it.
        let outcome = app::slash::parse_in_context(name, '/', spec.context);
        assert!(
            !matches!(outcome, app::slash::SlashOutcome::Error(_)),
            "bare `{name}` is an error, so a click on {id:?} would do nothing",
        );
    }
}

/// A modal overlay owns the screen: while one is open the border controls are
/// unreachable by both click and hover, so a stray pointer cannot toggle
/// anything behind a dialog.
#[test]
fn a_modal_overlay_takes_the_controls_out_of_reach() {
    let mut st = story(Some(3));
    let (_, hits) = draw(&st, 44, 6);
    let (_, r) = hits[0];
    assert!(control_at(&st, &hits, r.x, r.y).is_some());
    st.overlays.quit_dialog = true;
    assert!(st.any_modal_overlay_open(), "the quit dialog is modal");
    assert_eq!(control_at(&st, &hits, r.x, r.y), None);
}

// ── Geometry ─────────────────────────────────────────────────────────────────

/// The cluster's columns come OUT of the title's before the title is centred, so
/// a title long enough to reach the controls is trimmed by the strip's own
/// overflow rules instead of being painted over them.
#[test]
fn a_long_title_never_overwrites_a_control() {
    let mut st = story(Some(6));
    st.pane_title = "A VERY LONG ADVENTURE TITLE INDEED".into();
    for w in 30..=60u16 {
        let (buf, hits) = draw(&st, w, 5);
        let r = row(&buf, 0);
        for (id, rect) in &hits {
            let sym = buf.cell((rect.x, rect.y)).unwrap().symbol().to_owned();
            let view = controls_for(&st).into_iter().find(|v| v.id == *id).unwrap();
            assert_eq!(sym, view.glyph.to_string(), "w={w}: title overwrote {id:?} — {r:?}");
        }
    }
}

/// Below the width the cluster needs, nothing is drawn and no hit-rect is
/// handed back — a half-cluster would be unclickable chrome.
#[test]
fn a_pane_too_narrow_for_the_cluster_draws_none_of_it() {
    let st = story(Some(6));
    let want = header_controls_width(5); // 2 caps + 5 glyphs + 4 gaps = 11
    assert_eq!(want, 11);
    for w in 4..=(want + 2) {
        let (_, hits) = draw(&st, w, 4);
        assert!(hits.is_empty(), "w={w}: the cluster needs {want} + a gap + two corners");
    }
    let (_, hits) = draw(&st, want + 4, 4);
    assert_eq!(hits.len(), 5, "…and it appears as soon as it fits");
}

/// …and the hint follows the same rule as the click: a modal that opens while
/// the pointer is resting on a control must not leave a hint floating over the
/// dialog. The hint is drawn after the overlay ladder, so this is its own guard
/// rather than a consequence of the hit test.
#[test]
fn a_modal_overlay_also_suppresses_the_hint() {
    let mut st = story(Some(3));
    st.control_hover = Some(BorderControl::Map);
    let area = Rect::new(0, 0, 50, 8);
    let mut buf = Buffer::empty(area);
    let views = controls_for(&st);
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let (_, rects) = draw_panel_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: None,
            body_fill: None,
        },
        &ctls,
        &st.colors.theme,
    );
    let hits: Vec<_> = views.iter().map(|v| v.id).zip(rects).collect();
    assert!(draw_control_hint(&mut buf, area, &st, &views, &hits).is_some(), "…normally it draws");
    st.overlays.quit_dialog = true;
    assert!(draw_control_hint(&mut buf, area, &st, &views, &hits).is_none());
}
