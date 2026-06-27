// gvm-cli — an interactive Glulx player.
//
// Usage: gvm-cli <story.ulx | story.gblorb>
//
// Loads a raw `.ulx` Glulx image or extracts the `GLUL` executable from a Blorb,
// drives it through a Glk step loop, routing Glk output to a terminal backend
// and reading the player's input (cooked line / raw single key on a TTY), and
// prints any diagnostics to stderr. The original terminal mode is always
// restored on exit.

use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read};
use std::process;

use gvm::glk::keycode;
use gvm::{GlkBackend, Machine, Memory, StepResult};

mod glk_term;
use glk_term::TerminalBackend;

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

/// Build a machine from story bytes, sending Glk output to `backend`.
fn build_machine(bytes: Vec<u8>, backend: Box<dyn GlkBackend>) -> Result<Machine, String> {
    let image = extract_executable(bytes)?;
    let mem = Memory::new(image).map_err(|e| format!("Error loading Glulx image: {e:?}"))?;
    Ok(Machine::with_glk(mem, backend))
}

/// Decode a raw key (one byte, or an escape sequence) into a Glk char-input code:
/// printable bytes pass through as their Latin-1 value; Enter/Backspace/Tab/Esc
/// and the arrow / Home / End / Page keys map to their `keycode_*`; an
/// unrecognized escape sequence becomes `keycode_Unknown`. An empty slice (EOF)
/// is treated as Enter.
fn decode_key(bytes: &[u8]) -> u32 {
    match bytes.first().copied() {
        None => keycode::RETURN,
        Some(0x1b) if bytes.len() == 1 => keycode::ESCAPE,
        Some(0x1b) => {
            // A CSI / SS3 sequence: ESC [ X  or  ESC O X.
            match bytes.last().copied() {
                Some(b'A') => keycode::UP,
                Some(b'B') => keycode::DOWN,
                Some(b'C') => keycode::RIGHT,
                Some(b'D') => keycode::LEFT,
                Some(b'H') => keycode::HOME,
                Some(b'F') => keycode::END,
                Some(b'~') => match bytes.get(2).copied() {
                    Some(b'5') => keycode::PAGE_UP,
                    Some(b'6') => keycode::PAGE_DOWN,
                    _ => keycode::UNKNOWN,
                },
                _ => keycode::UNKNOWN,
            }
        }
        Some(b'\n') | Some(b'\r') => keycode::RETURN,
        Some(0x7f) | Some(0x08) => keycode::DELETE,
        Some(b'\t') => keycode::TAB,
        Some(b) => b as u32,
    }
}

/// Capture the current terminal mode (`stty -g`) for later restore; `None` when
/// stdin is not a TTY (nothing to restore).
fn capture_mode(stdin_is_tty: bool) -> Option<String> {
    if !stdin_is_tty {
        return None;
    }
    process::Command::new("stty")
        .arg("-g")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

/// Restore the terminal to the captured (cooked + echo) mode.
fn restore_mode(orig: &Option<String>) {
    if let Some(s) = orig {
        let _ = process::Command::new("stty").arg(s).status();
    }
}

/// Read a cooked line of input from stdin (the terminal echoes it).
fn read_line_stdin() -> String {
    let mut line = String::new();
    let _ = io::stdin().lock().read_line(&mut line);
    line
}

/// Read one keypress. On a TTY: raw single key (escape sequences decoded), always
/// restoring the cooked mode afterward. Piped: the first byte of the next line.
fn read_char_input(stdin_is_tty: bool, orig: &Option<String>) -> u32 {
    if !stdin_is_tty {
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
        return decode_key(line.as_bytes());
    }
    let _ = process::Command::new("stty")
        .args(["-icanon", "-echo", "min", "1", "time", "0"])
        .status();
    let mut first = [0u8; 1];
    let n = io::stdin().read(&mut first).unwrap_or(0);
    let key = if n == 0 {
        keycode::RETURN
    } else if first[0] == 0x1b {
        // Grab the (brief, non-blocking) continuation and decode the sequence.
        let _ = process::Command::new("stty").args(["min", "0", "time", "1"]).status();
        let mut rest = [0u8; 8];
        let m = io::stdin().read(&mut rest).unwrap_or(0);
        let mut seq = vec![0x1b];
        seq.extend_from_slice(&rest[..m]);
        decode_key(&seq)
    } else {
        decode_key(&first[..1])
    };
    restore_mode(orig); // always return to the known-good cooked+echo mode
    key
}

/// Drive `machine` to completion through the Glk step loop, pulling input from
/// the supplied readers. `before_input` runs just before each blocking read (the
/// CLI uses it to flush output so the prompt is visible). This is the shared,
/// backend-agnostic loop — the CLI wires real stdin/terminal readers; tests wire
/// scripted ones.
fn drive(
    machine: &mut Machine,
    mut before_input: impl FnMut(&mut Machine),
    mut read_line: impl FnMut() -> String,
    mut read_char: impl FnMut() -> u32,
) {
    loop {
        match machine.step() {
            StepResult::Continue => {}
            StepResult::Quit => break,
            StepResult::NeedLine { .. } => {
                before_input(machine);
                let line = read_line();
                machine.supply_line(line.trim_end_matches(['\n', '\r']));
            }
            StepResult::NeedChar { .. } => {
                before_input(machine);
                let key = read_char();
                machine.supply_char(key);
            }
        }
    }
}

fn main() {
    let argv: Vec<String> = env::args().collect();
    let Some(path) = argv.get(1) else {
        eprintln!("Usage: {} <story.ulx | story.gblorb>", argv[0]);
        process::exit(1);
    };

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{path}': {e}");
            process::exit(1);
        }
    };

    let mut machine = match build_machine(bytes, Box::new(TerminalBackend::new())) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    let stdin_is_tty = io::stdin().is_terminal();
    // Capture the original terminal mode once; each raw read restores to it and
    // we restore again on exit, so echo is always returned to the user.
    let orig_mode = capture_mode(stdin_is_tty);

    drive(
        &mut machine,
        |m| {
            // Flush pending output (without tearing down the scroll region) so the
            // prompt is visible before we block for input.
            if let Some(t) = m.backend_mut().as_any_mut().downcast_mut::<TerminalBackend>() {
                t.flush_out();
            }
        },
        read_line_stdin,
        || read_char_input(stdin_is_tty, &orig_mode),
    );

    machine.flush();
    restore_mode(&orig_mode);

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

    // ── Task 3: interactive input (decode_key + the drive loop) ───────────────

    #[test]
    fn decode_key_maps_bytes_and_escape_sequences() {
        assert_eq!(decode_key(b"a"), b'a' as u32);
        assert_eq!(decode_key(b"7"), b'7' as u32);
        assert_eq!(decode_key(b""), keycode::RETURN, "EOF → Enter");
        assert_eq!(decode_key(b"\n"), keycode::RETURN);
        assert_eq!(decode_key(b"\r"), keycode::RETURN);
        assert_eq!(decode_key(&[0x7f]), keycode::DELETE);
        assert_eq!(decode_key(&[0x08]), keycode::DELETE);
        assert_eq!(decode_key(b"\t"), keycode::TAB);
        assert_eq!(decode_key(&[0x1b]), keycode::ESCAPE, "lone ESC");
        assert_eq!(decode_key(&[0x1b, b'[', b'A']), keycode::UP);
        assert_eq!(decode_key(&[0x1b, b'[', b'B']), keycode::DOWN);
        assert_eq!(decode_key(&[0x1b, b'[', b'C']), keycode::RIGHT);
        assert_eq!(decode_key(&[0x1b, b'[', b'D']), keycode::LEFT);
        assert_eq!(decode_key(&[0x1b, b'[', b'H']), keycode::HOME);
        assert_eq!(decode_key(&[0x1b, b'[', b'5', b'~']), keycode::PAGE_UP);
        assert_eq!(decode_key(&[0x1b, b'[', b'Z']), keycode::UNKNOWN, "unknown escape");
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
        drive(&mut m, |_| {}, String::new, move || keys.next().unwrap_or(keycode::RETURN));
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
        drive(&mut m, |_| {}, move || lines.next().unwrap_or_default(), || keycode::RETURN);
        let text = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
        assert_eq!(text, "hello", "the typed line was supplied into the buffer and printed");
    }
}
