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

The PC releases shipped the *same* pictures in a different wrapper, and babelmap
reads that too. `.MG1` (MCGA), `.EG1`/`.EG2` (EGA) and `.CG1` (CGA) are the same
sixteen-byte header and the same directory written the other way round —
little-endian, x86-style — but the pixels inside are GIF's LZW rather than
Infocom's Huffman, with no run-length pass and no XOR. One picture, two codecs
that share nothing: decode *Zork Zero*'s MCGA archive and its Amiga floppy side
by side and all 383 pictures whose directories agree on size come out
byte-for-byte identical, which is a nicer proof than any spec. Arthur and Journey
split their EGA art across two files — see [Two files, one
archive](#two-files-one-archive) below; CGA keeps its big pictures as one bit per pixel,
so *Arthur*, *Journey* and *Shogun* have 228 pictures between them that are
literally black-and-white. Only MCGA stores palettes — EGA and CGA had theirs
soldered in, so their directory records are two bytes shorter with nowhere to put
one.

#### Two files, one archive

EGA art is 640 pixels wide and did not fit on a 360K floppy, so *Arthur* and
*Journey* shipped their EGA renditions on two disks — `.EG1` and `.EG2` — and the
header's first byte says which one you are holding. **Name the first and babelmap
loads both.** You do not have to know the set is split, and you cannot pick half
of it by accident: the launch dialog and the info panel show a two-disk set as a
single row, counting the whole thing.

That matters more than it sounds. The split is not a partition — each disk had to
stand alone for its stretch of the game, so a picture wanted on both sides is
stored on *both*, and 55 of *Arthur*'s ids live in two places. Read only disk one
and you get 97 of *Arthur*'s 137 pictures and 80 of *Journey*'s 135; the rest are
simply absent, including two of *Arthur*'s largest plates. Merged, the two disks
come to exactly what `arthur.mg1` holds undivided — 171 entries, 137 of them with
pixels — which is the nicest possible check that nothing is being lost or
invented. (*Journey*'s EGA set carries one picture MCGA does not: id 59, a
220×126 rectangle of solid black and the only single-colour plate in the archive,
which looks very much like an EGA-only way of blanking the illustration window.)

Following the part number to the next file is *not* the guess-the-pairing-from-a-
filename rule the tier list below rejects, and the difference is worth being
precise about: you have already told babelmap which archive this story uses, and
this only follows that archive's own in-band part number to the rest of itself.
The header is then checked — a file under the next part's name that says it is
some other part, or was written by another codec, or adds no picture the set
lacks, is **refused and reported**, never merged on the strength of its name.
babelmap keeps looking until a part is missing, so a title that shipped on three
disks would work too.

*Zork Zero* is unaffected: its 360K release gave EGA a whole disk, so `zork0.eg1`
is complete on its own and stays at 396 pictures.

### Choosing which artwork a game draws

Three sources, in decreasing order of how sure babelmap can be that the art and
the story belong together:

1. **A Blorb.** The container validates its own contents. Nothing to configure.
2. **A disk image.** Story and archive came off one floppy, so the medium
   guarantees the pairing. Nothing to configure.
3. **You say so.** Put a `pictures` line in the game's own `config.toml` — the
   small per-game sidecar in `<save-dir>/<story>.save/`, alongside the per-game
   `style.toml`:

   ```toml
   pictures = "zork0.mg1"
   ```

   The name is relative to the story file (an absolute path works too), and a
   named archive **wins outright** — over a Blorb sitting right beside the story,
   over a floppy's own `Pic.data`, over everything. Naming it is an instruction,
   not a hint.

Tier 3 is how you *pick a rendition*, not just how you rescue a game. *Zork Zero*
alone can be played four ways from the files that survive — the Amiga `zork0.pic`,
the MCGA `zork0.mg1`, the EGA `zork0.eg1`, the CGA `zork0.cg1` — and they are
genuinely different pictures, not the same art at different sizes. Point the key
at whichever one you want and restart. *Arthur*, *Journey* and *Shogun* offer the
same choice.

#### Three ways to say it

The config key is the *durable* form, and editing a file before you can see the
result is a strange way to choose something you can only judge by looking at it.
So there are two more doors into the same mechanism, and all three end in the
same place:

| You are… | Say it with |
| --- | --- |
| launching one story from a shell | `--pictures <path>` |
| browsing the library | **Shift-Enter** (or `o`, or a double right-click) on the story |
| setting it once and forgetting it | `pictures = "…"` in the game's `config.toml` |

`--pictures` is the try-it-once path — `babelmap zork0.z6 --pictures zork0.mg1` —
and it composes with your shell, so you can flip between renditions in successive
launches without touching any config. It **outranks** the config key: the more
specific and more recent instruction wins. It also *requires* a story on the
command line, and says so immediately rather than starting the browser and
quietly discarding the flag — the flag names art for a story, so it has no
meaning without one.

**The launch-options dialog** is the richest door, because it can show you what
you have before you choose. Select a story in the browser and press **Shift-Enter**
— plain Enter launches as it always has, so you meet this only when you ask for
it. (`o` does the same thing on terminals that can't tell Shift-Enter from Enter,
and so does double-right-clicking a row.) It lists the archives detected **for
that story** — flavour, picture count, and a "2 disks" note on a split set — and it shows you the
interpreter number your choices imply *and where that number came from*, because
picking prettier art can quietly change the machine you're emulating and that is
not a thing to discover later.

"Detected for that story" means the name matches, in either direction, once both
names are reduced to their letters and digits. That is enough to connect
`zork0.mg1` to `zork0-r393-s890714.z6`, `beyondzo.mg1` to *Beyond Zork* under
either of its filenames, and `shogun.*` to a floppy called *James Clavell's
Shogun* — across every game in a real library it finds each one's art and nobody
else's, so *Zork Zero*'s dialog offers four renditions rather than a folder. An
archive under a name that resembles nothing simply isn't in the list; you reach
it the way you always could, by naming it — `--pictures`, or the `pictures` key
— and the dialog says so on its last line rather than leaving you to wonder.

The same list appears, read-only, in the browser's **info panel**, so you can see
what a game has without opening anything: each detected archive with its flavour
and picture count, and an arrow against the one the game's `config.toml` actually
names. Panel and dialog run the same detection, so they cannot tell you two
different stories about what you own. If your `pictures` key names something the
detector would never have found — that renamed `FMVPOKER.EG1` — the panel names
it anyway, because it is what the game will draw.

Everything in that dialog applies to **this launch only** — until you tick *Save
as this game's default*, which writes it to the game's `config.toml`. That is the
point of the checkbox: try the EGA art, look at it, and only write it down if you
keep it. It writes only what you actually changed, so a setting you left alone
stays inherited rather than being pinned at today's value.

Two options are in there and no others, and the rule is not arbitrary: **only
choices that cannot be changed after boot**. The artwork is opened as the story
starts; the interpreter number is read out of the story header by the *game*.
Anything the running app can already change — colours, the v6 render mode,
map behaviour — belongs in the settings screen, and putting it in both places
would give you two editors for one value.

It also rescues games nothing could pair automatically. `fmvpoker.z6`, a fan-made
video-poker cabinet, ships a readme telling you to rename one of *Zork Zero*'s
graphics files to `FMVPOKER.EG1` and drop it alongside. No rule could ever have
guessed that; one line of config says it outright.

What babelmap deliberately will **not** do is find an archive by name. It sounds
harmless and it is not. These files carry no release number and no serial —
nothing that ties one to a story — and every Infocom Amiga release names its
archive the identical `Pic.data`, so a name-based rule needs you to rename things
anyway. Get it wrong and there is no error: *Arthur*'s illuminated plates simply
appear in *Zork Zero*, looking exactly like artwork. Better to be asked than to
guess wrong silently. (Listing what it *finds*, so you can pick, is a different
and perfectly safe thing — that just hands the choice back to the person who
knows which game they own. Which is exactly why the name matching described
above is allowed to exist: it decides which rows you are *shown*, never which
file gets opened. Nothing downstream of that list acts on it.)

And when the key names a file babelmap can't use — missing, truncated, or not a
picture archive at all — it says so, out loud, naming the file and the reason,
before falling back to the Blorb. The one outcome worth ruling out is a player
who believes they're looking at original artwork and isn't.

Naming an archive also picks the machine. Ask for a game's EGA rendition and you
are asking for the IBM PC that drew it; ask for its `Pic.data` and you are asking
for the Amiga, colours and all — see
[the interpreter profile](interpreter.md#the-interpreter-profile). babelmap works
out which from the file's *contents*, never its extension, because the two codecs
are structurally different and a filename can lie. An explicit
`interpreter_number` still overrules it.

Native archives carry no `Reso` chunk — the format has no such concept — so the
standard window comes from the machine instead, and every machine that shipped
one of these games drew v6 on the same 320×200 one. That is precisely what every
Infocom v6 Blorb's `Reso` declares anyway, so the geometry below is unchanged.

What *does* differ between renditions is how densely the art is stored. EGA and
CGA addressed a 640-column screen with pixels half as wide, so their plates are
640 across where MCGA's are 320 — the same picture, twice the samples, each one
half the width. Both cover the same rectangle, so both land on the same 640×400
screen: an MCGA or Amiga plate doubles on both axes, an EGA or CGA plate doubles
only vertically. *Arthur* is the clean proof — all 125 pictures its `.mg1` and
`.eg1` share come out at byte-identical sizes once each is mapped that way, and
*Zork Zero* agrees on 446 of its 503 (the rest differ by a pixel or two, because
these are separately drawn renditions rather than one scaled copy). Frotz reads
the same header bit as `x_scale = (flags & 0x08) ? 640 : 320`; Spatterlight's
bocfel calls it `pixelwidth` and sets it to 0.5.

The character grid never moves. EGA ran 640×200 on an 8×8 cell, which is 80×25
characters — the very grid the 640×400 screen already lays out on its 8×16 cell —
so choosing a rendition changes the artwork you are looking at and nothing about
the machine underneath it.

### The colours come with the card

An MCGA or Amiga picture arrives with its own sixteen colours attached. An EGA or
CGA one does not, and there is nowhere in its directory record to put them —
those cards had their palettes soldered in, so Infocom stored the pixels and let
the hardware supply the rest. babelmap now supplies it too, reading the rendition
straight out of the directory: nobody carrying a palette means EGA or CGA, and
every picture flagged two-colour means CGA. (Never the file extension. A `.CG1`
that somebody renamed is still a `.CG1`.)

**EGA** gets the card's sixteen: each channel off, a third, two thirds or full —
0, 85, 170, 255 — with one famous exception. Colour 6 should arithmetically be a
dark yellow, and the hardware halves its green and shows **brown** instead,
`#AA5500`, because IBM thought brown more useful than mustard and wired in the
extra circuitry to get it. That single entry is not a footnote: *Zork Zero*'s
proscenium arch is drawn as brown dithered against bright red, and getting it
wrong turned the whole frame pink and olive.

And getting it *right* is only half the arch, because EGA has no bronze at all —
the artist made one. Look closely at the original and the arch is not brown, and
not red: it is brown and bright red in alternating **columns**, one pixel wide,
and on a 640×200 screen those pixels are half as wide as an MCGA one, so the card
fused each pair into a colour the palette does not contain. Bocfel puts it
perfectly: no single pixel of the artwork is the colour the eye actually sees.
babelmap keeps all 640 columns — that is what makes an EGA plate cover exactly
the rectangle a 320-wide one does — so it has to do the fusing itself, with a
three-tap tent across columns as the art comes out of the archive. Do it there
and bronze is a property of the artwork; leave it to the scale onto your terminal
and it becomes a property of *your terminal*, since that scale is
nearest-neighbour on purpose and blends at no width at all. Measured on *Zork
Zero*'s border, the fused EGA frame's neighbour-to-neighbour variation falls from
49.1 to 8.4, against the MCGA rendition's own 4.3, and it now reads the same at a
pane of 320 pixels or 1280.

**CGA is deliberately left alone**, and it is the reason the rule is written the
way it is. A `.CG1` is 640 wide exactly as an `.EG1` is, so a rule keyed on width
would soften it too — and there is nothing in it to fuse. Its 640-wide art is
genuine one-bit line work, and blending line work only makes it grey. What the
fusing asks is not "how wide?" but "how many colours?", off the archive's own
two-colour flags.

**And if you would rather see the pixels Infocom shipped**, set
`fuse_art_dither = false` and every column comes back distinct, dither and all.
The default is on, because on is what the card did to the eye — but the archive's
own bytes are a perfectly reasonable thing to want to look at, and this is the
only setting that changes them. It cannot make CGA blend; that answer belongs to
the artwork, not to you.

#### Where the fusing stops, and why it stays there

The tent is a *notch*, not a blur. It zeroes an alternation of exactly two
columns — which is what the arch is, and why the frame's flat interior comes out
at a neighbour-to-neighbour variation of 0.00 — and it barely touches anything
coarser. *Zork Zero*'s **pillars** are dithered the other way: not a clean
two-colour alternation but error diffusion over seven EGA entries in irregular
runs, the sort of thing an automatic colour reducer produces on a smooth bronze
gradient. Broadband noise has energy at every frequency, so the notch removes
only the top of it. Across the flank columns the fusing takes the pillars from
62.9 to 12.7, against 12.3 for the MCGA pillars measured in their own 320-wide
space — much better, and still visibly a weave where MCGA is smooth metal.

Widening the kernel does finish the job: `[1, 2, 2, 2, 1] / 8` has zeros at both
of the frequencies a 320-wide plate cannot carry, and it takes the flank to 6.98
against MCGA's 6.05 while pulling the whole frame's distance to the MCGA
rendition from 27.79 down to 26.04. Every number improves. It is still not what
babelmap does, because the same frame carries the **compass rose**, whose N, W, E
and S are 640-wide line art the card resolved perfectly well — and at that width
they stop being letters and become smudges. One plate, two kinds of detail at the
same frequency, and no single linear filter tells them apart. The tent keeps the
lettering; the pillars keep some of their weave. That is the trade, made
deliberately.

**CGA** gets two colours, and that surprises people who remember CGA's cyan and
magenta. Those belong to its 320-wide four-colour mode; the 640-wide mode these
archives are stored for — mode 6, the only 640-wide one the card had — is one bit
per pixel. So *Zork Zero*'s CGA rendition really is crisp black-and-white line
art, exactly as it was in 1988, and not a washed-out version of the EGA one.

Two colours also make it a **stencil**, which is the part worth knowing. Count
the border: 46,336 pixels of opaque white, 17,152 of opaque black — and 192,512
transparent. The white is paint, the lit face of the pillars; the transparency is
deliberate, and whatever sits behind it becomes a colour the artwork never had to
store. Both are lost the moment something paints a page underneath, and *Zork
Zero* asks for one — it sets black-on-white at boot and does so for every video
card alike, because the story file cannot see which archive you loaded. So
babelmap tells a game drawing two-colour artwork that the interpreter has no
colours to offer, which is true, and the game stops asking. Your theme owns the
page, the stencil reveals it, and the artwork comes out in your colours. It
applies to that story only — it never touches your saved settings, so opening a
`.cg1` once does not quietly strip the colours from everything else you play.

Neither is *adaptive*, which matters more than it sounds. A picture that carries
no palette normally means "draw me with whatever palette is current" (below), and
an EGA picture carries none for an entirely different reason — it has no say in
its colours at all. babelmap keeps those out of the Current-Palette machinery
altogether, so nothing can tint a rendition whose colours were decided by a chip.

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

Some games never lay out at all. Inform 6's v6 library leaves *every* window at
height zero and flows its prose through the transcript, so the screen model would
otherwise come out completely empty and the composite would ship a blank page. For
that — and only that — babelmap synthesises a full-screen story window out of the
header's own character dimensions, so the streamed text still has somewhere to
live. The question it asks is "did *nothing at all* survive?", which sounds
obvious and was not: it used to ask whether the surviving windows had a zero
*character grid*, and a game that never resizes window 0 off its boot rect never
sets one. `sunburst.z6` is that game — a real 640×400 story window with a 0×0
char grid — so it got a phantom twin at the same rect, filed away as frame
furniture. One screen, one story window.

## A window keeps its own text style

Bold, italic and reverse video are *per window* in Version 6. The standard lists
the style as window property 10 (§8.8.3.2) and says it "is set just as in Version
4, using `set_text_style` (which sets that for the current window)" — so selecting
a window makes that window's style live, exactly as it makes that window's colour
pair live. A game can leave the status bar reversed indefinitely and go on printing
plain prose below it, and on a conforming interpreter it never has to say so.

Shogun does precisely that, but only when it thinks it is on an Amiga: it selects
window 1, turns reverse video on, paints the status line, and returns to window 0
without turning it off. Reading the style as one global setting therefore left the
Amiga release printing everything in inverse from its second turn onwards — the `>`
prompt, the room headings, the death notice. It is the kind of bug that only one
build shows, so it is worth saying which: `James Clavell's Shogun.adf`, release 295
/ serial 890321, which is a different build from the `shogun-r322-s890706.z6`
sitting beside it and the only title in the corpus the fix moves at all.

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

## Art grows with a hard filter and shrinks with a soft one

Everything above lands the artwork on a 640×400 game screen. Getting *that* onto
your terminal is one more scale, and which way it goes changes what the right
answer is.

Growing is easy and it is the case pixel art is famous for: nearest-neighbour,
which replicates whole source pixels and invents no colours. Journey's canyon
plate is 222×254 native pixels drawn from a palette of fourteen; magnified 1.48×
it still holds exactly fourteen. Run the same plate through a smoothing filter
and you get 1,636 — every one of them a blend nobody painted. That is why the
scale caps out (`MAX_V6_UPSCALE`) rather than reaching for your pane's full
device resolution, and why it has always been nearest.

Shrinking is the same rule read backwards, and that is the trap. The instruction
"take the source pixel nearest this destination pixel" *replicates* one on the way
up and **drops** one on the way down. At a 60×24 pane Journey's plate is asked for
168×198, and 54 of its 222 columns and 56 of its 254 rows are then never sampled
at all. On a flat wall you would not notice. On a dithered one — a checkerboard of
two inks standing in for a third — every surviving pixel is a coin toss about which
ink you keep, and the shadow that should read as a smooth gradient breaks into
noise. Which is precisely the report this section exists because of: distortion in
the artwork *only when the artwork is smaller*, worst in the foreground rocks and
the dithered shadow.

So the filter is now chosen by direction, per axis, at every one of the places art
is resampled: the raster composite, the hybrid ring's bands, and the stretched
flanks. An axis that grows gets nearest; an axis that shrinks gets an area filter
whose kernel is as wide as the ratio, so the dither *fuses* into the colour it was
always standing in for. The two axes are decided separately because a band can grow
on one while it shrinks on the other — that is exactly what an elongated frame
column is — and a pass at 1:1 is a bit-exact identity, so the ordinary case still
costs a single resize.

Measured against the honest ideal (an area average where an axis shrinks,
replication where it grows), on Journey's plate at the sizes the pane sweep
actually produces:

| filter | RMS on a shrink | what it does to a dithered gradient |
|---|---:|---|
| Nearest | 9.9–10.7 | drops rows and columns; the reported aliasing |
| **Triangle** | **0.4–1.6** | fuses the dither — the area filter |
| CatmullRom | 2.1–2.6 | over-sharpens; raises contrast *above* the ideal |
| Lanczos3 | 3.8–4.1 | over-sharpens harder, and rings |
| Gaussian | 2.4–3.5 | over-blurs |

There is a second, quieter fix folded in. The raster composite's own pre-scale was
clamped at 1.0, so a pane smaller than the composite made a full identity copy of
it that bought nothing at all, and then left the actual shrink to the image
protocol's *default* filter — nearest again. It now hands over the native canvas
and names the filter, which is one resample from the best source there is instead
of two from a worse one.

`/dump-windows` reports the decision, since a band's cell rect never could: every
band's log line ends with `resample 222x254->200x234 x:area y:area`. If art ever
looks wrong at a particular size again, that line says which direction it moved and
which filter it went through.

Nothing changes at or above native size. A magnifying resample is still exact pixel
replication, and the corpus tests pin it that way.

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
  frame is handed to the full-picture composite instead — and the rule is simply
  that, with no ring to draw in, a story window whose own picture paints *anything*
  has nowhere else to put it. That was arrived at the long way round, by asking
  first whether the art *filled* the screen (Zork Zero's map, Arthur's illustrated
  intro plates, Journey's title) and then whether it merely *enclosed* it. fmvpoker's
  poker table is the second kind: a 640×400 frame with a hollow middle that the game
  prints its whole title inside, only 17% of its pixels painted, which the fill test
  missed at every point that mattered. The Mysterious Adventures are neither kind —
  their boot stacks two 512×192 title cards down the left of the screen, leaving the
  right-hand quarter bare — and for a while babelmap drew neither card, because two
  tests for two particular shapes had quietly stood in for the one fact that
  mattered. Both shapes are special cases of it, and the general rule moves no other
  title: crisp terminal cells are worth having, but not at the price of the picture.
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
  A game that prints a **label over its own rule** leaves stray fragments of that
  rule buried inside the label's pixels — Journey's release-30 menu header has one
  under each of its two titles — and those are the label's, not dividers: they are
  dropped in the game's own coordinates, native against native, so a title's rule
  closes up against it at every pane. Judged in terminal columns instead, the answer
  moved with the pane: a stray 80 native pixels into a 19-character title fell one
  column past the title's last cell once the pane passed about 1.9 columns per native
  cell, and pushed the rule behind it one further right again — a single blank cell
  after `Individual Commands`, at 155 columns, and 157 and up, but not at 154 or 156.
  The ring layout is also **dynamic**: on a pane taller than the game's native
  aspect there is vertical letterbox dead space, and hybrid mode reclaims it rather
  than centring the frame in it. When nothing sits below the story — header art,
  side borders, a status bar on top, but an open bottom (Arthur) — the ring is
  anchored to the pane top and the story viewport grows all the way to the pane
  bottom at its exact inset width; where the side art runs out, the border is
  **tiled** down the rest of the flank (below). When the game has a bottom text
  chrome instead (Journey's command menu), that strip is anchored to the pane
  *bottom* edge and the story fills the space between the top chrome and the menu.
  A game whose frame *encloses* the story to the screen bottom (Zork Zero's full
  frame) keeps the centred letterbox untouched, and a pane at or below the scaled
  native height (no dead space) degrades to that same centred layout.
  **One window is fixed-height and the rest take what remains — and in Journey it
  is the one at the bottom.** Nearly every v6 title puts its fixed window on top
  (Arthur's status bar, Zork Zero's banner) and lets the story grow downward;
  Journey inverts it. Its artwork sits left, its story right, and its command menu
  runs along the foot of the screen, and it is the *menu* whose height is a property
  of itself: fixed in y, dynamic in x, with the art and the story taking the width
  and whatever height is left above it. Since hybrid draws chrome text as text —
  one game row to one terminal row — that fixed height is simply the span of the
  game rows the menu carries, which is how "native pixels → terminal rows" cashes
  out for a strip made of characters. The planner used to compute it the other way
  round, deriving the story viewport from the letterbox and handing the menu the
  remainder, so the band's height wandered with the pane (nine rows at one scale,
  eleven at another) while its content stayed a constant seven game rows. The rows
  the menu never reached were painted by nothing, which stranded Journey's own
  `└────┘` three rows above the pane's last row and trailed an empty upload after
  the band; at a short pane the same arithmetic ran the other way and clipped the
  menu's last line off the screen entirely. The menu now ends exactly where the
  screen does, at every pane shape and on both releases.
  **Side border art is TILED down the flank, never stretched into it.** Three
  titles frame their story window with artwork drawn for a 320×200 screen —
  Arthur's poles, Shogun's single-piece border, Zork Zero's pillars — and a modern
  pane is taller than all of them. Stretching the band to fit elongates the art by
  whatever the slack happens to be: measured at 1.8× vertical against 1.3×
  horizontal on Shogun at a 100×40 terminal, and 2.2× against 1.0× on Zork Zero at
  117×64. So each flank is now composed in the game's own native pixels — capital,
  a repeated shaft, then the art's own foot back on at the bottom — and the whole
  strip is scaled once, at the same factor as everything else. Two consequences
  worth knowing: the side art keeps the header plate's horizontal factor, so the
  frame still meets exactly at its corners at every pane width; and Arthur's flanks
  are no longer cut off at the row his poles happen to stop (native 379 of 400,
  which on a 64-row pane left the frame standing open down its lower half). The
  three recipes are per title, because the artwork is: the mechanism is a port of
  Bocfel's `draw_border.cpp`, which Spatterlight ships, and which hard-codes per
  game *and* per platform for the same reason. It can afford to, because it draws
  one rendition per run; babelmap lets you switch archives mid-library, and Zork
  Zero's renditions **disagree about where its pillars start** — the banner above
  them is 34 raw rows on MCGA, 37 on EGA and 39 on CGA, while the pillars are 166
  rows in all three. A repeat unit pinned to one of those layouts lands inside the
  ring beneath the capital on the other two and tiles that ring down the whole
  column as a horizontal seam. So Zork Zero's pillars are **measured, not pinned**:
  the shaft is the longest run of rows holding one opaque width, the capital and
  base are what flare out above and below it, and the cut, the repeat and the foot
  all come off that. On the MCGA and Amiga art the measurement returns Bocfel's own
  four constants to the row, which is what makes it a derivation of them rather
  than a replacement. **Alternate tiles are drawn mirrored**, which is what finally
  killed the CGA seam. Cutting in the plain shaft is not enough on its own, because
  Zork Zero's CGA pillar is a *lit* column: mean row luminance runs 97 down to 82
  from its capital to its base, where MCGA holds a flat 54 and EGA a flat 51. A
  repeat that merely translates such a strip butts its darkest row against its
  brightest and resets the shading at every join — a step of 22.98 against the 14.08
  the art's own shaft ever manages, plainly visible once SQ-0806 stopped painting the
  page white behind it. Flipping every other tile makes each join an exact duplicated
  row instead, so the shading folds back on itself and the seam has nothing to show;
  on the two flat renditions a mirror and a translation are indistinguishable, so
  they are untouched. Spatterlight reaches the same place by hard-coding
  `flip = true` for CGA (and forcing the first EGA tile flipped) with an 11-row
  overlap to hide what is left; a duplicated row needs no overlap at all.
  Which title a flank belongs to is measured too, and **reaching the bottom of the
  screen is not what makes a flank Zork Zero's**. Shogun's DOS artwork is drawn for
  the full 200-line screen where its Amiga art stops at 168, so `.MG1`, `.EG1`,
  `.CG1` and the Blorb all reached the last row and were handed Zork Zero's masonry
  recipe — cut at the shaft, repeat, stamp a foot — applied to a Japanese lacquer
  frame, with `.CG1`'s two flanks disagreeing with each other for good measure. The
  second measurement is the flank's *shape*: a pillar narrows below its banner, a
  slab holds one width top to bottom. Narrowest ÷ widest painted row is 0.96–1.00
  across every Shogun rendition and both flanks, and 0.02–0.81 across every Zork Zero
  rendition and all three of its scene borders, so the cut sits at 9/10 in the gap
  between them.
  **A shaft has to be most of the flank, or it isn't one.** Zork Zero has three
  scene borders — the castle, the underground and the jungle — and Spatterlight
  picks between them by reading the game's own border global, which babelmap has no
  path to: picture numbers do not survive the engine boundary, and the renderer is
  handed a flattened canvas. Measuring the repeat unit rather than pinning it was
  right for the castle and wrong for the other two, and wrong in an unusually
  visible way: the underground is alternating stone blocks and the jungle is
  foliage, so the longest run of rows holding one width in them is a coincidence —
  and a *different* coincidence in each flank. Composed from each archive's own
  pictures the way the game draws them, `.CG1`'s underground cut its left flank at
  row 78 and its right at row 296, and `.MG1`'s jungle derived a 14-row repeat unit
  on the left while the right fell back to the castle's 284. Six of the eight
  non-castle flank pairs got different recipes from the two halves of one symmetric
  border. The castle holds one span for 70–73% of the flank on every rendition and
  both flanks; nothing else measured manages more than 45%. So a run has to be at
  least half the flank to count — the definition of a pillar rather than a number
  fitted to the corpus — and the underground and the jungle now take the castle's
  constants uniformly, which is what they were getting before the measurement
  existed and is still the right answer for them. The mirrored repeat covers the
  rest: Spatterlight's per-scene overlaps (37 rows underground, 59 in the jungle)
  exist to hide the seam a duplicated row already has nothing to show. What is
  genuinely out of reach is the underground's *stone alternation* — Spatterlight
  swaps the two flanks' 37-pixel stone blocks on alternate tiles so the courses
  trade sides, and that is a statement about the pair which only the scene identity
  justifies.
  The obvious escape from all of this — autocorrelate the flank down its own y
  axis, take the strongest period as the tile height, and never ask which scene is
  on screen — was measured across the corpus and **does not work**, for a reason
  that is structural rather than a matter of tuning: *a pillar shaft has no
  period*. `.MG1`'s is uniform, its rows pixel-identical, so every lag scores
  exactly alike and the search answers with the smallest one it is offered — a
  4-row repeat unit against the 284 the shape measurement gives. `.CG1`'s is a lit
  column shading 97 down to 82, and a gradient is no more periodic than a flat
  wall, so its best lag scores *worse* than an average one. Meanwhile the two
  scenes the idea was meant to rescue fare worse still: the underground's stone
  course does turn up at 74 rows, but with no more confidence than a coin toss, and
  on `.CG1` the two flanks disagree about it — 76 left against 74 right, the very
  asymmetry the majority test had just finished removing. The statistic rewards
  self-similarity, and a plain shaft is more self-similar than patterned masonry,
  which makes it anti-correlated with the thing it was asked to detect. No
  threshold anywhere in the corpus admits the underground and the jungle without
  also admitting the castle, Arthur's poles and Shogun's slab, whose repeat units
  are not periods in the art at all but choices about how much of it to reuse. The
  per-scene dispatch stays, and the corpus measurement is pinned so that a future
  statistic which *does* separate it will say so out loud.
  One trap the recipe has to dodge:
  the canvas a band ships is the artwork *minus* whatever the renderer draws as
  terminal cells instead, so a repeat cut from it copies the holes those cells
  left. Shogun's status line is two 16-pixel rows the top of its border sits
  behind, and cutting the repeat there put a 64-row hole at the join between the
  tiled pieces — 94 screen pixels of black between two ornate gold panels at
  120×90. Its repeats come off the graphics-only canvas instead, which is the
  order Spatterlight works in too: it covers the status bar *after* extending, not
  before. `/dump-windows` labels a band `[Art, tiled]`, reports the native size of
  the source it was composed from, and counts the rows in it that carry no art at
  all — the longest run and where it starts, since a hole is invisible in the
  band's rectangle and shows up only on screen.
  **Raster mode gets the same frame**, because it builds the whole thing at the
  640×400 native screen and hands the finished canvas to a single scale — the same
  way Spatterlight composes at native resolution and stretch-blits once. The flanks
  are extended before that scale rather than at draw time, so raster's corners
  agree structurally instead of by arrangement. It had been left behind when tiling
  landed, and the two pixel modes were drawing different screens from the same turn:
  Shogun's Amiga border ends at native row 336 of 400 and Arthur's poles at 379, and
  raster showed those last rows as one flat colour inside the frame's own lower edge
  — 64 native rows on Shogun, 21 on Arthur. Zork Zero was unaffected either way; its
  pillars already reach the bottom.
  **A picture column over a command menu is not a border, and raster leaves it
  alone.** The hybrid ring had always known this — it builds no tiled flank at all
  for a game with a text strip under its story window, because that flank is a
  picture seated in a panel rather than a frame to extend — and the raster
  extension arrived without the exclusion. Journey paid for it. On the Amiga disk
  (release 30, serial 890322) its illustration paints native rows 25–279 of columns
  0–264, its story window ends at row 288, and "The Party" is printed at row 289;
  recognised as nobody's border in particular, the column fell through to Arthur's
  pole handler, which cut four rows of canyon wall at 90% of the art's height and
  tiled them to row 400 with a 28-row "foot" stamped on the end. The player got
  "Individual Commands" alone on a menu strip half-buried in scenery, and an
  illustration reading a third taller than the artist drew it. Release 83 has the
  same shape and was showing the same thing, so it was never a quirk of one medium.
  Now the two modes agree with the machine again: the art stops where the picture
  stops, and both labels sit side by side on the strip below it.
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
  a story window's edge rounds away goes to the flanks too — **both** edges, since a
  story box has two of them: the frame closes at its top corner instead of leaving an
  unwritten row between the top rule and the first line of prose, and its side rules
  run down to the menu instead of stopping a row short with a pane-wide band painted
  across them. (A band spans the whole pane by definition, so a leftover one under the
  story paints over both side rules at once; the flanks own those columns and take the
  row.) A band that carries the game's own chrome there — a frame closing along the
  pane's last row, as Zork Zero's does — keeps it: the test is whether there is
  artwork *between* the flanks, not where the row sits.
  Clicks follow the same seam. The command menu a game like Journey puts at the foot
  of the screen is a bottom-anchored strip when the layout has slack to reclaim and an
  ordinary ring strip when it has none, and both are drawn by packing the game's rows
  onto consecutive terminal rows — so the click map inverts by row index in both,
  rather than inverting the pane linearly and landing a row off in the second.
  And this holds at *every* pane shape, reclaimed layout or
  centred letterbox alike — a short, wide pane leaves no dead space to reclaim and
  used to hand the whole flank, border columns included, to one uploaded band, which
  swallowed the frame's rules into the picture beside them.
  **If a game draws a border, the artwork does not overlap it**: the picture's
  allocated span stops where the rule's column begins, and the rule is stamped as
  the character the game printed. Nothing is lost in the trade — the column was
  already established to hold no artwork before the rule can claim it.
  The border's unit is the game's own **text cell**, not one terminal column, and
  that matters as soon as the scale exceeds a column per native cell (2.93 at a
  236-column pane). A band's crop is *where it is placed* mapped back through the
  letterbox scale, so a destination trimmed by whole columns still starts a native
  pixel or two inside the rule's cell — and Journey inks its `│` three pixels in,
  which is how the game's own rule ended up rasterised *beside* the glyph stamped
  for it: three lines down the left edge, the innermost visibly fatter, and only at
  the wider panes. So the rule's extension spans every column its native cell falls
  in, those columns carry the cell's own ground, and the cell's pixels are erased
  from the canvas the bands are built from — the column-wise twin of the row-wise
  carve that has always kept a text strip out of the bands. The character itself
  still stands in exactly one column; stamping it across the span would be the
  doubled rule this whole rule exists to avoid — and *which* column is decided by
  the game's own screen: a glyph in the screen's edge cell aligns outward, so the
  frame's `┐`, its `┘` and the rule down that side all reach the pane's last column
  instead of leaving a blank one beside it. Everything inside the screen, every
  interior divider included, keeps the column its own run maps to.
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

**And the reserved margin is page, not chrome.** The columns a float holds back
are drawn as leading spaces on every row of prose beside the picture, and those
spaces used to inherit the transcript's base style while the prose an inch to
their right sat on the background its own text run named. Nothing showed while
the two were the same colour — which is every machine but one. Under the Amiga
interpreter the base is the machine's screen pair (§8.3, and the same pair the
pixel ring around the viewport is drawn in), whose page is dark grey, while Zork
Zero's window 0 declares a light grey page of its own; the difference turned up
as a dark stripe down the right-hand edge of every drop-cap and every room icon.
The margin now takes the ground the prose beside it sits on — its background and
nothing else, no bold, no reverse, no hyperlink — so the picture, its gutter and
the paragraph read as one sheet of paper. A paragraph that names no background of
its own copies nothing and inherits exactly as before, which is why the IBM PC
profile and both `honor_game_colours` settings render byte-for-byte what they
always did.

There is a second question, because clearing the screen also puts the cursor
back at its top-left corner: **is there any room left beside the picture?** A
float, by definition, has prose flowing next to it. A picture that spans window
0 from edge to edge leaves no column for that prose, so it cannot be one — it is
a backdrop, and it goes on the window's own canvas with the story text drawn
over it. Frobozz Magic Videopoker paints its whole card table that way, and
Journey its title illustration; both draw at (1,1) immediately after erasing the
screen and would otherwise be mistaken for the world's largest drop-cap. The
margin there is not a fine one: the widest genuine float in the Infocom v6
catalogue — Shogun's ship — covers 58% of its window, and both of those
backdrops cover all of it.

The Mysterious Adventures are the reason there is a third question. Their title
cards are 512 pixels wide on a 640-pixel screen, so they span neither the window
nor any threshold worth arguing about, and no reading of "how wide is it?" was
ever going to place them. What settles them instead is asking what the cursor
test is actually worth on that frame: landing on the text cursor means the
picture belongs to the line being written, and at boot **nothing has been written
at all**. The cursor is simply where the screen-clear left it, so a picture that
matches it matches nothing. babelmap now counts the characters window 0 has
streamed, and treats hitting the cursor as evidence only when there were some.
Every genuine float in the catalogue is drawn into a window that has already
printed something; every coincidence is not.

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

**A keypress turn's output does not automatically open a line, either.** The
transcript puts each turn's output on a fresh line, and for a typed command that is
right: an interpreter echoes a `read` together with its terminating newline
(§7.1.1.1), so babelmap appends what you typed to the game's `>` and lets the reply
start below. `read_char` echoes nothing at all (§10.7), so for a keypress turn that
line break is babelmap's own invention — and whether it belongs cannot be read off
the text, because a game redrawing a menu moves its cursor and prints no newline
either way. The game's cursor is asked instead: output whose first character lands
exactly where the last output left the cursor continues that line, and everything
else opens a new one. `sunburst.z6` is what it buys — a game with no line reader
that runs `read_char` in a loop and echoes each key back, so typing `look` and
pressing Enter used to arrive as `>look` and then a lone `.` a line lower, where the
game's own screen has `>look.` on one row. Games that reposition between reprints —
the Mysterious Adventures re-asking `Resume play on a game ?`, Journey's and
fmvpoker's menu repaints — keep their line breaks, because their cursor says so.

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

### The ground has to survive a restore too

The painted ground rides *beside* the window tree rather than inside it, and for a
while that meant it was the one v6 screen layer no restore touched. A Save State
swaps VM memory under a game that never learns it happened, so the story issues no
repaint — and `auto_load` fires only after the story has already booted and painted
its opening screen. Resuming scopa therefore came back with the main menu's cards and
buttons still on the ground, the restored hand's own text drawn over the top of them;
the model was perfectly correct underneath, so clicking where the *real* cards should
be played the right card. Shogun showed the mirror image of the same hole: it lays its
backdrop down one keypress into the boot, so a resumed Shogun arrived with no ground at
all and lost the backdrop.

The archive now carries the ground as `pictures/ground.png`, and every restore path
replaces it — including with *nothing*, when the archive has none. Pixels rather than a
recipe, which is the exception the "persist the recipe" rule allows for a derived
artifact that is genuinely authoritative: the ground's inputs are an unbounded stream
of `erase_window` fills (scopa repaints its table hundreds of times per card), which is
why it is a surface and not a list of rectangles in the first place. It is stored in
the story's own native pixels, so it stays as backend- and terminal-neutral as the rest
of the archive — a save taken on kitty at 117×64 restores unchanged onto half-blocks at
80×24.

### …and so do the ground's two siblings

The ground was not travelling alone in that gap; it was simply the one anybody
noticed. Two more layers ride beside the window tree, and neither was archived nor
reset either.

The **erase fills** are the first. The standard makes erasing a window a fill of its
rect with the window's background colour, and on a v6 screen that is opaque paint —
it is what makes advent's help panel a solid panel rather than text hovering over the
story. Journey two steps into its boot is covering three windows that way; one step
later it is covering none. Restore the later save onto the earlier screen and all
three bands used to stay, three opaque slabs over a game that had moved on; restore
it the other way and the three the save *did* carry never arrived at all.

The **canvas anchors** are the second. An anchor is what remembers where a window's
art was painted, so that when the window moves the pixels stay behind (the standard is
explicit: "subsequent movements of the window do not move what was printed"). A
restored session used to inherit the *previous* game's anchors, so the first window
move after a restore stranded the restored art at coordinates belonging to a screen
that no longer existed. Journey, Zork Zero, Shogun and fmvpoker all hold live anchors
within three steps of boot, so this was not a corner case.

Both now travel inside `display.json` — and as a **recipe**, not as pixels, because
unlike the ground they are bounded: one small struct per window however long the
session runs. Two session-local numbers are deliberately left behind, since neither
means anything in the session that reads them back. A fill's draw stamp comes from a
process-global counter, so only the *order* of the fills travels and the restore
re-stamps them from the live counter, exactly as it does for restored canvases. And a
fill's character stamp decides exactly one thing — whether any prose has printed since,
which is what stops it covering — so only the fills that still cover travel at all; the
counter never runs backwards, so a fill the story has printed past can never cover
again.

`@restart` gets the same treatment, for the same reason and by the same argument the
reboot path already makes about the canvases and the display list. A rebooted story
inherits neither the dead screen's anchors nor its ground.

### …and so does the prose that is sitting on the glass

Three more runs were missing, and this time from *inside* the window tree. A v6
window keeps its text as three layers in the same pixel space: what it has painted,
what it has **streamed** — where the prose it sent to the transcript is currently
sitting — and what a move or resize has **retired**, frozen at coordinates the window
has since walked away from ([above](#prose-freezes-where-it-was-printed-when-its-window-moves)).
Only the first of the three was in the save.

The one game that renders from the streamed layer is the one game that keeps its text
inside its own picture frame: fmvpoker's "Current Bet: 10" and "Total Winnings: 990"
live nowhere else a save was carrying, so a resumed hand came back with its legends
gone from the table. It hid for a long time because the *character* grid was archived
all along — the bet was there in cell mode and missing from the pixel composite, which
is the mode almost everybody plays in. Shogun lost the other layer: one keypress into
its boot it is holding all nine frozen title lines, and a restore used to hand them
back blank or leave the previous screen's standing over the new one.

All three layers now travel together in `screen.json`, as the game's own runs in its
own native pixels — a recipe like `texts` beside them, with no cell coordinate, font
metric or picker state anywhere in it, so one archive restores identically into an
80×24 terminal and a 200×80 one and draws the same on either graphics backend. The one
thing deliberately left behind is the per-burst *stream origin*, which only means
anything between one keypress and the next and nothing at all across a save.

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

**A row that names a background is filled behind its runs — and bridges the gaps
between them only when it is a bar.** A status band a game paints as several
separate runs has to read as one solid strip, gaps and all: Shogun prints
`Erasmus :`, `SHOGUN` and `Score:` black-on-white across a window whose two ends
they all but touch, so the band floods that white from one window edge to the
other and the bare cells between the labels come out the same colour as the ones
under them. A row whose runs stop well short of both edges is not a band, though —
it is two labels the game happened to print on one line, and what lies between
them belongs to the window. Scopa's end-of-hand score screen is the case: it
prints its whole board into a single 640×400 grid, and `Denari` and `Primiera`
(with the two pairs of totals below them) sit either side of a green divider the
game leaves between its two blue card panels. Filling each of those rows from its
first label to its last painted three blue bridges straight through the divider.
Each run's own cells are filled; the table between them stays the table.

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
(a stock 16-colour table, close to the EGA card's but not it — see above) and are
flagged in the Blorb's `APal` chunk as
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

## Artwork you stop looking at is handed back

An image sent to a kitty terminal stays there until something says otherwise.
Placing a new one over it does not free it; closing the window it belonged to does
not free it; clearing the screen does not free it. Only an explicit delete does,
and babelmap now sends one for every picture it walks away from — a chrome band
whose art changed, a band the ring no longer draws, and the full-pane raster
composite, which is the largest single thing the app ever uploads (2.8 MB of
Journey's opening screen, at a 117×64 terminal).

This is not tidiness. Kitty terminals evict by least-recently-used when their image
memory fills, and they will happily evict a picture that is *currently on screen* —
so a long session that keeps sending art and never takes any back can blank the very
frame you are looking at. Journey's first five keypresses used to upload 4.1 MB and
free none of it; a game like scopa, whose whole screen is one image, stranded a
fresh copy of it on every move. Now each one is handed back in the same breath as
its replacement goes out, and `band uploads since launch` counts against a terminal
that is no longer quietly accumulating everything it was ever shown.

Order matters more than it looks. A picture being *replaced* is the one your
terminal is drawing at that moment, and its replacement can be most of a megabyte
away — Zork Zero's banner used to be 618 KB every time a compass arrow changed.
Free it first and those cells have nothing to draw until the new upload lands, which
reads on screen as a flicker: the compass blinking as it composites, the on-screen
map blinking as it updates its corner, Arthur's graveyard blinking as Merlin appears
in it. So a picture nothing is showing any more is freed immediately, and a picture
being replaced in place is freed *after* its replacement is on screen. Same frame,
same batch, nothing held longer than the width of one placement.

## A band is cut into tiles, so a small change is a small upload

That 618 KB is worth staring at. Zork Zero's banner is one 920×126 image, and a
compass arrow is about 45×40 pixels of it — a third of a percent. Every band already
hashes only its own native footprint, so a change under *another* band never disturbs
it; but a change under *this* band re-encoded and re-transmitted all 151 chunks of it.
Eight arrows over the boot animation, and 4.9 MB had gone down the wire to redraw a
compass.

There is no way to ask a terminal to patch pixels into a picture it already has.
Kitty cannot, iterm2 cannot, and building a patch-over-base layer on virtual
placements only trades the bandwidth for bookkeeping and drift. What you *can* do is
send a **smaller picture**, so the ring's full-width bands now go up as a row of
8-column tiles rather than one strip: fifteen images across Zork Zero's banner
instead of one, and a compass arrow re-sends the one or two it lands in.

The tiles are the same pixels. Every band crops its rectangle out of one scaled
canvas the frame builds once, at whole device pixels, so column 41 reads exactly the
same source however the band around it is cut — no resampling boundary at a seam, no
ceil-versus-round trap, and the first frame's transmitted payload is byte for byte
what it always was (618,240 bytes: fourteen tiles of 43,008 and one of 16,128). The
partition is exact by construction — no gap that would leave a column of the ring
unwritten, no overlap that would put two images on one cell.

Eight columns is arithmetic rather than taste. Kitty takes 4096 base64 characters per
chunk, so every tile rounds its last chunk up and wastes about 2 KB; cut finer and
that fixed cost eats the win (115 one-column tiles would add 230 KB to every first
frame, and leave a terminal that evicts by LRU juggling 115 resident images per band);
cut coarser and the re-send climbs straight back. Measured on the real binary under a
pty at 117×64, the same three frames on either side:

| frame | one strip | 15 tiles | |
|---|---:|---:|---|
| first frame | 2,089,630 B | 2,093,195 B | +0.17% |
| compass, one tile | 629,280 B | 43,947 B | **14.3×** |
| compass, two tiles | 628,566 B | 88,349 B | **7.1×** |
| whole three-key boot | 4,604,778 B | 2,358,042 B | **1.95×** |

Granularity is per backend, because the trade is not the same everywhere. Kitty and
iterm2 tile. **Sixel does not** — every sixel image carries its own palette
definition, so fifteen tiles would mean fifteen palettes where the strip had one,
which is a real first-frame regression bought for a redraw win. Half-blocks does not
either, and does not need to: it draws glyphs, and ratatui's own cell diff has always
sent it just the cells that changed. Side flanks are left whole as well — they are
tall and thin, and column tiles would buy them nothing.

None of this is visible. With the flicker fixed, this is purely how much goes down
the wire; on a local terminal you would never notice, and over ssh it is the
difference between a boot animation that feels snappy and one that does not.

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
