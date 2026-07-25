# Saves & persistence

[← back to README](../../README.md)

Quit mid-dungeon and come back to exactly where you stood — same room, same
inventory, same map, same screen. babelmap layers a few different kinds of save
on top of each other so you never lose progress, whether you save deliberately,
let the game save itself, or never save at all.

- **`.babelmap` Save States — freeze the whole session, not just the game.**
  Ctrl+S (`/save-state`) snapshots everything into one self-contained file: the
  VM's exact state, the map you've drawn, the on-screen windows, and the
  transcript. Ctrl+R (`/restore-state`) thaws it back. It's the emulator's own
  save-anywhere snapshot — engine-neutral, and the game never knows it happened,
  so you can bail out mid-sentence or mid-puzzle and land right back in it.
- **Named slots.** Keep as many Save States as you like. The saves-manager modal
  lists them (Enter to load, `s` to save-as, `d` to delete, `i` to import), each
  slot showing its name, turn count, and timestamp.
- **Bring saves in from other interpreters — standard Quetzal.** Point the saves
  manager's built-in file browser at a `.qzl`/`.sav` game save from `dfrotz` (or
  any other interpreter), import it, and keep the map you've already accumulated.
  It works the other way too: when a story runs its own `SAVE`, babelmap writes a
  bare, portable standard Quetzal `.qzl` — the same file any other interpreter
  reads — for every Z-machine version, right down to v3 (Zork-era) branch-form
  `@save`/`@restore`. That interoperability is golden-tested against `dfrotz` in
  both directions (`scripts/gen-interop-goldens.sh`, or
  `cargo test -p zvm --test save_interop -- --ignored`).

  This game-written `.qzl` is a genuinely different file from a `.babelmap` Save
  State: the `.qzl` holds VM state only, while the Save State also carries the
  map, screen, and transcript. Glulx games likewise write a real, standard
  Glulx-Quetzal in-game save — VM state only, the same shape as the Z-machine's
  `.qzl`. Its round-trip is verified internally; Glulx *cross-interpreter* save
  interop isn't golden-tested yet (tracked in SQ-0229).

  One practical consequence for graphical (v6) stories: an in-game `restore`
  into a *fresh* session brings back the game state but not your scrollback —
  the Quetzal format simply carries no transcript, so neither the prose history
  nor the inline artwork woven through it can come back (every interpreter
  behaves this way). Within a running session your scrollback — art included —
  is untouched by an in-game `restore`. To get history back across a relaunch,
  resume through a Save State or auto-load first (which restores the transcript
  and its inline images), then `restore` in-game if you need a different
  in-game save.
- **Auto-save and auto-load.** Turn on auto-save and babelmap snapshots after
  every turn; leave auto-load on (the default) and launching a story drops you
  straight back where you quit, map and all. Both are configurable — start fresh
  while keeping the accumulated map by switching auto-load off.
- **Glulx external files just persist.** A Glulx game's own Glk files — its
  transcripts, command recordings, and data files — live in an in-memory VFS that
  auto-persists per story across sessions, so they survive a plain quit with no
  explicit save (Kerkerkruip's scores and preferences stick, for instance). When
  a game asks the *player* where to write (`create_by_prompt`), babelmap prompts
  for a name; when it asks which file to read, it shows a picker of the story's
  existing files. These files ride inside `.babelmap` Save States too. A Glulx
  game's **own** fixed-name saves (`create_by_name` — e.g. Counterfeit Monkey's
  init cache, autosave, and undo slots) are written and read **silently**, with no
  prompt, and stay hidden from the player saves list; because they persist per
  story, a relaunch auto-restores them so the game skips its long init (SQ-0296).
  → [persistence model](../persistence.md)
- **Rewind, replay, resume.** Switch on `record_turn_history` and babelmap keeps
  a per-turn history — each turn's game save plus a snapshot of the map and
  transcript — inside the `.babelmap` archive. Open the replay modal (the leader
  key then `h`, or `/open-history`) and step or auto-play through every past turn
  with the map reconstructed exactly as it looked at that moment, then resume the
  game from any earlier turn. It's undo that reaches back further than the game's
  own UNDO — and survives across sessions.
