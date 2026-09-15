//! SQ-1514: Kerkerkruip's clickable UI — its in-game menus and the "[detailed
//! status report]" link in its statistics panel — did nothing at all under the
//! mouse, while typing the equivalent command worked.
//!
//! # What the game actually does
//!
//! Traced off the real archive (`stories/Kerkerkruip.gblorb`, Kerkerkruip 9.0.1,
//! IFID AC0DAF65-F40F-4A41-A4E4-50414F836E14) with its shipped `Kerkerkruip.ini`
//! beside it — which is the opt-in to this whole presentation, see
//! `glulx_garglk_style_sentinel.rs`. In ordinary play the game runs a
//! **twenty-one-window** layout: a 60-column story buffer in the middle, a
//! one-row status grid above it, side panels left (Inventory) and right
//! (Statistics, Powers), and a graphics window for every rule between them. Its
//! screen trace says, in order:
//!
//! ```text
//! glk_window_open(4, 33, 25, 3, 240) -> win 7    // the Statistics panel
//! glk_set_hyperlink(1)                            // "[detailed status report]"
//! glk_set_hyperlink(0)
//! glk_request_hyperlink_event(7)
//! ```
//!
//! …and eight windows hold a standing hyperlink request at the parser prompt
//! (`hyperlink_windows() == [4, 7, 13, 17, 23, 27, 35, 41]`). **Exactly one of
//! them is the primary buffer.** The stats panel is win 7, and its first line is
//! `"You are human. [detailed status report]"` with a run over chars 16..38
//! carrying link 1. Typing HELP replaces the panels with a full-pane menu in
//! non-primary buffer **win 47**, whose nine choices carry links 49..56 and 81 —
//! so the reported "clickable menus do nothing" and the reported dead stats link
//! are one bug in two places, not two bugs.
//!
//! # The drop point
//!
//! `render::screen::render_node`'s `WinNode::Buffer` arm forks on `b.primary`:
//! the primary window goes to `render_transcript`, which returns its link cells
//! in `StoryPaneMetrics::links`, and **every other buffer window** went to
//! `render_inline_buffer`, which drew the styled runs and recorded nothing. So
//! the frame's cell→link map came back EMPTY on a screen with nine live links on
//! it, and `main.rs`'s hyperlink arm — which looks the clicked cell up in that
//! map before anything else — never fired. Nothing else was wrong: gvm stamped
//! the links, `AppGlk` carried them onto the runs, and the window's drawn rect
//! was recorded in `win_rects`, so `glk_hyperlink_window` would have resolved the
//! click the instant it was asked. It was never asked.
//!
//! Fixed by `render::transcript::record_run_links`, the transcript's own
//! char-offset→display-column recording lifted into one function and called from
//! `render_inline_buffer` as well — the same "one recorder, every route" shape
//! `record_band_links` took for SQ-1503's pictures, which is the identical defect
//! one level out.
//!
//! The GRAPHICAL main menu ("New Game / Help / Options / Quit") is not affected
//! and never was: it is a real Glk graphics window with
//! `glk_request_mouse_event`, so it travels `glk_mouse_target` →
//! `deliver_mouse`, a path this quest does not touch. Checked, not assumed —
//! `the_graphical_main_menu_click_was_never_the_broken_path` below drives it.
//!
//! # Fixture and how the frames below are reached
//!
//! `stories/Kerkerkruip.gblorb` plus `stories/Kerkerkruip.ini`, both gitignored,
//! so every case here skips vacuously without them (and on CI, which has
//! neither). The game boots into a timer-driven intro animation rather than to a
//! prompt, so [`into_gameplay`] delivers Glk timer ticks until the game asks for
//! input (~110 at these dimensions), presses the spacebar its title card asks
//! for, and ticks again until the parser prompt (~20 more) — a self-paced loop
//! with a cap, not a pinned tick count, because the count is the game's
//! animation and not a fact worth pinning. Needs no save, no VFS sidecar and no
//! `game_dir`: this is a fresh install reaching its own first turn.

use std::path::{Path, PathBuf};

use app::engine::{Engine, WinNode};
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;
use app::session::InputKind;
use app::state::{AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

const STORY: &str = "Kerkerkruip.gblorb";
/// The pane the frames below are measured at. Wide enough for the game to lay
/// out its side panels, which is the whole shape this quest is about.
const COLS: u16 = 120;
const ROWS: u16 = 40;
/// Cap on the intro's timer ticks — generous; the real counts are ~110 and ~20.
const TICK_CAP: usize = 4000;

/// The story, and the ini that turns its clickable presentation on, or `None`.
fn story_path() -> Option<PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: gitignored fixture missing at {}", p.display());
        return None;
    }
    if !p.with_file_name("Kerkerkruip.ini").is_file() {
        eprintln!("SKIP: this suite needs the shipped Kerkerkruip.ini beside the story");
        return None;
    }
    Some(p)
}

/// The theme colours the app pushes into the Glk backend for the story at `path`
/// — the real chain `startup.rs` runs: a scheme, the garglk.ini beside the story
/// overlaid, then the per-Glk-style pairs. Same helper as
/// `glulx_garglk_style_sentinel`; without it the game takes its plain
/// screen-reader branch and never builds the panelled UI at all.
fn theme_pairs_for(path: &Path) -> GlkStylePairs {
    let mut cs = app::colors::ColorScheme::default();
    if let Some(ov) = app::garglk_ini::discover(path) {
        ov.apply(&mut cs);
    }
    app::glk_backend::theme_style_colours(&cs)
}

fn boot(path: &Path) -> GlulxSession {
    let bytes = std::fs::read(path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    GlulxSession::new_in(
        PathBuf::new(), // no persistent store: a fresh, never-played install
        image,
        COLS as u32,
        ROWS as u32,
        true,  // acceleration
        true,  // graphics (the panels' rules are graphics windows)
        false, // sound
        false, // borderless
        (8, 16),
        Some(blorb),
        &[], // no VFS sidecar
        theme_pairs_for(path),
        false,
        None,
    )
    .expect("Kerkerkruip boots")
}

/// Deliver Glk timer ticks until the game asks for input (its intro is a
/// timer-driven animation, not a prompt). Returns how many it took.
fn tick_to_input(sess: &mut GlulxSession) -> usize {
    let mut n = 0;
    while Engine::pending_input(sess) == InputKind::Event && n < TICK_CAP {
        let _ = sess.deliver_timer();
        n += 1;
    }
    n
}

/// Lit pixels in the canvas strip the main menu paints its `New Game / Help /
/// Options / Quit` bar into (y 40..76 of the 960x624 canvas): zero until the
/// intro animation gets that far, ~3200 once it has. The intro asks for a
/// keypress at the sword splash well BEFORE it paints the bar, so a
/// tick-until-input drive stops short of the menu frame — this is what tells the
/// two apart.
fn menu_bar_pixels(sess: &GlulxSession) -> usize {
    fn graphics(node: &WinNode) -> Option<&app::engine::GraphicsWindow> {
        match node {
            WinNode::Graphics(g) => Some(g),
            WinNode::Pair { first, second, .. } => graphics(first).or_else(|| graphics(second)),
            _ => None,
        }
    }
    let model = Engine::screen(sess);
    let Some(g) = graphics(&model.root) else { return 0 };
    (40..76)
        .flat_map(|y| (0..g.canvas.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            y < g.canvas.height() && {
                let p = g.canvas.get_pixel(x, y).0;
                p[0] as u32 + p[1] as u32 + p[2] as u32 > 60
            }
        })
        .count()
}

/// Tick until the menu bar is painted (or the cap runs out), and report how many
/// of its pixels are lit.
fn tick_to_menu_bar(sess: &mut GlulxSession) -> usize {
    for _ in 0..TICK_CAP {
        let lit = menu_bar_pixels(sess);
        if lit > 0 {
            return lit;
        }
        let _ = sess.deliver_timer();
    }
    menu_bar_pixels(sess)
}

/// Boot and reach the parser prompt of a real dungeon: tick through the intro,
/// press the spacebar its title card asks for, tick through the deal.
fn into_gameplay() -> Option<GlulxSession> {
    let mut sess = boot(&story_path()?);
    tick_to_input(&mut sess);
    let _ = Engine::submit_key(&mut sess, app::engine::KeyInput::Char(' '));
    tick_to_input(&mut sess);
    assert_eq!(
        Engine::pending_input(&sess),
        InputKind::Line,
        "the intro should have reached the parser prompt; window layout was:\n{}",
        Engine::window_dump(&sess).join("\n")
    );
    Some(sess)
}

/// A headless `AppState` for the render — the game's own colours honoured, which
/// is the shipped default.
fn render_state() -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.focus = Focus::Game;
    state
}

fn render(sess: &GlulxSession, state: &AppState) -> app::render::screen::StoryPaneMetrics {
    let area = Rect::new(0, 0, COLS, ROWS);
    let mut buf = Buffer::empty(area);
    app::render::screen::render_story_pane(&Engine::screen(sess), false, None, state, area, &mut buf)
}

/// The window with id `want`, if it is a buffer in the current tree.
fn buffer_window(node: &WinNode, want: u32) -> Option<&app::engine::BufferWindow> {
    match node {
        WinNode::Buffer(b) if b.win == want => Some(b),
        WinNode::Pair { first, second, .. } => buffer_window(first, want).or_else(|| buffer_window(second, want)),
        _ => None,
    }
}

/// Every `(line index, text, link)` triple of a buffer window's linked runs.
fn linked_lines(b: &app::engine::BufferWindow) -> Vec<(usize, String, u32)> {
    b.runs
        .iter()
        .enumerate()
        .flat_map(|(i, rl)| {
            rl.iter().filter(|r| r.link != 0).map(move |r| (i, b.lines[i].clone(), r.link))
        })
        .collect()
}

/// Run the two calls `main.rs`'s hyperlink arm makes, for the first cell in the
/// frame's map carrying `link`: resolve the owning window, then deliver the
/// event. Panics — with the empty map that IS the reported symptom — if the
/// click finds no recorded link.
fn click_link(sess: &mut GlulxSession, m: &app::render::screen::StoryPaneMetrics, link: u32) -> u32 {
    let &((col, row), v) = m.links.iter().find(|&&(_, v)| v == link).unwrap_or_else(|| {
        panic!(
            "a click on the drawn link {link} must find it in the frame's cell→link map; \
             got {:?}",
            m.links
        )
    });
    let windows = sess.hyperlink_windows();
    let win = app::glulx_session::glk_hyperlink_window(false, col, row, (0, 0, COLS, ROWS), &windows, &m.win_rects)
        .unwrap_or_else(|| {
            panic!(
                "the click at ({col},{row}) must resolve to a hyperlink-watching window; \
                 windows={windows:?} win_rects={:?}",
                m.win_rects
            )
        });
    let _ = sess.deliver_hyperlink(win, v);
    win
}

// ── The reported bug, at the two places it was reported ──────────────────────

/// The statistics panel's "[detailed status report]" link: clicked through the
/// app's real delivery path, the game must switch the panel to its detailed
/// view — the same thing the link's text promises.
///
/// Reverted (drop `record_run_links` from `render_inline_buffer`), this fails in
/// `click_link` with `got []` — an empty cell→link map on a screen with nine
/// live links, which is the report exactly.
#[test]
fn the_statistics_panels_detailed_status_report_link_answers_a_click() {
    let Some(mut sess) = into_gameplay() else { return };

    // Non-vacuity, and the mechanism read off the game rather than assumed: the
    // stats panel is a NON-PRIMARY buffer, it is hyperlink-armed, and its first
    // line really is the reported link.
    let model = Engine::screen(&sess);
    let stats = buffer_window(&model.root, 7).expect("Kerkerkruip's Statistics panel is Glk window 7");
    assert!(!stats.primary, "the panel is a non-primary buffer — the window class this quest is about");
    let armed = sess.hyperlink_windows();
    assert!(armed.contains(&7), "the panel must hold a standing hyperlink request; armed: {armed:?}");
    assert!(
        armed.iter().filter(|&&w| w != 4).count() >= 4,
        "the shape of the bug: most of this game's hyperlink-watching windows are NOT the \
         primary buffer, so recording links only there covers almost none of its UI; armed: {armed:?}"
    );
    let (_, line, link) = linked_lines(stats)
        .into_iter()
        .find(|(_, line, _)| line.contains("detailed status report"))
        .unwrap_or_else(|| panic!("the panel's first line carries the reported link; panel: {:?}", stats.lines));
    assert_ne!(link, 0);
    assert!(line.starts_with("You are human."), "the pre-click panel names the player's race: {line:?}");

    let state = render_state();
    let m = render(&sess, &state);
    let win = click_link(&mut sess, &m, link);
    assert_eq!(win, 7, "the statistics panel owns the click");

    // …and the game ACTS on it. Asserted as the CHANGE the click caused: the
    // panel's first line was the race plus the link, and the detailed view
    // replaces it with its own "< back" affordance under a fresh link value.
    let after = Engine::screen(&sess);
    let stats = buffer_window(&after.root, 7).expect("the panel is still window 7 after the click");
    let first = stats.lines.first().cloned().unwrap_or_default();
    assert!(
        first.contains("back"),
        "clicking [detailed status report] must switch the panel to the detailed view, which \
         offers its way back; the panel now reads {:?}",
        stats.lines
    );
    let back = linked_lines(stats);
    assert!(
        back.iter().any(|&(_, _, l)| l != link && l != 0),
        "the detailed view arms its own link, so the panel is still clickable; got {back:?}"
    );
}

/// The in-game HELP menu: nine choices, all in a non-primary buffer (win 47),
/// all dead to the mouse for the same reason. Clicking the first must navigate
/// into its submenu exactly as typing `1` does.
///
/// Reverted, this fails in `click_link` with `got []`.
#[test]
fn an_in_game_menus_choices_answer_a_click() {
    let Some(mut sess) = into_gameplay() else { return };
    let _ = Engine::submit(&mut sess, "help");

    let model = Engine::screen(&sess);
    let menu = buffer_window(&model.root, 47).expect("Kerkerkruip's in-game menu is Glk window 47");
    assert!(!menu.primary, "the menu is a non-primary buffer");
    let choices = linked_lines(menu);
    let first = choices
        .iter()
        .find(|(_, line, _)| line.contains("Players new to Interactive Fiction"))
        .unwrap_or_else(|| panic!("the top-level HELP menu lists its first chapter; menu: {:?}", menu.lines))
        .2;
    assert!(
        choices.iter().map(|&(_, _, l)| l).collect::<std::collections::BTreeSet<_>>().len() >= 5,
        "a real menu: several distinct link values, one per choice; got {choices:?}"
    );

    let state = render_state();
    let m = render(&sess, &state);
    let win = click_link(&mut sess, &m, first);
    assert_eq!(win, 47, "the menu window owns the click");

    // The menu navigated: the same window now holds the CHAPTER's own items,
    // not the table of contents it was showing.
    let after = Engine::screen(&sess);
    let menu = buffer_window(&after.root, 47).expect("the menu is still window 47 after the click");
    assert!(
        menu.lines.iter().any(|l| l.contains("Interactive Fiction basics")),
        "clicking \"Players new to Interactive Fiction\" must open that chapter the way typing \
         its number does; the menu now reads {:?}",
        menu.lines
    );
    assert!(
        !menu.lines.iter().any(|l| l.contains("Credits, Copyright & Afterword")),
        "…and left the table of contents behind; the menu now reads {:?}",
        menu.lines
    );
}

/// The GRAPHICAL main menu was never on the broken path, and this says so by
/// driving it: "New Game" is a region of a real Glk graphics window with
/// `glk_request_mouse_event`, so a click travels `glk_mouse_target` →
/// `deliver_mouse` and nothing in this quest touches it.
///
/// Here so that "clickable menus do nothing" is not re-investigated from the
/// graphics end: it is the text menus above that were dead.
#[test]
fn the_graphical_main_menu_click_was_never_the_broken_path() {
    let Some(path) = story_path() else { return };
    let mut sess = boot(&path);
    // The boot animation runs past its first keypress request (the sword splash)
    // and keeps painting until the MENU BAR itself is on the canvas, so drive by
    // the bar's own pixels rather than by a tick count — and it doubles as the
    // non-vacuity guard that this really is the menu frame.
    let lit = tick_to_menu_bar(&mut sess);
    assert!(lit > 0, "the intro must reach the frame that paints the New Game / Help / Options / Quit bar");

    // The menu is drawn in a real mouse-watching GRAPHICS window, and carries no
    // hyperlink at all — so it is served by `glk_mouse_target`/`deliver_mouse`
    // and this quest's cell→link map has nothing to do with it either way.
    let watching = sess.mouse_windows();
    assert_eq!(watching, vec![5], "the menu is drawn in mouse-watching graphics window 5; got {watching:?}");
    let state = render_state();
    let m = render(&sess, &state);
    assert!(m.links.is_empty(), "nothing on the menu is a hyperlink; got {:?}", m.links);

    // "New Game" sits at canvas px ~(240..362, 44..72) = cells ~(30..45, 2..4).
    // The same call main.rs makes, at the cell under its label.
    let target = app::glulx_session::glk_mouse_target(
        false,
        37,
        3,
        (0, 0, COLS, ROWS),
        &watching,
        &m.win_rects,
        sess.char_pixels(),
        None,
    )
    .expect("a click on the menu must resolve to the graphics window");
    assert_eq!(target, (5, 300, 56), "window-relative pixels for cell (37,3) at an 8x16 cell");
    let _ = sess.deliver_mouse(target.0, target.1, target.2);
    tick_to_input(&mut sess);

    // The game answered, and unmistakably: it dealt a dungeon. A click the game
    // ignored leaves it sitting on the menu waiting for the next one.
    assert_eq!(Engine::pending_input(&sess), InputKind::Line, "New Game reaches the parser prompt");
    let after = Engine::screen(&sess);
    assert!(
        buffer_window(&after.root, 7).is_some(),
        "clicking New Game must start a game, laying out its Statistics panel; layout is:\n{}",
        Engine::window_dump(&sess).join("\n")
    );
}
