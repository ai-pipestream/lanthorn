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

use scott::{Database, Vm};

/// The canonical Scott Adams input prompt (mirrors `ScottSession::PROMPT` in the
/// app). ScottFree prints it from its input routine; the VM stays input-agnostic,
/// so the host layer owns it. Scott used this phrase, never the Infocom-style `>`.
const PROMPT: &str = "\nTell me what to do ? ";

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

fn main() {
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

    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
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
            last_block = block;
        }
        let _ = write!(out, "{PROMPT}");
        let _ = out.flush();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            let _ = writeln!(out); // tidy trailing newline on EOF
            break;
        }
        let line = line.trim_end_matches(['\n', '\r']);
        // A TTY echoes the typed line itself; when input is piped, echo it so the
        // captured transcript reads naturally next to the prompt.
        if !interactive {
            let _ = writeln!(out, "{line}");
        }

        vm.supply_line(line);
        let _ = vm.step();
        turns += 1;
        // Prints this turn's output; a quitting turn (win/death) is drained here
        // and the loop's top-of-iteration `has_quit` check ends the session.
        let _ = write!(out, "{}", vm.take_output());
    }
    let _ = out.flush();
}
