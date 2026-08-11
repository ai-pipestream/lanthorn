//! **In hybrid, never rasterise what the game printed as a character.** SQ-0750,
//! and the hole its position-based classification left at the frame's corner,
//! SQ-0747.
//!
//! Both quests are one mechanism, which is why they are one file. The hybrid ring
//! carves the chrome into strips and classified a SIDE band by where it sat — every
//! band narrower than the pane became one `Art` strip, and each of its border columns
//! was carried down the reclaimed gap as a one-native-row crop stretched into a
//! narrow band. Journey's frame is drawn as TEXT under both interpreter profiles
//! (box-drawing glyphs on the Amiga, reverse-video spaces on the IBM PC), so its four
//! vertical rules were uploaded as their own RGBA bitmaps — measured off the release
//! floppy at a 117x64 terminal: 8x900px each for the columns at 1, 47 and 114, 16x900
//! for the one at 115, about 192 KB of raw RGBA per frame that redraws them, to render
//! what is 200 `│` characters.
//!
//! The decisive observation was that the SAME rule changed medium partway down the
//! screen: column 1 was a kitty placeholder for rows 3..52 and a font glyph for rows
//! 53..59, where it crosses the menu strip. So this was never "Journey draws verticals
//! as pictures" — the text path already drew this exact rule correctly, in the same
//! frame, for seven rows. It needed to cover the other fifty.
//!
//! And where the frame's top rule should have met those flanks there was a one-row
//! hole: `story_viewport_box` quantizes the story's top edge OUTWARD to a whole cell
//! while the top band ends at that quantized row, so a sliver row fell between the two,
//! carried no runs and no art, classified Empty → Art → skipped, and was never written
//! at all (terminal row 2 of the same capture, across all 115 columns).
//!
//! ## What is asserted here, and why it is a CONTENT test
//!
//! The oracle is the game's own paint runs, never the pane geometry and never the
//! interpreter profile:
//!
//!   * (a) every border character the game printed beside the story box reaches the
//!     terminal AS that character, in the cell its own run maps to, on every row from
//!     the frame's top rule down to the menu — so the box is one medium all the way
//!     round instead of font glyphs on top and a resampled bitmap down the sides;
//!   * (b) no row between the frame's top rule and the story's first row is left
//!     unwritten — the flanks claim the gap the quantization opens;
//!   * (c) and the reserved case: a side column that is genuine ARTWORK — Zork Zero's,
//!     Shogun's and Arthur's frames — stays a bitmap, because the runs cannot account
//!     for its pixels. Getting that wrong is a worse regression than the bug.
//!
//! Swept across pane widths (the SQ-0742 lineage repeatedly turns up defects that
//! exist only at some widths), at two pane origins, and in BOTH `honor_game_colours`
//! modes per the project's colour-render convention.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use app::engine::{Engine, PxText, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// `zvm::screen::set_palette` is process-global, so no two cases here may boot at once.
static PALETTE: Mutex<()> = Mutex::new(());

/// A rect as `/dump-windows` records it: `(x, y, w, h)`.
type Quad = (u16, u16, u16, u16);

/// Pane widths swept. 80 is the game's own column count, where the placement rates
/// coincide and a defect can hide; the rest do not coincide, and 115 is the story pane
/// of the 117x64 terminal the defect was captured at.
const WIDTHS: [u16; 6] = [80, 96, 110, 115, 138, 150];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story exactly as `startup.rs` does — the profile comes from the medium
/// unless one is named — and drive `turns` "keep going" turns. `None` (with a SKIP
/// note) when the gitignored fixture is absent.
fn boot(file: &str, profile: Option<InterpreterProfile>, turns: usize) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let loaded = match app::hints::load_mounted_story(&path) {
        Ok((l, _)) => l,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = profile.unwrap_or_else(|| InterpreterProfile::resolve(&path, None));
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        loaded.bytes().to_vec(),
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..turns {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        // Arthur asks whether to restore a saved game before its story starts; a short
        // key list bounces off that prompt and looks like rejected input.
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(s)
}

/// A hybrid render at real kitty-ish cell metrics (8x18) with the transcript fed in.
/// `Picker::halfblocks()` reports a 1x2 cell — a layout regime that reproduces no scale
/// defect at all — so the sweep runs at a plausible font cell (the SQ-0548 lesson).
#[allow(deprecated)]
fn render_pane(
    model: &app::engine::ScreenModel,
    honor: bool,
    pane: Quad,
    transcript: &str,
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    let area = Rect::new(pane.0, pane.1, pane.2, pane.3);
    let mut buf = Buffer::empty(Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (state, area, buf)
}

/// Every `/dump-windows` record whose label starts with `prefix`: `(label, cells)`.
fn records(state: &app::state::AppState, prefix: &str) -> Vec<(String, Quad)> {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .filter(|e| e.label.starts_with(prefix))
        .map(|e| (e.label.clone(), e.cells))
        .collect()
}

fn viewport_of(state: &app::state::AppState) -> Quad {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label == "viewport")
        .map(|e| e.cells)
        .expect("a hybrid ring frame records its story viewport")
}

/// The story window's native pixel box, from the model — the frame is what stands
/// beside it, so this is what "beside the story box" is measured against.
fn story_box(model: &app::engine::ScreenModel) -> (u32, u32, u32, u32) {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let s = app::render::v6_layout::classify_windows(items).story.expect("story window");
    (s.x_px as u32, s.y_px as u32, s.w_px as u32, s.h_px as u32)
}

/// Every paint run the game has on screen.
fn chrome_runs(model: &app::engine::ScreenModel) -> Vec<PxText> {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The frame's SIDE-RULE runs: single-character runs the game printed beside the story
/// box (never above or below it) on the story's own middle row, one per rule. This is
/// the whole oracle — the characters the game itself put in those columns.
fn side_rule_runs(model: &app::engine::ScreenModel) -> Vec<PxText> {
    let (sx, sy, sw, sh) = story_box(model);
    let mid = sy + sh / 2;
    chrome_runs(model)
        .into_iter()
        .filter(|t| {
            let py = t.y.max(1) as u32 - 1;
            let px0 = t.x.max(1) as u32 - 1;
            let w = t.text.chars().count().max(1) as u32 * 8;
            (py..py + 16).contains(&mid) && (px0 + w <= sx || px0 >= sx + sw)
        })
        .collect()
}

/// Where a run's first character lands, through the ring's own letterbox scale.
/// Recomputed from `uniform_scale` rather than read off the `scale` dump record,
/// which rounds to two decimals and drifts a column at some pane widths.
fn run_col(t: &PxText, model: &app::engine::ScreenModel, area: Rect) -> u16 {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let (cw, ch) = (8u32, 18u32);
    let s = app::render::v6_layout::uniform_scale(native, (area.width as u32 * cw, area.height as u32 * ch));
    let px = t.x.max(1) as f32 - 1.0;
    area.x + ((s.off_x as f32 + px * s.s) / cw as f32).round() as u16
}

/// Did this cell come out as the character `ch`, drawn as the game's own text? A
/// reverse-video SPACE is a solid block whose ink is the REVERSED modifier, so it is
/// the character AND the modifier that have to agree.
fn holds(buf: &Buffer, x: u16, y: u16, t: &PxText) -> bool {
    let Some(c) = buf.cell((x, y)) else { return false };
    let want = t.text.chars().next().unwrap_or(' ');
    let reversed = c.style().add_modifier.contains(Modifier::REVERSED);
    c.symbol().chars().next().unwrap_or(' ') == want && reversed == (t.style & 1 != 0)
}

// ── (a) + (b): the frame is one medium, all the way round, with no hole at its corner ──

/// SQ-0750 / SQ-0747 — Journey, on the release the report was captured off and on the
/// bare story file, under both interpreter profiles.
///
/// Every side rule the game PRINTED must be in the buffer as that character, in its own
/// column, on every row from the frame's top rule down to the menu. Before the fix the
/// rows below the top strip were an uploaded bitmap of the same character and the cell
/// buffer held nothing there at all; the row where the top rule meets the flanks was
/// written by nothing whatsoever.
///
/// Each of the ring's flank-border columns is checked against the game's own runs, in
/// both directions: a column the runs DO account for must have reached the screen as
/// that character, in every cell of its span; a column that stayed a bitmap must be one
/// no run covers (Journey's release 83 under the IBM PC profile prints one border run
/// and no right-hand rule at all, so its right flank is pixels the runs cannot explain
/// and correctly keeps the band).
///
/// FALSIFY by returning `None` from the `text_border` test in `flank_border_extension`
/// (so every column goes back to a stretched band): every case fails with
/// `the frame's side rule at column 47 came out as a BITMAP … a run the game printed
/// covers it`.
#[test]
fn journeys_frame_side_rules_are_the_characters_the_game_printed() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for (file, profile, want_glyphs) in [
        ("Journey - The Quest Begins.adf", None, 3),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga), 3),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::IbmPc), 1),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        let rules = side_rule_runs(&model);
        assert!(
            !rules.is_empty(),
            "{file} {profile:?}: Journey prints its frame's side rules as characters — the whole \
             premise of this case"
        );
        for honor in [true, false] {
            for pane in WIDTHS.iter().flat_map(|&w| [(0, 0, w, 61), (1, 1, w, 61), (1, 1, w, 71)]) {
                let (state, area, buf) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {pane:?}");
                let mut borders = records(&state, "flank-divider");
                borders.extend(records(&state, "flank-border"));
                let mut glyphs = 0usize;
                for (label, r) in &borders {
                    // Which of the game's own side rules, if any, lands in this column?
                    // One column of slack: the ring places the rule by its run and the
                    // band by the cells its ink covers, and those can round apart.
                    let run = rules.iter().find(|t| run_col(t, &model, area).abs_diff(r.0) <= 1);
                    match run {
                        Some(t) => {
                            assert!(
                                label.contains("glyph"),
                                "{ctx}: the frame's side rule at column {} came out as a BITMAP \
                                 ({label}) — a run the game printed covers it ({:?} at native x \
                                 {}), so in hybrid it must be drawn as that character",
                                r.0,
                                t.text,
                                t.x
                            );
                            glyphs += 1;
                            for y in r.1..r.1 + r.3 {
                                assert!(
                                    holds(&buf, r.0, y, t),
                                    "{ctx}: the frame's side rule {:?} is missing from column {} \
                                     on row {y} — the cell holds {:?}",
                                    t.text,
                                    r.0,
                                    buf.cell((r.0, y)).map(|c| c.symbol().to_string()).unwrap_or_default()
                                );
                            }
                        }
                        None => assert!(
                            !label.contains("glyph"),
                            "{ctx}: {label} at {r:?} was stamped as a character, but no run the \
                             game printed covers that column — the glyph path is reserved for \
                             pixels the runs account for.\nruns: {:?}",
                            rules.iter().map(|t| (t.x, t.text.clone())).collect::<Vec<_>>()
                        ),
                    }
                }
                assert_eq!(
                    glyphs, want_glyphs,
                    "{ctx}: this frame's side rules are {want_glyphs} characters the game \
                     printed, and every one of them must reach the screen as a character\n{borders:#?}"
                );
            }
        }
    }
}

/// SQ-0747 — no unwritten row between the frame's top rule and the story's first row.
///
/// The gap is one row of ceil-quantization, and it belonged to nothing: no runs map
/// into it, no art stands behind it, so it classified Empty → Art and the ring skipped
/// it. On the captured frame that left terminal row 2 never written across all 115
/// columns — a bare stripe through the picture panel and a one-row hole where the
/// frame's top rule should meet its two side rules.
///
/// FALSIFY by dropping the gap-absorption block in `screen.rs` (the `strips` fixup
/// after `strip_has_art`): the 115-column panes fail with `row 2 lies between the
/// frame's top rule (ends at row 2) and the story's first row (3) and is written by
/// nothing at all`.
#[test]
fn no_unwritten_row_stands_between_the_frames_top_rule_and_the_story() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for (file, profile) in [
        ("Journey - The Quest Begins.adf", None),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga)),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in WIDTHS.iter().flat_map(|&w| [(0, 0, w, 61), (1, 1, w, 61), (1, 1, w, 71)]) {
                let (state, area, buf) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {pane:?}");
                let vp = viewport_of(&state);
                let Some(top) = records(&state, "strip:text").into_iter().map(|(_, r)| r.1 + r.3).min() else {
                    continue;
                };
                // An untouched ratatui cell is a blank symbol with `Color::Reset` on both
                // channels and no modifier — precisely the `·` the pty harness prints for
                // "never written". Asking `Style::bg.is_some()` would not do it: that is
                // `Some(Reset)` on a cell nothing has touched.
                let untouched = |x: u16, y: u16| {
                    buf.cell((x, y)).is_some_and(|c| {
                        c.symbol().trim().is_empty()
                            && c.fg == ratatui::style::Color::Reset
                            && c.bg == ratatui::style::Color::Reset
                            && c.modifier.is_empty()
                    })
                };
                for y in top..vp.1 {
                    assert!(
                        (area.x..area.right()).any(|x| !untouched(x, y)),
                        "{ctx}: row {y} lies between the frame's top rule (ends at row {top}) and \
                         the story's first row ({}) and is written by nothing at all — the flanks \
                         must claim the gap the viewport's quantization opens",
                        vp.1
                    );
                }
                // …and the frame's own side rules run through it, so the box closes at its
                // corner instead of leaving a one-row hole where the top rule meets them.
                let mut borders = records(&state, "flank-divider");
                borders.extend(records(&state, "flank-border"));
                for (label, r) in &borders {
                    assert!(
                        r.1 <= top,
                        "{ctx}: {label} starts at row {} — the frame's side rule must reach the \
                         top rule (which ends at row {top}), not the quantized viewport top ({})",
                        r.1,
                        vp.1
                    );
                }
            }
        }
    }
}

// ── (c) the reserved case: genuine artwork stays a bitmap ──

/// The corpus this rule must NOT reach: a side column that is a picture.
///
/// Zork Zero's, Shogun's and Arthur's frames are artwork down both sides — the runs
/// cannot account for those pixels, so they stay bitmaps, placed exactly where they
/// were. Getting this wrong is a worse regression than the defect being fixed, and it
/// is the reason SQ-0750 sat open through five passes.
///
/// FALSIFY by classifying a side band as text unconditionally (rather than on the
/// graphics canvas being clear): every case fails with `Zork Zero's left flank is no
/// longer drawn as art`.
#[test]
fn a_side_column_that_is_artwork_stays_a_bitmap() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for file in [
        "zork0-r393-s890714.z6",
        "Zork Zero - The Revenge of Megaboz.adf",
        "shogun-r322-s890706.z6",
        "arthur-r74-s890714.z6",
        "Arthur - The Quest for Excalibur.adf",
    ] {
        let Some(mut session) = boot(file, None, 12) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in WIDTHS.iter().flat_map(|&w| [(0, 0, w, 61), (1, 1, w, 64)]) {
                let (state, _, _) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{file} honor={honor} pane {pane:?}");
                let vp = viewport_of(&state);
                // Side ART strips: narrower than the pane, and DRAWN (a strip the ring
                // skipped says so in its own label).
                let sides: Vec<Quad> = records(&state, "strip:art")
                    .into_iter()
                    .filter(|(label, r)| label == "strip:art" && r.2 < pane.2)
                    .map(|(_, r)| r)
                    .collect();
                assert!(
                    !sides.is_empty(),
                    "{ctx}: this game's side columns are ARTWORK and must still be drawn as \
                     bitmaps — no side art strip survives.\n{:#?}",
                    records(&state, "strip")
                );
                for r in &sides {
                    // A flank never starts BELOW the story's first row: it either begins
                    // with it or reaches up into the quantization remainder above it
                    // (SQ-0747). Its bottom is the ring's business — the Extend plan
                    // clips a flank to the art's own last row, which is the point of it.
                    assert!(
                        r.1 <= vp.1,
                        "{ctx}: the side art strip {r:?} starts below the story viewport {vp:?}"
                    );
                }
                // …and none of them was reclassified into the game's own characters.
                let mut glyphs = records(&state, "flank-divider");
                glyphs.extend(records(&state, "flank-border"));
                assert!(
                    glyphs.iter().all(|(l, _)| !l.contains("glyph")),
                    "{ctx}: a border column of an ARTWORK flank was drawn as a character — the \
                     content test must reserve the glyph path for pixels the game's runs \
                     account for.\n{glyphs:#?}"
                );
            }
        }
    }
}
