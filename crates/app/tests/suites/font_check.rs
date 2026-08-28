//! SQ-1104 — the first-run font check, end to end through the real files.
//!
//! The unit tests in `render::font_check_dialog` pin the QUESTION (two rows,
//! same slots, one glyph per source family); these pin the ANSWER, which has to
//! survive a round trip lanthorn does not control in one place: `toml_edit`
//! writes into the seeded `style.toml`, `style::parse_style_toml` reads it back,
//! and `SymbolSet::resolve` turns it into the glyphs the map draws. Any of the
//! three could be individually right and the chain still wrong.

use app::render::font_check_dialog::{ASSIST_LAMP, NERD_ARROWS, NERD_PORTALS};
use app::style::{load_style, style_write_path, write_font_check_answer};
use app::symbols::SymbolSet;

/// A throwaway lanthorn home, seeded exactly as a first launch seeds it.
fn seeded_home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lanthorn-fontcheck-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a throwaway lanthorn home");
    app::theme::template::auto_seed(&dir);
    assert!(dir.join("style.toml").is_file(), "the seed writes a style.toml");
    dir
}

/// Read the home's style file back the way startup does, and resolve the glyphs.
fn glyphs(dir: &std::path::Path) -> SymbolSet {
    let (doc, warnings) = load_style(None, dir);
    assert!(warnings.is_empty(), "the file we just wrote must parse clean: {warnings:?}");
    SymbolSet::resolve(&app::style::finalize_symbols(&doc.symbols))
}

/// The affirmative answer reaches the map: nerdfont arrows, the four distinct
/// stairs/door portal icons, and the lamp as the Guiding Light's mark.
#[test]
fn a_yes_installs_every_family_the_prompt_sampled() {
    let dir = seeded_home("yes");
    let path = style_write_path(None, &dir).expect("no pointer means the personal file");
    write_font_check_answer(&path, true).expect("writing the answer");

    let set = glyphs(&dir);
    let want_arrows = app::symbols::Arrows::preset(NERD_ARROWS).unwrap();
    let want_portal = app::symbols::PortalGlyphs::preset(NERD_PORTALS).unwrap();
    assert_eq!(set.arrows, want_arrows, "the arrow preset arrived");
    assert_eq!(set.portal.up, want_portal.up, "the stairs arrived");
    assert_eq!(set.portal.marker, want_portal.marker);
    assert_eq!(set.assist_gutter, ASSIST_LAMP, "the Guiding Light's mark is the lamp");
    assert_eq!(
        set.controls,
        app::symbols::ControlGlyphs::preset(app::render::font_check_dialog::NERD_CONTROLS).unwrap(),
        "the border toggle controls came with the rest (SQ-1123)",
    );

    // PRESET NAMES, not forty expanded per-slot overrides: the file has to stay
    // something a person can read and re-decide, and a later improvement to the
    // preset has to keep reaching them. Asked of the LIVE lines only — the seeded
    // template documents every slot name in comments, `arrow.north` included.
    let text = std::fs::read_to_string(&path).unwrap();
    let live: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    assert!(
        live.iter().any(|l| l.contains(&format!("arrow_set = \"{NERD_ARROWS}\""))),
        "{text}"
    );
    assert!(
        live.iter().any(|l| l.contains(&format!("portal_icons = \"{NERD_PORTALS}\""))),
        "{text}"
    );
    assert!(
        live.iter().any(|l| l.contains("control_icons = \"nerdfont\"")),
        "{text}"
    );
    assert!(
        !live.iter().any(|l| l.contains("arrow.")),
        "the answer must not expand into per-slot overrides:\n{live:#?}"
    );
    assert_eq!(
        live.iter().filter(|l| l.contains("gutter.assist")).count(),
        1,
        "the one glyph with no preset of its own is the one override written"
    );
}

/// The negative answer is written too, not merely left unwritten — so a
/// re-check after a font change lands on the same two keys instead of leaving a
/// stale pair behind. And it takes the lamp back out with it.
#[test]
fn a_later_no_undoes_an_earlier_yes() {
    let dir = seeded_home("no-after-yes");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true).unwrap();
    write_font_check_answer(&path, false).unwrap();

    let set = glyphs(&dir);
    let plain = SymbolSet::default();
    assert_eq!(set.arrows, plain.arrows, "back to the filled triangles");
    assert_eq!(set.portal.up, plain.portal.up, "back to the plain portal icons");
    assert_eq!(
        set.assist_gutter, plain.assist_gutter,
        "and back to ● — NOT `*`, which Infocom games spend on footnotes"
    );
    assert_eq!(set.controls, plain.controls, "and back to the plain border controls");
}

/// A mark the user chose themselves is not ours to remove. The answer is about a
/// FONT; only the glyph the font check itself wrote is cleared by a later "no".
#[test]
fn a_no_leaves_a_gutter_mark_the_user_chose() {
    let dir = seeded_home("keeps-user-mark");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true).unwrap();

    // The user then picks their own mark, by hand, the way the file invites.
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replace(&ASSIST_LAMP.to_string(), "☼");
    std::fs::write(&path, text).unwrap();

    write_font_check_answer(&path, false).unwrap();
    assert_eq!(glyphs(&dir).assist_gutter, '☼', "their mark survives an answer about fonts");
}

/// Repeatable: answering twice the same way must not stack up duplicate keys or
/// a second `[map]` table. (`/run-font-check` exists to be run whenever a font
/// changes, so "again" is the normal case, not the edge one.)
#[test]
fn answering_twice_rewrites_rather_than_appends() {
    let dir = seeded_home("idempotent");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true).unwrap();
    let once = std::fs::read_to_string(&path).unwrap();
    write_font_check_answer(&path, true).unwrap();
    let twice = std::fs::read_to_string(&path).unwrap();
    assert_eq!(once, twice, "the second identical answer is a no-op on the text");
    let live = twice
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter(|l| l.contains("arrow_set = "))
        .count();
    assert_eq!(live, 1, "one live key, not two:\n{twice}");
}

/// The seed's comments are documentation the user needs; a settings write must
/// not eat them, exactly as `write_config_at` must not eat `config.toml`'s.
#[test]
fn the_seeded_commentary_survives_the_write() {
    let dir = seeded_home("comments");
    let path = style_write_path(None, &dir).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    write_font_check_answer(&path, true).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();

    let comments = |t: &str| t.lines().filter(|l| l.trim_start().starts_with('#')).count();
    assert_eq!(comments(&after), comments(&before), "every comment line survives");
    assert!(after.contains("[map.overrides]"), "and the override table's header with them");
}

/// A file that does not parse is the text the user has to READ to fix it.
/// Refuse, the way `config::write_config_at` refuses, rather than rewriting it.
#[test]
fn a_broken_style_file_is_refused_not_overwritten() {
    let dir = seeded_home("broken");
    let path = style_write_path(None, &dir).unwrap();
    std::fs::write(&path, "[map\narrow_set = oops\n").unwrap();
    let err = write_font_check_answer(&path, true).expect_err("a broken file must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[map\narrow_set = oops\n",
        "and left exactly as it was"
    );
}

/// `style = "default"` names the built-in style, which lives in the binary.
/// There is no file, so there is nothing to write and the caller must be told.
#[test]
fn the_builtin_style_has_no_file_to_write() {
    let dir = seeded_home("builtin");
    assert!(style_write_path(Some("default"), &dir).is_none());
    // Any other pointer resolves to a real path, relative to the home.
    assert_eq!(
        style_write_path(Some("mine.toml"), &dir),
        Some(dir.join("mine.toml")),
    );
}

/// The written file must ANSWER the prompt's own question: whatever the two
/// sample rows showed is what the two answers install. Pinned against the rows
/// rather than against literals, so the dialog and the writer cannot drift.
#[test]
fn the_rows_the_prompt_shows_are_the_glyphs_the_answers_install() {
    use app::render::font_check_dialog::sample_row;

    let dir = seeded_home("rows-match");
    let path = style_write_path(None, &dir).unwrap();

    write_font_check_answer(&path, true).unwrap();
    let set = glyphs(&dir);
    for ch in sample_row(true).chars().filter(|c| !c.is_whitespace()) {
        let on_the_map = [
            set.arrows.north, set.arrows.south, set.arrows.east, set.arrows.west,
            set.portal.marker, set.portal.up, set.portal.down,
            set.portal.in_, set.portal.out, set.portal.unknown,
            set.assist_gutter,
        ];
        assert!(on_the_map.contains(&ch), "row 1 showed {ch:?}, which the yes answer does not install");
    }

    write_font_check_answer(&path, false).unwrap();
    let set = glyphs(&dir);
    for ch in sample_row(false).chars().filter(|c| !c.is_whitespace()) {
        let on_the_map = [
            set.arrows.north, set.arrows.south, set.arrows.east, set.arrows.west,
            set.portal.marker, set.portal.up, set.portal.down,
            set.portal.in_, set.portal.out, set.portal.unknown,
            set.assist_gutter,
        ];
        assert!(on_the_map.contains(&ch), "row 2 showed {ch:?}, which the no answer does not install");
    }
}

/// `--font-check` is a bare noun with a value, like `--sound` / `--images` /
/// `--accel` / `--guidance` — and it is declared on `Cli` in `app::config`,
/// which is what the TUI parses with. (`cli-host/src/args.rs` is the scanner for
/// the three headless CLI players and knows nothing about this flag.)
#[test]
fn the_flag_has_three_states_and_none_is_the_default() {
    use app::config::{Cli, OnOff};
    use clap::Parser;

    let parse = |args: &[&str]| Cli::try_parse_from(args).expect("parses");
    assert_eq!(parse(&["lanthorn", "story.z5"]).font_check, None, "absent = ask only on a first run");
    assert_eq!(parse(&["lanthorn", "--font-check", "on", "story.z5"]).font_check, Some(OnOff::On));
    assert_eq!(parse(&["lanthorn", "--font-check", "off", "story.z5"]).font_check, Some(OnOff::Off));
    // No `set-` prefix — that belongs to the slash command, whose registry
    // requires a verb.
    assert!(Cli::try_parse_from(["lanthorn", "--set-font-check", "on", "s.z5"]).is_err());
}

/// The slash spelling is verb-noun and takes no argument: the dialog IS the
/// question, so there is no second grammar to keep in step with the buttons.
#[test]
fn the_slash_command_is_verb_noun_and_argument_free() {
    use app::slash::{find_command, parse, Category, SlashOutcome};

    assert!(matches!(parse("run-font-check", '/'), SlashOutcome::RunFontCheck));
    assert!(matches!(parse("run-font-check anything", '/'), SlashOutcome::RunFontCheck));
    let spec = find_command("run-font-check").expect("registered");
    assert_eq!(spec.category, Category::Style);
    assert!(find_command("font-check").is_none(), "a bare noun is not a command name");
}

/// **The harness guard, as a guard rather than a convention** (SQ-1104).
///
/// "There is no config.toml" is the NORMAL state of a throwaway user-dir, and a
/// first run raises a modal that waits for a keypress — under a REAL pty, so no
/// tty check saves it. Falsified by removing the line: `pty_emitted_stream` still
/// reported PASS, and the prompt had quietly eaten one of its four Enters and
/// written `arrow_set = "filled"` into the harness's style.toml. A pty harness
/// can lose a keystroke and stay green, so nothing downstream will report this;
/// the line itself is what has to be defended.
#[test]
fn the_pty_driver_still_writes_a_config_for_every_harness() {
    let src = include_str!("../pty_stream/driver.rs");
    let run = src
        .split_once("pub fn run(spec: Spec)")
        .expect("the driver still has a `run`")
        .1;
    let head = &run[..run.len().min(2000)];
    assert!(
        head.contains("user_dir.join(\"config.toml\")"),
        "pty_stream::driver::run must seed a config.toml into every Spec's user_dir, \
         or a first-run font check blocks every pty harness at once"
    );
}
