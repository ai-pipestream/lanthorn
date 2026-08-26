# Bundled font assets

## `u_vga16-subset.bdf` — the v6 raster text face (SQ-0932)

The **Uni-VGA** font by Dmitry Bolkhovityanov: an 8×16 VGA console face, drawn
from scratch and extended to Unicode. `crates/app/src/render/vga16.rs` is
generated from this file, and `crates/app/src/render/bitfont.rs` draws v6 raster
text with it.

### Why this font

The v6 raster path had one face — a public-domain **8×8** master nearest-row
doubled into the 16-pixel cell — so half the cell's vertical resolution went to
duplicated scanlines and no glyph had a real descender. Uni-VGA is natively
8×16, which is exactly `v6_layout`'s `FONT_W`×`FONT_H`, so it blits 1:1 with no
resampling at all.

It is also the *right* face rather than merely a better one. `InterpreterProfile`
falls through to `IbmPc` whenever no medium names a machine, so most v6 raster
frames are drawn as a DOS machine — and since SQ-0928 they are painted in that
machine's own EGA blue and white. Uni-VGA's Latin glyphs are the IBM VGA text
face those pixels belong to.

**No ROM was used.** Uni-VGA was drawn from scratch; this repository has never
contained a dumped hardware font, and the alternative candidates that *are* ROM
tracings were rejected on licence grounds (VileR's Ultimate Oldschool PC Font
Pack is CC BY-SA, incompatible with shipping inside a BSD-3-Clause binary).

### Provenance

| | |
|---|---|
| upstream | <https://www.inp.nsk.su/~bolkhov/files/fonts/univga/> |
| tarball | `uni-vga.tgz`, sha256 `6667a143e8d06765fb7027116af563b1b8ee833f310744673fea6a5f20d25617` |
| full font | `uni_vga/u_vga16.bdf`, sha256 `6a7420c7f49bb1888ebd318c6adede6c8458565232bcb7fcc9e1f27d8e40fdce` |
| author | Dmitry Bolkhovityanov, `bolkhov@inp.nsk.su` |
| licence | the X licence — see below |

The licence is stated on the author's distribution page, **not** inside the
tarball, which is worth knowing before anyone goes looking for a `LICENSE` file
and concludes there isn't one. The page says verbatim:

> The UNI-VGA font can be distributed and modified freely, according to the X
> license.

The BDF header carries `COPYRIGHT "Copyright (c) 2000 Dmitry Bolkhovityanov,
bolkhov@inp.nsk.su"` and `NOTICE "VGA is a trademark of IBM Corporation."`; the
subset preserves the original header intact, so both travel with the file.
`uni-vga.lsm` in the tarball says only `Copying-policy: Freely Distributable`,
which is a category, not a licence — the page above is the authority.

### Why a subset, and what is deliberately left out

The full font is 2,899 glyphs / 386 KB, almost all of it Cyrillic, Greek and
CJK-adjacent coverage no Infocom title prints. The subset is 194 glyphs — the
**text** the v6 raster path can reach:

| range | what |
|---|---|
| `U+0020`–`U+007E` | printable ASCII |
| `U+00A0`–`U+00FF` | Latin-1 Supplement — the ZSCII default Unicode table (ZMSD §3.8.5) |
| `U+0152`, `U+0153` | `Œ`/`œ`, also ZSCII default table |
| `U+FFFD` | the unknown-glyph placeholder |

**Font 3 is excluded on purpose.** Box drawing, block elements and the cursor
arrows stay on `bitfont.rs`'s 8×8 masters, because font 3 is a graphics character
set rather than a typeface: nothing printed as *text* can reach `U+2500`, and
those codepoints exist in the raster path only because
`zvm::cpu::exec::font3_translate` maps font-3 codes onto them. Drawing them from a
VGA text font is a category error, and on the Amiga it is the wrong machine's
glyph twice over — *Journey* ships its own 8×8 font-3 file (`Char.data`,
byte-identical to *Beyond Zork*'s `Graphic.Data`) and that is what a real Amiga
drew its rules with.

It is not a free mistake either. Uni-VGA's `│` is **two** pixels wide, CP437's
authentic double-column vertical, where `font8x8`'s is one. An earlier cut of this
subset included box drawing, moved Journey's frame border by a pixel, and failed
`v6_journey_prose_containment::journeys_frame_border_is_a_single_native_pixel_column`
— whose hairline reading later assertions depend on. v6 rule geometry was not
SQ-0932's to change; `bitfont::tests::the_vertical_rule_stays_one_pixel_wide` now
pins it directly.

### How to regenerate

Fetch the upstream tarball, check it against the sha256 above, and keep only the
records whose `ENCODING` is wanted — every glyph in this font is uniformly
`BBX 8 16 0 -4` / `DWIDTH 8 0`, so a record is 16 bytes of bitmap and needs no
bearing arithmetic. Then regenerate `vga16.rs` from the subset;
`vga16::tests::the_table_matches_the_bdf_it_was_generated_from` parses this file
and fails if the two ever disagree.

## `7x14-subset.bdf` — the v6 raster fallback for a 7-wide cell (SQ-1016)

The **X11 misc-fixed** `7x14` font by Markus Kuhn: a public-domain 7×14
fixed-pitch console face from the X.Org `misc-fixed` distribution.
`crates/app/src/render/misc7x14.rs` is generated from this file.

**When it draws:** `render::bitfont::blit_glyph_styled` reaches for it when the
cell is exactly **7 wide and at least 14 tall**, and the release's own face has
already declined — so in practice a Macintosh 7×15 cell (SQ-0917) with no volume
behind it. `FONT` 524 off the game's own floppy still wins (SQ-1011); `vga16`
answers every other cell, unchanged. `cw == 7` exactly, because this face has no
horizontal resampler — its rows are 7 bits and column `c` is drawn at `dx == c`.
`ch >= 14` because that is where all fourteen source rows survive: at 15,
`dy * 14 / 15` uses every one and doubles row 0, which is blank in 174 of the 194
glyphs, while at 13 source row 13 is never reached and 28 glyphs ink it (the tails
of `g j p q y`, the comma, the semicolon, `Ç`'s cedilla).

### Why this font

The Macintosh's 7×15 body face (FONT 524, `mac/xzip.lst`) lives only on
gitignored commercial disk media. A Mac-shaped v6 cell reachable without one —
for instance a bare `.z6` booted under `--interpreter 3` (SQ-1011) — has no
face to draw with, and every disk font is otherwise untestable in CI since none
of them can be committed. `misc-fixed`'s `7x14` is 7 pixels wide, matching the
Macintosh cell, and is public domain, so it can be embedded and exercised in
CI without touching any commercial fixture.

It does **not** substitute for the Macintosh's 7×12 *alt* face (FONT 1033),
which is Z-machine font 3 — box drawing, block elements, cursor arrows — a
graphics character set that must tile edge to edge, a constraint no text font
satisfies. See SQ-1017.

### Provenance

| | |
|---|---|
| upstream | <https://www.cl.cam.ac.uk/~mgk25/ucs-fonts.html> |
| tarball | `misc-fixed` source tarball, sha256 `702fd1cdef9123e1871622a897727977c0933a420c50c94198f5bb22de8f0f8a` |
| full font | `7x14.bdf`, sha256 `e366d3c685659fb69ab05d5994d3d6debe897d89853f0188a1fca62d1132503f` |
| author | Markus Kuhn (and other X11/X.Org contributors) |
| licence | public domain |

This is stronger evidence than Uni-VGA's, whose licence is only asserted on the
author's web page: the `COPYRIGHT` property is baked directly into the BDF
itself, verbatim:

> Public domain font.  Share and enjoy.

The subset preserves the original `STARTFONT`/`FONT`/`STARTPROPERTIES`…
`ENDPROPERTIES` header intact (including that `COPYRIGHT` line), so the file is
self-documenting about its own provenance. **No ROM was traced** — same
standard as Uni-VGA above: `misc-fixed` was drawn for X11 from scratch, not
dumped from hardware.

### Why a subset, and what is deliberately left out

The full font is 2,576 glyphs. The subset is 194 glyphs, the exact same
repertoire cut from Uni-VGA above:

| range | what |
|---|---|
| `U+0020`–`U+007E` | printable ASCII |
| `U+00A0`–`U+00FF` | Latin-1 Supplement — the ZSCII default Unicode table (ZMSD §3.8.5) |
| `U+0152`, `U+0153` | `Œ`/`œ`, also ZSCII default table |
| `U+FFFD` | the unknown-glyph placeholder |

**Font 3 is excluded**, for the same reason as Uni-VGA's subset: box drawing,
block elements and cursor arrows are a graphics character set, not a typeface,
and nothing printed as *text* can reach `U+2500`.

### How to regenerate

Fetch the upstream tarball, check it against the sha256 above, and keep only
the records whose `ENCODING` is wanted — every glyph in this font is uniformly
`BBX 7 14 0 -2` / `DWIDTH 7 0`, so a record is 14 bytes of bitmap and needs no
bearing arithmetic. Then regenerate `misc7x14.rs` from the subset;
`misc7x14::tests::the_table_matches_the_bdf_it_was_generated_from` parses this
file and fails if the two ever disagree.
