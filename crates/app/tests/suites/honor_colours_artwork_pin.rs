//! SQ-0860: the monochrome-artwork force-off has to survive the boot
//! `reload_style` — and stay one-run while it does.
//!
//! Two colourless-interpreter decisions meet at boot and, until this quest, the
//! second silently undid the first:
//!
//! 1. `startup.rs` loads the artwork, asks
//!    [`PictSource::declines_game_colours`] whether it has any colours to give,
//!    and on `false` sets `honor_game_colours = false` **and pins it one-run**
//!    (SQ-0806, refined by SQ-0846) so a fact about this launch's archive can
//!    never be written into the user's global `config.toml`.
//! 2. A few lines later — once the IFID is known and `game_dir` is set — the
//!    post-IFID `reload_style` recomputes that same key from the two per-story
//!    FILES (`garglk.ini`, `<game_dir>/config.toml`). Neither file knows which
//!    archive was loaded, so on a story that has neither it fell back to the
//!    global base, and the base was captured *before* step 1 ran.
//!
//! Measured on `stories/zork0-r393-s890714.z6` (Zork Zero v6 **release 393,
//! serial 890714**) playing `stories/zork0.cg1` — the IBM PC CGA rendition, two
//! colours, on the profile that has no colours of its own to declare — the flag
//! read `false` going into the reload and `true` coming out, with the pin
//! released in the same breath. (`.mg1` and `.eg1` are *not* two-colour; the
//! `.cg1` stencil is the archive SQ-0806 was written about.) From there
//! `loop_tick::poll_zvm_default_colours` writes header $2C/$2D that ZMSD §8.3.2
//! says to leave alone, and an `@restart` (`reset.rs`) rebuilds the session
//! honouring exactly the colours that paint a two-colour stencil out.
//!
//! The fix carries the fact on `AppState::artwork_declines_colours` and folds it
//! into `reload_style`'s per-story answer, beside SQ-0855's `game_colours_cli`
//! — a per-story value, so it stays PINNED rather than lowering the base.
//!
//! Both fixtures are gitignored commercial media; every test here skips
//! vacuously without them, and the sweep at the bottom is what stops the whole
//! file quietly passing on a machine that has neither.

use app::config::keys;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::state::AppState;
use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn fixture(name: &str) -> Option<PathBuf> {
    let path = stories_dir().join(name);
    if !path.exists() {
        eprintln!("SKIP: gitignored fixture missing at {}", path.display());
        return None;
    }
    Some(path)
}

/// The IBM PC CGA rendition of Zork Zero, on the bare story file (r393/s890714).
const PC_STORY: &str = "zork0-r393-s890714.z6";
const PC_MONO_ART: &str = "zork0.cg1";
/// The Macintosh medium (Zork Zero v6 r296/s881019) and its two-colour archive —
/// the same `EF_MONO` answer as the `.cg1`, on a machine SQ-0846 ruled must NOT
/// be declared colourless, because it states its own default page and ink. The
/// disk's *default* archive is `CPic.data`, the colour one, so the mono launch
/// has to be named.
const MAC_DISK: &str = "Zork Zero Disk.image";
const MAC_MONO_ART: &str = "Pic.data";

fn any_fixture_present() -> bool {
    [PC_STORY, PC_MONO_ART, MAC_DISK].iter().all(|n| stories_dir().join(n).exists())
}

/// What `startup.rs` decides about colour for a launch, without the TUI: the
/// archive's answer, and whether the force-off fired.
struct Boot {
    profile: InterpreterProfile,
    monochrome: bool,
    declines: bool,
}

fn boot_colour_decision(story: &str, pictures: Option<&str>) -> Option<Boot> {
    let path = fixture(story)?;
    if let Some(p) = pictures {
        fixture(p)?;
    }
    // Six of this helper's nine callers pass `PC_STORY`, so the story's length is
    // no more a discriminator between them than the pid is (SQ-1131).
    let dir = app::scratch_dir("sq860-boot");
    let over = PictureOverride::resolve_with_session(&path, &dir, pictures);
    // `startup.rs`'s order: the named archive's flavour, then the medium — and,
    // since SQ-0928, WHERE that answer came from, because only a medium licenses a
    // machine's own colours. `startup.rs` asks `Config::machine_default_colours`;
    // this models it, and modelling it is the point of the harness.
    let (profile, source) =
        InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
    let licensed = source.licenses_machine_colours(false);
    let machine_pair = licensed.then(|| profile.default_colours()).flatten();
    let picts = PictSource::resolve_with_override(&path, over, None);
    let decision = Boot {
        profile,
        monochrome: picts.is_monochrome(),
        declines: picts.declines_game_colours(machine_pair),
    };
    let _ = std::fs::remove_dir_all(&dir);
    Some(decision)
}

/// `AppState` as `startup.rs` leaves it at the post-IFID `reload_style`, for a
/// story with neither a `garglk.ini` nor a per-game sidecar — the case the boot
/// reload used to fall through on.
///
/// `global_honour` is the user's `config.toml`, written for real so a settings
/// save can be measured against it.
fn state_at_the_boot_reload(
    tag: &str,
    global_honour: bool,
    declines: bool,
) -> (AppState, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("lanthorn-sq860-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let style = dir.join("style.toml");
    std::fs::write(&style, "[colors]\n\"transcript\" = { fg = \"white\" }\n").unwrap();
    let game_dir = dir.join("game.save");
    std::fs::create_dir_all(&game_dir).unwrap();
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, format!("honor_game_colours = {global_honour}\n")).unwrap();

    let mut state = AppState::default();
    state.config.user_dir = dir.clone();
    state.config.config_file = cfg_path.clone();
    state.config.style = Some(style.to_string_lossy().to_string());
    state.game_dir = game_dir;
    // The base is captured before the artwork is even loaded.
    state.honor_game_colours_base = global_honour;
    state.config.honor_game_colours = global_honour;
    // …and then the archive has its say.
    if declines && global_honour {
        state.artwork_declines_colours = true;
        state.config.honor_game_colours = false;
        state.config.one_run.pin(keys::HONOR_GAME_COLOURS, false);
    }
    (state, dir, cfg_path)
}

/// The premise, on the real archive: `zork0.cg1` is two-colour, it resolves to
/// the IBM PC (the profile with no default colours of its own), and therefore
/// `startup.rs` declares the interpreter colourless.
#[test]
fn the_pc_monochrome_rendition_declares_the_interpreter_colourless() {
    let Some(b) = boot_colour_decision(PC_STORY, Some(PC_MONO_ART)) else { return };
    assert_eq!(b.profile, InterpreterProfile::IbmPc, "a PC-flavoured archive states the PC");
    assert!(b.monochrome, "{PC_MONO_ART} is the two-colour CGA stencil");
    assert!(b.declines, "two colours and no machine to speak for → colourless (SQ-0806)");
}

/// SQ-0846's half, unchanged: the Macintosh `Pic.data` is two-colour as well, and
/// must NOT be declared colourless — that machine's own interpreter states a
/// white page under black ink, and turning colours off there cost SQ-0846's
/// status banner its ink.
#[test]
fn the_macintosh_medium_keeps_its_colours() {
    let Some(b) = boot_colour_decision(MAC_DISK, Some(MAC_MONO_ART)) else { return };
    assert_eq!(b.profile, InterpreterProfile::Macintosh, "HFS states the machine");
    assert!(b.monochrome, "the Mac disk's Pic.data is two-colour too");
    assert!(!b.declines, "a machine that states its own colours outranks the guess");
}

/// The defect, end to end on the real archive: boot the PC monochrome rendition,
/// run the post-IFID reload, and the interpreter is still colourless.
///
/// Before the fix this asserted `true`: the reload found no `garglk.ini` and no
/// sidecar, fell back to the base captured before the force-off, and turned the
/// game's colours back on for every consumer after boot.
#[test]
fn the_boot_reload_does_not_undo_the_artwork_force_off() {
    let Some(b) = boot_colour_decision(PC_STORY, Some(PC_MONO_ART)) else { return };
    assert!(b.declines, "premise: this is the archive that forces the flag off");

    let (mut state, dir, _cfg) = state_at_the_boot_reload("pc-mono", true, b.declines);
    assert!(!state.config.honor_game_colours, "startup.rs forced it off");
    app::reload::reload_style(&mut state);
    assert!(
        !state.config.honor_game_colours,
        "the boot reload reads garglk.ini and the sidecar — neither of which knows \
         what archive was loaded — and must not overrule the archive"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// …and it is still ONE RUN afterwards, which is the other half: a settings save
/// in that session must leave the user's global `honor_game_colours` alone.
#[test]
fn the_artwork_force_off_stays_out_of_the_users_global_config() {
    let Some(b) = boot_colour_decision(PC_STORY, Some(PC_MONO_ART)) else { return };
    let (mut state, dir, cfg_path) = state_at_the_boot_reload("pc-mono-save", true, b.declines);
    app::reload::reload_style(&mut state);
    assert!(state.config.one_run.holds(keys::HONOR_GAME_COLOURS), "still one-run");

    app::config::write_config_file(&state.config).unwrap();
    let back = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        toml::from_str::<app::config::Config>(&back).unwrap().honor_game_colours,
        "the global file must still say true — one archive spoke, not the user: {back}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Macintosh launch, all the way through the same reload: nothing forces
/// anything off, nothing is pinned, and the colours the machine declares survive.
/// This is the SQ-0846 guard against an over-eager fix here.
#[test]
fn the_macintosh_launch_still_honours_its_colours_through_the_reload() {
    let Some(b) = boot_colour_decision(MAC_DISK, Some(MAC_MONO_ART)) else { return };
    let (mut state, dir, _cfg) = state_at_the_boot_reload("mac", true, b.declines);
    app::reload::reload_style(&mut state);
    assert!(state.config.honor_game_colours, "the Macintosh keeps its page and ink");
    assert!(
        !state.config.one_run.holds(keys::HONOR_GAME_COLOURS),
        "nothing one-run is in force here, so nothing is pinned"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other honour mode (the project's colour-render convention), on the real
/// archive: a user whose global file already says `false` never triggers the
/// force-off at all, and their own choice keeps persisting.
#[test]
fn a_global_off_is_the_users_and_persists_even_on_the_monochrome_archive() {
    let Some(b) = boot_colour_decision(PC_STORY, Some(PC_MONO_ART)) else { return };
    let (mut state, dir, cfg_path) = state_at_the_boot_reload("pc-mono-off", false, b.declines);
    app::reload::reload_style(&mut state);
    assert!(!state.config.honor_game_colours);
    assert!(
        !state.config.one_run.holds(keys::HONOR_GAME_COLOURS),
        "the user's own base is not a one-run value"
    );
    app::config::write_config_file(&state.config).unwrap();
    let back = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        !toml::from_str::<app::config::Config>(&back).unwrap().honor_game_colours,
        "a global choice persists as always: {back}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A deliberate choice outranks a guess about a machine — and still reaches the
/// file. `/set-game-colours` and the settings panel's row edit both clear
/// `artwork_declines_colours`; this walks the `/set-game-colours on` path on the
/// real monochrome launch and then the panel's global edit.
#[test]
fn a_deliberate_choice_beats_the_archive_and_still_persists() {
    let Some(b) = boot_colour_decision(PC_STORY, Some(PC_MONO_ART)) else { return };
    let (mut state, dir, cfg_path) = state_at_the_boot_reload("pc-mono-user", true, b.declines);
    app::reload::reload_style(&mut state);
    assert!(!state.config.honor_game_colours);

    // `/set-game-colours on`: sidecar written, the guess released, reload.
    app::styles::write_per_game_honor(&state.game_dir, Some(true)).unwrap();
    state.artwork_declines_colours = false;
    app::reload::reload_style(&mut state);
    assert!(state.config.honor_game_colours, "the player settled it by hand");

    // The settings panel's global edit: the row edit releases the pin, so the
    // value it leaves persists like any other setting — and a later reload keeps
    // it rather than recomputing the archive's guess back on.
    app::styles::write_per_game_honor(&state.game_dir, None).unwrap();
    app::reload::reload_style(&mut state);
    state.config.one_run.release(keys::HONOR_GAME_COLOURS);
    state.config.honor_game_colours = false;
    state.honor_game_colours_base = false;
    app::config::write_config_file(&state.config).unwrap();
    let back = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        !toml::from_str::<app::config::Config>(&back).unwrap().honor_game_colours,
        "a deliberate global choice must still reach the file: {back}"
    );
    app::reload::reload_style(&mut state);
    assert!(!state.config.honor_game_colours, "and is not undone by the next reload");
    let _ = std::fs::remove_dir_all(&dir);
}

/// CI has no `stories/` at all, so every test above returns early there and this
/// file would pass without measuring anything. Count one real decision and say so.
#[test]
fn the_real_media_smokes_were_not_vacuous() {
    let mut ran = 0;
    if let Some(b) = boot_colour_decision(PC_STORY, Some(PC_MONO_ART)) {
        assert!(b.declines);
        ran += 1;
    }
    if let Some(b) = boot_colour_decision(MAC_DISK, Some(MAC_MONO_ART)) {
        assert!(!b.declines);
        ran += 1;
    }
    assert!(
        ran > 0 || !any_fixture_present(),
        "the fixtures are present but nothing booted — this suite was vacuous"
    );
}
