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
//   RestoreRequest → prompt for filename, read Quetzal bytes, complete_restore_success

use std::any::Any;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;

use cli_host::{HostMode, TerminalGuard};

use zvm::cpu::exec::{Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

mod screen;
mod auxiliary; // "aux" is a reserved filename on Windows — module renamed accordingly

// ── sound ──────────────────────────────────────────────────────────────────────

/// The CLI's owned audio state: backend + resolved Blorb + live sound tracking.
struct CliSound {
    backend: audio::AudioBackend,
    blorb: Option<blorb::Blorb>,
    ids: HashMap<u16, audio::SoundId>,
    routines: HashMap<audio::SoundId, u16>,
}

fn sound_kind_to_format(k: blorb::SoundKind) -> Option<audio::SoundFormat> {
    match k {
        blorb::SoundKind::Aiff => Some(audio::SoundFormat::Aiff),
        blorb::SoundKind::Ogg => Some(audio::SoundFormat::Ogg),
        blorb::SoundKind::Mod => Some(audio::SoundFormat::Mod),
        blorb::SoundKind::Other => None,
    }
}

/// Play a drained batch of `SoundEvent`s: bleeps (#1/#2) → tones; samples (#>=3)
/// → Blorb resource playback tracked by number, remembering finish routines.
fn play_cli_sounds(cs: &mut CliSound, events: &[zvm::cpu::exec::SoundEvent]) {
    for ev in events {
        match ev.number {
            0 => {}
            1 | 2 => {
                if ev.effect == 0 || ev.effect == 2 {
                    let freq = if ev.number == 1 { 800.0 } else { 400.0 };
                    cs.backend.play_tone(freq, 150, ev.volume);
                }
            }
            n => match ev.effect {
                3 => { if let Some(id) = cs.ids.remove(&n) { cs.backend.stop(id); } }
                1 => {}
                _ => {
                    if let Some(blorb) = &cs.blorb {
                        if let Some((bytes, kind)) = blorb.sound(n as u32) {
                            if let Some(fmt) = sound_kind_to_format(kind) {
                                if let Some(id) = cs.backend.play_sample(bytes, fmt, ev.volume, ev.repeats) {
                                    cs.ids.insert(n, id);
                                    if ev.routine != 0 { cs.routines.insert(id, ev.routine); }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

/// Poll finished sampled sounds; run their finish-routines and reprint the frame.
fn poll_sound_finish(sound: Option<&mut CliSound>, machine: &mut Machine, view: &mut screen::ScreenView, is_tty: bool) {
    let Some(cs) = sound else { return };
    let done = cs.backend.finished();
    let mut ran = false;
    for id in done {
        // Always forget the number->id mapping for a finished sound, even one
        // with no finish routine.
        cs.ids.retain(|_, v| *v != id);
        if let Some(routine) = cs.routines.remove(&id) {
            if routine != 0 {
                machine.run_routine(routine);
                ran = true;
            }
        }
    }
    // A finish routine may itself start sounds (into machine.pending_sounds);
    // play them now rather than deferring to the next main-loop step().
    if !machine.pending_sounds.is_empty() {
        let events: Vec<zvm::cpu::exec::SoundEvent> = machine.pending_sounds.drain(..).collect();
        play_cli_sounds(cs, &events);
    }
    if ran && is_tty {
        print!("{}", view.frame(machine));
        release_prompt(machine);
        let _ = io::stdout().flush();
    }
}

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
    /// Line-position tracking, and — in plain mode — holding the prompt back so
    /// the status block can be written before it. Shared with gvm-cli, which had
    /// grown its own half of the same answer (`cli_host::LineHold`, SQ-0611).
    /// Outside plain mode it holds nothing: the status is painted in a pinned
    /// region and never enters the text flow at all.
    hold: cli_host::LineHold,
}

impl StdoutOutput {
    fn new(
        is_tty: bool,
        paging: bool,
        page_height: u16,
        cols: u16,
        honor_game_colours: bool,
        hold_partial: bool,
    ) -> Self {
        StdoutOutput {
            is_tty,
            paging,
            page_height,
            cols,
            current_col: 0,
            lines: 0,
            buffer_mode: false,
            honor_game_colours,
            hold: cli_host::LineHold::new(hold_partial),
        }
    }

    /// Write one character, holding it back when it is an unterminated tail and
    /// this sink is holding. A complete line always goes straight out.
    fn emit_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        print!("{}", self.hold.feed(ch.encode_utf8(&mut buf)));
    }

    /// Release the held prompt.
    ///
    /// Must be called before anything else writes to stdout, and before
    /// blocking or exiting — otherwise the prompt arrives after whatever
    /// overtook it, or never.
    fn release_partial(&mut self) {
        let held = self.hold.release();
        if !held.is_empty() {
            print!("{held}");
            let _ = io::stdout().flush();
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
            self.emit_char(ch);
            if ch == '\n' {
                // Saturating: `lines` is only ever reset by `check_paging`, an
                // `erase_window`, or an input request, so a story that prints
                // without pausing — piped output, `--no-more`, or a game in a
                // print loop — climbs forever and used to panic on overflow at
                // exactly 65_536 newlines (SQ-0601). Nothing reads the count
                // once it is past `page_height`, so pinning it at the ceiling
                // is behaviour-neutral.
                self.lines = self.lines.saturating_add(1);
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

/// Let go of the prompt the sink is holding back (plain mode — SQ-0611).
///
/// Pair it with every status emission (status first, then this) and call it
/// before anything else writes to stdout, blocks, or exits.
fn release_prompt(machine: &mut Machine) {
    if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
        o.release_partial();
    }
}

/// Answer a host command (`/status`) mid-turn without wrecking the transcript.
///
/// The prompt has already gone out by now, so the answer would otherwise land on
/// the prompt line — the very thing SQ-0611 fixed one layer down. Start a fresh
/// line, print the answer, then put the prompt back so the player can see it is
/// still their turn.
fn print_host_answer(machine: &mut Machine, text: &str) {
    let (at_line_start, prompt) = match machine.out.as_any().downcast_ref::<StdoutOutput>() {
        Some(o) => (o.hold.at_line_start(), o.hold.last_prompt().to_string()),
        None => (true, String::new()),
    };
    if !at_line_start {
        println!();
    }
    println!("{text}");
    print!("{prompt}");
    let _ = io::stdout().flush();
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
        Ok((blorb::ExecKind::Scott, _)) => {
            Err("Error: this is a Scott Adams Blorb; run it with babelmap.".to_string())
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
    term_rows: u16,
    term_cols: u16,
    honor_game_colours: bool,
    interpreter_number: Option<u8>,
    // Plain mode: hold the game's prompt back so the status can precede it.
    hold_prompt: bool,
) -> Result<Machine, String> {
    use zvm::error::ZError;
    let mem = Memory::new(story).map_err(|e| match e {
        ZError::UnsupportedVersion(v) => format!("Error: Z-machine version {v} is not supported."),
        ZError::NotAStoryFile => "Error: file is not a valid Z-machine story file.".to_string(),
        ZError::Truncated => "Error: story file is truncated.".to_string(),
        _ => format!("Error loading story: {e:?}"),
    })?;
    // v6 is a graphical, mouse-and-menu format: its games drive a windowed
    // display this front-end has no way to present, and none of them can be
    // driven through a plain character stream. Measured (SQ-0601): every v6
    // story we have — Zork Zero, Shogun, Arthur, Journey, advent.z6 — runs away
    // the moment its opening screen asks for input, whatever key it is given.
    // Zork Zero and Arthur flood the terminal with newlines; Shogun spins
    // silently with no output at all and no prompt to interrupt.
    //
    // So refuse here rather than hang. The refusal is the FRONT-END's, not the
    // library's: `zvm` supports v6 fully and babelmap plays these games — run
    // them there.
    if mem.version() == 6 {
        return Err(
            "Error: Z-machine v6 graphical games are not supported by zvm-cli.\n\
             Run it with babelmap, which renders v6 graphics and menus."
                .to_string(),
        );
    }
    let mut machine = Machine::with_output(mem, Box::new(StdoutOutput::new(
        stdout_is_tty,
        paging,
        page_height,
        term_cols,
        honor_game_colours,
        hold_prompt,
    )));
    machine.set_interpreter_number(interpreter_number);
    machine.init_caps();
    // Report the real terminal size to the game. init_caps seeds a generous
    // 80×24 default; without this override the game centres and wraps against
    // 80 columns regardless of the actual pane width (e.g. a title page stays
    // centred for 80 in a 50-column terminal). Kept in sync on resize.
    zvm::screen::write_screen_dims(
        &mut machine.mem,
        term_rows.min(255) as u8,
        term_cols.min(255) as u8,
    );
    Ok(machine)
}

// ── argument parsing ──────────────────────────────────────────────────────────

struct Args {
    story: Option<String>,
    story_only: bool,
    show_status: bool,
    no_aux: bool,
    no_more: bool,
    no_timed_input: bool,
    no_sound: bool,
    data_dir: Option<String>,
}

fn parse_args(argv: &[String]) -> Args {
    let mut a = Args { story: None, story_only: false, show_status: false, no_aux: false, no_more: false, no_timed_input: false, no_sound: false, data_dir: None };
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--story-only" | "--lower-only" => a.story_only = true,
            // Renamed in SQ-0613: `--no-status` read as the same thing plain
            // mode does to the status line, and it is stronger than that. Say
            // so rather than ignoring it, which would look like it worked.
            "--no-status" => {
                eprintln!(
                    "{}: --no-status has been renamed --story-only (it suppresses the whole \
                     upper window, menus included); honouring it this time.",
                    env!("CARGO_PKG_NAME")
                );
                a.story_only = true;
            }
            "--show-status" => a.show_status = true,
            "--no-aux" => a.no_aux = true,
            "--no-more" | "--no-page" => a.no_more = true,
            "--no-timed-input" => a.no_timed_input = true,
            "--no-sound" => a.no_sound = true,
            "--volume" => i += 1, // also skip the following value token
            // NB: --no-game-colours is handled separately by parse_game_colours;
            // it falls through here (a `--flag` is never taken as the story arg).
            "-I" | "--interpreter" => i += 1, // also skip the following value token
            "--data-dir" => { i += 1; if i < argv.len() { a.data_dir = Some(argv[i].clone()); } }
            s if !s.starts_with("--") && a.story.is_none() => a.story = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }
    a
}

/// Per-game directory name: the story file's basename (incl. extension),
/// sanitized to a filesystem-safe token. Empty -> "game".
fn story_key(story_path: &std::path::Path) -> String {
    let name = story_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let s: String = name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() { "game".to_string() } else { s }
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

/// Returns `true` (honour game colours) unless `--no-game-colours` is present.
/// `NO_COLOR` is folded in at the call site, so this stays a pure function of
/// the command line and its test stays independent of the environment.
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

/// Read the master volume from `--volume N` (0..=100). None when absent/invalid.
fn parse_volume(args: &[String]) -> Option<u8> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--volume" {
            return it.next().and_then(|v| v.parse::<u8>().ok()).map(|v| v.min(100));
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
        if let Ok(Event::Key(_)) = event::read() {
            break;
        }
    }
    let _ = terminal::disable_raw_mode();
}

/// Read one keypress in raw mode. Resize events that arrive before the key are
/// captured and returned alongside the key so the caller can update its state.
/// Piped stdin: reads the first byte of the next line.
///
/// When `timeout` is `Some((time, _))` (a timed `read_char` and timed input is
/// honored), each wait is bounded to `time * 100` ms; on expiry the pending
/// interrupt routine runs via `machine.run_timed_interrupt()`. If it aborts the
/// read, this returns with `aborted = true` (caller completes via
/// `abort_timed_input`); otherwise the routine's output is redrawn via `view`
/// and the wait resumes. `timeout = None` keeps today's exact blocking read.
fn read_char_input(
    is_tty: bool,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
    timeout: Option<(u16, u16)>,
    sound: &mut Option<CliSound>,
) -> (u8, Option<(u16, u16)>, bool) {
    if !is_tty {
        return (read_byte_stdin(), None, false);
    }
    let _ = terminal::enable_raw_mode();
    let mut last_resize: Option<(u16, u16)> = None;
    let result = loop {
        if timeout.is_none() && sound.as_ref().is_some_and(|cs| !cs.routines.is_empty())
            && !event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                continue;
            }
        if let Some((t, _)) = timeout {
            if !event::poll(std::time::Duration::from_millis(t as u64 * 100)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                let out = machine.run_timed_interrupt();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                if out.aborted {
                    break (0u8, last_resize, true);
                }
                print!("{}", view.frame(machine));
                release_prompt(machine);
                let _ = io::stdout().flush();
                continue;
            }
        }
        match event::read() {
            Ok(Event::Key(KeyEvent { code, .. })) => break (decode_keycode(code), last_resize, false),
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            _ => {}
        }
    };
    let _ = terminal::disable_raw_mode();
    result
}

// ── aux ("global state") persistence ──────────────────────────────────────────

/// Load the IFID-keyed aux file into the machine's aux_data (preload); warn on decode error.
fn aux_preload(machine: &mut Machine, aux_file: &Path, no_aux: bool) {
    if no_aux {
        return;
    }
    if let Ok(bytes) = fs::read(aux_file) {
        match auxiliary::decode_aux(&bytes) {
            Ok(map) => {
                machine.aux_data = map;
                machine.aux_dirty = false;
            }
            Err(e) => eprintln!("zvm: warning: ignoring corrupt {}: {:?}", aux_file.display(), e),
        }
    }
}

/// Flush aux_data to the per-game aux file when dirty; clear the flag regardless.
fn aux_flush(machine: &mut Machine, aux_file: &Path, no_aux: bool) {
    if no_aux || !machine.aux_dirty {
        return;
    }
    if let Some(dir) = aux_file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(aux_file, auxiliary::encode_aux(&machine.aux_data)) {
        eprintln!("zvm: warning: aux save to {} failed: {}", aux_file.display(), e);
    }
    machine.aux_dirty = false;
}

// ── prompt + read helpers ─────────────────────────────────────────────────────

fn prompt_and_read_line(prompt: &str) -> String {
    print!("{}", prompt);
    let _ = io::stdout().flush();
    // EOF here yields an empty filename, which the caller already treats as
    // "cancel" — unlike a *game's* input request, an unanswered save prompt is
    // recoverable, so this one does not exit.
    cli_host::read_line_stdin().unwrap_or_default()
}

/// One line of piped input, or a clean exit at true EOF.
///
/// This path had the SQ-0604 bug all along, one path over from where it was
/// fixed: a 0-byte read left `line` empty and the game was handed a blank
/// command forever. Measured before the fix, `zvm-cli gostak.z5 < /dev/null`
/// printed 90 KB of `>Louk?` and had to be killed. The char path next door
/// (`read_byte_stdin`) had checked for EOF for months.
fn read_line_stdin() -> String {
    match cli_host::read_line_stdin() {
        Some(line) => line,
        None => cli_host::input::exit_at_eof(&crate::screen::leave_region()),
    }
}

/// Read a line of input in RAW mode, echoing as the user types. Unlike cooked
/// `read_line_stdin`, arrow/function-key escape sequences are parsed by
/// crossterm into key events (so they never leak onto the screen as garbage),
/// and the echo is drawn in the game's current style/colour (`echo`). Resize
/// events during the read are captured and returned so the caller can update
/// its layout. Falls back to cooked line input when stdin is not a TTY (piped).
///
/// When `timeout` is `Some((time, _))` (a timed `read` and timed input is
/// honored), each wait is bounded to `time * 100` ms; on expiry the pending
/// interrupt routine runs via `machine.run_timed_interrupt()`. If it aborts the
/// read, this returns with `aborted = true` (caller completes via
/// `abort_timed_input`, buffer preserved in `line`); otherwise the routine's
/// output is redrawn via `view` and the line-edit resumes. `timeout = None`
/// keeps today's exact blocking read.
fn read_line_raw(
    is_tty: bool,
    echo: zvm::io::TextAttrs,
    machine: &mut Machine,
    view: &mut screen::ScreenView,
    timeout: Option<(u16, u16)>,
    sound: &mut Option<CliSound>,
) -> (String, u8, Option<(u16, u16)>, bool) {
    if !is_tty {
        return (read_line_stdin(), 13, None, false);
    }
    let _ = terminal::enable_raw_mode();
    let mut buf = String::new();
    let mut terminator: u8 = 13; // Enter unless a function-key terminator ends the line
    let mut last_resize: Option<(u16, u16)> = None;
    let mut aborted = false;
    let sgr = crate::screen::sgr_open(echo);
    if !sgr.is_empty() {
        print!("{sgr}");
        let _ = io::stdout().flush();
    }
    loop {
        if timeout.is_none() && sound.as_ref().is_some_and(|cs| !cs.routines.is_empty())
            && !event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                continue;
            }
        if let Some((t, _)) = timeout {
            if !event::poll(std::time::Duration::from_millis(t as u64 * 100)).unwrap_or(false) {
                let _ = terminal::disable_raw_mode();
                let out = machine.run_timed_interrupt();
                poll_sound_finish(sound.as_mut(), machine, view, is_tty);
                let _ = terminal::enable_raw_mode();
                if out.aborted {
                    aborted = true;
                    break;
                }
                print!("{}", view.frame(machine));
                release_prompt(machine);
                let _ = io::stdout().flush();
                continue;
            }
        }
        match event::read() {
            Ok(Event::Key(KeyEvent { code, modifiers, .. })) => match code {
                KeyCode::Enter => break,
                // Ctrl-C / Ctrl-D: raw mode swallows signals, so exit cleanly
                // ourselves (drop the colour, leave raw mode + the scroll region).
                KeyCode::Char('c') | KeyCode::Char('d')
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if !sgr.is_empty() { print!("\x1b[0m"); }
                    print!("\r\n"); // close the echoed line before the terminal goes back
                    let _ = io::stdout().flush();
                    // The scroll region is this renderer's own teardown, so it
                    // rides along as the prefix; everything after it is the
                    // shared restore.
                    cli_host::restore_and_exit(&crate::screen::leave_region(), 0);
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
                // A function key listed in the game's terminating-characters
                // table (header 0x2E) ends the line, reported to the game via the
                // stored terminator — e.g. the cursor keys BeyondZork uses to
                // scroll its boxed description (ZMSD §10.7). Any other special key
                // is consumed (no on-screen garbage), as before.
                _ => {
                    let z = decode_keycode(code) as u16;
                    if machine.is_terminator(z) {
                        terminator = z as u8;
                        break;
                    }
                }
            },
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            _ => {}
        }
    }
    if !sgr.is_empty() {
        print!("\x1b[0m");
    }
    let _ = terminal::disable_raw_mode();
    if terminator == 13 {
        // Raw mode does not translate Enter to CRLF. Skip for a function-key
        // terminator so the prompt doesn't drift as the game redraws in place.
        print!("\r\n");
    }
    let _ = io::stdout().flush();
    (buf, terminator, last_resize, aborted)
}

/// One byte of piped input, or a clean exit at true EOF.
///
/// The EOF rule (a 0-byte read is the end; a blank line is still a real `\n`)
/// lives in `cli_host::input` — see SQ-0604/0605 for why it is worth having in
/// exactly one place. What this path used to get wrong was the *exit*: it called
/// `process::exit(0)` directly, leaving the terminal wearing the game's page
/// background and a block cursor whenever stdin was a pipe but stdout was not
/// (`echo commands | zvm-cli story.z5`). `exit_at_eof` restores first.
fn read_byte_stdin() -> u8 {
    match cli_host::read_byte_stdin() {
        Some(b) => b,
        None => cli_host::input::exit_at_eof(&crate::screen::leave_region()),
    }
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
    // Keep the game's view of the screen size current so it re-centres and
    // re-wraps against the new pane width on its next redraw.
    zvm::screen::write_screen_dims(
        &mut machine.mem,
        new_rows.min(255) as u8,
        new_cols.min(255) as u8,
    );
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

const HELP: &str = "\
zvm-cli — DOS-style Z-machine player (no map)

Usage: zvm-cli [OPTIONS] <story-file>

Arguments:
  <story-file>          Z-code story (.z3/.z5/.z8 …, or a .zblorb container)
                        Graphical v6 stories are not supported — play those
                        with babelmap.

Host commands (typed at any line prompt, never passed to the game):
  /status               Repeat the current status line / upper window

Options:
      --plain           Linear plain text, for screen readers (alias:
                        --screen-reader; also selected by TERM=dumb). Emits no
                        escape sequences at all — no colour, no cursor
                        addressing, no pinned status line — hands line editing
                        and echo back to the terminal, and turns off the [MORE]
                        pager. The status line is not narrated every turn (see
                        --show-status); menus and forms still are. Ask for the
                        status any time with /status.
      --story-only      Show only the story text: suppress the whole upper
                        window, menus and forms included. Stronger than what
                        --plain does to the status line, and independent of it.
                        (alias: --lower-only)
      --show-status     Narrate the status line whenever the story updates it,
                        undoing --plain's quietening. Off under --plain because
                        a v3 status line carries a move counter, so it changes —
                        and would be re-read — on every single turn.
      --no-aux          Don't read or write v5 auxiliary (VFS) sidecar files
      --no-more         Disable [MORE] paging on long output (alias: --no-page)
      --no-timed-input  Ignore timed-input interrupts
      --no-game-colours Ignore the game's set_colour / true-colour output
                        (also honoured: NO_COLOR)
      --no-sound        Disable sound (bleeps + sampled audio)
      --volume <0-100>  Set the master volume
  -I, --interpreter <n> Set the Z-machine interpreter number
      --data-dir <path> Base dir for saves/sidecars (default: beside the story)
  -V, --version         Print version and exit
  -h, --help            Print this help and exit
";

fn main() {
    let argv: Vec<String> = env::args().collect();
    if cli_host::handled_common_flags(&argv, HELP, env!("CARGO_PKG_NAME"), buildinfo::LONG) {
        return;
    }
    let args = parse_args(&argv);
    let Some(story_arg) = args.story.clone() else {
        eprintln!("{}: no story file given. Run `zvm-cli --help` for usage.", env!("CARGO_PKG_NAME"));
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

    // Per-game directory: base (--data-dir override, else the story's own
    // directory) joined with the sanitized story filename.
    let base = args.data_dir.as_deref().map(std::path::PathBuf::from)
        .unwrap_or_else(|| story_path.parent().filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from(".")));
    // `.save` suffix keeps the dir from colliding with the story file itself
    // when `base` is the story's own directory (SQ-0294).
    let game_dir = base.join(format!("{}.save", story_key(&story_path)));
    let aux_file = auxiliary::aux_path(&game_dir);

    // One decision about what this terminal can take, published process-wide so
    // the exit paths buried in the read helpers can reach it (SQ-0605).
    // `--plain`/`--screen-reader`, or TERM=dumb, turns off both escape output
    // and raw-mode line editing (SQ-0606).
    let mode = HostMode::detect_with(cli_host::plain_requested(&argv)).install();
    let stdout_is_tty = mode.rich();
    let stdin_is_tty = mode.raw_input();
    let both_tty = mode.both_tty();

    // Restores the terminal on every way out of `main`, including a panic. The
    // explicit `restore` calls below hand it the scroll-region teardown; this
    // just guarantees it happens at all.
    let mut guard = TerminalGuard::new();

    // Force a steady block cursor (SQ-0281); reset to the terminal default on exit.
    if stdout_is_tty {
        print!("{}", cli_host::cursor_steady_block());
        let _ = io::stdout().flush();
    }

    // Paging is only safe when BOTH ends are TTYs (else it would block the
    // headless harness); --no-more disables it.
    let (mut term_rows, mut term_cols) = detect_term_size();
    // Plain mode also drops the pager: a [MORE] prompt is a blocking modal that
    // hides the rest of the output behind a keypress, which is exactly the shape
    // a screen reader cannot cope with (SQ-0606).
    let paging = both_tty && !args.no_more && !mode.plain();
    let mut page_height = term_rows.saturating_sub(2).max(2);
    // Timed reads (read/read_char time+routine) are honored unless disabled.
    let timed = !args.no_timed_input;
    let sound_enabled = !args.no_sound;
    let volume = parse_volume(&argv).unwrap_or(100);

    // `NO_COLOR` (no-color.org) means colour, not layout: it drops the game's
    // colours exactly as `--no-game-colours` does, and leaves the pinned status
    // line alone. Plain mode emits no escapes at all, so colour is moot there.
    let honor = parse_game_colours(&argv) && !cli_host::no_color();
    let interpreter = parse_interpreter(&argv);

    let mut machine = match build_machine(
        story_bytes,
        stdout_is_tty,
        paging,
        page_height,
        term_rows,
        term_cols,
        honor,
        interpreter,
        mode.plain(),
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    machine.set_honor_game_colours(honor);
    machine.set_sound_available(sound_enabled);
    aux_preload(&mut machine, &aux_file, args.no_aux);

    let mut sound: Option<CliSound> = if sound_enabled {
        let blorb = match blorb::resolve_resource_blorb(&story_path) {
            Some((b, path)) => {
                eprintln!("zvm: loaded sound resources from {}", path.display());
                Some(b)
            }
            None => None,
        };
        Some(CliSound {
            backend: audio::AudioBackend::new(volume),
            blorb,
            ids: HashMap::new(),
            routines: HashMap::new(),
        })
    } else {
        None
    };

    // Plain mode does not narrate the status line every turn unless asked
    // (SQ-0612). `--no-status` still wins outright: it is the stronger,
    // already-documented switch and suppresses the upper window entirely.
    let quiet_status_line = mode.plain() && !args.show_status;
    let mut view =
        screen::ScreenView::new(stdout_is_tty, args.story_only, quiet_status_line, term_rows);
    print!("{}", view.start());
    let _ = io::stdout().flush();

    // Page background: reflects the game's current bg onto the terminal's own
    // default background (OSC 11) so the whole window is painted, not just
    // coloured text runs. Honor- and stdout-TTY-gated; reset (OSC 111) on
    // every exit path below.
    let mut last_page_bg: Option<(u8, u8, u8)> = None;

    loop {
        let step = machine.step();
        for d in machine.diagnostics.drain(..) {
            eprintln!("zvm: warning: {d}");
        }
        // Bleeps + sampled sounds: drain the turn's sound events. Ring the bell for
        // #1/#2 (TTY only), and play audio when enabled.
        if !machine.pending_sounds.is_empty() {
            let events: Vec<zvm::cpu::exec::SoundEvent> = machine.pending_sounds.drain(..).collect();
            let beeps = events.iter().filter(|e| e.number == 1 || e.number == 2).count();
            if beeps > 0 {
                // The device fact, not `rich`: a bell is not an escape sequence
                // and a game ringing it is saying something, so plain mode keeps
                // it. Identical to `rich` in every other case.
                print!("{}", screen::bleep_bytes(beeps, mode.stdout_tty()));
                let _ = io::stdout().flush();
            }
            if let Some(cs) = sound.as_mut() {
                play_cli_sounds(cs, &events);
            }
        }
        // v3 show_status redraw request.
        if machine.screen.show_status_requested {
            print!("{}", view.frame(&machine));
            release_prompt(&mut machine);
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
                // `view.leave()` is the renderer's own teardown (drop the scroll
                // region, park the cursor below it); the guard does the rest.
                guard.restore(&view.leave());
                break;
            }

            StepResult::Fault => {
                guard.restore(&view.leave());
                if let Some(trace) = machine.take_fault_trace() {
                    for line in trace.to_lines() {
                        eprintln!("{line}");
                    }
                }
                std::process::exit(70); // EX_SOFTWARE: internal software error
            }

            StepResult::Restart => {
                machine = match build_machine(
                    original_bytes.clone(),
                    stdout_is_tty,
                    paging,
                    page_height,
                    term_rows,
                    term_cols,
                    honor,
                    interpreter,
                    mode.plain(),
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                machine.set_honor_game_colours(honor);
                machine.set_sound_available(sound_enabled);
                aux_preload(&mut machine, &aux_file, args.no_aux);
            }

            StepResult::NeedLine { .. } => {
                // Poll for terminal resize before line input (crossterm returns
                // current size; on piped stdout this is a no-op via is_tty guard).
                maybe_resize(both_tty, &mut term_rows, &mut term_cols, &mut page_height, &mut machine, &mut view);
                print!("{}", view.frame(&machine));
                release_prompt(&mut machine);
                let _ = io::stdout().flush();
                let cur_bg = if stdout_is_tty && machine.honor_game_colours {
                    screen::zcolour_rgb(machine.screen.current_bg)
                } else {
                    None
                };
                if let Some(esc) = cli_host::page_bg_escape(cur_bg, last_page_bg) {
                    print!("{esc}");
                    let _ = io::stdout().flush();
                    last_page_bg = cur_bg;
                }
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
                let timeout = if timed { machine.pending_timeout() } else { None };
                // `/status` is answered by the host and the game never sees it,
                // so loop until a real command arrives (SQ-0610). A timed read
                // that expired is NOT re-read here: the interrupt has already
                // run and the game is owed its answer.
                let (line, terminator, resize, aborted) = loop {
                    let r = read_line_raw(stdin_is_tty, echo, &mut machine, &mut view, timeout, &mut sound);
                    if r.3 || !cli_host::input::is_status_request(&r.0) {
                        break r;
                    }
                    let status = screen::ScreenView::status_now(&machine);
                    let text = if status.is_empty() { "[no status]".to_string() } else { status };
                    print_host_answer(&mut machine, &text);
                };
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                if aborted {
                    machine.abort_timed_input(line.trim_end());
                } else {
                    machine.supply_line(line.trim_end(), terminator);
                }
                if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                    o.lines = 0;
                    o.current_col = 0; // cursor is at line start after user input + Enter
                }
            }

            StepResult::NeedChar => {
                // Poll for terminal resize before char input.
                maybe_resize(both_tty, &mut term_rows, &mut term_cols, &mut page_height, &mut machine, &mut view);
                print!("{}", view.frame(&machine));
                release_prompt(&mut machine);
                let _ = io::stdout().flush();
                let cur_bg = if stdout_is_tty && machine.honor_game_colours {
                    screen::zcolour_rgb(machine.screen.current_bg)
                } else {
                    None
                };
                if let Some(esc) = cli_host::page_bg_escape(cur_bg, last_page_bg) {
                    print!("{esc}");
                    let _ = io::stdout().flush();
                    last_page_bg = cur_bg;
                }
                let timeout = if timed { machine.pending_timeout() } else { None };
                let (ch, resize, aborted) = read_char_input(stdin_is_tty, &mut machine, &mut view, timeout, &mut sound);
                // Handle any resize that happened DURING the char read.
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                if aborted {
                    machine.abort_timed_input("");
                } else {
                    machine.supply_char(ch);
                }
                if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                    o.lines = 0;
                    o.current_col = 0;
                }
            }

            StepResult::SaveRequest => {
                let filename = prompt_and_read_line("\nSave to file: ");
                let filename = filename.trim();
                let path = resolve_save_input(filename, &game_dir);
                if let Some(dir) = path.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let save_data = machine.save_quetzal();
                match fs::write(&path, &save_data) {
                    Ok(()) => {
                        println!("Saved to '{}'.", path.display());
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
                let path = resolve_save_input(filename, &game_dir);
                match fs::read(&path) {
                    Ok(data) => match machine.complete_restore_success(&data) {
                        Ok(()) => {} // restored; @save descriptor completed forward
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
mod v6_tests {
    use super::*;

    /// A story file with just enough header to load: `version` in byte 0 and a
    /// plausible layout. Not runnable — `build_machine` only has to get as far
    /// as reading the version.
    fn story_of_version(v: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = v;
        buf[0x04] = 0x04; // high_mem_base = 0x0400
        buf[0x05] = 0x00;
        buf[0x06] = 0x00; // initial_pc = 0x0040
        buf[0x07] = 0x40;
        buf[0x0C] = 0x01; // object table
        buf[0x0D] = 0x00;
        buf[0x0E] = 0x02; // globals
        buf[0x0F] = 0x00;
        buf[0x18] = 0x00; // abbrev table
        buf[0x19] = 0x60;
        buf
    }

    fn build(story: Vec<u8>) -> Result<Machine, String> {
        build_machine(story, false, false, 24, 24, 80, true, None, false)
    }

    /// SQ-0601: v6 is a graphical, mouse-and-menu format this front-end cannot
    /// present, and every v6 story we have runs away the moment its opening
    /// screen asks for input — whatever key it is given. Zork Zero and Arthur
    /// flood the terminal; Shogun spins silently with nothing to interrupt.
    /// Refusing at load is the only outcome that neither crashes nor hangs.
    #[test]
    fn a_v6_story_is_refused_rather_than_run() {
        let err = match build(story_of_version(6)) {
            Err(e) => e,
            Ok(_) => panic!("v6 must be refused, not loaded"),
        };
        assert!(err.contains("not supported by zvm-cli"), "{err}");
        assert!(err.contains("babelmap"), "the message points at the front-end that can: {err}");
    }

    /// The refusal is zvm-cli's alone — the library still supports v6, which is
    /// what babelmap plays. A version check that lived in `Memory::new` would
    /// take the TUI's v6 support down with it.
    #[test]
    fn the_library_still_loads_a_v6_story() {
        let mem = Memory::new(story_of_version(6)).expect("zvm itself accepts v6");
        assert_eq!(mem.version(), 6);
    }

    #[test]
    fn the_ordinary_versions_still_load() {
        for v in [3u8, 5, 8] {
            assert!(build(story_of_version(v)).is_ok(), "v{v} must still load");
        }
    }
}

#[cfg(test)]
mod paging_tests {
    use super::*;

    /// SQ-0601: `lines` only resets on a page break, an `erase_window`, or an
    /// input request, so a story printing without pause climbs without bound —
    /// piped output and `--no-more` never page at all. It used to be a `u16`
    /// incremented with `+=`, which panicked with "attempt to add with
    /// overflow" at exactly 65_536 newlines. Zork Zero reached that in under
    /// twenty seconds.
    #[test]
    fn the_line_counter_saturates_instead_of_overflowing() {
        let mut o = StdoutOutput::new(false, false, 24, 80, true, false);
        o.lines = u16::MAX - 2;
        for _ in 0..10 {
            o.write_counted("\n");
        }
        assert_eq!(o.lines, u16::MAX, "the counter pins at its ceiling and never wraps");
    }

    /// SQ-0611. The sink writes to the real stdout, so what is testable here is
    /// the holding decision itself: with holding on, an unterminated prompt must
    /// still be pending after the write, leaving the stream at a line start for
    /// the status block to occupy — and gone once released.
    #[test]
    fn plain_mode_holds_the_prompt_so_the_status_can_precede_it() {
        let mut o = StdoutOutput::new(false, false, 24, 80, true, true);
        o.write_counted("You are in a room.\n");
        assert!(!o.hold.is_holding(), "a complete line goes straight out");
        o.write_counted("\n>");
        assert!(o.hold.is_holding(), "the prompt is held back");
        assert!(o.hold.at_line_start(), "so the status starts its own line");
        o.release_partial();
        assert!(!o.hold.is_holding(), "and the prompt follows the status");
    }

    #[test]
    fn without_plain_mode_nothing_is_ever_held() {
        // The pinned-region path must keep writing straight through: its status
        // never enters the text flow, so there is nothing to make room for.
        let mut o = StdoutOutput::new(true, false, 24, 80, true, false);
        o.write_counted("\n>");
        assert!(!o.hold.is_holding(), "the TTY path holds nothing");
    }
}

#[cfg(test)]
mod arg_tests {
    use super::*;

    #[test]
    fn parses_flags_and_story() {
        let a = parse_args(&["zvm-cli".into(), "--story-only".into(), "game.z5".into()]);
        assert_eq!(a.story.as_deref(), Some("game.z5"));
        assert!(a.story_only && !a.no_aux);

        let b = parse_args(&["zvm-cli".into(), "--no-aux".into(), "g".into()]);
        assert!(b.no_aux && !b.story_only);

        let c = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!c.story_only && !c.no_aux);
    }

    /// SQ-0613. `--no-status` read as the same thing plain mode does to the
    /// status line while being stronger than it, so it was renamed. The old
    /// spelling still works — silently ignoring it would look like it had.
    #[test]
    fn the_old_and_new_spellings_all_select_story_only() {
        for flag in ["--story-only", "--lower-only", "--no-status"] {
            let a = parse_args(&["zvm-cli".into(), flag.into(), "g".into()]);
            assert!(a.story_only, "{flag} should select story-only");
            assert_eq!(a.story.as_deref(), Some("g"), "{flag} is not the story path");
        }
    }

    #[test]
    fn parses_no_more_flag() {
        let a = parse_args(&["zvm-cli".into(), "--no-more".into(), "g".into()]);
        assert!(a.no_more);
        let b = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!b.no_more);
    }

    #[test]
    fn parses_no_timed_input_flag() {
        let a = parse_args(&["zvm-cli".into(), "--no-timed-input".into(), "g".into()]);
        assert!(a.no_timed_input);
        let b = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!b.no_timed_input);
    }

    #[test]
    fn parses_no_sound_flag() {
        let a = parse_args(&["zvm-cli".into(), "--no-sound".into(), "g".into()]);
        assert!(a.no_sound);
        let b = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!b.no_sound);
    }

    #[test]
    fn parse_volume_reads_flag() {
        assert_eq!(parse_volume(&["--volume".into(), "60".into(), "g".into()]), Some(60));
        assert_eq!(parse_volume(&["g".into()]), None);
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
        let tmp = std::env::temp_dir().join(format!("zvm-cli-storage-collision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let story_path = tmp.join("game.z5");
        std::fs::write(&story_path, b"x").unwrap(); // a FILE named game.z5

        let game_dir = tmp.join(format!("{}.save", story_key(&story_path)));
        assert_eq!(game_dir, tmp.join("game.z5.save"));
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
}

#[cfg(test)]
mod stdin_eof_tests {
    // The implementation moved to `cli_host::input` (SQ-0605); zvm-cli keeps its
    // own regression test, because this is one of the two crates the bug shipped
    // in — and it shipped here twice, the char path fixed months before the line
    // path (`zvm-cli gostak.z5 < /dev/null` printed 90 KB of prompts until it
    // was killed).
    use cli_host::read_byte_or_eof;

    #[test]
    fn true_eof_returns_none_instead_of_looping() {
        // A closed/empty stdin yields a 0-byte read_line, which must be
        // reported as EOF (None) rather than a synthesized b'\n' — the bug
        // that caused read_byte_stdin() to busy-spin forever on piped input.
        let mut empty: &[u8] = b"";
        assert_eq!(read_byte_or_eof(&mut empty), None);
    }

    #[test]
    fn blank_line_is_not_confused_with_eof() {
        let mut input: &[u8] = b"\n";
        assert_eq!(read_byte_or_eof(&mut input), Some(b'\n'));
    }

    #[test]
    fn returns_first_byte_of_line() {
        let mut input: &[u8] = b"abc\n";
        assert_eq!(read_byte_or_eof(&mut input), Some(b'a'));
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

#[cfg(test)]
mod sound_idmap_tests {
    use std::collections::HashMap;

    // Mirrors the `cs.ids.retain(|_, v| *v != id)` line in `poll_sound_finish`:
    // a finished sound's number->id mapping must be cleared even when it has no
    // finish routine (Bug A — previously this only ran inside the
    // `if let Some(routine) = cs.routines.remove(&id)` branch, so a routine-less
    // sound left a stale entry). Exercises the exact retain predicate against a
    // plain `HashMap<u16, SoundId>`, device-free.
    #[test]
    fn retain_clears_finished_id_without_routine() {
        let id: audio::SoundId = 7;
        let mut ids: HashMap<u16, audio::SoundId> = HashMap::new();
        ids.insert(3, id); // sound #3 played with routine == 0 → no `routines` entry
        ids.insert(4, 99); // unrelated sound, still playing

        ids.retain(|_, v| *v != id);

        assert!(!ids.contains_key(&3), "finished id must be cleared even without a routine");
        assert_eq!(ids.get(&4), Some(&99), "unrelated entries must be untouched");
    }
}

#[cfg(test)]
mod restore_request_tests {
    use super::*;

    /// Minimal but structurally valid v4 story buffer. Mirrors zvm's own
    /// `header::tests_support::sample_story`, which is `pub(crate)` to the
    /// `zvm` crate and not visible from here.
    fn sample_v4_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x400];
        buf[0x00] = 4; // version
        buf[0x04] = 0x04;
        buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00;
        buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x02;
        buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01;
        buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03;
        buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04;
        buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00;
        buf[0x19] = 0x40; // abbrev_table = 0x0040
        buf
    }

    /// A v4 story: `save -> G0` at 0x40 (store form, one store byte), then quit.
    fn save_v4_into_g0_story() -> Vec<u8> {
        let mut buf = sample_v4_story();
        buf[0x40] = 0xB5; // 0OP:0x05 save (store form, v4+)
        buf[0x41] = 0x10; // store -> global 0 (var 0x10)
        buf[0x42] = 0xBA; // quit
        buf
    }

    /// SQ-0283 Task 7: the `RestoreRequest` arm must call
    /// `machine.complete_restore_success(&data)`, not the raw
    /// `machine.restore_quetzal(&data)`. The raw call would leave the
    /// machine resuming AT the `@save`'s result descriptor (the store byte
    /// itself, per Quetzal §5.8) with that descriptor never resolved;
    /// `complete_restore_success` completes it forward — storing 2 into the
    /// `@save`'s result and resuming PAST the descriptor — matching how the
    /// app completes an in-game restore.
    #[test]
    fn restore_request_completes_the_save_descriptor_forward() {
        let mem = Memory::new(save_v4_into_g0_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;

        let r = m.step();
        assert_eq!(r, StepResult::SaveRequest, "save opcode suspends with SaveRequest");
        assert_eq!(m.state.pc, 0x42, "PC is post-instruction after @save suspends");

        let blob = m.save_quetzal();
        m.complete_save(true);
        assert_eq!(m.global(0), 1, "save success stored 1 into G0");

        // Clobber state the way later play would, so the restore must actually reset it.
        m.do_store(Some(0x10), 0x99);
        m.state.pc = 0x00AB;

        // This is the exact call zvm-cli's RestoreRequest arm makes.
        m.complete_restore_success(&blob).expect("in-game restore must succeed");

        assert_eq!(m.global(0), 2, "descriptor advanced: the original @save 'returns' 2");
        assert_eq!(m.state.pc, 0x42, "execution resumes PAST the @save, not at its descriptor");

        // A properly-resumed machine must not immediately re-suspend.
        let r2 = m.step();
        assert_ne!(r2, StepResult::SaveRequest, "resumed machine does not re-suspend on save");
    }
}
