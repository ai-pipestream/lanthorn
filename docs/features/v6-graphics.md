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

## Splitting the screen TILES it

A v6 game reserves room for artwork by splitting the screen, and the standard is
precise about what that means (§8.8.4.1): the opcode "tiles windows 0 and 1
together to fill the screen, so that window 1 has the given height and is placed
at the top left, while window 0 is placed just below it (with its height suitably
shortened, possibly making it disappear altogether if window 1 occupies the whole
screen)". The split *places* the story window; it does not merely shrink it.

That matters because most games never move the story window themselves — they
don't have to. `mysterious01.z6` splits off 260 pixels, draws its illustration up
there, and starts narrating; if the story window is left in the top-left corner it
sits inside the picture and the prose prints across the artwork. And Adventure's
Inform 6 library goes further: it splits, asks the interpreter where the split left
window 0, and positions its own prose window at the answer. A game reads the
tiling back, so getting it wrong misplaces everything downstream of it — bar, room
description, and menus alike.

The spec's own escape hatch is worth naming: a split that takes the entire screen
leaves the story window with zero height, which is exactly what Zork Zero's
full-screen title splash relies on. Nothing is carved over the picture, and the
game re-places the window itself when the splash goes.

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

The doubling follows the **`Reso` chunk**, though, not the version number. Blorb
§11 is explicit that a resource file without one has no scalable images at all —
"non-scalable images are always displayed at their actual size. (One image pixel
per screen pixel.)" Every Infocom v6 blorb declares a 320×200 standard window, so
they all double. scopa.blb declares nothing: its card art is drawn for the 640×400
screen already, the same 52×84 as the vector deck hardwired into its own z-code.
Doubling *that* told the game its cards were 104×168, and it dutifully laid out a
menu whose sample cards overlapped each other and hung off the bottom of the
screen. So the screen is still 640×400 either way, and the art scales only when the
story says what it should scale against.

And that screen is a **hard edge**. A v6 game may size a window far past it,
because `window_size` doubles as a measuring instrument: scopa opens a scratch
window 1000×1000 so a string it is about to print cannot wrap, reads the width
back, and moves on. Taken literally that one window is bigger than the screen,
and since the composite spans every window the game has open, the whole picture
would shrink to fit it — the table crammed into a corner with black bands where
the oversized window's page fell off the world. babelmap draws the part of a
window that exists: each box is clipped to the screen the header declares
(§8.4.3's width and height words) before anything is composited. The clip is
purely what gets *drawn* — the interpreter still reports the size the game wrote
when the game asks for it back, which is the whole point of the trick scopa is
pulling. `/dump-windows` shows both: the size the game set, and what of it is on
screen.

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
  around them reads as one solid block. A game that never reserves a band and
  instead **overlays** its bar on the top row of a full-screen prose window
  (advent.z6) is given one: a full-width strip of at most two rows, pinned to the
  top of the screen, has its rows reserved off the story viewport so it decomposes
  into an ordinary text strip — a solid bar with the transcript starting beneath it,
  rather than glyphs stamped over scrolling prose. Such a bar need not be
  reverse-video to fill the row; a window that shape *is* the status bar.
  Not every v6 game *has* a story window to ring, though. scopa's card table
  streams no prose at all — its screen is three grid windows and a table drawn out
  of filled rectangles, with two button labels on top — and a ring around nothing
  is nothing. A screen with no story window is presented whole instead: as crisp
  positioned terminal text when it really is only text (a hint menu, a boot menu),
  and as the **full-picture composite** when the game has painted pixels onto it,
  because those pixels *are* the screen and the composite draws the labels over
  them anyway. So hybrid shows scopa's table exactly as raster does, and Zork
  Zero's InvisiClues stays the readable full-pane text screen it has always been.
- **More than one scrolling text window.** A v6 game may run several flowing-prose
  windows at once — advent.z6's `style` opens one across the top of the screen and
  keeps playing in another below it. Both are wrap+scroll, so both stream through
  the same text path, and splicing them into one transcript scrolled the top
  window's text away with the story (the game warns about exactly that). Which
  window carries the narrative is the game's own declaration: ZMSD §8.8.3.1's
  attribute 2, "text copied to output stream 2", is set on the transcript's window
  and cleared on a display window. babelmap follows that — corroborated by the
  window the game reads input through — and gives every other prose window its own
  buffer, drawn in its own rect. A secondary window is **live
  screen state**: what it currently shows, with no scrollback, cleared when the game
  erases it — but persisted with the rest of the screen, because a game that splits
  the display does not necessarily repaint it after a restore (advent doesn't).
- **Erased windows are opaque.** On a real interpreter every v6 window is a
  clipping region over one shared screen bitmap, so erasing a window paints its
  rect with that window's background — which is what makes a hint menu hide the
  story behind it. babelmap composites layers instead, so it tracks the erase:
  a window stays an opaque field until the story prints another character, at
  which point the prose is the newer paint and the fill stops covering. That one
  rule keeps both cases right — advent.z6's `help` (erase the screen, split a
  160px window, erase it, paint the menu, and print no prose) reads as a solid
  panel on blank background, while Zork Zero's full-screen decorative window,
  erased to white during boot *before* a word of story has printed, never
  blankets the transcript. The rows that become cells are carved out
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
    All three are painted *after* the windows' erase fills go down, because a
    window's erase is the ground its own text is written on, not a lid over it:
    paint the band first and Adventure's status bar disappears under the very
    window that drew it. The band's height is measured up front (it decides where
    the transcript starts) and drawn at the end, with the rest of the text.
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

The tell is where the game put the picture: **on the current text line, or
somewhere it chose for itself.** A drop-cap is drawn at window 0's text cursor —
it belongs to the paragraph beside it and has to travel with it. Ask for a
picture at a row the cursor is nowhere near and you mean something else
entirely: you have placed it. (An inline float's horizontal position is a margin
choice — Shogun parks its ship at the right edge and still means "beside this
paragraph".)

There is a second question, because clearing the screen also puts the cursor
back at its top-left corner: **is there any room left beside the picture?** A
float, by definition, has prose flowing next to it. A picture that spans window
0 from edge to edge leaves no column for that prose, so it cannot be one — it is
a backdrop, and it goes on the window's own canvas with the story text drawn
over it. Frobozz Magic Videopoker paints its whole card table that way, Journey
its title illustration, the Mysterious Adventures their title cards; every one of
them draws at (1,1) immediately after erasing the screen and would otherwise be
mistaken for the world's largest drop-cap. The margin between the two readings is
not a fine one: the widest genuine float in the Infocom v6 catalogue — Shogun's
ship — covers 58% of its window, and every backdrop covers all of it.

## Full-page plates — art the game placed itself

Arthur opens on three illustrated screens, and it lays each one out by hand.
It clears every window, asks window 0 how big it is, does the centring
arithmetic itself — a 584×392 plate in a 640×400 window lands at x=29, y=5 —
and draws the plate there. The Merlin screen redraws the graveyard at that same
origin and composites Merlin *inside* it, so the wizard appears on the graveyard
in a single frame rather than beneath it in a second one.

babelmap honours that arithmetic. A window-0 picture the game placed rather than
inlined gets a real window canvas, at the pixel origin the game named, and later
draws composite into the same canvas exactly as they would on an Amiga. The
centring margin the game deliberately left around the plate stays the story
window's own page — we don't stretch art to fill space the author left empty.
Because such a screen has no frame ring at all, `hybrid` hands the frame to the
full-canvas compositor (the same path Zork Zero's map takes), which ships the
plate as one image.

**A plate is drawn *instead of* prose, not underneath it.** Arthur's illustrated
screens carry no text at all: the game erases the screen, draws the plate, hides
the cursor and waits for a key — the whole graveyard→Merlin turn is thirty-one
instructions and prints not one character. Its narration is a *separate*,
picture-less screen, erased before the next plate goes up. So when a placed plate
leaves no column wide enough to wrap prose into, the picture owns the screen and
babelmap draws no story text on that frame — the same rule a window-filling
picture like Zork Zero's rebus already followed. A plate that *does* leave a real
column — a margin illustration, a corner logo — still gets prose beside it.

## Pictures land one after another, the way they were drawn

A v6 turn can draw more than one picture, and the order matters to how the screen
reads. Arthur's graveyard→Merlin screen is the case: **one** turn erases every
window, paints the 584×392 graveyard plate, and fourteen instructions later paints
Merlin into the middle of it. Compositing both before anything reaches the terminal
hands you the finished picture instantly — correct, and completely flat. On the
machines these games were written for you watched the graveyard fill the screen and
*then* watched Merlin arrive on top of it, because each `draw_picture` blitted as
its opcode ran.

babelmap plays that back. The turn still runs straight through — the interpreter
never blocks, never yields mid-picture, and the composite it ends on is exactly the
one it built before — but the renderer walks the screens the turn passed through on
the way there, one per frame. The wait between them is proportional to the area
each picture painted, so a full-page plate rests for a beat you can see and a small
compass tile barely pauses at all; that is roughly what the original hardware
imposed, for the same reason.

It is not an Arthur rule and there is nothing to switch on. Any v6 turn that draws
more than one picture paces, so Zork Zero's border assembles itself at startup,
Shogun's title screen arrives in two beats, and Journey's scene art lands after the
frame it sits in. And you are never made to wait: **any keypress collapses the rest
of the sequence instantly**, landing on precisely the pixels waiting it out would
have given you. The key still does whatever you pressed it for — pacing is
decoration over a turn that already finished, so it never swallows a keystroke.

There is no Z-machine construct for any of this. Nothing in those turns busy-waits
or sleeps, and the `read_char` timers on Arthur's illustrated screens are an
auto-advance for a player who has wandered off, not an animation clock. This is a
presentation choice, made deliberately, because the games were written for
machines that painted at a visible speed.

## Prose the game positions itself

A v6 window that wraps and scrolls streams its text, and babelmap renders that
stream as real terminal text — selectable, scrollable, reflowing to your pane.
But a game can still position that prose horizontally, and some do. Shogun's
title screen is the case: for every header line it reads its own window's width,
computes the centred column, moves the cursor there, and prints the line with no
leading spaces whatsoever. The centring lives entirely in the cursor move.

babelmap carries that declaration into the text stream as an indent. The v6 cell
is 8 pixels wide, so the pixel column and the character column are the same
measurement — column 297 is character 37, which is exactly where a six-letter
title centres on an eighty-column screen. Every line of Shogun's header lands on
the column it asked for, and Journey's title screen, which centres itself the
same way, comes out right for the same reason. A game that never declares a
column never gains an indent.

## Prose freezes where it was printed when its window moves

The Z-machine standard is blunt about it: moving or resizing a window "does not
change the current display". Text already printed is pixels, and pixels do not
follow a box around. Shogun's opening depends on it — the whole nine-line title
header is printed while window 0 *is* the screen, and then window 0 drops to a
tiny box at the bottom beside the menu and prints "You may choose to:" there. On
an Amiga the header simply stays up top; babelmap streamed both halves into one
transcript, so the prompt came out jammed under the banner and the banner
promptly scrolled out of a four-row box.

So a scrolling window's prose is now frozen the moment its window moves out from
under it: the lines become paint, at the exact rows and columns the game printed
them at, and the transcript starts again at the window's new origin. Everything
frozen stays in your scrollback — nothing is deleted, it just stops being the
live screen. Shogun's title now reads the way it does on the original: the header
centred across the top, "You may choose to:" down beside START/RESTORE/QUIT.

**Only prose the window walks away from freezes.** A window resized *around* the
text it just printed still covers it, so that text is still the window's own and
keeps streaming — which is what Arthur does on nearly every turn of play, and
what makes the difference between a faithful title screen and a transcript that
quietly stops scrolling.

**Frozen prose keeps the columns it was given.** `raster` composites the frozen
layer as pixels, so it lands exactly where the game put it. `hybrid` and
`frameless` have no pixels to composite there: text sitting above the story window
is drawn by the anchored status-band renderer, which stretches a game's 40- or
80-column bar across whatever width your terminal is by sorting each run into a
left, centre or right field. It decides by where the run *starts*, which is how a
location name finds the left margin and a score finds the right one — and which
would tear a centred paragraph apart, since a longer line starts further left. So
a run whose margins are equal on the game's own screen is taken as deliberately
centred and is centred again in your pane, however far left it begins. A field
that starts at the screen's edge is not centred text and stays anchored where it
was.

## …and so do pictures

Same rule, one layer over. babelmap keeps each v6 window's art on a canvas of its
own and paints that canvas wherever the window currently is, which tells the truth
right up until the game moves the window. scopa never stops moving it. Every
picture it draws goes through a scratch window it borrows for exactly one
operation — move it to the corner the card belongs in, size it to 1000×1000 so
nothing can clip, draw at (1,1), and immediately move it again for the next card
or the next fill. Its Neapolitan and Sicilian decks were being drawn into a window
that had already left, clipped to whatever sliver it had been shrunk to and then
erased outright by the following fill, so the only deck that ever reached the
opening menu was the vector one the z-code draws with fills instead of pictures.

Two things fix it, and both are the standard read literally. The engine now
records the window's **box at the moment of the call** on the picture event
itself, the same way it already records the rect an `erase_window` painted — a
scratch window's geometry is only meaningful at the instant it is used. And when a
window with art on it moves, that art is frozen onto the screen's painted ground
at the coordinates it was drawn at, exactly as prose is. Picture draws and erase
fills also drain as one ordered timeline now rather than one queue after the
other: scopa's boot fills its green table, draws two card pictures and *then* fills
the menu buttons over the top, and replaying the fills last let the opening
full-screen clear wipe cards that had already been painted.

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

Every *other* window's page follows the same rule, because ZMSD §8.8.3.2 gives
each Version 6 window its own pair — not one page shared by the screen. It
matters most where the art is mostly holes: Zork Zero's compass and room icons
are line art on a clear ground (95% transparent) and hang below the banner
artwork, so the pixels behind them are pixels nobody painted. Left transparent,
the graphics protocol picks the colour, and it picks black. babelmap paints each
chrome window's own page into its untouched pixels instead, so the ring it
uploads is self-contained. Only `alpha == 0` pixels are filled — artwork, status
bands, glyphs and the icons' own ink are untouched, the story box stays clear for
the terminal transcript, and a window the game gave no colour keeps the host
page. It is the whole screen's look for Scopa, whose green baize is a window
background and nothing else.

**A window the game has drawn into keeps its page even when you decline game
colours.** That exception exists because Scopa's baize is not a preference at all:
read from the screen ops, it sizes window 1 to the whole 640×400 screen, names an
explicit true-colour green and issues `erase_window` — the same fill opcode that
draws its cards. A fill spanning the entire screen is treated as a screen clear
rather than as paint (otherwise every game that merely erases would gain a
backdrop it never asked for), so the window's background is the only surviving
record of that drawing. Gating that record on `honor_game_colours` while leaving
the smaller fills ungated split one picture in half: turn game colours off and you
got a *black* table carrying the green bands and the cards the game had drawn onto
it. The discriminator is the painted ground — a window with the game's own pixels
inside it is a canvas and keeps its page either way; a window with none is
presentation, and your theme still owns it. The story window is never in scope:
its page and ink are the surface prose is read on, and those are exactly what the
setting is for. Nothing else in the v6 corpus paints a ground at all, so Zork
Zero, Arthur, Shogun, Journey and Adventure are untouched.

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

## Click the compass, walk the map

A click inside the game image is mapped back through the letterbox to the game
pixel it landed on and delivered the way the original interpreters did it: the
coordinates go into the header extension table and the click terminates the
pending read (ZSCII 254, §3.8) — at a `>` prompt too, when the story asks for
click terminators, which is exactly how Zork Zero's banner compass works. Click
a spoke and you walk.

The automapper comes along for the ride. A click types nothing, so there is no
command to parse a direction from — but the game echoes the command it
synthesized (`north`, alone on the first output line), and babelmap adopts that
echo as the turn's movement command. A compass-clicked move draws the same
directional edge on the map, and records the direction as tried, as if you had
typed it.

## Not yet there
- **Proportional fonts** — status and chrome text currently use fixed-width
  metrics; the v6 titles' proportional font tables aren't honored yet.
- **Save State across v6** — the host Save State snapshot captures the
  underlying machine as it does for any Z-machine game; carrying the v6-
  specific render state (window geometry, floats, pictures) across a restore
  so the chrome comes back pixel-identical isn't verified yet. Standard
  in-game `@save`/`@restore` follows the normal Z-machine path (see
  [the persistence model](../persistence.md)).
