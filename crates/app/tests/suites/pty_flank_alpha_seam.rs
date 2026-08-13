//! SQ-0827 — no band babelmap uploads may carry a colour the game never drew.
//!
//! Reported by eye on the Amiga Zork Zero floppy (**release 366 / serial 890323**),
//! measured off a 583x850 screenshot: a **one pixel** wide line down both edges of
//! the story pane, RGB(130,130,130) against the page's RGB(163,163,163), which went
//! away when the terminal was made wider.
//!
//! One pixel is sub-cell, so the line could not be a background run; and it moved
//! with the pane, so it had to be a resample. A bisect put it on `e39fc95f`
//! (SQ-0824), which taught the art paths to shrink through an area filter — and an
//! area filter averages the four RGBA channels independently. Zork Zero's flank ends
//! in the story page abutting CLEAR canvas, so the filter averaged the page with the
//! `(0,0,0)` a transparent pixel carries and dropped the alpha to match: the band
//! went out with `(38,38,38,57)` at the seam, which over the page the flank is drawn
//! on composites to 142 against that page's own 173. `resize_directional` now
//! associates alpha across a blending pass, which is what this suite is here to keep.
//!
//! The invariant is stated the way the report was — a one-pixel column, at the seam,
//! darker than the page either side of it — and measured on the screen a real terminal
//! emulator resolves from babelmap's own bytes, because a defect one pixel wide inside
//! an image is invisible to every cell-buffer harness in the tree. An earlier draft
//! judged the emitted image instead, asking that a partly transparent pixel lie between
//! the opaque pixels touching it; that is not an invariant a shrink obeys, because
//! adjacent OUTPUT pixels are not the source pixels either was resampled from, and it
//! failed on ordinary pillar art.
//!
//! Both `honor_game_colours` modes, per the project's colour-render convention — with
//! the honest note that only the honouring one FALSIFIES. With the fix reverted the
//! default mode fails naming `x=84` and `x=496`, 142 against 173, and the declining
//! mode passes: declining leaves the flank's inner columns clear rather than flooding
//! them with the game's opaque page, so the seam has no opaque neighbour to be averaged
//! with and the defect never had anything to bite on there. The second case is a guard
//! against a future fix that trades one mode for the other, which is what pinning both
//! modes is for. `stories/` is gitignored, so both skip vacuously without the floppy.

use std::path::PathBuf;
use std::time::Duration;

// Declared once by the group binary (`tests/pty.rs`) and shared by every pty suite
// in it.
use super::pty_stream::{self, driver, oracle};

/// The Amiga floppy, and the build the report was made on.
const STORY: &str = "Zork Zero - The Revenge of Megaboz.adf";

/// The user's own terminal, to within the two pixels of window chrome their
/// screenshot carried: 83x50 cells of 7x17 makes a 581x850 frame against their
/// 583x850. At this width the flanks resample 95x806 native pixels down to 84x714,
/// which is the shrinking regime the defect lives in; past ~95 columns they grow
/// instead, take the Nearest arm, and the line was never there.
const COLS: u16 = 83;
const ROWS: u16 = 50;
const CELL: (u16, u16) = (7, 17);

/// How much darker than the page either side of it a seam column may read: the 8-bit
/// premultiply round trip costs one level, and nothing else may. The reported line was
/// worth 31 (142 against 173 in our own capture of that frame; 130 against 163 through
/// the colour profile of the user's screenshot).
const SLACK: i32 = 4;

/// How far either side of a flank's inner edge to look, in cells. The seam is at the
/// edge itself; two cells of margin catch it wherever the rounding puts it.
const WINDOW_CELLS: u32 = 2;

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The colour of screen column `x` over rows `y0..y1`, when every pixel of it is the
/// same — a column that varies is text or art, and says nothing about a seam.
fn uniform_column(screen: &image::RgbaImage, x: u32, y0: u32, y1: u32) -> Option<[u8; 3]> {
    let first = screen.get_pixel(x, y0).0;
    (y0..y1)
        .all(|y| screen.get_pixel(x, y).0 == first)
        .then_some([first[0], first[1], first[2]])
}

/// A one-pixel column, uniform down the flank, darker than the uniform columns either
/// side of it: the reported line, stated as it was measured. Returns the first one.
fn dark_seam_column(screen: &image::RgbaImage, xs: std::ops::Range<u32>, y0: u32, y1: u32) -> Option<String> {
    for x in xs {
        let (Some(here), Some(before), Some(after)) = (
            uniform_column(screen, x, y0, y1),
            uniform_column(screen, x - 1, y0, y1),
            uniform_column(screen, x + 1, y0, y1),
        ) else {
            continue;
        };
        if before != after {
            continue; // a genuine edge between two colours, not a line drawn on one
        }
        let drop = (0..3).map(|c| i32::from(before[c]) - i32::from(here[c])).max().unwrap_or(0);
        if drop > SLACK {
            return Some(format!(
                "screen column x={x} is {here:?} down all of rows {y0}..{y1}, against the \
                 {before:?} of the columns either side of it — a line ONE PIXEL wide, {drop} \
                 levels darker than the page it sits in, at the seam where the flank art \
                 meets the story pane (SQ-0827)"
            ));
        }
    }
    None
}

/// One capture at one colour policy.
fn no_one_pixel_line_at_the_flank_seam(honor: bool) {
    let story = driver::stories_dir().join(STORY);
    if !story.is_file() {
        eprintln!("SKIP: gitignored story missing at {}", story.display());
        return;
    }
    let user_dir = out_dir().join(format!("flank-seam-honor-{honor}"));
    let _ = std::fs::remove_dir_all(&user_dir);
    std::fs::create_dir_all(&user_dir).expect("a throwaway babelmap home");
    // The driver writes the per-game sidecar itself; the colour policy is a global
    // key, and babelmap reads the file rather than reseeding it when it exists.
    std::fs::write(user_dir.join("config.toml"), format!("honor_game_colours = {honor}\n"))
        .expect("seeding the colour policy");

    let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_babelmap"), &story, &user_dir);
    spec.cols = COLS;
    spec.rows = ROWS;
    spec.cell_w = CELL.0;
    spec.cell_h = CELL.1;
    // Two returns past the title into the castle frame, which is where the pillars
    // and the page they abut are both on screen.
    spec.keys = vec![
        driver::Key::Wait(Duration::from_millis(2500)),
        driver::Key::Bytes(b"\r".to_vec()),
        driver::Key::Wait(Duration::from_millis(1500)),
        driver::Key::Bytes(b"\r".to_vec()),
        driver::Key::Wait(Duration::from_millis(1500)),
    ];

    let cap = driver::run(spec).expect("the pty harness should boot babelmap");
    let neg = cap.negotiated();
    assert!(
        neg.is_kitty(),
        "the capture must exercise the kitty path or it measures the half-block backend \
         and every pixel in it is worthless: {}",
        neg.explain()
    );
    let res = oracle::resolve(
        &cap.bytes,
        cap.spec.cols,
        cap.spec.rows,
        u32::from(cap.spec.cell_w),
        u32::from(cap.spec.cell_h),
    );
    // The flanks: the tallest images on the screen, and the ones that shrink.
    let flanks: Vec<_> = res.placements.iter().filter(|p| p.bottom - p.top > 20).collect();
    assert!(
        flanks.len() >= 2,
        "honor={honor}: this frame must carry both side flanks as images, or the seam \
         they make with the page is not on screen at all — found {}\n{}",
        flanks.len(),
        res.describe_placements()
    );

    let screen = pty_stream::raster::render(&res);
    let (cw, ch) = (u32::from(cap.spec.cell_w), u32::from(cap.spec.cell_h));
    let win = WINDOW_CELLS * cw;
    let mut failures = Vec::new();
    for f in &flanks {
        // Down the flank, a cell in from each end so the corners it shares with the
        // top plate and the bottom strip cannot speak for it.
        let (y0, y1) = ((u32::from(f.top) + 1) * ch, (u32::from(f.bottom)) * ch);
        // Its INNER edge — the one the story pane is on. A flank in the left half of
        // the screen faces right, and the other faces left.
        let inner = if u32::from(f.left) * cw < screen.width() / 2 {
            (u32::from(f.right) + 1) * cw
        } else {
            u32::from(f.left) * cw
        };
        let xs = inner.saturating_sub(win).max(1)..(inner + win).min(screen.width() - 1);
        if let Some(why) = dark_seam_column(&screen, xs, y0, y1) {
            failures.push(why);
        }
    }
    // The picture and the report are written whether or not it failed, so a future
    // failure has the frame beside it.
    let term = pty_stream::decode_capture(&cap);
    let _ = std::fs::write(
        out_dir().join(format!("zork0-r366-flank-seam-honor-{honor}.txt")),
        pty_stream::report(&cap, &term),
    );
    let _ = screen.save(out_dir().join(format!("zork0-r366-flank-seam-honor-{honor}.png")));
    assert!(failures.is_empty(), "honor={honor}: {}", failures.join("\n"));
}

#[test]
fn no_line_down_the_amiga_story_panes_edges() {
    no_one_pixel_line_at_the_flank_seam(true);
}

#[test]
fn no_line_down_the_amiga_story_panes_edges_with_the_games_colours_declined() {
    no_one_pixel_line_at_the_flank_seam(false);
}
