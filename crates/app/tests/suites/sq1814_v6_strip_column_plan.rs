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
/// columns. At 80 and up the menu is not touched.
#[test]
fn journey_menu_columns_stay_aligned_through_the_plan() {
    for honor in [true, false] {
        let Some(session) = journey_menu(honor) else { return };
        let model = session.screen();
        for cols in [40u16, 42, 44, 80, 100] {
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
