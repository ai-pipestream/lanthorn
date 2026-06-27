// zvm-cli — dumb-terminal Z-machine host driver (Task 16).
//
// Usage: zvm-cli <story-file>
//
// Reads a story file and plays it via stdin/stdout.  The host loop:
//   Continue     → keep stepping
//   Quit         → exit
//   Restart      → reload and restart from the original story bytes
//   NeedLine     → read a line from stdin, supply to machine
//   NeedChar     → read one byte from stdin, supply to machine
//   SaveRequest  → prompt for filename, write Quetzal bytes, complete_save
//   RestoreRequest → prompt for filename, read Quetzal bytes, restore_quetzal

use std::any::Any;
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::process;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

mod screen;
mod aux; // implemented in Task 5; declared now so the module tree is stable

// ── StdoutOutput ──────────────────────────────────────────────────────────────

/// Output sink that writes directly to stdout and flushes after each call.
/// On a TTY it wraps styled lower-window text in SGR; when piped it stays plain.
struct StdoutOutput {
    is_tty: bool,
}

impl StdoutOutput {
    fn new(is_tty: bool) -> Self {
        StdoutOutput { is_tty }
    }
}

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) {
        print!("{}", s);
        let _ = io::stdout().flush();
    }

    fn print_styled(&mut self, s: &str, style: u8) {
        let out = crate::screen::style_wrap(s, style, self.is_tty);
        print!("{}", out);
        let _ = io::stdout().flush();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── build_machine ─────────────────────────────────────────────────────────────

fn build_machine(story: Vec<u8>, stdout_is_tty: bool) -> Result<Machine, String> {
    use zvm::error::ZError;
    let mem = Memory::new(story).map_err(|e| match e {
        ZError::GraphicalV6 => "Error: Z-machine v6 graphical games are not supported.".to_string(),
        ZError::UnsupportedVersion(v) => format!("Error: Z-machine version {v} is not supported."),
        ZError::NotAStoryFile => "Error: file is not a valid Z-machine story file.".to_string(),
        ZError::Truncated => "Error: story file is truncated.".to_string(),
        _ => format!("Error loading story: {e:?}"),
    })?;
    let mut machine = Machine::with_output(mem, Box::new(StdoutOutput::new(stdout_is_tty)));
    machine.init_caps();
    Ok(machine)
}

// ── argument parsing ──────────────────────────────────────────────────────────

struct Args {
    story: Option<String>,
    no_status: bool,
    no_aux: bool,
}

fn parse_args(argv: &[String]) -> Args {
    let mut a = Args { story: None, no_status: false, no_aux: false };
    for arg in &argv[1..] {
        match arg.as_str() {
            "--no-status" | "--lower-only" => a.no_status = true,
            "--no-aux" => a.no_aux = true,
            s if !s.starts_with("--") && a.story.is_none() => a.story = Some(s.to_string()),
            _ => {}
        }
    }
    a
}

// ── terminal size + raw single-key input ──────────────────────────────────────

fn detect_term_rows() -> u16 {
    let stty = process::Command::new("stty")
        .arg("size")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    let env_lines = env::var("LINES").ok();
    screen::term_rows(stty.as_deref(), env_lines.as_deref())
}

/// Read one keypress in raw mode via stty (TTY only); fall back to a line byte.
fn read_char_input(stdin_is_tty: bool) -> u8 {
    use std::io::Read;
    if !screen::wants_raw_char(stdin_is_tty) {
        return read_byte_stdin();
    }
    let saved = process::Command::new("stty")
        .arg("-g")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    let _ = process::Command::new("stty")
        .args(["-icanon", "-echo", "min", "1", "time", "0"])
        .status();
    let mut buf = [0u8; 1];
    let n = io::stdin().read(&mut buf).unwrap_or(0);
    if let Some(s) = saved {
        let _ = process::Command::new("stty").arg(s.trim()).status();
    }
    if n == 0 {
        b'\n'
    } else {
        buf[0]
    }
}

// ── aux ("global state") persistence ──────────────────────────────────────────

/// Load the IFID-keyed aux file into the machine's aux_data (preload); warn on decode error.
fn aux_preload(machine: &mut Machine, aux_file: &Path, no_aux: bool) {
    if no_aux {
        return;
    }
    if let Ok(bytes) = fs::read(aux_file) {
        match aux::decode_aux(&bytes) {
            Ok(map) => {
                machine.aux_data = map;
                machine.aux_dirty = false;
            }
            Err(e) => eprintln!("zvm: warning: ignoring corrupt {}: {:?}", aux_file.display(), e),
        }
    }
}

/// Flush aux_data to the IFID-keyed aux file when dirty; clear the flag regardless.
fn aux_flush(machine: &mut Machine, aux_file: &Path, no_aux: bool) {
    if no_aux || !machine.aux_dirty {
        return;
    }
    if let Err(e) = fs::write(aux_file, aux::encode_aux(&machine.aux_data)) {
        eprintln!("zvm: warning: aux save to {} failed: {}", aux_file.display(), e);
    }
    machine.aux_dirty = false;
}

// ── prompt + read helpers ─────────────────────────────────────────────────────

fn prompt_and_read_line(prompt: &str) -> String {
    print!("{}", prompt);
    let _ = io::stdout().flush();
    let stdin = io::stdin();
    let mut line = String::new();
    let _ = stdin.lock().read_line(&mut line);
    line
}

fn read_line_stdin() -> String {
    let stdin = io::stdin();
    let mut line = String::new();
    let _ = stdin.lock().read_line(&mut line);
    line
}

fn read_byte_stdin() -> u8 {
    let stdin = io::stdin();
    let mut line = String::new();
    let _ = stdin.lock().read_line(&mut line);
    // Return the first byte of the input, or newline if empty.
    line.bytes().next().unwrap_or(b'\n')
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let argv: Vec<String> = env::args().collect();
    let args = parse_args(&argv);
    let Some(story_arg) = args.story.clone() else {
        eprintln!("Usage: {} [--no-status] [--no-aux] <story-file>", argv[0]);
        process::exit(1);
    };
    let story_path = std::path::PathBuf::from(&story_arg);

    let story_bytes = match fs::read(&story_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{}': {}", story_path.display(), e);
            process::exit(1);
        }
    };

    // Keep the original bytes for Restart.
    let original_bytes = story_bytes.clone();

    // Compute the IFID once; the aux file lives next to the story, keyed by IFID.
    let ifid = zvm::ifid::compute_ifid(&original_bytes);
    let aux_file = aux::aux_path(&story_path, &ifid);

    let stdout_is_tty = io::stdout().is_terminal();
    let stdin_is_tty = io::stdin().is_terminal();

    let mut machine = match build_machine(story_bytes, stdout_is_tty) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };
    aux_preload(&mut machine, &aux_file, args.no_aux);

    let mut view = screen::ScreenView::new(stdout_is_tty, args.no_status, detect_term_rows());

    loop {
        let step = machine.step();
        for d in machine.diagnostics.drain(..) {
            eprintln!("zvm: warning: {d}");
        }
        // Bleeps: drain and ring (TTY only).
        let beeps = machine.pending_beeps.len();
        machine.pending_beeps.clear();
        if beeps > 0 {
            print!("{}", screen::bleep_bytes(beeps, stdout_is_tty));
            let _ = io::stdout().flush();
        }
        // v3 show_status redraw request.
        if machine.screen.show_status_requested {
            print!("{}", view.frame(&machine));
            let _ = io::stdout().flush();
            machine.screen.show_status_requested = false;
        }
        // Persist aux tables as soon as the game commits one.
        aux_flush(&mut machine, &aux_file, args.no_aux);

        match step {
            StepResult::Continue => {}

            StepResult::Quit => {
                print!("{}", view.leave());
                let _ = io::stdout().flush();
                break;
            }

            StepResult::Restart => {
                machine = match build_machine(original_bytes.clone(), stdout_is_tty) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("{e}");
                        process::exit(1);
                    }
                };
                aux_preload(&mut machine, &aux_file, args.no_aux);
            }

            StepResult::NeedLine { .. } => {
                print!("{}", view.frame(&machine));
                let _ = io::stdout().flush();
                let line = read_line_stdin();
                machine.supply_line(line.trim_end());
            }

            StepResult::NeedChar => {
                print!("{}", view.frame(&machine));
                let _ = io::stdout().flush();
                let ch = read_char_input(stdin_is_tty);
                machine.supply_char(ch);
            }

            StepResult::SaveRequest => {
                let filename = prompt_and_read_line("\nSave to file: ");
                let filename = filename.trim();
                let save_data = machine.save_quetzal();
                match fs::write(filename, &save_data) {
                    Ok(()) => {
                        println!("Saved to '{filename}'.");
                        machine.complete_save(true);
                    }
                    Err(e) => {
                        eprintln!("Save failed: {e}");
                        machine.complete_save(false);
                    }
                }
            }

            StepResult::RestoreRequest => {
                let filename = prompt_and_read_line("\nRestore from file: ");
                let filename = filename.trim();
                match fs::read(filename) {
                    Ok(data) => match machine.restore_quetzal(&data) {
                        Ok(()) => {} // restored; execution continues from saved PC
                        Err(e) => {
                            eprintln!("Restore failed: {e:?}");
                            machine.complete_restore_failure();
                        }
                    },
                    Err(e) => {
                        eprintln!("Restore failed: {e}");
                        machine.complete_restore_failure();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod arg_tests {
    use super::*;

    #[test]
    fn parses_flags_and_story() {
        let a = parse_args(&["zvm-cli".into(), "--no-status".into(), "game.z5".into()]);
        assert_eq!(a.story.as_deref(), Some("game.z5"));
        assert!(a.no_status && !a.no_aux);

        let b = parse_args(&["zvm-cli".into(), "--no-aux".into(), "g".into()]);
        assert!(b.no_aux && !b.no_status);

        let c = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!c.no_status && !c.no_aux);
    }
}

#[cfg(test)]
mod stdout_tests {
    // The sink writes to the real stdout, so its behavior is exercised by the
    // manual smoke in Task 6; this pins the wrapping helper the sink must use.
    #[test]
    fn print_styled_wraps_only_on_tty() {
        assert_eq!(crate::screen::style_wrap("hi", 2, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(crate::screen::style_wrap("hi", 2, false), "hi");
    }
}
