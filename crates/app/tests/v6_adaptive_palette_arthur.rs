//! Arthur (Infocom v6): a palette change must recolour adaptive pictures that are
//! ALREADY on screen (Blorb spec §11.3, SQ-0567).
//!
//! Arthur's decorative frame — the border art the location/date header sits in — is
//! its three `APal` adaptive pictures (54, 170, 171), drawn ONCE during the intro
//! and never again. An adaptive picture is plotted with the "Current Palette" (the
//! palette of the most recently drawn NON-adaptive picture), so when a scene
//! establishes a new one the frame is meant to follow it. Each scene carries its
//! own 16-colour PLTE:
//!
//! - churchyard — Pict 4, blue-dominant
//! - church     — Pict 10 (`ddaa88`, `775500`), brown
//! - hiding behind the gravestone — Pict 7 (`7080f0`), a different blue
//!
//! babelmap re-decoded an adaptive picture with the Current Palette only when the
//! game DREW it, so Arthur's frame kept the churchyard palette for the whole game:
//! it stayed blue in the brown church and never shifted when the gravestone scene
//! swapped the blues.
//!
//! Asserted on the window canvases, upstream of any rendering, and run in BOTH
//! `honor_game_colours` modes — the palette here comes from picture data, not from
//! the theme, and pinning both modes is what proves that independence.
//!
//! Skips cleanly when the gitignored story is absent (CI).

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

/// Boot Arthur past the sword-in-the-stone intro to the churchyard, where the
/// frame is drawn and the game is taking commands. `None` when the story is absent.
fn arthur_at_churchyard(honor_game_colours: bool) -> Option<GameSession> {
    let story_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session = GameSession::new_with_trace(
        story_bytes, honor_game_colours, false, None, false, picture_dims, std_window, None,
    )
    .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..12 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    let _ = session.take_transcript();
    Some(session)
}

/// Mean RGB of the frame window's opaque pixels, plus how many there are — the
/// colour of the band the header sits in.
fn frame_tint(session: &GameSession) -> ([u32; 3], u64) {
    let canvas = session.pictures_canvas.get(&7).expect("Arthur's frame is window 7");
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for p in canvas.img.pixels() {
        if p.0[3] != 0 {
            r += p.0[0] as u64;
            g += p.0[1] as u64;
            b += p.0[2] as u64;
            n += 1;
        }
    }
    let d = n.max(1);
    ([(r / d) as u32, (g / d) as u32, (b / d) as u32], n)
}

fn step(session: &mut GameSession, cmd: &str) -> String {
    let r = session.submit(cmd);
    r.transcript.trim().lines().next().unwrap_or("").trim().to_string()
}

#[test]
fn arthur_frame_follows_the_scene_palette() {
    for honor_game_colours in [true, false] {
        let Some(mut session) = arthur_at_churchyard(honor_game_colours) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let label = format!("honor_game_colours={honor_game_colours}");

        let (yard, yard_px) = frame_tint(&session);
        assert!(
            yard[2] > yard[0] && yard[2] > yard[1],
            "{label}: the churchyard frame is blue-dominant, got rgb{yard:?}"
        );

        // Into the church: its brown palette must recolour the frame drawn at boot.
        assert!(step(&mut session, "in").contains("CHURCH"), "{label}: entered the church");
        let (church, church_px) = frame_tint(&session);
        assert!(
            church[0] > church[1] && church[1] > church[2],
            "{label}: the church frame is brown (r > g > b), got rgb{church:?}"
        );
        assert_eq!(
            church_px, yard_px,
            "{label}: recoloured in place — the same pixels, not a moved or resized frame"
        );

        // Back out: the churchyard palette returns, exactly.
        assert!(step(&mut session, "west").contains("CHURCHYARD"), "{label}: back outside");
        assert_eq!(frame_tint(&session).0, yard, "{label}: the churchyard tint returns exactly");

        // Hiding swaps to a DIFFERENT blue palette — still blue, but not the same.
        let hid = step(&mut session, "hide behind gravestone");
        assert!(hid.contains("gravestone"), "{label}: hid behind the gravestone, got {hid:?}");
        let (hiding, _) = frame_tint(&session);
        assert!(
            hiding[2] > hiding[0] && hiding[2] > hiding[1],
            "{label}: still blue-dominant while hiding, got rgb{hiding:?}"
        );
        assert_ne!(
            hiding, yard,
            "{label}: but a different blue — the gravestone scene swaps the palette"
        );
    }
}
