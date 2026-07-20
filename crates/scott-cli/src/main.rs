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
use std::io::{self, BufRead, IsTerminal, Write};
use std::process;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use scott::{Database, Vm};

/// The canonical Scott Adams input prompt (mirrors `ScottSession::PROMPT` in the
/// app). ScottFree prints it from its input routine; the VM stays input-agnostic,
/// so the host layer owns it. Scott used this phrase, never the Infocom-style `>`.
const PROMPT: &str = "\nTell me what to do ? ";

/// The authentic Scott divider drawn between the room "window" and the command
/// area: `<` and `>` bracketing a run of em-dashes, sized to the room block.
fn separator(block: &str) -> String {
    let width = block.lines().map(|l| l.chars().count()).max().unwrap_or(0).max(20);
    format!("<{}>", "\u{2014}".repeat(width.saturating_sub(2)))
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
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            return None;
        }
        let line = line.trim_end_matches(['\n', '\r']).to_string();
        let _ = writeln!(out, "{line}"); // echo for a readable piped transcript
        return Some(line);
    }
    // Raw mode disables canonical line editing and echo; on failure, fall back to a
    // cooked read (arrow keys may still echo, but input still works).
    if terminal::enable_raw_mode().is_err() {
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            return None;
        }
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

fn parse_args() -> Result<Args, String> {
    let mut path = None;
    let mut seed = None;
    let mut max_turns = None;
    let mut argv = env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--seed" => {
                let v = argv.next().ok_or("--seed needs a value")?;
                seed = Some(v.parse().map_err(|_| format!("bad --seed value: {v}"))?);
            }
            "--max-turns" => {
                let v = argv.next().ok_or("--max-turns needs a value")?;
                max_turns = Some(v.parse().map_err(|_| format!("bad --max-turns value: {v}"))?);
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if path.replace(other.to_string()).is_some() {
                    return Err("expected exactly one story file".to_string());
                }
            }
        }
    }
    let path = path.ok_or("usage: scott-cli <adv.dat> [--seed <n>] [--max-turns <n>]")?;
    Ok(Args { path, seed, max_turns })
}

const HELP: &str = "\
scott-cli — DOS-style Scott Adams (ScottFree) player (no map)

Usage: scott-cli [OPTIONS] <adv.dat>

Arguments:
  <adv.dat>           Scott Adams ScottFree .dat adventure

Options:
      --seed <n>      Seed the RNG for reproducible play
      --max-turns <n> Stop after n turns (headless/testing)
  -V, --version       Print version and exit
  -h, --help          Print this help and exit
";

fn main() {
    if env::args().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return;
    }
    if env::args().any(|a| a == "--version" || a == "-V") {
        println!("{} {}", env!("CARGO_PKG_NAME"), buildinfo::LONG);
        return;
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("scott-cli: {e}");
            process::exit(2);
        }
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

    let interactive = io::stdin().is_terminal();
    let mut out = io::stdout();

    // Any pending output before the first prompt (empty today — the room is shown
    // via the room block below).
    let _ = write!(out, "{}", vm.take_output());

    let mut turns = 0u64;
    let mut last_block = String::new();
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
            let _ = writeln!(out, "{}", separator(&block));
            last_block = block;
        }
        let _ = write!(out, "{PROMPT}");
        let _ = out.flush();

        let line = match read_command(interactive, &mut out) {
            Some(l) => l,
            None => {
                let _ = writeln!(out); // tidy trailing newline on EOF / quit
                break;
            }
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
                let _ = writeln!(out, "{}", separator(&block));
            }
        }
    }
    let _ = out.flush();
}
