# Graphical Z-machine v6

[← back to README](../../README.md) · see also [Interpreter](interpreter.md) · [Customization](customization.md)

Z-machine **v6** is Infocom's graphical story format — the one behind *Zork
Zero*, *Shogun*, *Journey*, and *Arthur*. It splits the screen into pixel-
addressed windows, draws pictures at exact coordinates, and expects the
interpreter to composite it all into one illustrated page. babelmap's `zvm`
implements the v6 windowing and picture opcodes needed to run that model, and
the app renders the result. The depth below is verified against *Zork Zero*,
whose full frame — banner, side columns, the per-room exit compass, and
in-text illustrations — renders faithfully; the same engine and opcode set
targets the format's other titles, but they haven't been played through end
to end in this repo yet.

## The game lays itself out — you just answer its questions

v6 games don't hard-code their own layout. At boot, Zork Zero (and its
siblings) queries `picture_data` on a handful of pictures that are never
actually drawn — they exist purely to answer "how big is this thing," and the
game uses the answer to position its banner, columns, and compass. Those
pictures are Blorb `Rect` chunks: an 8-byte, dimension-only placeholder (width
then height, big-endian) with no pixel data at all. babelmap recognizes a
`Rect` chunk and answers `picture_data` straight from it, which is exactly the
mechanism these games rely on — it isn't a general Blorb image feature, it's a
placement protocol these specific titles speak.

## The authentic screen: 640×400, an 8×16 cell, art doubled

There's a subtlety in "how big is this thing" that decides whether the whole
frame looks right. Infocom's v6 artwork is 320×200 MCGA, but the games were
authored and tested against the Amiga/DOS interpreter, which presents them on a
**640×400** screen with a **non-square 8×16 pixel font cell** — 80 columns × 25
rows of text — and scales every picture **2×** on the way to the screen. That
2× is the whole trick: 80 columns spread across 640px of doubled art make the
text read at its period-screenshot size *relative to the picture*, instead of
the oversized 40-columns-over-320px look you get if you take the art dimensions
at face value.

babelmap now does exactly that (matching Frotz's DOS/Amiga profile). The engine
reports a 640×400 screen (2× the Blorb `Reso` standard window, or a plain
640×400 when a story ships no `Reso`), an 8-wide-by-16-tall font cell, and
answers `picture_data` with the **doubled** dimensions — so the game lays its
banner, columns, and compass out on the same 640-wide grid the original did.
The 320×200 pictures themselves stay art-native in storage and are blitted 2×
(crisp nearest-neighbour, DOS-authentic) into the composite; the bitmap text is
rendered by doubling the 8×8 glyph masters vertically to fill the 8×16 cell.
Screen size and picture size double *together*, so the frame-vs-content picture
classification (which is pure ratios) lands exactly where it did before.

## Render modes

Set `v6_render` in the config (or cycle it from the settings screen) to pick
how a v6 story's pane is drawn on an image-capable terminal (Kitty, iTerm2, or
Sixel). Want to compare looks mid-game? `/set-v6-render` hops to the next mode
on the spot (or jumps straight to one: `/set-v6-render raster`) — a
session-only switch that never touches your saved config:

- **`hybrid`** (the default) — the decorative chrome (banner, borders, the
  compass) renders as a single scaled pixel image forming a **ring** around an
  inset viewport, and the story text inside that viewport is real terminal
  text: crisp, selectable, scrollable, and styled exactly like any other
  babelmap transcript — including its own inline images (see below). The ring
  is tiled into up to four non-overlapping bands (top/bottom/left/right)
  around the viewport; a band flush against the pane edge is simply omitted.
  Each top/bottom band is then decomposed further: a horizontal strip that is
  **pure chrome text** — status/menu runs with *no* opaque frame art behind it —
  drops out of the pixel ring and paints as **real terminal cells** (crisp,
  selectable, themed via the game-colour resolver, with solid reverse-video
  bars), while a strip that sits over actual artwork keeps the scaled pixel
  image. So Journey's bottom command menu ("Proceed / Back / Game", the party
  column, the verb columns — a full-width window sized to zero height that paints
  fixed pixel runs) becomes terminal text while its left picture column (and the
  reversed vertical divider the game paints between picture and text) stays
  imaged; Arthur's location/date status row becomes a crisp reverse bar sitting
  between the graphics panel above it and the story below; and Zork Zero's
  status, painted directly *onto* its banner art, stays in the ring. A pure
  reverse-video row (a status/menu bar) fills **edge to edge across the full pane
  width**, so a bar the game drew as separate runs with bare cells between and
  around them reads as one solid block. The rows that become cells are carved out
  of the pixel bands entirely — their rasterized ink never reaches an uploaded
  band image (no raster bar showing through behind the cells), and because a
  band's image no longer depends on that text, navigating the menu re-encodes only
  the genuinely changed artwork rather than every band. The whole cell strip is
  first flooded with the chrome background so the panel reads as one solid block —
  no theme backdrop peeks through the cells between the runs — and when the
  letterbox scale spreads the menu's native rows across *more* terminal rows than it
  has (leaving a blank row mid-menu), that gap row is folded back into the panel and
  its reversed vertical column dividers are carried through, so the lines never
  break. Text that a game positions with proportional (sub-cell) pixel metrics —
  Arthur emits its status words as separate abutting single-glyph runs — is
  reassembled: fragments whose pixel start touches the previous run's end merge into
  one word stamped from a single cell (so "Churchyard" stays whole instead of
  scattering into "Chu rch yard"), while runs held apart by a real pixel gap (menu
  items, column dividers) keep their spacing and never fuse.
  The ring layout is also **dynamic**: on a pane taller than the game's native
  aspect there is vertical letterbox dead space, and hybrid mode reclaims it rather
  than centring the frame in it. When nothing sits below the story — header art,
  side borders, a status bar on top, but an open bottom (Arthur) — the ring is
  anchored to the pane top and the story viewport grows all the way to the pane
  bottom at its exact inset width; where the side art ends, the flanks below it are
  the theme backdrop (no stretching, no tiling). When the game has a bottom text
  chrome instead (Journey's command menu), that strip is anchored to the pane
  *bottom* edge and the story fills the space between the top chrome and the menu.
  A game whose frame *encloses* the story to the screen bottom (Zork Zero's full
  frame) keeps the centred letterbox untouched, and a pane at or below the scaled
  native height (no dead space) degrades to that same centred layout.
- **`raster`** — the whole pane, story text included, bakes into one
  device-resolution pixel image with a bitmap font, the way the original v6
  engine drew it natively. Its default ink/page follow the theme; where the
  theme leaves them at "terminal default", babelmap probes the terminal's own
  foreground/background at startup (OSC 10/11) and paints in those, so raster
  text stays readable on a light-background terminal instead of forcing a
  fixed light-grey-on-black.
- **`frameless`** — a deliberate "classic terminal interpreter, but the
  pictures still show" presentation: **no decorative frame at all**. The story
  runs as a normal full-pane terminal transcript at full size with native
  scrollback, the game's chrome/status text collapses to compact terminal bands,
  and window-0's inline story pictures (drop-caps, room icons) still render via
  the transcript image path. The decorative borders, compass, and banner are
  simply not drawn — but a full-screen **splash** (a title screen, a cutscene
  illustration) *does* show, inline, once, in the flow of the transcript (see
  below). With `--no-images` this is identical to the cell fallback below.
  - The pane is laid out by **relation to the story window**, never by absolute
    pixel row — because v6 games put their chrome wherever their artwork leaves
    room. Chrome text *above* the story becomes the status band and pins to the
    **top** (Zork Zero and Shogun paint theirs on native row 0; Arthur paints his
    on row 12, under a twelve-row art panel frameless doesn't draw — and it still
    lands on line one, not a quarter of the way down an empty pane). Chrome text
    *below* the story becomes a command band pinned to the **bottom**, so
    Journey's verb menu stays welded to the last row at any pane height instead
    of floating over the prose. Chrome text *inside* the story box — Shogun's
    boot menu, a hint screen — paints over the transcript where the game put it.
  - A graphics window sitting wholly **beside** the story is story content, not
    frame, so it keeps its column: Journey's half-screen character portrait
    renders at its native proportion with the prose inset alongside it. Art that
    spans or overlaps the story stays undrawn — that's what frameless means.
  - Because native 320×200-era art is postage-stamp tiny on a modern display,
    frameless **resizes inline images to taste**: a drop-cap floats at about
    **3–4 text rows** tall, and any band-rendered picture (splashes included)
    is **upscaled by a crisp integer factor** (2× or 3×) up to roughly 60% of
    the viewport width — pixel art stays sharp, never blurred or shrunk below
    life size. (Hybrid and raster keep their own letterbox-matched sizing.)
  - *What you lose:* the compass rose and the decorative borders/banner.
  - *What you gain:* full-size story text (no letterbox shrink), native
    terminal scrollback, selectable text everywhere, and splash art that
    scrolls with the story instead of living in a fixed frame — the most
    legible mode on a small window or a slow terminal.
- **Cell fallback** — without an image protocol (a remote or text-only
  terminal, or while a menu/dialog is open), everything (graphics windows,
  status grids, and story text) composites as terminal cells instead of
  pixels, so the game stays playable everywhere. This is what `frameless`
  makes the *deliberate, always-on* choice even when an image protocol is
  available.

The status and command bands in `frameless` and the cell fallback are themed by
the `upper_window` style selector (the same one that colours a v4+ status line);
a beside-the-story picture column letterboxes in the `graphics` selector's style.

## Illuminated drop-caps and room icons, inline

Window 0's own pictures — Zork Zero's illuminated drop-caps and small room
icons — aren't separate chrome; they're story content. babelmap floats them
at the left margin of the story text and wraps the surrounding lines beside
them, so they scroll naturally with the transcript instead of sitting in a
fixed frame.

## Margin pictures — text that flows past the art

Some v6 scenes put the picture on one side and let the prose flow past it.
Shogun's opening is the classic: the game draws its harbour illustration at the
**right** of window 0 and calls `set_margins` to shrink the text's right edge
back past the art (that's the Z-Machine Standard's margin-picture idiom — a
picture parked at a window edge with the margins pulled in around it). The story
text fills the narrower **left** column beside the picture, then reclaims the
full width the moment it scrolls below the art. babelmap honours the game's own
margins: the engine records them (and snaps the cursor home on either edge, per
§15), and both `raster` and `hybrid` float the picture to its placed side —
right for Shogun, left for a drop-cap — wrapping the prose in the column the game
left for it. A picture too wide to leave a readable column falls back to a
full-width band.

## Splash art, inline (frameless mode)

Some v6 titles paint a big picture straight into a graphics window: Shogun's
320×200 title screen, Zork Zero's cutscene illustrations. `hybrid` and
`raster` draw those windows directly, but `frameless` drops the graphics windows
that frame the story — so it would lose the splash entirely. Instead, frameless
recognizes a
*content-sized* draw and re-emits it as a one-off inline image band in the
transcript, upscaled by the sizing policy above, anchored at the point in the
story where the game drew it. It scrolls away with the rest of the turn.

The catch is telling a splash from decoration. babelmap classifies a
graphics-window picture by its size against the reported screen: a picture is
**content** when it covers ≥ 40% of the screen area, or is ≥ 60% of the screen
width *and* ≥ 30% of its height; a narrow strip (≤ 15% of screen width, like
Shogun's 23-pixel side borders) or any small tile stays **frame** and is left
undrawn. On the real games this lands cleanly — Shogun's title (320×200) and
Zork Zero's full-screen cutscenes come through, while their borders, banners,
and 45×40 compass tiles do not. A repeated redraw of the same splash into the
same window is de-duplicated, so a per-turn refresh can't stamp the same
picture down the page twice; clearing the window resets that, so a genuinely
new splash shows again.

## Pixel-faithful status text and colour

Status and chrome text isn't drawn in character cells — it's drawn at the
exact pixel position the game specified, matching the source game's actual
layout instead of an approximated grid. Colour is honored too: a text run's
packed foreground/background (from `set_colour`/`set_true_colour`) resolves
to real RGB, and the reverse-video style bit swaps fg/bg — which is what
makes Zork Zero's scroll ribbons come out dark-on-tan instead of inverted.
On the pixel canvas the Standard palette colours (2–9) resolve to the
Z-Machine Standard's own recommended true-colour RGB (ZMSD §8.3.1) — so
white is real white (255,255,255) and black is black (0,0,0), rather than the
dim VGA base values the terminal ANSI palette would give (white → 170,170,170
"light grey"). The terminal *cell* path still routes Standard colours through
the theme's ANSI palette (so a user's Ghostty colours apply); only the pixel
paths take the authoritative RGB directly.
The story page itself fills with the window's own background colour (when
the game set one) rather than leaving the terminal's theme backdrop showing
through.

This colour honoring now spans *every* v6 presentation, not just the pixel
raster: the frameless classic status band, the painted menu/hint overlays,
the hybrid story-strip overlay, and the plain cell fallback all resolve a
run's game colours the same way. The rule is the shared one every engine
follows — a channel the game explicitly set (a real palette entry or a true
colour) wins; a "current"/"default" sentinel is inheritance, so the theme
keeps that channel — and it's gated on `honor_game_colours` like the rest.
A game that sets no colours (Shogun) is untouched: its runs stay theme-styled
in every mode.

## Adaptive palettes: overlays that borrow their colours

Some of Zork Zero's pictures — the compass rose overlays, the little scene
tiles — don't carry a real palette of their own. They ship with a placeholder
(the stock 16-colour EGA table) and are flagged in the Blorb's `APal` chunk as
*adaptive*: the interpreter is meant to draw them with the "Current Palette"
established by the last ordinary picture it plotted (Blorb spec §11.3). Zork
Zero leans on this hard — it paints a base illustration to set the mood's
colours, then stamps adaptive overlays on top expecting them to inherit that
mood. Decode each one with its own placeholder instead and the compass comes
out in garish primary EGA, clashing with everything around it.

babelmap now tracks that Current Palette as it draws, and when an adaptive
picture comes up it splices the current colours into the picture before
decoding — keeping the overlay's own transparency intact, so the arrow still
cuts a clean hole in the rose. Because the *same* overlay can legally be drawn
under different base palettes as a game moves between scenes, the decoded result
is cached per palette, not just per picture, so a palette change re-tints it
rather than serving a stale copy. All the v6 render modes — ring, raster,
frameless inline, cell fallback — share this decode path, so the fix lands
everywhere at once.

## Arrow keys: movement or map panning, your call

Several v6 titles bind the arrow keys straight to movement — press ↑ and your
character walks north. That's authentic, but it collides with babelmap's own
use of arrows for scrollback recall and map panning, which some players would
rather keep. Set `v6_arrow_keys = false` in the config (or flip it right in
the settings screen) and arrows are withheld from v6 stories — but only at the
`>` prompt, where the movement-vs-panning clash actually happens. There, instead
of being delivered as a ZSCII cursor code, the keypress falls through to whatever
babelmap would do with it if no game input were pending — command-history recall
or map panning, depending on focus.

Menus are the deliberate exception. Whenever a v6 story is waiting on a single
keypress rather than a line — Shogun's startup menu, hint menus, a "press any
key" pause — arrows always reach the game, setting or no setting, because those
screens are unnavigable without them. So the rule is simply: arrows drive
babelmap at the prompt, and drive the game everywhere else.

Enter and every other key are untouched, and v1–v5/Glulx stories keep getting
arrows regardless of this setting; it only ever withholds them from a v6 prompt.

## Not yet there

- **Mouse and menu input** — `read_mouse`/`mouse_window` are recognized but
  not yet wired to real pointer events, so clicking the banner compass
  doesn't yet issue a move.
- **Proportional fonts** — status and chrome text currently use fixed-width
  metrics; the v6 titles' proportional font tables aren't honored yet.
- **Save State across v6** — the host Save State snapshot captures the
  underlying machine as it does for any Z-machine game; carrying the v6-
  specific render state (window geometry, floats, pictures) across a restore
  so the chrome comes back pixel-identical isn't verified yet. Standard
  in-game `@save`/`@restore` follows the normal Z-machine path (see
  [the persistence model](../persistence.md)).
