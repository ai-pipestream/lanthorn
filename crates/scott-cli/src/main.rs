// scott-cli — dumb-terminal Scott Adams (ScottFree `.dat`) host driver.
//
// The zero-Glk counterpart to `zvm-cli`/`gvm-cli`: it loads a real Adventure
// International `.dat` and plays it via stdin/stdout, so a published game can be
// smoke-tested headlessly by piping a command script in and diffing the
// transcript. Scott is line-only (no char input, windows, colour, or sound and
// no in-game save protocol), so the whole host loop is: describe → prompt →
// read a line → step → print. That is all this binary needs to be.
//
// Usage: scott-cli <adv.dat> [--seed <n>] [--max-turns <n>]
//   --seed <n>       seed the VM's occurrence-roll PRNG for reproducible runs
//   --max-turns <n>  stop after N commands (a safety cap for scripted input)

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

use cli_host::{HostMode, TerminalGuard};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use scott::{Database, Vm};

/// The canonical Scott Adams input prompt (mirrors `ScottSession::PROMPT` in the
/// app). ScottFree prints it from its input routine; the VM stays input-agnostic,
/// so the host layer owns it. Scott used this phrase, never the Infocom-style `>`.
const PROMPT: &str = "\nTell me what to do ? ";

/// The authentic Scott divider drawn between the room "window" and the command
/// area: `<` and `>` bracketing a run of em-dashes, sized to the room block.
///
/// `None` in plain mode. The rule is pure decoration — it stands in for the
/// boundary a real Scott terminal drew between its two windows — and a screen
/// reader has no way to take it as one: it is thirty-odd em-dashes, announced
/// one at a time or swallowed entirely depending on the reader's punctuation
/// settings, and either way it says nothing. The blank line before the room
/// block already separates the two for a listener (SQ-0608).
fn separator(block: &str, plain: bool) -> Option<String> {
    if plain {
        return None;
    }
    let width = block.lines().map(|l| l.chars().count()).max().unwrap_or(0).max(20);
    Some(format!("<{}>", "\u{2014}".repeat(width.saturating_sub(2))))
}

/// Read one command line. Returns `None` at end of input (EOF, or Ctrl-C/Ctrl-D
/// when interactive).
///
/// Piped input (non-TTY): a plain line read, echoed so a captured transcript
/// reads naturally. Interactive (TTY): a minimal raw-mode line editor that echoes
/// typed characters, handles Backspace, and — crucially — swallows arrow keys and
/// other escape sequences instead of letting the terminal spew `^[[A` garbage into
/// the line. It deliberately does not implement history or cursor movement; this
/// is a play/smoke harness, not a full readline.
fn read_command(interactive: bool, out: &mut impl Write) -> Option<String> {
    if !interactive {
        let line = cli_host::read_line_stdin()?;
        let line = line.trim_end_matches(['\n', '\r']).to_string();
        let _ = writeln!(out, "{line}"); // echo for a readable piped transcript
        return Some(line);
    }
    // Raw mode disables canonical line editing and echo; on failure, fall back to a
    // cooked read (arrow keys may still echo, but input still works).
    if terminal::enable_raw_mode().is_err() {
        let line = cli_host::read_line_stdin()?;
        return Some(line.trim_end_matches(['\n', '\r']).to_string());
    }
    let mut buf = String::new();
    let result = loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                // Ctrl-C / Ctrl-D quit (raw mode routes them as keys, not signals).
                KeyCode::Char('c' | 'd') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    break None;
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    let _ = write!(out, "{c}");
                    let _ = out.flush();
                }
                KeyCode::Backspace => {
                    if buf.pop().is_some() {
                        let _ = write!(out, "\u{8} \u{8}"); // erase the last glyph
                        let _ = out.flush();
                    }
                }
                KeyCode::Enter => {
                    let _ = write!(out, "\r\n");
                    let _ = out.flush();
                    break Some(buf.clone());
                }
                // Arrows, Home/End, function keys, etc.: ignored (no garbage).
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    let _ = terminal::disable_raw_mode();
    result
}

struct Args {
    path: String,
    seed: Option<u32>,
    max_turns: Option<u64>,
}

/// Every option `scott-cli` accepts; `cli_host::args` applies the rules.
const OPTS: &[cli_host::Opt] = &[
    cli_host::Opt::flag(&["--screen-reader", "--plain"]),
    cli_host::Opt::flag(&["--help", "-h"]),
    cli_host::Opt::flag(&["--version", "-V"]),
    cli_host::Opt::valued(&["--seed"]),
    cli_host::Opt::valued(&["--max-turns"]),
];

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let m = cli_host::scan(argv, OPTS)?;
    if m.positional.len() > 1 {
        return Err(format!("unexpected extra argument: {}", m.positional[1]));
    }
    // Strict, unlike zvm-cli's lenient interpreter number: a seed or turn cap
    // that does not parse is a typo in a reproducibility knob, and silently
    // ignoring it would make a run that looks reproducible but is not.
    let num = |flag: &str| -> Result<Option<u64>, String> {
        match m.value(flag) {
            None => Ok(None),
            Some(v) => v.parse().map(Some).map_err(|_| format!("bad {flag} value: {v}")),
        }
    };
    Ok(Args {
        path: m.first_positional().ok_or("no story file given")?.to_string(),
        seed: num("--seed")?.map(|v| v as u32),
        max_turns: num("--max-turns")?,
    })
}

const HELP: &str = "\
scott-cli — DOS-style Scott Adams (ScottFree) player (no map)

Usage: scott-cli [OPTIONS] <adv.dat>

Arguments:
  <adv.dat>           Scott Adams ScottFree .dat adventure

Host commands (typed at any prompt, never passed to the game):
  /status         Repeat the room block (location, exits, what is here)

Options:
      --screen-reader Linear plain text (alias: --plain; also selected by
                      TERM=dumb). Hands line editing and echo back to the
                      terminal and drops the em-dash divider rule, which a
                      reader can only spell out. Scott has no status window to
                      suppress — the room block IS the story — so there is no
                      --story-only here. Ask for the room again any time with
                      /status.
      --seed <n>      Seed the RNG for reproducible play
      --max-turns <n> Stop after n turns (headless/testing)
  -V, --version       Print version and exit
  -h, --help          Print this help and exit
";

fn main() {
    let argv: Vec<String> = env::args().collect();
    if cli_host::handled_common_flags(&argv, HELP, env!("CARGO_PKG_NAME"), buildinfo::LONG) {
        return;
    }
    let args = match parse_args(&argv) {
        // Already the only CLI that rejected unknown flags; now it shows the
        // help alongside the message, like the other two (SQ-0614).
        Err(e) => cli_host::usage_error(env!("CARGO_PKG_NAME"), &e, HELP),
        Ok(a) => a,
    };

    let bytes = fs::read(&args.path).unwrap_or_else(|e| {
        eprintln!("scott-cli: cannot read {}: {e}", args.path);
        process::exit(1);
    });
    let src = std::str::from_utf8(&bytes).unwrap_or_else(|_| {
        eprintln!("scott-cli: {} is not a text .dat", args.path);
        process::exit(1);
    });
    if !scott::looks_like_scott(src) {
        eprintln!("scott-cli: {} does not look like a Scott .dat", args.path);
        process::exit(1);
    }
    let db = Database::parse(src).unwrap_or_else(|e| {
        eprintln!("scott-cli: invalid Scott .dat: {e:?}");
        process::exit(1);
    });

    let mut vm = Vm::new(db);
    if let Some(seed) = args.seed {
        vm.seed_rng(seed);
    }

    // scott-cli emits no escape sequences at all, so `rich` never comes up here
    // — the only terminal state it touches is raw mode, and until SQ-0605 it had
    // no teardown of any kind: a panic mid-game left the shell in raw mode.
    // `raw_only` fixes that without putting escapes into a transcript that has
    // never had any.
    //
    // That also makes plain mode nearly free here: the one thing `--plain` has
    // to change is handing line editing back to the terminal, which is exactly
    // what `raw_input` decides (SQ-0606).
    let mode = HostMode::detect_with(cli_host::plain_requested(&argv)).install();
    let interactive = mode.raw_input();
    let _guard = TerminalGuard::raw_only();
    let mut out = io::stdout();

    // Any pending output before the first prompt (empty today — the room is shown
    // via the room block below).
    let _ = write!(out, "{}", vm.take_output());

    let mut turns = 0u64;
    let mut last_block = String::new();
    // Scott stores no score: it is the count of treasures deposited in the
    // treasure room, recomputed each turn (SQ-0616). Screen-reader mode only —
    // otherwise the SCORE verb is the way to ask.
    let mut score_watch = cli_host::ScoreWatch::new();
    let announce_scores = mode.plain();
    loop {
        if vm.has_quit() {
            break;
        }
        if let Some(max) = args.max_turns {
            if turns >= max {
                eprintln!("\nscott-cli: reached --max-turns {max}, stopping");
                break;
            }
        }
        // The room block is the top "window" in the real game; on a dumb terminal
        // we print it inline whenever the room (or its contents) changes.
        let block = vm.room_block();
        if block != last_block {
            let _ = writeln!(out, "\n{block}");
            if let Some(rule) = separator(&block, mode.plain()) {
                let _ = writeln!(out, "{rule}");
            }
            last_block = block;
        }
        if announce_scores {
            let (stored, _total) = vm.treasures_stored();
            if let Some(line) = score_watch.update(Some(stored)) {
                let _ = writeln!(out, "{line}");
            }
        }
        let _ = write!(out, "{PROMPT}");
        let _ = out.flush();

        // `/status` is answered by the host and the game never sees it, so loop
        // until a real command arrives. Scott's "status" is the room block —
        // where you are, the exits, what is here — which the loop above prints
        // only when it *changes*, so after a few turns of conversation it has
        // scrolled away (SQ-0610).
        let read = loop {
            match read_command(interactive, &mut out) {
                Some(l) if cli_host::input::is_status_request(&l) => {
                    let _ = writeln!(out, "{}", vm.room_block());
                    let _ = write!(out, "{PROMPT}");
                    let _ = out.flush();
                }
                // Both a real command and end-of-input leave the loop; only the
                // status request goes round again. Keeping EOF as `None` here
                // matters — turning it into an empty command is the bug that hung
                // the other two CLIs (SQ-0604/0605).
                other => break other,
            }
        };
        let Some(line) = read else {
            let _ = writeln!(out); // tidy trailing newline on EOF / quit
            break;
        };

        vm.supply_line(&line);
        let _ = vm.step();
        turns += 1;
        // Prints this turn's output; a quitting turn (win/death) is drained here
        // and the loop's top-of-iteration `has_quit` check ends the session.
        let _ = write!(out, "{}", vm.take_output());

        // On game end, print the final room block: the panel (upper "window")
        // reflects the closing state, but the loop's top-of-iteration block print
        // won't run because `has_quit` breaks first. Mirrors the app, which keeps
        // the final panel on screen at game over.
        if vm.has_quit() {
            let block = vm.room_block();
            if block != last_block {
                let _ = writeln!(out, "\n{block}");
                if let Some(rule) = separator(&block, mode.plain()) {
                let _ = writeln!(out, "{rule}");
            }
            }
        }
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_rule_is_sized_to_the_room_block() {
        // `<` + em-dashes + `>`, as wide as the widest line (floor of 20).
        let rule = separator("I'm in a forest", false).expect("a rule outside plain mode");
        assert_eq!(rule.chars().count(), 20, "short block gets the minimum width");
        assert!(rule.starts_with('<') && rule.ends_with('>'), "bracketed: {rule:?}");

        let wide = separator(&"x".repeat(40), false).unwrap();
        assert_eq!(wide.chars().count(), 40, "grows to the block");
    }

    #[test]
    fn separator_is_absent_in_plain_mode() {
        // Thirty-odd em-dashes are pure decoration: a screen reader either
        // announces each one or drops the line entirely, and neither conveys the
        // window boundary it stands for (SQ-0608).
        assert_eq!(separator("I'm in a forest", true), None);
        assert_eq!(separator(&"x".repeat(60), true), None);
    }
}
