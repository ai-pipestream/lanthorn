#!/usr/bin/env python3
"""Build the two volumes SQ-1037's cascade is tested against.

    sysfont.hfs   a Mac OS System startup disk — a FAMILY of proportional faces
    relfont.hfs   an Infocom Macintosh release — one FIXED-PITCH `FONT` 524

`macfont.hfs` is the older sibling and cannot play the release here: its `FONT`
524 carries a deliberately narrow `D` (SQ-0916's discriminator for
`measure_proportional`), so it reads as a TYPEFACE rather than as the cell, which
is the opposite of what the real resource does. `relfont.hfs` is the same
resource without that trap — every code it covers advances by exactly 7 — so
`native_font::fit` calls it `FaceFit::Cell` and it fills the fixed-pitch role the
Macintosh's real `FONT` 524 fills.


The System disk plays the part of the media a player supplies: an
operating-system volume carrying a FAMILY of proportional faces at several sizes,
which is what `MacOS_6.0.8_System_Startup.img` is and what no game disk ever is.

It exists so the cascade in `app::native_font::resolve` can be tested without
depending on what the person running the tests keeps in `~/.lanthorn/`. Three
resources, each of which the cascade must treat differently:

    FONT   12   family 0 (the system font), 15 rows   — right height, WRONG family
    FONT  394   family 3 (Geneva) at 10pt, 12 rows    — right family, WRONG height
    FONT  396   family 3 (Geneva) at 12pt, 15 rows    — the one the machine drew

A `FONT` id is `family * 128 + point size`, so family 3 owns 384..511. Those are
the same three discriminators the real System 6.0.8 disk carries — it lists
`FONT 12` at 14x15, `FONT 394` at 12x12 and `FONT 396` at 15x15 — reproduced
here at a size a fixture can hold.

Regenerate BOTH with:

    python3 unit_tests/mk_sysfont_hfs.py unit_tests

Deterministic, like its sibling: a regeneration that changes a byte means the
generator changed. Every primitive below comes from `mk_macfont_hfs.py`, whose
header documents the formats and the sources they were written from.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mk_macfont_hfs as base  # noqa: E402

BLANK9 = "........."


def bars(width, height, top_pad, form_rows):
    """A glyph that is `width` columns of solid ink, `form_rows` tall.

    Legible in a hex dump and, more to the point, a DIFFERENT width per code —
    which is what makes the face proportional and therefore a `FaceFit::Metric`
    one. The shape carries no meaning; the widths do.
    """
    row = "#" * width + "." * (9 - width)
    return [BLANK9] * top_pad + [row] * form_rows + [BLANK9] * (height - top_pad - form_rows)


# Four codes with four DIFFERENT advances. `native_font::fit` asks whether every
# printable character advances by one declared cell; varying widths answer no,
# which is the whole reason a system face is worth reading.
PROPORTIONAL_ADVANCES = {0x41: 4, 0x42: 6, 0x43: 8, 0x44: 9}
PROPORTIONAL_BEARINGS = {0x41: 1, 0x42: 1, 0x43: 1, 0x44: 1}


def proportional_font(height, ascent, form_rows, top_pad):
    """One face of the family, `height` rows tall."""
    glyphs = {
        code: bars(min(PROPORTIONAL_ADVANCES[code] - 1, 9), height, top_pad, form_rows)
        for code in PROPORTIONAL_ADVANCES
    }
    advances = dict(PROPORTIONAL_ADVANCES)
    advances["missing"] = 9
    bearings = dict(PROPORTIONAL_BEARINGS)
    bearings["missing"] = 1
    return base.build_font(
        first=0x41,
        last=0x44,
        glyphs=glyphs,
        missing=base.missing_box(height, form_rows, top_pad),
        rect_w=9,
        rect_h=height,
        ascent=ascent,
        descent=height - ascent,
        leading=0,
        wid_max=9,
        kern_max=0,
        advances=advances,
        bearings=bearings,
    )


# The three faces. Only 396 is both the right family and the right height.
FONT_12 = proportional_font(height=15, ascent=12, form_rows=9, top_pad=3)
FONT_394 = proportional_font(height=12, ascent=9, form_rows=8, top_pad=2)
FONT_396 = proportional_font(height=15, ascent=12, form_rows=9, top_pad=3)

SYSTEM_RESOURCES = [
    (b"FONT", 12, "System 12", FONT_12),
    (b"FONT", 394, "Geneva 10", FONT_394),
    (b"FONT", 396, "Geneva 12", FONT_396),
]


# ---------------------------------------------------------------------------
# The release volume: `FONT` 524 — family 4 (Monaco) at 12pt — drawn for the
# Macintosh Version 6 cell and advancing by it uniformly.
#
# `native_font::fit` asks whether EVERY printable character advances by one
# declared cell. The real resource answers yes (SQ-0916 measured its printable
# set as exactly {7}), so it is a `FaceFit::Cell` face: the machine's ZMONO
# alternate, blitted 1:1, and never the body face a proportional System face
# supplies.
# ---------------------------------------------------------------------------

MONO_ADVANCE = 7


def mono_glyph(width, height, top_pad, form_rows):
    row = "#" * width + "." * (5 - width)
    return [BLANK9[:5]] * top_pad + [row] * form_rows + [BLANK9[:5]] * (
        height - top_pad - form_rows
    )


FONT_524 = base.build_font(
    first=0x41,
    last=0x44,
    # Four different SHAPES at one advance: the point is that the pen never
    # varies, not that the ink does not.
    glyphs={
        0x41: mono_glyph(5, 15, 3, 9),
        0x42: mono_glyph(4, 15, 3, 9),
        0x43: mono_glyph(3, 15, 3, 9),
        0x44: mono_glyph(2, 15, 3, 9),
    },
    missing=base.missing_box(15, 9, 3),
    rect_w=7,
    rect_h=15,
    ascent=12,
    descent=3,
    leading=0,
    wid_max=7,
    kern_max=0,
    advances={c: MONO_ADVANCE for c in (0x41, 0x42, 0x43, 0x44, "missing")},
    bearings={c: 1 for c in (0x41, 0x42, 0x43, 0x44, "missing")},
)

RELEASE_RESOURCES = [(b"FONT", 524, "Lanthorn Mono 12", FONT_524)]


def emit(out, resources, volume_name):
    base.RESOURCES = resources
    base.RESOURCE_FORK = base.build_fork()
    base.VOLUME_NAME = volume_name
    image = base.build_volume()
    with open(out, "wb") as f:
        f.write(image)
    print(f"{out}: {len(image)} bytes, volume {volume_name!r}")
    for typ, ident, name, payload in resources:
        family, size = divmod(ident, 128)
        print(
            f"  {typ.decode()} {ident:>4}: {len(payload)} bytes, family {family} at {size}pt, "
            f"{name}"
        )


def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else "."
    emit(os.path.join(out_dir, "sysfont.hfs"), SYSTEM_RESOURCES, "Lanthorn System")
    emit(os.path.join(out_dir, "relfont.hfs"), RELEASE_RESOURCES, "Lanthorn Release")


if __name__ == "__main__":
    main()
