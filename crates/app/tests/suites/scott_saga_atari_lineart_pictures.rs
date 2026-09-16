//! SQ-1524/SQ-1525: the Atari 8-bit US S.A.G.A. LINE-ART releases' own room
//! and object artwork, wired all the way to the picture band —
//! *Adventureland*, *Pirate Adventure*, *Mission Impossible*, *Strange
//! Odyssey*.
//!
//! `crates/scott/tests/saga_atari_specimens.rs` pins the (usage, index) table
//! reader and the drawing grammar at the decoder level, against the same four
//! real specimens this file uses — see that file for the per-title tables and
//! the exact byte-level findings, including the two host-side facts SQ-1525's
//! own note names (the darkness card's paused-frame rule and the six room
//! records that never clear). This file is the other half: that the table
//! reaches `ScottSession`'s picture band the way `startup.rs` actually boots
//! one, and that the two host-side facts and the object draw order survive
//! the trip through `app::graphics::PictSource`.
//!
//! All four fixtures are gitignored commercial disk images; every case here
//! skips vacuously with an explanation when `stories/scott-dialects/atari/`
//! is absent.

use app::engine::{Engine, GraphicsWindow, WinNode};
use app::scott_session::ScottSession;

use crate::fixture_paths::fixture_path;

/// One line-art title, booted the way `startup.rs` boots it: side A mounted,
/// `ScottPictureSources::resolve` pairs it with its own side B and reads the
/// line-art table against side B itself (unlike the bitmap titles', which
/// live on side A — `scott::saga_atari`'s module docs).
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

/// The four titles' side-A file names and their (adventure, version) release
/// identity — the same specimens and numbers `saga_atari_specimens.rs` pins.
const LINE_ART_TITLES: [(&str, &str, u16, u16); 4] = [
    ("SAGA #1 - Adventureland [side A].atr", "Adventureland", 1, 416),
    ("SAGA #2 - Pirate Adventure [side A].atr", "Pirate Adventure", 2, 408),
    ("SAGA #3 - Mission Impossible [side A].atr", "Mission Impossible", 3, 306),
    ("SAGA #6 - Strange Odyssey [side A].atr", "Strange Odyssey", 6, 119),
];

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
/// the composited band a player actually sees.
fn distinct_colours(gw: &GraphicsWindow) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in gw.canvas.pixels() {
        seen.insert(p.0);
    }
    seen.len()
}

/// The release identity to hand [`app::graphics::PictSource::from_scott_saga_atari_lineart`]
/// directly — bypassing the VM, which is what the sub-checks below need:
/// they want to reach specific records (the darkness card, a never-clearing
/// room, a room-object pair) that a scripted `submit()` walkthrough has no
/// reliable way to.
fn atari_lineart_release(adventure: u16, version: u16) -> scott::SagaUs {
    scott::SagaUs { version, adventure, platform: scott::SagaPlatform::Atari8Bit }
}

/// A title's own side B, read whole, or `None` with a reason on stderr when
/// either side of the pair is absent.
fn read_side_b(side_a_file: &str) -> Option<Vec<u8>> {
    let a_path = fixture_path(&format!("scott-dialects/atari/{side_a_file}"));
    let b_file = side_a_file.replace("side A", "side B");
    let b_path = fixture_path(&format!("scott-dialects/atari/{b_file}"));
    if !a_path.exists() || !b_path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/atari/ (gitignored commercial fixtures)");
        return None;
    }
    Some(std::fs::read(&b_path).expect("side B reads"))
}

/// Boot each of the four titles, dismiss the SQ-1495 title card with one
/// blank command, and check the room band that reaches the player is real
/// art — several colours, not a blank canvas and not the debunked "same
/// small picture on every title" `decode_line_art_opening` used to answer
/// with (SQ-1524 retired that reading) — and that `/dump-windows` names the
/// line-art source rather than "family C" or "none".
///
/// **Mission Impossible does not appear here.** Its side A is the
/// known-damaged database SQ-1524's own investigation note names (§12.13,
/// ~50 corrupt bytes) — `parse_saga_us` refuses it, so it cannot mount
/// through `ScottSession::new_with_options` at all, which is a pre-existing
/// gap in the database this quest did not introduce and is not the one to
/// fix here. `mission_impossibles_line_art_table_still_reads_off_side_b`
/// below covers its picture wiring directly, off side B alone, exactly as
/// SQ-1524's note says a lane must: "the implementation lane must not expect
/// to load MI's side A through the strict parser."
#[test]
fn each_line_art_title_boots_and_shows_real_room_art() {
    for (side_a_file, adventure, adv_num, version) in LINE_ART_TITLES {
        if adventure == "Mission Impossible" {
            continue;
        }
        let Some(mut session) = atari_session(side_a_file) else { return };
        let _ = session.submit("");

        let model = session.screen();
        let gw = picture_band(&model)
            .unwrap_or_else(|| panic!("{adventure}: the start room should show a picture band"));
        assert_eq!(
            (gw.canvas.width(), gw.canvas.height()),
            (
                scott::saga_atari_lineart::CANVAS_WIDTH as u32,
                scott::saga_atari_lineart::CANVAS_HEIGHT as u32,
            ),
            "{adventure}: the band is the line-art canvas, not family C's"
        );
        assert!(
            distinct_colours(gw) >= 3,
            "{adventure}: the start room's band should show real multi-colour art, not a blank canvas"
        );

        let dump = session.window_dump().join("\n");
        assert!(
            dump.contains("S.A.G.A. line-art (Atari 8-bit,"),
            "{adventure}: /dump-windows should name the line-art source, got:\n{dump}"
        );
        let _ = (adv_num, version);
    }
}

/// *Mission Impossible*'s picture wiring, without going through the mount —
/// its own line-art table lives entirely on side B (SQ-1524), so it needs
/// nothing from the damaged side-A database at all.
#[test]
fn mission_impossibles_line_art_table_still_reads_off_side_b() {
    let Some(side_b) = read_side_b("SAGA #3 - Mission Impossible [side A].atr") else { return };
    let release = atari_lineart_release(3, 306);
    let mut picts = app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release)
        .expect("Mission Impossible's line-art table should read off side B alone");
    // Room 2 is the start room (SQ-1524's own note: "the office desk").
    let img = picts.image(2).expect("room 2 should decode").to_rgba8();
    let distinct: std::collections::HashSet<[u8; 4]> = img.pixels().map(|p| p.0).collect();
    assert!(distinct.len() >= 3, "room 2 should be real multi-colour art, not a blank canvas");
}

/// **SQ-1525's first host-side fact, reached through the wiring**: the
/// shared darkness card (index 0) must show its animation's LAST PAUSED
/// frame, never the true final (black) one.
#[test]
fn the_darkness_card_shows_the_paused_frame_not_black() {
    let Some(side_b) = read_side_b("SAGA #1 - Adventureland [side A].atr") else { return };
    let release = atari_lineart_release(1, 416);
    let mut picts = app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release)
        .expect("Adventureland's line-art table should read");

    let dark = picts
        .scott_composite(scott::DARKNESS_PICTURE as u32, &[])
        .expect("the darkness card should reach the band");
    let rgba = dark.to_rgba8();
    let distinct: std::collections::HashSet<[u8; 4]> = rgba.pixels().map(|p| p.0).collect();
    assert!(
        distinct.len() >= 2,
        "the darkness card should show its lettering (the paused frame), not a plain black canvas: \
         {} distinct colours",
        distinct.len()
    );

    // Falsification: the TRUE final frame (what a naive `LineArtCanvas::draw`
    // would give instead of `draw_darkness_card`) is a plain black clear —
    // pin that here too, against the crate-level suite's own count.
    let spliced = scott::saga_atari::splice_vtoc(&side_b);
    let table = scott::saga_atari::read_line_art_table(&spliced, 1).expect("the table reads");
    let off = table.find(scott::PictureUsage::Room, 0).expect("the darkness card's own entry");
    let stream = &spliced[scott::saga_atari::spliced_of(off)..];
    let naive = scott::saga_atari_lineart::LineArtCanvas::new().draw(stream).expect("plays");
    let naive_distinct: std::collections::HashSet<_> =
        naive.pixels().iter().copied().collect();
    assert_eq!(naive_distinct.len(), 1, "the true final frame is plain black — the bug this avoids");
}

/// **SQ-1525's second host-side fact, reached through the wiring**: one of
/// the six never-clearing room records (*Pirate Adventure*'s room 86)
/// composites over whatever the running canvas already held, rather than
/// painting a fresh black canvas — driven through the same
/// [`app::graphics::PictSource::scott_composite`] `ScottSession`'s own band
/// refresh calls.
#[test]
fn a_never_clearing_room_composites_through_scott_composite() {
    let Some(side_b) = read_side_b("SAGA #2 - Pirate Adventure [side A].atr") else { return };
    let release = atari_lineart_release(2, 408);

    // The wrong approach: room 86 as the very first thing composited — a
    // fresh source, so its own running canvas starts black.
    let mut fresh = app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release)
        .expect("Pirate Adventure's line-art table should read");
    let fresh_img = fresh.scott_composite(86, &[]).expect("room 86 should reach the band alone");

    // The composite: room 20 first (a normal, clearing room), then room 86
    // on the SAME source — its running canvas carries room 20's picture in.
    let mut picts = app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release)
        .expect("Pirate Adventure's line-art table should read");
    let _ = picts.scott_composite(20, &[]).expect("room 20 should reach the band");
    let composited_img = picts.scott_composite(86, &[]).expect("room 86 should reach the band over it");

    assert_ne!(
        fresh_img.to_rgba8().as_raw(),
        composited_img.to_rgba8().as_raw(),
        "room 86's band should differ depending on what was drawn before it — that is the whole point \
         of never clearing the screen"
    );
}

/// **Object draw order** (SQ-1525): `PictSource::scott_overlays` must return
/// object records sorted by DESCENDING item index, so `scott_composite`
/// draws the highest index first and item 0 last — on top.
#[test]
fn line_art_object_overlays_are_ordered_descending_item_zero_last() {
    let Some(side_b) = read_side_b("SAGA #1 - Adventureland [side A].atr") else { return };
    let release = atari_lineart_release(1, 416);
    let picts = app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release)
        .expect("Adventureland's line-art table should read");

    // Objects 0 and 2 both have artwork on Adventureland (LINE_ART_TABLES,
    // crate-level suite). Ask for them in ascending order and expect the
    // descending answer back.
    let overlays = picts.scott_overlays(scott::PictureUsage::ObjectInRoom, &[0, 2]);
    assert_eq!(overlays.len(), 2, "both objects should resolve to a record");
    // The synthetic "atari:<offset>" names do not sort by index themselves,
    // so decode which is which by asking the table directly and comparing.
    let off = |index: u16| {
        picts
            .scott_overlays(scott::PictureUsage::ObjectInRoom, &[index])
            .into_iter()
            .next()
            .expect("resolves")
    };
    assert_eq!(overlays, vec![off(2), off(0)], "item 2 first, item 0 last — descending, item 0 on top");

    // The same query for the INVENTORY usage answers the same two records —
    // SQ-1524: no inventory flag bit here, one picture serves both uses.
    let inv_overlays = picts.scott_overlays(scott::PictureUsage::ObjectInInventory, &[0, 2]);
    assert_eq!(inv_overlays, overlays, "the inventory usage should resolve the same records");
}

/// An object overlay really composites over a room picture — *Adventureland*
/// room 1's own picture, with object 0 (`Dark hole`) painted over it, must
/// change pixels within its own rectangle (the 17-pixel, 132-136 x 71-76 box
/// this crate measured).
#[test]
fn a_line_art_object_overlay_composites_over_a_room_picture() {
    let Some(side_b) = read_side_b("SAGA #1 - Adventureland [side A].atr") else { return };
    let release = atari_lineart_release(1, 416);
    let mut picts = app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release)
        .expect("Adventureland's line-art table should read");

    let overlays = picts.scott_overlays(scott::PictureUsage::ObjectInRoom, &[0]);
    assert_eq!(overlays.len(), 1, "object 0 should resolve to exactly one record");

    let plain = picts.image(1).expect("room 1's own picture should decode").to_rgba8();
    let composite = picts.scott_composite(1, &overlays).expect("the composite should decode").to_rgba8();
    assert_ne!(plain.as_raw(), composite.as_raw(), "the overlay should really change pixels");
}

/// **Falsification**: a line-art table this crate cannot verify must refuse
/// the whole source rather than draw through garbage — corrupt side B's own
/// marker in front of the table and
/// [`app::graphics::PictSource::from_scott_saga_atari_lineart`] must answer
/// `None`, the same way [`app::graphics::PictSource::from_scott_saga_atari`]
/// does for the bitmap titles.
#[test]
fn a_corrupted_line_art_table_marker_refuses_the_whole_source() {
    let Some(side_b) = read_side_b("SAGA #1 - Adventureland [side A].atr") else { return };
    let release = atari_lineart_release(1, 416);

    assert!(
        app::graphics::PictSource::from_scott_saga_atari_lineart(&side_b, release).is_some(),
        "premise: the real disk should read"
    );

    let mut corrupted = side_b;
    corrupted[scott::saga_atari::LINE_ART_TABLE_MARKER] ^= 0xFF;
    assert!(
        app::graphics::PictSource::from_scott_saga_atari_lineart(&corrupted, release).is_none(),
        "a corrupted marker should refuse the whole table rather than read past it"
    );
}
