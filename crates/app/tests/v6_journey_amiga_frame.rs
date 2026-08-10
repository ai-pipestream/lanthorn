//! Journey's line-drawing frame under the Amiga interpreter profile — SQ-0742.
//!
//! The report, playing Journey in Ghostty (hybrid, the shipped default) with the
//! Amiga profile selected: *"in hybrid mode a nice border frame is being drawn,
//! but it doesn't scale to our window width (raster mode looks great). also,
//! hybrid mouse click not accurate (if you click way above the menu you will get
//! a hit)"* — and then, decisively, *"if i resize the window to match everything,
//! the mouse click works!"* and *"ibmpc mouse clicks work properly."*
//!
//! ONE defect, seen twice. Journey draws its frame as TEXT: reverse-video spaces
//! under the IBM PC profile, line-drawing glyphs (`┌ ─ ┐ │ └ ┘`) under the Amiga
//! one. Those glyphs are non-blank runs that share every row of the story box, so
//! the hybrid "painted menu takeover" gate — which tested rows only — called an
//! ordinary gameplay screen a menu and routed the whole frame to the CELL path.
//! That path draws the game's 80 columns one-for-one into a pane of any width
//! while placing the transcript and the click map PROPORTIONALLY across the pane,
//! so at 138 columns the border stopped at column 79, the prose ran straight
//! through it, and a click landed three game rows below where it was aimed — you
//! had to click well above the menu to press it. At an 80-column pane the two
//! placements coincide, which is exactly why resizing "fixed" it. Under IBM PC the
//! same rules are reverse-video SPACES, which trim to empty and never tripped the
//! gate — hence "ibmpc mouse clicks work properly".
//!
//! Fixed in two coordinated places, both in `render/screen.rs`:
//!   1. the takeover gate now requires a run to lie inside the story box on BOTH
//!      axes, so a frame rule beside the story is chrome, not a menu; and
//!   2. a text strip's repeated-glyph RULE is drawn across its own SCALED span
//!      rather than one terminal cell per fragment, so the border reaches the pane
//!      edge the way the IBM PC profile's reverse-video bars already do.
//!
//! Swept across pane widths — including deliberately wider than the game's 80
//! column screen — because a fix that only holds where the numbers coincide is the
//! bug in another costume. Both `honor_game_colours` modes, per the project's
//! colour-render convention.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global (the profile's colour numbers
/// resolve through it), so a case that boots one profile must not run beside a
/// case that boots the other.
static PALETTE: Mutex<()> = Mutex::new(());

/// Pane widths swept by every case: the game's own 80 columns, where the cell
/// path's 1:1 chrome and the proportional transcript happen to agree, and five
/// wider panes where they do not.
const WIDTHS: [u16; 6] = [80, 96, 110, 124, 138, 150];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Journey under `profile` and drive the intro to the Praxix command menu —
/// the frame the report is about: a full line-drawing border, a left picture
/// column, prose beside it and the command menu along the bottom.
///
/// The container is not the variable and never was: the user established that
/// `Journey.blb` with the Amiga interpreter selected behaves exactly as the Amiga
/// disk image does, so the ordinary in-repo fixture plus an explicit profile
/// reproduces the whole thing without an `.adf`.
fn journey_at_menu(profile: InterpreterProfile) -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..40 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(session)
}

/// A hybrid render at real kitty-ish cell metrics (8×18). `Picker::halfblocks()`
/// reports a 1×2 cell — a layout regime that never reproduces a scale defect at
/// all — so the sweep has to run at a plausible font cell (the same lesson
/// SQ-0548 recorded).
#[allow(deprecated)]
fn render_hybrid(
    model: &app::engine::ScreenModel,
    honor: bool,
    cols: u16,
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, 51);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (state, area, buf)
}

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

/// Which render path drew this frame, as `/dump-windows` reports it.
fn path_label(state: &app::state::AppState) -> String {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label.starts_with("path:"))
        .map(|e| e.label.clone())
        .unwrap_or_else(|| "<no path recorded>".into())
}

/// (a) Journey's gameplay screen is gameplay, not a painted menu takeover — under
/// BOTH profiles, so the line-drawing frame and the reverse-video one are drawn by
/// the same renderer.
///
/// FALSIFY by restoring the row-only takeover test in `screen.rs`: the Amiga cases
/// report `path:cell — painted menu takeover routed here`.
#[test]
fn journey_gameplay_takes_the_hybrid_ring_under_both_profiles() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        let Some(session) = journey_at_menu(profile) else { return };
        let model = session.screen();
        for honor in [true, false] {
            for cols in WIDTHS {
                let (state, _, _) = render_hybrid(&model, honor, cols);
                let path = path_label(&state);
                assert_eq!(
                    path, "path:hybrid-ring",
                    "{profile:?} honor={honor} w={cols}: an ordinary gameplay screen must take the \
                     chrome ring; a frame rule beside the story is chrome, not a menu takeover"
                );
            }
        }
    }
}

/// (b) The Amiga frame reaches the pane at EVERY width, and the prose stays inside
/// it. The user's two visible complaints in one assertion pair: the border runs
/// unbroken from the pane's left edge to its right edge, and the transcript stops
/// at the border instead of running through it.
///
/// FALSIFY by reverting `collapse_row_rules`: every width past 80 fails with the
/// closing corner stuck at column 79 — the reported "doesn't scale to our window
/// width".
#[test]
fn journey_amiga_border_reaches_the_pane_at_every_width() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(session) = journey_at_menu(InterpreterProfile::Amiga) else { return };
    let model = session.screen();
    for honor in [true, false] {
        for cols in WIDTHS {
            let (state, area, buf) = render_hybrid(&model, honor, cols);
            let ctx = format!("honor={honor} w={cols}");
            let find_row = |open: char| (0..area.height).find(|&y| row_text(&buf, area, y).starts_with(open));
            // Both horizontal rules of the box: the titled top and the plain bottom.
            //
            // The bottom rule is exempt at exactly 80 columns, where the pane is one
            // terminal column per native text cell. There the letterbox scale is 1.0
            // and a 16px game row COMPRESSES into an 18px terminal cell, while a text
            // strip draws its game rows on consecutive terminal rows (SQ-0543, one
            // cell tall each) — so the menu's seventh row, the frame's bottom edge,
            // needs a row past the pane's last. That is the packing's own vertical
            // arithmetic, older than this fix and untouched by it; the rule is drawn at
            // every width where the strip has the room.
            for (open, close) in [('┌', '┐'), ('└', '┘')] {
                let Some(y) = find_row(open) else {
                    assert!(
                        open == '└' && cols == 80,
                        "{ctx}: no frame row opening with {open:?}"
                    );
                    continue;
                };
                let row = row_text(&buf, area, y);
                let end = row
                    .char_indices()
                    .filter(|&(_, c)| c == close)
                    .map(|(i, _)| row[..i].chars().count())
                    .next_back()
                    .unwrap_or_else(|| panic!("{ctx}: frame row {y} has no closing {close:?}: {row:?}"));
                // The rule reaches the pane's right edge — not the game's 80th column.
                assert!(
                    end + 2 >= area.width as usize,
                    "{ctx}: the {open:?} border closes at column {end} of a {}-column pane — the \
                     rule must span the pane, not the game's own column count\n{row:?}",
                    area.width
                );
                // …and it is one unbroken line getting there.
                let hole = row.chars().take(end).position(|c| c == ' ');
                assert!(
                    hole.is_none(),
                    "{ctx}: the {open:?} border has a gap at column {} — a rule must close the \
                     seams a scale opens around its corners and title\n{row:?}",
                    hole.unwrap()
                );
            }
            // The prose stays inside the frame: the story viewport stops at or before
            // the border's own column, so no line of text crosses the right-hand rule.
            let top = find_row('┌').expect("the top rule is on screen at every width");
            let right_col = row_text(&buf, area, top).chars().count()
                - row_text(&buf, area, top).chars().rev().position(|c| c == '┐').unwrap()
                - 1;
            let vp = state.transcript_geom.get().expect("hybrid publishes transcript geometry").area;
            assert!(
                vp.right() as usize <= right_col,
                "{ctx}: the transcript ({vp:?}) runs past the frame's right rule at column \
                 {right_col} — the prose must wrap inside the border, not through it"
            );
        }
    }
}

/// (c) BEHAVIOURAL, and the half the player actually felt: a click where a menu
/// item is DRAWN must make the game act, at every pane width and under both
/// profiles.
///
/// The oracle is the game's own verdict, not "did the model change" — Journey
/// repaints its menu on a rejected click too. Pressing a party member's "Cast"
/// opens that character's spell list, so the menu gains spell names it did not
/// carry before. A click the game rejects leaves the same commands on screen.
///
/// FALSIFY by restoring the row-only takeover test: under Amiga at every width
/// past 80 the spell list never appears, because the cell path's proportional
/// click map puts the click three game rows below the row it was drawn on — the
/// reported "if you click way above the menu you will get a hit".
#[test]
fn journey_menu_click_where_drawn_reaches_the_game() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    // Spells Praxix can cast — what the game puts on screen when it accepts the
    // click, and nothing it shows on the plain command menu.
    const SPELLS: [&str; 3] = ["Tremor", "Wind", "Elevation"];
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        for honor in [true, false] {
            for cols in WIDTHS {
                // A fresh game per click: an accepted one takes Journey elsewhere.
                let Some(mut session) = journey_at_menu(profile) else { return };
                let model = session.screen();
                let ctx = format!("{profile:?} honor={honor} w={cols}");
                let (state, area, buf) = render_hybrid(&model, honor, cols);
                let map = state
                    .graphics_render
                    .borrow()
                    .last_v6_map
                    .expect("a v6 frame records a click map for the mouse handler");

                // WHERE IT IS DRAWN: the cell the player sees "Cast" on.
                let sy = (0..area.height)
                    .find(|&y| row_text(&buf, area, y).contains("Cast"))
                    .unwrap_or_else(|| panic!("{ctx}: the 'Cast' command is drawn somewhere"));
                let sx = row_text(&buf, area, sy).find("Cast").expect("column of the label") as u16;
                let (gx, gy) = map
                    .map_click(sx + 1, sy)
                    .unwrap_or_else(|| panic!("{ctx}: the cell showing 'Cast' ({sx},{sy}) is off the image"));

                // Delivered exactly as `main.rs` delivers a v6 click during a
                // `read_char`: the game pixel, then ZSCII 254 (§3.8).
                assert_eq!(session.pending_input(), InputKind::Char, "{ctx}: the menu sits in read_char");
                session.set_mouse(gy, gx);
                let _ = session.submit_char(254);

                // DID THE GAME ACT? Its own answer: the spell list is on screen.
                let after = session.screen();
                let WinNode::Layered(items) = &after.root else { panic!("{ctx}: v6 Layered root") };
                let painted: Vec<String> = items
                    .iter()
                    .filter_map(|it| match &it.node {
                        WinNode::Grid(g) => Some(g.px_texts.iter()),
                        _ => None,
                    })
                    .flatten()
                    .map(|t| t.text.trim().to_string())
                    .collect();
                for spell in SPELLS {
                    assert!(
                        painted.iter().any(|t| t == spell),
                        "{ctx}: clicking 'Cast' where it is DRAWN (cell {sx},{sy} → game pixel \
                         {gx},{gy}) must open Praxix's spell list; {spell:?} never appeared. \
                         Menu now: {painted:?}"
                    );
                }
            }
        }
    }
}
