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
| `app` | The `babelmap` TUI binary (ratatui + crossterm): play loop, live map rendering, debug inspector, all interactive features. |
| `zvm-cli` / `gvm-cli` / `scott-cli` | Standalone DOS-style command-line players (no map): save/restore, single-key input, terminal-bell bleeps — and, piped, a clean deterministic harness for testing/scripting. `zvm-cli` declines graphical **v6** stories at load: they drive a windowed display it cannot present, and every one of them runs away at its first input prompt. `zvm` itself supports v6 fully — play those in `babelmap`. |
| `cli-host` | The plumbing those three CLIs share: terminal escapes, the input/EOF rule, an RAII terminal restore, and `--help`/`--version`. Not the renderers — see below. |
| `blorb` | Blorb container parsing — bundled story, cover art, and sound/image resources. |
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

## See also

- [Interactive-fiction standards babelmap implements](standards.md) (Z-Machine,
  Glulx, Glk, Quetzal, Blorb, Treaty of Babel).
- Design/strategy notes under [`docs/design/`](design/).
