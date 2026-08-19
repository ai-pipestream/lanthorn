//! SQ-0532 wave-3 verification: the ZMSD §8 screen-model compliance waves,
//! checked against the REAL Infocom games rather than synthetic stubs.
//!
//! Wave 1 (zvm) made the header capability bits truthful, changed v6 window-0
//! boot geometry, split the v6 `erase_window` -1/-2 paths, and started
//! preserving upper-window contents across a v4+ re-split. Wave 2 (app) made
//! `$20`/`$21` follow the story pane and `$2C`/`$2D` follow the host's page/ink.
//! Each of those touches behaviour that shipping games depend on, so this file
//! drives the games headlessly and pins what they actually do.
//!
//! Every test uses the skip-if-missing pattern of the other gitignored-story
//! smokes: the `stories/` tree is local-only, so an absent file SKIPs rather
//! than fails. The companion file `v6_game_colour_regression.rs` holds the
//! reproducers for the regressions this verification found.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story with its Blorb pictures, exactly as the app does.
/// Whether ANY v6 story fixture is present. `stories/` is gitignored (the files are
/// commercial), so CI and fresh worktrees legitimately have none — there, these smokes
/// skip like every other real-game test. The `ran > 0` guards below exist to catch a
/// LOCAL run that silently tested nothing because a filename drifted; they must not
/// fire where the fixtures simply cannot exist.
fn any_v6_story_present() -> bool {
    let dir = stories_dir();
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.filter_map(Result::ok).any(|e| {
                e.file_name().to_string_lossy().ends_with(".z6")
            })
        })
        .unwrap_or(false)
}

/// `colours` is the app's `honor_game_colours` (its config default is **true**).
fn boot_v6(file: &str, colours: bool, trace: bool) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, colours, false, None, trace, dims, picts.std_window(), Some((2, 9)), None)
        .expect("v6 story should load and boot without a ZError");
    assert!(!s.quit, "{file} quit during boot");
    assert!(s.machine.fault_trace.is_none(), "{file} faulted during boot: {:?}", s.machine.fault_trace);
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

fn boot_z(file: &str, default_colours: (u8, u8), trace: bool) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(bytes, true, false, None, trace, Vec::new(), None, Some(default_colours), None)
            .expect("story should load and boot without a ZError"),
    )
}

/// Drive `n` turns, feeding `cmds` to line prompts and Enter to char prompts.
/// Panics on any fault; returns the concatenated transcript.
fn drive(s: &mut GameSession, cmds: &[&str], n: usize, who: &str) -> String {
    let mut out = String::new();
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
        out.push_str(&r.transcript);
    }
    out
}

// ── R1: the header capability bits are now truthful ──────────────────────────

/// ZMSD §11.1. Wave 1 started advertising timed keyboard input (Flags 1 bit 7)
/// and v6 picture display (bit 1), preserving the game's font-3/pictures request
/// (Flags 2 bit 3) and mouse request (bit 5), and clearing the menus request
/// (bit 8, `make_menu` being a stub). Those bits steer which code path a game
/// takes at boot, so every shipping v6 title is driven through boot and several
/// turns with the bits in their new state.
///
/// Journey is the load-bearing case: its raw header ASKS for menus (Flags 2 bit
/// 8 set in the story file), and it is the only one of the four that does. It
/// must still reach its command menu with that request denied.
#[test]
fn v6_titles_boot_and_play_with_truthful_capability_bits() {
    let cases: [(&str, &str, &[&str], &str); 4] = [
        ("Zork Zero", "zork0-r393-s890714.z6", &["ne"], ""),
        ("Shogun", "shogun-r322-s890706.z6", &["look", "xyzzy", "look"], "Bridge"),
        ("Arthur", "arthur-r74-s890714.z6", &["look", "wait"], ""),
        ("Journey", "journey-r83-s890706.z6", &["", "", ""], ""),
    ];
    let mut ran = 0;
    for (name, file, cmds, marker) in cases {
        let Some(mut s) = boot_v6(file, true, false) else { continue };
        ran += 1;

        let f1 = s.machine.mem.read_byte(0x01);
        let f2 = s.machine.mem.read_word(0x10);
        // Flags 1 bit 7 "Timed keyboard input available?" — timed read/read_char
        // are implemented, so it is advertised (it used to be cleared).
        assert_ne!(f1 & (1 << 7), 0, "{name}: Flags1 bit 7 (timed input) must be advertised");
        // Flags 1 bit 1 "Picture displaying available?" (Version 6) — the whole
        // draw_picture/picture_data path over Blorb Pict resources is implemented.
        assert_ne!(f1 & (1 << 1), 0, "{name}: Flags1 bit 1 (v6 pictures) must be advertised");
        // Flags 2 bit 3 (v6: "game wants to use pictures") and bit 5 ("game wants
        // to use a mouse") are the GAME's requests; all four titles ask for both
        // in the story file and both are provided, so both must survive.
        assert_ne!(f2 & (1 << 3), 0, "{name}: Flags2 bit 3 (pictures wanted) must be preserved");
        assert_ne!(f2 & (1 << 5), 0, "{name}: Flags2 bit 5 (mouse wanted) must be preserved");
        // Flags 2 bit 8 "game wants to use menus" — `make_menu` always branches
        // false, so the request is denied. Journey is the only title that asks.
        assert_eq!(f2 & (1 << 8), 0, "{name}: Flags2 bit 8 (menus) must be cleared — make_menu is a stub");

        let transcript = drive(&mut s, cmds, 10, name);
        if !marker.is_empty() {
            assert!(transcript.contains(marker), "{name}: expected marker {marker:?} in the drive transcript");
        }
        assert!(s.machine.fault_trace.is_none(), "{name}: faulted across the drive");
        // The v6 window model survives the drive (a Layered composite, not the
        // degenerate fallback).
        assert!(
            matches!(s.screen().root, app::engine::WinNode::Layered(_)),
            "{name}: v6 screen model must stay Layered"
        );
    }
    assert!(
        ran > 0 || !any_v6_story_present(),
        "v6 story files are present but none booted — this smoke was vacuous"
    );
}

/// Journey specifically: its story header is the only one of the four that sets
/// Flags 2 bit 8 ("game wants to use menus"). Wave 1 clears it. Non-vacuously
/// pin that the request IS present in the file and IS denied after boot, and
/// that Journey still paints its own (game-drawn, not `make_menu`) command menu.
#[test]
fn journey_menu_request_is_denied_but_its_painted_menu_survives() {
    let path = stories_dir().join("journey-r83-s890706.z6");
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let raw_f2 = u16::from_be_bytes([raw[0x10], raw[0x11]]);
    assert_ne!(raw_f2 & (1 << 8), 0, "Journey's story file really does request menus (test is non-vacuous)");

    let Some(mut s) = boot_v6("journey-r83-s890706.z6", true, false) else { return };
    assert_eq!(s.machine.mem.read_word(0x10) & (1 << 8), 0, "the menus request is denied at boot");

    // Tap through the intro vignettes to the Praxix command-menu page.
    for _ in 0..40 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "Journey faulted reaching its menu: {:?}", r.fault);
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    // Journey's menu is its own painted text, not an interpreter menu: it lives
    // as absolute pixel runs in the window table, so denying bit 8 cannot remove it.
    let v6 = s.machine.screen.v6.as_ref().expect("v6 screen");
    let runs: usize = v6.windows.iter().map(|w| w.texts.len()).sum();
    assert!(runs > 0, "Journey still paints its command menu as absolute text runs");
}

/// Beyond Zork (v5) is the reason Flags 2 bit 3 must be preserved: ZMSD §8.1.5.1
/// says only "an interpreter which cannot provide the character graphics font
/// should clear bit 3", and lanthorn renders font 3. Drive the game to its first
/// room and assert the upper window really is drawn with font-3-translated
/// box-drawing glyphs, in colour — i.e. the preserved bit changed the game's code
/// path in the intended direction.
#[test]
fn beyond_zork_draws_its_font3_box_with_the_preserved_flag() {
    let Some(mut s) = boot_z("beyondzork-r57-s871221.z5", (2, 9), false) else { return };
    assert_ne!(s.machine.mem.read_word(0x10) & (1 << 3), 0, "Flags2 bit 3 (font 3 wanted) preserved");

    // Answer the setup questions, then play until the status box is split.
    let mut last = s.take_transcript();
    for t in 0..30 {
        let low = last.to_lowercase();
        let cmd = if low.contains("vt220") {
            "yes"
        } else if low.contains("begin, restore") {
            "begin"
        } else {
            "look"
        };
        let r = match s.pending_input() {
            InputKind::Line => s.submit(cmd),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "Beyond Zork faulted on turn {t}: {:?}", r.fault);
        assert!(!r.quit, "Beyond Zork quit on turn {t}");
        last = r.transcript.clone();
        if s.machine.screen.upper.rows > 0 && t > 4 {
            break;
        }
    }

    let up = &s.machine.screen.upper;
    assert!(up.rows >= 8, "Beyond Zork splits a tall status/description box, got {} rows", up.rows);
    let grid: String = (1..=up.rows)
        .flat_map(|r| (1..=up.cols).map(move |c| (r, c)))
        .map(|(r, c)| up.cell(r, c).ch)
        .collect();
    // `zvm::cpu::exec::font3_translate` maps the font-3 codes onto these Unicode
    // box-drawing glyphs; seeing them proves the game selected font 3 and the
    // translation ran.
    for want in ['┌', '┐', '│', '─'] {
        assert!(grid.contains(want), "font-3 box glyph {want:?} missing from Beyond Zork's status box");
    }
    // ...and it is a COLOUR box: at least one cell carries a non-default pair.
    let coloured = (1..=up.rows)
        .flat_map(|r| (1..=up.cols).map(move |c| (r, c)))
        .any(|(r, c)| {
            let cell = up.cell(r, c);
            cell.fg != zvm::screen::ZColour::Default || cell.bg != zvm::screen::ZColour::Default
        });
    assert!(coloured, "Beyond Zork's box should carry the colours it set");
}

// ── R3: does any shipping game issue v6 erase_window -2? ─────────────────────

/// Wave 1 split the v6 `erase_window` -1 and -2 paths, which used to be
/// identical. -1 ("erase to window 0's background, unsplit, select window 0",
/// ZMSD §8.8.5.3.1) is exercised heavily; -2 ("erase to the CURRENT background,
/// changing no window attributes or cursor positions", §8.8.5.3.2) is the newly
/// distinct one.
///
/// Empirical finding recorded here: across boot plus a gameplay drive, **none**
/// of the four shipping v6 titles ever issues -2, so the -2 change cannot regress
/// them. The assertion that -1 IS issued keeps this test honest — if the screen
/// trace ever stops reporting erases at all, this fails rather than silently
/// passing on an empty trace. If a game ever does start issuing -2, the
/// `eprintln!` inventory below makes it visible in the test log.
#[test]
fn no_shipping_v6_title_issues_erase_window_minus_two() {
    let cases: [(&str, &str, &[&str]); 4] = [
        ("Zork Zero", "zork0-r393-s890714.z6", &["ne"]),
        ("Shogun", "shogun-r322-s890706.z6", &["look", "look"]),
        ("Arthur", "arthur-r74-s890714.z6", &["look", "wait"]),
        ("Journey", "journey-r83-s890706.z6", &["", "", ""]),
    ];
    let mut ran = 0;
    let mut saw_minus_one = false;
    for (name, file, cmds) in cases {
        let Some(mut s) = boot_v6(file, true, true) else { continue };
        ran += 1;
        let mut erases: Vec<String> = s
            .take_screen_trace()
            .into_iter()
            .filter(|l| l.starts_with("@erase_window"))
            .collect();
        for t in 0..8 {
            let r = match s.pending_input() {
                InputKind::Line => s.submit(cmds.get(t).copied().unwrap_or("look")),
                InputKind::Char => s.submit_char(13),
                InputKind::Event => s.submit(""),
            };
            erases.extend(s.take_screen_trace().into_iter().filter(|l| l.starts_with("@erase_window")));
            assert!(r.fault.is_none(), "{name} faulted: {:?}", r.fault);
            if r.quit {
                break;
            }
        }
        eprintln!("{name} erase_window ops: {erases:?}");
        // `zscreen_window_name` renders -2 as "all" and -1 as "all(unsplit)".
        assert!(
            !erases.iter().any(|l| l == "@erase_window(all)"),
            "{name} now issues erase_window(-2) — the §8.8.5.3.2 path is live for it and needs its own smoke"
        );
        if erases.iter().any(|l| l == "@erase_window(all(unsplit))") {
            saw_minus_one = true;
            // §8.8.5.3.1: after -1 the screen is unsplit and window 0 selected.
            let v6 = s.machine.screen.v6.as_ref().expect("v6 screen");
            assert!(v6.windows[0].y_size > 0, "{name}: window 0 has a real height after an unsplit erase");
        }
    }
    assert!(
        ran > 0 || !any_v6_story_present(),
        "v6 story files are present but none booted — this smoke was vacuous"
    );
    assert!(
        saw_minus_one || !any_v6_story_present(),
        "erase_window(-1) was never observed — the trace plumbing is broken, not the games"
    );
}

// ── R4: v4+ split_window preserves the upper window ──────────────────────────

/// ZMSD §15 `split_window`: "In Version 3 (only) the upper window should be
/// cleared after the split." Wave 1 therefore made v4+ re-splits preserve the
/// existing grid (`resize_preserving`) instead of reallocating it blank.
///
/// A Mind Forever Voyaging (v4) is the game-shaped case: it splits SEVEN rows at
/// boot for its "* PART I *" title card, then on the first turn erases, re-splits
/// to TWO rows, and paints its Mode/Location/Time/Date status there. So a real
/// game genuinely re-splits at two different heights in one session — the exact
/// scenario the change alters. Assert the status survives every later turn
/// without going blank and without stale rows from the taller split leaking back.
#[test]
fn amfv_resplit_keeps_its_status_and_drops_the_taller_splits_rows() {
    let Some(mut s) = boot_z("amfv-r77-s850814.z4", (2, 9), true) else { return };
    assert_eq!(s.machine.mem.version(), 4, "AMFV is a v4 story");

    let boot_ops = s.take_screen_trace();
    assert!(
        boot_ops.iter().any(|l| l == "@split_window(7)"),
        "AMFV splits 7 rows at boot for its title card; got {boot_ops:?}"
    );
    let tall: String = (1..=s.machine.screen.upper.cols).map(|c| s.machine.screen.upper.cell(5, c).ch).collect();
    assert!(tall.contains("PART I"), "the 7-row split carries the title card on row 5, got {tall:?}");
    let _ = s.take_transcript();

    // First turn: the game erases and re-splits SHORTER (7 → 2).
    let r = s.submit("look");
    assert!(r.fault.is_none(), "AMFV faulted on the first turn: {:?}", r.fault);
    let ops = s.take_screen_trace();
    assert!(ops.iter().any(|l| l == "@split_window(2)"), "AMFV re-splits to 2 rows; got {ops:?}");
    assert_eq!(s.machine.screen.upper.rows, 2, "the upper window shrank to the new split height");

    // Shrinking TRUNCATES: the title card's row 5 is gone, not retained as garbage.
    let grid_now: String = (1..=s.machine.screen.upper.rows)
        .flat_map(|r| (1..=s.machine.screen.upper.cols).map(move |c| (r, c)))
        .map(|(r, c)| s.machine.screen.upper.cell(r, c).ch)
        .collect();
    assert!(!grid_now.contains("PART I"), "the taller split's title row must not survive the shrink");

    // Every later turn keeps a coherent status bar: never blank, never stale.
    for t in 0..6 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(if t % 2 == 0 { "wait" } else { "look" }),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "AMFV faulted on turn {t}: {:?}", r.fault);
        let up = &s.machine.screen.upper;
        assert_eq!(up.rows, 2, "turn {t}: the split height is stable");
        let row1: String = (1..=up.cols).map(|c| up.cell(1, c).ch).collect();
        let row2: String = (1..=up.cols).map(|c| up.cell(2, c).ch).collect();
        assert!(row1.contains("Mode:"), "turn {t}: status row 1 lost its Mode field: {row1:?}");
        assert!(row1.contains("Time:"), "turn {t}: status row 1 lost its Time field: {row1:?}");
        assert!(row2.contains("Location:"), "turn {t}: status row 2 lost its Location field: {row2:?}");
        assert!(!row1.contains("PART I"), "turn {t}: stale title text reappeared");
    }
}

// ── R5: $20/$21 follow the story pane ────────────────────────────────────────

/// ZMSD §8.4: the interpreter "may change the exact dimensions whenever it likes
/// but must write the current height (in lines) and width (in characters) into
/// bytes $20 and $21." Wave 2 feeds the story pane's measured size through
/// `Engine::set_screen_dims`. End-to-end on a real v5 game: the header tracks
/// both a grow and a shrink mid-session, the v5+ unit words at $22/$24 follow,
/// and the game keeps playing faultlessly across the resizes.
#[test]
fn sherlock_header_dims_track_pane_resizes_mid_game() {
    let Some(mut s) = boot_z("sherlock-r26-s880127.z5", (2, 9), false) else { return };
    assert_eq!(s.machine.mem.version(), 5);
    drive(&mut s, &["look", "wait"], 3, "Sherlock");

    for (rows, cols) in [(30u16, 100u16), (20, 50), (24, 80)] {
        Engine::set_screen_dims(&mut s, rows, cols);
        assert_eq!(
            (s.machine.mem.read_byte(0x20) as u16, s.machine.mem.read_byte(0x21) as u16),
            (rows, cols),
            "$20/$21 follow the pane"
        );
        assert_eq!(
            (s.machine.mem.read_word(0x22), s.machine.mem.read_word(0x24)),
            (cols, rows),
            "§8.4.3: the v5+ unit words follow too"
        );
        // The game keeps running across the resize.
        drive(&mut s, &["look", "wait"], 2, "Sherlock after resize");
        let up = &s.machine.screen.upper;
        let row1: String = (1..=up.cols).map(|c| up.cell(1, c).ch).collect();
        assert!(row1.contains("Baker Street"), "Sherlock's status survives a {rows}x{cols} resize: {row1:?}");
    }
}

/// A-F1(d): a v6 story lays out on its own fixed 640×400 pixel screen, which the
/// app SCALES into the pane, so the pane feed must not touch its header. Pinned
/// on the real Zork Zero rather than the stub the wave-2 unit test uses.
#[test]
fn zork0_v6_header_dims_ignore_the_pane_feed() {
    let Some(mut s) = boot_v6("zork0-r393-s890714.z6", true, false) else { return };
    let before = (
        s.machine.mem.read_byte(0x20),
        s.machine.mem.read_byte(0x21),
        s.machine.mem.read_word(0x22),
        s.machine.mem.read_word(0x24),
    );
    assert_eq!((before.2, before.3), (640, 400), "Zork0 runs on the 640x400 v6 unit screen");
    Engine::set_screen_dims(&mut s, 45, 200);
    let after = (
        s.machine.mem.read_byte(0x20),
        s.machine.mem.read_byte(0x21),
        s.machine.mem.read_word(0x22),
        s.machine.mem.read_word(0x24),
    );
    assert_eq!(before, after, "a v6 story keeps its native screen dimensions through a pane resize");
}

// ── R6: $2C/$2D published in both orders ─────────────────────────────────────

/// ZMSD §8.3.3: the interpreter writes its own default background/foreground into
/// $2C/$2D. Wave 2 resolves that pair from the host's real page/ink, so on a dark
/// terminal it publishes (2, 9) and on a light one (9, 2) — the reverse order.
/// Beyond Zork reads the header pair while building its colour scheme, so drive
/// it fully through setup and into its first room under BOTH orders: neither may
/// fault, and both must reach the same playable state.
#[test]
fn beyond_zork_plays_under_both_default_colour_orders() {
    let mut seen = Vec::new();
    for pair in [(2u8, 9u8), (9u8, 2u8)] {
        let Some(mut s) = boot_z("beyondzork-r57-s871221.z5", pair, false) else { return };
        assert_eq!(
            (s.machine.mem.read_byte(0x2C), s.machine.mem.read_byte(0x2D)),
            pair,
            "the host's default pair reaches $2C/$2D as published"
        );
        let mut last = s.take_transcript();
        for t in 0..30 {
            let low = last.to_lowercase();
            let cmd = if low.contains("vt220") {
                "yes"
            } else if low.contains("begin, restore") {
                "begin"
            } else {
                "look"
            };
            let r = match s.pending_input() {
                InputKind::Line => s.submit(cmd),
                InputKind::Char => s.submit_char(13),
                InputKind::Event => s.submit(""),
            };
            assert!(r.fault.is_none(), "Beyond Zork faulted with defaults {pair:?} on turn {t}: {:?}", r.fault);
            assert!(!r.quit, "Beyond Zork quit with defaults {pair:?} on turn {t}");
            last = r.transcript.clone();
            if s.machine.screen.upper.rows > 0 && t > 4 {
                break;
            }
        }
        let up = &s.machine.screen.upper;
        assert!(up.rows >= 8, "reached the split status box with defaults {pair:?}");
        let row1: String = (1..=up.cols).map(|c| up.cell(1, c).ch).collect();
        assert!(row1.contains("Hilltop"), "reached Hilltop with defaults {pair:?}: {row1:?}");
        seen.push((s.machine.screen.current_fg, s.machine.screen.current_bg));
    }
    if seen.len() == 2 {
        // FINDING (recorded, not asserted): Beyond Zork picks the same cyan-on-black
        // scheme either way — it does NOT re-derive its palette from $2C/$2D on this
        // path. The requirement that matters is that neither order faults, which the
        // drive above establishes.
        eprintln!("Beyond Zork current fg/bg under (2,9) vs (9,2): {seen:?}");
    }
}

// ── R7: the frameless click map ──────────────────────────────────────────────

/// A-F4: frameless v6 draws no game image, so it recorded no click map and mouse
/// clicks were dead there while raster and hybrid both worked — now that the
/// mouse capability bit is honestly advertised, that gap is visible to games.
/// End-to-end: render real Zork Zero frameless and confirm a click in the pane
/// maps to a game pixel inside the native screen.
#[test]
fn zork0_frameless_render_records_a_usable_click_map() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let Some(s) = boot_v6("zork0-r393-s890714.z6", true, false) else { return };
    let model = s.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    // Force the CELL path: SQ-0895 removed frameless, which was the
    // deliberate route in. Dropping the picker is the substitute whose
    // ONLY effect is the one frameless contributed here — draw no game
    // image. (A modal overlay also lands on the cell path, but it
    // additionally suppresses the inlined input line, which shifts row
    // counts.)
    state.game_picker = None;

    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let gr = state.graphics_render.borrow();
    let map = gr.last_v6_map.as_ref().expect("frameless render records a v6 click map");
    // The pane's full cell rect stands for the whole native screen, so both the
    // top-left and a mid-pane click land inside it.
    let tl = map.map_click(0, 0).expect("a click at the pane origin maps into the game screen");
    let mid = map.map_click(40, 15).expect("a mid-pane click maps into the game screen");
    assert!(tl.0 >= 1 && tl.1 >= 1, "game pixel coords are 1-based, got {tl:?}");
    assert!(mid.0 > tl.0 && mid.1 > tl.1, "a click further into the pane maps further into the screen");
    assert!(mid.0 <= 640 && mid.1 <= 400, "the mapped pixel stays inside Zork0's 640x400 screen, got {mid:?}");
}
