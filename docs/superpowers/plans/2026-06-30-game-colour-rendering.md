# Game Colour & Upper-Window Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** BeyondZork and colour v5+ games render game colour faithfully in both hosts — visible menu highlight, red score box, and painted background — matching Frotz.

**Architecture:** Host-only fixes (no engine change). zvm-cli's upper-window renderer emits per-cell fg/bg and paints the game's `current_bg` on clears/lines; the app maps standard colours to concrete ANSI (instead of `Color::Reset`) and fills the story pane with `current_bg`. Everything stays gated behind `honor_game_colours` (default on), and the automap/chrome keep the theme.

**Tech Stack:** Rust workspace; `zvm-cli` (crossterm, raw ANSI SGR), `app` (ratatui). Design: `docs/superpowers/specs/2026-06-30-game-colour-rendering-design.md`.

## Global Constraints

- Host-only: do NOT modify `crates/zvm` or `crates/gvm` (both zero-dependency; `gvm` untouched). All engine state needed is already public: `machine.honor_game_colours: bool` and `machine.screen.current_bg: ZColour` / `current_fg`.
- Z-machine standard colours: `Standard(2)`=black, `(3)`=red, `(4)`=green, `(5)`=yellow, `(6)`=blue, `(7)`=magenta, `(8)`=cyan, `(9)`=white.
- All colour rendering stays gated on `honor_game_colours` (the F2 toggle, default on). When off, behaviour is unchanged (theme/terminal colours).
- Theme-safe: no forced default background. The automap pane and app chrome are never painted with the game background — only the story-output pane is.
- 0 compiler warnings AND full `cargo test --workspace` green before every commit.
- Stage only the files each task names; never `git add -A`. Scratch stays under `.superpowers/`.
- Commit locally only; do NOT push. Commit trailers on every commit, NO backticks in bodies:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

---

## File Structure

- `crates/zvm-cli/src/screen.rs` — `upper_row_ansi` per-cell fg/bg (Task 1); `erase`/`render`/line background paint (Task 2).
- `crates/zvm-cli/src/main.rs` — pass `current_bg`/`honor` into the paint call sites (Task 2).
- `crates/app/src/colors.rs` — `terminal_default().palette` → concrete ANSI (Task 3).
- `crates/app/src/engine.rs` — `ScreenModel.bg` field (Task 4).
- `crates/app/src/session.rs` — populate `ScreenModel.bg` from `current_bg` (Task 4).
- `crates/app/src/render/screen.rs` — story-pane background fill (Task 4).

---

## Task 1: zvm-cli — upper-window per-cell colour

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`upper_row_ansi` at 121-147; `rows_ansi` at 225-234)
- Test: `crates/zvm-cli/src/screen.rs` (test module)

**Interfaces:**
- Consumes: `zvm::io::TextAttrs { style: u8, fg: ZColour, bg: ZColour }`; existing `sgr_open(attrs: TextAttrs) -> String` (screen.rs:46) which builds an `ESC[..m` set-sequence (no trailing reset) for style+fg+bg; `zvm::screen::{Cell, UpperWindow, ZColour}`; `machine.honor_game_colours: bool`.
- Produces: `upper_row_ansi(upper: &UpperWindow, row: u16, honor: bool) -> String` — per-cell runs carrying style AND fg/bg (fg/bg suppressed when `honor` is false).

- [ ] **Step 1: Write the failing test**

Add to the `screen.rs` test module (near the existing `upper_row_text_and_ansi` test at ~496):

```rust
    #[test]
    fn upper_row_ansi_emits_per_cell_fg_bg() {
        use zvm::screen::ZColour;
        let mut u = UpperWindow::default();
        u.resize(1, 6);
        // "Hi" in red-on-black, honor on.
        u.put(1, 1, 'H', 0, ZColour::Standard(3), ZColour::Standard(2));
        u.put(1, 2, 'i', 0, ZColour::Standard(3), ZColour::Standard(2));
        let on = upper_row_ansi(&u, 1, true);
        assert!(on.contains("31"), "red fg SGR present: {on:?}");
        assert!(on.contains("40"), "black bg SGR present: {on:?}");
        assert!(on.contains("Hi"), "text present: {on:?}");
        // honor off: no colour SGR, text still present.
        let off = upper_row_ansi(&u, 1, false);
        assert!(!off.contains("31") && !off.contains("40"), "no colour when honor off: {off:?}");
        assert!(off.contains("Hi"), "text present when honor off: {off:?}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p zvm-cli upper_row_ansi_emits 2>&1 | tail -20`
Expected: FAIL — `upper_row_ansi` takes 2 args (arity mismatch) / no colour emitted.

- [ ] **Step 3: Rewrite `upper_row_ansi` to emit per-cell attributes**

Replace the whole function body (screen.rs:121-147). The run breaks whenever any of style/fg/bg changes; when `honor` is false, fg/bg are forced to `Default` (mirroring `StdoutOutput::print_attr` in main.rs) so only style bits show:

```rust
/// One upper-window row with per-cell SGR runs (for the pinned TTY region).
/// Carries style AND colour (fg/bg); colour is suppressed when `honor` is false.
pub fn upper_row_ansi(upper: &UpperWindow, row: u16, honor: bool) -> String {
    // Last column with any non-default attribute (blank cell = ' ' at style 0,
    // Default/Default); trailing defaults are dropped so the row closes reset,
    // matching the `ESC[2K` line-clear done before each row is written.
    let last = (1..=upper.cols)
        .rev()
        .find(|&c| {
            let cell = upper.cell(row, c);
            cell.ch != ' '
                || cell.style != 0
                || (honor && !matches!(cell.fg, ZColour::Default))
                || (honor && !matches!(cell.bg, ZColour::Default))
        })
        .unwrap_or(0);
    let mut out = String::new();
    let mut cur: Option<TextAttrs> = None;
    for c in 1..=last {
        let cell = upper.cell(row, c);
        let attrs = if honor {
            TextAttrs { style: cell.style, fg: cell.fg, bg: cell.bg }
        } else {
            TextAttrs { style: cell.style, fg: ZColour::Default, bg: ZColour::Default }
        };
        if cur != Some(attrs) {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_open(attrs));
            cur = Some(attrs);
        }
        out.push(cell.ch);
    }
    if cur.is_some() {
        out.push_str("\x1b[0m");
    }
    out
}
```

Add `use zvm::io::TextAttrs;` and `ZColour` to the existing `use` at the top of screen.rs if not already imported (the file already imports `ZColour` at line 5; `TextAttrs` at line 4). Ensure `TextAttrs` derives `PartialEq` — check `crates/zvm/src/io.rs`; if it does not, compare fields instead of the struct: replace `cur != Some(attrs)` with a helper that compares `(style, fg, bg)` tuples (all three are `Copy`+`PartialEq`).

- [ ] **Step 4: Update the one caller `rows_ansi`**

In `rows_ansi` (screen.rs:225-234), the v4+ branch calls `upper_row_ansi`. Thread `machine.honor_game_colours`:

```rust
            (1..=top)
                .map(|r| upper_row_ansi(&machine.screen.upper, r, machine.honor_game_colours))
                .collect()
```

- [ ] **Step 5: Fix the existing `upper_row_text_and_ansi` test call site**

The pre-existing test (screen.rs ~496) calls `upper_row_ansi(&u, 1)`. Update it to `upper_row_ansi(&u, 1, true)` so it compiles; its existing assertions on style still hold.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p zvm-cli 2>&1 | tail -20`
Expected: PASS (new test + updated existing test).
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 7: Commit**

```bash
git add crates/zvm-cli/src/screen.rs
git commit -F - <<'EOF'
fix(zvm-cli): render upper-window game colour (fg/bg), not just style

upper_row_ansi emitted only text-style bits and dropped each cell's fg/bg,
so BeyondZork's colour-highlighted menu selection and red score box were
invisible. Emit per-cell colour via sgr_open, gated on honor_game_colours.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 2: zvm-cli — paint the game background on clears and lines

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`erase` at 316-322; `render` per-row clear at 263-270; new `bg_sgr` helper)
- Modify: `crates/zvm-cli/src/main.rs` (`view.erase()` call site at 586; `view.frame(&machine)` sites already pass machine)
- Test: `crates/zvm-cli/src/screen.rs` (test module)

**Interfaces:**
- Consumes: `machine.screen.current_bg: ZColour`; `machine.honor_game_colours: bool`; `push_colour_sgr` (screen.rs:27, private).
- Produces: `bg_sgr(bg: ZColour, honor: bool) -> String` (the `ESC[..m` that sets just the background, or `""` for Default/honor-off); `erase(&mut self, bg: ZColour, honor: bool) -> String`.

- [ ] **Step 1: Write the failing tests**

Add to the `screen.rs` test module:

```rust
    #[test]
    fn bg_sgr_sets_background_only() {
        use zvm::screen::ZColour;
        assert_eq!(bg_sgr(ZColour::Standard(2), true), "\x1b[40m", "black bg");
        assert_eq!(bg_sgr(ZColour::Default, true), "", "default = no SGR");
        assert_eq!(bg_sgr(ZColour::Standard(2), false), "", "honor off = no SGR");
    }

    #[test]
    fn erase_paints_current_bg() {
        use zvm::screen::ZColour;
        let mut v = ScreenView::new(true, false, 24);
        let out = v.erase(ZColour::Standard(2), true);
        assert!(out.contains("\x1b[40m"), "bg SGR before clear: {out:?}");
        assert!(out.contains("\x1b[2J"), "screen clear present: {out:?}");
        assert!(out.find("\x1b[40m").unwrap() < out.find("\x1b[2J").unwrap(),
            "bg set before the clear: {out:?}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p zvm-cli 'bg_sgr\|erase_paints' 2>&1 | tail -20`
Expected: FAIL — `bg_sgr` undefined; `erase` takes 0 args.

- [ ] **Step 3: Add the `bg_sgr` helper**

Add near `sgr_open` (screen.rs, after line 58). It reuses the existing private `push_colour_sgr` with `fg = false`:

```rust
/// SGR that sets only the background colour (`ESC[..m`, no reset), or `""` when
/// the background is Default or colours are not honoured. Used to paint clears
/// and line padding with the game's chosen background.
pub fn bg_sgr(bg: ZColour, honor: bool) -> String {
    if !honor {
        return String::new();
    }
    let mut params: Vec<String> = Vec::new();
    push_colour_sgr(&mut params, bg, false);
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}
```

- [ ] **Step 4: Paint the background in `erase`**

Change `erase` (screen.rs:316-322) to take the background and prepend its SGR before the clear, so `ESC[2J` fills with the game background. Reset after so later output is unaffected:

```rust
    pub fn erase(&mut self, bg: ZColour, honor: bool) -> String {
        if !self.is_tty {
            return String::new();
        }
        self.active_rows = 0;
        let paint = bg_sgr(bg, honor);
        if paint.is_empty() {
            format!("{}\x1b[2J\x1b[H", leave_region())
        } else {
            // Set bg, clear (fills with bg), home, then reset so subsequent
            // prompt/text is not forced onto the painted background run.
            format!("{}{}\x1b[2J\x1b[H\x1b[0m", leave_region(), paint)
        }
    }
```

- [ ] **Step 5: Update the `erase` call site in main.rs**

At main.rs:586, change `view.erase()` to pass the game background and honor flag:

```rust
            print!("{}", view.erase(machine.screen.current_bg, machine.honor_game_colours));
```

- [ ] **Step 6: Paint the per-row clear in `render`**

In `render` (screen.rs:263-270), the TTY branch clears each upper row with `ESC[2K`. `render` does not have the machine; thread the background in from `frame`. Change `render`'s signature to accept `bg_paint: &str` (already-computed via `bg_sgr`) and prepend it before each row's `ESC[2K` so the cleared row fills with bg:

In `frame` (screen.rs:237-245), compute the paint and pass it:

```rust
    pub fn frame(&mut self, machine: &Machine) -> String {
        if self.no_status {
            return String::new();
        }
        let top = Self::top_rows(machine);
        let plain = Self::rows_plain(machine, top);
        let ansi = Self::rows_ansi(machine, top);
        let bg_paint = bg_sgr(machine.screen.current_bg, machine.honor_game_colours);
        self.render(top, &plain, &ansi, &bg_paint)
    }
```

In `render` change the signature and the per-row loop:

```rust
    fn render(&mut self, top: u16, rows_plain: &[String], rows_ansi: &[String], bg_paint: &str) -> String {
        // ... unchanged no_status / active_rows / enter_region logic ...
            if top > 0 {
                out.push_str("\x1b7"); // DECSC save cursor
                for (i, row) in rows_ansi.iter().enumerate() {
                    // position, paint bg, clear-to-EOL (fills with bg), then row
                    out.push_str(&format!("\x1b[{};1H{}\x1b[2K", i as u16 + 1, bg_paint));
                    out.push_str(row);
                }
                out.push_str("\x1b8"); // DECRC restore cursor
            }
        // ... unchanged non-TTY branch (ignores bg_paint) ...
    }
```

Update the existing `render` unit-test call sites in the test module (search `\.render(` — e.g. screen.rs:343/345/352/355/363/387/402) to pass a 4th argument `""` (empty paint). Their existing assertions are unaffected because `""` changes nothing.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p zvm-cli 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 8: Commit**

```bash
git add crates/zvm-cli/src/screen.rs crates/zvm-cli/src/main.rs
git commit -F - <<'EOF'
fix(zvm-cli): paint game background on screen clears and upper rows

Clears (erase / per-row ESC[2K) previously used the terminal ambient
colour, so a game that set a black background showed grey around and
between text. Emit the current background SGR (via bg_sgr) before the
clear/row so cleared regions fill with the game background. Gated on
honor_game_colours; Default background stays terminal-neutral.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 3: app — concrete ANSI palette (so game colour renders)

**Files:**
- Modify: `crates/app/src/colors.rs` (`terminal_default()` palette at 411)
- Test: `crates/app/src/render/mod.rs` or `colors.rs` test module

**Interfaces:**
- Consumes: `resolve_zcolour(c: ZColour, scheme: &ColorScheme) -> Color` (render/mod.rs:58) which maps `Standard(2..=9)` → `scheme.palette[n-2]`.
- Produces: a `terminal_default()` whose `palette` holds the 16 concrete ratatui ANSI colours, so `Standard(2)`→black … `Standard(9)`→white render as real colours instead of `Color::Reset`.

- [ ] **Step 1: Write the failing test**

Add to the `colors.rs` test module (or `render/mod.rs` tests). `resolve_zcolour` is `pub(crate)`:

```rust
    #[test]
    fn terminal_default_palette_maps_standard_colours_concretely() {
        use zvm::screen::ZColour;
        use ratatui::style::Color;
        let s = ColorScheme::terminal_default();
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(2), &s), Color::Black, "black");
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(3), &s), Color::Red, "red");
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(9), &s), Color::Gray, "white(9)->ANSI white");
    }
```

(Note: `Standard(9)` = Z-machine white → ANSI colour 7, which is ratatui `Color::Gray` — matching zvm-cli's SGR `37`. `Color::White` is ANSI bright-white/15; keeping `Gray` makes the two hosts consistent.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app terminal_default_palette_maps 2>&1 | tail -20`
Expected: FAIL — resolves to `Color::Reset`, not `Color::Black`/`Red`/`Gray`.

- [ ] **Step 3: Replace the palette**

In `terminal_default()` (colors.rs:411), replace `palette: [Color::Reset; 16],` with the standard 16-colour ANSI palette (indices 0–7 normal, 8–15 bright). `resolve_zcolour` uses indices 0–7 for `Standard(2..=9)`:

```rust
            palette: [
                Color::Black,      // 0  Z Standard(2) black
                Color::Red,        // 1  Standard(3) red
                Color::Green,      // 2  Standard(4) green
                Color::Yellow,     // 3  Standard(5) yellow
                Color::Blue,       // 4  Standard(6) blue
                Color::Magenta,    // 5  Standard(7) magenta
                Color::Cyan,       // 6  Standard(8) cyan
                Color::Gray,       // 7  Standard(9) white (ANSI 7)
                Color::DarkGray,   // 8  bright black
                Color::LightRed,   // 9
                Color::LightGreen, // 10
                Color::LightYellow,// 11
                Color::LightBlue,  // 12
                Color::LightMagenta,//13
                Color::LightCyan,  // 14
                Color::White,      // 15 bright white
            ],
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/colors.rs
git commit -F - <<'EOF'
fix(app): map standard game colours to concrete ANSI in default scheme

terminal_default's palette was all Color::Reset, so resolve_zcolour
flattened every Standard(2..=9) game colour (black, red, the menu
highlight) to "terminal decides" and they rendered invisible. Map the 16
slots to the concrete ratatui ANSI colours; a loaded Ghostty theme still
overrides. White(9) uses Color::Gray (ANSI 7) to match zvm-cli's SGR 37.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 4: app — fill the story pane with the game background

**Files:**
- Modify: `crates/app/src/engine.rs` (`ScreenModel` struct at 214-219)
- Modify: `crates/app/src/session.rs` (`screen_model_from_machine` at 531-564; and the two other `ScreenModel { .. }` literals — `glulx_session.rs:160` and `glk_backend.rs:150`)
- Modify: `crates/app/src/render/screen.rs` (`render_story_pane` at ~54-77)
- Test: `crates/app/src/render/screen.rs` (test module)

**Interfaces:**
- Consumes: `machine.screen.current_bg: ZColour`; `crate::state::pack_zcolour(ZColour) -> u32` and `unpack_zcolour(u32) -> ZColour` (state.rs:215/225); `crate::render::resolve_zcolour`; `state.config.honor_game_colours: bool`.
- Produces: `ScreenModel.bg: u32` (packed current background); story-pane fill using it.

- [ ] **Step 1: Write the failing test**

Add to the `render/screen.rs` test module. The test builds a minimal `ScreenModel` with a black `bg` and asserts the story-pane area's cells carry a black background after `render_story_pane`. Match the existing tests' setup style in that module for `AppState`/`Buffer`/`Rect` construction (read the neighbouring tests first):

```rust
    #[test]
    fn story_pane_fills_game_background() {
        use ratatui::style::Color;
        // ... build AppState `state` with honor_game_colours = true (mirror the
        // nearest existing test's AppState construction) ...
        let mut model = /* a simple ScreenModel with an empty transcript */;
        model.bg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(2)); // black
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);
        // A blank interior cell carries the game background (black).
        assert_eq!(buf.cell((0, 4)).unwrap().style().bg, Some(Color::Black),
            "story pane blank cell painted with game background");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app story_pane_fills_game_background 2>&1 | tail -20`
Expected: FAIL — `ScreenModel` has no `bg` field / blank cell has no black bg.

- [ ] **Step 3: Add the `bg` field to `ScreenModel`**

In engine.rs (214-219):

```rust
pub struct ScreenModel {
    /// The window tree.  In 3b-i this is the degenerate `Pair { Grid, Buffer }`.
    pub root: WinNode,
    /// The status line the app draws above the transcript.
    pub status: StatusModel,
    /// The game's current background colour, packed (see `crate::state::pack_zcolour`).
    /// `pack_zcolour(ZColour::Default)` when unset; used to paint the story pane.
    pub bg: u32,
}
```

- [ ] **Step 4: Populate `bg` at every `ScreenModel` construction**

In `screen_model_from_machine` (session.rs:555-563), add the field:

```rust
    ScreenModel {
        root: WinNode::Pair {
            vertical: true,
            split: Split { fixed: screen.upper.rows },
            first: Box::new(WinNode::Grid(grid)),
            second: Box::new(WinNode::Buffer(BufferWindow::default())),
        },
        status: status_model_from_machine(machine),
        bg: crate::state::pack_zcolour(screen.current_bg),
    }
```

Also add `bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),` to the other two `ScreenModel { .. }` literals so the crate compiles: `glulx_session.rs:160` (`blank_screen`) and `glk_backend.rs:150` (`screen_model`). (Glulx has no Z-machine background; Default is correct.)

- [ ] **Step 5: Fill the story pane before drawing**

In `render_story_pane` (render/screen.rs, the function starting ~54), add a background fill at the very top of the function body — before both the `is_simple` and generic branches — so it underlays the upper grid and transcript. Reuse the pattern of the existing `fill` helper (screen.rs:164) but with the game background:

```rust
    // Paint the story-pane background with the game's current background
    // (theme-safe: only the story pane, never the map/chrome; only a concrete,
    // honoured background — Default keeps the theme).
    if state.config.honor_game_colours {
        let bg = crate::state::unpack_zcolour(model.bg);
        if !matches!(bg, zvm::screen::ZColour::Default) {
            let bg_color = crate::render::resolve_zcolour(bg, &state.colors);
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(ratatui::style::Style::new().bg(bg_color));
                    }
                }
            }
        }
    }
```

Add any missing imports (`ratatui::style::Style` is already used in this crate; import if the module lacks it).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p app 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/engine.rs crates/app/src/session.rs crates/app/src/render/screen.rs crates/app/src/glulx_session.rs crates/app/src/glk_backend.rs
git commit -F - <<'EOF'
fix(app): paint the story pane with the game background

The app never read the game's current background, so BeyondZork's black
background left the story text on the theme background. Carry current_bg
on ScreenModel and fill the story pane (only) with it before drawing;
the automap and chrome keep the theme. Gated on honor_game_colours;
Default background keeps the theme.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 5: zvm-cli — paint upper-window default cells with the screen background

**Why:** Tasks 1+2 interact on centered upper-window text (BeyondZork's title). Task 2 fills each row with the game background via `ESC[40m ESC[2K`, but Task 1's `upper_row_ansi` emits a leading `ESC[0m` for the first Default/style-0 run, which resets that fill to terminal grey — so the leading centering spaces render grey while the text (concrete bg) renders black. Fix: resolve a cell's Default fg/bg to the screen's `current_fg`/`current_bg` so blank/leading cells carry the same background as the row fill.

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`upper_row_ansi` at 122-156; caller `rows_ansi`)
- Test: `crates/zvm-cli/src/screen.rs` (test module)

**Interfaces:**
- Consumes: `machine.screen.current_fg: ZColour`, `machine.screen.current_bg: ZColour` (both public).
- Produces: `upper_row_ansi(upper: &UpperWindow, row: u16, honor: bool, current_fg: ZColour, current_bg: ZColour) -> String` — Default cell fg/bg resolve to `current_fg`/`current_bg` when `honor`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn upper_row_ansi_paints_default_cells_with_current_bg() {
        use zvm::screen::ZColour;
        let mut u = UpperWindow::default();
        u.resize(1, 6);
        // cols 1-2 blank (Default bg); cols 3-4 'Hi' explicit white-on-black.
        u.put(1, 3, 'H', 0, ZColour::Standard(9), ZColour::Standard(2));
        u.put(1, 4, 'i', 0, ZColour::Standard(9), ZColour::Standard(2));
        // Screen background is black: leading blank cells must be painted black
        // (bg 40) BEFORE the text, not left to reset-to-terminal-default.
        let out = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Standard(2));
        let first40 = out.find("40").expect("bg 40 present");
        let hpos = out.find('H').expect("H present");
        assert!(first40 < hpos, "leading blanks painted black before text: {out:?}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p zvm-cli upper_row_ansi_paints_default 2>&1 | tail -20`
Expected: FAIL — `upper_row_ansi` takes 3 args (arity), and leading blanks aren't painted.

- [ ] **Step 3: Resolve Default cells to current_fg/current_bg**

Change the signature and the per-cell effective-attr computation in `upper_row_ansi` (screen.rs:122). Keep the `last` computation on the RAW cells as-is (trailing blanks still trimmed; the row's `ESC[2K` fill covers the trailing region). Only the emitted run's colours change:

```rust
pub fn upper_row_ansi(
    upper: &UpperWindow,
    row: u16,
    honor: bool,
    current_fg: ZColour,
    current_bg: ZColour,
) -> String {
    // ... unchanged `last` computation on raw cells ...
    let mut out = String::new();
    let mut cur: Option<(u8, ZColour, ZColour)> = None;
    for c in 1..=last {
        let cell = upper.cell(row, c);
        let (style, fg, bg) = if honor {
            // The upper window's "default" colour is the screen's current
            // colour (what the row-fill paints); resolve Default cells to it so
            // blank/leading cells match the painted background instead of
            // resetting to terminal-default.
            let fg = if matches!(cell.fg, ZColour::Default) { current_fg } else { cell.fg };
            let bg = if matches!(cell.bg, ZColour::Default) { current_bg } else { cell.bg };
            (cell.style, fg, bg)
        } else {
            (cell.style, ZColour::Default, ZColour::Default)
        };
        if cur != Some((style, fg, bg)) {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_open(TextAttrs { style, fg, bg }));
            cur = Some((style, fg, bg));
        }
        out.push(cell.ch);
    }
    if cur.is_some() {
        out.push_str("\x1b[0m");
    }
    out
}
```

- [ ] **Step 4: Update the caller `rows_ansi`**

In `rows_ansi` (screen.rs), pass the screen's current colours:

```rust
            (1..=top)
                .map(|r| upper_row_ansi(
                    &machine.screen.upper,
                    r,
                    machine.honor_game_colours,
                    machine.screen.current_fg,
                    machine.screen.current_bg,
                ))
                .collect()
```

- [ ] **Step 5: Fix the other `upper_row_ansi` call sites in tests**

`upper_row_text_and_ansi` (~496) and `upper_row_ansi_emits_per_cell_fg_bg` (from Task 1) call the 3-arg form. Update both to pass `ZColour::Default, ZColour::Default` as the two new args (no substitution → their existing assertions still hold: the per-cell test uses explicit Standard(3)/Standard(2) cells, unaffected by Default current colours).

- [ ] **Step 6: Run tests + warnings**

Run: `cargo test -p zvm-cli 2>&1 | tail -20`
Expected: PASS (new test + updated existing tests).
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 7: Commit**

```bash
git add crates/zvm-cli/src/screen.rs
git commit -F - <<'EOF'
fix(zvm-cli): paint upper-window default cells with the screen background

Centered upper-window text (BeyondZork's title) left its leading centering
spaces grey: the row is filled with the game background, but upper_row_ansi
reset the first Default/style-0 run to terminal-default, clobbering the
fill. Resolve Default cell fg/bg to the screen current_fg/current_bg so
blank and leading cells carry the same background as the painted row.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 6: app — colour the live input line with the game colour

**Why:** In BeyondZork the game sets an input colour (e.g. cyan). zvm-cli echoes the live input line in the game's `current_fg`/`current_bg`. The app's live input line (`render_input_content`) uses only the theme fields `input_text`/`input_prompt`, so what you type is uncoloured until submitted. Fix: thread the game's current fg/bg to the input-line render and patch the prompt+text with it (honor-gated). This mirrors Task 4's `current_bg` threading but adds `current_fg`.

**Files:**
- Modify: `crates/app/src/engine.rs` (`ScreenModel` — add `fg: u32` next to `bg` at ~221)
- Modify: `crates/app/src/session.rs` (`screen_model_from_machine` ~563; other `ScreenModel {}` literals in `glulx_session.rs`, `glk_backend.rs`, and test modules — set `fg`)
- Modify: `crates/app/src/render/screen.rs` (`render_story_pane` ~54, `render_node` ~81 — compute + thread `game_input: Option<Style>`)
- Modify: `crates/app/src/render/transcript.rs` (`render_transcript` ~748 signature + the two `render_input_content` calls ~805/807; `render_input_content` ~904 patches styles)
- Test: `crates/app/src/render/transcript.rs` (test module)

**Interfaces:**
- Consumes: `machine.screen.current_fg: ZColour`; `crate::state::{pack_zcolour, unpack_zcolour}`; `crate::render::resolve_zcolour`.
- Produces: `ScreenModel.fg: u32`; `render_transcript(.., game_input: Option<Style>)`; `render_input_content(.., game_input: Option<Style>)`.

- [ ] **Step 1: Write the failing test**

Add to the `render/transcript.rs` test module. Mirror the nearest existing test's `AppState` construction; set `state.input = "look".into()` and honor on. Assert the typed text cell carries the game fg (cyan = `Color::Cyan` after the Task 3 palette) when a game input style is passed:

```rust
    #[test]
    fn input_line_uses_game_colour() {
        use ratatui::style::Color;
        // ... build AppState `state` (honor_game_colours=true), state.input="x" ...
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let game = Some(ratatui::style::Style::new().fg(Color::Cyan));
        render_input_content(&state, &mut buf, area, ratatui::style::Style::new(), game);
        // The "> " prompt occupies cols 0-1; the typed 'x' is at col 2.
        assert_eq!(buf.cell((2, 0)).unwrap().style().fg, Some(Color::Cyan),
            "typed input uses the game colour");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app input_line_uses_game_colour 2>&1 | tail -20`
Expected: FAIL — `render_input_content` takes 4 args; typed text uses theme colour, not cyan.

- [ ] **Step 3: Add `fg` to `ScreenModel`**

In engine.rs, next to `bg` (~221):

```rust
    /// The game's current foreground colour, packed (see `crate::state::pack_zcolour`).
    /// `pack_zcolour(ZColour::Default)` when unset; used to colour the live input line.
    pub fg: u32,
```

- [ ] **Step 4: Populate `fg` at every `ScreenModel` literal**

In `screen_model_from_machine` (session.rs ~563), beside `bg`:

```rust
        bg: crate::state::pack_zcolour(screen.current_bg),
        fg: crate::state::pack_zcolour(screen.current_fg),
```

Add `fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),` (or `fg: 0`) to the other `ScreenModel {}` literals: `glulx_session.rs` `blank_screen`, `glk_backend.rs` `screen_model`, and every test-module literal (grep `ScreenModel {` across crates/app). `0 == pack_zcolour(Default)`.

- [ ] **Step 5: Compute + thread the game input style in render/screen.rs**

Add a helper near `render_story_pane` (screen.rs):

```rust
/// The game's live input colour (fg/bg) for the input line, or None when
/// colours are off or the game left both channels Default (theme-neutral).
fn game_input_style(model: &ScreenModel, state: &AppState) -> Option<ratatui::style::Style> {
    if !state.config.honor_game_colours {
        return None;
    }
    let fg = crate::state::unpack_zcolour(model.fg);
    let bg = crate::state::unpack_zcolour(model.bg);
    if matches!(fg, zvm::screen::ZColour::Default) && matches!(bg, zvm::screen::ZColour::Default) {
        return None;
    }
    let mut s = ratatui::style::Style::new();
    if !matches!(fg, zvm::screen::ZColour::Default) {
        s = s.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, zvm::screen::ZColour::Default) {
        s = s.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    Some(s)
}
```

In `render_story_pane`, compute `let gi = game_input_style(model, state);` once (after the existing background fill). Pass `gi` to `render_transcript` at the `is_simple` call site AND to `render_node`. Extend `render_node`'s signature with `game_input: Option<Style>` and thread it through its recursive `Pair` calls and into the `WinNode::Buffer` primary `render_transcript` call.

- [ ] **Step 6: Thread through `render_transcript` into `render_input_content`**

Add `game_input: Option<Style>` as the last parameter of `render_transcript` (transcript.rs:748) and pass it to both `render_input_content` calls (transcript.rs:805/807). Update the two `render_transcript(...)` call sites in render/screen.rs (already done in Step 5) to pass `gi`. If any OTHER `render_transcript` caller exists (grep), pass `None`.

- [ ] **Step 7: Patch the input styles in `render_input_content`**

Change the signature (transcript.rs:904) to accept `game_input: Option<Style>` and patch both prompt and text so the game colour wins over the theme fields:

```rust
    let base_prompt = normal_style.patch(state.colors.input_prompt);
    let base_text = normal_style.patch(state.colors.input_text);
    let (prompt_style, text_style) = match game_input {
        Some(gs) => (base_prompt.patch(gs), base_text.patch(gs)),
        None => (base_prompt, base_text),
    };
```

(the rest of the function — prefix/text draw, cursor — is unchanged.)

- [ ] **Step 8: Run the tests + warnings**

Run: `cargo test -p app 2>&1 | tail -20`
Expected: PASS (the new test + all existing).
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/engine.rs crates/app/src/session.rs crates/app/src/render/screen.rs crates/app/src/render/transcript.rs crates/app/src/glulx_session.rs crates/app/src/glk_backend.rs
git commit -F - <<'EOF'
fix(app): colour the live input line with the game colour

The live input line (what the user types at the prompt) used only the
theme input_text/input_prompt styles, so it stayed uncoloured until
submit, unlike zvm-cli which echoes input in the game's current colour.
Thread the game current_fg/current_bg (new ScreenModel.fg beside bg) to
render_input_content and patch the prompt+text with it, honor-gated.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Manual verification (after all tasks)

- `cargo run -p zvm-cli -- stories/beyondzork-r57-s871221.z5` → Character Setup menu selection is highlighted in colour; score box red; screen background black. `--no-game-colours` restores terminal colours.
- `cargo run -p app -- stories/beyondzork-r57-s871221.z5` (binary `lanthorn`) → menu highlighted; colours render; story-pane background black; automap keeps its theme. F2 `honor_game_colours` off restores the theme.

---

## Self-Review Notes

- **Spec coverage:** Component 2 (zvm-cli upper colour) → Task 1. Component 3 (zvm-cli bg paint) → Task 2. Component 4 (app palette) → Task 3. Component 5 (app story-pane bg) → Task 4. Component 1 (engine) → intentionally no task (no engine change under theme-safe seeding). Bug 3 (cursor/region) → out of scope, tracked separately.
- **Type consistency:** `upper_row_ansi(&UpperWindow, u16, bool)`; `bg_sgr(ZColour, bool) -> String`; `erase(&mut self, ZColour, bool)`; `render(.., bg_paint: &str)`; `ScreenModel.bg: u32` via `pack_zcolour`/`unpack_zcolour`; `resolve_zcolour(ZColour, &ColorScheme) -> Color`. All match the consuming sites.
- **Gate:** every colour/paint path checks `honor_game_colours`; Default background is a no-op in both hosts → theme preserved.
