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
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

// ── StdoutOutput ──────────────────────────────────────────────────────────────

/// Output sink that writes directly to stdout and flushes after each call.
struct StdoutOutput;

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) {
        print!("{}", s);
        let _ = io::stdout().flush();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── build_machine ─────────────────────────────────────────────────────────────

fn build_machine(story: Vec<u8>) -> Result<Machine, String> {
    use zvm::error::ZError;
    let mem = Memory::new(story).map_err(|e| match e {
        ZError::GraphicalV6 => "Error: Z-machine v6 graphical games are not supported.".to_string(),
        ZError::UnsupportedVersion(v) => format!("Error: Z-machine version {v} is not supported."),
        ZError::NotAStoryFile => "Error: file is not a valid Z-machine story file.".to_string(),
        ZError::Truncated => "Error: story file is truncated.".to_string(),
        _ => format!("Error loading story: {e:?}"),
    })?;
    let mut machine = Machine::with_output(mem, Box::new(StdoutOutput));
    machine.init_caps();
    Ok(machine)
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
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <story-file>", args[0]);
        process::exit(1);
    }

    let path = Path::new(&args[1]);
    let story_bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{}': {}", path.display(), e);
            process::exit(1);
        }
    };

    // Keep the original bytes for Restart.
    let original_bytes = story_bytes.clone();

    let mut machine = match build_machine(story_bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    loop {
        match machine.step() {
            StepResult::Continue => {}

            StepResult::Quit => {
                break;
            }

            StepResult::Restart => {
                machine = match build_machine(original_bytes.clone()) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("{e}");
                        process::exit(1);
                    }
                };
            }

            StepResult::NeedLine { .. } => {
                let line = read_line_stdin();
                machine.supply_line(line.trim_end());
            }

            StepResult::NeedChar => {
                let ch = read_byte_stdin();
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
