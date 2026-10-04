//! SQ-1653: Superluminal Vagrant Twin's `map` and `prospects` commands minting a phantom
//! room on the map — a room-DETECTION bug, not a mapper bug: `TurnResult.location` itself
//! names the wrong room, so any host consuming it directly (not only lanthorn's own TUI)
//! gets a phantom room every time the player checks the in-game map or trading log.
//!
//! # The report
//!
//! Typing `map` (or `prospects`) sets `TurnResult.location` to a bogus room — "Other
//! Worlds" / "Other Opportunities:" — even though the player never moved. A subsequent
//! real `look` correctly reports the actual current room again.
//!
//! # The mechanism
//!
//! `map` prints `"VISITED WORLDS\n\nOther Worlds\nBoony (not yet landed)"`, all in the
//! `Subheader` style Inform 7 rooms are printed in. `"Other Worlds\nBoony (not yet
//! landed)"` is styled and shaped EXACTLY like a real room heading joined to its
//! description (SQ-1625's own-line-heading-then-immediate-body rule) — nothing in the
//! buffer text tells the two apart. `prospects` does the same:
//! `"Known Merchants:\n-None\n\nOther Opportunities:\n-None known"`.
//!
//! Two signals that corroborate a heading elsewhere in this file were checked and don't
//! apply here: `AppGlk::status_room_name` never names a room for this story at all — its
//! status grid reads `"Credits: 0k ... Fuel: 2/5"`, never a room — so
//! `GlulxSession::refuse_banner_the_status_line_contradicts` never gets a vote; and
//! `crate::glulx_roomlock::RoomLock` never locks for this story even after several real
//! `land`/`launch` moves, so `GlulxSession::adopt_heading_for_room`'s "only adopt a
//! locked-and-moved heading" guard never engages either.
//!
//! The actual fix is `AppGlk::heading_is_a_later_distinct_entry` (`glk_backend.rs`, and its
//! own `heading_tests` unit cases): a SECOND, differently-named own-line `Subheader`
//! heading appearing in the same turn as an earlier one is a listing (a page title over
//! item entries), not a room arrival — but only once a room is already known
//! (`GlulxSession::finish_turn`), which `map`/`prospects` always follow. A game's OWN
//! title banner joined to its credits block legitimately precedes the opening room on the
//! turn a title menu is dismissed — *King of Shreds and Patches* is exactly this shape —
//! so the same signal on the very first room a session ever names is not refused;
//! `room_description::glulx_king_of_shreds_and_patches_description_excludes_the_letter_and_banner_that_precede_it`
//! is the non-vacuity guard for that half, and is what caught the first version of this
//! fix reaching too far.
//!
//! Every case skips vacuously without `stories/` (gitignored).

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

fn boot() -> Option<GlulxSession> {
    let path = fixture_path("Superluminal_Vagrant_Twin.gblorb.blorb");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let blorb = blorb::Blorb::parse(bytes).expect("Superluminal Vagrant Twin parses as a Blorb");
    let (kind, exec) = blorb.executable().expect("Superluminal Vagrant Twin carries an executable");
    assert_eq!(kind, blorb::ExecKind::Glulx, "Superluminal Vagrant Twin is a Glulx story");
    let store = app::scratch_dir("sq1653-superluminal");
    let mut s = GlulxSession::new_in(
        store,
        exec.to_vec(),
        80,
        24,
        true,
        false,
        false,
        false,
        (1.0, 1.0),
        None,
        &[],
        [[(None, None); 11]; 2],
        false,
        Some(1),
    )
    .unwrap_or_else(|e| panic!("Superluminal Vagrant Twin boots: {e:?}"));
    for _ in 0..60 {
        if s.current_location().is_some() {
            break;
        }
        if s.pending_input() != InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some(s)
}

/// `map` must not move the map off the real room, and a `look` right after it must still
/// find the real room — the non-regression half (a refusal that reached too far would take
/// real headings with it too, the same falsifier `sq1351_nguhd_topics::the_map_still_follows_a_real_move`
/// uses for its own refusal).
#[test]
fn map_does_not_mint_a_phantom_room() {
    let Some(mut s) = boot() else { return };
    let r0 = Engine::submit(&mut s, "look");
    let real_room = r0.location.clone().expect("Superluminal names its opening room").name;
    assert_ne!(real_room, "Other Worlds", "premise: the real opening room is not the phantom");

    let r1 = Engine::submit(&mut s, "map");
    assert!(
        r1.transcript.contains("VISITED WORLDS") && r1.transcript.contains("Other Worlds"),
        "premise: the fixture still prints the stacked banner shape this quest is about\n{}",
        r1.transcript
    );
    assert_eq!(
        r1.location.as_ref().map(|l| l.name.as_str()),
        Some(real_room.as_str()),
        "`map` must not move the player: {:?}",
        r1.location
    );

    let r2 = Engine::submit(&mut s, "look");
    assert_eq!(
        r2.location.as_ref().map(|l| l.name.as_str()),
        Some(real_room.as_str()),
        "a `look` right after `map` still finds the real room: {:?}",
        r2.location
    );
}

/// `prospects` (its full name, `trading log`, prints nothing the first time it is asked —
/// `p`/`prospects` is the short form the game's own `about` text names) must not mint a
/// phantom room either.
#[test]
fn prospects_does_not_mint_a_phantom_room() {
    let Some(mut s) = boot() else { return };
    let r0 = Engine::submit(&mut s, "look");
    let real_room = r0.location.clone().expect("Superluminal names its opening room").name;

    let r1 = Engine::submit(&mut s, "p");
    assert!(
        r1.transcript.contains("Known Merchants:") && r1.transcript.contains("Other Opportunities:"),
        "premise: the fixture still prints the stacked banner shape this quest is about\n{}",
        r1.transcript
    );
    assert_eq!(
        r1.location.as_ref().map(|l| l.name.as_str()),
        Some(real_room.as_str()),
        "`prospects` must not move the player: {:?}",
        r1.location
    );

    let r2 = Engine::submit(&mut s, "look");
    assert_eq!(
        r2.location.as_ref().map(|l| l.name.as_str()),
        Some(real_room.as_str()),
        "a `look` right after `prospects` still finds the real room: {:?}",
        r2.location
    );
}
