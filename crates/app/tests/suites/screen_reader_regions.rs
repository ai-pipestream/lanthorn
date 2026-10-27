//! SQ-1765: `screen_reader::regions` and the Z-code `Engine::score`, driven on
//! the real Infocom Version 6 releases whose windows were established.
//!
//! Every frame is a fixture. Each case boots the story the way `startup.rs`
//! does (`InterpreterProfile::resolve` -> `MachineBoot::resolve` ->
//! `GameSession::new_for_machine`), pins the release it loaded FIRST, drives
//! the keys in the table, and guards the frame's SHAPE before asserting roles.
//!
//! | case | file | release / serial | turns (in order) |
//! |---|---|---|---|
//! | Zork Zero gameplay | `zork0-r393-s890714.z6` | 393 / 890714 | 3: `cr` `cr` `look` |
//! | Zork Zero hint menu | same | same | 4: `cr` `cr` `hint` `y`; then `n` (5) |
//! | Zork Zero score | same | same | the walkthrough's first 22 commands, ending `take scepter` |
//! | Shogun title card | `shogun-r322-s890706.z6` | 322 / 890706 | 1: `cr` |
//! | Shogun gameplay + score | same | same | 2: `cr` `cr`; then `steer port` x2, `steer starboard` (5) |
//! | Arthur gameplay | `arthur-r74-s890714.z6` | 74 / 890714 | 12 taps as `v6_arthur_status` (Enter; `n` to the restore question) |
//! | band-text fallback | `James Clavell's Shogun.adf` | 295 / 890321 | 5, as Shogun above |
//!
//! `cr` is Enter at a keypress and an empty line at a line prompt; any other
//! command is a typed line, or its first character at a keypress. Skip-if-missing
//! (gitignored stories): the `ran` guard in each case is non-vacuity against the
//! fixture being present but the frame having drifted, not against absence.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::screen_reader::{regions, Region, RegionRole};
use app::session::{GameSession, InputKind};
use cli_host::MenuBlock;

fn boot(file: &str, release: u16, serial: &str) -> Option<GameSession> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(file);
    let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let bytes = loaded.bytes().to_vec();
    let profile = InterpreterProfile::resolve(&path, None, PictureOverride::Unset.flavour(), None);
    let mut picts = PictSource::resolve_with_override(&path, PictureOverride::Unset, None);
    let dims = picts.all_pict_dims();
    let honoured = !picts.declines_game_colours(profile.default_colours());
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
        .expect("story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let id = s.game_identifier();
    assert_eq!((id.release, id.serial_str().into_owned()), (release, serial.to_string()), "{file}: the pinned release");
    Some(s)
}

fn turn(s: &mut GameSession, cmd: &str) {
    match s.pending_input() {
        InputKind::Line => {
            let _ = s.submit(if cmd == "cr" { "" } else { cmd });
        }
        InputKind::Char => {
            let _ = s.submit_char(if cmd == "cr" { 13 } else { cmd.as_bytes()[0] });
        }
        InputKind::Event => {
            let _ = s.submit("");
        }
    }
}

fn read(s: &GameSession) -> Vec<Region> {
    regions(&s.screen(), Some(&s.game_identifier()))
}

fn find(rs: &[Region], window: u8) -> Option<&Region> {
    rs.iter().find(|r| r.window == Some(window))
}

fn joined(r: &Region) -> String {
    r.text.join("\n")
}

const ZORK0: (&str, u16, &str) = ("zork0-r393-s890714.z6", 393, "890714");
const SHOGUN: (&str, u16, &str) = ("shogun-r322-s890706.z6", 322, "890706");
const ARTHUR: (&str, u16, &str) = ("arthur-r74-s890714.z6", 74, "890714");

#[test]
fn zork0_r393_gameplay_band_is_chrome_and_the_page_is_content() {
    let Some(mut s) = boot(ZORK0.0, ZORK0.1, ZORK0.2) else { return };
    for c in ["cr", "cr", "look"] {
        turn(&mut s, c);
    }
    let rs = read(&s);
    let band = find(&rs, 1).expect("non-vacuity: window 1 is the band");
    assert_eq!(band.role, RegionRole::Chrome);
    let b = joined(band);
    assert!(b.contains("Banquet Hall") && b.contains("Moves:") && b.contains("Score:"), "band: {b:?}");
    let page = find(&rs, 0).expect("window 0 is the story page");
    assert_eq!(page.role, RegionRole::Content);
    assert!(joined(page).contains("The hall is filled to capacity"), "page: {:?}", page.text);
    assert_eq!(rs.len(), 2, "frame and picture windows contribute nothing: {rs:?}");
    assert!(rs[0].rect.1 <= rs[1].rect.1, "reading order is top to bottom: {rs:?}");
}

#[test]
fn zork0_r393_hint_menu_is_a_menu_with_twenty_topics_and_a_moving_cursor() {
    let Some(mut s) = boot(ZORK0.0, ZORK0.1, ZORK0.2) else { return };
    for c in ["cr", "cr", "hint", "y"] {
        turn(&mut s, c);
    }
    let rs = read(&s);
    let menu = find(&rs, 0).expect("window 0 carries the topic grid");
    // Non-vacuity: the menu frame is window 0 in positioned-paint mode, which is a
    // text grid and not the page: its text names the topics.
    assert!(joined(menu).contains("PROLOGUE") && joined(menu).contains("GENERAL QUESTIONS"), "{:?}", menu.text);
    assert_eq!(menu.role, RegionRole::Menu);
    let block = MenuBlock::parse(&joined(menu)).expect("a Menu region's text parses");
    // Twenty rows of topics; the two entries drawn beside the first rows
    // (`AS A LAST RESORT`, `FOR YOUR AMUSEMENT`) are a second column of the same
    // rows and are not counted as lines of their own.
    assert_eq!((block.count(), block.current()), (20, 1));
    assert_eq!(block.item(1), Some("PROLOGUE                     AS A LAST RESORT"));
    let legend = find(&rs, 1).expect("the legend is window 1");
    assert_eq!(legend.role, RegionRole::Chrome);
    assert!(joined(legend).contains("InvisiClues") && joined(legend).contains("N for next item."));

    turn(&mut s, "n");
    let rs = read(&s);
    let block = MenuBlock::parse(&joined(find(&rs, 0).unwrap())).expect("still a menu after N");
    assert_eq!(block.current(), 2, "the reverse-video row moved down one");
    assert_eq!(block.item(2), Some("GREAT HALL AREA              FOR YOUR AMUSEMENT"));
}

#[test]
fn zork0_r393_score_follows_the_band_through_a_scoring_action() {
    let Some(mut s) = boot(ZORK0.0, ZORK0.1, ZORK0.2) else { return };
    assert_eq!(s.score(), Some(0));
    let walk = [
        "northeast", "south", "west", "wait", "wait", "dive under table", "wait", "wait", "wait", "stand",
        "take calendar", "up", "south", "south", "take bag", "press button", "north", "north", "down", "south",
        "south",
    ];
    for c in walk {
        turn(&mut s, c);
    }
    assert_eq!(s.score(), Some(0), "non-vacuity: nothing scored before the scepter");
    assert!(joined(find(&read(&s), 1).unwrap()).contains("Score:   0"));
    turn(&mut s, "take scepter");
    assert_eq!(s.score(), Some(12), "taking the scepter scores 12");
    let band = joined(find(&read(&s), 1).unwrap());
    assert!(band.contains("Score:  12") && band.contains("Throne Room"), "the band shows what score() says: {band:?}");
}

#[test]
fn shogun_r322_title_card_is_content_and_the_start_panel_is_a_menu() {
    let Some(mut s) = boot(SHOGUN.0, SHOGUN.1, SHOGUN.2) else { return };
    turn(&mut s, "cr");
    let rs = read(&s);
    let card = find(&rs, 0).expect("non-vacuity: the title card is window 0's grid");
    assert!(joined(card).contains("A Story of Japan"), "{:?}", card.text);
    assert_eq!(card.role, RegionRole::Content);
    let panel = find(&rs, 2).expect("window 2 holds the START/RESTORE choice");
    assert!(joined(panel).contains("START the game"), "{:?}", panel.text);
    assert_eq!(panel.role, RegionRole::Menu, "{:?}", panel.text);
    let choice = MenuBlock::parse(&joined(panel)).expect("a Menu region's text parses");
    assert_eq!(choice.count(), 3, "{:?}", panel.text);
    assert!(rs.iter().all(|r| r.role != RegionRole::Chrome), "no band yet: {rs:?}");
}

#[test]
fn shogun_r322_band_is_chrome_and_score_is_global_0x66_times_five() {
    let Some(mut s) = boot(SHOGUN.0, SHOGUN.1, SHOGUN.2) else { return };
    for c in ["cr", "cr"] {
        turn(&mut s, c);
    }
    let rs = read(&s);
    let band = find(&rs, 1).expect("non-vacuity: window 1 is the band");
    assert_eq!(band.role, RegionRole::Chrome);
    assert!(joined(band).contains("Bridge") && joined(band).contains("Score:"), "{:?}", band.text);
    let page = find(&rs, 0).expect("window 0 is the page");
    assert_eq!(page.role, RegionRole::Content);
    assert!(joined(page).contains("This is the bridge of the"), "{:?}", page.text);
    assert_eq!(s.score(), Some(0));
    for c in ["steer port", "steer port"] {
        turn(&mut s, c);
    }
    assert_eq!(s.score(), Some(0), "non-vacuity: centring the wheel is what scores");
    turn(&mut s, "steer starboard");
    assert_eq!(s.score(), Some(5));
    assert!(joined(find(&read(&s), 1).unwrap()).contains("Score:    5"), "the band shows 5");
}

#[test]
fn arthur_r74_band_below_the_panel_is_chrome_and_there_is_no_score() {
    let Some(mut s) = boot(ARTHUR.0, ARTHUR.1, ARTHUR.2) else { return };
    for _ in 0..12 {
        let before = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if before.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    let screen = s.screen();
    let rs = regions(&screen, Some(&s.game_identifier()));
    let band = find(&rs, 1).expect("non-vacuity: window 1 is the status ribbon");
    assert_eq!(band.role, RegionRole::Chrome);
    assert!(joined(band).contains("Churchyard"), "{:?}", band.text);
    assert_eq!(band.rect.1, 12, "the ribbon sits below the picture panel, not at the top");
    assert!(rs.iter().filter(|r| r.role == RegionRole::Chrome).count() == 1, "{rs:?}");
    assert!(rs.iter().all(|r| matches!(r.window, Some(0) | Some(1) | Some(3))), "poles, panel and frame are silent: {rs:?}");
    // The generic rule alone would call the ribbon content (it is not at the top):
    // this is what the override buys.
    let generic = regions(&screen, None);
    assert_eq!(find(&generic, 1).unwrap().role, RegionRole::Content);
    assert_eq!(s.score(), None, "Arthur's band prints no score");
}

#[test]
fn an_unknown_release_reads_its_score_off_the_chrome_band() {
    // The Amiga floppy is a different release: its score is not in global 0x66
    // (it reads 19 there) and it is NOT in the override table, so the band text
    // is what answers.
    let Some(mut s) = boot("James Clavell's Shogun.adf", 295, "890321") else { return };
    for c in ["cr", "cr"] {
        turn(&mut s, c);
    }
    let rs = read(&s);
    assert_eq!(find(&rs, 1).expect("non-vacuity: the band").role, RegionRole::Chrome, "generic rule: top, two rows");
    assert_eq!(s.score(), Some(0));
    for c in ["steer port", "steer port", "steer starboard"] {
        turn(&mut s, c);
    }
    assert_eq!(s.score(), Some(5));
}
