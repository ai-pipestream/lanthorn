# zvm-cli Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the zvm-cli DOS-parity polish — `read_char` escape-key decoding, `[MORE]` paging, IFID-keyed aux storage — and fix two TTY bugs in the shipped screen model (no screen reset on startup; terminal echo lost after a raw-input menu).

**Architecture:** All in `crates/zvm-cli` except moving the pure `compute_ifid` helper into `crates/zvm` (the app re-exports it so its call sites/tests are untouched). TTY-only effects stay inert when piped.

**Tech Stack:** Rust, std only (zero new deps). ANSI bytes; `stty`/`std::process::Command` for terminal mode + size; `std::io::IsTerminal`.

**Spec:** `docs/superpowers/specs/2026-06-27-zvm-cli-polish-design.md`

## Global Constraints

- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green after every task.
- `zvm-cli` stays zero-dependency. The only engine change is moving `compute_ifid` into `zvm` (pure; app re-exports it, so app tests stay green).
- TTY-only effects (`[MORE]`, escape decoding, screen reset, raw mode) MUST be inert when piped — never block or emit ANSI into piped output. `--no-status` stays byte-identical to legacy; `--no-more`/`--no-aux` behave as named.
- Commit-only on local `main`; one commit per task (TDD). No push.
- Commit trailers on every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Do not edit `TODO.md`.

## Reference: current code

- `crates/app/src/ifid.rs`: `pub fn compute_ifid(story: &[u8]) -> String` (+ `map_path`/`archive_path` which STAY in app). Used by `app::picker` etc. as `crate::ifid::compute_ifid`.
- `crates/zvm-cli/src/aux.rs`: `pub fn aux_path(story: &Path) -> PathBuf` (= `story.with_extension("aux")`), `encode_aux`/`decode_aux`.
- `crates/zvm-cli/src/main.rs`: `StdoutOutput { is_tty }`, `build_machine(story, stdout_is_tty)`, `parse_args` → `Args { story, no_status, no_aux }`, `detect_term_rows()`, `read_char_input(stdin_is_tty)` (captures `stty -g` per-call), `aux_preload`/`aux_flush(machine, story_path, no_aux)`, and the run loop.
- `crates/zvm-cli/src/screen.rs`: pure helpers + `ScreenView { is_tty, no_status, term_rows, active_rows, last_block }` with `frame(&Machine)`, `leave()`, `enter_region`, `leave_region`.

---

## Task 1: Move `compute_ifid` into `zvm`; key aux by IFID

**Files:**
- Create: `crates/zvm/src/ifid.rs`; Modify: `crates/zvm/src/lib.rs` (`pub mod ifid;`)
- Modify: `crates/app/src/ifid.rs` (re-export; drop the moved fn + its tests)
- Modify: `crates/zvm-cli/src/aux.rs` (new `aux_path` signature + `sanitize_ifid`)
- Modify: `crates/zvm-cli/src/main.rs` (compute IFID once; thread the aux file path)

**Interfaces:**
- Produces: `zvm::ifid::compute_ifid(&[u8]) -> String`; `aux::aux_path(story_path: &Path, ifid: &str) -> PathBuf`; `aux::sanitize_ifid(&str) -> String`.

- [ ] **Step 1: Create `crates/zvm/src/ifid.rs`** (move the fn + its two tests verbatim)

```rust
//! Interpreter IFID derived from a Z-code story's header.

/// `ZCODE-<release>-<serial>-<checksum hex>` (release @0x02, serial @0x12, checksum @0x1C).
pub fn compute_ifid(story: &[u8]) -> String {
    if story.len() < 0x1E {
        return "ZCODE-INVALID".to_string();
    }
    let release = u16::from_be_bytes([story[0x02], story[0x03]]);
    let serial: String = story[0x12..0x18]
        .iter()
        .map(|&b| if b.is_ascii() && !b.is_ascii_control() { b as char } else { '-' })
        .collect();
    let checksum = u16::from_be_bytes([story[0x1C], story[0x1D]]);
    format!("ZCODE-{}-{}-{:04X}", release, serial, checksum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story_with(release: u16, serial: &[u8; 6], checksum: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x40];
        b[0x02] = (release >> 8) as u8; b[0x03] = release as u8;
        b[0x12..0x18].copy_from_slice(serial);
        b[0x1C] = (checksum >> 8) as u8; b[0x1D] = checksum as u8;
        b
    }

    #[test]
    fn computes_zcode_ifid() {
        let s = story_with(42, b"871124", 0xABCD);
        assert_eq!(compute_ifid(&s), "ZCODE-42-871124-ABCD");
    }

    #[test]
    fn invalid_when_too_short() {
        assert_eq!(compute_ifid(&[0u8; 4]), "ZCODE-INVALID");
    }
}
```

- [ ] **Step 2: Export it from zvm** — add `pub mod ifid;` to `crates/zvm/src/lib.rs` (next to the other `pub mod` lines).

- [ ] **Step 3: Re-export from app; drop the moved code** in `crates/app/src/ifid.rs`

Replace the `compute_ifid` fn body with a re-export and remove the now-moved tests (`computes_zcode_ifid`, `invalid_when_too_short`) and the `story_with` helper they used. Keep `map_path`/`archive_path` and their tests:

```rust
pub use zvm::ifid::compute_ifid;

pub fn map_path(base_dir: &std::path::Path, ifid: &str) -> std::path::PathBuf {
    base_dir.join(format!("{ifid}.map.json"))
}

pub fn archive_path(base_dir: &std::path::Path, ifid: &str) -> std::path::PathBuf {
    base_dir.join(format!("{ifid}.lanthorn"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn map_path_uses_ifid() {
        let p = map_path(Path::new("/tmp/maps"), "ZCODE-42-871124-ABCD");
        assert_eq!(p, Path::new("/tmp/maps/ZCODE-42-871124-ABCD.map.json"));
    }

    #[test]
    fn archive_path_uses_ifid() {
        let p = archive_path(Path::new("/tmp/maps"), "ZCODE-42-871124-ABCD");
        assert_eq!(p, Path::new("/tmp/maps/ZCODE-42-871124-ABCD.lanthorn"));
    }
}
```

- [ ] **Step 4: Run** — `cargo test -p zvm -p app` PASS (ifid tests now under zvm; app re-export compiles; picker etc. unchanged).

- [ ] **Step 5: New aux path test (failing)** in `crates/zvm-cli/src/aux.rs` — replace the existing `path_uses_stem_and_aux_ext` test:

```rust
    #[test]
    fn aux_path_uses_ifid_in_story_dir() {
        assert_eq!(
            aux_path(Path::new("/g/story.z5"), "ZCODE-1-840726-ABCD"),
            Path::new("/g/ZCODE-1-840726-ABCD.aux")
        );
        // unsafe characters in an IFID are sanitized
        assert_eq!(sanitize_ifid("ZCODE-1-../x"), "ZCODE-1-___x");
    }
```

- [ ] **Step 6: Implement** in `crates/zvm-cli/src/aux.rs`

```rust
pub fn sanitize_ifid(ifid: &str) -> String {
    ifid.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

pub fn aux_path(story_path: &Path, ifid: &str) -> PathBuf {
    let dir = story_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    dir.join(format!("{}.aux", sanitize_ifid(ifid)))
}
```

- [ ] **Step 7: Thread the IFID through `main.rs`**

After reading the story bytes, compute the aux file once and pass it to preload/flush (change their signatures from `story_path: &Path` to `aux_file: &Path`):

```rust
    let ifid = zvm::ifid::compute_ifid(&original_bytes);
    let aux_file = aux::aux_path(&story_path, &ifid);
    // ...
    aux_preload(&mut machine, &aux_file, args.no_aux);   // startup + each Restart
    // ...
    aux_flush(&mut machine, &aux_file, args.no_aux);     // after each step
```

```rust
fn aux_preload(machine: &mut Machine, aux_file: &Path, no_aux: bool) {
    if no_aux { return; }
    if let Ok(bytes) = fs::read(aux_file) {
        match aux::decode_aux(&bytes) {
            Ok(map) => { machine.aux_data = map; machine.aux_dirty = false; }
            Err(e) => eprintln!("zvm: warning: ignoring corrupt {}: {:?}", aux_file.display(), e),
        }
    }
}
fn aux_flush(machine: &mut Machine, aux_file: &Path, no_aux: bool) {
    if no_aux || !machine.aux_dirty { return; }
    if let Err(e) = fs::write(aux_file, aux::encode_aux(&machine.aux_data)) {
        eprintln!("zvm: warning: aux save to {} failed: {}", aux_file.display(), e);
    }
    machine.aux_dirty = false;
}
```

- [ ] **Step 8: Run + commit** — `cargo test --workspace` green, 0 warnings.

```bash
git add crates/zvm/src/ifid.rs crates/zvm/src/lib.rs crates/app/src/ifid.rs crates/zvm-cli/src/aux.rs crates/zvm-cli/src/main.rs
git commit  # feat(zvm-cli): key aux storage by IFID (compute_ifid moved into zvm)
```

---

## Task 2: Reset the screen on startup (Bug A)

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`ScreenView::start` + test)
- Modify: `crates/zvm-cli/src/main.rs` (emit it once before the loop)

**Interfaces:**
- Produces: `ScreenView::start(&self) -> String`.

- [ ] **Step 1: Failing test** in `screen.rs` view tests

```rust
    #[test]
    fn start_clears_screen_only_when_interactive() {
        assert_eq!(ScreenView::new(true, false, 24).start(), "\x1b[2J\x1b[H");
        assert_eq!(ScreenView::new(false, false, 24).start(), ""); // piped
        assert_eq!(ScreenView::new(true, true, 24).start(), "");   // --no-status
    }
```

- [ ] **Step 2: Run → fail** (`no method named start`).

- [ ] **Step 3: Implement** in `screen.rs`

```rust
impl ScreenView {
    /// Clear+home the screen at startup (interactive only), so existing
    /// scrollback is not overwritten by the pinned region.
    pub fn start(&self) -> String {
        if self.is_tty && !self.no_status {
            "\x1b[2J\x1b[H".to_string()
        } else {
            String::new()
        }
    }
}
```

(If `is_tty`/`no_status` are private and unreachable from the test module, they already are in the same module — fine. No new fields.)

- [ ] **Step 4: Emit it in `main`** — right after constructing `view`, before the loop:

```rust
    let mut view = screen::ScreenView::new(stdout_is_tty, args.no_status, detect_term_rows());
    print!("{}", view.start());
    let _ = io::stdout().flush();
```

- [ ] **Step 5: Run + commit**

```bash
git add crates/zvm-cli/src/screen.rs crates/zvm-cli/src/main.rs
git commit  # fix(zvm-cli): clear the screen on startup so existing terminal text is not overwritten
```

---

## Task 3: Raw-key input — capture-once restore + escape decoding (Bug B + arrows/F-keys)

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`decode_escape_seq` + tests)
- Modify: `crates/zvm-cli/src/main.rs` (orig-mode capture, `restore_mode`, rewritten `read_char_input`, restore on exit)

**Interfaces:**
- Produces: `screen::decode_escape_seq(seq: &[u8]) -> Option<u8>`; `read_char_input(stdin_is_tty, orig: &Option<String>) -> u8`; `restore_mode(orig: &Option<String>)`.

- [ ] **Step 1: Failing test** in `screen.rs`

```rust
    #[test]
    fn decode_escape_seq_maps_arrows_and_fkeys() {
        assert_eq!(decode_escape_seq(b"[A"), Some(129)); // up
        assert_eq!(decode_escape_seq(b"[B"), Some(130)); // down
        assert_eq!(decode_escape_seq(b"[D"), Some(131)); // left
        assert_eq!(decode_escape_seq(b"[C"), Some(132)); // right
        assert_eq!(decode_escape_seq(b"OA"), Some(129)); // up (SS3)
        assert_eq!(decode_escape_seq(b"OP"), Some(133)); // F1
        assert_eq!(decode_escape_seq(b"OS"), Some(136)); // F4
        assert_eq!(decode_escape_seq(b"[Z"), None);      // unknown
        assert_eq!(decode_escape_seq(b""), None);
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement `decode_escape_seq`** in `screen.rs`

```rust
/// Map the bytes AFTER an ESC into a Z-machine input code (ZMSD §3.8):
/// cursor keys 129-132 (up/down/left/right), F1-F4 133-136. `None` if unknown.
pub fn decode_escape_seq(seq: &[u8]) -> Option<u8> {
    match seq {
        b"[A" | b"OA" => Some(129),
        b"[B" | b"OB" => Some(130),
        b"[D" | b"OD" => Some(131),
        b"[C" | b"OC" => Some(132),
        b"OP" => Some(133),
        b"OQ" => Some(134),
        b"OR" => Some(135),
        b"OS" => Some(136),
        _ => None,
    }
}
```

- [ ] **Step 4: Rewrite the raw-input path in `main.rs`**

Capture the original terminal mode ONCE (in `main`, after `stdin_is_tty`):

```rust
    let orig_mode: Option<String> = if stdin_is_tty {
        process::Command::new("stty").arg("-g").output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    } else {
        None
    };
```

Replace `read_char_input` and add `restore_mode`:

```rust
fn restore_mode(orig: &Option<String>) {
    if let Some(s) = orig {
        let _ = process::Command::new("stty").arg(s).status();
    }
}

/// Read one keypress (TTY: raw, decoding escape sequences); always restores the
/// original cooked mode afterward. Piped: a byte from a line, as before.
fn read_char_input(stdin_is_tty: bool, orig: &Option<String>) -> u8 {
    use std::io::Read;
    if !screen::wants_raw_char(stdin_is_tty) {
        return read_byte_stdin();
    }
    let _ = process::Command::new("stty").args(["-icanon", "-echo", "min", "1", "time", "0"]).status();
    let mut first = [0u8; 1];
    let n = io::stdin().read(&mut first).unwrap_or(0);
    let key = if n == 0 {
        b'\n'
    } else if first[0] == 0x1B {
        // Read the (brief, non-blocking) continuation and decode it.
        let _ = process::Command::new("stty").args(["min", "0", "time", "1"]).status();
        let mut rest = [0u8; 8];
        let m = io::stdin().read(&mut rest).unwrap_or(0);
        screen::decode_escape_seq(&rest[..m]).unwrap_or(0x1B)
    } else {
        first[0]
    };
    restore_mode(orig); // always back to the known-good cooked+echo mode
    key
}
```

Update the `NeedChar` call site to `read_char_input(stdin_is_tty, &orig_mode)`.

Restore the terminal on exit — in the `Quit` arm and before each `process::exit` that happens after `orig_mode` exists:

```rust
            StepResult::Quit => {
                print!("{}", view.leave());
                let _ = io::stdout().flush();
                restore_mode(&orig_mode);
                break;
            }
```

- [ ] **Step 5: Run + manual smoke** — `cargo test --workspace` green, 0 warnings. Manually: after navigating a `read_char` menu, the next line prompt echoes typing (Bug B fixed); arrow keys move a v4+ menu.

- [ ] **Step 6: Commit**

```bash
git add crates/zvm-cli/src/screen.rs crates/zvm-cli/src/main.rs
git commit  # fix(zvm-cli): own terminal mode (restore echo) + decode arrow/F-key read_char
```

---

## Task 4: `[MORE]` paging

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`should_page` + test)
- Modify: `crates/zvm-cli/src/main.rs` (`StdoutOutput` paging state; `--no-more`; reset after input)

**Interfaces:**
- Consumes: `restore_mode`/raw read from Task 3.
- Produces: `screen::should_page(lines: u16, page_height: u16) -> bool`; `Args.no_more`; paging fields on `StdoutOutput`.

- [ ] **Step 1: Failing tests**

In `screen.rs`:
```rust
    #[test]
    fn should_page_at_threshold() {
        assert!(!should_page(0, 24));
        assert!(!should_page(22, 24));
        assert!(should_page(23, 24)); // page_height - 1
        assert!(should_page(99, 24));
        assert!(!should_page(5, 1));  // degenerate height never pages
    }
```

In `main.rs` arg tests:
```rust
    #[test]
    fn parses_no_more_flag() {
        let a = parse_args(&["zvm-cli".into(), "--no-more".into(), "g".into()]);
        assert!(a.no_more);
        let b = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!b.no_more);
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement `should_page`** in `screen.rs`

```rust
/// True once `lines` reaches the page limit (`page_height - 1`); a height < 2
/// never pages (avoids a zero/looping page).
pub fn should_page(lines: u16, page_height: u16) -> bool {
    page_height >= 2 && lines >= page_height - 1
}
```

- [ ] **Step 4: Add `--no-more` to args**

```rust
struct Args { story: Option<String>, no_status: bool, no_aux: bool, no_more: bool }
// in parse_args: default no_more: false; arm: "--no-more" | "--no-page" => a.no_more = true,
```

- [ ] **Step 5: Make `StdoutOutput` page**

```rust
struct StdoutOutput {
    is_tty: bool,
    paging: bool,
    page_height: u16,
    lines: u16,
    orig_mode: Option<String>,
}

impl StdoutOutput {
    fn new(is_tty: bool, paging: bool, page_height: u16, orig_mode: Option<String>) -> Self {
        StdoutOutput { is_tty, paging, page_height, lines: 0, orig_mode }
    }

    fn write_counted(&mut self, s: &str) {
        // Emit, counting newlines; pause with [MORE] when a page fills.
        for (i, segment) in s.split('\n').enumerate() {
            if i > 0 {
                print!("\n");
                self.lines += 1;
                if self.paging && crate::screen::should_page(self.lines, self.page_height) {
                    let _ = io::stdout().flush();
                    print!("\x1b[7m[MORE]\x1b[0m");
                    let _ = io::stdout().flush();
                    let _ = read_char_input(true, &self.orig_mode); // wait for a key
                    print!("\r\x1b[2K"); // erase the [MORE] prompt
                    self.lines = 0;
                }
            }
            print!("{}", segment);
        }
        let _ = io::stdout().flush();
    }
}

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) { self.write_counted(s); }
    fn print_styled(&mut self, s: &str, style: u8) {
        let out = crate::screen::style_wrap(s, style, self.is_tty);
        self.write_counted(&out);
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

- [ ] **Step 6: Wire paging config in `main`** (capture `orig_mode` BEFORE `build_machine` so the sink can hold a clone)

```rust
    let term_rows = detect_term_rows();
    let paging = stdout_is_tty && stdin_is_tty && !args.no_more;
    let page_height = term_rows.saturating_sub(2).max(2);
    // build_machine now takes these:
    let mut machine = build_machine(story_bytes, stdout_is_tty, paging, page_height, orig_mode.clone())?;
```

Update `build_machine` to forward them into `StdoutOutput::new(...)`. After each input prompt (`NeedLine`/`NeedChar`), reset the page counter:

```rust
            if let Some(o) = machine.out.as_any_mut().downcast_mut::<StdoutOutput>() {
                o.lines = 0;
            }
```

(Add this right after `supply_line` / `supply_char`.)

- [ ] **Step 7: Run + manual smoke** — `cargo test --workspace` green, 0 warnings. Manual: a long room description on a small terminal pauses with `[MORE]` and continues on a keypress; piped output (no TTY) never pauses and contains no `[MORE]`.

- [ ] **Step 8: Commit**

```bash
git add crates/zvm-cli/src/screen.rs crates/zvm-cli/src/main.rs
git commit  # feat(zvm-cli): [MORE] paging (TTY-only, --no-more opt-out)
```

---

## Self-review checklist (run before final review)

- IFID: `compute_ifid` lives only in zvm; app re-exports it; app + picker tests green; aux file is `<dir>/<ifid>.aux`, sanitized.
- Screen reset only when `is_tty && !no_status`; never piped/`--no-status`.
- After a `read_char` menu the line prompt echoes again; terminal restored on Quit.
- `[MORE]` never engages when piped or `--no-more`; resets after input; `should_page` floored at height ≥ 2.
- `--no-status` output byte-identical to legacy; no ANSI/`[MORE]`/`\x07` ever in piped output.
- 0 warnings; `cargo test --workspace` green.
