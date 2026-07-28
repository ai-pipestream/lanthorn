//! SQ-0533: a LIVE upper window follows the host screen width.
//!
//! ZMSD §8.4 lets the interpreter "change the exact dimensions whenever it
//! likes" so long as it writes the new height/width into header bytes $20/$21.
//! The wave-2 pane feed does exactly that, but only `split_window` ever sized
//! the upper grid from $21 — so a v4/v5 game that splits ONCE at boot and never
//! re-splits (Sherlock, Trinity) kept its boot-time grid width for the rest of
//! the session and its status bar was clipped in a widened pane. AMFV
//! self-healed only because it re-splits on every mode change.
//!
//! These tests take the same route the app does at runtime
//! (`loop_tick::poll_zvm_screen_dims` → `Engine::set_screen_dims`) and check the
//! result all the way through to the RENDERED grid row.
//!
//! The real-story cases use the skip-if-missing pattern of the other
//! gitignored-story smokes; the synthetic v5 case always runs.

use std::path::PathBuf;

use app::colors::ColorScheme;
use app::engine::{Engine, GridWindow, WinNode};
use app::render::paneframe::{BorderStyle, PaneSides};
use app::render::upper_window::draw_upper_window;
use app::session::{screen_model_from_machine, GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(file: &str) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, Some((2, 9)))
            .expect("story should load and boot without a ZError"),
    )
}

/// Drive `n` turns, feeding `cmds` to line prompts and Enter to char prompts.
fn drive(s: &mut GameSession, cmds: &[&str], n: usize, who: &str) {
    let mut ci = 0;
    for t in 0..n {
        let r = match s.pending_input() {
            InputKind::Line => {
                let c = cmds.get(ci).copied().unwrap_or("look");
                ci += 1;
                s.submit(c)
            }
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "{who} faulted on turn {t}: {:?}", r.fault);
        if r.quit {
            break;
        }
    }
}

/// The upper window as the app's renderer receives it.
fn model_grid(s: &GameSession) -> GridWindow {
    match screen_model_from_machine(&s.machine).root {
        WinNode::Pair { first, .. } => match *first {
            WinNode::Grid(g) => g,
            other => panic!("expected a Grid as the pair's first child, got {other:?}"),
        },
        other => panic!("expected a Pair root, got {other:?}"),
    }
}

/// Render `grid` into a `pane_w`-wide pane (no border chrome, so the grid's own
/// columns map 1:1 onto buffer cells) and return row 0 as a string.
fn render_row0(grid: &GridWindow, pane_w: u16) -> String {
    let mut colors = ColorScheme::terminal_default();
    colors.virtual_window_border = BorderStyle::None;
    colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);
    let area = Rect::new(0, 0, pane_w, 6);
    let mut buf = Buffer::empty(area);
    let used = draw_upper_window(grid, false, &colors, area, &mut buf, true, &mut Vec::new());
    assert!(used > 0, "the grid must consume at least one pane row");
    (0..pane_w).map(|x| buf.cell((x, 0)).unwrap().symbol().to_string()).collect()
}

/// The span of `row` that actually carries the grid, i.e. from the first to the
/// last non-blank column, plus how wide the grid claims to be.
fn drawn_span(row: &str) -> (usize, usize) {
    let first = row.find(|c: char| c != ' ').unwrap_or(0);
    let last = row.rfind(|c: char| c != ' ').unwrap_or(0);
    (first, last)
}

// ── The defect, on the games that exhibit it ─────────────────────────────────

/// Sherlock (v5) splits its status window once at boot and never re-splits.
/// After a widen the model grid — and the rendered row — must span the new pane
/// width, with the status text still there.
#[test]
fn sherlock_upper_grid_follows_a_mid_game_widen() {
    let Some(mut s) = boot("sherlock-r26-s880127.z5") else { return };
    assert_eq!(s.machine.mem.version(), 5);

    Engine::set_screen_dims(&mut s, 24, 80);
    drive(&mut s, &["look", "wait"], 3, "Sherlock");
    let before = model_grid(&s);
    assert!(before.rows > 0, "Sherlock splits an upper window at boot");
    assert_eq!(before.cols, 80, "the boot-time grid is the boot-time screen width");

    // The terminal (and so the story pane) got wider. This is exactly what
    // `loop_tick::poll_zvm_screen_dims` does once $20/$21 disagree with the pane.
    Engine::set_screen_dims(&mut s, 24, 100);

    let after = model_grid(&s);
    assert_eq!(after.cols, 100, "the LIVE grid follows the new width without a re-split");
    assert_eq!(after.rows, before.rows, "the split height stays the game's");

    // Mark the grid's brand-new last column: it only exists at all if the grid
    // widened, and it can only render at the pane's last cell if the whole
    // 100-column grid is drawn flush (an 80-column grid would be centred, an
    // island 10 cells in from each edge).
    s.machine.screen.upper.put(
        1, 100, '#', 0,
        zvm::screen::ZColour::Default, zvm::screen::ZColour::Default,
    );
    let row = render_row0(&model_grid(&s), 100);
    assert_eq!(row.chars().count(), 100);
    assert!(
        row.contains("Baker Street"),
        "the status text survives the widen: {row:?}"
    );
    assert_eq!(
        drawn_span(&row).1, 99,
        "the widened grid is drawn flush to the pane's last column: {row:?}"
    );

    // …and the game keeps playing at the new width.
    drive(&mut s, &["look", "wait"], 2, "Sherlock after widen");
    assert_eq!(model_grid(&s).cols, 100, "still 100 wide after further turns");
}

/// Trinity (v4) is the other single-split title. Same rule, and the shrink half:
/// the grid truncates to the narrower screen rather than overflowing it.
#[test]
fn trinity_upper_grid_follows_a_mid_game_shrink() {
    let Some(mut s) = boot("trinity-r12-s860926.z4") else { return };
    assert_eq!(s.machine.mem.version(), 4);

    Engine::set_screen_dims(&mut s, 24, 80);
    drive(&mut s, &["look", "wait"], 3, "Trinity");
    let before = model_grid(&s);
    assert!(before.rows > 0, "Trinity splits an upper window at boot");
    assert_eq!(before.cols, 80);

    Engine::set_screen_dims(&mut s, 24, 60);

    let after = model_grid(&s);
    assert_eq!(after.cols, 60, "the LIVE grid follows the narrower screen");
    assert_eq!(after.rows, before.rows, "the split height stays the game's");
    let row = render_row0(&after, 60);
    assert_eq!(row.chars().count(), 60, "the rendered row fits the narrower pane");
    // Whatever the leftmost field was, it survived the truncation.
    let before_head: String = (1..=20).map(|c| before.cell(1, c).ch).collect();
    let after_head: String = (1..=20).map(|c| after.cell(1, c).ch).collect();
    assert_eq!(after_head, before_head, "the surviving columns are preserved verbatim");

    drive(&mut s, &["look", "wait"], 2, "Trinity after shrink");
}

// ── Synthetic v5: the same path with no gitignored story ─────────────────────

/// A minimal v5 story whose program splits a 1-row upper window and then blocks
/// on `read` — the "split once at boot, never again" shape of Sherlock/Trinity,
/// with none of their content. Header layout mirrors zvm's own crate-private
/// `header::tests_support::sample_story` (see `inventory.rs`'s shim).
fn split_once_v5_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x1000];
    buf[0x00] = 5; // version
    buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
    buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
    buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary   = 0x0200
    buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
    buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars  = 0x0300
    buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
    buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060

    // Dictionary at 0x0200: 0 word-separators, entry-size 4, 0 entries.
    buf[0x0200] = 0;
    buf[0x0201] = 4;
    buf[0x0202] = 0; buf[0x0203] = 0;

    // Text buffer at 0x0380: max 20 chars.
    buf[0x0380] = 20;

    // Program at 0x0040:
    //   split_window 1   VAR:0x0A → 0xEA, types (small, ---) = 0x7F, 0x01
    //   set_window   1   VAR:0x0B → 0xEB, types (small, ---) = 0x7F, 0x01
    //   read text        VAR:0x04 → 0xE4, types (large, ---) = 0x3F, 0x0380, store→stack
    let prog: [u8; 11] = [
        0xEA, 0x7F, 0x01,
        0xEB, 0x7F, 0x01,
        0xE4, 0x3F, 0x03, 0x80, 0x00,
    ];
    buf[0x0040..0x0040 + prog.len()].copy_from_slice(&prog);
    buf
}

/// The end-to-end rule with no story file needed: split at 80, widen to 100 →
/// the model grid AND the rendered row are 100 wide with the painted status
/// text preserved left-aligned; shrink to 60 → truncated.
#[test]
fn synthetic_v5_upper_grid_follows_the_pane_without_a_re_split() {
    let mut s = GameSession::new_with_trace(
        split_once_v5_story(), true, false, None, false, Vec::new(), None, Some((2, 9)),
    )
    .expect("the synthetic v5 story should boot");
    assert!(!s.quit, "the synthetic story parks on `read`, it does not quit");
    assert!(matches!(s.pending_input(), InputKind::Line));

    Engine::set_screen_dims(&mut s, 24, 80);
    let grid = model_grid(&s);
    assert_eq!((grid.rows, grid.cols), (1, 80), "the game's single split, at the boot width");

    // Paint a status line, as the game would with the upper window selected.
    for (i, ch) in "SYNTHETIC HALL".chars().enumerate() {
        s.machine.screen.upper.put(
            1, i as u16 + 1, ch, 0,
            zvm::screen::ZColour::Default, zvm::screen::ZColour::Default,
        );
    }

    // Widen: no re-split happens (the story is parked on `read`).
    Engine::set_screen_dims(&mut s, 24, 100);
    let wide = model_grid(&s);
    assert_eq!(wide.cols, 100, "the live grid widened");
    assert_eq!(wide.rows, 1, "the split height is untouched");
    let row = render_row0(&wide, 100);
    assert_eq!(row.chars().count(), 100, "the rendered row spans the whole new width");
    assert!(row.starts_with("SYNTHETIC HALL"), "content preserved left-aligned: {row:?}");
    assert_eq!(drawn_span(&row).1, 13, "the grown columns are blank, not clipped chrome");

    // Shrink past the text: truncation, and the cursor comes back in range.
    s.machine.screen.cursor_row = 1;
    s.machine.screen.cursor_col = 90;
    Engine::set_screen_dims(&mut s, 24, 10);
    let narrow = model_grid(&s);
    assert_eq!(narrow.cols, 10, "the live grid narrowed");
    let row = render_row0(&narrow, 10);
    assert_eq!(row, "SYNTHETIC ", "the row truncates to the new width");
    assert_eq!(
        s.machine.screen.cursor_col, 10,
        "§8.7.2.3: an out-of-window cursor is illegal, so it clamps to the last column"
    );
    assert_eq!(s.machine.screen.cursor_row, 1, "the row the game set is kept");
}
