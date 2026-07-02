# Glulx Room Detection (v1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect the current room in Glulx games from the Inform 7 room-heading (`GlkStyle::Subheader`) and feed it into the live map, so Glulx games automap.

**Architecture:** The app Glk backend (`AppGlk`) captures the last `Subheader` line of each turn as the room heading. `GlulxSession` turns it into a name-based `ObjectSnapshot` (sticky across heading-less turns) and reports it via `TurnResult.location` / `current_location()`, tagged with a new trusted `LocationMethod::RoomHeading` that bypasses the existing `NameOnly`-empty-graph gate. `gvm` is untouched.

**Tech Stack:** Rust workspace; `gvm` (Glulx VM, zero-dep), `app` (TUI: `glk_backend.rs`, `glulx_session.rs`, `session.rs`, `render/map.rs`), `zvm::location::LocationMethod` (shared UI vocabulary), `crate::roomid::synthetic_room_id`.

**Design source:** `docs/superpowers/specs/2026-07-02-glulx-room-detection-design.md`.

## Global Constraints

- `gvm` and `zvm` stay zero-dependency at runtime (test-only dev-deps are fine).
- Z-machine detection and the BeyondZork `NameOnly`-empty-graph gate (`crates/app/src/session.rs` `apply_turn`) must remain behaviorally unchanged — regression-free.
- Room identity is name-based: `crate::roomid::synthetic_room_id(name)` (FNV of the normalized name), `parent: 0`. Same-name disambiguation is explicitly OUT of scope (tracked as its own TODO).
- Only `GlkStyle::Subheader` is a room heading. `Header` (game title), `Emphasized`/`Input` (inline bold), and every other style are NOT.
- The room signal is the **last** complete `Subheader` line of a turn (skips the banner title, which precedes the room heading).
- Pre-game menus/setup screens must produce NO room; this falls out for free (they emit no `Subheader`), so do NOT add a Glulx-specific gate.
- `LocationInfo` is a type alias for `zvm::ObjectSnapshot` (`crates/app/src/engine.rs:280`); `ObjectSnapshot { number: u16, parent: u16, name: String }`.

---

## File Structure

- `crates/zvm/src/location.rs` — add `RoomHeading` to the `LocationMethod` enum (the app's shared detection-method vocabulary). No `Location` variant is added; the Glulx path sets the method directly.
- `crates/app/src/render/map.rs` — `loc_method_label` gains the `RoomHeading => "via room heading"` arm (exhaustive match — will not compile without it).
- `crates/app/src/glk_backend.rs` — `AppGlk` gains `Subheader` heading capture + `take_room_heading()`.
- `crates/app/src/glulx_session.rs` — thread the heading into `TurnResult.location` / `current_location()`, sticky via a new `last_room` field.
- `crates/app/src/session.rs` — no production change; add one test locking in that `RoomHeading` is not gated.
- `crates/app/tests/glulx_room_detection.rs` (new, `#[ignore]`d) — story-level verification against local `stories/`.

---

## Task 1: `RoomHeading` detection method + label

**Files:**
- Modify: `crates/zvm/src/location.rs` (the `LocationMethod` enum, ~line 235)
- Modify: `crates/app/src/render/map.rs` (`loc_method_label`, ~line 607)
- Test: `crates/app/src/render/map.rs` (existing `loc_method_label` test, ~line 2236)

**Interfaces:**
- Produces: `zvm::location::LocationMethod::RoomHeading` (used by Tasks 3–5); `loc_method_label(RoomHeading) == "via room heading"`.

- [ ] **Step 1: Add the enum variant**

In `crates/zvm/src/location.rs`, extend the enum:

```rust
pub enum LocationMethod {
    GlobalVar0,
    PlayerParent,
    StatusName,
    NameOnly,
    /// Glulx: the room was read from the Inform 7 `Subheader` room heading in
    /// the story buffer (name-based; no backing object). Trusted directly — not
    /// subject to the `NameOnly`-empty-graph gate.
    RoomHeading,
}
```

- [ ] **Step 2: Run the build to see the exhaustive-match break**

Run: `cargo build -p app 2>&1 | grep -A3 "non-exhaustive\|not covered"`
Expected: a compile error at `loc_method_label` in `render/map.rs` — `RoomHeading` not covered.

- [ ] **Step 3: Add the label arm**

In `crates/app/src/render/map.rs`, `loc_method_label`:

```rust
    match m {
        GlobalVar0 => "via status variable",
        PlayerParent => "via player object",
        StatusName => "via name match",
        NameOnly => "via name (unlinked)",
        RoomHeading => "via room heading",
    }
```

- [ ] **Step 4: Extend the label test**

In the existing `loc_method_label` test in `render/map.rs` (~line 2236, under `use zvm::location::LocationMethod::*;`), add:

```rust
        assert_eq!(loc_method_label(RoomHeading), "via room heading");
```

- [ ] **Step 5: Verify**

Run: `cargo test -p app --lib render::map:: 2>&1 | tail -5` and `cargo test -p zvm --lib location:: 2>&1 | tail -5`
Expected: both PASS (zvm enum change compiles; label test green).

- [ ] **Step 6: Commit**

```bash
git add crates/zvm/src/location.rs crates/app/src/render/map.rs
git commit -m "feat(map): add RoomHeading location method + label"
```

---

## Task 2: `AppGlk` Subheader room-heading capture

**Files:**
- Modify: `crates/app/src/glk_backend.rs` (`AppGlk` struct, `put_text_attr`, new methods)
- Test: `crates/app/src/glk_backend.rs` (tests module)

**Interfaces:**
- Consumes: `self.primary` (primary buffer window id), `GlkStyle::Subheader`.
- Produces: `pub fn take_room_heading(&mut self) -> Option<String>` (used by Task 3).

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/app/src/glk_backend.rs` (a text-buffer window must be opened first so `primary` is set; use the trait method via `GlkBackend`):

```rust
#[cfg(test)]
mod heading_tests {
    use super::*;
    use gvm::glk::{GlkBackend, GlkStyle, Rect as GlkRect, WinType};

    fn primary_backend() -> AppGlk {
        let mut b = AppGlk::new(80, 24);
        b.window_open(1, WinType::TextBuffer);
        b.window_layout(&[(1, WinType::TextBuffer, GlkRect { x: 0, y: 0, width: 80, height: 24 })]);
        b
    }

    // Feed a run via the colourless trait entry (delegates to put_text_attr).
    fn put(b: &mut AppGlk, style: GlkStyle, s: &str) {
        b.put_text(1, style, s);
    }

    #[test]
    fn subheader_line_is_the_heading() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Studio Apartment\n");
        put(&mut b, GlkStyle::Normal, "You climb out of bed.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("Studio Apartment"));
        // Drained: a second call with no new heading is None.
        assert_eq!(b.take_room_heading(), None);
    }

    #[test]
    fn last_subheader_wins_over_banner_title() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Coloratura");
        put(&mut b, GlkStyle::Normal, " by lynnea glasser\n");
        put(&mut b, GlkStyle::Subheader, "Inside the Cellarium");
        put(&mut b, GlkStyle::Normal, "A white structure.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("Inside the Cellarium"));
    }

    #[test]
    fn emphasized_and_header_are_not_headings() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Header, "Superluminal Vagrant Twin\n");
        put(&mut b, GlkStyle::Emphasized, "Knock.");
        put(&mut b, GlkStyle::Normal, "Prose.\n");
        assert_eq!(b.take_room_heading(), None);
    }

    #[test]
    fn menu_only_normal_text_has_no_heading() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "1) Yes\n2) No\n");
        assert_eq!(b.take_room_heading(), None);
    }

    #[test]
    fn heading_char_by_char_runs_accumulate() {
        // Games often emit one glk_put_char per character.
        let mut b = primary_backend();
        for ch in "War Chest".chars() {
            put(&mut b, GlkStyle::Subheader, &ch.to_string());
        }
        put(&mut b, GlkStyle::Normal, "\nThe battle.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("War Chest"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app --lib heading_tests 2>&1 | tail -20`
Expected: FAIL — `take_room_heading` does not exist (compile error).

- [ ] **Step 3: Add the accumulator fields**

In `crates/app/src/glk_backend.rs`, add to the `AppGlk` struct (after `primary`):

```rust
    /// Accumulator for the current run of `Subheader` text in the primary
    /// window (the Inform 7 room heading, captured char-by-char).
    heading_acc: String,
    /// The last completed `Subheader` line seen since the previous drain — the
    /// current room heading (`None` if this turn printed none).
    last_heading: Option<String>,
```

Initialize both in `AppGlk::new` (add `heading_acc: String::new(), last_heading: None,` to the struct literal).

- [ ] **Step 4: Capture on the primary window in `put_text_attr`**

In `put_text_attr`, before pushing to the buffer log:

```rust
    fn put_text_attr(&mut self, win: u32, style: GlkStyle, colour: StyleColour, s: &str) {
        if Some(win) == self.primary {
            self.capture_heading(style, s);
        }
        let (bits, fg, bg) = resolve_glk_colour(style, colour);
        let buf = self.buffers.entry(win).or_default();
        buf.log.push((bits, fg, bg, s.to_string()));
    }
```

- [ ] **Step 5: Add the capture/finalize/drain methods**

Add to the `impl AppGlk` block (near `take_transcript`):

```rust
    /// Feed one primary-window output run into the room-heading detector.
    /// Accumulates consecutive `Subheader` text; a newline or any non-`Subheader`
    /// run finalizes the current heading line. Keeps the LAST finalized line, so
    /// the banner title (printed before the room heading) is overwritten by it.
    fn capture_heading(&mut self, style: GlkStyle, s: &str) {
        if style == GlkStyle::Subheader {
            for ch in s.chars() {
                if ch == '\n' {
                    self.finalize_heading();
                } else {
                    self.heading_acc.push(ch);
                }
            }
        } else {
            self.finalize_heading();
        }
    }

    /// Promote the accumulated `Subheader` text (if any, trimmed non-empty) to
    /// the last-heading slot and clear the accumulator.
    fn finalize_heading(&mut self) {
        let line = self.heading_acc.trim().to_string();
        self.heading_acc.clear();
        if !line.is_empty() {
            self.last_heading = Some(line);
        }
    }

    /// Return and clear the last `Subheader` room heading captured since the
    /// previous call. Drained once per turn, alongside `take_transcript`.
    pub fn take_room_heading(&mut self) -> Option<String> {
        self.finalize_heading(); // flush a heading with no trailing separator yet
        self.last_heading.take()
    }
```

- [ ] **Step 6: Verify**

Run: `cargo test -p app --lib heading_tests 2>&1 | tail -10`
Expected: all 5 PASS.

- [ ] **Step 7: Guard against regressions in the transcript path**

Run: `cargo test -p app --lib glk_backend 2>&1 | tail -10`
Expected: PASS (existing `take_transcript`/render tests unaffected — capture is additive).

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/glk_backend.rs
git commit -m "feat(glulx): capture Inform Subheader room heading in AppGlk"
```

---

## Task 3: Thread the heading through `GlulxSession`

**Files:**
- Modify: `crates/app/src/glulx_session.rs` (`GlulxSession` struct, `new`, `finish_turn`, `current_location`, imports)
- Test: `crates/app/src/glulx_session.rs` (tests module)

**Interfaces:**
- Consumes: `AppGlk::take_room_heading` (Task 2), `LocationMethod::RoomHeading` (Task 1), `crate::roomid::synthetic_room_id`.
- Produces: `TurnResult.location = Some(room)` + `location_method = Some(RoomHeading)` on heading turns; `current_location()` returns the sticky room.

- [ ] **Step 1: Write the failing test (pure heading→room helper)**

Add to the tests module in `crates/app/src/glulx_session.rs`:

```rust
    #[test]
    fn heading_to_room_uses_synthetic_id() {
        let r = super::heading_to_room("Studio Apartment");
        assert_eq!(r.name, "Studio Apartment");
        assert_eq!(r.parent, 0);
        assert_eq!(r.number, crate::roomid::synthetic_room_id("Studio Apartment"));
        // Same name → same id (identity is name-based).
        assert_eq!(super::heading_to_room("Studio Apartment").number, r.number);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app --lib glulx_session::tests::heading_to_room 2>&1 | tail -10`
Expected: FAIL — `heading_to_room` does not exist.

- [ ] **Step 3: Add the helper + the sticky field**

In `crates/app/src/glulx_session.rs`:

Add imports at the top (extend the existing `use` lines):

```rust
use zvm::location::LocationMethod;
```

Add a field to `GlulxSession` (after `aux_dirty`):

```rust
    /// The current room, derived from the last Inform `Subheader` heading and
    /// held sticky across heading-less turns (examine/talk/failed-move).
    last_room: Option<LocationInfo>,
```

Initialize it in `new` (add `last_room: None,` to the struct literal), then seed it from the opening banner AFTER `session.refresh_screen();`:

```rust
        session.last_room =
            session.appglk().take_room_heading().map(|n| heading_to_room(&n));
```

Add the free helper (near `blank_screen`):

```rust
/// Build a name-based room snapshot from an Inform room heading. Glulx has no
/// readable object tree, so identity is the synthetic id of the normalized name.
fn heading_to_room(name: &str) -> LocationInfo {
    zvm::ObjectSnapshot {
        number: crate::roomid::synthetic_room_id(name),
        parent: 0,
        name: name.to_string(),
    }
}
```

- [ ] **Step 4: Update `finish_turn` to set location + method**

In `finish_turn`, after `self.refresh_screen();` and before building the `TurnResult`:

```rust
        if let Some(name) = self.appglk().take_room_heading() {
            self.last_room = Some(heading_to_room(&name));
        }
        let location = self.last_room.clone();
        let location_method = location.as_ref().map(|_| LocationMethod::RoomHeading);
```

Then in the `TurnResult { … }` literal, replace `location: None,` with `location,` and `location_method: None,` with `location_method,`.

- [ ] **Step 5: Update `current_location`**

Replace the body:

```rust
    fn current_location(&self) -> Option<LocationInfo> {
        self.last_room.clone()
    }
```

- [ ] **Step 6: Fix the now-stale "always None" test**

The existing test `introspect_and_location_are_none` (~line 547) asserts `current_location().is_none()`. `introspect()` still returns `None`; split the assertion so it no longer claims location is always None. Update it to assert only `introspect().is_none()` and rename to `introspect_is_none` (drop the location clause — location is now covered by Step 1 and Task 5).

- [ ] **Step 7: Verify**

Run: `cargo test -p app --lib glulx_session 2>&1 | tail -12`
Expected: PASS (helper test + updated introspect test).

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/glulx_session.rs
git commit -m "feat(glulx): report Subheader room via current_location + TurnResult"
```

---

## Task 4: Lock in that `RoomHeading` bypasses the NameOnly gate

**Files:**
- Test: `crates/app/src/session.rs` (tests module; no production change)

**Interfaces:**
- Consumes: `apply_turn` (unchanged), `LocationMethod::RoomHeading`.

- [ ] **Step 1: Write the test**

Add to the tests module in `crates/app/src/session.rs`, next to `apply_turn_gates_nameonly_until_first_real_room`:

```rust
    #[test]
    fn apply_turn_observes_roomheading_on_empty_map() {
        // Glulx rooms use RoomHeading (never NameOnly) precisely so the
        // NameOnly-empty-graph gate does NOT suppress the first Glulx room —
        // a Glulx game never produces an object-backed room to un-gate it.
        let mut m = Mapper::default();
        let result = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 333, parent: 0, name: "Orbiting Boony".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            diagnostics: vec![],
            location_method: Some(LocationMethod::RoomHeading),
            pending_io: None,
            timed_out: false,
        };
        apply_turn(&mut m, "", &result);
        assert_eq!(m.graph.current(), Some(333));
        assert_eq!(m.graph.rooms().count(), 1);
    }
```

- [ ] **Step 2: Verify**

Run: `cargo test -p app --lib session::tests::apply_turn_observes_roomheading 2>&1 | tail -6`
Expected: PASS (the gate keys only on `NameOnly`, so no production change is required — this test proves it).

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/session.rs
git commit -m "test(glulx): RoomHeading location is observed on an empty map"
```

---

## Task 5: Story-level verification (ignored, local `stories/`)

**Files:**
- Create: `crates/app/tests/glulx_room_detection.rs`

**Facts (verified):** `blorb` is already a dependency of `app` (`crates/app/Cargo.toml:18`) — no manifest change needed. Turn submission and location are `Engine` **trait** methods (`crates/app/src/engine.rs:341` `fn submit(&mut self, command: &str) -> TurnResult`; `current_location(&self) -> Option<LocationInfo>`), so the test must bring the trait into scope: `use app::engine::Engine;`. The session type is `app::glulx_session::GlulxSession`.

**Interfaces:**
- Consumes: `GlulxSession::new`, `Engine::submit`, `Engine::current_location`, `blorb::Blorb`.

- [ ] **Step 1: (no action — facts above are verified)** Extract the Glulx image with `blorb::Blorb` exactly as `crates/gvm/tests/accel_story_equivalence.rs::extract_glulx` does (shown in Step 2). Bring `use app::engine::Engine;` into scope to call `submit`/`current_location`.

- [ ] **Step 2: Write the ignored survey-backed tests**

Create `crates/app/tests/glulx_room_detection.rs`. Mirror `accel_story_equivalence`'s skip-when-absent pattern (`stories/` is gitignored). For each game, drive `GlulxSession` to its opening and assert `current_location()`:

```rust
//! Local-only Glulx room-detection verification (stories/ is gitignored; these
//! are #[ignore]d). Run: cargo test -p app --test glulx_room_detection -- --ignored --nocapture
use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

// Extract a Glulx image from a Blorb (or pass through a raw .ulx), like
// gvm/tests/accel_story_equivalence.rs::extract_glulx. Returns None for non-Glulx.
fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(stories_dir().join(name)).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) { return Some(bytes); }
    let b = blorb::Blorb::parse(bytes).ok()?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

// Boot a session at 80x24 with acceleration on. Returns None if the story is absent.
// (Use the real GlulxSession + its Engine::submit; import paths per Step 1.)
```

Then a table-driven test asserting the starting room:

```rust
#[test]
#[ignore]
fn starting_rooms_resolve() {
    let cases = [
        ("FooFoo.gblorb.blorb", "Studio Apartment"),
        ("Superluminal_Vagrant_Twin.gblorb.blorb", "Orbiting Boony"),
        ("Sub_Rosa.gblorb.blorb", "Leathery Cliff"),
        ("Dr Ludwig and the Devil.gblorb", "Laboratory"),
        ("TAKE.gblorb", "War Chest"),
    ];
    for (file, want) in cases {
        let Some(image) = glulx_image(file) else { continue };
        let mut s = app::glulx_session::GlulxSession::new(image, 80, 24, true)
            .expect("GlulxSession::new");
        let got = s.current_location().map(|r| r.name);
        assert_eq!(got.as_deref(), Some(want), "{file}: starting room");
    }
}
```

Notes for the implementer:
- Coloratura ("Inside the Cellarium") and Magpie ("Station") reach their first room only after a few intro key-presses in the survey; include them only if a small fixed key sequence via the session's key-submission entry gets there deterministically — otherwise omit rather than guess.
- Add a second `#[ignore]` test `pregame_menus_have_no_room` asserting `current_location().is_none()` right after `new()` for `Zozzled.gblorb.blorb`, `Brain_Guzzlers_from_Beyond!.gblorb.blorb`, and `And_Then_You_Come_to_a_House.gblorb.blorb` (each skipped individually when absent). These sit on a menu / setup question, which emits no `Subheader`.

- [ ] **Step 3: Run the ignored tests locally**

Run: `cargo test -p app --test glulx_room_detection -- --ignored --nocapture 2>&1 | tail -20`
Expected: PASS for present stories (FooFoo → "Studio Apartment", Superluminal → "Orbiting Boony", …; pre-game games → None). Absent stories skip.

- [ ] **Step 4: Confirm the default tier stays green without stories**

Run: `cargo test -p app 2>&1 | grep "test result"`
Expected: all green (the new tests are `#[ignore]`d).

- [ ] **Step 5: Commit**

```bash
git add crates/app/tests/glulx_room_detection.rs
git commit -m "test(glulx): story-level room-detection verification (ignored)"
```

---

## Final verification (whole branch)

- [ ] `cargo test -p zvm 2>&1 | grep "test result"` — green (enum change).
- [ ] `cargo test -p app --lib 2>&1 | grep "test result"` — green (all unit tests).
- [ ] `cargo test -p app --test glulx_room_detection -- --ignored --nocapture` — FooFoo/Superluminal resolve; pre-game games None.
- [ ] Manual smoke (optional): `cargo run -p app -- stories/FooFoo.gblorb.blorb`, confirm the map shows "Studio Apartment" and the indicator reads "via room heading".

## Self-Review notes (for the executor)

- The `NameOnly` gate in `apply_turn` is intentionally untouched; Task 4 proves `RoomHeading` bypasses it. Do NOT broaden the gate.
- Do NOT add a grid-status fallback (Strategy C) — out of scope.
- Do NOT attempt same-name disambiguation — separate TODO.
- Method-name consistency: the enum variant, the `heading_to_room` mapping, and the `"via room heading"` label must all refer to `RoomHeading`.
