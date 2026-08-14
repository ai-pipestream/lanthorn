# Interpreter (Z-machine, Glulx & Scott Adams)

[← back to README](../../README.md)

Point babelmap at a story and it works out the format from the file itself and
boots the right engine — you never choose. Under the hood are three from-scratch,
zero-dependency virtual machines written clean-room in Rust: a Z-machine
(`zvm`), a Glulx engine (`gvm`), and a Scott Adams / ScottFree engine (`scott`).
All three converge on one neutral screen model, so the host features below —
sound, colour, timed input, crash-proofing — light up no matter which you're
playing.

- **Z-machine** (`zvm`) — the Infocom canon and decades of Inform 6, in story-file
  versions **v3/v4/v5/v6/v7/v8**, including graphical **v6** — verified in depth
  against *Zork Zero*, whose pictures and text composite together on
  image-capable terminals, with the same engine targeting the wider v6
  catalogue (*Shogun*, *Journey*, *Arthur*). See [Graphical v6](v6-graphics.md)
  for how. (v1/v2 are not supported.)
- **Glulx** (`gvm`) — modern Inform 7, with a complete **Glk 0.7.6** layer verified
  against the standard Glulx/Glk test suites, an accelerated Inform veneer, and the
  full floating-point opcode set. It targets Glulx spec 3.1.3 and reports every
  capability it does and doesn't have honestly through `gestalt`.
- **Scott Adams** (ScottFree `.dat`) — the classic 8-bit text adventures
  (*Adventureland*, *Pirate Adventure*, …), played through the same TUI and live
  automap as everything else. Room illustrations render when the game ships as a
  Blorb with PNG artwork (drawn by the graphics pipeline); babelmap plays the
  `.dat` text engine and shows the bundled images — it does **not** decode the
  original SAGA line-draw graphics format.

## What counts as a story file

Point babelmap at whatever the game arrived in and it digs the story out itself.

- **Bare images** — `.z3`–`.z8`, `.ulx`, and Scott Adams `.dat` are read straight.
- **Blorb containers** — `.zblorb`/`.gblorb`/`.blorb`/`.blb` yield their executable
  chunk, and the same file's `Pict`/`Snd `/`Data` resources become the game's art
  and audio. A resources-only Blorb sitting *beside* the story counts too.
- **ZIP archives** — the first `.z3`/`.z5`/`.z8` entry is unwrapped in memory.
- **Amiga `.adf` disk images** — the original release floppy, played as it shipped.
- **Macintosh disk images** — a DiskCopy 4.2 `.image` (or a bare HFS volume), the
  Mac release floppy, likewise played as it shipped.
- **DOS floppy images** — `.ima`, `.img`, or any name at all: the PC release disk,
  from a single-game 360 KB floppy to a *Lost Treasures* collection.
- **Atari ST floppy images** — `.st`, the GEMDOS press, which turns out to be the
  same filesystem one machine over.
- **Apple II ProDOS disk images** — `.2mg`, the 800 KB 3.5" press: single-game
  Apple IIgs disks and the seven-volume *Lost Treasures of Infocom* collection.

Those last five are worth their own paragraphs. Infocom's Amiga releases came on 880 KB
floppies, and the disk images those turned into are still how the graphical
titles circulate in their native form. Hand babelmap one — `babelmap "Zork
Zero_Disk1.adf"` — and it mounts the AmigaDOS filesystem (both OFS and FFS),
walks it, and plays what it finds. No unpacking step, no loose files, nothing to
rename.

AmigaOS has no filename extensions to go by, and while Infocom's convention was
`Story.data` beside `Pic.data`, the convention is not a promise — one Zork Zero
disk lists a file in its own manifest that was never written to it. So babelmap
identifies the story by **content**: a Z-machine header whose version, memory
map, serial, and declared length all agree with the bytes actually present. The
two saved games sitting on the Zork Zero disk look superficially like v3 stories
and are rejected on exactly those grounds. Conventional names only break a tie if
a disk somehow offers two candidates; a disk with none — the plain AmigaDOS boot
floppy that ships as Disk 0 — says so instead of booting a system library.

The artwork comes along for free: a native Infocom picture archive on the *same*
image is that story's art, because a shared floppy is as strong a guarantee of
pairing as a Blorb is. Loose archives are a different matter — babelmap will use
one, but only if you name it, and it never guesses from a filename. See
[Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws).

The Macintosh floppy is the same story one filesystem over, and a good deal more
work: a DiskCopy 4.2 image is an 84-byte header wrapped around an HFS volume,
with 12 bytes of sector tag per block trailing behind that are *not* part of the
filesystem. Inside is a B\*-tree catalog — the most structure any medium here
asks for — and babelmap walks it, extents overflow file and all. macOS is no help
whatsoever: `hdiutil attach` has refused HFS-standard images since 10.14, so
every layer of that chain is hand-rolled, with the same zero dependencies the
rest of the container reading takes.

And the same content-first rule decides what to run, because the `.image`
extension means nothing in particular and the Mac disk carries a story, an
application, the Finder's desktop database and **two** picture archives — one for
the colour screen and one for the black-and-white one. babelmap draws the colour
archive; the monochrome one packs its directory differently and is not yet
decoded.

The PC and the Atari ST are one paragraph, not two, and that is the interesting
part. GEMDOS put its BIOS Parameter Block at **exactly** the DOS offsets — bytes
per sector at `0x0B`, sectors per cluster at `0x0D`, and so on down the block —
so a plain FAT12 reader opens an Atari compilation with no Atari-specific code in
it whatsoever. What differs is the machine, and the machine is a question for the
boot sector: DOS's own load protocol requires the sector to begin with an x86
jump over the BPB (`EB xx 90`, or `E9 xx xx`), because the BIOS executes it from
offset 0. TOS has no such rule. Across all twenty-four floppies in the reference
collection the test is unanimous — fifteen DOS images open with that jump under
four *different* OEM strings, and nine Atari ones open with `00 00 4E` or three
zeros and an OEM field that is blank. Nothing else was usable: the extension is
worthless (`.ima` and `.img` are one format), the OEM string names a formatter
rather than a machine, and the `55 AA` at the end of the sector is a boot
signature on one machine and a checksum word on the other.

These disks push the content-first rule harder than any other medium, because
their filenames give up entirely. Every story on an Atari ST compilation is
called `STORY.DAT` — four of them, on four different games — so the *directory*
is what names the game, and babelmap lists them as `HITCHHIK/STORY.DAT` and
`BUREAUCR.ACY/STORY.DAT`. Subdirectories are not optional here: a root-only walk
would find nothing at all on that disk, and would miss the `DEMO` folder on the
standalone DOS *Hitchhiker's*. Beside the games sit somebody's 1996 saved
positions (`BILL1.SAV`, `STEVE1.SAV`) and a pile of `.COM`, `.EXE`, `.PRG` and
`.SYS` files, and the header check throws out every one of them. One more piece
of Infocom trivia falls out for free: `ZORK0.ZIP` is **not** a PKZIP archive but
Infocom's DOS name for a bare Z-machine story — byte-identical to the loose
`zork0.z6` — so it needs no unwrapping and never did.

The one thing a PC disk cannot do is be a whole release by itself. *Zork Zero*'s
story lives on *Lost Treasures* floppy 5 with its EGA artwork, while its CGA
artwork is one disk over on floppy 4; the standalone 360 KB release spreads
installer, story and EGA art across three floppies. babelmap mounts **one image**
and offers what that image holds, so pick the disk with the game on it. Joining
several disks into one release is a set model that does not exist yet.

The Apple II arrives wrapped. A `.2mg` is a 64-byte little-endian header bolted
onto an 800 KB ProDOS volume, and every image in the reference collection carries
a small trap in that header: the field that says how long the data is reads
**zero**. That is a known quirk of the tool that wrote them — CiderPress signs
its images `WOOF` — so babelmap takes the declared length when there is one, the
block count when there is not, and the tail of the file only as a last resort,
and insists in every case that what it lands on is a whole number of 512-byte
blocks that are actually present. A bare ProDOS volume with no wrapper reads the
same way.

Underneath, ProDOS is the tidiest filesystem here and the one that nests deepest.
Files come in four shapes and babelmap reads all of them: a *seedling* is a
single block, a *sapling* points at an index block of 256 pointers, a *tree*
points at a master index of index blocks, and a GS/OS *extended* file keeps a
mini-entry per fork in an extended key block — of which babelmap reads the data
fork, exactly as it does on the Macintosh. Holes are real: a zero pointer means a
block ProDOS never allocated, and it reads back as 512 zero bytes rather than as
an error. Directories nest two deep on the GS/OS disks, so files are named by
path — `SYSTEM/SYSTEM.SETUP/TOOL.SETUP` — which the launcher volume insists on,
since it carries three different files called `FINDER.DATA`.

Two of these disks are worth knowing about before you open them. *Arthur* and
*Journey* on the Apple II are the ProDOS **8** press, and they do not contain a
story file at all: the game is split across `ARTHUR.D1`–`D5` and
`JOURNEY.D1`–`D4`, none of which begins with a Z-machine header, so the disks
mount, list their files and tell you there is no game on them rather than
pretending. And *Lost Treasures* volume 1 is the GS/OS launcher — fifty-three
files of system software and not one game. Volumes 2–7 carry thirty games
between them, and since no ProDOS release uses a conventional story name, opening
one of those disks gives you the largest game on it while the picker and
`--story` offer the whole list.

The thirtieth of those games took an extra quest to find. Deciding what is a
story means reading a Z-machine header, and one of the things a header carries is
a six-character serial — `871214`, or `------` on some builds — which is a fine
sanity check right up until you meet a disk written on a machine that sets the
high bit on every character it stores. `LEATHRGODDESSES` on volume `INFOCOM6` is
a perfectly good Version 3 story whose serial reads `C2 EC EF F7 EE A1`; take bit
7 off and it spells **`Blown!`**, somebody's joke, not damage. babelmap now masks
that bit before it judges a serial, so *Leather Goddesses of Phobos* is on the
list where it always belonged — and the check keeps doing the job it was there
for, because what it is really guarding against is the saved games sitting beside
the games on these disks, whose serial field is binary rather than text either
way.

Disk images are first-class in the library too: point babelmap at a directory of
them and the picker's TYPE column names the container alongside the format —
`Z6 (ADF)` off an Amiga disk, `Z6 (HFS)` off a Macintosh one, `Z6 (DOS)` off a PC
floppy, `Z3 (ST)` off an Atari one and `Z5 (ProDOS)` off an Apple II disk — from the same content-based
identification, so a floppy is never listed as a bare story file, and one
machine's media is never labelled as another's. See
[Story picker](interface.md#story-picker).

### One road in, whatever the disk is

Two filesystems this far apart could easily have grown two of everything, and for
a while they did: the "is this a disk, and what is on it" question was written
out three separate times — once for artwork, once for story loading, once in
`zvm-cli` — and the third copy had never learned about Macintosh disks at all. So
the command-line player mounted an Amiga floppy happily and refused a Mac one a
month after babelmap had learned to read it. Nobody wrote that rule; it was what
you get when three places each answer the same question separately.

There is one road now. A single table inside `blorb` lists the formats, and
everything that touches a disk — the picker, story loading, artwork, the CLI's
menu, the interpreter number the medium implies — asks that table rather than
naming a filesystem. **Whatever babelmap can recognise as a disk, it can open**,
because recognising and opening are the same lookup. A format added to the table
arrives everywhere at once, and DOS and the Atari ST proved it: they landed as
two rows and one reader, and the picker, the CLI menu and the launch dialog all
gained them without a line changed. Apple ProDOS then landed on exactly those
terms — one row, one new reader, and not a line of the picker, the launch dialog
or the command-line player touched. Apple II 5.25" `.dsk` media is next, and will
arrive the same way.

The proof was not free, mind. One function had been missed — the one that reads
an archive you name *inside* a disk, which predated the table and still carried a
hand-written two-reader chain. It was merely stale while two formats existed, and
became a defect the instant a third arrived: the launch dialog enumerated a PC
floppy's `ZORK0.EG1` through the table and offered it, and the loader had no arm
that could open it. Offered, picked, nothing drawn. It goes through the one table
now, which is exactly the failure mode the table exists to make impossible.

### A floppy is a different release

Worth knowing before you compare two runs: the disk image is not the same story
as the `.z6` sitting beside it. It is a different **build** of the game, and the
builds do not always behave alike. *Journey*'s floppy is release 30; the bare
story file is release 83 — and where r83 narrates through window 0, r30 narrates
through window 2. A screen rule that is right on one of them can be wrong on the
other, which is exactly what happened once.

What each medium carries, measured across the collection:

| Title | Amiga floppy | Bare story file |
| --- | --- | --- |
| Journey | release 30, serial 890322 | release 83, serial 890706 |
| Zork Zero | release 366, serial 890323 | release 393, serial 890714 |
| Shogun | release 295, serial 890321 | release 322, serial 890706 |
| Arthur | release 54, serial 890606 | release 74, serial 890714 |
| Beyond Zork | release 57, serial 871221 | release 57, serial 871221 |
| Zork I | release 88, serial 840726 | release 88, serial 840726 |
| Zork II | release 48, serial 840904 | release 48, serial 840904 |
| Zork III | release 17, serial 840727 | release 17, serial 840727 |
| Zork: The Undiscovered Underground | release 16, serial 970828 | — |

Zork Zero has a third medium, and it is the outlier of the whole collection: the
Macintosh floppy carries **release 296, serial 881019** — October 1988, where the
Amiga disk is March 1989 and the bare story file July 1989. Ninety-seven releases
separate the Mac build from the PC one. Treat a finding made on it as describing
that build and no other. It will also tell you which machine it thinks it is on
if you ask — `version` off that disk answers *"Macintosh Interpreter version
6.65"*, which is the game reading header byte `0x1E` back to you.

And *Hitchhiker's* takes the rule to its limit, now that the PC and Atari presses
are readable. Three media, three releases, and **two different Z-machine
versions**:

| Medium | Release |
| --- | --- |
| Atari ST compilation (`STORY.DAT`) | v3, release 56, serial 841221 |
| DOS standalone 360 KB floppy | v3, release 58, serial 851002 |
| DOS *Lost Treasures* collection | **v5**, release 31, serial 871119 |

The collection ships the later "Solid Gold" edition — a different engine version,
45 KB more story, and built-in hints the other two do not have. A result measured
on one of those describes exactly one of them.

The Apple II press makes the same point once more, and this time with a game that
is *not* Hitchhiker's. *Trinity* is release 12, serial 860926 on the Apple IIgs
*Lost Treasures* volume 5 and release 11, serial 860509 on `Infocom Compilation 8`
for the Atari ST — two floppies, two builds, six months apart. What each ProDOS
volume opens with:

| Volume | Opens |
| --- | --- |
| `Beyond Zork (1988)(Infocom).2mg` | Beyond Zork, v5 release 57, serial 871221 |
| *Lost Treasures* 1 (`INFOCOM1`) | — the GS/OS launcher, no game on it |
| *Lost Treasures* 2 (`INFOCOM2`) | Beyond Zork, v5 release 57, serial 871221 |
| *Lost Treasures* 3 (`INFOCOM3`) | Stationfall, v3 release 107, serial 870430 |
| *Lost Treasures* 4 (`INFOCOM4`) | The Lurking Horror, v3 release 203, serial 870506 |
| *Lost Treasures* 5 (`INFOCOM5`) | Trinity, v4 release 12, serial 860926 |
| *Lost Treasures* 6 (`INFOCOM6`) | Sherlock, v5 release 21, serial 871214 |
| *Lost Treasures* 7 (`INFOCOM7`) | Wishbringer, v3 release 69, serial 850920 |

Each of volumes 2–7 carries three to seven games; the one listed is the largest,
which is what opening the disk gives you when nothing on it wears a conventional
story name. Ask the picker or `--story` for the rest. The Apple IIgs *Beyond
Zork* is a happier note to end on than the trio above: it is the **same build**
as the Amiga floppy and the bare `.z5`, so for once all three media agree.

The PC disks add a smaller trap worth naming: *the same release can be a
different file size on different media*. `LURKING` is 153,600 bytes on one Atari
compilation and 129,024 on another, and both are v3 release 203 serial 870506 —
identical builds with different trailing padding. Size is never a release
identifier. Read the header.

Every graphical title ships a *different* build on its floppy; the v3/v5 ones
ship the same build on both media. A resource `.blb` beside a story is never a
third build — it holds artwork and no executable, so the release you play is
decided entirely by the file you open.

The practical rule, and the one the interpreter's own tests follow: a report made
on a disk image is reproduced on that disk image, and a finding names the release
it was measured on. `crates/app/tests/suites/real_media_releases.rs` pins this whole
table, plus the frame each build lays out, so an upgraded fixture announces
itself instead of quietly rebasing someone's investigation.

### The command-line player takes a floppy too

`zvm-cli` — the no-map DOS-style player — mounts a disk image exactly the way the
TUI does, and it cost nothing to give it: `blorb` hand-rolls every one of these
readers with zero dependencies, and `zvm-cli` already linked it. So
`zvm-cli "Zork I - The Great Underground Empire.adf"` drops you at *West of
House* off the original floppy, no unpacking, no rename, and the same
content-based identification decides what on the disk is a story.

**Exactly the way** is meant literally: the CLI opens every format the TUI does,
Macintosh disks included, because both go through the same table. Point it at a
graphical v6 disk of either kind and you get the v6 refusal — the one every
graphical Amiga floppy already gets, telling you to run it in babelmap — rather
than a complaint about the disk. That distinction matters: it says the mount
worked and only the renderer is missing.

One thing the CLI needs that a single-game floppy never asks for: **which one**.
Amiga releases came one game to a disk, but the compilations did not — an Atari
ST or PC collection carries four to six stories on a single image — so when more
than one turns up you get a menu. Here is a real Atari one:

```
This disk holds 4 stories:
  1) HITCHHIK/STORY.DAT  (v3 r56 s841221)
  2) BUREAUCR.ACY/STORY.DAT  (v4 r86 s870212)
  3) CUTHROAT/STORY.DAT  (v3 r23 s840809)
  4) LEATHER.GOD/STORY.DAT  (v3 r59 s860730)
Which one? [1-4] 3
Opening 3) CUTHROAT/STORY.DAT  (v3 r23 s840809)
```

That is the naming rule doing visible work. All four files are called
`STORY.DAT`; without the folder in front of them the menu would be four identical
lines, and `--story cuthroat` would have nothing to match. And every line carries
its Z-machine version, release and serial, which is not decoration either — the
collection holds three different builds of *Hitchhiker's* alone. The header tells
them apart when the filename refuses to.

A disk with one story opens straight into it and asks nothing. A disk with none
says what it mounted instead of failing later as a corrupt story file. And
nothing here ever blocks a script: pipe stdin, and rather than prompt at a
terminal that isn't there, `zvm-cli` lists the candidates and tells you to pass
**`--story <n|name>`** — a menu number, or any part of a name that picks out one
story.

**And each of those six games gets its own saves** (SQ-0850). A per-game save
directory used to be named after the story file, which was fine while one image
meant one game and quietly catastrophic once it did not: all six stories on an ST
compilation shared one `<image>.save/`, one `default.babelmap`, and whichever you
played last owned it. A story taken off a disk image is now keyed by its own
**release and serial** — `hitchhikers-guide-r56-s841221` — so two games on one
disk cannot collide, renaming the image keeps your saves, and the Amiga, DOS and
Atari ST presses of *Zork I* r88/840726 all reach the same directory because they
are the same build. A loose story file still keys on its filename, exactly as
before, so nothing you already have moves. `zvm-cli` and the TUI read one helper
for this, which is why `--story 3` off a compilation and the same game opened in
babelmap find each other's saves.

And the floppy now tells the CLI which *machine* it is, not merely which story.
A disk format is evidence, and evidence that only reaches one front-end is half
an answer: for a while the TUI took an `.adf` for an Amiga while `zvm-cli`
mounted the same floppy and then ran it as an IBM PC. Both now ask the same
question of the same code — `blorb::medium`, the one crate that recognises these
filesystems and the only one both front-ends share — so
`zvm-cli "Zork - The Undiscovered Underground.adf"` answers VERSION with
*Interpreter 4* where the bare story file answers *Interpreter 1*. It is a
**default**, never a verdict: `-I 6` still puts you on the IBM PC, off the
Amiga floppy or anywhere else.

## Z-machine

- **Standard Quetzal save/restore** — the game's own SAVE/RESTORE writes and reads
  the interchange Quetzal format, so a save you make here opens in Frotz and vice
  versa.
- **Story-dictionary introspection** — babelmap reads the game's built-in word list
  and turns it into live verb/noun autocomplete, so you type `exam` and the game's
  actual vocabulary completes it.
- **v4+ upper-window screen model** — cursor-addressed status lines and full-screen
  forms (Bureaucracy's infamous licence application, for one) render in a fixed
  grid pinned atop the transcript, and `read_char` keystrokes are forwarded so you
  fill those forms in place. The game is told the story pane's **real** size — the
  standard asks the interpreter to keep the current height and width in the header
  and lets it change them whenever it likes, so babelmap measures the pane and
  re-measures it on every terminal resize. A game's full-width form therefore lines
  up column-for-column with the prose beside it instead of floating in a fixed
  80-column box. Pin a fixed screen with `virtual_screen_cols`/`virtual_screen_rows`
  if you want a game's original layout back; when the pane is smaller than a pinned
  screen, the viewport auto-follows the cursor. The virtual window is themeable
  from `[elements]`: `upper_window` inks its cells, and `upper_window_border`
  both colours and shapes the frame around them. That frame is off by default —
  the bar sits flush against the story and the game keeps every row and column
  of the pane — so set `style = "single"` (or `double`/`thick`/`rounded`) if you
  want it boxed.
  During a `read_char` prompt keystrokes go to the game; only the hotkey prefix
  (default `Ctrl+P`) stays reserved.
- **Timed / interrupt input** — v4+ `read` and `read_char` honor their `time`+
  `routine` operands, so real-time games keep ticking while you think: the game's
  interrupt routine fires every N tenths of a second (countdowns and clocks — the
  bomb in Border Zone) and can cut the read short. Controlled by
  `honor_timed_input` (default on), the `/toggle-timed-input` command, and the
  settings row; `zvm-cli` takes `--no-timed-input`. The VM stays zero-dependency —
  the wall clock lives in the hosts, not the interpreter.
- **A different game every time you sit down** — every engine here runs the same
  xorshift generator, and a VM core built in isolation seeds it from a fixed
  constant so its own tests mean something. That is exactly the wrong thing to
  hand a *player*: a story that never calls the seeding opcode would deal the
  identical sequence on every launch, and a roguelike would be the same dungeon
  forever. So the app seeds each engine from the OS before the story boots —
  before, because a game's initialisation routine is precisely where the
  shuffling is done, and a seed installed after the first prompt changes nothing
  the player will ever see. Set `random_seed` in `config.toml` to pin it instead
  and the run becomes reproducible end to end; babelmap names the seed it used on
  the console at startup, so an interesting run can be asked for again. The VM
  crates stay dependency-free through all of it — the entropy comes from std's
  own OS-seeded hasher, not a crate.
- **Interpreter number** — the story header's interpreter number (byte `0x1E`)
  defaults to **1 (DECSystem-20)**, following Frotz's rule (6 / IBM PC only for
  v6) — unless you opened a release disk image, in which case the medium picks
  the number instead (an `.adf` is an Amiga's 4, an HFS volume a Macintosh's 3,
  a `.st` floppy an Atari ST's 5), in every front-end alike.
  One medium deliberately does **not** move it. A DOS floppy is an IBM PC, and
  the IBM PC's honest number is version-dependent — that *is* Frotz's rule — so
  it is already in force and there is nothing for the disk to add; pinning a flat
  6 would quietly flip *Beyond Zork* on the *Lost Treasures* disk over to CP437
  character graphics, which is a rendering decision and not a container one.
  The Atari ST used to be the second such abstention, and it is worth saying why
  it no longer is, because the reasoning is the useful part. The worry was that a
  number here travels with a palette, a screen and a set of default colours, and
  that announcing a machine we could not fully describe would produce an
  incoherent one. But the thing that goes wrong in that scenario is a number
  *contradicting* the artwork — and there is no ST artwork to contradict. Infocom
  never wrote a version-6 interpreter for the ST, so all thirty-nine stories
  across the nine ST compilations are v3, v4 or v5, and the collision cannot
  happen. Meanwhile the ST's own interpreters turned out to answer the rest of
  the questions outright: `INTWRD DC.B 5 — MACHINE ID FOR ATARI ST`, a white page
  under black text, and a colour table that is the standard's own eight colours.
  So the ST profile states what it knows, declines the one thing it does not (a
  version-6 screen, which the machine never had), and *Trinity* off an ST disk now
  answers VERSION with *Interpreter 5*.
  **Apple ProDOS is the second abstention, and for a different reason again**: it
  is the only medium here that names a *family* rather than a machine. ProDOS is
  the Apple II's filesystem from the IIe onward, and §11.1.3 gives that family
  three numbers — 2 Apple IIe, 9 Apple IIc, 10 Apple IIgs — with nothing on the
  volume to choose between them. Nor is that pedantry: eight of the ten ProDOS
  images in the reference collection boot GS/OS and carry 16-bit `SYS16`
  applications, which is a IIgs and nothing else, while *Arthur* and *Journey*
  ship `INFOCOM.SYSTEM` beside `BASIC.SYSTEM` — the 8-bit ProDOS 8 press, equally
  at home on a IIe. And unlike the ST, this corpus does contain version-6 art for
  a wrongly-claimed machine to disagree with. So a ProDOS disk leaves the rule
  already in force exactly where it is, which for a family whose own number cannot
  be named is the honest answer.
  This byte is what unlocks colour on several Infocom games: Beyond Zork, for
  instance, only emits colour to a non-IBM interpreter and falls back to
  reverse-video under IBM PC. Override it with the app's `interpreter_number` config
  key, `babelmap --interpreter-number N`, or `zvm-cli -I N` / `--interpreter N` —
  e.g. `6` selects the IBM PC path, which draws Beyond Zork's map box and cursor
  arrows as CP437 character graphics instead of Font 3. The `--interpreter-number`
  flag applies to one run only and is never written back to your config, so probing
  a game's behaviour can't quietly pin one machine for every story afterwards —
  unless you then set the value in the settings screen, which is a decision rather
  than a flag and persists like any other setting. Setting that row back to
  **default** removes the key, restoring the per-version rule on the next launch. The
  values are ZMSD §11.1.3's:

  | | | | |
  |---|---|---|---|
  | 1 DECSystem-20 | 4 Amiga | 7 Commodore 128 | 10 Apple IIgs |
  | 2 Apple IIe | 5 Atari ST | 8 Commodore 64 | 11 Tandy Color |
  | 3 Macintosh | 6 IBM PC | 9 Apple IIc | |
- **Interpreter profiles — the whole machine, not one byte.** Byte `0x1E` is not
  the only thing that makes a machine. A Version 6 game that reads it goes on to
  ask about the screen it has, the colours the interpreter calls default, and what
  "red" looks like here — and answering one of those as an Amiga while answering
  the rest as an IBM PC produces a machine that never existed. So the answers
  travel together as a named **profile**.

  **IBM PC** is the default and is simply what babelmap has always done: the
  Frotz interpreter-number rule above, the resource file's own declared art
  resolution, your terminal's colours reported as the interpreter defaults, and
  ZMSD §8.3.1's colour table.

  **Amiga** is the sibling, and it selects itself: a story booted straight out of
  an `.adf` release floppy came off an Amiga, so babelmap presents one — 
  interpreter number 4, the Amiga's 320×200 standard window (which is what makes
  the artwork in a native `Pic.data` archive scale onto the 640×400 screen, since
  that format has no `Reso` chunk to declare it), a dark grey page and white ink
  reported as the interpreter's defaults, and the palette Infocom's own Amiga
  interpreter loaded — a slightly darker green and blue than the standard's, a
  warmer yellow, and its own three Version 6 greys. Whatever you name outright
  still wins: a number set in config, `--interpreter-number`, or `-I` outranks
  the medium every time, and only the *default* moves.

  **Macintosh** is the third, and it was the last one to arrive because it was
  the last one anybody could *prove*. A Mac release floppy has mounted and played
  for a while, but what a Mac's page, palette and screen looked like was not
  something the media in hand could settle, and a bundle guessed from memory is
  exactly the incoherent half-machine profiles exist to prevent. Infocom's own
  Macintosh interpreter settles all of it, so the bundle now ships: interpreter
  number 3, black ink on a **white** page — the Mac's whole visual signature, and
  the exact opposite of the Amiga's dark grey — and the standard colour table,
  because the Mac's own colour mapping *is* that table and nothing more.

  It hangs on the **medium**, and it has to. The Amiga and the Macintosh wrote
  the same colour archive, byte for byte indistinguishable, and the Mac release
  disk proves it by carrying one. A volume cannot be mistaken that way: HFS is
  Apple's filesystem and nobody else wrote one.

  And the Macintosh is the one machine with **two screens**, which is the part
  worth knowing about. Infocom's Mac interpreter sized its window and picked its
  picture file in a single decision — a big colour Mac got a 640×400 window and
  the colour archive drawn at double size, and a standard compact Mac got a
  480×300 window and the *monochrome* archive drawn 1:1. So on a Mac disk the
  artwork you choose is the screen you get, and
  [the artwork's own page](v6-graphics.md#two-macintosh-screens) has the numbers.
  (512×342, the compact Mac's famous screen, is the *hardware* — the game window
  sits inside it under the menu bar, and the story is told about the window.)

  **Atari ST** is the fourth, and it is the one that shows a profile is allowed
  to say *"I don't know"* about part of itself. It answers interpreter number 5,
  black ink on a white page, and the standard colour table — all of it read out
  of Infocom's own ST interpreters, where `INTWRD DC.B 5` is labelled `MACHINE ID
  FOR ATARI ST`, `DEF_BACK 9`/`DEF_FORE 2` are commented *"default ST background
  id = white"* and *"foreground id = black"*, and the ST's colour table asks for
  the standard's own eight colours at full saturation. It states **no standard
  window at all**, and that absence is a fact rather than a gap: Infocom never
  wrote a version-6 interpreter for the ST, so there is no ST artwork for a
  standard window to be the resolution of. (The machine could show only four of
  its eight colours at once in 80-column mode, one of them always the background
  — a display ceiling a terminal does not have, so there is nothing to express.)

  The ST is also the clearest demonstration that this byte is not decoration.
  *Beyond Zork* on an ST compilation, told it was a DECSystem-20, opened by
  asking **"Is this a VT220?"** — a question about a 1983 DEC terminal, put to
  someone who has just inserted an Atari floppy — and a player who answered *no*
  got a stripped-down screen: no box around the room description, the compass
  rose drawn as `\` and `@-`, and *"use the UP and DOWN arrow keys"* spelled out
  in words. Told it is an Atari ST, the game never asks, because an ST is not a
  terminal that might or might not have line-drawing characters. It goes straight
  to the boxed layout with its block-graphic compass and real `↑`/`↓` arrows —
  the same screen the DEC-20 player only reached by answering *yes* — and it
  signs itself *"Atari ST Color Version A"* where it used to say *"DEC-20"*. That
  "Version A" is corroboration in its own right: Infocom's ST version-5
  interpreter is stamped **FROZEN Version A** in its source.

  Across the rest of that corpus the change is quiet, which is the point of
  having measured it: of the thirty-nine stories on the nine ST compilations,
  thirty-two behave identically, six merely print the new number in their VERSION
  block, and only *Beyond Zork* does anything differently. The version-3 stories
  cannot notice at all — byte `0x1E` has no meaning before version 4, which is
  why the ST's own version-3 interpreter leaves it zero and comments it
  *"(UNUSED)"*.

  The artwork can select the machine too, and it sits between the two. If you
  name a picture archive for a game — the `pictures` key described under
  [Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws) —
  then you have said which machine's rendition you want to look at, and babelmap
  presents that machine: a `Pic.data` is an Amiga, an `.MG1`/`.EG1`/`.CG1` is an
  IBM PC. It reads that from the file's *contents*, never its extension, since
  the two containers are structurally different and a renamed file would
  otherwise lie about which machine you asked for. (The Macintosh wrote the same
  container as the Amiga and cannot be told apart from it in general, so it is
  not claimed to be — naming an archive off a Mac *disk* still gets you the
  Macintosh, from the disk underneath it.) MCGA, EGA and CGA are three video
  cards in one machine, so
  all three name the IBM PC and none of them moves byte `0x1E`; what a card does
  change is how densely its artwork was stored, which is
  [the art's business rather than the machine's](v6-graphics.md#choosing-which-artwork-a-game-draws).
  The character cell is 8×16 on every profile — EGA's own 640×200 mode on an 8×8
  cell is the same 80×25 grid — so no *rendition* alters the screen a game is
  handed. (The one machine that would have moved it is the Macintosh, whose
  interpreter typeset Version 6 in 12-point Geneva on a 7×15 cell. babelmap keeps
  its 8×16, so a standard-Mac screen comes out 60×19 characters where a real Mac
  fitted 68×20 — slightly larger type, and four pixels of slack at the bottom.
  Making the cell a per-machine runtime value reaches into every corner of the
  screen model, and is not something a profile should smuggle in.) Setting
  `interpreter_number` yourself names the
  machine outright and outranks both, so `interpreter_number = 4` gets you
  the whole Amiga rather than just the byte — which is the point: a number that
  changed what games did without changing the machine it implied was never a
  useful thing to be able to set.

  You can set it per game as well as globally. The
  [launch-options dialog](v6-graphics.md#three-ways-to-say-it) — **Shift-Enter**
  on a story in the browser — shows the number your art choice implies *and where
  it came from*, lets you pin a different one for that launch, and will write
  `interpreter_number` into the game's own `config.toml` if you tick the box.
  Most specific first: the dialog's choice for this launch, then
  `--interpreter-number`, then the game's sidecar, then the global config, then
  the inference above. It belongs in a *launch* dialog rather than the settings
  screen because header byte `$1E` is read by the story itself at boot — a game
  that has already started has already made decisions from it, so offering to
  change it mid-session would be offering something babelmap cannot deliver.

  Authenticity can cost readability — *Zork Zero* under an Amiga picks a colour
  scheme that was easy on a 1989 monitor and is merely adequate in a modern
  terminal. There is no separate switch for that on purpose: `honor_game_colours`
  already decides whether the game's colour choices are honoured at all, so
  turning it off hands the screen back to your theme, profile or no profile.

  **The Amiga had two pens, and moving one repaints the screen.** This is the one
  place where claiming to be an Amiga changes not just what a game is *told* but
  what happens when it acts on it, and the standard is blunt about it. Version 6
  normally gives every window its own foreground and background — eight windows,
  eight pairs — but ZMSD §8.3 carves out this machine: a Version 6 interpreter
  going under the Amiga interpreter number "must use the same pair of colours for
  all windows when running Infocom's games", and if either colour changes it "must
  change the colour of all text on the screen to match". The reason is hardware.
  The Amiga drew text through two colour *registers* and changed a colour by
  reloading the register, so every glyph already on the display changed with it —
  there was no way to give one window, or one word, a colour of its own.

  babelmap does exactly that. Under interpreter 4 a `set_colour` **from window 0**
  loads the machine's two pens, every window adopts them, and every glyph already
  drawn — status grids, the pixel-positioned labels on *Zork Zero*'s banner
  ribbons, the prose a window has scrolled, even prose left frozen behind a window
  that has since moved — is repainted in them. *Zork Zero* is the title that shows
  it off: it boots black-on-light-grey on its story window and the whole screen
  goes with it.

  **And a `set_colour` from any other window is ignored** — which is the one place
  babelmap deliberately departs from the letter of §8.3, so it is worth saying why.
  The standard does not mention such a gate; Infocom's own released Amiga
  interpreter does, in as many words: it changes text colours *"only in window 0,
  and ignore[s] requests in other windows (except for the special case of
  bg = -1)"*. §8.3's stated purpose is to **simulate the Amiga hardware**, so a
  reading of it that makes babelmap diverge from that hardware defeats the rule's
  own reason for existing — and Infocom's interpreter is the better authority on
  how Infocom's games looked on Infocom's machine. *Journey* settles it: its Amiga
  release (30 / 890322) makes exactly one `set_colour`, asking for white on black,
  and makes it on window 3. Applied globally that paints the game black; real
  Amiga captures show *Journey* on grey with white text instead — the Amiga's
  *default* pair, `DEF_BACK` over `DEF_FORE 9`. The real machine dropped the call,
  and so does babelmap. (If you are ever tempted to "correct" this back to the bare
  text of the standard: that is the change, and this is the paragraph explaining
  why it was not made.)

  **And the floppy outranks the leaked source.** babelmap took the Amiga's numbers
  from `amiga/yzip1.c` and `amiga/yzip.h` in Infocom's leaked interpreter sources,
  which are a *development* snapshot. In two places they disagree with what
  Infocom actually pressed onto the disks, and the second of the two is the whole
  screen:

  | constant | leaked source | on every release floppy |
  |---|---|---|
  | `colortable[5]` — standard colour 5, yellow | `$0EE0` | **`$0FD0`** |
  | `DEF_BACK` — the page every Amiga game is played on | 11, medium grey `$777` | **12, dark grey `$444`** |

  Each Amiga disk in `stories/` carries its own 68000 interpreter beside the
  story, and those programs are the authority: they are what painted the screens.
  `set_back()` opens `if (id == 1) id = DEF_BACK;` and compiles to
  `cmpi.w #1,d7` / `bne.s` / `moveq #12,d7` in all four; `set_color()`'s
  `return ((DEF_BACK << 8) | DEF_FORE)` assembles to `move.w #$0C09,d0` in all
  four; `$0B09` occurs in none of them. Real captures agree — a *Journey*
  release‑30 screen tallies 173,994 pixels of `#444444` under 25,878 of `#FFFFFF`,
  and an *Arthur* church screen is `#444444` under `#FFFFFF` with the status bar
  *reversed* to `#444444` on `#FFFFFF`, which is pens 0 and 1 swapped and so proves
  the page is the text background register rather than artwork.
  `crates/app/tests/suites/v6_amiga_shipped_interpreter.rs` reads all of this back
  off the disks on every run, precisely so that a future reader who reaches for
  `yzip.h` is told by a failing test that the machine disagrees. (SQ-0822.)

  **On this machine, a bracketed line is not a message from the interpreter.**
  babelmap normally mutes a whole line in `[brackets]` in the transcript, on the
  reasonable guess that it came from the interpreter rather than the story. Under
  §8.3's Amiga that guess is wrong twice: *Arthur*'s
  `[You have earned ten chivalry points.]` is the game's own prose in the game's
  own pens, and the muted colour was chosen to recede against your *theme's* page,
  not against the machine's dark grey — where it reads as grey on grey. So the
  rule stands down while the machine owns the ink. Your own `[transcript.rules]`
  entries are unaffected (they are explicit, and they always win), and so is the
  room-heading highlight, which paints an accent rather than a mute and stays
  legible on any page.

  **The machine's default pair is painted, not merely advertised.** §8.3.3 has an
  interpreter write its own default background and foreground into header bytes
  `$2C`/`$2D` so the story can read them, and babelmap has always written the
  Amiga's. Under interpreter 4 those two bytes are also the *screen*: on real
  hardware they are the registers, so every pixel no picture and no `set_colour`
  claimed is the background pen. So they are what babelmap paints with too — the
  page under the frame, the ink of any text that named no colour of its own. That
  is what makes an Amiga *look* like an Amiga rather than merely report as one:
  *Journey* on its release floppy is white text on the machine's dark grey, frame
  and menu and prose alike, instead of your terminal's own colours.

  **The Macintosh needed the same thing, and found out the same way.** *Zork
  Zero* off its Mac disk never calls `set_colour` even once — the game asks a
  Macintosh for nothing — so every window sat at "default", and with nothing
  painting `$2C`/`$2D` the whole screen fell through to your theme. The visible
  symptom was the status banner: location and score drawn in the theme's grey on
  the game's own white plate, on a two-colour machine that has no grey in it
  anywhere, which reads as text that failed to render. A Mac window was white
  with black type, Infocom's own interpreter says so in one line, and that is
  now the page babelmap paints. There is no claim about shared pens here — that
  part is the Amiga's alone; a Mac `set_colour` still colours one window, exactly
  as §8.3 describes. This is only the ground beneath a window that asked for
  nothing, and `honor_game_colours = false` still hands it back to your theme.

  **What you are typing stands on that page too.** The line you are composing is
  drawn by babelmap rather than by the story, and it used to resolve its ink from
  your theme alone — which on a machine page is a coin toss. On the Amiga it won
  the toss, because the theme's body ink is white and so is `DEF_FORE`; on the
  Macintosh it lost it completely, and typing into a white Mac page was typing in
  white on white. You could not see a word until you pressed Enter, whereupon the
  game echoed the command back as prose and it appeared, in black. So the live
  echo now stands on the same ground the committed text does: the machine's own
  pair, the same characters rendering the same way whether you have pressed Enter
  or not. A game that *asks* for colours with `set_colour` still wins over the
  machine's defaults, exactly as it always did — and so does a `style.toml` that
  names `input_text` or `input_prompt` by hand, because the machine's page is a
  default and anything you declare outranks a default.

  Two things the rule deliberately does *not* do. Colour **-1**, "the colour of the
  pixel under the cursor", names no colour, so it loads no pen — it stays a
  request to draw over what is already there, which is how *Zork Zero* prints its
  banner labels straight onto the ribbon artwork (and it is the one request a
  window other than 0 may still make). And a pen carries ink and page both, but a
  page nobody ever laid down is not a pixel a pen can reach: a window the game
  never gave a background keeps painting nothing behind its glyphs, or a single
  black `set_colour` would paint *Journey*'s own illustration out of its frame.
  Everything else — every non-Amiga profile, and any profile at all with
  `honor_game_colours` off, where babelmap has told the story it has no colours to
  offer — keeps one pair per window and the host theme's own page, exactly as §8.3
  describes for every other machine.
- **v6 graphical stories** — babelmap boots and plays graphical v6 titles,
  verified against *Zork Zero*'s full frame. On an image-capable terminal
  (Kitty / iTerm2 / Sixel) the game's chrome — the decorative frame, status
  line, and per-room compass — renders as one scaled, **pixel-aspect-accurate**
  image (uniform scaling, letterboxed, never stretched); the game itself lays
  this out by querying invisible "placement" pictures, which babelmap answers
  from the Blorb's own dimension data. The `v6_render` setting (see
  Customization) picks how the story text is drawn: the default `hybrid` mode
  keeps it as real, crisp terminal text inside the chrome; `raster` bakes it
  into the pixel image instead, bitmap-font style. Without an image protocol,
  v6 falls back to a character-cell rendering. Full depth — the three render
  modes, inline drop-caps, pixel-positioned status text and colour — is in
  [Graphical v6](v6-graphics.md). (v6's menu and mouse opcodes are not yet
  wired up.)

## Glulx

- **External files** — Glulx games persist their own data through Glk file streams;
  a game's fixed-name saves and caches are read and written for it silently. (See
  [saves](saves.md) for how this dovetails with babelmap's Save States.)
- **Accelerated-function interception** — big Glulx games reach the first prompt
  dramatically faster. Well-known Inform veneer functions the game registers via
  `accelfunc` are recognized and run natively instead of grinding through full VM
  dispatch, so a heavyweight like Counterfeit Monkey stops making you wait through
  its startup. On by default; disable with `--no-accel` (`gvm-cli` and the app).
- **Floating-point math** — the complete float opcode set is implemented, in both
  single **and** double precision: conversions, arithmetic, `sqrt`/`exp`/`log`/
  `pow`, trigonometry, and the fuzzy comparisons `jfeq`…`jisinf`. Games that
  compute with floats — Counterfeit Monkey's in-game graphics scaling, say — run
  instead of faulting, and the `gestalt` opcode answers `Float` and `Double`
  truthfully so a game can probe first.
- **Line-input terminators** — babelmap honors `glk_set_terminators_line_event`, so
  a game can register special keys (Escape and the function keys `Func1`–`Func12`)
  that end a line of input; the terminating keycode comes back in the line event's
  second value (`val2`; `0` for a normal Enter).
  `glk_gestalt(gestalt_LineTerminators/LineTerminatorKey)` answers truthfully so
  games can check before relying on it.

## Sound

- **Z-machine** — the `sound_effect` opcode's two built-in bleeps (#1 high / #2 low)
  play as real synthesized tones, and Blorb `Snd ` resources (#≥3) play as sampled
  audio (AIFF, Ogg, or ProTracker MOD), in both the `app` TUI and `zvm-cli`. Sound
  resources come from the story file itself if it's a Blorb, else from a sibling
  `.blb`/`.blorb` next to it. On every bleep the story-pane border also flashes in
  a distinct, themeable colour (`sound_beep_high` / `sound_beep_low`) — a
  complementary and accessibility cue, and the *only* cue when sound is off.
  Controlled by `enable_sound` (default on) and `volume` (0–100, default 100);
  toggle it with `/toggle-sound` or the `F2` settings row, adjust it with
  `/volume <0-100>`, and use `/play-sound <resource-id>` to fire a Blorb `Snd `
  resource on demand for verifying the audio path. Both the `app` and `zvm-cli`
  take `--no-sound` to start muted for a single run (leaving `enable_sound`
  untouched); `zvm-cli` also takes `--volume <0-100>`.
- **Glulx** — Glk sound channels (`glk_schannel_*`) play a Blorb's AIFF/Ogg/MOD
  `Snd ` resources with per-channel volume (including gradual volume ramps) and
  sound-finished notify events, so music and effects behave the way the author
  wired them.

Sound always plays on the local device babelmap runs on; to route audio from a
remote/SSH session back to your own machine, see
[`docs/remote-sound.md`](../remote-sound.md). Unimplemented-opcode warnings
surface in the transcript as meta lines (hidden by `/filter story`) rather than
spilling onto stderr.

## Game-driven colour

When a game asks for colour, babelmap gives it colour — on your terms. The
Z-machine's v5+ `set_colour` and `set_true_colour` are honored: the standard
palette (black/red/green/…) maps onto *your* colour scheme, so a game's "red" is
your red rather than a hard-coded shade, while greys and true-colour render as
exact 24-bit RGB. Colour and reverse-video apply in both the transcript and the
upper-window grid. **Glulx/Glk** games get the same treatment —
`stylehint_TextColor`/`BackColor`/`ReverseColor` render at full 24-bit fidelity.

It all sits under one switch, `honor_game_colours` (default **on**): flip it in the
F2 settings screen to let your theme own every colour instead. `zvm-cli` and
`gvm-cli` render the same colours as ANSI SGR and both accept `--no-game-colours`
to opt out, as does setting `NO_COLOR` to a non-empty value.

One thing turns it off for you. A game drawing **two-colour (CGA) artwork** is
told the interpreter has no colours, because it has none — that artwork is a
stencil whose own white is paint and whose transparency is meant to show your
background through, and a story that thinks it is on a colour display paints over
both. See [The colours come with the card](v6-graphics.md#the-colours-come-with-the-card).
It applies to that story only and is never written back to your config, so
choosing a `.cg1` once cannot quietly strip the colours from every other game.

## Plain text, for screen readers

All three CLIs accept **`--screen-reader`** (alias `--plain`), and select it
automatically under `TERM=dumb`. It emits no escape sequences at all: no colour,
no cursor addressing, no scroll region, no pinned status line, no alternate
screen — just linear, append-only text a screen reader can follow and scrollback
can review. `[MORE]` paging goes too, since a blocking prompt that hides the rest
of the output behind a keypress is the shape a reader copes with worst. Line
editing and echo go back to the terminal, so the reader announces typed
characters and the user's familiar editing keys work.

What would otherwise be spatial arrives in reading order instead: the Z-machine
status line and upper window come through as ordinary lines, and Glk TextGrid
windows stream inline, deduped so an unchanged status bar doesn't repeat every
turn. **Menus** get more than that — see below.

**The status line is not narrated every turn.** A Z-machine v3 status line
carries a move counter, so it differs on every single turn and no amount of
change-detection will suppress it — measured, Ballyhoo repeats it on four turns
out of four. Screen-reader mode therefore leaves it out and lets you ask with `/status`.
`--show-status` puts it back if you would rather have it whenever the story
updates it.

The suppression goes by *size*, and only a one-row region is treated as chrome.
Anything taller is content the game means you to read: the Infocom releases with
integrated InvisiClues draw their hint menus in the upper window — Planetfall's
is twelve chapter headings and a `RETURN = See hint / Q = Resume story` legend —
and Lost Pig's HELP menu and Bureaucracy's licence-application form are the same
shape. Those always come through. **`--story-only`** is the blunt instrument for
anyone who wants the whole upper window gone, menus included — it is deliberately
a separate, stronger switch, and it works with or without `--plain`. `gvm-cli`
takes it too, where it suppresses every Glk grid window. (Scott has no status
window to suppress: its room block *is* the story.)

The status also lands in the right place. A game writes its prompt last and
without a trailing newline, and the host only learns the turn is over when the
game asks for input — so a naive host can only append the status *after* the
prompt, giving `> In the Wings   Score: 0`, which reads as though the prompt were
showing you a room. In this mode the prompt is held back until the status has
gone out, so a turn reads description, then status, then prompt. `/status`
answers the same way, and puts the prompt back after itself.

`scott-cli` drops its em-dash divider rule in this mode. It stands in for the
boundary a real Scott terminal drew between its two windows, and a reader either
announces thirty-odd em-dashes one at a time or swallows the line — neither of
which conveys a boundary.

### Menus are numbered, and a move is one line

A menu is a rectangle the game repaints: a list with a `>` parked on the current
item and a legend saying which keys move it. Sighted, the marker jumps and
nothing else happens. Linearised, *every repaint is a fresh block of text* —
measured, `N` at Planetfall's InvisiClues menu read out sixteen lines, and
Arthur's read out twenty-three, on every single press, to say that a `>` had
moved down one row. Followable, but not usable.

So in screen-reader mode the host recognises the repaint. A menu is read out
**once**, host-numbered:

```
                               INVISICLUES (tm)
 N = Next                                                     P = Previous
 RETURN = See hint                                        Q = Resume story

[menu — type a number to jump, Enter to select]
>1. THE FEINSTEIN
 2. THE POD TRIP
 3. THE DORMITORY
 …
```

and after that a marker move is announced in one line:

```
>3. THE DORMITORY (3 of 12)
```

**Detection is a mechanical diff, not a guess about content.** The host keeps the
last block emitted from each source (the Z-machine upper window; each Glk grid;
the Glk story stream) and compares. If two blocks differ *only* in where the
marker sits — same items, same headers, same legend — that is navigation. Any
other difference is content and is emitted in full, unchanged. A status line
whose text changed, a menu that scrolled, a form that gained a field: all differ
somewhere other than the marker column, so none of them is ever swallowed. This
is the whole safety argument, and it is pinned by tests on both engines.

**Which lines get numbers** is decided by shape: an item is a non-blank line
whose text begins at the same column as the marked line's, with nothing but
blanks and marker characters in front of it. That is exactly the items in all
three measured menus and none of their furniture — Arthur's centred title
(column 20), its `N = next item` legend (column 1) and its `(more)` pagination
hint (column 4) are all left unnumbered, as are Planetfall's title and two
legend rows. The rule errs towards numbering more lines rather than fewer: an
over-numbered header is an annoyance, an unreachable item is a dead end. (The
alternative — numbering only the lines the marker has been seen on — renumbers
the menu under the player as they explore it.) A list the game repaints twice
into one block, as Counterfeit Monkey's does, counts once.

**Typing a number jumps to that item.** The host cannot teleport the marker — the
game owns it — so it walks the menu with the game's own keys: `n`/`p` when the
legend names them (`N = Next`, `P = previous item`), else Down/Up (ZSCII 129/130
for the Z-machine, `keycode_Up`/`keycode_Down` for Glk). It steers rather than
counting: press, read where the marker actually landed, decide again — because
Arthur's `N` steps straight over its unselectable section headings, and a
press-count worked out in advance would sail past the item you asked for. The
landing is announced in the move format; the intermediate steps are silent; and
an ordinal the marker will not stop on gives up and reports where it ended
instead of pressing forever.

Numbers are only intercepted while a menu is open, and only for an ordinal the
menu actually has. Everything else — `n`, `p`, Enter, `q`, a digit at an ordinary
prompt — reaches the game untouched.

**`/menu`** re-reads the open menu, numbered, on demand, and says
`[no menu is open]` when there isn't one. It is the `/status` precedent, and
because screen-reader mode leaves the terminal cooked, a menu "keypress" is
really a whole line terminated by Enter — so `/menu` and multi-digit jumps work
at a menu's own prompt, not just at a line prompt. (That termination rule is not
a choice: it is the shape of the read. Raw mode would deliver `1` then `2` with
no way to tell `12` from item 1 followed by item 2.)

None of this applies outside `--screen-reader`. On a terminal the menu is painted
in place and nothing repeats; on a plain pipe the output is a transcript that
stays byte-identical.

### Score changes are announced

Quietening the status line takes the score with it, and the score is the part
that carries news — a sighted player watches it tick over, a listener would have
to keep asking. So in screen-reader mode a score that *moves* is announced above
the prompt:

```
You put the gold idol on the pedestal.

[Score 1, up 1]
>
```

Only on change, never on the first sighting (the score you started with is not an
event), and words rather than `+1`, because a reader announces "plus" only at
higher punctuation settings.

Where the number comes from differs sharply by format, and two of the three are
exact while the rest is pattern-matching:

| | source | |
|---|---|---|
| Z-machine v1–v3 | global 2, which the standard reserves for the score (ZMSD §8.2) | exact |
| Z-machine v4+ | the status line the game drew | recovered from text |
| Glulx | the Glk grid window — Glk has no concept of a score at all | recovered from text |
| Scott Adams | treasures deposited in the treasure room, recounted each turn | exact |

The text-recovery cases look for a `Score: N` field and take the last one on the
line, so a room called "Score Board" doesn't become your score. A game that words
it differently — "Points", a bare number, a translated status line — simply isn't
matched, and the announcement stays silent rather than reporting a wrong figure.
A Z-machine *time* game has a clock where the score would be, and is correctly
never announced.

### `/status`

Status text reaches a listener only when the game chooses to write it, and then
it scrolls away; a sighted player re-reads a pinned line for free. All three CLIs
answer **`/status`** at any line prompt with the current status — the Z-machine
status line or upper window, the Glk grid windows, or a Scott room block — and
the game never sees the command. The leading slash is what makes intercepting it
safe: no interactive-fiction parser gives `/` a meaning, so no game verb is
shadowed, and babelmap's own TUI already spells host commands that way. (A `char`
prompt — "press any key" — takes the keypress as itself; `/status` is a line
command. `/menu` is the exception, because a menu *is* a char prompt and a
command that could not be typed at one would be useless.)

This is the same output path piped/redirected use has always taken — kept honest
by the test harnesses that read it — so `--screen-reader` mostly makes it *selectable*
without giving up an interactive terminal. `NO_COLOR` deliberately does **not**
imply this mode: [the convention](https://no-color.org/) is about colour, and
someone who sets it has not asked to lose their status line.

> **Not yet validated with a real screen reader.** The escape output, input
> paths, and TTY gating are measured; NVDA/Orca/VoiceOver behaviour is not. If
> you use one, we would like to hear how this goes.

## `[MORE]` paging in the CLIs

A turn that prints more than a screenful used to scroll its own beginning away in
`gvm-cli` and `scott-cli`; only `zvm-cli` paused. All three now stop at the
bottom of a page with a reverse-video `[MORE]` bar and wait for a key, the way
the original interpreters did and the way the TUI already did. `--no-more`
(alias `--no-page`) turns it off.

Paging requires **both** ends to be a terminal — pausing for a key that a pipe
will never send is a hang, which is why the headless harnesses never see it — and
is off in `--screen-reader` by choice. `gvm-cli` pages only its streaming story
window; a game using several buffer windows is painted as fixed panels with their
own scrollback, so there is no bottom of the page to stop at.

## Robustness

When a story faults — out-of-bounds memory, stack under/overflow, an unimplemented
opcode — it doesn't take the interpreter down with it. The game halts with a
call-frame stack trace (the faulting PC and opcode, plus each frame's return
address and locals). In the app the trace appears inline in the transcript and the
app **stays interactive**: the map, scrollback, and a deliberate quit all keep
working, and a durable copy lands in `~/.babelmap/crash.log`. `zvm-cli`/`gvm-cli`
print the trace to stderr and exit 70.
