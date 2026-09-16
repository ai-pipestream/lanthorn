//! SQ-1496/SQ-1498: the Atari 8-bit US S.A.G.A. releases' own family-C
//! artwork, wired all the way to the picture band.
//!
//! `crates/scott/tests/saga_atari_specimens.rs` pins the (usage, index) table
//! reader and the SQ-1498 bad-sector fallback at the decoder level, against
//! the same three real specimens this file uses (*Voodoo Castle*, *The
//! Count*, *The Sorcerer of Claymorgue Castle*) — see that file for the
//! per-title tables and the exact byte-level findings. This file is the other
//! half: that the table reaches `ScottSession`'s picture band the way
//! `startup.rs` actually boots one, side A paired with its side B through
//! [`app::hints::saga_companion_side`] via
//! [`app::graphics::ScottPictureSources::resolve`].
//!
//! All three fixtures are gitignored commercial disk images; every case here
//! skips vacuously with an explanation when `stories/scott-dialects/atari/`
//! is absent.

use app::engine::{Engine, GraphicsWindow, WinNode};
use app::scott_session::ScottSession;

use crate::fixture_paths::fixture_path;

/// One Atari bitmap title, booted the way `startup.rs` boots it: side A
/// mounted, `ScottPictureSources::resolve` pairs it with its own side B and
/// reads the picture table against it.
fn atari_session(side_a_file: &str) -> Option<ScottSession> {
    let path = fixture_path(&format!("scott-dialects/atari/{side_a_file}"));
    if !path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/atari/ (gitignored commercial fixtures)");
        return None;
    }
    let mounted = app::hints::load_mounted_story_full(&path, None)
        .unwrap_or_else(|e| panic!("{side_a_file} should mount and hold one Scott database: {e}"));
    let app::hints::LoadedStory::Scott(bytes) = mounted.story else {
        panic!("{side_a_file}'s story is a Scott Adams database");
    };
    let game_dir = path.parent().expect("a directory").to_path_buf();
    let pictures = app::graphics::ScottPictureSources::resolve(&path, &bytes, &game_dir, None, None, None);
    assert!(
        pictures.atari_side_b.is_some(),
        "{side_a_file}: side B should pair through saga_companion_side"
    );
    Some(
        ScottSession::new_with_options(bytes, false, None, scott::Options::default(), pictures)
            .unwrap_or_else(|e| panic!("{side_a_file} should boot off its own side A: {e}")),
    )
}

fn picture_band(model: &app::engine::ScreenModel) -> Option<&GraphicsWindow> {
    match &model.root {
        WinNode::Pair { first, .. } => match &**first {
            WinNode::Graphics(gw) => Some(gw),
            _ => None,
        },
        _ => None,
    }
}

/// How many DISTINCT RGBA values a band's canvas holds — the non-vacuity bar
/// `saga_atari_specimens.rs`'s own pinned-picture cases use, applied here to
/// the composited band a player actually sees rather than to a bare decode.
fn distinct_colours(gw: &GraphicsWindow) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in gw.canvas.pixels() {
        seen.insert(p.0);
    }
    seen.len()
}

/// Boot each of the three titles, dismiss the SQ-1495 title card with one
/// blank command, and check the room band that reaches the player is real
/// art — several colours, not a blank or near-blank canvas — and that
/// `/dump-windows` reports the family-C Atari source rather than "none".
#[test]
fn each_bitmap_title_boots_and_shows_real_room_art() {
    for (side_a_file, adventure) in [
        ("SAGA #5 - The Count [side A].atr", "the Count"),
        ("SAGA #4 - Voodoo Castle [side A].atr", "Voodoo Castle"),
        ("SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side A.atr", "Claymorgue Castle"),
    ] {
        let Some(mut session) = atari_session(side_a_file) else { return };

        // At boot the title card is up (SQ-1495) — real art too, but a
        // different frame from the room's own, so dismiss it first with one
        // blank command the way a player's first keypress would.
        let _ = session.submit("");

        let model = session.screen();
        let gw = picture_band(&model)
            .unwrap_or_else(|| panic!("{adventure}: room 1 should show a picture band"));
        assert_eq!(
            (gw.canvas.width(), gw.canvas.height()),
            (scott::saga_pictures::CANVAS_WIDTH as u32, scott::saga_pictures::CANVAS_HEIGHT as u32),
            "{adventure}: the band is the family-C canvas"
        );
        assert!(
            distinct_colours(gw) >= 3,
            "{adventure}: room 1's band should show real multi-colour art, not a blank canvas"
        );

        let dump = session.window_dump().join("\n");
        assert!(
            dump.contains("S.A.G.A. family C (Atari 8-bit,"),
            "{adventure}: /dump-windows should name the Atari family-C source, got:\n{dump}"
        );
    }
}

/// The SQ-1495 title card itself is Atari artwork too — picture index 99,
/// reached through the very same table [`each_bitmap_title_boots_and_shows_real_room_art`]
/// exercises for room 1.
#[test]
fn the_title_card_is_showing_at_boot_before_any_command() {
    let Some(session) = atari_session("SAGA #5 - The Count [side A].atr") else { return };
    let model = session.screen();
    let gw = picture_band(&model).expect("the title card should show a picture band at boot");
    assert!(distinct_colours(gw) >= 3, "the title card should be real art, not a blank canvas");
}

/// **SQ-1498's fallback reaches the band.** *The Count*'s room 6 (CRYPT) and
/// room 16 (Dungeon) are the two records the ordinary scan cannot read at
/// all — real damage on this specimen, not a decoder bug
/// (`saga_atari_specimens.rs` pins the byte-exact shape) — and the picture
/// table names both anyway. Asked directly of the [`app::graphics::PictSource`]
/// the band itself draws from, both should still answer with real art rather
/// than nothing.
#[test]
fn the_counts_two_damaged_records_still_reach_the_band() {
    let a_path = fixture_path("scott-dialects/atari/SAGA #5 - The Count [side A].atr");
    let b_path = fixture_path("scott-dialects/atari/SAGA #5 - The Count [side B].atr");
    if !a_path.exists() || !b_path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/atari/ (gitignored commercial fixtures)");
        return;
    }
    let side_a = std::fs::read(&a_path).expect("side A reads");
    let side_b = std::fs::read(&b_path).expect("side B reads");
    let release = scott::SagaUs { version: 115, adventure: 5, platform: scott::SagaPlatform::Atari8Bit };
    let mut picts = app::graphics::PictSource::from_scott_saga_atari(&side_a, &side_b, release)
        .expect("the Count's picture table should read");
    for (room, name) in [(6u32, "room 6 (CRYPT)"), (16, "room 16 (Dungeon)")] {
        let img = picts
            .image(room)
            .unwrap_or_else(|| panic!("{name} should reach the band via the SQ-1498 fallback"));
        let rgba = img.to_rgba8();
        let distinct: std::collections::HashSet<[u8; 4]> = rgba.pixels().map(|p| p.0).collect();
        assert!(
            distinct.len() >= 3,
            "{name}'s SQ-1498 fallback decode should be real (if slightly marred) art, \
             not a blank/near-blank canvas: {} distinct colours",
            distinct.len()
        );
    }
}

/// **Falsification**: a picture table this crate cannot verify must refuse
/// the whole source rather than draw through garbage — corrupt side A's own
/// marker in front of the table (the same byte
/// `scott::saga_atari::PICTURE_TABLE_MARKER` names) and
/// [`app::graphics::PictSource::from_scott_saga_atari`] must answer `None`,
/// the same way a differently-mastered disk would.
///
/// This is the deliberate break-it-and-watch-it-fail check CLAUDE.md's
/// "falsify fixes" convention asks for: temporarily breaking the table's own
/// arithmetic (here, its marker) must be visible as a boot failure rather
/// than silently reading nothing as "no pictures on this release".
#[test]
fn a_corrupted_table_marker_refuses_the_whole_atari_source() {
    let a_path = fixture_path("scott-dialects/atari/SAGA #5 - The Count [side A].atr");
    let b_path = fixture_path("scott-dialects/atari/SAGA #5 - The Count [side B].atr");
    if !a_path.exists() || !b_path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/atari/ (gitignored commercial fixtures)");
        return;
    }
    let mut side_a = std::fs::read(&a_path).expect("side A reads");
    let side_b = std::fs::read(&b_path).expect("side B reads");
    let release = scott::SagaUs { version: 115, adventure: 5, platform: scott::SagaPlatform::Atari8Bit };

    // Premise: the real bytes DO produce a source.
    assert!(
        app::graphics::PictSource::from_scott_saga_atari(&side_a, &side_b, release).is_some(),
        "premise: the real disk should read"
    );

    side_a[scott::saga_atari::PICTURE_TABLE_MARKER] ^= 0xFF;
    assert!(
        app::graphics::PictSource::from_scott_saga_atari(&side_a, &side_b, release).is_none(),
        "a corrupted marker should refuse the whole table rather than read past it"
    );
}

/// The object-overlay round trip through the synthetic `"atari:<offset>"`
/// name (`app::graphics`'s own doc on why there is one) really composites —
/// *The Count*'s object 6 (a sheet going into a window, drawn in a room)
/// painted over room 1's own picture must change pixels within its own
/// rectangle.
#[test]
fn an_atari_object_overlay_composites_over_a_room_picture() {
    let a_path = fixture_path("scott-dialects/atari/SAGA #5 - The Count [side A].atr");
    let b_path = fixture_path("scott-dialects/atari/SAGA #5 - The Count [side B].atr");
    if !a_path.exists() || !b_path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/atari/ (gitignored commercial fixtures)");
        return;
    }
    let side_a = std::fs::read(&a_path).expect("side A reads");
    let side_b = std::fs::read(&b_path).expect("side B reads");
    let release = scott::SagaUs { version: 115, adventure: 5, platform: scott::SagaPlatform::Atari8Bit };
    let mut picts = app::graphics::PictSource::from_scott_saga_atari(&side_a, &side_b, release)
        .expect("the Count's picture table should read");

    let overlays = picts.scott_overlays(scott::PictureUsage::ObjectInRoom, &[6]);
    assert_eq!(overlays.len(), 1, "object 6 should resolve to exactly one record");

    let plain = picts.image(1).expect("room 1's own picture should decode").to_rgba8();
    let composite = picts.scott_composite(1, &overlays).expect("the composite should decode").to_rgba8();
    assert_ne!(plain.as_raw(), composite.as_raw(), "the overlay should really change pixels");
}
