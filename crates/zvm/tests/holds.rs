//! Whether an object holds things, and how — SQ-1792.
//!
//! `ObjectWords::holds` is READ from the story where its compiler names the
//! attributes (Inform 6/7: the *Table of Identifier Names*, see
//! `zvm::objects::attribute_names`) and INFERRED only for Infocom's openness
//! bit. The cases here run on fixtures CI always has: `anchor.z8` (Inform 6, names
//! present; FETCHED, so it skips vacuously when absent), `curses.z5` (Inform, no
//! names table) and `minizork.z3` (ZIL). The committed Inform fixtures here
//! (`praxix`, `etude`, `czech`, ...) were compiled without a names table.
//! The real-game play-through — `open mailbox` flipping the bit — is in the app
//! suite `holds_attributes`.

use zvm::memory::Memory;
use zvm::objects::{self, Holds, ParseNames};

fn fixture(name: &str) -> Memory {
    Memory::new(zvm::fixtures::load(name).expect("fixture CI always has")).unwrap()
}

/// A story from the app crate's fetched fixture set, or `None` so the case can skip.
fn fetched(name: &str) -> Option<Memory> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app/tests/fixtures/stories").join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    };
    Some(Memory::new(bytes).unwrap())
}

#[test]
fn an_inform_story_names_its_attributes_and_they_are_read_not_assumed() {
    let Some(mem) = fetched("anchor.z8") else { return };
    let names = objects::attribute_names(&mem).expect("Anchorhead carries its identifier names");
    assert_eq!(names.len(), 48);
    let container = objects::attribute_number(&mem, "container").expect("container is named");
    let supporter = objects::attribute_number(&mem, "supporter").expect("supporter is named");
    let open = objects::attribute_number(&mem, "open").expect("open is named");
    assert!(container != supporter && container != open && supporter != open);
    // The Inform 6 library declares `animate` first, so it is attribute 0 — the
    // anchor that proves the table is read from the right place.
    assert_eq!(objects::attribute_number(&mem, "animate"), Some(0));
    assert_eq!(objects::attribute_number(&mem, "no_such_attribute"), None);
}

/// An aliased attribute reads `a/b` (current inform6lib: `Attribute non_floating
/// alias absent`) and either spelling finds it; a v3 story has only 32 attributes.
#[test]
fn an_aliased_attribute_is_found_by_either_name_and_v3_stops_at_32() {
    let mut names: Vec<String> = vec![String::new(); 48];
    names[1] = "absent/non_floating".into();
    names[40] = "late".into();
    assert_eq!(objects::attribute_number_in(&names, 5, "absent"), Some(1));
    assert_eq!(objects::attribute_number_in(&names, 5, "non_floating"), Some(1));
    assert_eq!(objects::attribute_number_in(&names, 5, "non"), None, "whole components only");
    assert_eq!(objects::attribute_number_in(&names, 5, "late"), Some(40));
    assert_eq!(objects::attribute_number_in(&names, 3, "late"), None, "v3 objects carry 32 attributes");
}

#[test]
fn holds_follows_the_live_attribute_bits() {
    let Some(mut mem) = fetched("anchor.z8") else { return };
    let container = objects::attribute_number(&mem, "container").unwrap();
    let supporter = objects::attribute_number(&mem, "supporter").unwrap();
    let open = objects::attribute_number(&mem, "open").unwrap();
    let pn = ParseNames::detect(&mem).expect("Anchorhead has parse names");
    // Object 122 is its "table" (a supporter). Start from a known blank slate.
    for a in [container, supporter, open] {
        objects::clear_attr(&mut mem, 122, a);
    }
    let h = |mem: &Memory| pn.of(mem, 122).unwrap().holds;
    assert_eq!(h(&mem), Some(Holds::new(Some(false), Some(false), Some(false))));

    objects::set_attr(&mut mem, 122, container);
    assert_eq!(h(&mem), Some(Holds::new(Some(true), Some(false), Some(false))));
    objects::set_attr(&mut mem, 122, open);
    assert_eq!(h(&mem), Some(Holds::new(Some(true), Some(false), Some(true))));
    objects::clear_attr(&mut mem, 122, container);
    objects::set_attr(&mut mem, 122, supporter);
    assert_eq!(h(&mem), Some(Holds::new(Some(false), Some(true), Some(true))));
}

#[test]
fn an_inform_story_without_a_names_table_says_nothing_rather_than_guessing() {
    // curses.z5 is Inform-compiled with its symbol table omitted: `open` and
    // `openable` cannot be told apart from the object table, so it answers None.
    let mem = fixture("curses.z5");
    assert!(objects::attribute_names(&mem).is_none());
    let pn = ParseNames::detect(&mem).expect("curses has parse names");
    let all = pn.all(&mem);
    assert!(all.len() > 100, "non-vacuity: the story has objects");
    assert!(all.iter().all(|o| o.holds.is_none()));
}

#[test]
fn an_infocom_story_reports_open_and_never_container_or_supporter() {
    let mut mem = fixture("minizork.z3");
    assert!(objects::attribute_names(&mem).is_none(), "ZIL leaves no attribute names");
    let pn = ParseNames::detect(&mem).expect("Mini-Zork has parse names");
    let open = zvm::world::WorldModel::discover(&mem).open_attr.expect("Mini-Zork's openness bit is inferred");
    // Object 167 is the small mailbox (parse_names.rs).
    let mailbox = pn.of(&mem, 167).unwrap();
    assert_eq!(mailbox.holds, Some(Holds::new(None, None, Some(false))), "shut at boot");
    objects::set_attr(&mut mem, 167, open);
    assert_eq!(pn.of(&mem, 167).unwrap().holds, Some(Holds::new(None, None, Some(true))));

    // Non-vacuity, and the contract: nothing in the whole table claims container or supporter.
    let all = pn.all(&mem);
    assert!(all.len() > 50);
    assert!(all.iter().all(|o| {
        let h = o.holds.expect("open is known for every object");
        h.container.is_none() && h.supporter.is_none() && h.open.is_some()
    }));
}

#[test]
fn a_host_supplied_world_model_replaces_the_inferred_openness_bit_but_never_an_inform_one() {
    let mem = fixture("minizork.z3");
    let world = zvm::world::WorldModel::discover(&mem);
    let pn = ParseNames::detect(&mem).unwrap().with_world_model(&world);
    assert_eq!(pn.of(&mem, 167).unwrap().holds, Some(Holds::new(None, None, Some(false))));

    let mut none = world.clone();
    none.open_attr = None;
    let pn = ParseNames::detect(&mem).unwrap().with_world_model(&none);
    assert_eq!(pn.of(&mem, 167).unwrap().holds, None, "no openness bit: nothing is said");

    // An Inform story's attributes were NAMED, so a world model cannot overwrite them.
    let Some(mem) = fetched("anchor.z8") else { return };
    let before = ParseNames::detect(&mem).unwrap().of(&mem, 122).unwrap().holds;
    let after = ParseNames::detect(&mem).unwrap().with_world_model(&none).of(&mem, 122).unwrap().holds;
    assert!(before.is_some());
    assert_eq!(before, after);
}
