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

## Where the pictures come from

Most of the time: a Blorb. Either the story file *is* one, or a `.blb`/`.blorb`
sibling beside it carries the `Pict` resources, and babelmap resolves that on its
own.

There is a second source, for anyone playing from original media. Infocom's Amiga
releases stored their artwork in a single `Pic.data` archive on the game disk — a
big-endian Huffman + run-length + per-scanline-XOR codec of Infocom's own design,
nothing to do with PNG — and babelmap decodes it directly. Launch a game from its
[`.adf` disk image](interpreter.md#what-counts-as-a-story-file) and the archive
that shipped on that same floppy becomes the game's art. Nothing to configure:
the story and the pictures came off one disk, so the pairing is guaranteed by the
medium rather than guessed from a filename.

The two sources are close but not identical, and where they disagree the original
media wins. Five *Zork Zero* pictures are cropped in the circulating Blorb —
ids 5, 6 and 7 keep only a 29–39 row band of what are full 320×200 decorative
frames, id 8 is flattened to a plain rectangle, and id 33 loses most of a
"Four Fantastic Flies of Famathria" plate. The floppy has all five whole. The
other 383 pictures decode byte-for-byte identically to the Blorb's, which is its
own quiet confirmation that those Blorbs were converted from the Amiga release.

*Shogun*'s floppy tells the same story from the other side of the format. Its
archive is built the second way the format allows — every picture carrying its
own compression table rather than sharing one for the whole file, which costs two
extra bytes in each directory entry. The header says which shape a file is, and
babelmap reads both. Of the 39 pictures *Shogun*'s Blorb also holds, 34 come off
the floppy byte-for-byte identical; two of the rest differ only in how the Blorb
rounded the Amiga's 4-bit colours, and the others are places the Blorb kept a
band, or a retouched version, of art the floppy still has whole.

Native archives carry no `Reso` chunk, so a story loaded this way falls back to
the standard 320×200 art resolution — which is precisely what every Infocom v6
Blorb's `Reso` chunk declares anyway, so the geometry below is unchanged either
way.

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
  around them reads as one solid block. A **rule** — three or more abutting
  fragments of the same symbol glyph, which is how a game draws a horizontal line —
  gets the same treatment one layer up: it is drawn across the width its own pixels
  span, not one terminal cell per fragment, and it closes the seams the scale opens
  around its corners and titles. That is what lets Journey's line-drawing border
  (which is what the Amiga interpreter profile makes it draw instead of reverse-video
  spaces) reach both edges of the pane at any window size, with the prose wrapping
  inside it. Prose is untouched by the rule: a label's character count is its width,
  because it has to stay legible. The predicate is narrow on purpose — a game with
  proportional metrics emits one run per glyph, so "two equal abutting fragments"
  would read every doubled letter in the corpus as a rule, and Arthur's status bar
  loses its character's name the moment it does. A **lone** line-drawing or block
  glyph gets the other half of the same idea: it is a column **divider**, so it is
  kept out of the fragment merge and stamped at its own scaled column. Abutting
  fragments are otherwise glued back together, because a game with proportional
  lettering hands over one word as several runs and they must read as one word — but
  a glued run then advances one terminal cell per character, and Journey sets each
  party member's `-->` marker flush against the divider after it, so the divider rode
  the marker's letters and stood in a different column on every row that had one. A
  rule is a distance, a divider is a position, and both are placed by pixels; prose
  is neither. A game that never reserves a band and
  instead **overlays** its bar on the top row of a full-screen prose window
  (advent.z6) is given one: a full-width strip of at most two rows, pinned to the
  top of the screen, has its rows reserved off the story viewport so it decomposes
  into an ordinary text strip — a solid bar with the transcript starting beneath it,
  rather than glyphs stamped over scrolling prose. Such a bar need not be
  reverse-video to fill the row; a window that shape *is* the status bar.
  A ring is also nothing when the story window covers the **whole screen**: there
  are no bands left to carve, so no artwork behind it can be shown at all. Such a
  frame is handed to the full-picture composite instead — whether the art *fills*
  the screen (Zork Zero's map, Arthur's illustrated intro plates, Journey's title)
  or merely *encloses* it. fmvpoker's poker table is the second kind: a 640×400
  frame with a hollow middle that the game prints its whole title inside, only 17%
  of its pixels painted. Asking whether the art filled the screen missed it at
  every point that mattered, and the game drew not one picture in hybrid mode.
  A third way to have no ring: the story window's **own** picture is left out of the
  band canvas on purpose (it belongs inside the story viewport, blitted there as a
  float), which is right only while the picture is inside the window it belongs to.
  fmvpoker breaks that without redrawing anything — choosing *Change Current Bet*
  hands the read to its bottom panel, so the panel becomes the story window and the
  window still holding the poker table stops being one. The table then belonged to
  neither half and was drawn by nobody, and the frame vanished for as long as the
  player took to type a bet. A picture painting outside its own story window goes to
  the composite too. No other v6 title's picture ever leaves its story window.
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
  and cleared on a display window. babelmap follows that — with the window the game
  reads input through as the fallback for a game that declares nothing — and gives
  every other prose window its own buffer, drawn in its own rect. A **read does not
  overrule the declaration**: fmvpoker prints "Enter the new bet:" into its bottom
  panel and reads the answer through that panel, and treating the read as the
  answer split one screen across two sinks — the prompt stayed behind in the
  panel's buffer while the panel was published as the story window, whose lines are
  empty by construction, so the player got a blank panel with no prompt, no running
  totals and no echo of what they typed. The **live input line follows the read**
  rather than the story window, so it appears after the prompt in the window the
  player is actually typing into. A secondary window is **live
  screen state**: what it currently shows, with no scrollback, cleared when the game
  erases it — but persisted with the rest of the screen, because a game that splits
  the display does not necessarily repaint it after a restore (advent doesn't). Its
  lines are drawn on the **pixel composite** too, stacked from the window's own
  origin one text row each — fmvpoker prints its bottom menu and its "Select an
  option…" hint into one, and the composite used to draw graphics and grid windows
  and nothing else, so a screen the cell paths showed in full came out with that
  strip blank.
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
  items, column dividers) keep their spacing and never fuse. The glue stops at
  **padding**: a merged run is positioned once by the scale and then advances one
  terminal cell per character, so a field glued to the blank cells in front of it
  inherits their starting column and drifts away from its own as the pane grows.
  One blank cell is a word space and still merges — Arthur's "St Anne's Day,
  Compline" is one phrase — but a wider blank stretch is layout, and what follows
  it is a field with a column of its own. Shogun off the Amiga floppy paints its
  whole status band one run per cell, padding included, and that is why its `Score:`
  and `Moves:` used to line up only at an 80-column story pane and drift apart at
  every other width; Journey's `-->` party markers, glued to the names in front of
  them, stepped left beside the shorter ones for the same reason.
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
  In hybrid, **nothing the game printed as a character is ever rasterised**. A strip
  is classified by what is *in* it, never by where it sits: a side column whose
  pixels the game's own paint runs fully account for is drawn with those characters,
  and only pixels the runs cannot explain — genuine artwork — go up as a bitmap.
  Journey draws its frame as text under both interpreter profiles (box-drawing
  glyphs on the Amiga, reverse-video spaces on the IBM PC), so its four vertical
  rules are now stamped in the terminal's own font, standing in the same columns as
  the `┌` and `┐` on the rule above them, instead of arriving as four RGBA uploads —
  about 192 KB a frame to draw two hundred `│`s, in a different renderer from the
  corners they hang off. Zork Zero's, Shogun's and Arthur's side columns are
  pictures, the runs cannot account for them, and they stay pictures. The half-cell
  a story window's top edge rounds away goes to the flanks too, so the frame closes
  at its corner instead of leaving an unwritten row between the top rule and the
  first line of prose. And this holds at *every* pane shape, reclaimed layout or
  centred letterbox alike — a short, wide pane leaves no dead space to reclaim and
  used to hand the whole flank, border columns included, to one uploaded band, which
  swallowed the frame's rules into the picture beside them.
  **If a game draws a border, the artwork does not overlap it**: the picture's
  allocated span stops where the rule's column begins, and the rule is stamped as
  the character the game printed. Nothing is lost in the trade — the column was
  already established to hold no artwork before the rule can claim it.
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
    "Inside" means inside on **both axes**: a run merely level with the story is
    frame, not a takeover. Journey under the Amiga profile is the case that proves
    it — its border rules are line-drawing glyphs beside the story on every one of
    its rows, and a row-only test called an ordinary scene a menu screen and sent
    the whole frame down this path, where the game's eighty columns are laid out
    one per terminal column while the prose and the mouse map are still placed
    proportionally across the pane. The two agree only at eighty columns wide.
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

A game can also say it in words, and Zork Zero does. It follows an inline draw
with `set_margins`, reserving the column its prose is about to flow in, and that
declaration counts as much as landing on the cursor — which matters because the
cursor test is pixel-exact and Zork Zero does not always hit it. Booted off its
original Amiga floppy, the game reads a tiny placement record out of the native
picture archive and nudges each drop-cap a couple of pixels in from the line; the
converted Blorb records that same placeholder as zero-sized, so the same story
lands exactly on the cursor there and two pixels off it here. The reserved margin
is the same claim either way.

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

**"No room" means no room among the pixels the plate actually painted**, not
inside the rectangle it happens to span. fmvpoker draws a 640×400 poker table into
window 0 and then prints its title *inside* it — because the table is a frame with
a hollow middle, barely a sixth of it opaque. Measured by its bounding box it looked
like a plate that owned the screen and every line of the game's text disappeared;
measured by its ink, the largest clear rectangle it leaves is exactly the hole the
author meant to print in. Arthur's plates are dense enough to leave only their own
centring margin, so they still own their screens.

## What crowds the story window is *art*, not text

The story window has to be seated inside whatever the game drew around it: Zork
Zero rings it with a carved frame, Arthur hangs a graphics panel above it, Journey
puts an illustration down its left side. babelmap finds the room by shrinking the
window's rectangle, edge by edge, until no edge touches anything opaque — and for
a long time "opaque" meant *any* pixel already on the canvas, which includes the
rasterized glyphs of the game's own menus.

That is the wrong question, because a v6 game routinely prints *over* window 0.
Shogun's title puts "You may choose to:" at the left of a four-row window 0 and
its START/RESTORE/QUIT menu into a second window sitting inside the same four
rows; on an Amiga both are simply on the screen. Measured against the menu's
glyphs, window 0's 548×64 box shrank to 548×16 — one row, which leaves no room for
a line of prose *and* the input caret, so the title showed no text at all. Journey
fared worse: the screen-wide fill that closes the bare cells of a reverse-video bar
ran straight across its 392×304 text panel, and the panel measured 392×**0**.

So the shrink is measured against the artwork alone. Everything the game printed
still reaches the screen, and now so does the prose beside it: window 0's page is
painted *under* the labels other windows put inside its box, in the order the game
drew them — page first, then the menu, then the prose.

**And the transcript's own glyphs yield to those labels as well.** Sparing the
page was only half of it: the story text was still rasterized straight over the
labels the page had carefully filled under. fmvpoker is the case. Its story window
is the whole screen, so once five dealt cards fill the frame's interior the largest
clear rectangle left for the transcript drops onto the very box the game gave its
bottom panel — and the panel is where the hand is announced, *You draw (a) an
Eight, (b) a Three, (c) an Ace…*, the only place the cards are named. The boot
banner was written across it. The rule that settles it is a difference in kind: a
transcript is babelmap's re-reading of everything the story window has ever said,
while a label another window is holding is on the screen *right now*. Where they
land on the same cell, the live label wins, and everything the transcript owns
outside those cells still prints.

The same distinction settles how a reverse-video run is drawn. Highlighting a run
means painting a solid block and cutting the glyph out of it — except over frame
art, where a block would erase the picture, so babelmap draws dark ink directly on
the art instead (that is how Zork Zero's ribbon labels sit on their banner). The
"is there art here?" test also used to read the live canvas, where an earlier run's
own highlight block looks exactly like artwork. advent's help screen is drawn as
one run per label plus reversed spacer spaces, and one of those spacers lands in
the middle of "About Adventure" — so the header concluded it was sitting on a
picture, dropped its block, and drew itself in the page colour on the page. The
whole navigation bar was invisible in `raster` while reading perfectly as cells.
Both tests now consult the art layer, frozen before a single glyph is stamped.

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

**A second text window keeps its columns too.** A v6 game may run more than one
wrapping, scrolling window, and the one that is not the transcript keeps its own
lines rather than joining the stream. Those lines used to be recorded as plain
text with no note of where each run began, so a game that placed several runs
across one row got them back butted together: fmvpoker prints its five menu
options at pixel columns 1, 178, 372, 454 and 557 and read back as
`PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT`. Such a window now honours a
declared column the same way the stream does — with one difference that matters.
The line is padded out **to** the column, not indented **by** it: a run has to
land where the game named it, not that far past wherever the previous run
happened to end, or five labels at fixed columns drift into a ragged row. A
column already behind the line's end cannot be reached by appending and is
ignored; a line buffer only moves right. The declared *row* is honoured the same
way, with blank lines — the buffer is padded out to it, and a row already behind
its end is ignored.

The row is taken to the **nearest** text line, not the line it happens to fall
inside. A line buffer's only vertical unit is the line, and it is drawn at the
window's top plus sixteen pixels per line, so the question is which line the game
meant — and rounding down can lose a whole row of it. fmvpoker places its menu bar
and its *Continue* button at pixel row 80 of a bottom panel, which is five
sixteen-pixel lines down if you count from zero, the way the game did; taking the
line it falls *inside* gave four, drew the five labels fifteen pixels high, and
put them clear of the band the game's own mouse handler accepts for them. The
labels were visible and dead: clicking one did nothing, while clicking the blank
row beneath it played the hand.

## A window the game drew a frame around is a canvas, not a page

The story window is a transcript in every Infocom v6 title: text streams into it,
babelmap keeps the scrollback, and you can page back through it. Frobozz Magic
VideoPoker is not built that way. It draws a poker table across the whole screen,
grows its story window to the whole screen behind it, and then *positions*
everything it has to say — `HOLD` under each card you are holding, the running
totals in the panel at the bottom. Read as a transcript, all of it arrived as
narration: `HOLD` scrolled past in the story text instead of appearing under a
card, and the running totals stacked up as prose.

The tempting rule — "a run the game moved the cursor before is paint" — does not
work, and it was measured rather than argued. Arthur positions every room headline
in the story window, one character at a time, with only the first character
carrying the cursor move; Shogun and Journey centre each line of their title
headers the same way; the Mysterious Adventures re-home the cursor before every
prompt. All of them mean *resume the story here*, with the identical signal
fmvpoker uses to mean *paint this under that card*. Under that rule Arthur's
`CHURCH` came out as a painted `C` and a streamed `HURCH`.

So babelmap asks what kind of **surface** the window is, not what a run means.
Arthur's story window is a transcript that happens to have plates drawn on it;
fmvpoker's is a picture frame that happens to have text positioned in it. The
discriminator is that the window's own art **encloses** it — painted pixels within
a text row of all four edges — while *not* filling it: a solid full-page plate is
something a game narrates over, and a frame with a hole in the middle is something
a game positions text inside. A window like that renders as what is sitting on it,
at the coordinates the game named, and carries no transcript at all — which is
exactly how a real interpreter shows it, and it is the same idea as a window
keeping the ground it painted, applied to the text on that ground. Measured across
every v6 title babelmap is tested against, one game answers to it.

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

**And the transcript restarts in the box the game moved the window to.** Freezing
the old half was only half the job: the live half has to land somewhere, and
somewhere is the story window's own box. In the pixel composite that was always
true — the transcript is drawn inside the window's rectangle. The cell
presentations (`frameless`, and `hybrid` on a menu screen) build the pane by
relation instead: the chrome above the story packs against your pane's top edge,
the chrome below packs against its bottom, and the transcript fills between. That
packing used to start the transcript flush under the band, which is right for
every game that puts its story window directly under its status bar — and wrong
for Shogun, whose window 0 sits nine rows further down, level with the menu. The
prompt came out on the line below the banner instead of beside START/RESTORE/QUIT.

Now the story window's box says where its transcript starts: the gap the game left
between its chrome and its story window carries through into the pane, and anything
painted *inside* that box — a menu's items, and the ground erased under them —
travels with it. The gap is measured against the chrome's declared rectangle rather
than the text in it, so a status panel taller than its own two lines (Zork Zero's
is 78 pixels of which two rows carry text) does not push the transcript down for
art `frameless` has deliberately dropped. Nothing above the story window at all
means nothing to sit below, and the transcript keeps the top of your pane.

**A cleared screen starts at the top of its box, in every mode.** When a game
clears the screen, babelmap pins what it prints next to the *top* of the story
window and leaves the rest blank, rather than sticking to the bottom and dragging
pre-clear history back into view — your scrollback is all still there, one scroll
up. The cell paths have always done this; `raster` now does too, which is what
keeps Shogun's four-row box showing the one line the game printed into it instead
of redrawing the tail of the banner it had just frozen up top.

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

**The story page fills UNDER the game's own fills.** Window 0's page is the oldest
thing in its box — the game filled the window, then everything else was drawn on
top — so the page yields both to the labels other windows print inside that box
and to any rectangle the game itself painted with `erase_window`. fmvpoker is why
the second half matters. It draws its poker table with Zork Zero's picture file
(the original release ships that file renamed to `FMVPOKER.EG1`), so the frame's
top-centre tab natively reads *Double Fanucci* — and the game hides that title the
way a v6 game does, by parking a window over the banner and erasing it to the
colour it declared for that window. It never prints a title of its own there; the
banner is erased, not overwritten. With window 0 covering the entire 640×400
screen, a page fill that ignored the erase repainted the tab in window 0's white
and the frame appeared to have its top cut off — an artefact of the fill order, not
of the artwork, which is neither clipped nor mis-placed.

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

The original Amiga archive has no `APal` chunk, because it never needed one: it
writes a plain **zero** where each picture's palette would go, and a picture with
no palette can only be drawn through the one that is current. That is the same
statement, made per picture rather than in a list — and for *Zork Zero* it marks
exactly the same 172 pictures the Blorb's `APal` names, id for id, which is
another sign of where those Blorbs came from. So native artwork goes through the
machinery above rather than beside it: one Current Palette, in the same colours a
Blorb `PLTE` holds, tracked the same way and carried into a save the same way.
The check is Infocom's own: their converter pre-computed every
(illustration, overlay) pairing and shipped the results inside the Blorb, and the
Amiga archive reproduces 36980 of those 37152 answers exactly. The remainder are
all one illustration — picture 8, one of the five the Blorb replaced — where the
floppy is the source that is right.

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

## `/dump-windows` reports the last frame the *game* drew

When a v6 layout looks wrong, `/dump-windows` is how you say what you saw: one
block per window, merging the game's own window table, the model babelmap built
from it, and where the renderer actually put each one on the terminal — the three
things that have to agree.

There is a catch built into asking. Reach the command through the command palette
or a hotkey dialog and you are opening a modal overlay, which routes the v6 pane
off its pixel path — so the frame *most recently drawn* when the dump runs is the
palette's, in which every one of the game's windows is honestly reported as
`NOT DRAWN this frame`. That is the one thing nobody opened the dump to learn. So
babelmap keeps the mapping from each frame the **game** drew, and the dump
describes that one: its render path, its pane, its story viewport, its per-window
cells and chrome strips, and the ring's own plan and clip for that frame. A
`frame described:` line says which frame it is and how many modal frames have
gone by since. The game-side halves — the window table and the model — are read
live, because a modal overlay runs no game code and they still describe the frame
being reported. If no frame has ever been drawn without an overlay up, the
placements are reported as `UNAVAILABLE` rather than quietly swapped for the
overlay's.

Better still, don't open a modal at all. Reporting the right frame stops the dump
*lying*; it does not stop the palette **churning** the very numbers the dump
prints. Opening it costs a run of `cell — modal overlay open: palette` entries in
the render-path history, and coming back out invalidates every cached chrome band
so they all re-upload — visibly moving `band uploads since launch` and pushing the
frame of interest further into the past. Bind the command to a key instead and the
capture reads the counters without touching them:

```toml
[keymap.global]
"ctrl+d" = "dump-windows"
```

Ctrl rather than a bare key on purpose: while a v6 story waits on a single
keypress — Journey's menus, any "press any key" — plain keys go to the game, and
`F9` would answer the prompt instead of dumping. The Ctrl binding fires from map
focus too.

Under the window blocks the **band list** names every image the ring placed on that
frame: its cell rect, where it was placed, whether it re-encoded or came out of the
cache, and — the part that matters when a picture turns up somewhere it shouldn't —
the **native crop** it is showing, so you can read which rows of the game's own
screen an image is painting. A flank's picture is drawn at a rect the panel derives
rather than at the strip's, and it used to be missing from this list entirely: two
investigations of Journey's picture column reasoned about it from the strip beside
it, because the one band they wanted to see was the one band the dump could not
name. A flank's two border columns say which **medium** they came out in — a
`flank-divider (glyph '│' style=00)` line is a rule stamped in the terminal's font,
carrying no crop because there is nothing to crop, while a plain `flank-divider`
with a native crop beside it is still a bitmap. "The frame's sides are a picture of
a character" is a sentence this dump can now say in one line.

The dump also lands in **`~/.babelmap/dump-windows.log`**, appended, with a
timestamp per capture, and the transcript line names the path. Selecting the
on-screen copy off a v6 pane drags the graphics protocol's own placeholder glyphs
along with it — the diagnostic corrupted by the thing it is diagnosing — and the
file is the same text with nothing composited over it. Read it from a second
terminal while the game is still running, take several captures across a turn,
and paste any of them intact.

## `/dump-cells` writes the rendered screen — glyphs *and* colours — as plain text

`/dump-windows` answers *where did each window land*. It cannot answer the
question a v6 layout defect nearly always turns out to be: **which colour landed
in which cell**. A panel fill painting rows underneath a menu, a border cell
wearing the fill's colour instead of the frame's, a label the cell buffer holds
and the screen does not — geometry shows none of the three, and each one used to
cost a round trip through a screenshot.

`/dump-cells` writes the frame itself. Two lines per terminal row: the **glyph**
row, so borders and labels read as text, and directly under it the **style** row,
one key character per cell indexing a legend of the distinct styles. No ANSI
escapes anywhere — the whole point is text you can copy, paste and diff.

```
 52 g|│──────────────────────────The P──────────────────────────────────Individual Comm──│
 52 s|ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
```

Above the grid, three summaries do the counting for you:

- **`graphics:`** — every region an uploaded image covers, by rect. Those cells
  read `#` in the glyph row, because an image draws *over* the terminal's text
  layer and whatever character is underneath is not on screen. Their **style** row
  survives, though: placing an image does not touch the colours of the cells it
  covers, which is exactly how a fill painted beneath an art strip stays visible.
  Placements the renderer recorded are listed beside the rects recovered from the
  buffer, because halfblock and sixel backends paint without leaving escape cells
  behind at all.
- **`row backgrounds:`** — the rows every cell of which shares one background,
  as ranges. "These nine rows all carry the panel fill's colour" is one line here
  instead of a count done by eye off a screenshot.
- **`styles:`** — the distinct styles, commonest first, each with its exact
  foreground, background, attributes, cell count and bounding box, plus the rows
  it owns end to end. A picture rendered *into* cells can run to hundreds of
  styles (one per pixel pair); past the first 48 the tail is bucketed under `*`
  with its own count and extent, so the legend never buries the dozen that matter.

The whole capture goes to **`~/.babelmap/dump-cells.log`**, appended and
timestamped, and the transcript line names the path — only the path, because the
grid is two lines per row and echoing it would scroll the very frame your next
capture is meant to describe. Like the window dump, it lands in a file because a
selection dragged off a v6 pane brings the graphics protocol's placeholder glyphs
with it.

It describes the last frame drawn with **no modal over it**, for a reason sharper
than the window dump's: a modal is painted straight onto the cells, so a capture
taken through the palette would not report a stale frame — it would report the
palette's box sitting where the game's picture was. Bind it to a Ctrl key and no
modal ever opens:

```toml
[keymap.global]
"ctrl+g" = "dump-cells"
```

A bound-key capture moves neither the render-path history nor the band-upload
count; the palette route moves both.

## Not yet there
- **Proportional fonts** — status and chrome text currently use fixed-width
  metrics; the v6 titles' proportional font tables aren't honored yet.
- **Save State across v6** — the host Save State snapshot captures the
  underlying machine as it does for any Z-machine game; carrying the v6-
  specific render state (window geometry, floats, pictures) across a restore
  so the chrome comes back pixel-identical isn't verified yet. Standard
  in-game `@save`/`@restore` follows the normal Z-machine path (see
  [the persistence model](../persistence.md)).
