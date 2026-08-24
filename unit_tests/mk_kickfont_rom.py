#!/usr/bin/env python3
"""Generate `unit_tests/kickfont.rom` — a SYNTHETIC Kickstart-shaped ROM image
carrying three `TextFont` records, for SQ-1053.

    python3 unit_tests/mk_kickfont_rom.py

Why this exists at all: the Amiga's Version 6 interpreter drew prose in **topaz
8**, which lives in Kickstart ROM and on no floppy — a Workbench disk's `FONTS:`
drawer carries `topaz/11` and six proportional display faces and nothing else.
So the face cascade's Amiga rung can only be exercised against a ROM, and a real
Kickstart is copyrighted Commodore code that must never be committed here. Every
byte below is invented; nothing is copied from any Commodore image.

The three records are the three discriminators, matching `mk_sysfont_hfs.py`'s
shape on the Macintosh side:

  1. `topaz/8`  — 8x8 fixed, the RIGHT name at the RIGHT size. At the Amiga's
     system-face scale of (1, 2) this is exactly the 8x16 cell the machine
     declares, so the cascade must admit it.
  2. `topaz/9`  — 10x9 fixed, the right name at the WRONG size: 9 rows doubled is
     18 against a 16-row cell, so the cascade must decline it. (Kickstart 1.2
     really does carry a second topaz of this geometry.)
  3. `ruby/8`   — 8x8 fixed, byte-identical geometry to (1) under ANOTHER NAME.
     It passes every size and fitness test there is, so the only thing that can
     keep a Workbench display face out of an Amiga game is the machine's own
     `V6SystemFace::AmigaDrawer("topaz")`. Making the decoy *fixed* rather than
     proportional (real `ruby` is proportional) is deliberate: it removes the
     other reasons it might be refused and leaves only the name.

The image is also a shape test for the FINDER. `topaz/8` puts its name pointer
in one slot of the 20-byte `tf_Message` preamble and `topaz/9` in another,
because a ROM image does not initialise those link fields and `blorb` looks for
the name rather than indexing a fixed offset.

Glyph bitmaps are trivially reproducible so a test can assert on them without a
table: row `y` of the glyph for code `c` is the byte `(c + y) & 0xFF`.
"""

import os
import struct

ROM_LEN = 256 * 1024          # a Kickstart 1.2/1.3-sized image …
BASE = 0x0100_0000 - ROM_LEN  # … which maps at $FC0000, ending at $1000000.

FPF_ROMFONT = 0x01
FPF_DESIGNED = 0x40

LO, HI = 32, 127              # printable ASCII, as the real ROM faces cover.
NGLYPHS = HI - LO + 1

rom = bytearray(ROM_LEN)


def put(off, data):
    rom[off:off + len(data)] = data


# Every Kickstart opens `JMP <somewhere in ROM>`; blorb identifies the image by
# that instruction plus the length, and refuses to scan anything else.
put(0, struct.pack(">HHI", 0x1111, 0x4EF9, BASE + 0x00D2))

# ── the name strings ────────────────────────────────────────────────────────
NAMES = {"topaz": 0x0100, "ruby": 0x0120}
for stem, off in NAMES.items():
    put(off, (stem + ".font").encode("ascii") + b"\0")


def strike(off, xsize, ysize):
    """Write one font's strike and `tf_CharLoc`, returning (chardata, modulo,
    charloc, next free offset). All glyphs are `xsize` bits wide, side by side."""
    modulo = (NGLYPHS * xsize + 7) // 8
    chardata = off
    for y in range(ysize):
        row = bytearray(modulo)
        for i in range(NGLYPHS):
            byte = (LO + i + y) & 0xFF
            for bit in range(xsize):
                # The glyph's own byte, MSB-leftmost, repeated across a glyph
                # wider than eight bits.
                if byte & (0x80 >> (bit % 8)):
                    col = i * xsize + bit
                    row[col // 8] |= 0x80 >> (col % 8)
        put(chardata + y * modulo, bytes(row))
    charloc = chardata + ysize * modulo
    for i in range(NGLYPHS):
        put(charloc + i * 4, struct.pack(">HH", i * xsize, xsize))
    return chardata, modulo, charloc, charloc + NGLYPHS * 4


def text_font(off, name_slot, name_off, xsize, ysize, chardata, modulo, charloc):
    """One 52-byte `struct TextFont`, with its name pointer in `name_slot` of the
    uninitialised 20-byte `tf_Message` preamble."""
    rec = bytearray(52)
    struct.pack_into(">I", rec, name_slot, BASE + name_off)
    struct.pack_into(">H", rec, 20, ysize)                       # tf_YSize
    rec[22] = 0                                                  # tf_Style
    rec[23] = FPF_ROMFONT | FPF_DESIGNED                         # tf_Flags
    struct.pack_into(">H", rec, 24, xsize)                       # tf_XSize
    struct.pack_into(">H", rec, 26, 6)                           # tf_Baseline
    struct.pack_into(">H", rec, 28, 1)                           # tf_BoldSmear
    struct.pack_into(">H", rec, 30, 0)                           # tf_Accessors
    rec[32], rec[33] = LO, HI                                    # tf_Lo/HiChar
    struct.pack_into(">I", rec, 34, BASE + chardata)             # tf_CharData
    struct.pack_into(">H", rec, 38, modulo)                      # tf_Modulo
    struct.pack_into(">I", rec, 40, BASE + charloc)              # tf_CharLoc
    # tf_CharSpace / tf_CharKern stay NULL, which is how a fixed-pitch face is
    # stored: the advance is tf_XSize.
    put(off, bytes(rec))


free = 0x1000
plan = [
    # (record offset, name-pointer slot, stem, xsize, ysize)
    (0x0200, 14, "topaz", 8, 8),
    (0x0240, 10, "topaz", 10, 9),
    (0x0280, 14, "ruby", 8, 8),
]
for rec_off, slot, stem, xsize, ysize in plan:
    chardata, modulo, charloc, free = strike(free, xsize, ysize)
    text_font(rec_off, slot, NAMES[stem], xsize, ysize, chardata, modulo, charloc)

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "kickfont.rom")
with open(out, "wb") as f:
    f.write(bytes(rom))
print(f"wrote {out} ({len(rom)} bytes, base {BASE:#x})")
