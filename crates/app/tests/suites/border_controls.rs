//! The pane border's clickable toggle controls (SQ-1123).
//!
//! Guidance, the verb panel and the two v6 render switches had no presence on
//! screen at all: a player who turned guidance on saw nothing and could only
//! conclude it was broken. These assert the four things that close the gap —
//! the controls are THERE, they SHOW their state, the v6-only pair appears only
//! on a v6 story, and each one sits where the thing it governs is.
//!
//! **The placement rule, because it is the part most likely to be undone by a
//! later edit that means well.** A control rides the border nearest what it
//! switches: the command band opens BELOW the story pane and the map lives to
//! the RIGHT, so those toggles take the bottom border and its right-hand end;
//! guidance has no direction of its own and joins the band; and the two v6
//! switches govern how the story pane ITSELF is drawn, so they keep that pane's
//! own top border. Off v6 there is no top cluster at all.
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
    // Exactly what `main::draw_story_panel` does: a group the pane was too narrow
    // to hold leaves zero-area rects, and those are not on screen.
    let hits = views
        .iter()
        .map(|v| v.id)
        .zip(rects)
        .filter(|(_, r)| r.width > 0 && r.height > 0)
        .collect();
    (buf, hits)
}

/// Every control there is. A list rather than a match on the enum because the
/// enum has no iterator; a control added without a line here escapes the two
/// registry guards below, which is the only way those guards can go stale.
const EVERY_CONTROL: [BorderControl; 7] = [
    BorderControl::Map,
    BorderControl::Guidance,
    BorderControl::VerbPanel,
    BorderControl::V6Render,
    BorderControl::V6PixelLock,
    BorderControl::ReturnProbe,
    BorderControl::Reveal,
];

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

/// Light a word reveal, the way a press of the reveal control does — without an
/// engine, since nothing here is running a story. The trigger's control lights
/// for exactly as long as this is up (SQ-1107).
fn light_reveal(state: &mut AppState) {
    state.reveal = Some(app::reveal::Reveal {
        words: ["lantern".to_string()].into_iter().collect(),
        tier: app::reveal::RevealTier::Scope,
        until: std::time::Instant::now() + app::reveal::REVEAL_HOLD,
    });
}

fn open_band(state: &mut AppState) {
    state.overlays.command_band = Some(CommandBandState::new(
        app::render::command_band::default_verbs(),
        app::render::command_band::default_quick(),
    ));
    state.band_dock.toggle_to(true, true);
}

// ── The border, drawn ────────────────────────────────────────────────────────

/// Where each control sits, printed. The bottom border carries the two switches
/// centred and the map toggle at its right-hand end; the top border carries the
/// v6 pair and, off v6, nothing at all.
#[test]
fn the_controls_ride_the_border_nearest_what_they_switch() {
    let plain = story(Some(3));
    let (buf, hits) = draw(&plain, 44, 6);
    let (top, bottom) = (row(&buf, 0), row(&buf, 5));
    println!("z3 top:    {top}");
    println!("z3 bottom: {bottom}");
    assert_eq!(hits.len(), 4, "off v6: map, guidance, verb panel, reveal");

    // Bottom: `┤○ ▲ ◈├` centred, `┤◀├` anchored right, one corner clear of each.
    assert!(bottom.contains("┤○ ▲ ◈├"), "the centred group: {bottom:?}");
    assert!(bottom.ends_with("┤◀├┘"), "the map toggle takes the right end: {bottom:?}");
    // Off v6 the top border carries NO cluster — the two v6 switches are the only
    // controls that ever live there — so nothing is reserved and the title strip
    // is centred across the WHOLE row. (Which is a behaviour change of its own:
    // the first pass reserved eleven columns on every story, v6 or not, and
    // `render_overflow` clipped long titles against that. SQ-1127.)
    assert!(top.contains("ZORK I"), "the title still fits: {top:?}");
    for g in ['◀', '○', '▲', '◈', '◧', '□', '▶', '●', '▼', '■', '▣', '▦'] {
        assert!(!top.contains(g), "z3 top border must carry no control, found {g:?}: {top:?}");
    }
    let dashes = |t: &str, part: &str| t.split(part).map(|p| p.matches('─').count()).collect::<Vec<_>>();
    let d = dashes(&top, "┤ ZORK I ├");
    assert_eq!(d[0], d[1], "off v6 the title is centred across the whole row: {top:?}");

    let v6 = story(Some(6));
    let (buf, hits) = draw(&v6, 44, 6);
    let (top, bottom) = (row(&buf, 0), row(&buf, 5));
    println!("z6 top:    {top}");
    println!("z6 bottom: {bottom}");
    assert_eq!(hits.len(), 6, "on v6: the render mode and the pixel lock join");
    assert!(top.contains("┤◧ □├"), "the v6 pair keeps the top border: {top:?}");
    // …and now that the cluster IS reserved, the title is centred in what is left
    // of the row rather than in the row: fewer dashes on its left than its right.
    let d = dashes(&top, "┤ ZORK I ├");
    assert!(d[0] < d[1], "the v6 cluster's columns come out of the title's: {top:?}");
    assert!(bottom.contains("┤○ ▲ ◈├"), "…and the bottom row is unchanged by it: {bottom:?}");
    assert!(bottom.ends_with("┤◀├┘"), "{bottom:?}");
}

/// Nothing is ever drawn on the story pane's RIGHT border column, which is where
/// the vertical splitter is dragged (`story.right() - 1`, two columns wide with
/// the map pane's own left border). The map toggle is anchored one column inside
/// it, against the corner.
#[test]
fn no_control_lands_on_the_splitters_column() {
    let st = story(Some(6));
    let (buf, hits) = draw(&st, 44, 20);
    let right = buf.area.right() - 1;
    for (id, r) in &hits {
        assert!(r.right() <= right, "{id:?} at {r:?} reaches the splitter column {right}");
    }
    // …and the border column itself is still an unbroken run of frame.
    for y in 1..19u16 {
        assert_eq!(buf.cell((right, y)).unwrap().symbol(), "│", "row {y} of the right border");
    }
}

/// Every control draws a DIFFERENT glyph in its other state — a control that
/// looks the same on and off is half a control. Both borders are printed.
#[test]
fn every_control_changes_glyph_with_its_state() {
    let mut on = story(Some(6));
    on.layout = Layout::Split;
    on.config.guidance = true;
    open_band(&mut on);
    on.config.v6_render = app::config::V6RenderMode::Raster;
    on.config.v6_pixel_lock = true;
    // The reveal has no second GLYPH — it is a trigger, so its state is carried
    // by colour alone (see the case below). Lit here so the row is the every-on
    // row it claims to be.
    light_reveal(&mut on);

    let off = story(Some(6));

    let (on_buf, _) = draw(&on, 44, 6);
    let (off_buf, _) = draw(&off, 44, 6);
    for (tag, b) in [("on", &on_buf), ("off", &off_buf)] {
        println!("z6 {tag:>3} top:    {}", row(b, 0));
        println!("z6 {tag:>3} bottom: {}", row(b, 5));
    }

    // Map shown → ▶ (click and it leaves to the right); hidden → ◀.
    // Guidance lit → ●, out → ○. Band open → ▼ (click and it drops), closed → ▲.
    // Raster → ■ / hybrid → ◧. Lock on → ▣ / off → □.
    // The reveal is ◈ in both rows: a trigger has no other mode to draw.
    assert!(row(&on_buf, 5).contains("┤● ▼ ◈├"), "every-on bottom: {:?}", row(&on_buf, 5));
    assert!(row(&on_buf, 5).ends_with("┤▶├┘"), "every-on bottom: {:?}", row(&on_buf, 5));
    assert!(row(&on_buf, 0).contains("┤■ ▣├"), "every-on top: {:?}", row(&on_buf, 0));
    assert!(row(&off_buf, 5).contains("┤○ ▲ ◈├"), "every-off bottom: {:?}", row(&off_buf, 5));
    assert!(row(&off_buf, 5).ends_with("┤◀├┘"), "every-off bottom: {:?}", row(&off_buf, 5));
    assert!(row(&off_buf, 0).contains("┤◧ □├"), "every-off top: {:?}", row(&off_buf, 0));

    // …and the third render mode is a third glyph, not a repeat of either.
    let mut ext = story(Some(6));
    ext.config.v6_render = app::config::V6RenderMode::Extended;
    let (ext_buf, _) = draw(&ext, 44, 6);
    println!("z6 ext top:    {}", row(&ext_buf, 0));
    assert!(row(&ext_buf, 0).contains("┤▦ □├"), "extended top: {:?}", row(&ext_buf, 0));
}

/// **Every control that is ON is lit yellow**, and it gets that yellow from the
/// theme's `alert` role — the same slot `transcript_assist` uses — not from a
/// hard-coded colour. Restyling the role must move all of them with it.
///
/// So the state is carried TWICE: by the glyph and by the colour. That is
/// deliberate. A player who cannot tell the two colours apart still has the
/// shape, and the shape change is legible at a glance without reading colour.
#[test]
fn every_on_state_is_lit_from_the_alert_role_and_every_off_state_is_muted() {
    let alert = AppState::default().colors.theme.get("alert").style.fg.unwrap();
    let muted = AppState::default().colors.theme.get("muted").style.fg.unwrap();

    let mut on = story(Some(6));
    on.layout = Layout::Split;
    on.config.guidance = true;
    open_band(&mut on);
    on.config.v6_render = app::config::V6RenderMode::Raster;
    on.config.v6_pixel_lock = true;
    // The trigger has no on STATE; it lights while its reveal is up, which is the
    // click's own acknowledgement rather than a state report (SQ-1107).
    light_reveal(&mut on);
    let (buf, hits) = draw(&on, 44, 6);
    println!("all on  top: {} / bottom: {}", row(&buf, 0), row(&buf, 5));
    for (id, r) in &hits {
        let cell = buf.cell((r.x, r.y)).unwrap();
        assert_eq!(cell.fg, alert, "{id:?} is on and must be lit");
        assert!(cell.modifier.contains(Modifier::BOLD), "{id:?}: panel.control:lit is bold");
    }

    // …and off, every one of them is the quiet `panel.control`.
    let off = story(Some(6));
    let (buf, hits) = draw(&off, 44, 6);
    println!("all off top: {} / bottom: {}", row(&buf, 0), row(&buf, 5));
    for (id, r) in &hits {
        assert_eq!(buf.cell((r.x, r.y)).unwrap().fg, muted, "{id:?} is off and must be muted");
    }

    // The render mode is a CYCLE, not a switch, so "on" needs a reading: hybrid
    // is how the game arrives and is not lit; the other two are choices the
    // player made, and both are.
    for (mode, want_lit) in [
        (app::config::V6RenderMode::Hybrid, false),
        (app::config::V6RenderMode::Raster, true),
        (app::config::V6RenderMode::Extended, true),
    ] {
        let mut st = story(Some(6));
        st.config.v6_render = mode;
        let (buf, hits) = draw(&st, 44, 6);
        let (_, r) = hits.iter().find(|(id, _)| *id == BorderControl::V6Render).unwrap();
        let fg = buf.cell((r.x, r.y)).unwrap().fg;
        assert_eq!(fg == alert, want_lit, "{mode:?} lit? expected {want_lit}");
    }
}

/// A hovered control takes `panel.control:hover`/// A hovered control takes `panel.control:hover`, so whatever the pointer is on
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
/// same from the keyboard — and it goes INTO the pane, away from the icon being
/// pointed at. Guidance rides the BOTTOM border now, so "into the pane" is
/// upwards: a hint that still dropped one row would land in the command band.
#[test]
fn the_hint_names_the_control_and_sits_inside_the_pane() {
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
    assert!(tip.bottom() <= anchor.y, "a bottom-border hint rises into the pane, never over it");
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
    // The two hardest anchors: the top border's right-most control (slides left,
    // drops down) and the map toggle in the BOTTOM-RIGHT corner, which has to
    // slide left AND rise, with nothing below it to fall into.
    for anchor_id in [BorderControl::V6PixelLock, BorderControl::Map] {
    st.control_hover = Some(anchor_id);

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
            assert!(tip.right() <= area.right(), "{anchor_id:?} {w}x{h}: ran off the right edge");
            assert!(tip.bottom() <= area.bottom(), "{anchor_id:?} {w}x{h}: ran off the bottom");
            assert!(tip.x >= area.x && tip.y >= area.y, "{anchor_id:?} {w}x{h}: ran off the top-left");
        }
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
    // A cell one row up from a bottom control (inside the pane) is not a control.
    let guide = hits.iter().find(|(id, _)| *id == BorderControl::Guidance).unwrap().1;
    assert_eq!(control_at(&st, &hits, guide.x, guide.y - 1), None);
    // …nor is the separator column between two controls.
    assert_eq!(control_at(&st, &hits, guide.x + 1, guide.y), None);
}

/// Every control drives an existing `slash::COMMANDS` entry, bare. A control
/// that named a command the registry does not have would silently do nothing;
/// this is the guard, because nothing structural stops the string drifting.
#[test]
fn every_control_names_a_real_bare_slash_command() {
    for id in EVERY_CONTROL {
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

/// **What a click switches, it also remembers.** Every control's command
/// persists its result in the per-game `config.toml` sidecar, so a preference
/// chosen for one story stays with that story and no other.
///
/// This reads the registry's own description rather than the behaviour, because
/// behaviour is what each command's own dispatch case already pins — what this
/// catches is a control being added later whose command does not persist, which
/// would look identical on screen and quietly forget itself. Two commands changed
/// semantics to make this true: `set-v6-render` and `set-guidance` were both
/// session-only, and are now per-game like the pixel lock beside them.
///
/// **The reveal is the one exception, and it is stated rather than skipped**
/// (SQ-1107). It is a TRIGGER: there is nothing to remember about a light that
/// was on for four seconds, and `BorderControl::persists` is where that is
/// declared — so a future control added without a thought about persistence
/// still fails here, and only a control whose author wrote `persists() == false`
/// is exempt.
#[test]
fn every_control_switches_something_that_is_remembered_per_game() {
    for id in EVERY_CONTROL {
        let name = id.command();
        let spec = app::slash::COMMANDS.iter().find(|c| c.name == name).unwrap();
        if !id.persists() {
            assert!(
                !spec.description.contains("persisted per-game"),
                "{id:?} says it persists nothing, but `{name}` promises to remember: {:?}",
                spec.description,
            );
            continue;
        }
        assert!(
            spec.description.contains("persisted per-game"),
            "{id:?} runs `{name}`, whose description does not promise to remember it: {:?}",
            spec.description,
        );
    }
}

/// The trigger is not a switch, and the difference is worth pinning: it names a
/// command that takes no argument and stores nothing, it has ONE glyph in every
/// state, and its hint has to say what a press DOES because the glyph cannot.
#[test]
fn the_reveal_is_a_trigger_and_says_so() {
    assert!(!BorderControl::Reveal.persists(), "a trigger has nothing to remember");
    assert!(
        EVERY_CONTROL.iter().filter(|c| !c.persists()).count() == 1,
        "the reveal is still the only trigger; a second one wants its own thinking",
    );

    // Every other control's hint is two lines — a state and its opposite, then
    // the command. This one needs three: the glyph says nothing about WHAT it
    // lights, so the hint has to.
    let st = story(Some(3));
    let views = controls_for(&st);
    let reveal = views.iter().find(|v| v.id == BorderControl::Reveal).expect("drawn");
    let text = reveal.hint.join(" / ");
    println!("reveal hint: {text}");
    assert!(text.contains("light the words on screen"), "it says what it does: {text:?}");
    assert!(text.contains("/reveal-words"), "…and how to do it from the keyboard: {text:?}");
    // Guidance is out in `story()`, and a press would then do nothing at all —
    // which the hint has to say, or the player concludes the button is broken.
    assert!(text.contains("Guiding Light"), "…and why a press will do nothing: {text:?}");

    let mut lit = story(Some(3));
    lit.config.guidance = true;
    let on = controls_for(&lit);
    let reveal = on.iter().find(|v| v.id == BorderControl::Reveal).unwrap();
    assert!(
        !reveal.hint.join(" / ").contains("Guiding Light"),
        "with the light on there is nothing to warn about: {:?}",
        reveal.hint,
    );
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

/// Each group is drawn WHOLE or not at all, and the groups give way in a fixed
/// order as the pane narrows. A half cluster is unclickable chrome.
///
/// The map toggle is anchored and the pair is centred in what the anchor leaves,
/// so **the centred pair is what gives way first** — and the map toggle, the one
/// control that moves a whole pane, survives longest. The printed rows are the
/// record of where each threshold actually falls.
#[test]
fn the_groups_drop_whole_and_the_centred_pair_gives_way_first() {
    let st = story(Some(6));
    let has = |hits: &[(BorderControl, Rect)], id: BorderControl| {
        hits.iter().any(|(i, _)| *i == id)
    };
    let mut seen: Vec<(u16, bool, bool, bool)> = Vec::new();
    for w in 4..=24u16 {
        let (buf, hits) = draw(&st, w, 5);
        let map = has(&hits, BorderControl::Map);
        let pair = has(&hits, BorderControl::Guidance);
        let v6 = has(&hits, BorderControl::V6Render);
        // Guidance, the verb panel and the reveal are one group: never one of
        // them without the others.
        assert_eq!(pair, has(&hits, BorderControl::VerbPanel), "w={w}: half the centred group");
        assert_eq!(pair, has(&hits, BorderControl::Reveal), "w={w}: half the centred group");
        assert_eq!(v6, has(&hits, BorderControl::V6PixelLock), "w={w}: half the v6 pair");
        // The pair can never outlive the map toggle it has to make room for.
        assert!(!(pair && !map), "w={w}: the centred pair survived the anchored one");
        println!("w={w:>2} map={map:<5} pair={pair:<5} v6={v6:<5}  {} | {}", row(&buf, 0), row(&buf, 4));
        seen.push((w, map, pair, v6));
    }
    // The thresholds, pinned: 3 columns for `┤◀├` plus a spare, 7 for `┤○ ▲ ◈├`
    // plus a clear column between them, and 5 for the v6 pair plus a spare. The
    // centred group cost two more columns when the reveal joined it (SQ-1107),
    // which is two more columns of pane before it appears — the price of the
    // group being drawn whole or not at all.
    let first = |f: fn(&(u16, bool, bool, bool)) -> bool| seen.iter().find(|r| f(r)).unwrap().0;
    assert_eq!(first(|r| r.1), 7, "the map toggle needs a 7-column pane");
    assert_eq!(first(|r| r.2), 16, "the centred group needs 16");
    assert_eq!(first(|r| r.3), 9, "the top border's v6 pair needs 9");
}

/// A pane with no bottom border row to put them on draws no bottom controls —
/// and does not panic reaching for one.
#[test]
fn a_pane_with_no_room_for_a_bottom_border_draws_no_bottom_controls() {
    let st = story(Some(6));
    for h in 1..=3u16 {
        let (_, hits) = draw(&st, 44, h);
        for (id, r) in &hits {
            assert!(r.y < h, "h={h}: {id:?} at {r:?} is off the buffer");
        }
    }
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
