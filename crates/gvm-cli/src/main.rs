// gvm-cli — an interactive Glulx player.
//
// Usage: gvm-cli <story.ulx | story.gblorb>
//
// Loads a raw `.ulx` Glulx image or extracts the `GLUL` executable from a Blorb,
// drives it through a Glk step loop, routing Glk output to a terminal backend
// and reading the player's input (cooked line / raw single key on a TTY), and
// prints any diagnostics to stderr.

use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal};
use std::process;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;

use gvm::glk::keycode;
use gvm::{GlkBackend, Machine, Memory, StepResult};

mod glk_term;
use glk_term::TerminalBackend;

// ── key decoding ──────────────────────────────────────────────────────────────

/// Map a crossterm `KeyCode` to a Glk char-input keycode. Printable Latin-1
/// characters pass through as their code point; special keys map to the
/// `keycode::*` constants from the Glk spec.
fn decode_glk_keycode(code: KeyCode) -> u32 {
    match code {
        KeyCode::Char(c) if (c as u32) < 0x110000 => c as u32,
        KeyCode::Enter => keycode::RETURN,
        KeyCode::Backspace | KeyCode::Delete => keycode::DELETE,
        KeyCode::Tab => keycode::TAB,
        KeyCode::Esc => keycode::ESCAPE,
        KeyCode::Up => keycode::UP,
        KeyCode::Down => keycode::DOWN,
        KeyCode::Left => keycode::LEFT,
        KeyCode::Right => keycode::RIGHT,
        KeyCode::Home => keycode::HOME,
        KeyCode::End => keycode::END,
        KeyCode::PageUp => keycode::PAGE_UP,
        KeyCode::PageDown => keycode::PAGE_DOWN,
        KeyCode::F(n) if (1..=12).contains(&n) => keycode::FUNC1 - (n as u32 - 1),
        _ => keycode::UNKNOWN,
    }
}

// ── Blorb extraction ──────────────────────────────────────────────────────────

/// Get the runnable Glulx image: the `GLUL` executable inside a Blorb, or the
/// raw bytes when they aren't a Blorb (a plain `.ulx`).
fn extract_executable(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes).map_err(|e| format!("Error: invalid Blorb: {e:?}"))?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::ZCode, _)) => {
            Err("Error: this is a Z-code Blorb; run it with zvm-cli.".to_string())
        }
        Err(e) => Err(format!("Error: Blorb has no executable: {e:?}")),
    }
}

// ── machine builder ───────────────────────────────────────────────────────────

/// Build a machine from story bytes, sending Glk output to `backend`.
fn build_machine(bytes: Vec<u8>, backend: Box<dyn GlkBackend>) -> Result<Machine, String> {
    let image = extract_executable(bytes)?;
    let mem = Memory::new(image).map_err(|e| format!("Error loading Glulx image: {e:?}"))?;
    Ok(Machine::with_glk(mem, backend))
}

// ── input helpers ─────────────────────────────────────────────────────────────

/// Read a cooked line of input from stdin (the terminal echoes it). The second
/// value is the Glk line terminator keycode; cooked stdin only ever completes a
/// line with Enter, so it is always 0 (normal termination).
fn read_line_stdin() -> (String, u32) {
    let mut line = String::new();
    let _ = io::stdin().lock().read_line(&mut line);
    (line, 0)
}

/// How [`read_line_raw`] should echo the typed line.
enum LineEcho {
    /// Glk line echo is ON: echo the input (opened with this SGR, empty = plain)
    /// and leave it on screen, ending the line with CR/LF. The library owns the
    /// echo; the game does not repeat it.
    Shown(String),
    /// Glk line echo is OFF (`glk_set_echo_line_event(win, 0)`): the game echoes
    /// the command itself. Show the input live (plain) so the player isn't typing
    /// blind, then ERASE exactly what we echoed on Enter — so the game's own echo
    /// is the only copy that survives on a scrolling terminal (SQ-0282). Mirrors a
    /// redrawable Glk library, which removes echo-off input once the line ends.
    EraseOnEnter,
}

/// Read a line of input in RAW mode, echoing typed characters manually so the
/// terminal's cooked echo can't double a self-echoing game (SQ-0275). `echo`
/// selects library-echo (kept on screen) vs echo-off (shown live, then erased on
/// Enter — SQ-0282). Falls back to cooked line input on non-TTY stdin (piped
/// input has no terminal echo, so it is already correct). The terminator is
/// always 0 (normal Enter), matching the prior cooked path.
fn read_line_raw(is_tty: bool, echo: LineEcho) -> (String, u32) {
    if !is_tty {
        return read_line_stdin(); // (String, 0)
    }
    let (sgr, erase_after): (&str, bool) = match &echo {
        LineEcho::Shown(s) => (s.as_str(), false),
        LineEcho::EraseOnEnter => ("", true),
    };
    let _ = terminal::enable_raw_mode();
    let mut buf = String::new();
    // Count of visible characters we echoed, so EraseOnEnter can wipe exactly them.
    let mut echoed: usize = 0;
    if !sgr.is_empty() {
        print!("{sgr}");
        let _ = io::Write::flush(&mut io::stdout());
    }
    loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { code, modifiers, .. })) => match code {
                KeyCode::Enter => break,
                // Raw mode swallows signals; exit cleanly on Ctrl-C / Ctrl-D.
                KeyCode::Char('c') | KeyCode::Char('d')
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if !sgr.is_empty() { print!("\x1b[0m"); }
                    print!("\r\n");
                    let _ = io::Write::flush(&mut io::stdout());
                    let _ = terminal::disable_raw_mode();
                    // Unconditional: this path can't see `honor`/`last_page_bg`;
                    // resetting when nothing was set is harmless.
                    print!("{}", glk_term::osc_reset_bg());
                    print!("{}", glk_term::cursor_reset());
                    let _ = io::Write::flush(&mut io::stdout());
                    std::process::exit(0);
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    print!("{c}");
                    echoed += 1;
                    let _ = io::Write::flush(&mut io::stdout());
                }
                KeyCode::Backspace => {
                    if buf.pop().is_some() {
                        print!("\x08 \x08");
                        echoed = echoed.saturating_sub(1);
                        let _ = io::Write::flush(&mut io::stdout());
                    }
                }
                _ => {} // other special keys consumed (no on-screen garbage)
            },
            Ok(Event::Resize(..)) => {} // caught by next before_input size poll
            _ => {}
        }
    }
    if !sgr.is_empty() { print!("\x1b[0m"); }
    if erase_after {
        // Wipe exactly the characters we echoed so the game's own echo is the only
        // copy; stay on the same line (no CR/LF) so the game's output continues
        // from wherever its prompt left the cursor.
        for _ in 0..echoed {
            print!("\x08 \x08");
        }
    } else {
        print!("\r\n"); // raw mode does not translate Enter to CRLF
    }
    let _ = terminal::disable_raw_mode();
    let _ = io::Write::flush(&mut io::stdout());
    (buf, 0)
}

/// Read one keypress. On a TTY: enter raw mode, read a crossterm `Event::Key`,
/// then restore cooked mode. Resize events during the wait are silently
/// discarded (they will be caught by the next `before_input` poll). Piped
/// stdin: return the first byte of the next line.
fn read_char_input(stdin_is_tty: bool) -> u32 {
    if !stdin_is_tty {
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
        let byte = line.bytes().next().unwrap_or(b'\n');
        return match byte {
            b'\n' | b'\r' => keycode::RETURN,
            0x7f | 0x08 => keycode::DELETE,
            b'\t' => keycode::TAB,
            0x1b => keycode::ESCAPE,
            b => b as u32,
        };
    }
    let _ = terminal::enable_raw_mode();
    let key = loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { code, .. })) => break decode_glk_keycode(code),
            Ok(Event::Resize(..)) => {} // caught by next before_input size poll
            _ => {}
        }
    };
    let _ = terminal::disable_raw_mode();
    key
}

// ── drive loop ────────────────────────────────────────────────────────────────

/// Emit an OSC 11 page-background update if the game's Normal-style background
/// changed since `last` (honor-gated, TTY-only). Called just before blocking for
/// input, so the per-instruction step loop never pays for the lookup.
fn emit_page_bg(machine: &Machine, honor: bool, stdout_is_tty: bool, last: &mut Option<(u8, u8, u8)>) {
    let cur = if honor && stdout_is_tty {
        machine
            .style_colour(gvm::WinType::TextBuffer, gvm::glk::GlkStyle::Normal)
            .bg
            .map(glk_term::rgb24)
    } else {
        None
    };
    if let Some(esc) = glk_term::page_bg_escape(cur, *last) {
        print!("{esc}");
        let _ = io::Write::flush(&mut io::stdout());
        *last = cur;
    }
}

/// Drive `machine` to completion through the Glk step loop, pulling input from
/// the supplied readers. `before_input` runs just before each blocking read (the
/// CLI uses it to flush output so the prompt is visible). This is the shared,
/// backend-agnostic loop — the CLI wires real stdin/terminal readers; tests wire
/// scripted ones.
fn drive(
    machine: &mut Machine,
    vfs_path: &std::path::Path,
    honor: bool,
    stdout_is_tty: bool,
    mut before_input: impl FnMut(&mut Machine),
    mut read_line: impl FnMut(LineEcho) -> (String, u32),
    mut read_char: impl FnMut() -> u32,
) {
    // Page background: reflect the game's Normal-style bg onto the terminal's
    // default background (OSC 11), honor-gated and TTY-only. Checked only when
    // about to block for input (the screen is settled then), which keeps the
    // per-instruction step loop free of the lookup. Emits only on change; reset
    // (OSC 111) happens once at the single teardown point in `main`.
    let mut last_page_bg: Option<(u8, u8, u8)> = None;
    // The per-game directory: `vfs_path` is always `game_dir/default.glkvfs`
    // (constructed that way in `main`, and by the tests below), so its parent
    // is the game dir — no need to thread a second path through.
    let game_dir = vfs_path.parent().unwrap_or(std::path::Path::new("."));
    loop {
        // Flush the Glk file VFS to its sidecar whenever a game mutation dirtied
        // it, each iteration, so a game's files survive even if the process is
        // killed mid-session. Silently tolerate a write failure.
        if machine.vfs_dirty() {
            std::fs::create_dir_all(vfs_path.parent().unwrap_or(std::path::Path::new("."))).ok();
            let _ = fs::write(vfs_path, machine.vfs_bytes());
            machine.clear_vfs_dirty();
        }
        match machine.step() {
            StepResult::Continue => {}
            StepResult::Quit => break,
            StepResult::NeedLine { .. } => {
                emit_page_bg(machine, honor, stdout_is_tty, &mut last_page_bg);
                before_input(machine);
                // Show live typing in raw mode and erase it on Enter, then arm a
                // deferred echo: if the game reprints the command itself in
                // style_Input (Inform 7 / Counterfeit Monkey) that stands; otherwise
                // the backend echoes it so it isn't lost (e.g. sensory, which relies
                // on library echo gvm does not implement). See
                // TerminalBackend::arm_input_echo (SQ-0275 + SQ-0282).
                let (line, terminator) = read_line(LineEcho::EraseOnEnter);
                let cmd = line.trim_end_matches(['\n', '\r']);
                if let Some(t) =
                    machine.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>()
                {
                    t.arm_input_echo(cmd.to_string());
                }
                machine.supply_line_terminated(cmd, terminator);
            }
            StepResult::NeedChar { .. } => {
                emit_page_bg(machine, honor, stdout_is_tty, &mut last_page_bg);
                before_input(machine);
                let key = read_char();
                machine.supply_char(key);
            }
            // Game create_by_prompt: ask the user for a filename (blank = cancel).
            // `read_line`/`before_input` are this fn's params; stderr is unbuffered
            // so eprint! needs no flush.
            StepResult::NeedFilename { usage, .. } => {
                // SavedGame usage is host-intercepted (@save -> fixed default slot),
                // so don't prompt for it — auto-name and move on.
                if usage & 0x0f == 0x01 {
                    machine.supply_filename(Some(format!("__prompt_{}__", usage & 0x0f)));
                } else {
                    before_input(machine);
                    eprint!("Filename (blank to cancel): ");
                    let (line, _) = read_line(LineEcho::Shown(String::new()));
                    let name = line.trim_end_matches(['\n', '\r']);
                    machine.supply_filename(if name.is_empty() { None } else { Some(name.to_string()) });
                }
            }
            // @save. The game's OWN fixed-name saves (create_by_name: CM's init
            // cache, undo, autotesting) are serviced SILENTLY against a fixed
            // path in game_dir — no prompt. Only the player's SAVE verb
            // (create_by_prompt) prompts for a filename.
            StepResult::SaveRequest => {
                let req = machine.pending_saveload_request().unwrap_or_default();
                if req.by_prompt || req.name.is_empty() {
                    before_input(machine);
                    eprint!("Save to file: ");
                    let (line, _) = read_line(LineEcho::Shown(String::new()));
                    let name = line.trim_end_matches(['\n', '\r']);
                    if name.is_empty() {
                        machine.complete_save(false);
                    } else {
                        let path = resolve_save_input(name, game_dir);
                        std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("."))).ok();
                        let ok = fs::write(&path, machine.save_quetzal()).is_ok();
                        if ok {
                            eprintln!("[saved to {}]", path.display());
                        } else {
                            eprintln!("[save failed]");
                        }
                        machine.complete_save(ok);
                    }
                } else {
                    // Silent game-managed save: <game_dir>/<fileref name>.qzl.
                    let path = game_auto_save_path(game_dir, &req.name);
                    std::fs::create_dir_all(game_dir).ok();
                    let ok = fs::write(&path, machine.save_quetzal()).is_ok();
                    machine.complete_save(ok);
                }
            }
            // @restore. Symmetric: the game's own saves read a fixed file
            // silently (clean-failing when it's absent — the first run — so the
            // game runs its init); only the player's RESTORE verb prompts.
            StepResult::RestoreRequest => {
                let req = machine.pending_saveload_request().unwrap_or_default();
                if req.by_prompt || req.name.is_empty() {
                    before_input(machine);
                    eprint!("Restore from file: ");
                    let (line, _) = read_line(LineEcho::Shown(String::new()));
                    let name = line.trim_end_matches(['\n', '\r']);
                    if name.is_empty() {
                        machine.complete_restore_failure();
                    } else {
                        let path = resolve_save_input(name, game_dir);
                        match fs::read(&path) {
                            Ok(bytes) if machine.complete_restore_quetzal(&bytes) => {
                                eprintln!("[restored from {}]", path.display());
                            }
                            _ => {
                                eprintln!("[restore failed]");
                                machine.complete_restore_failure();
                            }
                        }
                    }
                } else {
                    // Silent game-managed restore: read the fixed file if present.
                    let path = game_auto_save_path(game_dir, &req.name);
                    match fs::read(&path) {
                        Ok(bytes) if machine.complete_restore_quetzal(&bytes) => {}
                        _ => machine.complete_restore_failure(),
                    }
                }
            }
        }
    }
}

// ── story-key / per-game save resolution ────────────────────────────────────

/// Per-game directory name: the story file's basename (incl. extension),
/// sanitized to a filesystem-safe token. Empty -> "game".
fn story_key(story_path: &std::path::Path) -> String {
    let name = story_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let s: String = name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() { "game".to_string() } else { s }
}

/// The fixed path for a game-managed (create_by_name) save slot:
/// `<game_dir>/<sanitized fileref name>.qzl`. `name` is already sanitized by the
/// VM (gvm's `sanitize_fileref_name`), so `@save` and `@restore` on the same
/// fileref name resolve to the same file across launches — the payoff that lets
/// a game skip its init on relaunch. These names are game-internal (CM's begin
/// with `_`), keeping them clear of the player's `<name>.qzl` saves.
fn game_auto_save_path(game_dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    game_dir.join(format!("{name}.qzl"))
}

/// Seed the machine's host-managed SavedGame existence index from every
/// `<game_dir>/*.qzl` on disk (raw basename minus the `.qzl` suffix, matching
/// [`game_auto_save_path`] and how the index is keyed), so a `create_by_name`
/// game probing `glk_fileref_does_file_exist` before `@restore` sees its save
/// across launches (SQ-0301). No-op when `game_dir` is unreadable (e.g. a first
/// run before it exists). Over-seeding player-save `.qzl` names is inert.
fn seed_saved_games(machine: &mut Machine, game_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(game_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("qzl") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as u32;
        machine.seed_saved_game_file(name.to_string(), size);
    }
}

/// Resolve an interactive `@save`/`@restore` filename prompt: a bare name
/// (no path separator) lands in `game_dir` with a `.qzl` extension; a
/// path-bearing value is honored verbatim.
fn resolve_save_input(input: &str, game_dir: &std::path::Path) -> std::path::PathBuf {
    let t = input.trim();
    if t.contains('/') || t.contains('\\') {
        std::path::PathBuf::from(t)
    } else {
        let name = if t.ends_with(".qzl") { t.to_string() } else { format!("{t}.qzl") };
        game_dir.join(name)
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let argv: Vec<String> = env::args().collect();
    // Honour the game's stylehint colours by default; --no-game-colours opts out
    // (mirrors zvm-cli). The story path is the first non-flag argument.
    let honor = !argv.iter().any(|a| a == "--no-game-colours");
    // Acceleration (Glulx accelfunc interception) is on by default; --no-accel opts out.
    let accel = !argv.iter().any(|a| a == "--no-accel");
    // --data-dir takes a value, so it (and its value) must be consumed before
    // scanning for the positional story path — otherwise the path scan would
    // mistake the --data-dir value for the story argument.
    let mut data_dir: Option<String> = None;
    let mut path: Option<&String> = None;
    {
        let mut it = argv.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "--data-dir" {
                data_dir = it.next().cloned();
            } else if !a.starts_with("--") && path.is_none() {
                path = Some(a);
            }
        }
    }
    let Some(path) = path else {
        eprintln!(
            "Usage: {} [--no-game-colours] [--no-accel] [--data-dir <path>] <story.ulx | story.gblorb>",
            argv[0]
        );
        process::exit(1);
    };

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{path}': {e}");
            process::exit(1);
        }
    };

    let mut backend = TerminalBackend::new();
    backend.set_honor_colours(honor);
    let mut machine = match build_machine(bytes, Box::new(backend)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };
    machine.set_acceleration(accel);

    let stdin_is_tty = io::stdin().is_terminal();
    let stdout_is_tty = io::stdout().is_terminal();
    let both_tty = stdin_is_tty && stdout_is_tty;

    // Force a steady block cursor (SQ-0281); reset to the terminal default on exit.
    if stdout_is_tty {
        print!("{}", glk_term::cursor_steady_block());
        let _ = io::Write::flush(&mut io::stdout());
    }

    // Track the last-known terminal size so we can detect resize events.
    // Re-poll before each input using crossterm::terminal::size(); when
    // changed, update the backend and queue a Glk evtype_Arrange event so
    // the game can re-lay out its windows.
    let mut last_size: Option<(u32, u32)> = if both_tty {
        terminal::size().ok().map(|(c, r)| (c as u32, r as u32))
    } else {
        None
    };

    // Per-game directory: base (--data-dir override, else the story's own
    // directory) joined with the sanitized story filename.
    let story_path = std::path::PathBuf::from(path);
    let base = data_dir.map(std::path::PathBuf::from)
        .unwrap_or_else(|| story_path.parent().filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from(".")));
    // `.save` suffix keeps the dir from colliding with the story file itself
    // when `base` is the story's own directory (SQ-0294).
    let game_dir = base.join(format!("{}.save", story_key(&story_path)));

    // The Glk file VFS sidecar: `<game_dir>/default.glkvfs`. Loaded here before
    // the run, flushed dirty-gated inside `drive`, so a game's external files
    // (scores, preferences) survive a plain quit-and-relaunch.
    let vfs_path = game_dir.join("default.glkvfs");
    if vfs_path.exists() {
        // Tolerate a missing/unreadable sidecar silently — it just means empty.
        if let Ok(bytes) = fs::read(&vfs_path) {
            machine.load_vfs(&bytes);
        }
        machine.clear_vfs_dirty(); // loading is not a game mutation
    }

    // Reseed host-managed SavedGame slots from disk before the first drive: the
    // machine's existence index is session-transient, so a create_by_name game
    // probing glk_fileref_does_file_exist during init would otherwise never see
    // its own on-disk `.qzl` save across launches (SQ-0301).
    seed_saved_games(&mut machine, &game_dir);

    drive(
        &mut machine,
        &vfs_path,
        honor,
        stdout_is_tty,
        |m| {
            // Re-poll terminal size before each input (interactive TTY only).
            if both_tty {
                if let Ok((cols, rows)) = terminal::size() {
                    let new_size = (cols as u32, rows as u32);
                    if last_size != Some(new_size) {
                        last_size = Some(new_size);
                        let changed = m
                            .backend_mut()
                            .as_any_mut()
                            .downcast_mut::<TerminalBackend>()
                            .map(|b| b.update_size(cols as u32, rows as u32))
                            .unwrap_or(false);
                        if changed {
                            m.notify_resize();
                        }
                    }
                }
            }
            // Flush pending output (without tearing down the scroll region) so the
            // prompt is visible before we block for input.
            if let Some(t) = m.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>() {
                t.flush_out();
            }
        },
        move |echo| read_line_raw(stdin_is_tty, echo),
        move || read_char_input(stdin_is_tty),
    );

    machine.flush();
    // Ensure raw mode is not left active on exit (harmless if already off).
    let _ = terminal::disable_raw_mode();
    // Restore the terminal's own background (OSC 111): covers normal quit and
    // the fault/exit(70) path below (single teardown point; drive() never
    // resets itself, to avoid a double-reset).
    if honor && stdout_is_tty {
        print!("{}", glk_term::osc_reset_bg());
        let _ = io::Write::flush(&mut io::stdout());
    }
    // Restore the terminal's default cursor shape (SQ-0281). Not honor-gated —
    // the cursor is a UI preference, not a game colour.
    if stdout_is_tty {
        print!("{}", glk_term::cursor_reset());
        let _ = io::Write::flush(&mut io::stdout());
    }

    if let Some(trace) = machine.take_fault_trace() {
        for line in trace.to_lines() {
            eprintln!("{line}");
        }
        // Still surface any other diagnostics, then exit non-zero.
        for d in &machine.diagnostics {
            eprintln!("gvm: {d}");
        }
        std::process::exit(70);
    }

    for d in &machine.diagnostics {
        eprintln!("gvm: {d}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gvm::TestBackend;

    /// A hand-assembled Glulx image: open a TextBuffer window, make it current,
    /// print "Hi", then quit. Start function at 0x24 with one 4-byte local.
    ///
    /// Operand mode nibbles are packed low-nibble-first; opcodes ≥ 0x80 use the
    /// 2-byte form `opcode + 0x8000`. (See `crates/gvm/asm.rs` for the encoder
    /// the in-crate tests use; this crate hand-encodes one tiny program.)
    fn hi_image() -> Vec<u8> {
        let func: Vec<u8> = vec![
            0xC1, 0x04, 0x01, 0x00, 0x00, // type C1; one 4-byte local; terminator
            // setiosys 2, 0   (opcode 0x149 → 81 49; modes C8,C8)
            0x81, 0x49, 0x11, 0x02, 0x00,
            // push window_open args (last pushed is popped first = args[0]=split):
            //   rock=0, wintype=3, size=0, method=0, split=0
            0x40, 0x80, // copy const0 -> push (rock)
            0x40, 0x81, 0x03, // copy C8(3) -> push (wintype TextBuffer)
            0x40, 0x80, // size 0
            0x40, 0x80, // method 0
            0x40, 0x80, // split 0
            // @glk glk_window_open (sel 0x23, argc 5) -> local0
            0x81, 0x30, 0x11, 0x09, 0x23, 0x05, 0x00,
            // push local0; @glk glk_set_window (sel 0x2F, argc 1), discard
            0x40, 0x89, 0x00, // copy local0 -> push
            0x81, 0x30, 0x11, 0x00, 0x2F, 0x01,
            // streamchar 'H'; streamchar 'i'  (opcode 0x70; mode C8)
            0x70, 0x01, b'H',
            0x70, 0x01, b'i',
            // quit (0x120 → 81 20)
            0x81, 0x20,
        ];
        let (ramstart, endmem) = (0x100u32, 0x200u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes()); // EXTSTART == RAMSTART
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    // ── minimal Blorb builder (mirrors the blorb crate's test helper) ─────────
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// A Blorb wrapping a single Exec resource of the given chunk type + data.
    fn build_blorb(exec_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let ridx_data_len = 4 + 12; // count + one 12-byte entry
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let exec_chunk = chunk(exec_type, data);

        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // number
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let ridx_chunk = chunk(b"RIdx", &ridx);

        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&exec_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// Run `bytes` against a [`TestBackend`] and return the buffer-window text.
    fn run_capturing(bytes: Vec<u8>) -> String {
        let mut m = build_machine(bytes, Box::new(TestBackend::new())).unwrap();
        m.run();
        m.backend_mut()
            .as_any_mut()
            .downcast_mut::<TestBackend>()
            .unwrap()
            .all_text()
    }

    #[test]
    fn runs_raw_ulx_to_completion() {
        assert_eq!(run_capturing(hi_image()), "Hi");
    }

    #[test]
    fn extracts_and_runs_glulx_blorb() {
        let blorb = build_blorb(b"GLUL", &hi_image());
        assert!(blorb::Blorb::is_blorb(&blorb));
        assert_eq!(run_capturing(blorb), "Hi");
    }

    #[test]
    fn rejects_zcode_blorb() {
        let zblorb = build_blorb(b"ZCOD", b"fake z-code");
        let err = match build_machine(zblorb, Box::new(TestBackend::new())) {
            Err(e) => e,
            Ok(_) => panic!("expected the Z-code Blorb to be rejected"),
        };
        assert!(err.contains("Z-code"), "got: {err}");
    }

    #[test]
    fn passes_through_non_blorb_bytes() {
        let img = hi_image();
        assert_eq!(extract_executable(img.clone()).unwrap(), img);
    }

    // ── key decoding tests ────────────────────────────────────────────────────

    // ── story-key / per-game save resolution ──────────────────────────────────

    #[test]
    fn story_key_keeps_extension_and_sanitizes() {
        use std::path::Path;
        assert_eq!(story_key(Path::new("/g/Zork1.z5")), "Zork1.z5");
        assert_ne!(story_key(Path::new("/g/Zork1.z5")), story_key(Path::new("/g/Zork1.gblorb")));
        assert_eq!(story_key(Path::new("/g/a b?.z5")), "a_b_.z5");
        assert_eq!(story_key(Path::new("")), "game");
    }

    /// Regression for SQ-0284/SQ-0294: the CLI's default `base` is the
    /// story's own directory, so `base.join(story_key(..))` used to collide
    /// with the story file itself (`mkdir` on an existing filename fails).
    /// The `.save` suffix makes the per-game dir a distinct path.
    #[test]
    fn game_dir_does_not_collide_with_same_named_story_file() {
        let tmp = std::env::temp_dir().join(format!("gvm-cli-storage-collision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let story_path = tmp.join("game.gblorb");
        std::fs::write(&story_path, b"x").unwrap(); // a FILE named game.gblorb

        let game_dir = tmp.join(format!("{}.save", story_key(&story_path)));
        assert_eq!(game_dir, tmp.join("game.gblorb.save"));
        std::fs::create_dir_all(&game_dir).expect("must not collide with the story file");
        assert!(game_dir.is_dir());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_save_input_bare_vs_path() {
        use std::path::{Path, PathBuf};
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_save_input("quick", gd), PathBuf::from("/data/Zork1.z5/quick.qzl"));
        assert_eq!(resolve_save_input("quick.qzl", gd), PathBuf::from("/data/Zork1.z5/quick.qzl"));
        assert_eq!(resolve_save_input("/tmp/foo.qzl", gd), PathBuf::from("/tmp/foo.qzl"));
    }

    #[test]
    fn game_auto_save_path_is_fixed_named_in_game_dir() {
        use std::path::{Path, PathBuf};
        // A game-managed (create_by_name) slot resolves to a fixed
        // <game_dir>/<name>.qzl — same path across launches, so a relaunch's
        // @restore finds it (init-skip). Names are pre-sanitized by the VM.
        let gd = Path::new("/data/CM.gblorb.save");
        assert_eq!(
            game_auto_save_path(gd, "_Counterfeit_Monkey-startup-data"),
            PathBuf::from("/data/CM.gblorb.save/_Counterfeit_Monkey-startup-data.qzl"),
        );
    }

    #[test]
    fn vfs_path_is_default_glkvfs_in_game_dir() {
        use std::path::{Path, PathBuf};
        let gd = Path::new("/data/Advent.gblorb");
        assert_eq!(gd.join("default.glkvfs"), PathBuf::from("/data/Advent.gblorb/default.glkvfs"));
    }

    #[test]
    fn decode_glk_keycode_maps_crossterm_keys() {
        assert_eq!(decode_glk_keycode(KeyCode::Char('a')), b'a' as u32);
        assert_eq!(decode_glk_keycode(KeyCode::Char('7')), b'7' as u32);
        assert_eq!(decode_glk_keycode(KeyCode::Enter), keycode::RETURN);
        assert_eq!(decode_glk_keycode(KeyCode::Backspace), keycode::DELETE);
        assert_eq!(decode_glk_keycode(KeyCode::Delete), keycode::DELETE);
        assert_eq!(decode_glk_keycode(KeyCode::Tab), keycode::TAB);
        assert_eq!(decode_glk_keycode(KeyCode::Esc), keycode::ESCAPE);
        assert_eq!(decode_glk_keycode(KeyCode::Up), keycode::UP);
        assert_eq!(decode_glk_keycode(KeyCode::Down), keycode::DOWN);
        assert_eq!(decode_glk_keycode(KeyCode::Left), keycode::LEFT);
        assert_eq!(decode_glk_keycode(KeyCode::Right), keycode::RIGHT);
        assert_eq!(decode_glk_keycode(KeyCode::Home), keycode::HOME);
        assert_eq!(decode_glk_keycode(KeyCode::End), keycode::END);
        assert_eq!(decode_glk_keycode(KeyCode::PageUp), keycode::PAGE_UP);
        assert_eq!(decode_glk_keycode(KeyCode::PageDown), keycode::PAGE_DOWN);
        assert_eq!(decode_glk_keycode(KeyCode::F(1)), keycode::FUNC1);
        assert_eq!(decode_glk_keycode(KeyCode::F(12)), keycode::FUNC12);
        assert_eq!(decode_glk_keycode(KeyCode::F(13)), keycode::UNKNOWN, "F13 → unknown");
    }

    // A tiny Glulx instruction encoder for the input integration programs.
    // Immediates use the 4-byte constant mode, avoiding sign-extension pitfalls.
    #[derive(Clone, Copy)]
    enum E {
        Imm(u32),
        LocLoad(u8),
        LocStore(u8),
        MemLoad(u32),
        Push,
        Discard,
    }
    fn emode(e: E) -> u8 {
        match e {
            E::Imm(_) => 3,
            E::LocLoad(_) | E::LocStore(_) => 9,
            E::MemLoad(_) => 7,
            E::Push => 8,
            E::Discard => 0,
        }
    }
    fn edata(e: E) -> Vec<u8> {
        match e {
            E::Imm(v) | E::MemLoad(v) => v.to_be_bytes().to_vec(),
            E::LocLoad(o) | E::LocStore(o) => vec![o],
            E::Push | E::Discard => vec![],
        }
    }
    fn enc(op: u32, args: &[E]) -> Vec<u8> {
        let mut out = Vec::new();
        if op <= 0x7f {
            out.push(op as u8);
        } else {
            out.extend_from_slice(&((op | 0x8000) as u16).to_be_bytes());
        }
        let mut modes = vec![0u8; args.len().div_ceil(2)];
        for (i, &a) in args.iter().enumerate() {
            let m = emode(a);
            if i % 2 == 0 {
                modes[i / 2] |= m;
            } else {
                modes[i / 2] |= m << 4;
            }
        }
        out.extend_from_slice(&modes);
        for &a in args {
            out.extend(edata(a));
        }
        out
    }

    /// Wrap `body` as a one-local start function and a runnable image (RAMSTART
    /// 0x100, ENDMEM 0x200 → RAM [0x100, 0x200) for the event struct + buffers).
    fn image_for(body: Vec<u8>) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, 0x01, 0x00, 0x00]; // type C1; one 4-byte local
        func.extend(body);
        let (ramstart, endmem) = (0x100u32, 0x200u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    /// Open a TextBuffer (id stored in local0) and make it current.
    fn open_buffer_prelude() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        for v in [Imm(0), Imm(3), Imm(0), Imm(0), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // rock, wintype=3, size, method, split
        }
        b.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(0)])); // window_open → local0
        b.extend(enc(0x40, &[LocLoad(0), Push]));
        b.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(local0)
        b
    }

    #[test]
    fn drive_echoes_a_typed_character() {
        use E::*;
        // request_char_event(local0); select(@0x100); put_char(event.val1 @0x108).
        let mut body = open_buffer_prelude();
        body.extend(enc(0x40, &[LocLoad(0), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event
        body.extend(enc(0x40, &[Imm(0x100), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select(@0x100)
        body.extend(enc(0x40, &[MemLoad(0x108), Push])); // the key code (val1)
        body.extend(enc(0x130, &[Imm(0x80), Imm(1), Discard])); // glk_put_char(key)
        body.extend(enc(0x120, &[])); // quit

        let mut m = build_machine(image_for(body), Box::new(TestBackend::new())).unwrap();
        let mut keys = vec![b'Z' as u32].into_iter();
        drive(&mut m, std::path::Path::new("unused.glkvfs"), true, false, |_| {}, |_echo| (String::new(), 0), move || keys.next().unwrap_or(keycode::RETURN));
        let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "Z", "the typed key was supplied, stored, and echoed");
    }

    #[test]
    fn drive_reads_and_prints_a_line() {
        use E::*;
        // request_line_event(local0, buf=0x180, maxlen=20, initlen=0); select(@0x100);
        // put_buffer(0x180, event.val1 @0x108).
        let mut body = open_buffer_prelude();
        for v in [Imm(0), Imm(20), Imm(0x180), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // initlen, maxlen, buf, win
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(0x100), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select(@0x100)
        body.extend(enc(0x40, &[MemLoad(0x108), Push])); // len = val1
        body.extend(enc(0x40, &[Imm(0x180), Push])); // addr
        body.extend(enc(0x130, &[Imm(0x84), Imm(2), Discard])); // glk_put_buffer(addr, len)
        body.extend(enc(0x120, &[])); // quit

        let mut m = build_machine(image_for(body), Box::new(TestBackend::new())).unwrap();
        let mut lines = vec!["hello".to_string()].into_iter();
        drive(&mut m, std::path::Path::new("unused.glkvfs"), true, false, |_| {}, move |_echo| (lines.next().unwrap_or_default(), 0), || keycode::RETURN);
        let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "hello", "the typed line was supplied into the buffer and printed");
    }

    // ── .glkvfs sidecar persistence ───────────────────────────────────────────

    /// Wrap `body` as a start function with `nlocals` 4-byte locals, and place the
    /// NUL-terminated fileref name `"gvfstest"` in ROM at 0x1C0. RAMSTART is 0x200
    /// (Glulx requires the memory-map bounds to be 256-aligned) so the file
    /// programs' code has room below the name string.
    fn vfs_image(body: Vec<u8>, nlocals: u8) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, nlocals, 0x00, 0x00];
        func.extend(body);
        let (ramstart, endmem) = (0x200u32, 0x300u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        assert!(0x24 + func.len() <= 0x1C0, "program code overruns the name string");
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        let name = b"gvfstest\0";
        img[0x1C0..0x1C0 + name.len()].copy_from_slice(name);
        img
    }

    const NAME_ADDR: u32 = 0x1C0;

    /// A program: open a Data fileref by name, open it for Write, put "Hi", close,
    /// quit. Locals: local0=fref (off 0), local1=stream (off 4).
    fn write_hi_program() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        // fileref_create_by_name(usage=0, nameptr, rock=0) -> local0
        for v in [Imm(0), Imm(NAME_ADDR), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // reverse arg order: rock, nameptr, usage
        }
        b.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(0)]));
        // stream_open_file(fref=local0, fmode=1 Write, rock=0) -> local1
        b.extend(enc(0x40, &[Imm(0), Push])); // rock
        b.extend(enc(0x40, &[Imm(0x01), Push])); // fmode = Write
        b.extend(enc(0x40, &[LocLoad(0), Push])); // fref
        b.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(4)]));
        // put_char_stream(str=local1, ch): reverse order push ch, str
        for ch in [b'H', b'i'] {
            b.extend(enc(0x40, &[Imm(ch as u32), Push]));
            b.extend(enc(0x40, &[LocLoad(4), Push]));
            b.extend(enc(0x130, &[Imm(0x81), Imm(2), Discard]));
        }
        // stream_close(str=local1, resultptr=0)
        b.extend(enc(0x40, &[Imm(0), Push]));
        b.extend(enc(0x40, &[LocLoad(4), Push]));
        b.extend(enc(0x130, &[Imm(0x44), Imm(2), Discard]));
        b.extend(enc(0x120, &[])); // quit
        b
    }

    /// A program: open the same fileref by name, open it for Read, echo its two
    /// bytes to a TextBuffer window, quit. Locals: window(0), fref(4), stream(8),
    /// char(12).
    fn read_hi_program() -> Vec<u8> {
        use E::*;
        let mut b = open_buffer_prelude(); // window -> local0, made current
        // fileref_create_by_name -> local1
        for v in [Imm(0), Imm(NAME_ADDR), Imm(0)] {
            b.extend(enc(0x40, &[v, Push]));
        }
        b.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(4)]));
        // stream_open_file(fref=local1, fmode=2 Read, rock=0) -> local2
        b.extend(enc(0x40, &[Imm(0), Push]));
        b.extend(enc(0x40, &[Imm(0x02), Push])); // Read
        b.extend(enc(0x40, &[LocLoad(4), Push]));
        b.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(8)]));
        // read two bytes, echo each to the current (window) stream
        for _ in 0..2 {
            b.extend(enc(0x40, &[LocLoad(8), Push])); // str
            b.extend(enc(0x130, &[Imm(0x90), Imm(1), LocStore(12)])); // get_char_stream -> local3
            b.extend(enc(0x40, &[LocLoad(12), Push])); // ch
            b.extend(enc(0x130, &[Imm(0x80), Imm(1), Discard])); // put_char(ch)
        }
        b.extend(enc(0x120, &[])); // quit
        b
    }

    #[test]
    fn drive_persists_and_reloads_the_vfs_sidecar() {
        // A private temp dir so we never write sidecars next to real files.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gvmcli-vfs-{}-{}", std::process::id(), stamp));
        fs::create_dir_all(&dir).unwrap();
        let vfs_path = dir.join("story.glkvfs");

        // Run 1: a game writes "Hi" into a Glk file, then quits. drive's in-loop,
        // dirty-gated flush should persist the VFS to the sidecar.
        let mut m = build_machine(vfs_image(write_hi_program(), 2), Box::new(TestBackend::new())).unwrap();
        drive(&mut m, &vfs_path, true, false, |_| {}, |_echo| (String::new(), 0), || keycode::RETURN);

        assert!(vfs_path.exists(), "the .glkvfs sidecar is written to disk");
        let blob = fs::read(&vfs_path).unwrap();
        let files = gvm::glk::decode_files(&blob);
        let stored = files.values().next().expect("exactly one file persisted");
        assert_eq!(stored.as_slice(), b"Hi", "the written bytes are in the sidecar");

        // Run 2: a fresh machine loads the sidecar (as main() does before drive),
        // then a game reads the file back and echoes it.
        let mut m2 = build_machine(vfs_image(read_hi_program(), 4), Box::new(TestBackend::new())).unwrap();
        m2.load_vfs(&fs::read(&vfs_path).unwrap());
        m2.clear_vfs_dirty();
        drive(&mut m2, &vfs_path, true, false, |_| {}, |_echo| (String::new(), 0), || keycode::RETURN);
        let text = m2.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "Hi", "the persisted bytes are readable after load_vfs");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── in-game @save/@restore via a prompted filename (SQ-0283 Task 5) ──────

    /// A program that exercises the full in-game save/restore prompt path:
    /// creates a SavedGame fileref by prompt (host-intercepted usage `0x01`,
    /// auto-resolved without a `read_line` call), opens it for writing, issues
    /// `@save`, then — only on the fresh (not-yet-restored) run — creates a
    /// second SavedGame fileref for reading, opens it, and issues `@restore`.
    /// The `jeq` guards against re-running that block on the resumed
    /// (post-restore) run: `@save`'s S1 then reads back `-1` (the "just
    /// restored" sentinel), and execution always resumes just after the
    /// original `@save`, so without the guard the read+`@restore` block would
    /// run again forever. Locals: fref-w(0), stream-w(4), fref-r(8), stream-r(12).
    fn save_restore_by_prompt_program() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk

        // fileref_create_by_prompt(usage=1 SavedGame, fmode=1 Write, rock=0) -> local0
        b.extend(enc(0x40, &[Imm(0), Push])); // rock
        b.extend(enc(0x40, &[Imm(0x01), Push])); // fmode = Write
        b.extend(enc(0x40, &[Imm(0x01), Push])); // usage = SavedGame (topmost -> args[0])
        b.extend(enc(0x130, &[Imm(0x62), Imm(3), LocStore(0)]));
        // stream_open_file(fref=local0, fmode=1 Write, rock=0) -> local1
        b.extend(enc(0x40, &[Imm(0), Push])); // rock
        b.extend(enc(0x40, &[Imm(0x01), Push])); // fmode = Write
        b.extend(enc(0x40, &[LocLoad(0), Push])); // fref
        b.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(4)]));

        // @save L1=local1, S1 -> mem[0x280]
        b.extend(enc(0x123, &[LocLoad(4), MemLoad(0x280)]));

        // The read-back + @restore block, skipped on the resumed run.
        let mut skip = Vec::new();
        // fileref_create_by_prompt(usage=1 SavedGame, fmode=2 Read, rock=0) -> local2
        skip.extend(enc(0x40, &[Imm(0), Push])); // rock
        skip.extend(enc(0x40, &[Imm(0x02), Push])); // fmode = Read
        skip.extend(enc(0x40, &[Imm(0x01), Push])); // usage = SavedGame
        skip.extend(enc(0x130, &[Imm(0x62), Imm(3), LocStore(8)]));
        // stream_open_file(fref=local2, fmode=2 Read, rock=0) -> local3
        skip.extend(enc(0x40, &[Imm(0), Push])); // rock
        skip.extend(enc(0x40, &[Imm(0x02), Push])); // fmode = Read
        skip.extend(enc(0x40, &[LocLoad(8), Push])); // fref
        skip.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(12)]));
        // @restore L1=local3, S1 -> mem[0x288]
        skip.extend(enc(0x124, &[LocLoad(12), MemLoad(0x288)]));

        let skip_len = skip.len() as u32;
        // jeq mem[0x280], -1, +skip_len -> skip the read-back+@restore block once
        // S1 reads the "just restored" sentinel.
        b.extend(enc(0x24, &[MemLoad(0x280), Imm(0xFFFF_FFFF), Imm(skip_len + 2)]));
        b.extend(skip);
        b.extend(enc(0x120, &[])); // quit
        b
    }

    #[test]
    fn drive_ingame_save_restore_round_trips_via_prompted_filename() {
        // A private temp dir so we never write next to real files.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gvmcli-qzl-{}-{}", std::process::id(), stamp));
        fs::create_dir_all(&dir).unwrap();
        let save_path = dir.join("story.qzl");
        let vfs_path = dir.join("story.glkvfs");

        // SavedGame create_by_prompt auto-resolves (no read_line call); the only
        // prompts driven through `read_line` are the SaveRequest/RestoreRequest
        // filename prompts, both answered with the same fixed path so the
        // restore reads back exactly what the save wrote.
        let save_path_str = save_path.to_str().unwrap().to_string();
        let mut m =
            build_machine(vfs_image(save_restore_by_prompt_program(), 4), Box::new(TestBackend::new())).unwrap();
        drive(&mut m, &vfs_path, true, false, |_| {}, move |_echo| (save_path_str.clone(), 0), || {
            keycode::RETURN
        });

        let bytes = fs::read(&save_path).expect("the prompted filename was written by SaveRequest");
        fn has_chunk(save: &[u8], id: &[u8; 4]) -> bool {
            save.windows(4).any(|w| w == id)
        }
        assert!(has_chunk(&bytes, b"IFhd"), "the .qzl carries IFhd");
        assert!(has_chunk(&bytes, b"CMem"), "the .qzl carries CMem");
        assert!(has_chunk(&bytes, b"Stks"), "the .qzl carries Stks");
        assert!(!has_chunk(&bytes, b"GReg"), "the .qzl is bare, not a full save_state snapshot");
        assert!(!has_chunk(&bytes, b"Glk "), "the .qzl is bare, not a full save_state snapshot");

        // The drive loop's own RestoreRequest arm already read this file back
        // mid-game (via complete_restore_quetzal, reaching Quit); confirm the
        // written bytes are independently a valid, self-contained save too.
        assert!(m.restore_quetzal(&bytes).is_ok(), "the written .qzl round-trips via restore_quetzal");

        let _ = fs::remove_dir_all(&dir);
    }
}
