//! SQ-1796: a dialog opened over a screen that is ALL ARTWORK must be seen.
//!
//! Reported in Ghostty on Kerkerkruip's opening menu: one Glk graphics window fills
//! the story pane, there is no prompt (so `/` is unavailable) and, with the map
//! panel off, Ctrl-P opened the hotkey dialog — its keys ran commands — but nothing
//! appeared. The first guess was kitty drawing its image above the dialog's cells.
//! It is not that: this app's kitty images are unicode-placeholder CELLS, so a
//! dialog written over them simply replaces them. The cause was `dialog_bounds`,
//! which carves graphics windows out of the region a dialog may use and, with the
//! art filling the pane, left a 2-row sliver that `draw_hotkey_dialog` declines to
//! draw into.
//!
//! This is the emitted-stream layer on purpose: the cell-buffer harness cannot tell
//! the dialog being DRAWN from the dialog being drawn and then covered, and the
//! oracle (the stream through a real terminal core) can. The fixture is fetched
//! (`scripts/fixtures.manifest`), so this runs on CI; it skips when it is absent.
//!
//! Layout fact the guard relies on: at 80x28 the art is a placeholder field, and
//! `oracle` reports `image_id` only for cells a terminal would draw image pixels in.

#[cfg(not(unix))]
#[test]
fn the_dialog_over_art_test_is_unix_only() {
    eprintln!("SKIP: driving a real terminal needs a pty, which this platform does not have");
}

#[cfg(unix)]
use std::cell::Cell;
#[cfg(unix)]
use std::rc::Rc;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use super::pty_stream::{self, driver, oracle};

#[cfg(unix)]
use crate::fixture_paths::fixture_path;

#[cfg(unix)]
const COLS: u16 = 80;
#[cfg(unix)]
const ROWS: u16 = 28;

#[cfg(unix)]
fn row_text(res: &oracle::Resolved, row: u16) -> String {
    (0..res.cols).map(|c| res.cell(row, c).ch).collect()
}

/// Ctrl-P over Kerkerkruip's all-art opening menu, map panel hidden (the harness's
/// default): the dialog's title must be on the terminal's screen.
#[cfg(unix)]
#[test]
fn hotkey_dialog_shows_over_an_all_art_screen_under_kitty() {
    let story = fixture_path("Kerkerkruip.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: Kerkerkruip.gblorb absent at {}", story.display());
        return;
    }
    let user_dir = app::scratch_dir("sq1796-dialog-over-art");
    let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
    spec.cols = COLS;
    spec.rows = ROWS;
    // The menu ANIMATES, so the pty is never quiet for the harness's usual 120 ms and a
    // `Key::Wait` (which waits for quiet) would never release the keystroke. Stage by
    // content instead: art flowing -> Ctrl-P -> a few more frames -> end on the ceiling.
    spec.quiet = Duration::ZERO;
    let booted_at = Rc::new(Cell::new(usize::MAX));
    let (mark, check) = (booted_at.clone(), booted_at.clone());
    spec.keys = vec![
        driver::Key::WaitUntil {
            label: "the menu's art flowing",
            ready: Rc::new(move |bytes: &[u8]| {
                let up = bytes.len() > 400_000;
                if up && mark.get() == usize::MAX {
                    mark.set(bytes.len());
                }
                up
            }),
            cap: Duration::from_secs(20),
        },
        driver::Key::Bytes(vec![0x10]),
        driver::Key::WaitUntil {
            label: "frames drawn after Ctrl-P",
            ready: Rc::new(move |bytes: &[u8]| bytes.len() > check.get().saturating_add(300_000)),
            cap: Duration::from_secs(20),
        },
    ];
    spec.timeout = Duration::from_secs(25);
    let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
    let neg = cap.negotiated();
    assert!(neg.is_kitty(), "the capture must exercise the kitty path: {}", neg.explain());
    let res = oracle::resolve(
        &cap.terminal_bytes(),
        cap.spec.cols,
        cap.spec.rows,
        u32::from(cap.spec.cell_w),
        u32::from(cap.spec.cell_h),
        Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
    );
    let _ = std::fs::remove_dir_all(&user_dir);

    // Non-vacuity: the art is on screen, in the story pane's corner cell, so this is
    // the all-graphics shape the report is about and not a text screen.
    let art_cells = (0..ROWS).flat_map(|r| (0..COLS).map(move |c| (r, c))).filter(|&(r, c)| res.cell(r, c).image_id.is_some()).count();
    assert!(art_cells > 300, "no art on screen ({art_cells} image cells): the case is not exercising an all-art screen");
    assert!(res.cell(2, 2).image_id.is_some(), "the story pane's corner is not covered by the game's art");

    let screen: Vec<String> = (0..ROWS).map(|r| row_text(&res, r)).collect();
    assert!(
        screen.iter().any(|l| l.contains("Commands (")),
        "the hotkey dialog is open but its title is nowhere on the terminal's screen:\n{}",
        screen.join("\n")
    );
}
