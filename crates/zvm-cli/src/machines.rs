//! `--machines`: print the §11.1.3 machine table zvm holds, and stop.
//!
//! [`zvm::interpreter::MACHINES`] is what both front-ends present a story as
//! (SQ-0872), and until now the only way to read it was to open the source. A
//! machine is a *bundle* — the byte in `$1E`, the page and ink in `$2C`/`$2D`,
//! the palette those colour numbers resolve through, and two §8.3 screen rules —
//! and a bundle whose members disagree is exactly the defect that quest was
//! filed for. Printing all of it side by side is what makes that checkable
//! without a debugger and without a game.
//!
//! # Generated, never transcribed
//!
//! Every row, every column and every decline below comes from the table itself,
//! for the reason [`crate::help`] is generated from `blorb::medium`'s: a second
//! copy kept by hand goes stale and then lies. Adding a machine to
//! `zvm::interpreter` reaches this output with no edit here, and REMOVING a
//! field would fail to compile rather than quietly stop being printed.
//!
//! # Why the colours are shown resolved
//!
//! `palette: Amiga` names a choice without stating it, and the choice is the
//! whole reason the field exists — ZMSD §8.3.1.1 lets an interpreter substitute
//! its own table, so the same colour NUMBER is a different colour on two
//! machines. **Colour 12 is the case that proves it here**, because it is the
//! Amiga's own page: §8.3.1 makes it `#5A5A5A` and Infocom's `colortable[]`
//! makes it `#424242`, so the Amiga's row would be showing a grey no Amiga ever
//! painted if this were resolved through the standard table. Six of the eleven
//! numbers differ between the two palettes (4, 5, 6, 10, 11, 12) and five agree
//! — 9 is `#FFFFFF` on both, which is why the ink column looks the same across
//! the table and the page column does not.
//!
//! # What is deliberately absent
//!
//! The Version 6 standard window. It is not a machine constant: an archive
//! states the picture space it was drawn for — the standard Macintosh's
//! monochrome plate asks for 480x300 where its colour plate asks for 320x200,
//! one machine and two spaces — so it belongs to the file rather than to the
//! table, and it lives in `app::interpreter` where a file can be read.
//! `zvm-cli` plays no v6 story anyway.

use zvm::interpreter::MACHINES;
use zvm::screen::{rgb15_to_888, standard_true_colour, Palette};

/// `#RRGGBB` for standard colour number `n` **through `palette`**, or a dash
/// when the number has no true-colour equivalent (§8.3.7's sentinels).
///
/// The palette is process-wide state (`zvm::screen::set_palette`) because a run
/// presents as one machine; a table presents as all of them, so this borrows it
/// per row and hands it straight back. Nothing else is running: `--machines`
/// prints and exits before a story is read.
fn swatch(n: u8, palette: Palette) -> String {
    let held = zvm::screen::palette();
    zvm::screen::set_palette(palette);
    let hex = standard_true_colour(n).map(rgb15_to_888).map(|(r, g, b)| format!("#{r:02X}{g:02X}{b:02X}"));
    zvm::screen::set_palette(held);
    hex.unwrap_or_else(|| "-".to_string())
}

/// One machine's page and ink, as `bg=N #RRGGBB fg=N #RRGGBB` — or why it has
/// none. A `None` here is a DECLINE with two different meanings, and the table
/// says which by naming the machine rather than by inventing a pair.
fn colours(m: &zvm::interpreter::MachineProfile) -> String {
    match m.default_colours {
        Some((bg, fg)) => {
            format!("bg {bg:>2} {}   fg {fg:>2} {}", swatch(bg, m.palette), swatch(fg, m.palette))
        }
        None => "the player's own terminal".to_string(),
    }
}

/// The whole table, ready to print.
pub fn table() -> String {
    let mut s = String::from(
        "ZMSD §11.1.3 machine profiles — what zvm models, in number order.\n\
         Colours are resolved through each machine's OWN palette (§8.3.1.1).\n\n",
    );
    let name_w = MACHINES.iter().map(|m| m.name.len()).max().unwrap_or(0);
    s.push_str(&format!(
        "  {:>2}  {:<name_w$}  {:<32}  {:<8}  {:<11}  {}\n",
        "#", "machine", "default page / ink ($2C/$2D)", "palette", "global pens", "v6 screen page"
    ));
    s.push_str(&format!("  {}\n", "-".repeat(name_w + 76)));
    for m in MACHINES {
        s.push_str(&format!(
            "  {:>2}  {:<name_w$}  {:<32}  {:<8}  {:<11}  {}\n",
            m.number,
            m.name,
            colours(m),
            match m.palette {
                Palette::Standard => "standard",
                Palette::Amiga => "Amiga",
            },
            yes_no(m.global_colour_pens),
            yes_no(m.v6_screen_page),
        ));
    }
    s.push_str(&format!("\n{LEGEND}"));
    // The gaps are the other half of the table: `machine()` answering None says
    // "a machine I do not model", and a reader who cannot see which numbers
    // those are cannot tell that from an oversight.
    let modelled: Vec<u8> = MACHINES.iter().map(|m| m.number).collect();
    let absent: Vec<String> = (1u8..=11)
        .filter(|n| !modelled.contains(n))
        .map(|n| format!("{n} ({})", NUMBERED.iter().find(|(k, _)| *k == n).map_or("?", |(_, v)| v)))
        .collect();
    s.push_str(&format!("\nNot modelled: {}.\n{ABSENT}", absent.join(", ")));
    s
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// §11.1.3's own names, for the numbers the table declines — the only place a
/// name is written by hand, because a row that does not exist cannot state one.
const NUMBERED: &[(u8, &str)] = &[
    (1, "DECSystem-20"),
    (2, "Apple IIe"),
    (3, "Macintosh"),
    (4, "Amiga"),
    (5, "Atari ST"),
    (6, "IBM PC"),
    (7, "Commodore 128"),
    (8, "Commodore 64"),
    (9, "Apple IIc"),
    (10, "Apple IIgs"),
    (11, "Tandy Color"),
];

const LEGEND: &str = "\
global pens     §8.3's Amiga rule: one pair of text pens for the whole screen,
                so a set_colour moves the screen rather than one window.
v6 screen page  the machine's default pair IS the Version 6 screen — the ground
                every window that names no colour of its own is read on.
";

// NOT a `"\` continuation: that escape eats the newline AND the leading
// whitespace after it, so the first row would lose the indent every other row
// keeps and the block would come out ragged.
const ABSENT: &str = "  1   what declining a number already falls through to; whether it deserves a
      bundle or is honestly \"a terminal, the same as the IBM PC\" is a decision.
  8   a .d64 is a 1541 image BOTH Commodore machines read, so the medium cannot
      choose between 7 and 8, and no Infocom Commodore interpreter has been read.
  11  no fixture and no sourced constant; anything here would be guesswork.
";

#[cfg(test)]
mod tests {
    use super::*;

    /// Every modelled machine appears, with its number and its name.
    #[test]
    fn the_table_lists_every_machine_zvm_models() {
        let t = table();
        for m in MACHINES {
            assert!(t.contains(m.name), "{} is missing from the table", m.name);
            assert!(
                t.lines().any(|l| l.trim_start().starts_with(&format!("{}  ", m.number))
                    && l.contains(m.name)),
                "{} ({}) has no row of its own:\n{t}",
                m.name,
                m.number
            );
        }
    }

    /// **Every field of the profile is printed.** The struct is the contract;
    /// a member added to it and not to the table is the failure this catches,
    /// and it is the whole reason the command exists rather than a hand-kept
    /// summary of a few interesting columns.
    ///
    /// Checked by naming a machine that answers distinctively on each member:
    /// the Amiga is the only row with a palette of its own and the only one with
    /// global pens, the Macintosh is the only one that is v6 screen page WITHOUT
    /// global pens, and the IBM PC is the only one that declines a colour pair.
    #[test]
    fn every_member_of_the_profile_reaches_the_output() {
        let t = table();
        let row = |name: &str| {
            t.lines().find(|l| l.contains(name)).unwrap_or_else(|| panic!("no {name} row")).to_string()
        };
        let amiga = row("Amiga");
        assert!(amiga.contains(" 4 "), "number");
        assert!(amiga.contains("bg 12") && amiga.contains("fg  9"), "default_colours: {amiga}");
        assert!(amiga.contains("Amiga  "), "palette named: {amiga}");
        assert_eq!(amiga.matches("yes").count(), 2, "global pens AND v6 page: {amiga}");

        let mac = row("Macintosh");
        assert!(mac.contains("bg  9") && mac.contains("fg  2"), "default_colours: {mac}");
        assert!(mac.contains("standard"), "palette: {mac}");
        assert_eq!(mac.matches("yes").count(), 1, "v6 page but NOT global pens: {mac}");

        let pc = row("IBM PC");
        assert!(pc.contains("the player's own terminal"), "a decline says so: {pc}");
        assert_eq!(pc.matches("yes").count(), 0, "no screen rules: {pc}");
    }

    /// The colours are resolved through the row's OWN palette, which is the
    /// claim the header makes and the only thing that makes the columns true.
    ///
    /// **Colour 12 is the case that can show it**, and picking the right number
    /// mattered: 9 is `#FFFFFF` in both palettes, so a row rendered through the
    /// wrong one would still print white and pass. Twelve is the Amiga's own
    /// page — `#5A5A5A` under §8.3.1 and `#424242` under Infocom's
    /// `colortable[]` — so it is on screen in the table AND divergent, which is
    /// what makes it evidence rather than coincidence.
    ///
    /// Falsifiable: resolve `swatch` through `Palette::Standard` for every row
    /// and the Amiga's page comes back `#5A5A5A`.
    #[test]
    fn each_row_resolves_its_colours_through_its_own_palette() {
        assert_eq!(swatch(12, Palette::Standard), "#5A5A5A", "§8.3.1's dark grey");
        assert_eq!(swatch(12, Palette::Amiga), "#424242", "the Amiga's own");
        assert_eq!(swatch(9, Palette::Standard), swatch(9, Palette::Amiga), "…and 9 agrees");
        let t = table();
        let amiga = t.lines().find(|l| l.contains("Amiga  ")).expect("the Amiga row");
        assert!(amiga.contains("bg 12 #424242"), "the Amiga's page is its own grey: {amiga}");
        assert!(!amiga.contains("#5A5A5A"), "…and never the standard table's: {amiga}");
    }

    /// Borrowing the process palette to resolve a row hands it straight back.
    #[test]
    fn printing_the_table_leaves_the_active_palette_where_it_found_it() {
        zvm::screen::set_palette(Palette::Amiga);
        let _ = table();
        assert_eq!(zvm::screen::palette(), Palette::Amiga);
        zvm::screen::set_palette(Palette::Standard);
        let _ = table();
        assert_eq!(zvm::screen::palette(), Palette::Standard);
    }

    /// The declines are printed too, and are exactly the numbers with no row.
    #[test]
    fn the_numbers_with_no_row_are_named_and_argued() {
        let t = table();
        for (n, name) in [(1u8, "DECSystem-20"), (8, "Commodore 64"), (11, "Tandy Color")] {
            assert!(t.contains(&format!("{n} ({name})")), "{name} must be listed as absent");
        }
        for m in MACHINES {
            assert!(
                !t.contains(&format!("Not modelled: {} ", m.number)),
                "{} is modelled and must not be listed absent",
                m.name
            );
        }
    }
}

