//! The period look: painting a v1–v4 story the way its machine's own
//! interpreter did (SQ-0873).
//!
//! [`zvm::interpreter::PeriodLook`] holds the measurements — a body pair, how the
//! status line was set apart, and the input cursor's shape and colour, for the
//! five machines `machine-screenshots/` has captures of. This module is the half
//! that decides whether they apply to *this* launch and turns them into terminal
//! cells.
//!
//! # The gate, and why every clause of it is there
//!
//! **Colour arrives with Version 5.** `set_colour` and the `$2C`/`$2D` header
//! bytes are v5+, so a v1–v4 story has no colour concept at all: it never sets,
//! never reads, never branches. Anything shown for one is presentation, which is
//! exactly what makes a period look legitimate — and exactly what would make it
//! a lie for a v5+ story, where the pair on screen is a fact the story can read
//! and [`crate::interpreter::InterpreterProfile::default_colours`] already
//! supplies it from Infocom's own code. The two must not be confused; hence
//! [`resolve`]'s version clause and not a knob.
//!
//! **`honor_game_colours` is the master switch and this one is narrower.** A
//! player who turns game colours off has said "keep my terminal's colours", and
//! a blue Amiga page painted over that would be fighting them — so an off
//! `honor_game_colours` (from `--no-game-colours`, a `garglk.ini` stylehint, the
//! per-game sidecar, or SQ-0860's monochrome-artwork force-off) takes the period
//! look with it. The reverse does **not** hold, which is the whole reason
//! [`crate::config::Config::period_look`] exists separately: declining the look
//! must not also cost a v5+ story the colours it asked for.
//!
//! SQ-0860's force-off cannot actually reach here, and it is worth saying why
//! rather than leaving it to be rediscovered: it fires only for a **monochrome
//! artwork archive**, and an archive means Version 6. A story cannot be both
//! v6 and v1–v4, so the two features are disjoint by construction and the
//! ordering above is belt-and-braces.
//!
//! `NO_COLOR` has no reader in the TUI at all — it is a convention about adding
//! ANSI colour to command output, and lanthorn is a full-screen program whose
//! whole surface is decoration. `zvm-cli` is where it applies, and there it
//! already folds into the same `honor` flag `--no-game-colours` sets, so the
//! period look inherits the composition rather than restating it.
//!
//! # What the terminal can and cannot say
//!
//! The measurements are in pixels: a 1x16 Macintosh caret between glyphs, an 8x1
//! Commodore underscore on the cell's bottom scanline, an Amiga status line
//! reverse-videoed behind each run of text with the page showing between. A cell
//! grid can express the last one exactly and the first two only as the glyph that
//! occupies the same part of the cell — `▏` and `▁`. That is the honest analogue,
//! and it is named here rather than in a comment at the draw site so the loss is
//! recorded once.

use ratatui::style::{Color, Modifier, Style};
use zvm::interpreter::{CursorShape, PeriodLook, StatusBand};

use crate::interpreter::InterpreterProfile;
use crate::theme::resolve::Theme;

/// Does this launch get a period look, and which?
///
/// `zversion` is the story's header byte 0; pass `None` for an engine that has no
/// such byte (Glulx, Scott Adams), which declines for the same reason a v5 story
/// does — §11.1.3 interpreter numbers are the Z-machine's vocabulary and nothing
/// else's.
///
/// See the module docs for why each clause is there.
pub fn resolve(
    profile: InterpreterProfile,
    enabled: bool,
    honor_game_colours: bool,
    zversion: Option<u8>,
) -> Option<PeriodLook> {
    if !enabled || !honor_game_colours {
        return None;
    }
    if !matches!(zversion, Some(1..=4)) {
        return None;
    }
    profile.period_look()
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

/// The machine's body pair as a style: its ink over its page.
pub fn body_style(look: &PeriodLook) -> Style {
    Style::new().fg(rgb(look.ink)).bg(rgb(look.page))
}

/// The status band's own base style — what the whole row is filled with before a
/// single segment is drawn.
///
/// [`StatusBand::PerRun`] fills with the **body** pair, because on the Amiga the
/// page is what shows between the two runs; the reversal is per segment and is
/// [`status_run_style`]'s. [`StatusBand::Ruled`] likewise fills with the body
/// pair — the Macintosh does not distinguish the band by ground at all — and
/// carries the rule as an underline, which is the one row-wide horizontal line a
/// cell grid has.
pub fn status_style(look: &PeriodLook) -> Style {
    let body = body_style(look);
    match look.status {
        StatusBand::FullReverse => reversed(body),
        StatusBand::PerRun => body,
        StatusBand::Own { ground, ink } => Style::new().fg(rgb(ink)).bg(rgb(ground)),
        StatusBand::Ruled => body.add_modifier(Modifier::UNDERLINED),
    }
}

/// The style for one run of status text when the band is [`StatusBand::PerRun`],
/// or `None` when the band's base already carries the whole answer.
///
/// The Amiga is the only machine measured that reverses behind the text and
/// leaves the page showing between; every other band is uniform, so its segments
/// inherit the base and this answers `None`.
pub fn status_run_style(look: &PeriodLook) -> Option<Style> {
    matches!(look.status, StatusBand::PerRun).then(|| reversed(body_style(look)))
}

fn reversed(s: Style) -> Style {
    Style::new().fg(s.bg.unwrap_or(Color::Reset)).bg(s.fg.unwrap_or(Color::Reset))
}

/// The caret at the END of the input line, where the cell is empty and the shape
/// is the whole of what is drawn: the glyph and the style to draw it in.
///
/// - [`CursorShape::Bar`] → `▏`, the left eighth of the cell. The Macintosh caret
///   is one pixel wide and sits in the gap *after* the last glyph, which is where
///   this cell is.
/// - [`CursorShape::Block`] → a space in the cursor's colour, filling the cell as
///   the Apple II's and the Amiga's do.
/// - [`CursorShape::Underscore`] → `▁`, the bottom eighth, which is where both
///   Commodores put their single scanline.
pub fn caret_cell(look: &PeriodLook) -> (&'static str, Style) {
    let colour = rgb(look.cursor_colour);
    match look.cursor_shape {
        CursorShape::Bar => ("▏", Style::new().fg(colour).bg(rgb(look.page))),
        CursorShape::Block => (" ", Style::new().fg(rgb(look.page)).bg(colour)),
        CursorShape::Underscore => ("▁", Style::new().fg(colour).bg(rgb(look.page))),
    }
}

/// The caret sitting ON a character — mid-line, or over the completion hint's
/// first glyph.
///
/// The glyph is kept and only the style applies, because the text has to stay
/// readable while it is edited. That means the SHAPE cannot be drawn: a `▏` or a
/// `▁` in this cell would replace the character rather than mark it. What the
/// machine's colours can still say is which cell the caret is in, so an
/// underscore machine underlines it in the cursor's colour and the other two
/// swap the pair. The shape is expressible only where the cell is empty; see
/// [`caret_cell`].
pub fn caret_over_text(look: &PeriodLook) -> Style {
    let colour = rgb(look.cursor_colour);
    match look.cursor_shape {
        CursorShape::Underscore => {
            Style::new().fg(colour).bg(rgb(look.page)).add_modifier(Modifier::UNDERLINED)
        }
        CursorShape::Bar | CursorShape::Block => Style::new().fg(rgb(look.page)).bg(colour),
    }
}

/// The selectors the period look paints, each with the registry parent that has
/// to be unclaimed too, and what it paints them with.
///
/// **Only the story pane's own surfaces.** The map, the dialogs and the rest of
/// the chrome are lanthorn's, not the machine's, and a Commodore 64's grey page
/// across the whole application would be dressing up rather than presenting.
///
/// Two kinds of paint, and the difference matters. The prose and the line being
/// typed take the machine's PAIR, because on that machine they were its ink on
/// its page and nothing else. lanthorn's own annotations — the echoed command,
/// the meta gutter, a warning — take only the PAGE: their ink says something
/// lanthorn means (this line is yours, this one is not the story's) and no
/// machine has an opinion about it, but leaving their ground alone would punch
/// the theme's page through the machine's in the middle of the transcript.
fn painted(look: &PeriodLook) -> [(&'static str, &'static str, Style); 7] {
    let body = body_style(look);
    let page = Style::new().bg(rgb(look.page));
    [
        ("transcript", "text", body),
        ("input_line", "line", body),
        ("input_text", "text", body),
        ("input_prompt", "text", body),
        ("transcript_input", "accent", page),
        ("transcript_meta", "muted", page),
        ("transcript_warning", "alert", page),
    ]
}

/// Lay the machine's colours under the resolved theme.
///
/// **A user's choice outranks a machine default**, per selector, which is the
/// same rule SQ-0847 applied when the Macintosh's white page first reached the
/// input line: a selector any layer wrote (global `style.toml`, a discovered
/// `garglk.ini`, the per-game sidecar) keeps what that layer said, and only one
/// still at [`Provenance::Default`](crate::theme::resolve::Provenance) — along
/// with the role it inherits from — is the machine's to fill. So a player who
/// themed their transcript gets their theme on an Amiga floppy, and one who never
/// touched it gets the Amiga.
///
/// Called from `reload::reload_style`, which is the single place the theme is
/// built — startup, `/reload-style`, the style watcher and the per-game overlay
/// all funnel through it, so patching there reaches every path.
pub fn apply_to_theme(theme: &mut Theme, look: &PeriodLook) {
    for (sel, role, style) in painted(look) {
        theme.fill_unclaimed(sel, role, style);
    }
    // Stated absolutely rather than patched; see `Theme::set_unclaimed`.
    theme.set_unclaimed("status_bar", "chrome", status_style(look));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look_of(n: u8) -> PeriodLook {
        zvm::interpreter::machine(n).expect("modelled").period_look.expect("measured")
    }

    /// The gate's whole point: a v5 story's pair is a fact it can read, so the
    /// period look must not touch it however the machine was reached.
    #[test]
    fn colour_arrives_with_version_five_and_the_look_stops_there() {
        let amiga = InterpreterProfile::Amiga;
        for v in 1..=4 {
            assert!(resolve(amiga, true, true, Some(v)).is_some(), "v{v} is pre-colour");
        }
        for v in 5..=8 {
            assert!(resolve(amiga, true, true, Some(v)).is_none(), "v{v} states its own pair");
        }
        // Glulx and Scott Adams have no §11.1.3 number to be a machine of.
        assert!(resolve(amiga, true, true, None).is_none());
    }

    /// One-way composition (SQ-0855/SQ-0860): the master switch takes the look
    /// with it, the narrow one does not reach game colours.
    #[test]
    fn honor_game_colours_is_the_master_and_the_key_is_narrower() {
        let amiga = InterpreterProfile::Amiga;
        assert!(resolve(amiga, true, false, Some(3)).is_none(), "colours off takes the look");
        assert!(resolve(amiga, false, true, Some(3)).is_none(), "and the key declines alone");
        assert!(resolve(amiga, true, true, Some(3)).is_some());
    }

    /// A machine with no capture has no look, and asking for one does not
    /// conjure it — the same sourced-or-declined standard the table itself keeps.
    #[test]
    fn an_unmeasured_machine_declines() {
        for p in [InterpreterProfile::AtariSt, InterpreterProfile::IbmPc] {
            assert!(resolve(p, true, true, Some(3)).is_none(), "{p:?} has no capture");
        }
    }

    /// The Amiga is the reason [`StatusBand::PerRun`] exists: the band's ground
    /// is the PAGE and only the text runs are reversed. Every other machine's
    /// band is uniform and its segments inherit the base.
    #[test]
    fn only_the_amiga_reverses_per_run() {
        let amiga = look_of(zvm::interpreter::AMIGA_INTERPRETER_NUMBER);
        assert_eq!(status_style(&amiga), body_style(&amiga), "the page shows between the runs");
        let run = status_run_style(&amiga).expect("the Amiga reverses behind its text");
        assert_eq!(run.fg, Some(rgb(amiga.page)));
        assert_eq!(run.bg, Some(rgb(amiga.ink)));

        for n in [
            zvm::interpreter::APPLE_IIE_INTERPRETER_NUMBER,
            zvm::interpreter::COMMODORE_128_INTERPRETER_NUMBER,
            zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER,
            zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER,
        ] {
            assert!(status_run_style(&look_of(n)).is_none(), "interpreter {n} has a uniform band");
        }
    }

    /// The Macintosh does not distinguish its band by ground at all — it rules
    /// it. A cell grid's one horizontal rule is the underline.
    #[test]
    fn the_macintosh_band_is_the_body_pair_with_a_rule_under_it() {
        let mac = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        let band = status_style(&mac);
        assert_eq!((band.fg, band.bg), (body_style(&mac).fg, body_style(&mac).bg));
        assert!(band.add_modifier.contains(Modifier::UNDERLINED), "rules, not a ground");
    }

    /// Three shapes, three glyphs, and the cursor's colour is neither the page
    /// nor the ink on two of the five machines — so the caret cannot be built out
    /// of the body pair.
    #[test]
    fn the_caret_draws_its_machines_shape_and_its_own_colour() {
        let mac = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        let amiga = look_of(zvm::interpreter::AMIGA_INTERPRETER_NUMBER);
        let c128 = look_of(zvm::interpreter::COMMODORE_128_INTERPRETER_NUMBER);
        assert_eq!(caret_cell(&mac).0, "▏");
        assert_eq!(caret_cell(&amiga).0, " ");
        assert_eq!(caret_cell(&c128).0, "▁");

        // The Amiga's orange is in neither channel of its body pair, which is the
        // case that would break if the caret were built by reversing the body.
        let cell = caret_cell(&amiga).1;
        assert_eq!(cell.bg, Some(rgb(amiga.cursor_colour)));
        assert_ne!(cell.bg, Some(rgb(amiga.page)));
        assert_ne!(cell.bg, Some(rgb(amiga.ink)));
    }

    /// SQ-0847's rule, reused: a machine default fills what nobody claimed and
    /// never overwrites a choice. Both directions, and the ROLE counts as a claim
    /// — [`Provenance`] does not travel down the parent chain, so a player who
    /// recoloured `text` and left `transcript` alone has still chosen the
    /// transcript's ink.
    #[test]
    fn a_users_choice_outranks_the_machine_and_an_untouched_selector_does_not() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        use crate::theme::registry::Delta;

        let look = look_of(zvm::interpreter::AMIGA_INTERPRETER_NUMBER);
        let one = |sel: &str, d: Delta| {
            let mut m = Decls::new();
            m.insert(sel.to_string(), d);
            m
        };

        // Nobody claimed anything: the Amiga's page and ink land.
        let mut bare = resolve(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
        apply_to_theme(&mut bare, &look);
        assert_eq!(bare.get("transcript").style.bg, Some(rgb(look.page)));
        assert_eq!(bare.get("transcript").style.fg, Some(rgb(look.ink)));

        // The selector itself claimed: the player's ink survives the floppy.
        let mine = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let mut themed = resolve(&Roles::terminal_default(), &mine, &Decls::new(), &Decls::new());
        apply_to_theme(&mut themed, &look);
        assert_eq!(themed.get("transcript").style.fg, Some(Color::Green));
        assert_ne!(themed.get("transcript").style.bg, Some(rgb(look.page)));

        // Only the ROLE claimed, and that is a claim too.
        let role = one("text", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let mut inherited = resolve(&Roles::terminal_default(), &role, &Decls::new(), &Decls::new());
        apply_to_theme(&mut inherited, &look);
        assert_eq!(inherited.get("transcript").style.fg, Some(Color::Green));
    }

    /// The status bar ships REVERSED as its registry default, which is lanthorn's
    /// way of setting the bar apart — and a swapped pair drawn under it swaps
    /// back. The band is stated absolutely for exactly that reason; this is the
    /// case that fails if it is ever patched instead.
    #[test]
    fn a_full_reverse_band_is_stated_and_not_left_to_reverse_itself() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        let look = look_of(zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER);
        let mut theme = resolve(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
        assert!(
            theme.get("status_bar").style.add_modifier.contains(Modifier::REVERSED),
            "the registry default this guards against"
        );
        apply_to_theme(&mut theme, &look);
        let band = theme.get("status_bar").style;
        assert_eq!(band.fg, Some(rgb(look.page)), "the C64 reverses its body pair");
        assert_eq!(band.bg, Some(rgb(look.ink)));
        assert!(!band.add_modifier.contains(Modifier::REVERSED), "or it would reverse back");
    }

    /// Over a character the glyph must survive, so the shape stands down and only
    /// the colour speaks — except for the underscore, which a terminal can draw
    /// under a character without hiding it.
    #[test]
    fn a_caret_over_text_keeps_the_text_readable() {
        let c64 = look_of(zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER);
        let over = caret_over_text(&c64);
        assert!(over.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(over.fg, Some(rgb(c64.cursor_colour)));

        let mac = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        let over = caret_over_text(&mac);
        assert!(!over.add_modifier.contains(Modifier::UNDERLINED), "a bar cannot go under a glyph");
        assert_eq!(over.bg, Some(rgb(mac.cursor_colour)));
    }
}
