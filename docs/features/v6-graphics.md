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
Sixel):

- **`hybrid`** (the default) — the decorative chrome (banner, borders, the
  compass) renders as a single scaled pixel image forming a **ring** around an
  inset viewport, and the story text inside that viewport is real terminal
  text: crisp, selectable, scrollable, and styled exactly like any other
  babelmap transcript — including its own inline images (see below). The ring
  is tiled into up to four non-overlapping bands (top/bottom/left/right)
  around the viewport; a band flush against the pane edge is simply omitted.
  A painted command menu that sits *below* the story window rides that bottom
  band — Journey drives its whole interface through one (a full-width bar sized
  to zero height that paints "Proceed / Back / Game", the party column, and the
  verb columns as fixed pixel runs), and it rasterizes into the bottom ring
  rather than the story viewport.
- **`raster`** — the whole pane, story text included, bakes into one
  device-resolution pixel image with a bitmap font, the way the original v6
  engine drew it natively.
- **`frameless`** — a deliberate "classic terminal interpreter, but the
  pictures still show" presentation: **no decorative frame at all**. The story
  runs as a normal full-pane terminal transcript at full size with native
  scrollback, the game's chrome/status text collapses to a compact terminal
  status band across the top of the pane, and window-0's inline story pictures
  (drop-caps, room icons) still render via the transcript image path. The
  decorative borders, compass, and banner are simply not drawn — but a
  full-screen **splash** (a title screen, a cutscene illustration) *does* show,
  inline, once, in the flow of the transcript (see below). With `--no-images`
  this is identical to the cell fallback below.
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

The status band in `frameless` and the cell fallback is themed by the
`upper_window` style selector (the same one that colours a v4+ status line).

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
`raster` draw those windows directly, but `frameless` drops graphics windows —
so it would lose the splash entirely. Instead, frameless recognizes a
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
The story page itself fills with the window's own background colour (when
the game set one) rather than leaving the terminal's theme backdrop showing
through.

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
