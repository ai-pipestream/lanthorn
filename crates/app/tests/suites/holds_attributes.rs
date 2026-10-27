//! SQ-1792: whether an object is a container and/or a supporter, and open or
//! shut, carried through `Introspect` on every engine that can say.
//!
//! A host choosing between a verb's "put X in Y" and "put X on Y" lines needs
//! this per in-scope object. Where it comes from, by family (the field docs on
//! `grammar_model::Holds` are the contract):
//!
//! * **Inform 6 and 7** name every attribute in the story's own *Table of
//!   Identifier Names*, so the numbers are READ (`zvm::objects::attribute_names`,
//!   `gvm::objects::attribute_number`).
//! * **Infocom** leaves no names: only `open` is reported, from the inferred
//!   openness bit; `container` and `supporter` are `None`.
//!
//! # The game is the oracle
//!
//! Every flip below is driven through the real story and the bit read back
//! through the seam: the object whose `open` changed is found by diffing the
//! whole object table around one command, so no hard-coded object number can be
//! wrong.
//!
//! # The specimens
//!
//! | fixture | engine / compiler | turns in | what it shows |
//! |---|---|---|---|
//! | `minizork-r34-s871124.z3` (fetched) | Z-machine, ZIL | 0 | `open mailbox` flips `open`; no container/supporter |
//! | `anchor.z8` (fetched) | Z-machine, Inform 6 | past the intro, 0 | worn trenchcoat is a container, `table` a supporter, `close umbrella` flips `open` |
//! | `AnchorheadDemo.gblorb` (fetched) | Glulx, Inform 6 | past the splash, 0 | the same, on the other back-end |
//! | `Never Gives Up Her Dead.gblorb` (fetched) | Glulx, Inform 7 | past the splash, 0 | the carried rucksack is a closed container; `open rucksack` flips `open`; tables are supporters |
//!
//! All four are fetched fixtures; each case skips vacuously when its fixture is
//! absent and carries a non-vacuity guard on the shape it asserts.

use std::collections::BTreeMap;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::{GameSession, InputKind};
use grammar_model::{Holds, ObjectWords};

use crate::fixture_paths::fixture_path;

// ── Booting ─────────────────────────────────────────────────────────────────

fn zsession(name: &str, tap_intro: bool) -> Option<GameSession> {
    let bytes = std::fs::read(fixture_path(name)).ok()?;
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("the story boots");
    if tap_intro {
        for _ in 0..6 {
            if s.pending_input() != InputKind::Char {
                break;
            }
            let _ = s.submit_char(13);
        }
    }
    Some(s)
}

fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(fixture_path(name)).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    match blorb::Blorb::parse(bytes).ok()?.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

fn gsession(name: &str) -> Option<GlulxSession> {
    let mut s = GlulxSession::new(glulx_image(name)?, 80, 24, true, false, false, (1.0, 1.0), None, &[]).ok()?;
    for _ in 0..6 {
        if s.pending_input() != InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some(s)
}

// ── Reading ─────────────────────────────────────────────────────────────────

fn all_objects(engine: &dyn Engine) -> Vec<ObjectWords> {
    engine
        .introspect()
        .expect("the story has an object tree")
        .all_object_words()
        .expect("the story's objects can be walked")
}

/// Every object's `holds`, keyed by its engine id.
fn holds_by_id(engine: &dyn Engine) -> BTreeMap<u32, Option<Holds>> {
    all_objects(engine).into_iter().map(|o| (o.id, o.holds)).collect()
}

fn object<'a>(all: &'a [ObjectWords], word: &str, pred: impl Fn(&Holds) -> bool) -> &'a ObjectWords {
    all.iter()
        .find(|o| o.holds.as_ref().is_some_and(&pred) && o.words.iter().any(|w| w == word))
        .unwrap_or_else(|| panic!("no object answering to `{word}` with the expected holds"))
}

/// Run `command` and return the objects whose `open` bit changed, as `(object, before, after)`.
fn open_flips(
    engine: &mut dyn Engine,
    command: &str,
) -> Vec<(ObjectWords, bool, bool)> {
    let before = holds_by_id(engine);
    let _ = engine.submit(command);
    all_objects(engine)
        .into_iter()
        .filter_map(|o| {
            let was = before.get(&o.id).copied().flatten()?.open?;
            let now = o.holds?.open?;
            (was != now).then_some((o, was, now))
        })
        .collect()
}

// ── Infocom ─────────────────────────────────────────────────────────────────

/// Mini-Zork's mailbox: `open` flips with the game, and neither `container` nor
/// `supporter` is ever claimed (a ZIL table carries the "holds things" bit too,
/// so it cannot say container-not-surface).
#[test]
fn an_infocom_story_reports_open_flipping_and_never_container_or_supporter() {
    let Some(mut s) = zsession("minizork-r34-s871124.z3", false) else { return };
    let _ = s.submit("");

    let flips = open_flips(&mut s, "open mailbox");
    assert_eq!(flips.len(), 1, "exactly the mailbox opened: {:?}", flips.iter().map(|f| &f.0.printed_name).collect::<Vec<_>>());
    let (mailbox, was, now) = &flips[0];
    assert!(mailbox.refers_to("mailbox"), "the object that opened is the mailbox: {mailbox:?}");
    assert_eq!((*was, *now), (false, true));
    assert_eq!(mailbox.holds, Some(Holds::new(None, None, Some(true))));

    let flips = open_flips(&mut s, "close mailbox");
    assert_eq!(flips.len(), 1);
    assert_eq!((flips[0].1, flips[0].2), (true, false));

    let all = all_objects(&s);
    assert!(all.len() > 50, "non-vacuity: the whole object table was walked");
    assert!(all.iter().all(|o| o.holds.is_some_and(|h| h.container.is_none() && h.supporter.is_none())));
}

// ── Inform 6, Z-machine ─────────────────────────────────────────────────────

#[test]
fn an_inform_6_z_code_story_reports_container_supporter_and_open() {
    let Some(mut s) = zsession("anchor.z8", true) else { return };
    let all = all_objects(&s);
    // A table is a supporter and not a container; the player's worn trenchcoat is a container.
    let table = object(&all, "table", |h| h.supporter == Some(true));
    assert_eq!(table.holds.unwrap().container, Some(false), "{table:?}");
    // The coat's `name` is a `parse_name` routine, so it has no words and is not in the walk
    // above — but it is in scope, and the in-scope list still carries what it is.
    let intro = s.introspect().expect("object tree");
    let carried = intro.contents(intro.player_object().expect("the player is an object"));
    let coat = carried.iter().find(|o| o.printed_name == "trenchcoat").expect("the player wears a trenchcoat");
    assert!(coat.words.is_empty(), "non-vacuity: this is the wordless case");
    assert_eq!(coat.holds, Some(Holds::new(Some(true), Some(false), Some(true))), "{coat:?}");

    // `close umbrella` / `open umbrella` flip exactly the umbrella.
    let flips = open_flips(&mut s, "close umbrella");
    assert_eq!(flips.len(), 1, "{flips:?}");
    assert!(flips[0].0.refers_to("umbrella"));
    assert_eq!((flips[0].1, flips[0].2), (true, false));
    let flips = open_flips(&mut s, "open umbrella");
    assert_eq!(flips.len(), 1, "{flips:?}");
    assert_eq!((flips[0].1, flips[0].2), (false, true));
}

// ── Inform 6, Glulx ─────────────────────────────────────────────────────────

#[test]
fn an_inform_6_glulx_story_reports_container_supporter_and_open() {
    let Some(mut s) = gsession("AnchorheadDemo.gblorb") else { return };
    let all = all_objects(&s);
    let coat = object(&all, "trenchcoa", |h| h.container == Some(true));
    assert_eq!(coat.holds.unwrap().supporter, Some(false), "{coat:?}");
    assert!(
        all.iter().any(|o| o.holds.is_some_and(|h| h.supporter == Some(true) && h.container == Some(false))),
        "non-vacuity: the demo has a pure supporter"
    );
    assert!(
        all.iter().any(|o| o.holds.is_some_and(|h| h.container == Some(true) && h.supporter == Some(false))),
        "non-vacuity: the demo has a pure container"
    );

    let flips = open_flips(&mut s, "close umbrella");
    assert_eq!(flips.len(), 1, "{flips:?}");
    assert!(flips[0].0.refers_to("umbrella"));
    assert_eq!((flips[0].1, flips[0].2), (true, false));
}

// ── Inform 7, Glulx ─────────────────────────────────────────────────────────

/// Never Gives Up Her Dead opens with the player carrying "a rucksack (closed)".
#[test]
fn an_inform_7_story_reports_container_supporter_and_open() {
    let Some(mut s) = gsession("Never Gives Up Her Dead.gblorb") else { return };
    let all = all_objects(&s);
    let table = object(&all, "table", |h| h.supporter == Some(true));
    assert_eq!(table.holds.unwrap().container, Some(false), "{table:?}");

    // The avatar is found by watching a turn (`Introspect::player_object`), so play one.
    let _ = s.submit("look");
    let _ = s.submit("inventory");
    let intro = s.introspect().expect("object tree");
    let carried = intro.contents(intro.player_object().expect("the player is an object"));
    let rucksack = carried.iter().find(|o| o.refers_to("rucksack")).expect("the player carries a rucksack");
    assert_eq!(rucksack.holds, Some(Holds::new(Some(true), Some(false), Some(false))), "closed at the start");

    let flips = open_flips(&mut s, "open rucksack");
    assert_eq!(flips.len(), 1, "exactly the rucksack opened: {flips:?}");
    let (opened, was, now) = &flips[0];
    assert_eq!(opened.id, rucksack.id);
    assert_eq!((*was, *now), (false, true));
    assert_eq!(opened.holds, Some(Holds::new(Some(true), Some(false), Some(true))));
}
