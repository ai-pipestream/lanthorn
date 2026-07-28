# Standards & specifications

babelmap is a from-scratch implementation of the interactive-fiction virtual machines and
file formats. This page collects the authoritative specifications it implements against, so
you can check our behaviour against the source of truth. Section references (e.g. "Glk spec
§4.4") that appear throughout the code point into these documents.

## Virtual machines

- **Z-Machine Standards Document 1.1** — the Infocom/Inform Z-code VM that the `zvm`
  crate implements (versions 3, 4, 5, 6, 7, and 8): opcodes, object model, text encoding,
  the v3/v5 status line, the v6 windowing/graphics model, and the save/restore/undo model.
  <https://inform-fiction.org/zmachine/standards/z1point1/index.html>

- **Glulx Specification (3.1.3)** — Andrew Plotkin's 32-bit VM for large Inform 7 games,
  implemented by the `gvm` crate: instruction set, memory map, the accelerated-function
  (`@accelfunc`) opcodes, and `@save`/`@restore` (Glulx spec §1.8).
  <https://eblong.com/zarf/glulx/Glulx-Spec.html> · [home](https://eblong.com/zarf/glulx/)

- **Scott Adams / ScottFree `.dat`** — the classic Scott Adams adventure format, implemented
  by the `scott` crate. There is no formal standards document; the de-facto reference is Alan
  Cox's **ScottFree** interpreter and the tokenized `.dat` database it reads (header counts,
  actions, verbs/nouns, rooms, messages). This format is entirely its own — **not** built on
  Glk or Quetzal — with its own text display and its own save/restore.
  <https://ifarchive.org/indexes/if-archive/scott-adams/>
  (Scott Adams games and ScottFree ports)

## I/O layer

- **Glk API Specification (0.7.6)** — the windowing/stream/event abstraction the **Glulx**
  engine drives its display through (text-buffer and text-grid windows, graphics windows,
  line/char and timer/mouse/hyperlink input events, file streams). babelmap projects the Glk
  window tree onto its terminal UI. Only `gvm` uses Glk; the Z-machine and Scott Adams engines
  render through their own native display models and converge with Glulx only at babelmap's
  neutral `ScreenModel` (see [architecture](architecture.md)). Referenced in-code at e.g. §3.3
  (window sizing), §4.2/§4.4 (event model), §11.2 (file streams).
  <https://eblong.com/zarf/glk/Glk-Spec-076.html> · [home](https://eblong.com/zarf/glk/)

- **Gargoyle `garglk_*` extensions** — the de-facto colour and reverse-video Glk extensions
  (`garglk_set_zcolors`, `garglk_set_reversevideo`, `gestalt_GarglkText`) that babelmap
  recognises so games written for Gargoyle render their per-span colours.
  <https://github.com/garglk/garglk>

## File formats

- **Quetzal** — the cross-interpreter saved-game standard. babelmap reads and writes Quetzal
  so an in-game `save` can be restored on another interpreter (and vice versa); the Glulx
  save path writes a Glulx-flavoured Quetzal.
  <https://inform-fiction.org/zmachine/standards/quetzal/index.html>

- **Blorb** — the resource-packaging format that bundles a game's executable together with its
  cover art, images, and sounds. babelmap reads Blorb (`.zblorb`/`.gblorb`/`.blorb`) to
  extract the story file and its graphics/audio resources.
  <https://eblong.com/zarf/blorb/Blorb-Spec.html> · [home](https://eblong.com/zarf/blorb/)

- **Treaty of Babel (revision 12)** — the community agreement on story identification. babelmap
  computes each story's **IFID** per the Treaty to key its per-game saves, maps, and
  bibliographic lookups.
  <https://babel.ifarchive.org/babel.html> · [specs repo](https://github.com/iftechfoundation/ifarchive-if-specs)

- **EA IFF 85 (Interchange File Format)** — the chunked container format that Blorb and
  Quetzal are both built on (`FORM`/chunk structure). babelmap's Blorb and Quetzal parsers
  implement this chunk layout.
  <https://en.wikipedia.org/wiki/Interchange_File_Format>

## Where we knowingly differ

Almost everything above is implemented to the letter. One place is not, and it is deliberate:

- **ZMSD §8.8.4.1 — Version 6 `split_window` placement.** The spec tiles windows 0 and 1
  together, so that "window 1 has the given height and is placed at the top left, while window
  0 is placed just below it". babelmap resizes both windows exactly as described but leaves
  window 0's **origin** where the game put it, rather than pushing it down below window 1.
  Zork Zero's boot splash is why: it splits the full screen height to hang its title art in
  window 1, and relocating window 0 for the duration would drag the story viewport across the
  picture — and leave it there, since the game only ever restores window 0's *size*. Geometry
  reporting is unaffected: `get_wind_prop` still answers with each window's true position and
  size, so a game that measures the screen sees exactly what the spec promises.

---

*Spec versions cited above were current as of July 2026 (Z-Machine Standard 1.1; Glulx 3.1.3;
Glk 0.7.6; Treaty of Babel rev 12). The linked pages are the canonical, versioned sources —
follow them for any later revision.*
