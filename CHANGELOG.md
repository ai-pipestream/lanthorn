# Changelog

All notable changes to babelmap are recorded here.

**Tag convention.** A release is cut by pushing a `v*` tag (see
[`.github/workflows/release.yml`](.github/workflows/release.yml)). A tag whose
name contains a hyphen — `v0.1.0-beta.1`, `v0.2.0-rc.1` — is published as a
**pre-release**; a bare `vMAJOR.MINOR.PATCH` is a full release. The workspace
version in `Cargo.toml` (currently `0.1.0-beta.4`) versions every crate and every
binary's `--version` at once, and carries the pre-release suffix so a build
identifies which beta it is without reading its git hash.

---

## Unreleased

### Added

- **v6 pictures land one after another, the way the game drew them.** A single v6
  turn can draw several pictures — Arthur's intro paints the graveyard plate and
  then paints Merlin into the middle of it, fourteen instructions later, without
  pausing in between. Compositing both before anything rendered handed you the
  finished screen instantly; now the renderer walks the screens the turn passed
  through, one per frame, so you watch the graveyard fill the screen and then watch
  Merlin arrive on it. The pause between pictures is proportional to the area each
  one painted, so a full-page plate rests for a beat you can see and a small tile
  barely pauses — roughly what the machines these games were written for imposed.
  The interpreter is not slowed or blocked for any of it: the turn runs straight
  through as before and the composite it settles on is byte-for-byte the one it
  always built. Every v6 game, with nothing to switch on — Zork Zero's border
  assembles itself at startup, Shogun's title arrives in two beats — and **any
  keypress collapses the rest of a sequence at once**, landing on exactly the pixels
  waiting it out would have given you, while still doing whatever you pressed it
  for.
- **Click a room in the matrix and it shows you the way there.** babelmap finds
  the shortest route it already knows how to walk, from the room you are standing
  in to the one you clicked, and marks one cell per step — the row of the room you
  are in, in the column you leave by — so the marks read top to bottom as walking
  instructions. Each cell keeps its own glyph, so you can still see whether the
  step you are about to take comes back or does not. Passages are only ever walked
  in the direction you walked them, so a one-way corridor is never offered
  backwards: a route babelmap shows you is a route you can actually walk. The
  search covers the whole map rather than just the layer on screen — steps on
  other layers have no row here, and where the route walks out of this layer the
  `⇱out` cell it leaves by is the one marked — and the view never jumps layers
  behind your back. With no known route the room still selects and babelmap says
  so. `Esc` clears the route and keeps the room selected; a second `Esc` unpins
  it, a third closes the dock. Styleable as `map.matrix.cell:path`.
- **The room dock — one panel that describes where you are.** The floating Room
  Info popup (left-click) and the diagnostics Inspector (right-click,
  `/toggle-inspector`) are retired: both are now BODIES of a single dock that
  slides in at the bottom of the map pane. It covers nothing, counts as no
  overlay, and stays up while you play — the keyboard never leaves the story
  prompt. With nothing selected it **follows** you, describing the room you are
  standing in and updating every move; clicking a room **pins** it there, and the
  header says which regime it is in. Unpin by clicking the pinned room again,
  clicking empty map space, or pressing `Esc`; a second `Esc` closes the dock.
  `/toggle-room-dock` (leader `k`) opens and closes it; `/toggle-inspector` keeps
  its name and now opens — or flips to — the Diagnostics body, no longer needing
  a room to be selected first. Its top edge drags like every other pane boundary
  (height persisted as `room_dock_pct`), it joins the `F3` resize-mode Tab cycle,
  it docks below the matrix view as happily as below the drawn map, and it is
  styleable through `room_dock`, `room_dock.header` and `room_dock.header:pinned`.
  The exit card spends the dock's WIDTH rather than its height — the twelve
  travel directions lay out in up to three columns (cardinals, diagonals,
  portals, matching the matrix's own grouping), so the card is four rows on a
  normal map pane instead of twelve, and falls back to the single column on a
  narrow one. Its two view names are a real tab strip — the same component, the
  same look and the same click as the map pane's layer tabs.
- **Double-click submits, in the command band** — a second click on the same
  word row within the double-click window fires the composed prompt, so the
  last word of a phrase goes straight into the game: click `open`, double-click
  `mailbox`. The first click of the pair picks the word as always.
- **Tab toggles focus in the IFDB search modal** — `Tab`/`Shift-Tab` hop
  between the `Search:` field and the results list, keeping the half-typed
  query and the list selection intact. Typing over the list already dropped
  you into the query editor; this is the way back that isn't `Enter` (an
  unwanted search) or `Esc` (leaves the modal / falls down its ladder).
- **`merge-layer` takes a target** — `merge-layer <name>` folds the active layer
  into any named layer, not just the one it was peeled from (`merge-layer main`).
  This closes a real trap: a room discovered while exploring a maze layer is
  minted onto the maze layer even when it belongs to the surface, and the bare
  merge could only round-trip it back into the maze. Peel the stranded region,
  then merge it home in one step. Merged rooms keep their map positions where
  free and take the nearest free cell where not; an unknown or ambiguous layer
  name refuses with a message and moves nothing.

### Fixed

- **fmvpoker's frame stops having a hole punched in the top of it.** Frobozz Magic
  VideoPoker draws its poker table with Zork Zero's artwork — the original ships that
  picture file renamed — so the frame's top-centre tab natively reads *Double
  Fanucci*, a title belonging to a different game. fmvpoker hides it the way a v6
  game does: it parks a window exactly over the banner and erases it to the blue it
  declared for that window. babelmap recorded the erase correctly and then flooded
  window 0's page straight over the top of it, and window 0 here is the entire
  screen — so the tab came out as a white gash across an otherwise complete blue
  frame, which is what three passes at this had recorded as artwork being clipped at
  the top edge. Nothing was ever clipped. The story page is now the oldest thing in
  its box: it fills under the game's own `erase_window` fills, exactly as it already
  filled under the labels a game prints inside window 0. Every other v6 title is
  byte-identical.

- **THE BAT's title page stops putting rooms on the map.** The game opens on an act
  list — *Prologue • ACT I • Interlude • …* — and then a prologue headed *Excerpted
  from the New Gothenburg Post:*, and the automap had drawn both as rooms before the
  player had typed a single command. Neither is a place; they only look like one,
  because Inform bolds a room heading and a title with the same style. babelmap now
  reads the shape of the page rather than the words on it. A room heading is joined
  to the description printed directly beneath it, and the turn that prints one ends
  by handing you the command prompt; a banner stands alone above a blank line on a
  page that ends by asking you to press any key. Both halves have to agree, and each
  is load-bearing: Adventure in `superbrief` prints a room as a bold line, a blank
  line and a list of what's lying about, which the first half alone would discard,
  while a room you really did walk into can perfectly well be followed by a cutscene
  that ends on a keypress. Kerkerkruip's screen-reader question, whose *Enable* was
  bold and at the start of its line, stops being a room too.
- **advent.z6's help bar stops losing letters, and wide terminals stop clipping the
  line.** Opening `help` in Adventure showed a navigation bar reading
  `N   n xt subj ct` and `RETURN = r ad subjec` — the `=`, three lowercase `e`s and
  the tail of "subject", gone. It looked like a font problem and was arithmetic. A
  line of v6 status text is *positioned* by the game's own pixel coordinates but
  *drawn* one terminal column per character, and those two only advance at the same
  rate when the pane happens to be one column per 8-pixel game cell. Widen the
  terminal past that and they drift: at 120 columns a game cell is a column and a
  half, so the blank cells the game paints across the bar — harmless where they
  are, sitting over the label's own spaces — landed on its neighbouring *letters*
  instead and wiped them, and the blank just past the end of a label reached back
  inside it and took the last character with it. Blank runs now paint only the
  cells no text claimed, so the bar reads whole at every terminal width.
- **fmvpoker draws its poker table in `hybrid` mode, instead of nothing at all.**
  Frobozz Magic Videopoker paints a full-screen frame and prints its title, bets
  and winnings inside it; hybrid showed all of that text on a blank white page with
  not one picture anywhere. Hybrid shows artwork as a ring *around* the story text,
  so a game that grows its story window over the whole screen leaves no ring to
  draw in and nothing can be shown — which is why such screens are handed to the
  full-picture composite instead. That handover asked whether the art *filled* the
  screen, and fmvpoker's table is a frame with a hollow middle: 17% painted, and
  missed at every point the test looked. It now also recognises art that *encloses*
  the screen, so the table arrives with the text still on it. Every other v6 title
  renders exactly as before.
- **The pixel composite stopped skipping a second text window.** A v6 game can run
  more than one scrolling text window, and the composite drew graphics and status
  grids and simply ignored those — so fmvpoker's bottom menu and its "Select an
  option with your mouse or by typing the first letter." hint were missing from
  `raster` mode entirely, on a screen where the terminal-cell paths showed both.
  They are drawn now, in the game's own ink where you are honouring game colours
  and the theme's where you are not, and the story page no longer paints over them.
- **A v6 menu bar keeps its columns instead of running into one word.**
  fmvpoker's bottom bar read `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT`
  — five options with nowhere to break. The game places each label at its own
  pixel column and prints them onto one row, and a second text window kept its
  text as plain lines with no note of where each run began, so every label simply
  followed the last. It now keeps the column the game named for a run and pads the
  line out to it, exactly as the main text window already did, and the bar reads
  `PLAY CURRENT BET  CHANGE CURRENT BET  SAVE  RESTORE  QUIT` again. Anything the
  game centres in such a window — fmvpoker's `CONTINUE` button — lands centred too.
- **The Mysterious Adventures now draw a map instead of nothing at all.** All
  eleven of Brian Howarth's games — Scott Adams adventures rebuilt as v6 Z-code —
  played from start to finish with a completely empty automap, even though every
  turn repaints "I'm in a dense SPOOKY Forest / Obvious exits: NORTH SOUTH" in
  plain sight. They defeat every way babelmap knows to find a room, all at once:
  the player is never put into the object tree at all, every room object carries
  the same compiled name (`ScottRoom`), and the line you read lives in a property
  where no name match will ever find it. babelmap now takes the room from the
  variable these games keep it in — but only after confirming that the room it
  points at is carrying, in its own properties, the very words on screen that
  turn. The screen and the object tree have to agree before either is believed,
  which is what makes the answer an exact room rather than a name: these games
  reuse a description across whole mazes, and ten rooms that all read "I'm in a
  Tunnel" now map as ten rooms. Nothing changes for any game that already found
  its rooms — the new check runs only where babelmap previously found none — and
  the automapper's own probing can no longer fault the story it is reading.
- **Text that vanished from `raster` mode in four v6 games.** Three separate field
  reports, one mistaken assumption: the pixel composite kept using "is this pixel
  opaque?" to mean "is there artwork here?", and rasterized glyphs are opaque too.
  - **Shogun's title showed no prose at all.** Its menu is printed *inside* window
    0's four-row box, and measuring the box against those glyphs shrank it to a
    single row — too little for one line and the caret together. The room a story
    window has is now measured against the artwork alone, so the prompt and the
    menu share the rows the way they do on an Amiga; window 0's page is painted
    *under* the labels other windows put in its box rather than over them. **Journey**
    had the same box shrink to zero height — the screen-wide fill that closes the
    bare cells of a reverse-video bar was running across its text panel — so its
    narration was missing from `raster` too, and is back.
  - **advent's help screen lost its whole navigation bar.** "About Adventure",
    "N = next subject", "RETURN = read subject" render fine as cells and were
    simply absent from the picture. The game paints the bar as one run per label
    plus reversed spacer spaces, and a spacer lands inside "About Adventure" — so
    the header saw the spacer's own highlight block, decided it was sitting on
    frame art (where a block would erase the picture), dropped its block and drew
    itself in the page colour on the page. The over-art test now reads the art
    layer, frozen before a single glyph is stamped.
  - **fmvpoker showed no text whatsoever** — a correct blue frame around a blank
    white interior. Its poker table is a 640×400 picture that is mostly *hole*, and
    a full-window picture in the story window is normally a plate the game draws
    instead of prose (Arthur's illustrated screens). Measured by its bounding box
    the frame owned the screen; measured by the pixels it actually paints, the
    largest clear rectangle it leaves is exactly where the game prints. Arthur's
    plates are dense enough to still own theirs.
  - And a cleared screen now starts at the *top* of the story window in `raster`
    as it always has on the cell paths, so Shogun's four-row box shows the line the
    game printed into it instead of redrawing the tail of the banner it had just
    frozen up top.
- **Shogun's "You may choose to:" now sits beside START/RESTORE/QUIT, not under
  the title.** The game prints its nine centred banner lines while window 0 is the
  whole screen, then moves window 0 down to a four-row box level with — and to the
  left of — its boot menu, and prints the prompt there. babelmap already froze the
  banner where it was painted and already held the right box; it just started the
  resumed transcript flush under the banner and let it flow, so the prompt landed
  nine rows above the menu it belongs beside and scrolled away with everything
  else. The story window's box now says where its transcript starts on every
  presentation, cell paths included: the gap a game leaves between its chrome and
  its story window carries through into your pane, and a menu painted inside that
  box — items and the ground erased under them — travels with it. Measured against
  the chrome's declared rectangle rather than the text in it, so a status panel
  taller than its own two lines (Zork Zero's) still keeps the transcript exactly
  where it was; Arthur, Journey and Adventure render byte-for-byte as before.
- **Selecting a card in scopa no longer smears the OK button across the table.**
  Choosing a card relabels the confirm button from "Choose" to "OK", and the label's
  white field came with it — running out of the button's rounded outline and off the
  right edge of the screen. scopa prints every button label into one scratch window
  it shoves around for each draw, and by the time the screen is composed that
  window's box is a leftover 1000×1000 measurement clamped to the screen, so it
  describes nothing. A text row that names its own background is padded out to its
  window's edges so a status band printed in pieces (Shogun's location and score
  bar) still reads as one solid bar — but only now when the row's text actually
  reaches those edges. A two-letter label with forty-five pixels of nothing beside
  it is a label, not a bar, and stays the size the game drew it.
- **A v6 game that splits its screen for artwork no longer prints the story over
  the picture.** `mysterious01.z6` reserves the top 260 pixels for its illustration
  and then simply narrates — it never repositions the text window, because the
  Z-machine standard says splitting the screen *tiles* the two windows: the upper
  one takes the height it asked for, and the story window "is placed just below
  it". babelmap only shortened the story window and left it pinned to the top
  corner, so it sat squarely inside the picture and the prose came out printed
  across the artwork. The split now places the story window where the standard puts
  it, and the picture and the prose each get their own half of the screen. Adventure
  benefits twice over: its library asks the interpreter where the split left the
  text window and positions its own from the answer, so its status bar, its room
  description and its `help` menu all now land where the game intended — the subject
  list used to be buried under a text window that still claimed the whole screen.
  Zork Zero's full-screen title splash is untouched: a split that takes the entire
  screen leaves the story window with no height at all, exactly as the standard
  describes.
- **Shogun's title header stays centred outside raster mode.** The nine centred
  lines the game paints across its title screen are frozen where it printed them,
  and the full-frame `raster` composite placed them perfectly — but `hybrid` (the
  default) and `frameless` route text above the story through the status-bar
  renderer, which sorts a line into a left, centre or right field by where it
  *starts*. That is the right question for a status bar and the wrong one for a
  paragraph: five of the nine lines began far enough left to be flushed against the
  left margin and the shortest ended far enough right to be flushed against the
  right one, so a carefully centred block came out ragged on both edges. A line
  with equal margins on the game's own screen was centred on purpose, so it is now
  centred in your pane too, at any terminal width. Status bars are untouched — a
  field that begins at the screen edge is still anchored there.
- **All three of scopa's card decks now show up, at the size they were drawn.**
  The opening menu invites you to click a card type to begin, and only ever offered
  one: the Milanese deck hardwired into the z-code. The Neapolitan and Sicilian
  decks live in the game's Blorb, and scopa draws every one of those pictures
  through a scratch window it borrows for a single instruction — move it, size it
  to 1000×1000, draw at the corner, move it straight on for the next card. By the
  time the renderer looked, that window had gone somewhere else and shrunk to an
  80×1 sliver, so both photographic decks were clipped out of existence and then
  erased by the next fill. Pictures now record the window box they were drawn into
  and freeze onto the screen where they landed when the window moves on — the same
  rule that already keeps a moved window's prose where it was printed. They are
  also drawn at their real size: scopa's Blorb declares no standard window, which
  the Blorb spec defines as "display at actual size, one image pixel per screen
  pixel", so doubling them (right for every Infocom v6 title, all of which do
  declare one) had told the game its cards were twice as big as they are and
  produced a menu row that overlapped itself and hung off the bottom of the screen.
  Pick a deck and the whole hand — table, hand, backs and all — now deals in it.
- **Turning game colours off no longer deletes half of a v6 game's board.** With
  `honor_game_colours` off, scopa's felt table disappeared and left a black
  card table with two green stripes across it — the only survivors being the bands
  the game had drawn on top of the felt. The table was never a colour preference:
  scopa sizes a window to the whole screen, names an explicit green and erases it,
  which is the same drawing operation that paints its cards, and only reaches the
  renderer as a window background because a full-screen erase is treated as a
  screen clear rather than as paint. So the felt is back whichever way the setting
  is thrown: a window the game has *drawn into* keeps the ground it drew on, while
  a window it merely coloured still defers to your theme, and the story window —
  the surface you actually read prose on — is governed by the setting exactly as
  before. Zork Zero, Arthur, Shogun, Journey and Adventure paint no ground at all
  and are untouched.
- **A v6 game with no story window now draws in hybrid mode too.** scopa's card
  table never streams prose — its whole screen is painted rectangles with a couple
  of buttons on top — and hybrid mode, which builds a picture frame *around* a
  terminal transcript, had no transcript to build around. It fell back to the path
  meant for hint menus, which presents a screen as plain positioned text: the two
  button labels arrived, seven characters in an otherwise empty pane, and the cards
  did not. Now a screen the game has painted goes to the full-picture composite
  whichever render mode you are in, so hybrid shows the table exactly as raster
  does. Genuinely text-only screens — Zork Zero's InvisiClues, Shogun's boot menu —
  are untouched and still come up as crisp terminal text.
- **A v6 game that measures text no longer shrinks its own screen.** Deal a hand
  in scopa and the whole table zoomed out — the cards crammed into a corner with
  big black rectangles beside them. The card game was not drawing any of that: to
  find out how wide a string is, it opens a scratch window 1000×1000 so the string
  cannot wrap, prints into it, and reads the width back. babelmap sized the
  composite to cover every window the game had open, so that one measuring window
  — two and a half times wider than the screen — decided how big the picture was,
  and everything real shrank to fit inside it. Now a window is drawn only where it
  exists: each box is clipped to the screen the story itself declared before
  anything is composited. What the *game* sees is untouched — it still reads back
  the size it asked for, which is the entire point of the trick it is pulling — so
  the measurement stays correct while the picture goes back to filling the pane.
  `/dump-windows` now says both, the size the game set and how much of it is on
  screen.
- **Shogun's title screen keeps its header where the game painted it.** The nine
  centred banner lines are printed while window 0 still *is* the whole screen;
  the game then drops window 0 to a small box at the bottom, beside its
  START/RESTORE/QUIT menu, and prints "You may choose to:" there. The Z-machine
  standard says moving a window changes nothing already on screen — so on the
  original the banner stays up top. babelmap streamed both halves into one
  transcript, which jammed the prompt under the banner and then scrolled the
  banner out of a four-row box. Prose now freezes where it was printed the moment
  its window moves out from under it: the banner becomes paint at the exact rows
  and columns the game chose, and the transcript starts again at the window's new
  origin, so the opening reads the way it does on an Amiga. Nothing is deleted —
  the frozen lines stay in scrollback. Prose a window is merely resized *around*
  keeps streaming as before, which is what every turn of Arthur does.
- **A backdrop that fills the screen is no longer mistaken for a drop-cap.**
  Frobozz Magic Videopoker came up with its card table missing — some graphics,
  no outline — and Journey's title illustration never arrived at all. Both games
  clear the screen and then paint a full 640×400 picture at its top-left corner,
  and clearing the screen is also what puts the text cursor there, so the picture
  looked exactly like one of Zork Zero's illuminated drop-caps: drawn on the
  current text line, meant to have prose flowing beside it. It got floated into
  the transcript and the screen never received it. babelmap now asks the question
  a float actually turns on — *is there room left beside it?* — and a picture
  spanning the window from edge to edge answers no. The table, the JOURNEY splash
  and the Mysterious Adventures' title cards all land on the screen now, with the
  story text over them. Zork Zero's drop-caps and room icons and Shogun's opening
  ship are untouched: the widest of those still leaves nearly half its window free.
- **Arthur's opening illustrations are no longer scribbled over.** The sword in
  the churchyard, and Merlin rising out of it, came up with the previous screen's
  narration rasterized straight across the artwork — a wall of text over the
  picture, unreadable in both directions. Arthur never asks for that: it clears
  the screen, draws the plate, hides the cursor and waits for a key, and its
  narration is a *separate* screen it erases before the next illustration goes up.
  The whole graveyard-to-Merlin turn prints not one character. babelmap was
  painting its own scrollback onto the plate. Now a placed picture that leaves no
  column wide enough to wrap prose into owns the screen outright — exactly as a
  window-filling picture already did — so the illustration ships alone, in both
  `hybrid` and `raster`. A picture that *does* leave a real column, like a margin
  illustration, still gets prose beside it.

- **Shogun's title screen is centred again — because Shogun centres it.** The
  header that opens Shogun (`SHOGUN`, `A Story of Japan`, the copyright block)
  arrived jammed against the left margin. Shogun does the centring itself, in
  pixels: for every line it reads its own window's width, works out the centred
  column, and moves the cursor there — then prints the text with no leading
  spaces at all. The centring was never in the text, so streaming the text and
  dropping the cursor column lost it entirely. babelmap now carries a declared
  column into the transcript as an indent; at the v6 cell width of 8 pixels the
  two measurements are the same one, and every line lands exactly where the game
  worked out it should. Journey's title screen, centred the same way, comes
  right with it. Games that declare nothing are untouched: Arthur only ever
  moves the cursor to switch it on and off, and Zork Zero only ever asks for
  column 1.

- **Arthur's intro illustrations actually appear — where Arthur put them.** The
  three plates that open Arthur (the sword in the stone, the churchyard, Merlin)
  never rendered at all. Arthur lays those screens out itself: it clears every
  window, asks window 0 how big it is, centres the 584×392 plate by hand at
  x=29, y=5, and narrates over it. babelmap treated *every* window-0 picture as
  an inline drop-cap — the Zork Zero idiom, where the art is drawn on the text
  cursor and has to scroll with the paragraph beside it — so Arthur's backdrops
  were pushed into the transcript as floats, no window canvas was ever made, and
  the art never rasterized. The two plates of the Merlin screen would also have
  stacked as separate bands instead of compositing, losing the effect of Merlin
  appearing *on* the graveyard. The engine now records whether a picture was
  placed on the window's current text line or at a position the game chose, and
  a placed one gets a real canvas at the pixel origin the game named, with later
  draws compositing into it. The margin Arthur deliberately left around each
  plate stays the page — the art is not stretched to fill it. Drop-caps, room
  icons and Shogun's margin-parked ship are untouched: all three are drawn on
  the cursor, and still float with the prose.

- **Zork Zero's room icons stop sitting on a black box.** The little compass and
  room icons in Zork Zero's banner are line art on a *clear* ground — 95% of
  each 45×40 picture is fully transparent — and the bottom of every one of them
  hangs below the banner artwork, where the game had painted nothing at all.
  Nothing of ours decided what the player saw there, so the graphics protocol
  decided instead, and its answer was black. The Z-Machine Standard is clear
  that it was ours to decide (§8.8.3.2: every Version 6 window has its OWN
  foreground/background pair) and Zork Zero's banner window says white, like the
  DOS original. babelmap now paints each chrome window's own page into the
  pixels no layer touched, so the ring it ships is self-contained instead of
  leaving holes for the terminal to colour in. Only untouched pixels are filled:
  artwork, status bands, glyphs and the icons' own ink are left byte for byte
  alone, the story area stays clear for the transcript, and a window the game
  gave no colour keeps exactly today's look. `/set-game-colours off` opts out as
  usual. The same rule gives Scopa its green baize back.
- **The status bar stops painting a black band on a light terminal.** With no
  `style.toml` and no colour scheme configured, babelmap's UI surfaces — the
  status bar, the v4+ upper window, story info, dialog backgrounds, the Glk grid
  styles — were drawn white-on-**black**, regardless of what colour the terminal
  actually is. It was never the game's doing: Anchorhead, the story it was
  reported on, sets no colours at all. It was ours. "No scheme configured" left
  the theme's `chrome` role with nothing to derive from, so it fell back to a
  hard-coded black page — a guess that happens to be right on a black terminal
  and wrong everywhere else, laying a band across the top of the screen. babelmap
  already asks the terminal for its real default colours at startup (the OSC
  10/11 probe that keeps the v6 raster canvas honest); that answer now reaches
  the theme as well, so the unconfigured look follows your terminal instead of
  overriding it. Terminals that don't answer the probe keep exactly today's
  behaviour, a half answer is declined whole rather than mixed into a probed ink
  on a guessed page, and a scheme you *did* choose is never second-guessed — as
  is a game that sets its own page colours, which still wins the grid outright.

- **The upper window's frame answers to `style.toml` again.** `upper_window_border`
  could be recoloured but not reshaped: its `style` / `style_top` / … keys were
  read straight past. The one place that applied them was the retired `[colors]`
  table, and the `style.toml` babelmap seeds has no `[colors]` section — the
  selector lives in `[elements]`, where its border keys parsed into the theme and
  stopped there. So `upper_window_border = { style = "none" }` sat in the file
  doing nothing, on Anchorhead and every other v4+ story. The frame's shape now
  travels from the file to the renderer, which both draws it and reserves its
  rows and columns, and the seeded template finally documents the spelling
  instead of showing only the colour form.

- **Quote boxes are readable again.** The Inform `box` statement — the framed
  reverse-video epigraphs a great many games open with — splits the upper window
  tall, prints into it, then shrinks it back to the status line *before* waiting
  for the keypress that is meant to display it. Truncating the grid at that
  shrink destroyed the quote before it could be read, so Anchorhead's two
  startup quotes (the Lovecraft epigraph beside the title, and
  `* THE FIRST DAY *`) rendered as blank screens waiting for a key. A split now
  shrinks the split height but keeps what was painted, so the box stays in the
  upper window exactly where the game placed it — drawn in the story pane, in
  the CLI's pinned region, and read aloud in `--screen-reader`, where a region
  taller than one row counts as content rather than quietened chrome. It is
  retired when the player next acts, which is the "scroll away over the next few
  command inputs" a real screen gave it. Fixed in the VM, so every front-end
  gets it; the per-turn status-line re-split is untouched.
- **The drawn map's one arrow rule: every arrow on a room border is that room's
  own exit.** A one-way passage used to stamp an inbound arrow on its destination
  (worst at a diagonal, where the side-derived `▶` landed on a box corner and read
  as an exit that does not exist — Zork I's Deep Canyon). The far end of a one-way
  line is now bare; the departure arrow and the line ending on the box carry the
  reading.
- **A passage collapsed into a shared line is stamped, not hidden.** When two
  rooms are joined by both a compass edge and a staircase, one line is drawn and
  the other passage used to vanish entirely — Zork I's Chasm knew its way back
  (`up` to the East-West Passage) and the map showed nothing. Each collapsed
  passage now stamps its own glyph (`↑`/`↓`, or its compass arrow) on the border
  of the room it departs from, beside the line it shares.

### Changed

- **A game's status/upper window is no longer boxed by default.** The single-line
  frame babelmap drew round it is off out of the box: the status line sits flush
  against the story, and the two rows and two columns the frame was costing go
  back to the game's own screen. Put it back — in any style, or one edge at a
  time — with `upper_window_border = { style = "single" }` in `[elements]`.
- The matrix view's tried-but-pathless cell is `×` rather than `_` — a mark
  centered in the cell instead of one hugging the baseline, where it read as
  an empty cell with an underline artifact. `·` (untried) is unchanged.

---

## v0.1.0-beta.4 — 2026-08-05

### Added

- **The matrix map view** — mazes finally have a representation that tells the
  truth. Any layer can switch between the drawn map and a **direction matrix**
  (`/view-map`): one row per room, a column for every direction, each cell
  saying exactly what is known — mutual passage, goes-there-returns-elsewhere
  (with the return direction), one-way, self-loop, tried-but-flat, or untried
  frontier. Selecting a room bolds its known entrances; identically-named
  rooms number themselves; the table thins its cells before it scrolls. A
  layer marked as a maze (`/mark-maze-layer`, or accept the offer babelmap
  makes when it notices a tangle) defaults to the matrix. Self-loops — "west
  leads back here" — are now recordable at all, one-way passages grow
  arrowheads on the drawn map, and the room panel gains the full per-direction
  exit card (retiring the explored rose and untried-exits list it replaces).
  Designed against a real player's half-mapped Colossal Cave maze, which now
  lives in the test suite. → [mapping](docs/features/mapping.md)
- **The command band** replaces the verb menu (which was a left-edge token
  palette nobody could drive). Modeled on Journey's clickable menu system: a
  bottom band (`F2`, or `open-command-band`) whose columns fill in
  left-to-right as a phrase narrows — verb, then the objects actually here
  and carried (live from the engine, refreshed every turn), then the
  preposition column when the verb wants one. Everything is clickable, letters
  filter the active column, nothing sends without Enter, and the band is a
  dock rather than a modal: the prompt stays visible, paste works, and
  graphical v6 keeps its pixel path. Verbs and their grammar are configurable
  via `[command_band]`.

- **Screen-reader mode** — all three CLIs take `--screen-reader` (alias
  `--plain`), and select it automatically under `TERM=dumb`. It emits no
  escape sequences at all, hands line editing and echo back to the terminal, and
  drops the `[MORE]` pager. `NO_COLOR` is honoured separately, as colour only.
- **`/status`** — a host command, at any line prompt in any of the three CLIs,
  that repeats the current status without the game seeing the command.
- **Score announcements** — in `--screen-reader`, a score that changes is
  announced above the prompt (`[Score 1, up 1]`), since quietening the status
  line otherwise takes the score with it. Exact for Z-machine v1–v3 (a global the
  standard reserves) and for Scott Adams (treasures deposited); recovered from
  the status text for v4+ and Glulx, where no score is exposed to the
  interpreter at all.
- **`[MORE]` paging in `gvm-cli` and `scott-cli`** — previously only `zvm-cli`
  paused at the bottom of a page, so a Glulx game with a long turn scrolled
  straight past; an ordinary Glk library pauses a text-buffer window the same
  way. All three now take `--no-more` (alias `--no-page`), page only when both
  ends are a terminal, and never page in `--screen-reader`.
- **`--show-status`** — narrate the status line whenever the story updates it.
  Off under `--screen-reader`, because a Z-machine v3 status line carries a move counter
  and so changes on every single turn.
- **Menus read as menus in `--screen-reader` mode.** A menu is a rectangle the
  game repaints, so linearised it used to re-read itself in full on every
  keypress — sixteen lines a press at Planetfall's InvisiClues menu, twenty-three
  at Arthur's, fifteen at Counterfeit Monkey's `ABOUT` — to say that a `>` had
  moved down one row. `zvm-cli` and `gvm-cli` now read a menu out **once**,
  host-numbered under a `[menu — type a number to jump, Enter to select]` line,
  and announce each move as `>3. THE DORMITORY (3 of 12)`. Typing a number jumps
  to that item: the host walks the menu with the game's own keys (`n`/`p` when
  the legend names them, else Down/Up), steering by where the marker actually
  landed rather than by a press count, because Arthur's `N` steps over its
  section headings. **`/menu`** re-reads the open menu on demand — at a menu's
  own prompt as well as a line prompt, since screen-reader mode leaves the
  terminal cooked and a keypress there is a whole line. Detection is a
  mechanical diff: only a block that differs from the last one *solely* in
  marker position is treated as navigation, so a status line that changed, a
  menu that scrolled, or a form that gained a field is still emitted in full.
  Nothing outside `--screen-reader` changes — piped and terminal output are
  byte-identical, verified across 13 Z-machine and 9 Glulx stories.
  → [interpreter](docs/features/interpreter.md)

### Changed

- **`--no-status` is now `--story-only`** (`zvm-cli`; `--lower-only` remains an
  alias, and the old spelling still works with a notice). It reads too much like
  what `--plain` does to the status line while being stronger — it suppresses the
  whole upper window, menus and forms included. `gvm-cli` gains the same flag.
- `gvm-cli` renders Glk grid windows as inline text when there is no TTY; they
  were previously tracked and then dropped, losing the status line from piped
  output entirely.

### Fixed

- **Unknown command-line options are now an error** in all three CLIs, naming
  the option, printing the help, and exiting 2. `zvm-cli` and `gvm-cli`
  previously ignored them — a mistyped `--no-statu` did nothing and exited 0 —
  and `zvm-cli` took an unknown single-dash argument such as `-x` for the story
  path. A missing option value and a second positional argument are errors too.
- **A full-workspace code review closed forty-odd defects** (SQ-0619–SQ-0661), the
  themes being:
  - *Hostile files can no longer crash or hang the host.* Illegal Z-machine
    instructions latch a fault instead of panicking; crafted stories, saves,
    blorbs and dictionaries that used to trigger unbounded recursion, multi-GB
    allocations, out-of-bounds indexing or infinite sibling walks are rejected
    or clamped in all three VMs; restored Glulx save data — stack frames, the
    Glk window tree, the heap block list — is structurally validated.
  - *Nothing overwrites a good file with a bad one.* Every persistence write
    (archive, config, saves, sidecar stores, downloads, exports) goes through
    one atomic temp-and-rename helper; a config that is valid TOML but has a
    wrongly-typed value no longer loads as defaults and then rewrites the
    user's file to defaults on the next save; the exit watchdog waits for an
    in-progress save.
  - *Save/restore honesty across engines.* A host restore over a suspended
    in-game `@save`/`@restore` abandons the old suspension in every engine
    (Glulx replayed the snapshot's last command as a free turn; the Z-machine
    recorded a discarded PC into the next save); a resize or a finishing sound
    no longer silently fails the save dialog the player is sitting in; v6
    `@restart` no longer replays pre-restart art on the next palette change;
    Quetzal v6 saves drop the dummy stack frame per §4.11.
  - *The terminal is treated like the shared resource it is.* Worker-thread
    panics no longer tear down a live session's terminal; kitty image ids are
    deleted when their windows close or resize instead of leaking; layered v6
    chrome art is cached instead of re-uploaded every frame; the CLIs accept
    only key presses (Windows doubled everything), reset the scroll region on
    every exit path, and enable VT processing on Windows.
  - *Text is unicode, everywhere it wasn't.* Typed non-ASCII input reaches the
    Z-machine as ZSCII instead of raw UTF-8 bytes; room notes, save
    timestamps, IFDB titles, selections, caret placement and field editing are
    char-, width- or grapheme-aware instead of byte- or column-indexed.
  - *The map's layers behave.* Peeling cuts only the true reciprocal edge,
    merging survives a deleted parent layer, a room can hold more than one
    non-compass passage, and Scott Adams noun resolution matches ScottFree
    (location-aware auto-get, the two-bottles problem).
  - *Styling is honest.* A whole family of parsed-but-dead style.toml keys
    (border sides, glyphs, dialog shadow/placement) now resolves; choosing a
    colour scheme no longer flips border structure; the modal selection,
    footer, inspector and search-highlight styles are themeable selectors
    instead of hard-coded colours.

---

## v0.1.0-beta.3 — 2026-08-03

Fifty commits, and most of them are about honesty: a saved game that restores into a
different terminal, a different graphics backend or a recoloured scene now shows what
it should rather than something that merely looked right when it was written. The
command-line players got the same treatment — `gvm-cli` learned to render a game's Glk
windows as the panels they are, and `zvm-cli` learned to say no to the v6 stories it
was never going to be able to drive. Along the way the map pane stopped stealing your
keyboard.

### Added

- **The IFDB download chooser tells its candidates apart.** Each file now carries
  IFDB's own description — "Release 16: latest version of the game.", "Competition
  version" — which is frequently the only thing that distinguishes two entries: IFDB
  lists two different *Photopia* builds under the identical filename `photopia.z5`. A
  file the library already holds is marked `✓ … · already downloaded`, and the chooser
  now opens even when a game offers a single file, so that mark is always visible
  before you fetch a duplicate.
- **`glk_pixel_scale`** — a Glulx game asks how big a character cell is in pixels and
  sizes its drawings from the answer. Reporting the terminal's true cell made
  *Adventure*'s toolbar render a third of its intended size on a HiDPI display.
  `native` (the default) keeps the honest answer; `auto` normalises the cell to a
  reference height so a game's pixel space scales with the font; `fixed = n` pins the
  divisor by hand.
- **`gvm-cli` renders each Glk buffer window at its own rect.** Games that lay their
  UI out in several windows — *Kerkerkruip* puts its inventory and status panels in
  six of them — used to have every panel's text dumped into the story stream, so
  "Health: 18 of 18" appeared inline in the prose. Windowed rendering engages only
  when a game actually uses more than one buffer window; every other game keeps the
  streaming path, and the terminal's own scrollback with it.
- **A second v6 prose window gets its own buffer**, so a game that streams narration
  through a window other than the main one keeps both readable.
- **`/dump-windows` describes a v6 story's real layout**, one block per window, and
  the render path is logged and stamped — including *why* the pixel path was skipped
  on a given frame, which is the question that actually comes up.
- **Compass clicks map the direction travelled**, so clicking a room's rose records
  the passage you took rather than the one you aimed at.

### Changed

- **The map pane no longer takes the keyboard.** `Tab` used to hand focus to the map,
  and with the map focused an arrow key panned instead of moving the command-line
  caret — with nothing on screen to say which mode you were in. Every keystroke goes
  to the story now. `Shift+Arrow` pans (as it always did), the mouse pans, zooms and
  selects, and zoom and centring moved onto the `Ctrl+P` leader panel's new **Map**
  group (`+`/`-` zoom, `0` centre). `Tab` still steps the debug inspector's windows,
  and is only advertised when the inspector is open.
- **Manual layout mode and room nudging are gone.** Both were permanent no-ops:
  nothing outside the test suite ever set manual mode, so `nudge-room` and its
  `F6`–`F9` keys could not move a room in any real session, and the refusal was
  silent. `F6`–`F9` now reach a story like any other function key. Room positions
  belong to the layout engine — re-run `tidy-map`.
- **`zvm-cli` declines graphical v6 stories** instead of accepting ones it cannot
  drive. Measured across every v6 story available, each one runs away at its first
  input prompt whatever key it is given: *Zork Zero* and *Arthur* flood the terminal,
  *Shogun* spins silently with nothing to interrupt. `zvm` itself supports v6 fully —
  play those in babelmap.
- **OS and C-library noise stays off the screen.** ALSA and friends write straight to
  file descriptor 2, which no Rust-side hook can intercept, so their messages landed
  mid-frame and corrupted the display. While the alternate screen is up, fd 2 goes to
  `<user_dir>/stderr.log` instead.

### Fixed

- **A restored game now shows what it should.** Quetzal saves no screen state by
  design — the standard assumes the *story* repaints — but a host Save State swaps
  memory under a game that never learns it happened, so everything the screen needs is
  ours to carry. A v6 archive now stores each graphics window's **display list and
  palette** rather than a snapshot of the pixels, so restored art follows a later
  recolour instead of freezing at the colours it happened to have; it is carried on
  every save path, not just auto-save. A restore **refits the saved screen to the
  terminal you restore into**, which a restore into a different size always was. And
  the archive is backend- and terminal-neutral, so a save moves between kitty,
  half-blocks and sixel.
- **Counterfeit Monkey starts in under a second** (5.4s → 0.76s from the second
  launch). Two faults: `@restore` read a fileref *name* instead of the stream it was
  handed, making a restore from a resource stream impossible for any game; and the
  blorb's own embedded save, whose identity chunk disagrees with the executable beside
  it, was being offered and then rejected. A save belonging to another story is no
  longer advertised, so the game takes its working file-cache path.
- **Graphical v6 rendering**, throughout: a status bar paints inside its own window
  and stays one row deep at any pane scale; prose follows the window the game actually
  streams through rather than window 0; `erase_window`'s background fill is tracked so
  menu panels are opaque; the chrome ring keeps off a secondary prose window's rows and
  re-uploads correctly when a band set changes, a terminal clears, or the pixel path
  resumes; and a full-screen picture takeover no longer mangles the transcript, the
  pager or the composite cache.
- **`gvm-cli` display correctness**: a text grid paints its window background across
  its whole rect rather than only behind the glyphs it drew; `window_clear` redraws a
  screen in place, so a menu updates instead of appending a fresh copy per keypress; a
  grid that shrinks stops repainting the rows it gave up; live input echo carries the
  window's own styling; and the page background is taken from the window tree, which
  is where a game that sets its colours per window actually records them.
- **Neither CLI hangs or panics on input it cannot use.** `zvm-cli`'s line counter was
  a `u16` incremented without check — any story printing 65,536 newlines without a
  pause panicked, which is how *Zork Zero* died in under twenty seconds. `gvm-cli`
  threw away `read_line`'s result, so end-of-input was indistinguishable from a blank
  line and a piped session looped forever.
- **A malformed `config.toml` no longer silently erases itself**, and notification
  toasts anchor to the transcript viewport rather than the story pane rect.

### Save format

- **`.babelmap` archive `format_version` 5 → 6.** A v6 archive now carries
  `display.json` — each graphics window's display list plus the Blorb §11.3 palette —
  and omits the canvas PNG for any window whose replay reproduced the live canvas at
  save time. Archives written before the bump still load and take the PNG path, which
  is this build's fallback anyway; a version-6 archive is rejected by older builds, as
  the format freeze intends. Bare Quetzal / Glulx-Quetzal interchange files are
  untouched. See
  [`docs/release/save-format-policy.md`](docs/release/save-format-policy.md).

### Known issues

- **`zvm-cli` cannot play graphical v6 stories at all** — it now says so at load
  rather than hanging. Play them in babelmap, which renders v6 graphics and menus.
- **Room selection lost its keyboard shortcuts.** `select-room next|prev` was bound to
  `n`/`p` only while the map held focus, and with that focus mode removed the command
  is reachable by clicking a room, `/select-room`, or the command palette.
- All beta.2 known issues still stand: **sub-cell buttons in a graphics window can't
  be clicked**, **a v6 game's own erase can take neighbouring art with it**, and the
  three v6 caveats from beta.1 (**Inform-compiled v6 status lines don't paint in
  `raster` mode**, **rasterized v6 text isn't selectable**, **sixel encode latency on
  very large panes**). `hybrid`, the default, avoids the beta.1 three.

---

## v0.1.0-beta.2 — 2026-07-29

Ninety-odd commits on from the first beta, most of them spent making the graphical v6
support that shipped in beta.1 actually behave: its screen model is now rebuilt against
ZMSD §8 rather than approximated, palettes adapt the way the Blorb spec says they
should, and the games that ask for mouse input get it. Alongside that, the map stopped
implying passages it has never seen, the Glulx mapper learned to identify rooms the way
the game itself does, and `config.toml` learned to explain itself.

### Added

- **Switch v6 render modes live** with **`/set-v6-render`** — cycle or name one of
  `hybrid` (crisp terminal story inside a scaled pixel frame), `raster` (the whole pane
  as one image) or `frameless` (no frame at all — full-pane text with a status band and
  inline pictures) without restarting the story. The raster bitfont also gained
  synthesized bold and italic faces, so emphasis survives the pixel path.
- **Adaptive palettes (Blorb §11.3).** A scene that swaps the palette now recolours
  the pictures already on screen, by replaying each window's draws *and* erases in
  order — which is what makes *Arthur*'s churchyard turn brown when you step into the
  church, and its blues invert behind the gravestone.
- **Mouse in v6.** Clicks are delivered during a line read, so *Zork Zero*'s border
  compass rose works while you're mid-command.
- **A map that admits what it hasn't tried.** The mapper records which directions
  you've actually attempted in each room, and an optional `?` overlay marks the ones
  you haven't — verticals included, as `u`/`d`. The room inspector grew a compass rose
  of explored directions that signals exploration by colour and draws real portal
  glyphs.
- **Keep playing with a room panel open.** The room inspector no longer takes the
  prompt hostage: you can read a room's details and keep typing.
- **Ghost-text completion at the story prompt.** Suggestions from the story's own
  vocabulary appear inline as you type, which also stops the prompt bouncing as hints
  appear and vanish.
- **The authentic `[more]` pager**, armed the way the original interpreters armed it —
  on char-input turns, on clears, and at boot.
- **IFDB ratings in the story browser** — the average rating, with its vote count
  beside it, so a 5-star single vote reads as what it is.
- **`--interpreter-number N`** overrides the story header's `0x1E` byte for one run
  (never written back), and **`/print-colors`** reports what the terminal answered to
  the OSC 10/11 colour probe.

### Changed

- **The v6 screen model, rebuilt to spec.** Seven waves of work replaced the beta.1
  approximation with ZMSD §8 behaviour: word wrap, the live cursor, stream 2, line
  counting and `buffer_mode` now do what the spec says. *Zork Zero*, *Arthur*,
  *Journey* and *Shogun* all lay out visibly better for it, and `scroll_window(0)` is
  a silent no-op instead of a player-facing warning.
- **`config.toml` explains itself.** On first run it is seeded like `style.toml`
  already was: every setting babelmap reads, grouped and commented, with the value
  shown being the default — so uncommenting a line changes nothing and the whole
  surface is browsable from the file. Only settings you actually change are written
  live; section headers stay uncommented; your comments survive later saves.
- **`diagonal_corners` is wired.** The switch the last release said was coming now
  works, under `[map]` in `style.toml` — set it `false` if your font lacks Unicode 13's
  half-diagonals. `[map]` is now the single section driving the map's glyphs, and the
  story-browser badge glyphs became settable too.
- **One line per room pair.** Parallel passages between the same two rooms collapse to
  a single line chosen by priority rather than stacking; staircases keep their own
  vertical slot instead of being folded into the compass line; and an unrelated
  crossing breaks the horizontal instead of drawing a junction that isn't there.
- **Glulx rooms are identified the way the game identifies them** — by its own location
  global rather than by the room's printed name, so two rooms sharing a name stay
  distinct and a renamed room stays itself.
- **One save format, whoever asked for it (SQ-0531).** A story's own `SAVE` now
  writes the same self-contained `.babelmap` archive Ctrl+S writes — map, screen,
  transcript and inline art included — instead of a bare VM-state-only file. So an
  in-game `restore` finally brings your scrollback back with it, even into a
  freshly launched session. The saves manager's **Type** column is now driven by
  which mechanism wrote the save rather than by its file extension, and marks the
  portable ones (**Game ↗**) — those hold standard save-instruction-PC bytes that
  unzip straight into another interpreter. Host snapshots are taken between turns,
  where no save instruction is executing, so they are honestly left unmarked.
  Restore still accepts a bare `.qzl`/`.sav` carried in from another interpreter,
  in the saves manager and at the game's own `restore` prompt alike.
- **Two new theme selectors** — `saves_portable` (accent + the `↗` glyph) and
  `saves_host_only` (muted) style the saves manager's Type cell.

### Fixed

- **A Glulx game's own `SAVE` now loads from the saves manager (SQ-0556).**
  `SAVE` behaves the same on every engine again: on Z-machine, Glulx and Scott
  Adams alike it writes a `.babelmap`, the archive appears in the manager, and it
  restores through both the game's own `RESTORE` and the host's. Picking a Glulx
  one from the manager used to answer `Glulx has no game-save (.qzl) format`
  outright. The restore keeps the windows you're looking at exactly as they are —
  the Glulx spec (§1.8.5) keeps Glk's window and stream state out of a save
  deliberately, so nothing in the file can drag a stale screen layout back over a
  live one. No archive format change: the bytes sealed for an in-game save are
  the same standard Glulx-Quetzal as before, and still unzip straight into
  another interpreter.
- **Glulx resume lands in the room you saved in**, not the boot room, and the room
  ids a resume seeds now match the ones a live turn would produce.
- **Toolbar verbs prime the prompt.** Glk's pre-filled line input (§4.2 `initlen`) is
  honoured, so *Adventure*'s graphical toolbar verbs put the word at your cursor
  instead of submitting an empty line — and the player's own edits are mirrored back
  into the game's buffer, so deleting the verb and pressing another button no longer
  re-inserts the first one.
- **The input line and caret stay put** — neither the map taking focus nor a room
  panel opening blanks them any more, and text-entry fields scroll to keep the caret
  visible.
- **v6 layout, a long tail of it.** *Arthur*'s header art no longer moves when the
  `map` command resizes the story window, and its location bar no longer renders as
  sliced half-glyphs at particular pane widths (both were the same class of bug:
  two different roundings of one boundary). *Zork Zero*'s full-screen map is visible
  in hybrid mode instead of being painted over by the transcript. *Journey*'s command
  menu inverts clicks by row, and the width-dependent dark bar under its picture
  column is gone. The v6 status band is found above the story window rather than
  assumed to be at the top of the screen.
- **Graphics are quieter and faster.** Kitty uploads are cached by canvas *content*,
  so a game that repaints an identical frame re-places the image instead of
  re-transmitting it, and image deletion is deferred a generation so animation frames
  no longer flash between steps. *Adventure*'s graphical toolbar renders as a real
  image rather than colour-averaged rule glyphs.
- **`--user-dir` now moves the config read, not just the writes**, so a run with an
  overridden home stops silently discarding everything it saves.

### Save format

- **`.babelmap` archive `format_version` 4 → 5.** `meta.json` gained
  `trigger: "ingame" | "hoststate"`; restore dispatches on it instead of on the
  file extension. Archives written before the bump still load and read as
  `"hoststate"` — which is exactly what they were — but a version-5 archive is
  rejected by older builds, as the format freeze intends. Bare Quetzal /
  Glulx-Quetzal interchange files are untouched. See
  [`docs/release/save-format-policy.md`](docs/release/save-format-policy.md).

### Known issues

- **Sub-cell buttons in a graphics window can't be clicked.** A game that hit-tests
  its own canvas in pixels — *Adventure*'s graphical toolbar is the case — can place
  buttons smaller than a terminal cell. Its compass rose puts **W** and **E** in a band
  that a cell-centre click can never name, so those two are unreachable however
  carefully you aim. Pixel-precise reporting (DEC mode 1016) was implemented for this
  and withdrawn before release: the cell size it divides by is reported in logical
  points while the protocol reports device pixels, which broke every click on a HiDPI
  display. It needs a `CSI 14t`-derived divisor to land. *Workaround:* type `west` /
  `east`; the toolbar's other buttons all work.
- **A v6 game's own erase can take neighbouring art with it.** Windows share one
  screen (ZMSD §8), so erasing a region clears whatever *any* window plotted there.
  *Arthur*'s map screen erases the columns its side borders occupy, and since the game
  never redraws them they stay gone for the session. This is what a real interpreter
  shows, and babelmap follows it deliberately rather than second-guessing the game.
- The three v6 caveats from beta.1 still apply: **Inform-compiled v6 status lines
  don't paint in `raster` mode**, **rasterized v6 text isn't selectable**, and **sixel
  encode latency on very large panes**. See their entries below for scope and
  workarounds — `hybrid` (the default) avoids all three.

---

## v0.1.0-beta.1 — first public beta

The first public build of babelmap: a terminal interactive-fiction interpreter
that draws you a live map as you play. This entry is an inventory of what the
beta ships, not a diff — there's no prior release to diff against.

Everything below has been built and exercised in-repo. Where a claim is scoped
("verified against *Zork Zero*"), that scope is the honest extent of testing — it
is not a promise that every game in a format works.

### Engines

- **Z-machine** (`zvm`, clean-room, zero-dependency) — story-file versions
  **v3–v8**, the Infocom canon and decades of Inform 6. Standard Quetzal
  save/restore (interoperable with Frotz, down to v3 branch-form `@save`), the
  v4+ cursor-addressed upper-window screen model, timed/interrupt input,
  configurable interpreter number, story-dictionary autocomplete, and
  `set_colour` / `set_true_colour` honored at 24-bit RGB.
- **Graphical Z-machine v6** — boots and plays graphical v6 titles, verified in
  depth against ***Zork Zero*** (full banner, side columns, per-room compass,
  illuminated drop-caps), with the same engine and opcode set targeting the wider
  v6 catalogue (*Shogun*, *Journey*, *Arthur*) and Inform-compiled v6 titles.
  Rendered at an **authentic 640×400 screen with an 8×16 cell and 2×-scaled
  art**, matching the DOS/Amiga profile. Three render modes — `hybrid` (crisp
  terminal story text inside a pixel chrome ring, the default), `raster` (the
  whole pane as one pixel image), and `frameless` (no frame; full-pane text with
  inline pictures).
- **Glulx** (`gvm`, clean-room, zero-dependency) — modern Inform 7, targeting
  Glulx spec 3.1.3 with a complete **Glk 0.7.6** layer verified against the
  standard Glulx/Glk test suites. Accelerated-function interception (the Inform
  veneer runs natively, so heavyweights like Counterfeit Monkey skip their long
  startup), the full single- and double-precision float opcode set, external-file
  persistence, line-input terminators, and honest `gestalt` reporting.
- **Scott Adams** (`scott`, ScottFree `.dat`) — the classic 8-bit text
  adventures (*Adventureland*, *Pirate Adventure*, …), played through the same
  TUI and automap. Blorb-bundled PNG artwork renders; the original SAGA
  line-draw format is not decoded.

### Automapping

- **Live, engine-agnostic mapper** — consumes a plain stream of locations and
  movements (never a VM opcode), so one map builder charts all three engines.
  Rooms boxed, exits routed through a lane system with crossing-elimination and
  overlap removal, then continuously re-tidied (configurable eagerness).
- **Room detection across engines** — status-variable (v3), status-line +
  object resolution (v4/v5, including centered custom titles like Beyond Zork /
  Trinity), Inform room-heading parsing (Glulx), and graphical v6. A hideable
  indicator shows *how* the current room was resolved.
- **Layered multi-level areas** — switchable named layer tabs; peel/merge
  regions by hand.
- **Awkward cases understood** — vertical up/down connections (dotted, never
  "distorted"), nautical fore/aft/port/starboard, and redundant multi-direction
  paths collapsed into one shared connector.
- **Hand edits & export** — select / rename rooms and layers, edit notes,
  delete connections, relabel edges; export the map as **SVG**, **Graphviz DOT**,
  or an annotatable text dump; `animate-tidy` steps through the whole layout
  assembly stage by stage.

### Interface

- **Story picker & IFDB** — browse a library as a sortable, badged **list** or a
  `g` cover-gallery **grid**, each with a live info panel (metadata, cover art,
  IFID, resources, saves). On-demand IFDB metadata fetch cached per game, and a
  `/` **IFDB search / browse / download** modal that drops a new story file
  straight into your library.
- **Full TUI cockpit** — mouse support (click a room for info, middle-drag to
  pan, wheel to scroll everything), select-and-copy to the system clipboard via
  OSC 52 (clean even over SSH), a verb/noun menu, dictionary autocomplete,
  readline-style line editing, command history, an inventory strip, and
  notification toasts.
- **Command palette & leader keymap** — a `/`-summoned fuzzy command palette over
  *every* command (reachable even inside modals), plus a tmux-style `Ctrl+P`
  leader panel of mnemonic single-letter map-editing verbs.
- **Transcript tools** — search / filter (story · meta · both) / export, with
  every line category independently themeable.
- **In-game hints** — auto-detected *InvisiClues* files boot in a second
  Z-machine over the story pane; ~50 Infocom titles can fetch a hint file on
  demand with `H`.
- **Sound** — Z-machine bleeps + Blorb sampled audio (AIFF/Ogg/MOD) and Glulx Glk
  sound channels with per-channel volume and finish events, plus a themeable
  border-flash accessibility cue; audio can be routed back from a remote/SSH
  session.
- **Deep theming** — a 7-role palette the whole UI derives from, first-class
  styling for all 11 standard Glk styles, per-game looks, a templated status bar,
  and a fully configurable keymap, all in an auto-seeded, live-reloadable
  `style.toml` (`style.example.toml` mirrors the registry).

### Debugging

- **Built-in debug inspector** (`/debug`, or `--debug` to trace from boot) turns
  the map pane into a live disassembler, retargeted to each engine's model:
  - **Z-machine** — live PC-tracking disassembly; Globals / Locals / Objects /
    Dictionary / Call-Stack / Stack / Memory tabs; opcode hover help;
    click-to-jump operands; execution coverage that persists per story.
  - **Glulx** — routine-discovery disassembly (call-graph + linear scan, tinted
    by confidence, promoted to certain on execution); Functions / Strings / Glk
    tabs; a real call/eval stack and absolute-address memory view with a `<RAM>`
    marker.
  - **Scott Adams** — the action table decompiled one rule per line, fired-action
    coverage, and `✗cond` flags naming the guard that blocked a matched action;
    State / Items / Vocab / World tabs.

### Formats & persistence

- **`.babelmap` Save States** — one self-contained file freezing the whole
  session (VM state + map + on-screen windows + transcript), with named slots,
  auto-save/auto-load, and an optional per-turn **rewind/replay** history.
- **Standard interchange, in and out** — game-written `@save` produces a portable
  Quetzal `.qzl` (Z-machine, golden-tested against `dfrotz` both directions) and
  a standard Glulx-Quetzal in-game save; other interpreters' saves import through
  the saves manager.
- **Everything else just persists** — Glulx external files (Glk file streams)
  auto-persist per story across sessions; a Glulx game's own fixed-name saves
  (init cache, autosave, undo) are read/written silently so it skips its long
  startup on relaunch.
- **Frozen formats.** For the beta, every persisted byte format is enumerated,
  version-stamped, and pinned by a round-trip freeze test, under three guarantee
  tiers — **Public spec** (Quetzal / Glulx-Quetzal, kept spec-clean and
  interoperable), **Frozen (0.x)** (private binary formats and the `.babelmap`
  archive: they may only change via a deliberate bump-and-note ritual, and reject
  a newer version marker cleanly), and **Tolerant** (TOML/JSON config &
  metadata: missing fields default, unknown fields ignored). Full inventory and
  policy in [`docs/release/save-format-policy.md`](docs/release/save-format-policy.md).

### Platforms

Runs on **Linux, macOS, and Windows**. Release archives ship four binaries
(`babelmap` + `zvm-cli` / `gvm-cli` / `scott-cli`) per platform: Linux x86_64
(glibc, needs `libasound2` at runtime), a macOS universal binary (Apple Silicon +
Intel, ad-hoc signed, not notarized), and Windows x86_64 (unsigned).

### Known issues

Honest gaps in the beta. Each is scoped, and carries a workaround where one
exists.

- **Inform-compiled v6 status lines don't paint in `raster` mode.** Inform 6's v6
  library leaves its windows at height 0 and streams prose through the
  transcript; `raster` synthesises a single full-pane buffer for that shape, so
  the game's cursor-positioned status line isn't drawn there. Its prose still
  reads correctly. *Workaround:* play Inform v6 titles in `hybrid` or `frameless`
  mode (Infocom's own v6 titles keep real windows and are unaffected).
- **Rasterized v6 text isn't selectable.** In `raster` mode the story text is
  baked into the pixel image, so mouse select-and-copy can't pick out cells over
  it. *Workaround:* `hybrid` (the default) and `frameless` keep the story as real
  terminal text you can select and copy normally.
- **Sixel encode latency on very large panes.** Sixel is the slowest of the three
  pixel protocols to encode, and the v6 `raster` mode is the heaviest producer;
  encoding runs off the UI thread so input stays responsive, but a full-screen
  raster refresh over sixel can visibly lag. *Workaround:* prefer a
  Kitty/iTerm2 terminal for v6 raster, use `hybrid`/`frameless`, or shrink the
  story pane.
- **Justified text doesn't combine with margin floats, and fully-justified
  ("fill") Glk text falls back to left-flush.** Centered and right-flush
  paragraph layout is honored; the `LeftRight` fill mode currently renders
  left-flush, and justification isn't applied to lines wrapping beside a
  left-margin inline image. Cosmetic — text is never lost.
- **v6 compass-click movement isn't wired end to end.** A mouse click over the v6
  banner compass is mapped to a game pixel and delivered to the VM, but clicking
  a compass spoke doesn't yet reliably issue the corresponding move. *Workaround:*
  type movement commands (the arrow-key and text paths work).
- **v6 proportional fonts aren't honored** — status and chrome text use
  fixed-width metrics, so proportional-font layout is approximated.
- **v6 Save State restore isn't render-verified.** The host Save State captures
  the underlying machine as for any Z-machine game; whether the v6-specific
  render state (window geometry, floats, pictures) comes back pixel-identical
  across a restore isn't verified yet. Standard in-game `@save`/`@restore`
  follows the normal Z-machine path.
- **Glulx cross-interpreter save interop isn't golden-tested.** The Glulx in-game
  save round-trips internally and follows the Glulx-Quetzal spec, but reading our
  Glulx saves in another interpreter (and vice versa) isn't yet pinned by a
  golden test the way the Z-machine `.qzl` interop is (tracked in SQ-0229).
- **v6 menu opcodes are stubs** — `print_form` / `make_menu` are recognized but
  not implemented (tracked in SQ-0457).
