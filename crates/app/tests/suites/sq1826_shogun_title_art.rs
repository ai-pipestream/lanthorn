//! SQ-1826: v6 chrome text and its fills stay BETWEEN the side art and clip on the right.
//!
//! The owner's rule (2026-10-24): "anything that doesn't fit on the line (or between
//! side-art) should be truncated on the right side." Text never moves over side art and
//! never reflows. On Shogun r322's title card the embedding host (39x30 cells of 27x60 px,
//! scale 1.6453; and 42x35 of 20x46) reported the opposite: the story box is x=3 w=33 with
//! ornament columns 0..3 and 36..39, yet "Copyright (c) 1988 by Infocom" ran to col 39,
//! the trademark line to cols 1..39, rows 6..8 were slid to col 0 (SQ-1820), and the ground
//! fills (0,3,39,1) / (0,5,39,5) wiped the ornament on those rows.
//!
//! Cause: `ChromeRowOracle::blocked` judged a row by where its glyphs would be drawn
//! UNCLIPPED, so a line longer than the box "reached" the flank columns, the flank yielded
//! the row, and the text strip took the full pane width. The oracle now asks where the text
//! stands once clipped at the art's inner edge, so the flank keeps the row, the strip is the
//! art-free span, and the plan (SQ-1814), rules (SQ-1818), slide (SQ-1820/1823) and fills
//! all use that span as their bounds.
//!
//! The same cause made Shogun's in-game score bar flip between spanning the art and sitting
//! between it: "Erasmus:" is laid 3 native px past the ornament, which the scale ROUNDS into
//! the ornament at some widths (not at others), and the flank yielded the row exactly when it
//! did. A word the scale lays left of the strip is now drawn at the strip's edge, so the bar
//! is between the art at every width.
//!
//! Specimen: `shogun-r322-s890706.z6` (release 322, serial 890706), IBM PC press. Title card:
//! ONE Enter past the splash picture. Gameplay: Space then Enter (the first gameplay frame).
//! Hybrid, `honor_game_colours` on.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::screen::{hybrid_chrome_layout, render_story_pane, V6HybridChromeLayout};
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const HOST_SIZES: [(u16, u16, u16, u16); 4] = [(39, 30, 27, 60), (42, 35, 20, 46), (71, 16, 27, 60), (88, 36, 10, 23)];

/// Shogun booted the way `startup.rs` boots it (CLAUDE.md): the profile from the medium,
/// then one `MachineBoot` carrying the machine's facts. The real app's colours, art scale and
/// text face come with it, which a bare `new_with_trace` boot does not carry: the in-game bar
/// is a reverse-video bar there, and that is the path a real terminal takes.
struct Booted {
    session: GameSession,
    palette: zvm::screen::Palette,
    face: app::native_font::TextFace,
    art_scale: (u32, u32),
}

/// Shogun r322 off the Blorb-backed story file (IBM PC press).
const Z6: &str = "shogun-r322-s890706.z6";
/// Shogun r295 off the Amiga floppy: a different press, whose bar is painted with reverse-video
/// blank runs and whose first word sits at a different column.
const ADF: &str = "James Clavell\u{27}s Shogun.adf";

fn boot_shogun(file: &str) -> Option<Booted> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(file);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => return None,
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    let mut picts = PictSource::resolve_with_override(&path, app::graphics::PictureOverride::Unset, None);
    let dims = picts.all_pict_dims();
    let honoured = !picts.declines_game_colours(profile.default_colours());
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        faces.clone(),
        profile.palette(),
        None,
    );
    let art_scale = boot.art_scale;
    let face = app::native_font::TextFace::new(profile, faces, art_scale);
    let mut session = GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot).ok()?;
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(Booted { session, palette: profile.palette(), face, art_scale: art_scale.unwrap_or((2, 2)) })
}

/// The title card: one Enter past the splash.
fn title_card() -> Option<Booted> {
    let mut b = boot_shogun(Z6)?;
    let _ = match b.session.pending_input() {
        InputKind::Char => b.session.submit_char(13),
        _ => b.session.submit(""),
    };
    Some(b)
}

/// The first gameplay frame: Space (title card and boot menu), then Enter (START).
fn gameplay() -> Option<Booted> {
    gameplay_of(Z6)
}

/// [`gameplay`] off either medium. The Z6 frame is checked for its prose; the floppy's opening
/// prose differs, so its premise is that the status window painted its two lines.
fn gameplay_of(file: &str) -> Option<Booted> {
    let mut b = boot_shogun(file)?;
    let _ = b.session.submit_char(b' ');
    let r = b.session.submit_char(13);
    if file == Z6 {
        assert!(r.transcript.contains("Bridge"), "premise: the first gameplay frame");
    }
    Some(b)
}

fn state(b: &Booted, picker: ratatui_image::picker::Picker) -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default_in(b.palette);
    st.v6_art_scale = b.art_scale;
    st.v6_text = b.face.clone();
    st.set_game_picker(Some(picker));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.honor_game_colours = true;
    st
}

fn host(b: &Booted, size: (u16, u16, u16, u16)) -> V6HybridChromeLayout {
    host_locked(b, size, false)
}

/// [`host`] with the pixel lock `lock` (the gallery shots and many players run it on).
fn host_locked(b: &Booted, (cols, rows, cw, ch): (u16, u16, u16, u16), lock: bool) -> V6HybridChromeLayout {
    let mut st = state(b, app::render::graphics::kitty_picker(cw, ch));
    st.config.v6_pixel_lock = lock;
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &st.v6_text);
    let layout = v6::classify_windows(items, st.v6_text.cell());
    hybrid_chrome_layout(&layout, native, Rect::new(0, 0, cols, rows), (cw, ch), &st).expect("a hybrid chrome layout")
}

fn tui(b: &Booted, cols: u16, rows: u16) -> Buffer {
    let st = state(b, ratatui_image::picker::Picker::halfblocks());
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = render_story_pane(&b.session.screen(), false, None, &st, area, &mut buf);
    buf
}

/// The art-free span the terminal ring uses at this pane: the host layout at the picker's own
/// font size (the one source the terminal draw and the host share).
fn tui_span(b: &Booted, cols: u16, rows: u16) -> Rect {
    let fs = ratatui_image::picker::Picker::halfblocks().font_size();
    host(b, (cols, rows, fs.width, fs.height)).viewport
}

fn glyph(buf: &Buffer, x: u16, y: u16) -> char {
    buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')
}

/// The chrome text runs above the viewport (the card, or the bar).
fn chrome_text(ring: &V6HybridChromeLayout) -> Vec<&app::render::screen::V6HybridChromeRun> {
    ring.runs.iter().filter(|r| r.row < i32::from(ring.viewport.y) && !r.run.text.trim().is_empty()).collect()
}

/// Every chrome run above the viewport lies within `[vp.x, vp.right())` once resolved, and no
/// ground fill above the viewport leaves it.
fn assert_between_the_art(ring: &V6HybridChromeLayout, who: &str) {
    let vp = ring.viewport;
    let (lo, hi) = (i32::from(vp.x), i32::from(vp.right()));
    for r in chrome_text(ring) {
        let (a, b) = r.resolved.unwrap_or_else(|| panic!("{who}: {:?} is dropped", r.run.text));
        assert!(a >= lo && b <= hi, "{who}: {:?} is drawn at {a}..{b}, outside the art-free span {lo}..{hi}", r.run.text);
    }
    for g in ring.ground.iter().filter(|g| g.rect.y < vp.y) {
        let r = g.rect;
        assert!(i32::from(r.x) >= lo && i32::from(r.right()) <= hi, "{who}: ground fill {r:?} covers side-art columns (span {lo}..{hi})");
    }
}

/// The title card at the host's portrait and landscape sizes: every run and every fill is
/// inside the art-free span, the lines that overrun are clipped on the right, and the card
/// keeps its nine lines.
///
/// FALSIFY by making `ChromeRowOracle::run_cols` return the unclipped `(c, c + n)`: the fills
/// at 39x30 are (0,3,39,1) and (0,5,39,5) again.
#[test]
fn title_card_stays_between_the_side_art_on_the_host_path() {
    let Some(s) = title_card() else { return };
    for size in HOST_SIZES {
        let ring = host(&s, size);
        let who = format!("{}x{}@{}x{}", size.0, size.1, size.2, size.3);
        assert_between_the_art(&ring, &who);
        assert_eq!(chrome_text(&ring).len(), 9, "{who}: the card's nine lines");
        let lit = ring.runs.iter().find(|r| r.run.text.starts_with("Original Literary")).expect("literary line");
        assert_eq!(lit.run.text.chars().count(), 54, "{who}: the run keeps its whole text (clipped, never reflowed)");
        let (a, b) = lit.resolved.unwrap();
        assert!(b - a <= i32::from(ring.viewport.width), "{who}: the long line is clipped to the span ({a}..{b})");
    }
}

/// The reported 39x30 / 42x35 numbers, pinned: the box is the art-free span, every fill is
/// exactly that wide, and the card's long lines start at its left edge.
#[test]
fn reported_portrait_sizes_are_pinned() {
    let Some(s) = title_card() else { return };
    for (size, vp_x, vp_w) in [(HOST_SIZES[0], 3u16, 33u16), (HOST_SIZES[1], 4, 34)] {
        let ring = host(&s, size);
        assert_eq!((ring.viewport.x, ring.viewport.width), (vp_x, vp_w), "{size:?}: the box");
        let fills: Vec<_> = ring.ground.iter().filter(|g| g.rect.y < ring.viewport.y).collect();
        assert!(!fills.is_empty(), "{size:?}: non-vacuity: the card has a ground fill");
        for fill in fills {
            assert_eq!((fill.rect.x, fill.rect.width), (vp_x, vp_w), "{size:?}: a fill leaves the box: {:?}", fill.rect);
        }
        for text in ["Original Literary", "Licensed by", "Release 322"] {
            let r = ring.runs.iter().find(|r| r.run.text.starts_with(text)).expect(text);
            assert_eq!(r.resolved.map(|(a, _)| a), Some(i32::from(vp_x)), "{size:?}: {text} at the box's left edge");
        }
    }
}

/// The terminal ring, across the pane widths a narrow terminal uses: the cells left and
/// right of the art-free span on every card row hold the ornament (a drawn, non-blank,
/// non-letter cell). This is also the landscape question: at 71x16 the old full-width fill
/// (7,6,57,4) wiped the ornament on rows 6..9 and made the gap in the side art; the game
/// paints art there (the flank draws it from the native frame), so the gap was our fill.
///
/// FALSIFY as above: the ornament cells on the card rows become card text or blanks.
#[test]
fn title_card_rows_show_art_beside_the_text_in_the_terminal() {
    let Some(s) = title_card() else { return };
    let mut checked = 0;
    for cols in (40u16..=100).step_by(4) {
        let span = tui_span(&s, cols, 25);
        let buf = tui(&s, cols, 25);
        for y in 0..10u16 {
            for x in (0..span.x).chain(span.right()..cols) {
                let c = glyph(&buf, x, y);
                assert!(!c.is_ascii_alphanumeric(), "{cols} cols: card text {c:?} over the art at ({x},{y})");
            }
            // The ornament's outer margin is bare in the game's own frame, so ask for ART on
            // each side of the row, not in every cell.
            let art = |r: std::ops::Range<u16>| r.filter(|&x| glyph(&buf, x, y) != ' ').count();
            assert!(art(0..span.x) >= 2, "{cols} cols: no ornament left of the span on row {y} (span {span:?})");
            assert!(art(span.right()..cols) >= 2, "{cols} cols: no ornament right of the span on row {y} (span {span:?})");
            checked += 1;
        }
    }
    assert!(checked >= 150, "non-vacuity: {checked}");
}

/// The in-game score bar is between the art at EVERY width, on the host path (30..=100
/// columns at 25/40/70 rows, and the four host sizes) and in the terminal: one behaviour,
/// not one that flips with the rounding of the art's edge. Both lines read whole.
///
/// FALSIFY by deleting the `is_word_run` clamps in `chrome_run_origin` and `run_cols`: the
/// bar spans the art at 70 and 71 columns (and at 84..87 and 98..100 on the taller panes).
#[test]
fn the_score_bar_is_between_the_art_at_every_width() {
    let Some(s) = gameplay() else { return };
    let mut sizes: Vec<(u16, u16, u16, u16)> = HOST_SIZES.to_vec();
    for rows in [25u16, 40, 70] {
        for cols in 30..=100u16 {
            sizes.push((cols, rows, 8, 16));
        }
    }
    for size in sizes {
        let ring = host(&s, size);
        let who = format!("{}x{}@{}x{}", size.0, size.1, size.2, size.3);
        assert_between_the_art(&ring, &who);
        let bar = chrome_text(&ring);
        assert_eq!(bar.len(), 7, "{who}: the bar's seven fields: {:?}", bar.iter().map(|r| &r.run.text).collect::<Vec<_>>());
        for r in bar {
            let (a, b) = r.resolved.unwrap();
            assert_eq!(b - a, r.run.text.chars().count() as i32, "{who}: {:?} whole", r.run.text);
        }
    }
    for rows in [25u16, 40, 70] {
        for cols in (30..=100u16).step_by(5) {
            let span = tui_span(&s, cols, rows);
            let buf = tui(&s, cols, rows);
            for y in 0..2u16 {
                for x in (0..span.x).chain(span.right()..cols) {
                    let c = glyph(&buf, x, y);
                    assert!(!c.is_ascii_alphanumeric(), "{cols}x{rows}: bar text {c:?} over the art at ({x},{y})");
                }
            }
        }
    }
}

/// The hosts' own sizes plus the pane geometries a real terminal reaches: every width from
/// 14 to 100 columns (30..=100 is where the whole bar fits; below it fields go and the rest
/// clips on the right) at three heights (20, 30, 70 rows), at larger cell sizes, the pixel lock off and on.
fn bar_sizes() -> Vec<((u16, u16, u16, u16), bool)> {
    let mut v: Vec<_> = HOST_SIZES.iter().map(|&h| (h, false)).chain(HOST_SIZES.iter().map(|&h| (h, true))).collect();
    for rows in [20u16, 30, 70] {
        for cols in (14..=100u16).filter(|c| *c <= 40 || c % 3 == 0) {
            v.push(((cols, rows, 8, 16), false));
        }
    }
    for (cw, ch) in [(10u16, 20u16), (20, 46)] {
        for cols in 14..=40u16 {
            for lock in [false, true] {
                v.push(((cols, 30, cw, ch), lock));
            }
        }
    }
    v
}

/// The score bar is between the art on BOTH presses and at the small widths too (SQ-1826,
/// owner's re-check of 9ff89a6a3): the Amiga floppy's bar is edge-to-edge reverse-video
/// blanks that reach over the ornament at 28..=31 columns and below 23, and its fill with
/// them. Every width from 14 to 100 columns, three heights, three cell sizes, pixel lock on and
/// off: no field is drawn outside the art-free span and no fill leaves it. Where the whole bar
/// fits (36 columns and up, 8x16 cells, no lock) every field is whole.
///
/// The art is drawn down to 4 columns (one cell each side), so there is no width in this sweep
/// where the bar may use the full pane; below 4 columns the viewport has no room for it at all
/// (no flank, nothing to be between).
///
/// FALSIFY by making `ChromeRowOracle::run_cols` skip its blank-run clamp (restore the
/// `!is_word_run(t)` early return): the floppy's fill spans the pane at 28..=31 and 14..=22.
#[test]
fn the_score_bar_is_between_the_art_on_both_presses_at_every_size() {
    let (mut checked, mut booted) = (0, 0);
    for file in [Z6, ADF] {
        let Some(s) = gameplay_of(file) else { continue };
        booted += 1;
        for (size, lock) in bar_sizes() {
            let ring = host_locked(&s, size, lock);
            let who = format!("{file} {}x{}@{}x{} lock {lock}", size.0, size.1, size.2, size.3);
            let vp = ring.viewport;
            // The lock quantises the scale and big cells shrink the box: only the plain 8x16
            // sweep promises the whole bar from 36 columns.
            let fits = size.0 >= 36 && !lock && size.2 == 8;
            for r in chrome_text(&ring) {
                if let Some((a, b)) = r.resolved {
                    assert!(
                        a >= i32::from(vp.x) && b <= i32::from(vp.right()),
                        "{who}: {:?} drawn at {a}..{b}, outside the art-free span {}..{}",
                        r.run.text,
                        vp.x,
                        vp.right()
                    );
                    if fits {
                        assert_eq!(b - a, r.run.text.chars().count() as i32, "{who}: {:?} whole", r.run.text);
                    }
                } else {
                    assert!(!fits, "{who}: {:?} dropped where the bar fits", r.run.text);
                }
            }
            for g in ring.ground.iter().filter(|g| g.rect.y < vp.y) {
                assert!(g.rect.x >= vp.x && g.rect.right() <= vp.right(), "{who}: fill {:?} covers the art (span {vp:?})", g.rect);
            }
            checked += 1;
        }
    }
    // CI has no `stories/`: with neither fixture present there is nothing to check.
    if booted > 0 {
        assert!(checked > 400, "non-vacuity: {checked}");
    }
}

/// A title-card line that fits the art-free span lands where the scale puts it, as at 7f116e7ec:
/// merging the card's rows into one strip (the flank now keeps every row) had moved "SHOGUN"
/// three columns right of centre at 71x16, because a strip's block origin, column plan and
/// slides are laid out over its own rows and the overrunning lines were now among them. Pinned
/// at 71x16@27x60 (box 12..59) and at 60 and 65 columns on 8x16 cells, where nothing that fits
/// moved; only the overrunning lines (trademark, "Original", "Licensed", "Release" where they
/// do not fit) slide or clip.
///
/// FALSIFY by removing the split in `decompose_chrome_strips`: SHOGUN reads 40..46 at 71x16
/// and 28..34 at 65 columns.
#[test]
fn lines_that_fit_stay_where_the_scale_puts_them() {
    let Some(s) = title_card() else { return };
    let at = |size: (u16, u16, u16, u16), want: &[(&str, (i32, i32))]| {
        let ring = host(&s, size);
        for (text, span) in want {
            let r = ring.runs.iter().find(|r| r.run.text.starts_with(text)).expect(text);
            assert_eq!(r.resolved, Some(*span), "{size:?}: {text}");
        }
    };
    at(
        (71, 16, 27, 60),
        &[("SHOGUN", (37, 43)), ("A Story", (32, 48)), ("Copyright", (25, 54)), ("All rights", (30, 50)), ("IBM Interp", (26, 54))],
    );
    at(
        (60, 25, 8, 16),
        &[("SHOGUN", (32, 38)), ("A Story", (27, 43)), ("Copyright", (21, 50)), ("All rights", (25, 45)), ("SHOGUN is", (16, 54)), ("Release", (11, 55)), ("IBM Interp", (20, 48))],
    );
    at(
        (65, 25, 8, 16),
        &[("SHOGUN", (33, 39)), ("A Story", (28, 44)), ("Copyright", (22, 51)), ("All rights", (26, 46)), ("SHOGUN is", (17, 55)), ("Release", (15, 59)), ("IBM Interp", (23, 51))],
    );
}
