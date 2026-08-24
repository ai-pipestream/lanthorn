//! The bottom command band (SQ-0664): the properties that only show up against
//! a real engine and a real render pass.
//!
//! The unit tests in `render/command_band.rs`, `state.rs`, `input.rs`,
//! `layout.rs` and `config.rs` cover the grammar, the focus ladder and the
//! geometry. What is left — and what the old verb menu got wrong — needs a
//! story running:
//!
//! * the object columns are LIVE (take something and it moves *here* → *carried*)
//! * the band is not a modal, so the story prompt line stays drawn
//! * …and graphical v6 keeps its pixel path while the band is open
//!
//! Real commercial stories are gitignored, so every test that needs one skips
//! vacuously when it is absent (the `any_v6_story_present` pattern).


use app::engine::Engine;
use app::graphics::PictSource;
use app::render::command_band::{
    default_quick, default_verbs, refresh_objects, COL_CARRIED, COL_HERE,
};
use app::session::GameSession;
use app::state::{AppState, CommandBandState};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


fn open_band(state: &mut AppState) {
    let band = CommandBandState::new(default_verbs(), default_quick());
    state.overlays.command_band = Some(band);
    state.band_dock.toggle_to(true, true);
}

fn dump(buf: &Buffer) -> String {
    buf.content().iter().map(|c| c.symbol().to_owned()).collect()
}

// ── Live objects ─────────────────────────────────────────────────────────────

/// Boot a plain (non-v6) Z-machine story from `stories/`, or `None` when the
/// gitignored fixture is absent.
fn boot_zmachine(file: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("story should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// The columns come from the engine's object tree, not a transcript scrape —
/// and they are refreshed per turn, so taking an object moves it from the
/// *here* column to the *carried* one. This is the defect the whole redesign
/// exists to fix: `build_verb_menu_nouns` tokenized the last 20 transcript
/// lines once, at open, and never looked again.
#[test]
fn taking_an_object_moves_it_from_here_to_carried() {
    // Mini-Zork opens in the West of House with a mailbox and a mat; the leaflet
    // inside the mailbox is the classic takeable.
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);

    // Turn 0: whatever is in the opening room is HERE, and nothing is carried.
    session.submit("look");
    seed_player_obj(&mut state, &session);
    refresh_objects(&mut state, &session);
    let here0 = state.overlays.command_band.as_ref().unwrap().here.clone();
    let carried0 = state.overlays.command_band.as_ref().unwrap().carried.clone();
    assert!(
        !state.overlays.command_band.as_ref().unwrap().here_is_seen,
        "a Z-machine story has a real object tree — never the scrape fallback"
    );
    assert!(!here0.is_empty(), "the opening room has objects in it: {here0:?}");
    assert!(carried0.is_empty(), "nothing carried at the start: {carried0:?}");

    // Take something that is here, then refresh: it must have MOVED, not been
    // added — the two columns are the object tree, not two independent lists.
    let target = here0
        .iter()
        .find(|o| {
            let l = o.to_lowercase();
            l.contains("mailbox") || l.contains("mat") || l.contains("leaflet")
        })
        .cloned()
        .unwrap_or_else(|| here0[0].clone());
    session.submit("open mailbox");
    session.submit("take leaflet");
    refresh_objects(&mut state, &session);

    let band = state.overlays.command_band.as_ref().unwrap();
    assert!(
        !band.carried.is_empty(),
        "the taken object shows up in *carried* on the next refresh (was here: {here0:?}, target {target:?})"
    );
    let taken = band.carried[0].clone();
    assert!(
        !band.here.iter().any(|h| h == &taken),
        "…and is gone from *here*: here={:?} carried={:?}",
        band.here,
        band.carried
    );

    // And the columns really are the band's pick sources.
    let carried_items = band.items(COL_CARRIED);
    assert!(carried_items.contains(&taken), "the carried column offers it: {carried_items:?}");
}

/// Mirror the turn loop's player-object lock, which the real run loop does in
/// `turn.rs` before the band ever refreshes.
fn seed_player_obj(state: &mut AppState, session: &GameSession) {
    if state.player_obj.is_none() {
        state.player_obj = session.introspect().and_then(|i| i.player_object());
    }
}

/// The player object is a child of whatever room they're in, so without an
/// explicit id-based exclusion it would show up in every room's *here*
/// column (Zork 1's player object prints as "cretin"). SQ-0667:
/// `refresh_objects` excludes it by id via `Introspect::room_objects_excluding`
/// — falsify by reverting the `refresh_objects` call site back to the plain
/// `room_objects` it used before.
#[test]
fn the_player_object_is_excluded_from_here() {
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    assert!(state.player_obj.is_some(), "a Z-machine story always finds the player object");

    refresh_objects(&mut state, &session);
    let here = state.overlays.command_band.as_ref().unwrap().here.clone();

    let loc = session.current_location().unwrap().number;
    let unfiltered = session.introspect().unwrap().room_objects(loc);
    assert_eq!(
        unfiltered.len(),
        here.len() + 1,
        "exactly the player object should be missing from the filtered list: \
         unfiltered={unfiltered:?} filtered(here)={here:?}"
    );
    let extra: Vec<&String> = unfiltered.iter().filter(|o| !here.contains(o)).collect();
    assert_eq!(extra.len(), 1, "one object — the player's own — is missing from `here`");
}

/// SQ-0676 end to end, against a REAL object tree: with the band open, typing
/// goes to the story prompt, the band highlights the nearest live object, and
/// Tab completes the word to that object's full (often multi-word) name. The
/// unit tests use a hand-made object list; this pins the same behaviour on
/// names the engine actually produces.
///
/// Falsifies against the pre-SQ-0676 band, where those keystrokes never reached
/// `state.input` at all (they filtered a column) and Tab swapped focus.
#[test]
fn typing_at_the_prompt_completes_from_the_live_object_columns() {
    use app::input::{apply_action, key_to_action, Action};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    let mut mapper = mapper::mapper::Mapper::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    refresh_objects(&mut state, &session);

    // Pick a real object from the room and type the first three characters of
    // its first word — whatever the story happens to call it.
    let here = state.overlays.command_band.as_ref().unwrap().here.clone();
    let target = here
        .iter()
        .find(|o| o.split_whitespace().next().is_some_and(|w| w.chars().count() > 3))
        .cloned()
        .unwrap_or_else(|| here[0].clone());
    let first_word = target.split_whitespace().next().unwrap().to_string();
    let typed: String = first_word.chars().take(3).collect();

    for c in format!("take {typed}").chars() {
        let a = key_to_action(&state, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        assert_eq!(a, Action::InputChar(c), "`{c}` must reach the story prompt");
        apply_action(a, &mut state, &mut mapper);
    }
    assert_eq!(state.input.value, format!("take {typed}"));

    let band = state.overlays.command_band.as_ref().unwrap();
    let (col, idx) = band.nearest_match(&state.input.value).expect("a live object matches");
    assert!(col == COL_HERE || col == COL_CARRIED, "after a verb, the object columns match");
    assert_eq!(band.items(col)[idx], target, "the nearest match is the object typed toward");

    let a = key_to_action(&state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(a, Action::BandTabPick(col, idx), "the open band owns Tab completion");
    apply_action(a, &mut state, &mut mapper);
    assert_eq!(
        state.input.value,
        format!("take {target}"),
        "Tab completed to the live object's full name"
    );
}

/// A closed band costs nothing: the refresh is a no-op that never touches the
/// engine's object tree.
#[test]
fn refresh_is_a_noop_while_the_band_is_closed() {
    let Some(session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    assert!(!refresh_objects(&mut state, &session));
    assert!(state.overlays.command_band.is_none());
}

/// The refresh reports "unchanged" on a second call with no turn in between, so
/// an idle frame does not force a repaint.
#[test]
fn an_unchanged_object_tree_does_not_force_a_repaint() {
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    assert!(refresh_objects(&mut state, &session), "the first fill is a change");
    assert!(!refresh_objects(&mut state, &session), "…and the second is not");
}

// ── Not a modal ──────────────────────────────────────────────────────────────

/// The story prompt line is gated on `!any_modal_overlay_open()`. The old verb
/// menu counted as a modal, so opening it HID the prompt — a half-typed command
/// with no sign it was still buffered. The band must not.
#[test]
fn the_story_prompt_line_is_still_drawn_while_the_band_is_open() {
    for honor in [true, false] {
        let Some(session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
        let area = Rect::new(0, 0, 80, 24);

        let render = |state: &AppState| -> String {
            let model = session.screen();
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
            dump(&buf)
        };

        let mut state = AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = honor;
        state.focus = app::state::Focus::Game;
        state.transcript = vec!["West of House".to_string(), "You are standing here.".to_string()];
        state.input.set("half a comm".to_string(), true);

        let closed = render(&state);
        assert!(closed.contains("half a comm"), "honor={honor}: sanity, the prompt draws closed");

        open_band(&mut state);
        let open = render(&state);
        assert!(
            open.contains("half a comm"),
            "honor={honor}: the band is a dock — the story prompt line stays live"
        );
        assert!(!state.any_modal_overlay_open(), "honor={honor}: and it is not a modal");
    }
}

/// Graphical v6 drops to the cell path while a MODAL overlay is up, because
/// image placements draw over terminal cells. The band is not a modal and lives
/// outside the story pane entirely, so the pixel path must survive it — the old
/// verb menu killed Arthur's and Zork Zero's artwork for as long as it was open.
///
/// Pinned in both `honor_game_colours` modes, per the colour-suite rule.
#[test]
fn v6_keeps_the_pixel_path_while_the_band_is_open() {
    for honor in [true, false] {
        let story_path = fixture_path("zork0-r393-s890714.z6");
        let Ok(story_bytes) = std::fs::read(&story_path) else {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return;
        };
        let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
        let picture_dims = picts.all_pict_dims();
        let std_window = picts.std_window();
        let mut session = GameSession::new_with_trace(
            story_bytes, honor, false, None, false, picture_dims, std_window, None, None
        )
        .expect("Zork0 (v6) should load and boot");
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();
        let _ = session.take_transcript();

        let mut state = AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        // A real kitty cell size: the pixel path only exists with a picker.
        state.game_picker = Some(app::render::graphics::kitty_picker(14, 28));
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        state.config.honor_game_colours = honor;
        let area = Rect::new(0, 0, 100, 40);

        let frame = |state: &AppState| {
            let model = session.screen();
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
        };
        let last_path = |state: &AppState| -> String {
            state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default()
        };

        frame(&state);
        assert_eq!(last_path(&state), "hybrid-ring", "honor={honor}: sanity, the ring runs");

        open_band(&mut state);
        frame(&state);
        assert_eq!(
            last_path(&state),
            "hybrid-ring",
            "honor={honor}: the command band must NOT drop v6 off the pixel path"
        );

        // Falsification anchor: a genuine modal still does drop it, so this test
        // is measuring the gate rather than a path that never changes.
        state.overlays.command_band = None;
        state.overlays.hotkey_dialog = true;
        frame(&state);
        assert_ne!(
            last_path(&state),
            "hybrid-ring",
            "honor={honor}: a real modal still drops the ring (the gate is live)"
        );
    }
}

// ── Geometry against a real frame ────────────────────────────────────────────

/// The band claims a bottom band of the frame and the story pane shrinks — it
/// never overlaps the help row, and it leaves the story pane usable.
#[test]
fn the_band_carves_a_bottom_strip_without_eating_the_help_row() {
    let mut state = AppState::default();
    let area = Rect::new(0, 0, 100, 40);

    let closed = app::layout::compute_pane_layout(area, &state, 0);
    open_band(&mut state);
    let open = app::layout::compute_pane_layout(area, &state, 0);

    assert_eq!(open.help_row, closed.help_row, "the help row does not move");
    assert!(open.command_band.height > 0);
    assert_eq!(open.command_band.y + open.command_band.height, open.help_row.y);
    assert_eq!(
        open.story.height + open.command_band.height,
        closed.story.height,
        "the band's rows come out of the story pane"
    );
    assert!(open.story.height > 0, "and the story pane survives");
}

/// Everything visible in the band is clickable, and the rects it emits land
/// inside the band's own area (so `band_mouse_action` can claim exactly them).
#[test]
fn every_band_element_emits_a_hit_rect_inside_the_band() {
    use app::render::command_band::{draw_command_band, CommandBandHits};

    let mut state = AppState::default();
    open_band(&mut state);
    {
        let b = state.overlays.command_band.as_mut().unwrap();
        b.here = vec!["iron door".to_string()];
        b.carried = vec!["brass key".to_string()];
        b.pick_word("unlock");
    }
    let area = Rect::new(0, 30, 100, 8);
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
    let mut hits = CommandBandHits::default();
    draw_command_band(&state, area, &mut buf, &mut 0, &mut hits);

    assert_eq!(hits.area, area);
    assert!(!hits.headers.is_empty(), "column headers are clickable");
    assert!(!hits.quick.is_empty(), "quick words are clickable");
    assert!(!hits.rows.is_empty(), "rows are clickable");
    assert!(!hits.columns.is_empty(), "columns are wheel targets");

    let inside = |r: &Rect| {
        r.x >= area.x && r.right() <= area.right() && r.y >= area.y && r.bottom() <= area.bottom()
    };
    for (_, r) in &hits.headers {
        assert!(inside(r), "header rect {r:?} escapes the band");
    }
    for (_, _, r) in &hits.rows {
        assert!(inside(r), "row rect {r:?} escapes the band");
    }
    for (_, r) in &hits.quick {
        assert!(inside(r), "quick rect {r:?} escapes the band");
    }
    // The object columns are reachable now, so their rows are real pick targets.
    assert!(hits.rows.iter().any(|(c, _, _)| *c == COL_HERE));
}
