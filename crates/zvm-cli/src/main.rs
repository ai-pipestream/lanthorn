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

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

mod screen;
mod aux; // implemented in Task 5; declared now so the module tree is stable

// ── Pure word-wrap helper ─────────────────────────────────────────────────────

/// Soft-wrap `text` by word-token boundaries, tracking `current_col` as the
/// starting column position. Returns `(wrapped_text, new_col)`. Explicit `\n`
/// in `text` always resets the column to 0. When `cols` is `u16::MAX`,
/// wrapping is disabled (used when `buffer_mode` is off).
fn wrap_line(text: &str, cols: u16, current_col: u16) -> (String, u16) {
    let mut out = String::with_capacity(text.len());
    let mut col = current_col;
    for word in text.split_inclusive(&[' ', '\n'][..]) {
        let is_nl = word.ends_with('\n');
        let clean = if is_nl { &word[..word.len() - 1] } else { word };
        let wlen = clean.chars().count() as u16;
        if col > 0 && col.saturating_add(wlen) > cols {
            out.push('\n');
            let trimmed = clean.trim_start();
            out.push_str(trimmed);
            col = trimmed.chars().count() as u16;
        } else {
            out.push_str(clean);
            col = col.saturating_add(wlen);
        }
        if is_nl {
            out.push('\n');
            col = 0;
        }
    }
    (out, col)
}

// ── StdoutOutput ──────────────────────────────────────────────────────────────

/// Output sink that writes directly to stdout and flushes after each call.
/// On a TTY it wraps styled lower-window text in SGR; when piped it stays plain.
struct StdoutOutput {
    is_tty: bool,
    paging: bool,
    page_height: u16,
    cols: u16,
    current_col: u16,
    lines: u16,
    /// Mirrors `machine.screen.buffer_mode`: when `false`, soft word-wrap at
    /// the column limit is suppressed (per Z-spec, unwrapped output is flushed
    /// immediately; explicit `\n` and `[MORE]` paging still apply).
    buffer_mode: bool,
    /// When `false`, game-supplied fg/bg colour SGR is suppressed at render time
    /// even if a non-conformant game sets colour after the header bit is cleared.
    /// Style bits (reverse/bold/italic) are always preserved.
    honor_game_colours: bool,
}

impl StdoutOutput {
    fn new(is_tty: bool, paging: bool, page_height: u16, cols: u16, honor_game_colours: bool) -> Self {
        StdoutOutput {
            is_tty,
            paging,
            page_height,
            cols,
            current_col: 0,
            lines: 0,
            buffer_mode: false,
            honor_game_colours,
        }
    }

    fn check_paging(&mut self) {
        if self.paging && crate::screen::should_page(self.lines, self.page_height) {
            let _ = io::stdout().flush();
            print!("\x1b[7m[MORE]\x1b[0m");
            let _ = io::stdout().flush();
            wait_for_keypress(self.is_tty);
            print!("\r\x1b[2K"); // erase the [MORE] prompt
            self.lines = 0;
        }
    }

    /// Emit `s` with optional token-based word wrapping (gated by `buffer_mode`).
    /// Calls `check_paging` after each newline (soft or hard).
    fn write_counted(&mut self, s: &str) {
        // When buffer_mode is off, pass u16::MAX as cols: saturating_add in
        // wrap_line will never trigger the wrap condition.
        let cols = if self.buffer_mode { self.cols } else { u16::MAX };
        let (wrapped, new_col) = wrap_line(s, cols, self.current_col);
        for ch in wrapped.chars() {
            print!("{}", ch);
            if ch == '\n' {
                self.lines += 1;
                self.check_paging();
            }
        }
        self.current_col = new_col;
        let _ = io::stdout().flush();
    }
}

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) {
        self.write_counted(s);
    }

    fn print_styled(&mut self, s: &str, style: u8) {
        use zvm::io::TextAttrs;
        self.print_attr(s, TextAttrs { style, ..Default::default() });
    }

    fn print_attr(&mut self, s: &str, attrs: zvm::io::TextAttrs) {
        // When honour is off, strip game fg/bg so a non-conformant game that sets
        // colour after the header bit is cleared still renders without colour.
        // Style bits (reverse/bold/italic) are preserved unconditionally.
        let effective = if self.honor_game_colours {
            attrs
        } else {
            zvm::io::TextAttrs {
                fg: zvm::screen::ZColour::Default,
                bg: zvm::screen::ZColour::Default,
                ..attrs
            }
        };
        let out = crate::screen::style_wrap(s, effective, self.is_tty);
        self.write_counted(&out);
    }

    fn set_buffer_mode(&mut self, on: bool) {
        self.buffer_mode = on;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── Blorb extraction ──────────────────────────────────────────────────────────

/// If `bytes` is a Blorb, return its Z-code executable; reject Glulx with a
/// clear error; otherwise pass the bytes through unchanged (a raw story file).
fn extract_story(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes).map_err(|e| format!("Error: invalid Blorb: {e:?}"))?;
    match b.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::Glulx, _)) => {
            Err("Error: Glulx story files are not yet supported.".to_string())
        }
        Err(e) => Err(format!("Error: Blorb has no executable: {e:?}")),
    }
}

// ── build_machine ─────────────────────────────────────────────────────────────

fn build_machine(
    story: Vec<u8>,
    stdout_is_tty: bool,
    paging: bool,
    page_height: u16,
    term_cols: u16,
    honor_game_colours: bool,
    interpreter_number: Option<u8>,
) -> Result<Machine, String> {
    use zvm::error::ZError;
    let mem = Memory::new(story).map_err(|e| match e {
        ZError::GraphicalV6 => "Error: Z-machine v6 graphical games are not supported.".to_string(),
        ZError::UnsupportedVersion(v) => format!("Error: Z-machine version {v} is not supported."),
        ZError::NotAStoryFile => "Error: file is not a valid Z-machine story file.".to_string(),
        ZError::Truncated => "Error: story file is truncated.".to_string(),
        _ => format!("Error loading story: {e:?}"),
    })?;
    let mut machine = Machine::with_output(mem, Box::new(StdoutOutput::new(
        stdout_is_tty,
        paging,
        page_height,
        term_cols,
        honor_game_colours,
    )));
    machine.set_interpreter_number(interpreter_number);
    machine.init_caps();
    Ok(machine)
}

// ── argument parsing ──────────────────────────────────────────────────────────

struct Args {
    story: Option<String>,
    no_status: bool,
    no_aux: bool,
    no_more: bool,
}

fn parse_args(argv: &[String]) -> Args {
    let mut a = Args { story: None, no_status: false, no_aux: false, no_more: false };
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--no-status" | "--lower-only" => a.no_status = true,
            "--no-aux" => a.no_aux = true,
            "--no-more" | "--no-page" => a.no_more = true,
            "--no-game-colours" => {}
            "-I" | "--interpreter" => i += 1, // also skip the following value token
            s if !s.starts_with("--") && a.story.is_none() => a.story = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }
    a
}

/// Returns `true` (honour game colours) unless `--no-game-colours` is present.
fn parse_game_colours(args: &[String]) -> bool {
    !args.iter().any(|a| a == "--no-game-colours")
}

/// Read the interpreter-number override from `-I N` / `--interpreter N`.
/// Returns None when absent or when N is not a valid u8 (lenient — falls back
/// to the engine's Frotz default).
fn parse_interpreter(args: &[String]) -> Option<u8> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-I" || a == "--interpreter" {
            return it.next().and_then(|v| v.parse::<u8>().ok());
        }
    }
    None
}

// ── terminal size ─────────────────────────────────────────────────────────────

/// Detect the terminal size. Returns `(rows, cols)` using crossterm; falls
/// back to 24×80 if crossterm returns an error (e.g. stdout is piped).
fn detect_term_size() -> (u16, u16) {
    match terminal::size() {
        // A zero dimension (some PTYs, e.g. macOS `script`, return Ok((0, 0)))
        // would yield a 0 wrap width; fall back to the default like an error.
        Ok((cols, rows)) if cols > 0 && rows > 0 => (rows, cols),
        _ => (screen::DEFAULT_ROWS, screen::DEFAULT_COLS),
    }
}

// ── key input (crossterm) ─────────────────────────────────────────────────────

/// Map a crossterm `KeyCode` to the Z-machine key code used by `supply_char`.
/// Printable ASCII passes through; special keys map to ZMSD §3.8 codes.
fn decode_keycode(code: KeyCode) -> u8 {
    match code {
        KeyCode::Char(c) if (c as u32) < 128 => c as u8,
        KeyCode::Enter => b'\n',
        KeyCode::Backspace | KeyCode::Delete => 8, // DEL/BS
        KeyCode::Esc => 0x1B,
        KeyCode::Up => 129,
        KeyCode::Down => 130,
        KeyCode::Left => 131,
        KeyCode::Right => 132,
        // Function keys F1–F12 → ZSCII 133–144 (ZMSD §3.8). Keypad digits
        // (ZSCII 145–154) are unreachable: terminals report them as ordinary
        // Char events, indistinguishable from the number row.
        KeyCode::F(n) if (1..=12).contains(&n) => 132 + n,
        _ => b'\n', // unknown → newline
    }
}

/// Wait for any key press (used by the `[MORE]` prompt). Resize events during
/// the wait are discarded; the next NeedLine/NeedChar poll will pick them up.
fn wait_for_keypress(is_tty: bool) {
    if !is_tty {
        read_byte_stdin();
        return;
    }
    let _ = terminal::enable_raw_mode();
    loop {
        match event::read() {
            Ok(Event::Key(_)) => break,
            _ => {} // ignore resize and other events during [MORE] wait
        }
    }
    let _ = terminal::disable_raw_mode();
}

/// Read one keypress in raw mode. Resize events that arrive before the key are
/// captured and returned alongside the key so the caller can update its state.
/// Piped stdin: reads the first byte of the next line.
fn read_char_input(is_tty: bool) -> (u8, Option<(u16, u16)>) {
    if !is_tty {
        return (read_byte_stdin(), None);
    }
    let _ = terminal::enable_raw_mode();
    let mut last_resize: Option<(u16, u16)> = None;
    let key = loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { code, .. })) => break decode_keycode(code),
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            _ => {}
        }
    };
    let _ = terminal::disable_raw_mode();
    (key, last_resize)
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

/// Read a line of input in RAW mode, echoing as the user types. Unlike cooked
/// `read_line_stdin`, arrow/function-key escape sequences are parsed by
/// crossterm into key events (so they never leak onto the screen as garbage),
/// and the echo is drawn in the game's current style/colour (`echo`). Resize
/// events during the read are captured and returned so the caller can update
/// its layout. Falls back to cooked line input when stdin is not a TTY (piped).
fn read_line_raw(is_tty: bool, echo: zvm::io::TextAttrs) -> (String, Option<(u16, u16)>) {
    if !is_tty {
        return (read_line_stdin(), None);
    }
    let _ = terminal::enable_raw_mode();
    let mut buf = String::new();
    let mut last_resize: Option<(u16, u16)> = None;
    let sgr = crate::screen::sgr_open(echo);
    if !sgr.is_empty() {
        print!("{sgr}");
        let _ = io::stdout().flush();
    }
    loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { code, modifiers, .. })) => match code {
                KeyCode::Enter => break,
                // Ctrl-C / Ctrl-D: raw mode swallows signals, so exit cleanly
                // ourselves (drop the colour, leave raw mode + the scroll region).
                KeyCode::Char('c') | KeyCode::Char('d')
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if !sgr.is_empty() { print!("\x1b[0m"); }
                    print!("\r\n");
                    let _ = io::stdout().flush();
                    let _ = terminal::disable_raw_mode();
                    print!("{}", crate::screen::leave_region());
                    let _ = io::stdout().flush();
                    std::process::exit(0);
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    print!("{c}");
                    let _ = io::stdout().flush();
                }
                KeyCode::Backspace => {
                    if buf.pop().is_some() {
                        // Move left, erase, move left again.
                        print!("\x08 \x08");
                        let _ = io::stdout().flush();
                    }
                }
                // Arrows, function keys, etc. are consumed (no on-screen garbage).
                _ => {}
            },
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            _ => {}
        }
    }
    if !sgr.is_empty() {
        print!("\x1b[0m");
    }
    let _ = terminal::disable_raw_mode();
    print!("\r\n"); // raw mode does not translate Enter to CRLF
    let _ = io::stdout().flush();
    (buf, last_resize)
}

fn read_byte_stdin() -> u8 {
    let stdin = io::stdin();
    let mut line = String::new();
    let _ = stdin.lock().read_line(&mut line);
    // Return the first byte of the input, or newline if empty.
    line.bytes().next().unwrap_or(b'\n')
}

// ── terminal resize helper ────────────────────────────────────────────────────

/// Apply a new terminal size `(new_rows, new_cols)` if it differs from the
/// current `(last_rows, last_cols)`. Updates the output sink, paging height,
/// and screen view. Only runs when `is_tty` is true.
fn apply_resize(
    new_rows: u16,
    new_cols: u16,
    last_rows: &mut u16,
    last_cols: &mut u16,
    page_height: &mut u16,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
) {
    if new_rows == *last_rows && new_cols == *last_cols {
        return;
    }
    *last_rows = new_rows;
    *last_cols = new_cols;
    *page_height = new_rows.saturating_sub(2).max(2);
    view.set_term_rows(new_rows);
    if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
        o.cols = new_cols;
        o.page_height = *page_height;
    }
}

/// Poll the current terminal size and apply it if it changed. Only runs when
/// `is_tty` is true (otherwise no-op — avoids crossterm errors on piped I/O).
fn maybe_resize(
    is_tty: bool,
    last_rows: &mut u16,
    last_cols: &mut u16,
    page_height: &mut u16,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
) {
    if !is_tty {
        return;
    }
    let (new_rows, new_cols) = detect_term_size();
    apply_resize(new_rows, new_cols, last_rows, last_cols, page_height, machine, view);
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let argv: Vec<String> = env::args().collect();
    let args = parse_args(&argv);
    let Some(story_arg) = args.story.clone() else {
        eprintln!("Usage: {} [--no-status] [--no-aux] <story-file>", argv[0]);
        std::process::exit(1);
    };
    let story_path = std::path::PathBuf::from(&story_arg);

    let story_bytes = match fs::read(&story_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{}': {}", story_path.display(), e);
            std::process::exit(1);
        }
    };

    // A .zblorb is a raw Blorb container; extract the embedded Z-code (reject
    // Glulx cleanly). Non-Blorb bytes (a raw .z5) pass through unchanged.
    let story_bytes = match extract_story(story_bytes) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Keep the original bytes for Restart.
    let original_bytes = story_bytes.clone();

    // Compute the IFID once; the aux file lives next to the story, keyed by IFID.
    let ifid = zvm::ifid::compute_ifid(&original_bytes);
    let aux_file = aux::aux_path(&story_path, &ifid);

    let stdout_is_tty = io::stdout().is_terminal();
    let stdin_is_tty = io::stdin().is_terminal();
    let both_tty = stdout_is_tty && stdin_is_tty;

    // Paging is only safe when BOTH ends are TTYs (else it would block the
    // headless harness); --no-more disables it.
    let (mut term_rows, mut term_cols) = detect_term_size();
    let paging = both_tty && !args.no_more;
    let mut page_height = term_rows.saturating_sub(2).max(2);

    let honor = parse_game_colours(&argv);
    let interpreter = parse_interpreter(&argv);
    let mut machine = match build_machine(
        story_bytes,
        stdout_is_tty,
        paging,
        page_height,
        term_cols,
        honor,
        interpreter,
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    machine.set_honor_game_colours(honor);
    aux_preload(&mut machine, &aux_file, args.no_aux);

    let mut view = screen::ScreenView::new(stdout_is_tty, args.no_status, term_rows);
    print!("{}", view.start());
    let _ = io::stdout().flush();

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
        // erase_window request (ZMSD §8.7.3): clear the lower window and reset
        // the pinned region so a game's screen clear / help-menu takeover (e.g.
        // Lost Pig's HELP, which issues erase_window -1) doesn't leave stale
        // story text bleeding through beneath the upper window.
        if machine.screen.erase_lower_requested {
            print!("{}", view.erase(machine.screen.current_bg, machine.honor_game_colours));
            let _ = io::stdout().flush();
            if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                o.lines = 0;
                o.current_col = 0;
            }
            machine.screen.erase_lower_requested = false;
        }
        // Persist aux tables as soon as the game commits one.
        aux_flush(&mut machine, &aux_file, args.no_aux);

        match step {
            StepResult::Continue => {}

            StepResult::Quit => {
                print!("{}", view.leave());
                let _ = io::stdout().flush();
                // Ensure raw mode is not left active on exit.
                let _ = terminal::disable_raw_mode();
                break;
            }

            StepResult::Restart => {
                machine = match build_machine(
                    original_bytes.clone(),
                    stdout_is_tty,
                    paging,
                    page_height,
                    term_cols,
                    honor,
                    interpreter,
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                machine.set_honor_game_colours(honor);
                aux_preload(&mut machine, &aux_file, args.no_aux);
            }

            StepResult::NeedLine { .. } => {
                // Poll for terminal resize before line input (crossterm returns
                // current size; on piped stdout this is a no-op via is_tty guard).
                maybe_resize(both_tty, &mut term_rows, &mut term_cols, &mut page_height, &mut machine, &mut view);
                print!("{}", view.frame(&machine));
                let _ = io::stdout().flush();
                // Echo input in the game's current style/colour (Default unless a
                // game set colour and honoring is on — matching the output sink).
                let echo = if honor {
                    zvm::io::TextAttrs {
                        style: machine.screen.text_style,
                        fg: machine.screen.current_fg,
                        bg: machine.screen.current_bg,
                    }
                } else {
                    zvm::io::TextAttrs::default()
                };
                let (line, resize) = read_line_raw(stdin_is_tty, echo);
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                machine.supply_line(line.trim_end(), 13);
                if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                    o.lines = 0;
                    o.current_col = 0; // cursor is at line start after user input + Enter
                }
            }

            StepResult::NeedChar => {
                // Poll for terminal resize before char input.
                maybe_resize(both_tty, &mut term_rows, &mut term_cols, &mut page_height, &mut machine, &mut view);
                print!("{}", view.frame(&machine));
                let _ = io::stdout().flush();
                let (ch, resize) = read_char_input(stdin_is_tty);
                // Handle any resize that happened DURING the char read.
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                machine.supply_char(ch);
                if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                    o.lines = 0;
                    o.current_col = 0;
                }
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

    #[test]
    fn parses_no_more_flag() {
        let a = parse_args(&["zvm-cli".into(), "--no-more".into(), "g".into()]);
        assert!(a.no_more);
        let b = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!b.no_more);
    }

    #[test]
    fn game_colours_default_on_unless_disabled() {
        assert!(parse_game_colours(&[]));
        assert!(parse_game_colours(&["story.z5".into()]));
        assert!(!parse_game_colours(&["--no-game-colours".into(), "story.z5".into()]));
    }

    #[test]
    fn parse_interpreter_reads_flag() {
        assert_eq!(parse_interpreter(&["-I".into(), "4".into(), "story.z5".into()]), Some(4));
        assert_eq!(parse_interpreter(&["--interpreter".into(), "3".into(), "story.z5".into()]), Some(3));
    }

    #[test]
    fn parse_interpreter_absent_is_none() {
        assert_eq!(parse_interpreter(&["story.z5".into()]), None);
    }

    #[test]
    fn parse_interpreter_bad_value_is_none() {
        assert_eq!(parse_interpreter(&["-I".into(), "notanumber".into(), "story.z5".into()]), None);
    }
}

#[cfg(test)]
mod stdout_tests {
    // The sink writes to the real stdout, so its behavior is exercised by the
    // manual smoke; this pins the wrapping helper the sink must use.
    #[test]
    fn print_styled_wraps_only_on_tty() {
        use zvm::io::TextAttrs;
        assert_eq!(crate::screen::style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(crate::screen::style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, false), "hi");
    }

    /// CLI gate: when honor_game_colours is OFF, print_attr strips fg/bg before
    /// passing to style_wrap so no colour SGR is emitted — but reverse/bold/italic
    /// style bits are always preserved.
    #[test]
    fn honour_off_strips_colour_preserves_style_bits() {
        use zvm::io::TextAttrs;
        use zvm::screen::ZColour;
        // Attrs with fg=red (Standard(3)→SGR 31), bg=blue (Standard(6)→SGR 44),
        // and reverse+bold style bits.
        let attrs = TextAttrs { style: 0x03, fg: ZColour::Standard(3), bg: ZColour::Standard(6) };

        // With honour ON: colour SGR present.
        let with_honour = crate::screen::style_wrap("hi", attrs, true);
        assert!(with_honour.contains("31"), "fg red SGR present with honour: {with_honour:?}");
        assert!(with_honour.contains("44"), "bg blue SGR present with honour: {with_honour:?}");

        // With honour OFF: strip fg/bg, pass Default channels to style_wrap.
        // This mirrors what StdoutOutput::print_attr does when honor_game_colours=false.
        let stripped = TextAttrs { fg: ZColour::Default, bg: ZColour::Default, ..attrs };
        let without_honour = crate::screen::style_wrap("hi", stripped, true);
        assert!(!without_honour.contains("31"), "fg colour absent when honour=false: {without_honour:?}");
        assert!(!without_honour.contains("44"), "bg colour absent when honour=false: {without_honour:?}");
        // Reverse (7) and bold (1) must still be present.
        assert!(without_honour.contains('7'), "reverse SGR preserved: {without_honour:?}");
        assert!(without_honour.contains('1'), "bold SGR preserved: {without_honour:?}");
    }
}

#[cfg(test)]
mod keycode_tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[test]
    fn decode_keycode_printable_ascii() {
        assert_eq!(decode_keycode(KeyCode::Char('a')), b'a');
        assert_eq!(decode_keycode(KeyCode::Char('Z')), b'Z');
        assert_eq!(decode_keycode(KeyCode::Char('5')), b'5');
    }

    #[test]
    fn decode_keycode_special_keys() {
        assert_eq!(decode_keycode(KeyCode::Enter), b'\n');
        assert_eq!(decode_keycode(KeyCode::Backspace), 8);
        assert_eq!(decode_keycode(KeyCode::Esc), 0x1B);
        assert_eq!(decode_keycode(KeyCode::Up), 129);
        assert_eq!(decode_keycode(KeyCode::Down), 130);
        assert_eq!(decode_keycode(KeyCode::Left), 131);
        assert_eq!(decode_keycode(KeyCode::Right), 132);
        assert_eq!(decode_keycode(KeyCode::F(1)), 133);
        assert_eq!(decode_keycode(KeyCode::F(4)), 136);
        assert_eq!(decode_keycode(KeyCode::F(5)), 137);
        assert_eq!(decode_keycode(KeyCode::F(12)), 144);
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::wrap_line;

    #[test]
    fn no_wrap_when_enough_space() {
        let (out, col) = wrap_line("hello world", 80, 0);
        assert_eq!(out, "hello world");
        assert_eq!(col, 11);
    }

    #[test]
    fn wraps_word_that_overflows_line() {
        // "world" at col 76: 76+5=81 > 80 → soft newline then "world"
        let (out, col) = wrap_line("world", 80, 76);
        assert_eq!(out, "\nworld");
        assert_eq!(col, 5);
    }

    #[test]
    fn no_wrap_at_line_start_even_if_word_is_long() {
        // A word longer than cols at col 0 is printed as-is (avoids infinite wrap).
        let long = "x".repeat(100);
        let (out, col) = wrap_line(&long, 80, 0);
        assert_eq!(out, long);
        assert_eq!(col, 100);
    }

    #[test]
    fn buffer_mode_off_disables_wrap() {
        // With cols = u16::MAX (buffer_mode off), no soft newline is ever inserted.
        let text = "this is a very long sentence that would normally be wrapped at 80 columns";
        let (out, _) = wrap_line(text, u16::MAX, 0);
        assert_eq!(out, text);
        assert!(!out.contains('\n'));
    }

    #[test]
    fn explicit_newline_resets_column() {
        let (out, col) = wrap_line("foo\nbar", 80, 70);
        assert_eq!(out, "foo\nbar");
        assert_eq!(col, 3);
    }

    #[test]
    fn trims_space_token_that_triggers_wrap() {
        // A space token at col 80 triggers a wrap; the space itself is trimmed.
        // " " at col 80: 80+1=81>80 → wrap, trim(" ")="", col=0
        // "world" at col 0: 0+5=5<=80 → emit, col=5
        let (out, col) = wrap_line(" world", 80, 80);
        assert_eq!(out, "\nworld");
        assert_eq!(col, 5);
    }

    #[test]
    fn multi_word_wrap_sequence() {
        // At cols=10, starting at col 0: "hello world" wraps.
        let (out, col) = wrap_line("hello world", 10, 0);
        // "hello " is 6 chars (≤10), then "world" (5) pushes 6+5=11 > 10 → wrap
        assert!(out.contains('\n'), "expected soft newline: {out:?}");
        assert_eq!(col, 5); // "world" = 5 chars after the wrap
    }
}
