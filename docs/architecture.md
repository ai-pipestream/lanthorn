# Architecture

[← back to README](../README.md)

babelmap is a Rust workspace. Two ideas shape it: the **interpreter and the
mapper are decoupled** (a VM reports *where you are*; the mapper turns the stream
of locations and movements into a spatial graph, knowing nothing about the
engine), and **three different story formats render through one neutral screen
model** so a single renderer draws them all.

## Crates

| Crate | Responsibility |
|-------|----------------|
| `zvm` | A from-scratch Z-machine virtual machine — executes story files, standard Quetzal save/restore. Zero-dependency. |
| `gvm` | A Glulx virtual machine (Glk I/O) for modern Inform 7 games — accelerated Inform veneer, full float opcodes. Zero-dependency. |
| `scott` | A Scott Adams (ScottFree `.dat`) virtual machine for the classic text adventures. Zero-dependency. |
| `mapper` | A VM-agnostic map model: rooms, connections, layered 2-D layout, overlap removal, edge routing. Serializable. |
| `app` | The `babelmap` TUI binary (ratatui + crossterm): play loop, live map rendering, debug inspector, all interactive features. `assets.rs` is its counterpart to `blorb::medium`: **one enumeration of every place a story's files can live** — the directory beside it, and the volume it was mounted out of — so a caller looking for a game's assets filters that one list instead of learning that disk images exist. `launch_options::discover_art_candidates` is the only filter over it today; before SQ-0843 it was a bare `read_dir`, which is why a Macintosh disk's two picture archives were unpickable while `blorb` had been reading them for a week. A new asset **source** is an arm in `assets::files`, a new asset **kind** is a filter beside that one, and a new disk **format** is still just a row in `blorb::medium::FORMATS`. `disk_set.rs` is a second small enumeration in the same spirit — **which files are volumes of one multi-disk release** — and it answers from filenames alone, never opening a disk, because the question is about how a collection was pressed rather than what is on it. It feeds two callers: `picker::StorySource` (what a launch argument *means* — a directory, or the release a named volume belongs to) and `picker::scan_stories` (which folds a set's duplicate builds together by IFID, since the ST shelf carries 39 stories for 33 games). The volume label was weighed as the grouping signal and rejected on measurement, not taste: nine of the corpus's volumes report none at all, and Zork Zero's two DOS presses both label their first disk `ZORK0 1`, so it would leave one family ungrouped and merge the one pair the filename rule correctly separates (SQ-0844). |
| `zvm-cli` / `gvm-cli` / `scott-cli` | Standalone DOS-style command-line players (no map): save/restore, single-key input, terminal-bell bleeps — and, piped, a clean deterministic harness for testing/scripting. `zvm-cli` declines graphical **v6** stories at load: they drive a windowed display it cannot present, and every one of them runs away at its first input prompt. `zvm` itself supports v6 fully — play those in `babelmap`. `zvm-cli` also opens an original release disk image — **every format `blorb` reads, without naming one of them** (`blorb::medium` mounts it, so this costs no dependency) — and picks between several stories on one disk with a startup menu or `--story <n|name>`; the medium also sets the interpreter number it advertises, exactly as the TUI's does, with `-I` still overriding — see [the interpreter's disk-image notes](features/interpreter.md#the-command-line-player-takes-a-floppy-too). A v6 disk still gets the v6 refusal rather than a disk error: the mount worked, the renderer is what is missing. |
| `cli-host` | The plumbing those three CLIs share: terminal escapes, the input/EOF rule, an RAII terminal restore, and `--help`/`--version`. Not the renderers — see below. Since SQ-0850 it also owns the one thing the CLIs share with the **TUI**: `storage.rs`, which answers *what do I call this game's save directory* for every host, and the `titles.rs` catalogue the readable half of that name comes from. `app` depends on it for exactly that, because a story taken off a disk image is keyed by its own release and serial and two implementations of that rule would be two directories. |
| `blorb` | Blorb container parsing — bundled story, cover art, and sound/image resources — plus the release-media readers beside it: Infocom's native picture archives, Amiga `.adf` floppies (`adf.rs`), Macintosh DiskCopy 4.2 / HFS disks (`hfs.rs`), DOS **and** Atari ST floppies (`fat12.rs` — one FAT12 reader for both, because GEMDOS put its BPB at the DOS offsets; the machine is decided by whether the boot sector opens with an x86 jump, which DOS's load protocol requires and TOS has no use for), and Apple II ProDOS disks (`prodos.rs` — a `2IMG` wrapper whose declared data length reads zero on every image in the corpus, so the block count is the fallback; then seedling/sapling/tree/extended files, sparse blocks and nested directories). All hand-rolled; the crate takes no dependencies. Beside the filesystems sits `infocom_packed.rs`, which is not one: the Apple II press of *Arthur* and *Journey* stores no story file at all but a **packed volume** — an index in block 0 of the first `.D1` segment, then per-segment runs mapping story pages to blocks scattered across every floppy in the set, so reading is a scatter-gather rather than a file read (SQ-0852). It takes named byte blobs rather than a `Volume` because the same index addresses the raw 5.25" pressings of *Shogun* and *Zork Zero*, which carry no filesystem whatsoever; and it assembles and then **verifies the story's own header checksum** before handing anything back, because a wrong page map yields a file just as plausible as a right one. `medium.rs`'s provided `Volume::stories` asks it on every format, so a story that is not a file is still a story on the list. `medium.rs` is the seam on top, and it is the **only place in the workspace that names a disk format**: a `FORMATS` table of one row per format, a `Volume` trait each reader implements by delegation, and a `MountedDisk` every front-end holds. Ask it whether bytes are an image, open them, list the stories on the volume, take the one to play, take the disk's own artwork, name the container for the picker, and get the Z-machine interpreter number the machine implies. Detect and mount walk the same table, so a format babelmap can recognise is a format it can open — the guarantee that was missing when `zvm-cli` detected an Amiga floppy and refused a Macintosh disk `blorb` had read for a month (SQ-0840). The row also carries the filename extensions a directory scan pre-filters on, which is the newest column and the one that had to be retrofitted: the TUI's story picker kept its own list, never heard about the DOS and ST rows, and left a shelf of mountable `.ima` and `.st` floppies out of the story list for two quests (SQ-0849). Extensions decide nothing — content still does — they only say which files are worth opening. Adding a format is a row here plus the reader it names, and every front-end gains it in the same commit — DOS and the Atari ST landed as **two rows over one reader**, which is what the row/reader split is for, since they are one filesystem and two machines, and ProDOS then landed as one row over one new reader with nothing outside `blorb` touched at all (SQ-0836); the interpreter-number default lives in the same row for the same reason two copies of "an `.adf` means interpreter 4" went stale in one place and not the other (SQ-0839). An explicit number always outranks it. |
| `audio` | Sound playback (rodio) — synthesized bleeps and sampled AIFF / Ogg / ProTracker MOD. |
| `buildinfo` | A tiny zero-dep helper: a `build.rs` that stamps the git commit hash into non-release build versions. |

The crates layer `zvm`/`gvm`/`scott` → `mapper` → `app`; the CLIs are thin VM
front-ends. The mapper has **no dependency on any VM**, so layout logic can be
tested in isolation, and the VM crates stay **zero-dependency** (image/audio/
resource types live in `app`, `blorb`, and `audio`).

### What `cli-host` does and does not share

The three CLIs share their *plumbing* and keep their *renderers*. The line is
drawn where it is because of what actually went wrong. Five escape helpers were
byte-identical in `zvm-cli` and `gvm-cli`, which was merely untidy — but the same
stdin-EOF bug (a 0-byte read taken for a blank command, so the game is fed a
fabricated newline forever) shipped **three** times: fixed in `zvm-cli`'s char
path long ago, still live in `gvm-cli` until SQ-0604, and still live in
`zvm-cli`'s own *line* path until SQ-0605 found it. Three copies, three chances
to get it wrong, and the terminal was left un-restored on the paths nobody was
thinking about.

So `cli-host` owns: the escape sequences, [`HostMode`] (may we emit escapes? may
we take over line editing?), the EOF-honest readers, `TerminalGuard` (restores on
every exit *including* a panic), and `--help`/`--version`.

It also owns the **save-directory key** — and that one is shared with `app`, which
is otherwise no CLI at all. The reason is the same drift argument one layer up:
the rule now has a case in it (a story mounted out of a disk image keys on its
release and serial, not on the image's filename, because one compilation carries
six games) and a second copy of a rule with a case in it is a second answer
waiting to happen. `app::storage` re-exports it rather than restating it.

It owns none of the drawing. `gvm-cli/glk_term.rs` and `zvm-cli/screen.rs` have
essentially no logic in common, and `scott-cli` — which emits no escape sequences
at all — would only pay for machinery it does not need. That last property is
load-bearing rather than incidental, so the guard comes in two flavours and
`scott-cli` takes the one that restores raw mode and emits nothing.

[`HostMode`]: ../crates/cli-host/src/mode.rs

## Three engines, one renderer — and Glk only for Glulx

All three VMs implement one `Engine` trait whose `screen()` returns an
engine-neutral **`ScreenModel`** (a window tree the app knows how to draw). The
one generic renderer draws every engine from that model. But *how* each engine
arrives at its `ScreenModel` differs, and this is a deliberate design decision:

- **Glulx (`gvm`) uses Glk.** A Glulx game drives Glk display calls (open/close/
  arrange windows, `put_text`, `grid_put`, …). The app's `AppGlk`
  (`app/src/glk_backend.rs`) records those calls and *projects them* onto the
  `ScreenModel`. Glk lives entirely in this **app-layer translator** — `gvm`
  itself just makes the calls; the VM crate carries no terminal or Glk types.
- **Z-machine (`zvm`) is native.** `zvm` has its **own** `ScreenState` + `Output`
  model (v3 status line, v4+ cursor-addressed upper window). The app *mirrors*
  that state into the same `ScreenModel` — no Glk involved.
- **Scott Adams (`scott`) is native.** The `scott` VM has no screen model of its
  own at all; the app builds a `ScreenModel` directly from its output. No Glk.

So **Glk is confined to the Glulx path.** Z-machine and Scott are implemented
against their own I/O models and converge with Glulx only at the neutral
`ScreenModel` layer.

### Why confine Glk to Glulx

- **Spec-faithful.** Glulx's I/O *is defined* in terms of Glk — using Glk there
  matches the standard. The Z-machine and Scott Adams formats are **not** defined
  against Glk; they have their own display models. Implementing each format's I/O
  the way its spec describes keeps every engine honest.
- **No leaky abstraction.** Routing the Z-machine's cursor-addressed upper window
  or Scott's fixed two-window layout *through* Glk's windowing model would be an
  impedance mismatch — format-specific behavior would be distorted or lost. Each
  engine keeps its exact semantics.
- **Unification at the right layer.** Cross-engine render unification is banked at
  the `ScreenModel`, so one renderer serves all three — *without* forcing a single
  I/O library onto formats that don't use it.
- **Smaller, self-contained VMs.** `zvm` and `scott` don't pull in a Glk layer
  they'd never use, so they stay zero-dependency and easy to reason about; Glk
  code lives in exactly one place (`app`'s Glulx backend).

## Graphical v6: a fourth window kind on the same model

Graphical Z-machine **v6** stories (*Zork Zero* and kin) don't fit the plain
window tree — pictures and text share one pixel-addressed screen. Rather than
build a second renderer, v6 gets one more `ScreenModel` node,
`WinNode::Layered`, carrying the game's windows z-ordered background-first:
`session.rs`'s `v6_screen_model` builds it from `zvm`'s native v6 window
state; `render/screen.rs`'s `Layered` arm composites it — per-cell without an
image protocol, or (with one) as one native-pixel-space canvas assembled by
`render/v6_layout.rs`'s classification/geometry helpers and drawn by
`render/graphics.rs::draw_v6_canvas`. Same generic renderer, same neutral
model — v6 is a fourth leaf kind, not a parallel pipeline. See [Graphical
v6](features/v6-graphics.md) for what that composite looks like from the
player's side.

## Input: a suspend/resume handshake

Input is engine-neutral too. A VM's `step()` returns a request —
`NeedLine` / `NeedChar` / `NeedEvent` — and the host resolves it with
`supply_line` / `supply_char` / `supply_filename`. The values are neutral (no
terminal types cross the boundary), so the same host loop drives every engine and
the CLIs can feed input from a pipe for deterministic testing.

## Reading back the bytes we actually emit

Every other harness in the repo renders into a ratatui `Buffer` and asserts on
cells — babelmap's own model of the screen. None of them can see the *stream*,
so a defect that is right in the model and wrong on the glass is invisible to
all of them. `crates/app/tests/pty_stream/` closes that gap: it runs the real
`babelmap` binary under a pty, plays the part of the terminal, and decodes the
escape bytes that come back.

Five parts, and the split matters for Windows:

| file | what it is |
| --- | --- |
| `tests/pty_stream/driver.rs` | The pty (`posix_openpt` + `libc`, no new dependency), the terminal-query answers, the keystroke script. **Unix only** — a pty is. |
| `tests/pty_stream/decode.rs` | Bytes → named sequences → a screen model: cursor, SGR, kitty APC commands, U+10EEEE placeholder cells. **Portable**, and unit-tested on every platform. |
| `tests/pty_stream/oracle.rs` | The same bytes through a real terminal emulator — see [the placement oracle](#a-second-reader-for-the-same-bytes-the-placement-oracle). **Portable.** |
| `tests/pty_stream/raster.rs` | That resolved screen drawn as a PNG — see [looking at the frame](#looking-at-the-frame-the-rasteriser). **Portable.** |
| `tests/pty_stream/mod.rs` | The report — protocol verdict, uploads, placement rects, a background map, and the finding. |

**It verifies the protocol first, and says so out loud.** babelmap picks its
graphics backend from `Picker::from_query_stdio`, which asks the terminal three
questions before the UI starts and falls back to half-blocks when nobody
answers. A bare pty answers nothing, so a naive harness silently measures the
half-block path and every number it produces is worthless. The driver answers
the kitty capability query, DA1, `CSI 16 t` (the cell size — not cosmetic: v6 art
is scaled by pixel and placed by cell) and the OSC 10/11 colour probes, and the
capture then *proves* kitty from the stream rather than from hope: no APC `_G`
traffic means no kitty, and the test refuses to go on.

**What it can tell apart that nothing else can.** A kitty placement is virtual:
the upload (`a=T,U=1`) says how big the image is and nothing about where it
goes, and the position comes from the placeholder cells printed afterwards. So
"this row is that colour" has two entirely different causes — an image is placed
over it, or a background was painted into the cells — and they are different
bugs with different fixes. The decoder builds a grid, marks which cells carry
placeholders, and the report's background map names each row's runs with
`(image)` on the ones an upload covers. SQ-0747's flank-panel fill was settled
this way in one run: the overrun rows were **painted cells, not a placement
rect**.

Ad hoc:

```sh
cargo build -p app                       # the harness drives the REAL binary
cargo run -p app --example pty_capture -- \
    --story "stories/Journey - The Quest Begins.adf" \
    --size 117x64 --keys "wait:1500,cr,wait:800,cr,wait:800,cr,wait:1200" \
    --out /tmp/journey.stream.txt
```

`--size` is the terminal, not the story pane: at `117x64` with the map hidden
(the default here) the frame border and the help row leave the story pane the
`115x61` a finding is usually quoted at. Exit status 3 means the run did not
negotiate kitty. `cargo run -p app --example pty_capture -- --help` lists the
rest.

From a test: `cargo test -p app --test pty_emitted_stream -- --nocapture`, which
writes its report to `target/pty-capture/`. It asserts that the harness measured
the right backend and could read a placement back, and deliberately does **not**
pin any particular defect's presence — a test that fails when a bug is fixed is
a trap for the next person, so the image-versus-paint reading is printed as a
finding instead. On Windows the whole thing compiles and the decoder's unit
tests run; the pty case is an explicit skip.

Its complement is `/dump-cells` ([Graphical v6](features/v6-graphics.md)), which
dumps the same screen from the *inside*: that shows what we computed, this shows
what we sent. Disagreement between the two is the interesting case.

## A second reader for the same bytes: the placement oracle

`pty_stream/decode.rs` is *our* reading of the emitted stream — a hand-rolled
decoder that shares whatever assumptions we built it with. When the model
looks right (Layer 1) and the stream also looks right by our own reading
(Layer 2) but the screen is still wrong, the next question is whether our
reading of the stream is itself the bug. `crates/app/tests/pty_stream/oracle.rs`
(SQ-0764) answers that by resolving the same captured bytes through
`qwertty-term-vt`, a dev-dependency that is a pure-Rust port of Ghostty's
terminal core (tracking upstream Ghostty commit `2da015cd6`, including the
297-entry diacritic table matching kitty's published list) — one dependency,
no build script, builds on all three platforms. Reach for it for placement
lifetime, z-order, overlap, stale placements, missing deletes, and anything
turning on the unicode-placeholder continuation rules our decoder doesn't
model.

**It's a port, not Ghostty.** `qwertty-term-vt` tracks Ghostty's algorithm
faithfully enough to answer "does this placement cover these cells" — but a
port can diverge from what a real terminal does in ways nobody's hit yet.
Before writing up a user-visible bug on the oracle's word alone, eyeball it
on a real terminal too.

**The two decoders name images differently.** Ours keys an image by the low
24 bits of the placeholder's foreground colour; the oracle keys it by the
full 32-bit `i=` value (`full = low24 | (high_byte << 24)`). Comparing a
babelmap-side id against an oracle-side id means masking the oracle's down to
the low 24 bits first, not comparing them raw.

**The two decoders agree on image coverage — now.** They didn't when the
oracle landed: ours attributes a cell to an image by foreground colour alone
and doesn't model the diacritic continuation rule, so it counted 33 runs of
orphaned placeholder cells a real terminal declined to draw. That was
SQ-0772, and it was babelmap's bug, not the harness's: virtual placements
were emitted as one anchored cell per row plus bare continuations, invisible
to ratatui's damage model, so a later frame could destroy the anchor and
strand the rest. Every placeholder cell now carries its own row, column and
id high byte and lives in the buffer like any other content, and the real
capture asserts agreement on *both* axes. Ours still can't read a high byte
(see above), so a disagreement there remains an id-masking question, not a
coverage one.

**A stronger oracle exists in principle but isn't built.** For literal
Ghostty ground truth (not a port of it), `libghostty-vt` — Ghostty's own C
library — is reachable in theory, but only as a prebuilt artifact. Building
it from source needs zig plus a full ghostty source checkout, which drags in
the entire GUI dependency graph (sentry, imgui, freetype, glslang, …) even
to get the headless VT core, and it doesn't build at all on macOS 26 with the
pinned toolchain. The viable route, not yet set up, is a GitHub Actions
matrix (IPv4-only runners, so no fetch-wall failures) that publishes
`libghostty-vt.a` plus headers and the generated `.pc`, consumed on a dev
machine through the `-sys` crate's `pkg-config` feature — which skips zig
entirely. Full findings live on SQ-0764; don't re-derive this, extend it.

## Looking at the frame: the rasteriser

Everything above answers questions *about* a frame. `pty_stream/raster.rs`
(SQ-0775) draws it. The oracle already resolves a capture to a cell grid with
per-cell colours plus every placement's source rect, destination size and
position; the rasteriser composites that into an RGBA canvas at the capture's
own cell size and writes a PNG. Development happens over ssh as often as not,
and half the render quests in the tracker end in "the user must go look at it" —
this turns that into "here is the picture, is this right?", and a before/after
pair makes a render change reviewable with no terminal at all.

```sh
cargo run -p app --example pty_capture -- \
    --story "stories/Journey - The Quest Begins.adf" \
    --size 117x64 --keys cr,wait:800,cr,wait:800,cr \
    --out /tmp/j.txt --png /tmp/j.png
```

A before/after pair is one more flag, not a second mode: capture the old build
to a PNG, then run the new one with `--png-diff /tmp/before.png --png
/tmp/pair.png` and the two frames come back side by side with a divider between
them.

**It is not a screenshot, and the difference is not cosmetic.** Text is drawn
with the repo's own 8×8 bitmap font (`render/bitfont.rs`, the one the v6 pixel
composite uses), nearest-neighbour-scaled to fill each cell: no real font
metrics, no hinting, no ligatures, no italics, no bold. It is an oracle for
**layout, art placement and colour** — where the panes are, where the art
landed, what was painted under it, which of two overlapping things won — drawn
with our glyphs from what Ghostty's *algorithm* resolved. Judge geometry from
it; never judge typography from it. Two more honest limits: cells the app never
painted show the emulator's own default background (palette entry 0, Ghostty's
`#1D1F21`) rather than whatever the real terminal answered the OSC 11 probe
with, because the capture only sees the app→terminal direction; and a
below-background placement (kitty `z < -1073741824`) is bucketed on the z the
*renderer* sorts by, which upstream hardcodes to `-1` for every virtual
placement whatever the client asked for.

**It refuses to hide the bug it was built beside.** Each placement is
rasterised from its OWN resolved source rect, one draw per resolved placement,
never from the aggregated cell rect. A virtual placement resolves one entry per
screen row, and an orphaned run redraws the image's *first* row down the whole
rect (SQ-0772) — sampling per draw means the picture shows that as the banded
smear it is on the glass. A rasteriser that drew each image once into its
bounding box would render a clean, plausible, wrong picture of exactly the
defect worth seeing.

The tests are in `tests/pty_oracle.rs`'s `raster` module: hand-authored streams
whose expected picture can be stated exactly, asserting **colours at
coordinates** — a PNG writer's obvious failure mode is emitting a plausible
blank, and "a file appeared" accepts one.

## See also

- [Interactive-fiction standards babelmap implements](standards.md) (Z-Machine,
  Glulx, Glk, Quetzal, Blorb, Treaty of Babel).
- Design/strategy notes under [`docs/design/`](design/).
