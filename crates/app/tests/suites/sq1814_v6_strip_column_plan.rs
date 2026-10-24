//! SQ-1814: a v6 hybrid chrome TEXT strip keeps its right-hand text on a narrow pane.
//!
//! `draw_chrome_text_strip` POSITIONS each glyph run through the pane scale but
//! advances one cell per character, so on a narrow pane the start columns shrink
//! while the text does not. Arthur's status bar (window 2, native 28..612) lost the
//! tail of "St Anne's Day, Compline" ("...Compli" at 70 columns, "...Com" at 60,
//! "...Day," at 50) with the middle of the bar blank. When something would clip the
//! strip now drops the columns blank on EVERY row (SQ-1809's plan, shared through
//! `upper_window::strip_column_plan`) and maps every row through it.
//!
//! Specimens: Arthur r74 serial 890714 (`arthur-r74-s890714.z6`) at the Churchyard
//! after 12 Enter taps and 'n' at the restore prompt; Journey r83 serial 890706 at
//! the Praxix command menu (<= 40 Enter taps); Shogun r322 serial 890706 six turns
//! in. Zork Zero r393 draws its status over the frame art (over-art runs, not a text
//! strip), so it is not in the survey. Both `honor_game_colours` modes throughout.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(name: &str, profile: Option<InterpreterProfile>, honor: bool) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = match app::hints::load_story(&story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::resolve(&story_path, None);
    let picture_dims = picts.all_pict_dims();
    let screen_px = picts.std_window().or_else(|| profile.and_then(|p| p.std_window()));
    let mut session = GameSession::new_with_trace(
        story_bytes,
        honor,
        false,
        profile.and_then(|p| p.interpreter_number()),
        false,
        picture_dims,
        screen_px,
        profile.and_then(|p| p.default_colours()),
        None,
    )
    .expect("v6 story should boot");
    if let Some(p) = profile {
        session.machine.set_palette(p.palette());
    }
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// Arthur r74 at the Churchyard: boot, 12 Enter taps, 'n' at the restore prompt.
fn arthur(honor: bool) -> Option<GameSession> {
    let mut session = boot("arthur-r74-s890714.z6", None, honor)?;
    for _ in 0..12 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(session)
}

fn render(model: &app::engine::ScreenModel, honor: bool, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.set_game_picker(Some(ratatui_image::picker::Picker::halfblocks()));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}


/// The status row of an Arthur render, with the frame's half-block art blanked so
/// only the game's own cells are compared.
fn arthur_status(buf: &Buffer, area: Rect) -> (u16, String) {
    let y = (0..area.height).find(|&y| row_text(buf, area, y).contains("Churchyard")).expect("status row");
    let text = row_text(buf, area, y)
        .chars()
        .map(|c| if ('\u{2580}'..='\u{259F}').contains(&c) { ' ' } else { c })
        .collect();
    (y, text)
}

/// Both status fields whole, in order, and the reverse bar solid across them.
///
/// FALSIFY by making `chrome_strip_shift` return `StripShift { plan: None, .. }`: the
/// date reads `...Compli` at 70, `...Com` at 60 and `...Day,` at 50.
#[test]
fn arthur_status_keeps_both_fields_on_narrow_panes() {
    use ratatui::style::Modifier;
    for honor in [true, false] {
        let Some(session) = arthur(honor) else { return };
        let model = session.screen();
        for cols in [50u16, 60, 70, 74] {
            let (area, buf) = render(&model, honor, cols, 25);
            let (y, text) = arthur_status(&buf, area);
            let where_ = format!("honor={honor} {cols} cols: {text:?}");
            let loc = text.find("Churchyard").unwrap_or_else(|| panic!("{where_}: location whole"));
            let date = text.find("St Anne's Day, Compline").unwrap_or_else(|| panic!("{where_}: date whole"));
            assert!(date > loc + "Churchyard".len(), "{where_}: order kept");
            let rev: Vec<u16> =
                (0..area.width).filter(|&x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED)).collect();
            let (first, last) = (rev[0], *rev.last().unwrap());
            assert!(
                (first..=last).all(|x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED)),
                "{where_}: the bar stays solid"
            );
            assert!(usize::from(last) >= date + "St Anne's Day, Compline".len() - 1, "{where_}: the bar covers the date");
        }
    }
}

/// Where the strip already fits nothing moves: the rows pinned here are what main
/// drew before SQ-1814.
#[test]
fn arthur_status_is_unchanged_where_it_already_fits() {
    for honor in [true, false] {
        let Some(session) = arthur(honor) else { return };
        let model = session.screen();
        let (area, buf) = render(&model, honor, 80, 25);
        assert_eq!(
            arthur_status(&buf, area).1,
            "     Churchyard                                    St Anne's Day, Compline      "
        );
        let (area, buf) = render(&model, honor, 100, 25);
        assert_eq!(
            arthur_status(&buf, area).1,
            "               Churchyard                                    St Anne's Day, Compline                "
        );
    }
}

fn journey_menu(honor: bool) -> Option<GameSession> {
    let mut s = boot("journey-r83-s890706.z6", None, honor)?;
    for _ in 0..40 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(s)
}

/// The character column of `word` on the first row that carries it.
fn col_of(rows: &[String], word: &str) -> Option<usize> {
    rows.iter().find_map(|r| {
        let cs: Vec<char> = r.chars().collect();
        let w: Vec<char> = word.chars().collect();
        cs.windows(w.len()).position(|x| x == w.as_slice())
    })
}

/// Journey's multi-row menu: every row maps through the same plan, so the verb
/// columns stay aligned, and "Individual Commands" is no longer clipped at 40-44
/// columns (SQ-1818: from 41 up; below that one blank per gap cannot be kept, so the
/// menu clips instead of fusing). At 80 and up the menu is not touched.
#[test]
fn journey_menu_columns_stay_aligned_through_the_plan() {
    for honor in [true, false] {
        let Some(session) = journey_menu(honor) else { return };
        let model = session.screen();
        for cols in [41u16, 42, 44, 80, 100] {
            let (area, buf) = render(&model, honor, cols, 25);
            let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
            let where_ = format!("honor={honor} {cols} cols:\n{}", rows.join("\n"));
            assert!(col_of(&rows, "Individual Commands").is_some(), "{where_}");
            let (cast, scout, drop) = (col_of(&rows, "Cast"), col_of(&rows, "Scout"), col_of(&rows, "Drop"));
            assert!(cast.is_some(), "{where_}");
            assert_eq!(cast, scout, "{where_}");
            assert_eq!(cast, drop, "{where_}");
            assert_eq!(col_of(&rows, "Examine"), col_of(&rows, "Inventory"), "{where_}");
        }
    }
}

/// Shogun's two-row band already fits at these widths: Score and Moves stay in one
/// column, the fields whole (SQ-0757's invariant, untouched).
#[test]
fn shogun_band_is_untouched_at_narrow_widths() {
    for honor in [true, false] {
        let Some(mut s) = boot("shogun-r322-s890706.z6", Some(InterpreterProfile::IbmPc), honor) else { return };
        for turn in 0..6 {
            let _ = match s.pending_input() {
                InputKind::Line => s.submit(if turn % 2 == 0 { "look" } else { "wait" }),
                InputKind::Char => s.submit_char(13),
                InputKind::Event => s.submit(""),
            };
        }
        let model = s.screen();
        for cols in [50u16, 60, 70] {
            let (area, buf) = render(&model, honor, cols, 25);
            let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
            let where_ = format!("honor={honor} {cols} cols:\n{}", rows.join("\n"));
            for w in ["Erasmus", "SHOGUN", "Score:", "Moves:", "Bridge"] {
                assert!(col_of(&rows, w).is_some(), "{w} {where_}");
            }
            assert_eq!(col_of(&rows, "Score:"), col_of(&rows, "Moves:"), "{where_}");
        }
    }
}

// ── SQ-1815: the click map follows the strip column plan ────────────────────

use app::engine::WinNode;
use app::render::v6_layout as v6;

/// `hybrid_chrome_layout` for `session` at a host pane of `cols` x 45 cells with a
/// host font (10x22, not the game's 8x16).
fn host_layout(session: &GameSession, honor: bool, cols: u16) -> app::render::screen::V6HybridChromeLayout {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = honor;
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    let pane = Rect::new(0, 0, cols, 45);
    app::render::screen::hybrid_chrome_layout(&layout, native, pane, (10, 22), &state)
        .expect("a hybrid chrome layout")
}

/// The first drawn cell of every glyph run clicks back to a game pixel inside
/// the run's own native span (a character is 8 native px wide), never a neighbour's.
///
/// FALSIFY by passing `Vec::new()` for the strip columns in
/// `V6HybridChromeLayout::from_frame`'s `build_hybrid_click_map` call: every shifted
/// word clicks onto its neighbour, which an embedding host saw as "clicking Esher
/// does nothing".
fn assert_clicks_land_on_their_words(ctx: &str, hyb: &app::render::screen::V6HybridChromeLayout, cols: u16) -> usize {
    let mut checked = 0;
    for r in hyb.runs.iter().filter(|r| !r.over_art && !r.run.text.trim().is_empty()) {
        let Some((c0, c1)) = r.resolved else { continue };
        if c0 < 0 || c1 > i32::from(cols) {
            continue;
        }
        let n = r.run.text.trim_end().chars().count() as i32;
        let x0 = i32::from(r.run.x.max(1)) - 1;
        let x1 = x0 + n * 8;
        // Only the FIRST cell: a run's later characters advance a column each while the
        // scale compresses the pane, so they overshoot the run's own native span by
        // design (and always did).
        {
            let (col, row) = (c0 as u16, r.row as u16);
            let (gx, _) = hyb
                .click_map
                .map_click(col, row)
                .unwrap_or_else(|| panic!("{ctx}: {:?} cell ({col},{row}) maps to nothing", r.run.text));
            let gx = i32::from(gx);
            assert!(
                gx > x0 && gx <= x1,
                "{ctx}: {:?} drawn at ({col},{row}) clicks to native x {gx}, outside its own span {x0}..{x1}",
                r.run.text
            );
            checked += 1;
        }
    }
    checked
}

#[test]
fn journey_menu_clicks_resolve_through_the_strip_plan() {
    for honor in [true, false] {
        let Some(session) = journey_menu(honor) else { return };
        for cols in [41u16, 42, 44] {
            let hyb = host_layout(&session, honor, cols);
            let ctx = format!("honor={honor} {cols} cols");
            assert!(!hyb.click_map.strip_cols.is_empty(), "{ctx}: premise, the menu strip is planned");
            let n = assert_clicks_land_on_their_words(&ctx, &hyb, cols);
            assert!(n > 10, "{ctx}: non-vacuity, only {n} cells checked");
        }
        // A wide pane draws no plan and clicks as it always did.
        for cols in [100u16, 140] {
            let hyb = host_layout(&session, honor, cols);
            assert!(hyb.click_map.strip_cols.is_empty(), "honor={honor} {cols} cols: no plan, no remap");
            assert!(assert_clicks_land_on_their_words(&format!("honor={honor} {cols} cols"), &hyb, cols) > 10);
        }
    }
}

// ── SQ-1816: a run's edge whitespace is blank to the plan ───────────────────

/// Arthur's ' Churchyard' run leads with a space; counted as ink it left the plan one
/// column short at ~37 columns and the date clipped to "St Anne's D". The status bar
/// now shows the date whole down to the narrowest width the blanks can cover.
///
/// FALSIFY by making `glyph_ink_span` return its span unchanged: the date clips at 37.
#[test]
fn arthur_status_fits_at_the_widths_a_leading_space_used_to_defeat() {
    for honor in [true, false] {
        let Some(session) = arthur(honor) else { return };
        let model = session.screen();
        // SQ-1818: at 37 columns the blanks cannot cover the overrun with one blank
        // kept between the fields, so there is no plan: the date clips (never fuses).
        let (area, buf) = render(&model, honor, 37, 25);
        let (_, text) = arthur_status(&buf, area);
        assert!(text.contains("Churchyard"), "honor={honor} 37 cols: {text:?}");
        assert!(!text.contains("Compline") && !text.contains("ChurchyardSt"), "honor={honor} 37 cols clips: {text:?}");
        for cols in [38u16, 40, 44] {
            let (area, buf) = render(&model, honor, cols, 25);
            let (_, text) = arthur_status(&buf, area);
            assert!(text.contains("Churchyard"), "honor={honor} {cols} cols: {text:?}");
            assert!(text.contains("St Anne's Day, Compline"), "honor={honor} {cols} cols: {text:?}");
            // SQ-1818: the two fields never fuse; at least one blank cell between them.
            let (loc, date) = (text.find("Churchyard").unwrap(), text.find("St Anne's").unwrap());
            assert!(date > loc + "Churchyard".len(), "honor={honor} {cols} cols: fields fused or reordered: {text:?}");
        }
        // The host sees the same plan: the dropped blank is not part of the run it
        // is told to draw at `col`.
        let hyb = host_layout(&session, honor, 38);
        let runs: Vec<_> = hyb.runs.iter().filter(|r| r.run.text.contains("Churchyard")).collect();
        assert!(!runs.is_empty());
        for r in runs {
            assert!(!r.run.text.starts_with(' '), "honor={honor}: dropped blank still published: {:?}", r.run.text);
        }
    }
}
