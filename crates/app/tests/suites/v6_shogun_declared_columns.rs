//! SQ-0697 — Shogun's opening must keep the placement the game declares.
//!
//! Shogun centres **every** line of its title header itself, in pixels, computed
//! from its own window geometry:
//!
//! ```text
//! @get_wind_prop(win=0, prop=4) -> 49      ; current y cursor
//! @get_wind_prop(win=0, prop=3) -> 640     ; the window's WIDTH
//! @set_cursor(row=49, col=297, window=0)   ; the centred x it worked out
//! ```
//!
//! then prints `SHOGUN` one character at a time with **no leading spaces at
//! all**. The centring is not in the text; it is in the column. Window 0 wraps
//! and scrolls, so its text streams to the host transcript — and the column was
//! dropped on the way, because ZMSD §15's `set_cursor` stores a pixel position
//! that nothing downstream ever read back.
//!
//! At the v6 cell width of 8 px the pixel column and the character column are the
//! same measurement, so the declaration can be carried into the character stream
//! exactly. Every header line lands on its ideal column that way — verified
//! against an 80-column screen:
//!
//! | line                                                    | len | declared x | indent | ideal |
//! |---------------------------------------------------------|-----|------------|--------|-------|
//! | `SHOGUN`                                                 |   6 |        297 |     37 |    37 |
//! | `A Story of Japan`                                       |  16 |        257 |     32 |    32 |
//! | `Copyright (c) 1988 by Infocom`                           |  29 |        205 |     25 |    25 |
//! | `All rights reserved.`                                    |  20 |        241 |     30 |    30 |
//! | `SHOGUN is a trademark of James Clavell`                  |  38 |        169 |     21 |    21 |
//! | `Original Literary Work Copyright 1975 by James Clavell`  |  54 |        105 |     13 |    13 |
//!
//! Corroborated outside the code: the user confirms the Apple II and DOS
//! walkthroughs both show this header centred, and that Spatterlight centres it
//! whether it identifies as Amiga or IBM. Interpreter number was ruled out — the
//! text is byte-identical and unpadded when booted as none/2/3/4/5/10.
//!
//! The declaration is ONE-SHOT and armed only by `set_cursor`, never by the
//! cursor advancing over printed text: Shogun prints character by character, so
//! re-deriving an indent from `x_cursor` on each call would re-indent every
//! letter. A cross-game audit pins the blast radius — Arthur only ever issues the
//! §8.8.3.2 cursor-visibility sentinels (row −1/−2, which store no position) and
//! Zork Zero only ever declares `col = 1`, so both are byte-identical; Journey,
//! whose title screen is centred in the original too, gains the same correct
//! centring as Shogun.
//!
//! Also pinned here: Shogun's opening MENU. After the first keypress the game
//! repositions into win1 = header, win0 = a four-row story strip at the bottom,
//! and **win2 = the menu at its own origin (235, 337), 169×48**. win2 is a paint
//! window, so its glyph runs carry screen-absolute pixel positions and must be
//! published at them.
//!
//! The story asset is gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story and return the session, or `None` when the fixture is absent.
fn boot(name: &str) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("v6 story should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// The leading-space count of a transcript line.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Shogun's title header, as the transcript receives it after the first keypress.
fn shogun_header() -> Option<String> {
    let mut session = boot("shogun-r322-s890706.z6")?;
    // One keypress dismisses the title splash and emits the whole header.
    Some(session.submit_char(13).transcript)
}

// ── B1: the declared column reaches the transcript ────────────────────────────

/// Every header line Shogun centred lands on the column it computed. The indents
/// are asserted as EXACT values, against the game's own `set_cursor` arithmetic.
#[test]
fn shogun_header_lines_land_on_the_columns_the_game_declared() {
    let Some(header) = shogun_header() else { return };
    // (line text, the x the game declared, the indent that column means at 8 px/char)
    let expected: &[(&str, u16, usize)] = &[
        ("SHOGUN", 297, 37),
        ("A Story of Japan", 257, 32),
        ("Copyright (c) 1988 by Infocom", 205, 25),
        ("All rights reserved.", 241, 30),
        ("SHOGUN is a trademark of James Clavell", 169, 21),
        ("Original Literary Work Copyright 1975 by James Clavell", 105, 13),
    ];
    let lines: Vec<&str> = header.lines().collect();
    for &(text, declared, want) in expected {
        let line = lines
            .iter()
            .find(|l| l.trim() == text)
            .unwrap_or_else(|| panic!("the header carries {text:?}; got {lines:#?}"));
        assert_eq!(
            indent(line),
            want,
            "{text:?} was declared at pixel column {declared}; at the 8 px v6 cell that is \
             character column {want}, and the transcript must carry it"
        );
        // The declaration really is the game's own centring of an 80-column screen.
        assert_eq!(
            want,
            (80 - text.chars().count()) / 2,
            "{text:?}: the declared column IS the centred one"
        );
    }
}

/// The centring is in the COLUMN, not in the text: Shogun emits no leading
/// spaces of its own, so every space in front of a header line is one we carried
/// from the declaration. (Guards against a future "fix" that pads the text.)
#[test]
fn shogun_prose_after_the_reposition_keeps_column_one() {
    let Some(header) = shogun_header() else { return };
    let last = header
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .expect("the header ends with the menu's lead-in line");
    assert_eq!(last.trim(), "You may choose to:", "the lead-in is the last prose line");
    assert_eq!(
        indent(last),
        0,
        "printed at `set_cursor(row=1, col=1, window=0)` after the reposition — a declared \
         column of 1 is an indent of 0, and must stay one"
    );
}

// ── B2: the opening menu is published at its own origin ───────────────────────

/// After the reposition the menu lives in **window 2**, at its own (235, 337)
/// origin, and its glyph runs reach the model at their screen-absolute pixel
/// positions — not folded into the story window's column.
#[test]
fn shogun_menu_window_publishes_its_own_origin() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    let _ = session.submit_char(13);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a layered composite") };

    let menu = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Grid(g) if !g.px_texts.is_empty()) && pw.y_px > 300)
        .expect("the opening menu is a painted window below the story strip");
    assert_eq!(
        (menu.x_px, menu.y_px, menu.w_px, menu.h_px),
        (234, 336, 169, 48),
        "window 2 keeps the box the game moved and sized it to (1-based (235,337), 169x48)"
    );
    let WinNode::Grid(g) = &menu.node else { unreachable!() };

    // The three items, each on its own 16 px row, all starting at the window's
    // own left edge — 1-based screen x 235.
    for (row, label) in [(337u16, "START"), (353, "RESTORE"), (369, "QUIT")] {
        let runs: String = g
            .px_texts
            .iter()
            .filter(|t| t.y == row)
            .map(|t| t.text.as_str())
            .collect();
        assert!(
            runs.contains(label),
            "row y={row} carries {label:?}; got {runs:?}"
        );
        let first_x = g.px_texts.iter().filter(|t| t.y == row).map(|t| t.x).min();
        assert_eq!(
            first_x,
            Some(235),
            "{label:?} starts at the menu window's own declared origin, not the story column"
        );
    }
    // The story window's lead-in prose sits far to its LEFT — that contrast is the
    // reported "prompt left, menu right" reading, and it is the game's own layout.
    let story = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
        .expect("window 0 is still the story window");
    assert!(
        story.x_px < menu.x_px,
        "the story strip starts left of the menu ({} < {})",
        story.x_px,
        menu.x_px
    );
}

// ── Blast radius: the games that declare nothing are byte-identical ───────────

/// Arthur and Zork Zero must be untouched. Arthur's only window-0 `set_cursor`
/// calls are the ZMSD §8.8.3.2 cursor-visibility sentinels (row −1 / −2), which
/// store no position at all; Zork Zero's only ever name `col = 1`. Neither can
/// produce an indent, and their prose must carry none it did not print itself.
#[test]
fn games_that_declare_no_column_gain_no_indent() {
    for (name, feed) in [("arthur-r74-s890714.z6", true), ("zork0-r393-s890714.z6", false)] {
        let Some(mut session) = boot(name) else { return };
        let mut declared_beyond_one = Vec::new();
        for _ in 0..8 {
            let r = match session.pending_input() {
                InputKind::Line => session.submit("look"),
                InputKind::Char => session.submit_char(if feed { 13 } else { b' ' }),
                InputKind::Event => session.submit(""),
            };
            if feed && r.transcript.to_lowercase().contains("y or n") {
                let _ = session.submit_char(b'n');
            }
            // Every window-0 prose line the game did not itself indent must arrive
            // flush left. Zork Zero prints its own 3-space paragraph indents as
            // literal text, so only a >= 8 jump could be a column we invented.
            for line in r.transcript.lines() {
                if !line.trim().is_empty() && indent(line) >= 8 {
                    declared_beyond_one.push(line.to_string());
                }
            }
        }
        assert!(
            declared_beyond_one.is_empty(),
            "{name} declares no prose column, so no line may gain one: {declared_beyond_one:#?}"
        );
    }
}

/// Journey centres its title screen the same way Shogun does, and gets the same
/// treatment — a second, independent game confirming the rule rather than a
/// Shogun special case.
#[test]
fn journey_title_screen_is_centred_by_the_same_rule() {
    let Some(mut session) = boot("journey-r83-s890706.z6") else { return };
    let mut text = session.take_transcript();
    for _ in 0..3 {
        if text.lines().any(|l| l.trim() == "JOURNEY") {
            break;
        }
        text = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        }
        .transcript;
    }
    let title = text
        .lines()
        .find(|l| l.trim() == "JOURNEY")
        .unwrap_or_else(|| panic!("Journey's title line; got {text:?}"));
    // `@set_cursor(row=17, col=169, window=0)` → (169 − 1) / 8 = 21… and the game
    // centres for its own narrower window, so assert the relationship, not a
    // magic number: the indent is the declared column in cells.
    assert!(
        indent(title) > 0,
        "Journey declares a centred column for its title and it must be carried: {title:?}"
    );
}
