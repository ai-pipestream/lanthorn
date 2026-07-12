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

use crossterm::event::{self, Event, KeyCode, KeyEvent};
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

/// Drive `machine` to completion through the Glk step loop, pulling input from
/// the supplied readers. `before_input` runs just before each blocking read (the
/// CLI uses it to flush output so the prompt is visible). This is the shared,
/// backend-agnostic loop — the CLI wires real stdin/terminal readers; tests wire
/// scripted ones.
fn drive(
    machine: &mut Machine,
    save_path: &std::path::Path,
    vfs_path: &std::path::Path,
    mut before_input: impl FnMut(&mut Machine),
    mut read_line: impl FnMut() -> (String, u32),
    mut read_char: impl FnMut() -> u32,
) {
    loop {
        // Flush the Glk file VFS to its sidecar whenever a game mutation dirtied
        // it, each iteration, so a game's files survive even if the process is
        // killed mid-session. Silently tolerate a write failure.
        if machine.vfs_dirty() {
            let _ = fs::write(vfs_path, machine.vfs_bytes());
            machine.clear_vfs_dirty();
        }
        match machine.step() {
            StepResult::Continue => {}
            StepResult::Quit => break,
            StepResult::NeedLine { .. } => {
                before_input(machine);
                let (line, terminator) = read_line();
                machine.supply_line_terminated(line.trim_end_matches(['\n', '\r']), terminator);
            }
            StepResult::NeedChar { .. } => {
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
                    let (line, _) = read_line();
                    let name = line.trim_end_matches(['\n', '\r']);
                    machine.supply_filename(if name.is_empty() { None } else { Some(name.to_string()) });
                }
            }
            // Game @save: write the snapshot to a single default slot next to the
            // story. Headless, so there is no name prompt — one slot, overwritten.
            StepResult::SaveRequest => {
                let ok = fs::write(save_path, machine.save_state()).is_ok();
                if ok {
                    eprintln!("[saved to {}]", save_path.display());
                } else {
                    eprintln!("[save failed]");
                }
                machine.complete_save(ok);
            }
            // Game @restore: read that same default slot back, or fail cleanly.
            StepResult::RestoreRequest => match fs::read(save_path) {
                Ok(bytes) if machine.complete_restore_success(&bytes) => {
                    eprintln!("[restored from {}]", save_path.display());
                }
                _ => {
                    eprintln!("[restore failed]");
                    machine.complete_restore_failure();
                }
            },
        }
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
    let Some(path) = argv.iter().skip(1).find(|a| !a.starts_with("--")) else {
        eprintln!(
            "Usage: {} [--no-game-colours] [--no-accel] <story.ulx | story.gblorb>",
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

    // Track the last-known terminal size so we can detect resize events.
    // Re-poll before each input using crossterm::terminal::size(); when
    // changed, update the backend and queue a Glk evtype_Arrange event so
    // the game can re-lay out its windows.
    let mut last_size: Option<(u32, u32)> = if both_tty {
        terminal::size().ok().map(|(c, r)| (c as u32, r as u32))
    } else {
        None
    };

    // The single default in-game save slot: the story path with a `.glksave`
    // suffix (headless, so there is no name prompt — one slot, overwritten).
    let save_path = std::path::PathBuf::from(format!("{path}.glksave"));

    // The Glk file VFS sidecar: `<story>.glkvfs`, next to the story. Loaded here
    // before the run, flushed dirty-gated inside `drive`, so a game's external
    // files (scores, preferences) survive a plain quit-and-relaunch.
    let vfs_path = std::path::PathBuf::from(format!("{path}.glkvfs"));
    if vfs_path.exists() {
        // Tolerate a missing/unreadable sidecar silently — it just means empty.
        if let Ok(bytes) = fs::read(&vfs_path) {
            machine.load_vfs(&bytes);
        }
        machine.clear_vfs_dirty(); // loading is not a game mutation
    }

    drive(
        &mut machine,
        &save_path,
        &vfs_path,
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
        read_line_stdin,
        move || read_char_input(stdin_is_tty),
    );

    machine.flush();
    // Ensure raw mode is not left active on exit (harmless if already off).
    let _ = terminal::disable_raw_mode();

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
        drive(&mut m, std::path::Path::new("unused.glksave"), std::path::Path::new("unused.glkvfs"), |_| {}, || (String::new(), 0), move || keys.next().unwrap_or(keycode::RETURN));
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
        drive(&mut m, std::path::Path::new("unused.glksave"), std::path::Path::new("unused.glkvfs"), |_| {}, move || (lines.next().unwrap_or_default(), 0), || keycode::RETURN);
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
        let save_path = dir.join("story.glksave");
        let vfs_path = dir.join("story.glkvfs");

        // Run 1: a game writes "Hi" into a Glk file, then quits. drive's in-loop,
        // dirty-gated flush should persist the VFS to the sidecar.
        let mut m = build_machine(vfs_image(write_hi_program(), 2), Box::new(TestBackend::new())).unwrap();
        drive(&mut m, &save_path, &vfs_path, |_| {}, || (String::new(), 0), || keycode::RETURN);

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
        drive(&mut m2, &save_path, &vfs_path, |_| {}, || (String::new(), 0), || keycode::RETURN);
        let text = m2.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "Hi", "the persisted bytes are readable after load_vfs");

        let _ = fs::remove_dir_all(&dir);
    }
}
