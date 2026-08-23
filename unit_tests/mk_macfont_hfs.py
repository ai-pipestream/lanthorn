#!/usr/bin/env python3
"""Build `macfont.hfs`: a minimal Macintosh volume carrying a bitmap FONT.

This is the *generator*, committed beside the fixture so the fixture can be
regenerated and — more to the point — so it can be AUDITED. It is written from
the published format descriptions and deliberately shares no code with the Rust
readers it exists to test:

  * HFS volume, MDB and catalog file  — Inside Macintosh: Files, chapter 2
    ("Data Organization on Volumes", "File Manager Data Structures"): the Master
    Directory Block at logical block 2, the B*-tree node descriptor, the catalog
    key/directory/file records.
  * Resource fork                     — Inside Macintosh: More Macintosh
    Toolbox, "Resource Manager", "Resource File Format": the 16-byte header, the
    resource data area, the resource map with its type list, reference lists and
    name list. NOTE the classic trap: every count in the map is stored ONE LESS
    than the truth.
  * FONT resource                     — Inside Macintosh: Text, "Font Manager",
    the `FontRec` layout: 26-byte header, then the strike, then the location
    table, then the offset/width table.

Regenerate with:

    python3 unit_tests/mk_macfont_hfs.py unit_tests/macfont.hfs

The output is deterministic — no timestamps are written — so a regeneration
that changes a byte means the generator changed.

WHY THIS EXISTS. `crates/blorb`'s own HFS tests build volumes with an in-test
builder, which is a mirror: a writer and a reader developed together agree with
each other whether or not either agrees with HFS. That is tolerable for the
data-fork paths those tests cover and it is NOT tolerable for the resource-fork
path, which no test covered at all before this fixture. Everything here is laid
out from the spec above and every expected value in
`crates/blorb/src/mac_font.rs`'s tests is written out by hand.
"""

import sys

BLOCK = 512

# ---------------------------------------------------------------------------
# Little helpers. Big-endian throughout; the Macintosh is a 68000.
# ---------------------------------------------------------------------------


def be16(v):
    return int(v).to_bytes(2, "big", signed=(v < 0))


def be32(v):
    return int(v).to_bytes(4, "big", signed=(v < 0))


def pstr(s):
    """A Pascal string: a length byte, then the bytes."""
    b = s.encode("ascii")
    assert len(b) < 256
    return bytes([len(b)]) + b


def put(buf, at, data):
    buf[at : at + len(data)] = data


# ---------------------------------------------------------------------------
# The FONT resource (Inside Macintosh: Text, `FontRec`).
#
#   0  fontType      2   1  firstChar     2   2  lastChar      2
#   6  widMax        2   8  kernMax       2  10  nDescent      2
#  12  fRectWidth    2  14  fRectHeight   2  16  owTLoc        2
#  18  ascent        2  20  descent       2  22  leading       2
#  24  rowWords      2  26  bitImage    ...
#
# `owTLoc` is the offset to the offset/width table measured in WORDS from the
# owTLoc field itself, i.e. from offset 16.
#
# Behind the strike sits the location table: `lastChar - firstChar + 3` words.
# Glyph n occupies strike columns loc[n]..loc[n+1], so a glyph's image width is
# the difference and the last entry terminates. Behind that the offset/width
# table, one byte pair per entry: the left side bearing (added to kernMax) and
# the advance width, or 0xFFFF for a character the font does not define.
#
# The two extra entries are the missing-character glyph and the terminator.
# ---------------------------------------------------------------------------

# Glyph art, five columns wide. `#` is ink. Rows are top to bottom.
# Written out in full so the fixture can be read as a picture rather than as a
# hex dump, and so the expected bytes in the Rust test can be checked by eye.

BLANK5 = "....."

FONT_524_GLYPHS = {
    # 'A' — advance 7, image 5 wide.
    0x41: [BLANK5] * 3 + [
        ".###.",
        "#...#",
        "#...#",
        "#...#",
        "#####",
        "#...#",
        "#...#",
        "#...#",
        "#...#",
    ] + [BLANK5] * 3,
    # 'B'
    0x42: [BLANK5] * 3 + [
        "####.",
        "#...#",
        "#...#",
        "#...#",
        "####.",
        "#...#",
        "#...#",
        "#...#",
        "####.",
    ] + [BLANK5] * 3,
    # 'C'
    0x43: [BLANK5] * 3 + [
        ".###.",
        "#...#",
        "#....",
        "#....",
        "#....",
        "#....",
        "#....",
        "#...#",
        ".###.",
    ] + [BLANK5] * 3,
    # 'D' — deliberately EMPTY, with a NARROWER advance than its neighbours.
    # A space is stored exactly like this: zero image width, non-zero advance.
    # It is here as the discriminator for `BitmapFont::measure_proportional`,
    # which must ignore blank glyphs — a reader that counted this one would
    # call a fixed-pitch font proportional.
    0x44: None,
}

FONT_1033_GLYPHS = {
    0x30: [BLANK5] * 2 + [
        ".###.",
        "#...#",
        "#...#",
        "#...#",
        "#...#",
        "#...#",
        "#...#",
        ".###.",
    ] + [BLANK5] * 2,
    0x31: [BLANK5] * 2 + [
        "..#..",
        ".##..",
        "..#..",
        "..#..",
        "..#..",
        "..#..",
        "..#..",
        ".###.",
    ] + [BLANK5] * 2,
}

# The missing-character glyph: a hollow box, which is what the Macintosh drew
# for an undefined code. It lives in the strike and in both tables, and a
# correct reader drops it from the glyph list.
def missing_box(height, form_rows, top_pad):
    rows = ["#####"] + ["#...#"] * (form_rows - 2) + ["#####"]
    return [BLANK5] * top_pad + rows + [BLANK5] * (height - top_pad - form_rows)


def build_font(first, last, glyphs, missing, rect_w, rect_h, ascent, descent,
               leading, wid_max, kern_max, advances, bearings):
    """Assemble one FONT resource.

    `glyphs` maps a character code to its rows (or None for an empty image),
    `advances`/`bearings` map a code to its offset/width table pair. The
    missing-character glyph is appended after the last character, as the format
    requires.
    """
    n_chars = last - first + 1
    # One image per character, plus the missing-character glyph.
    images = [glyphs[first + i] for i in range(n_chars)] + [missing]

    # The location table: where each image starts in the strike, and one final
    # entry terminating the last. `n_chars + 2` entries in all.
    loc = [0]
    for img in images:
        width = 0 if img is None else len(img[0])
        loc.append(loc[-1] + width)
    assert len(loc) == n_chars + 2

    strike_width = loc[-1]
    row_words = (strike_width + 15) // 16
    row_bits = row_words * 16

    # The strike: every glyph side by side in ONE wide bitmap, `rowWords` words
    # per row, `fRectHeight` rows. Bit 15 of the first word is column 0.
    strike = bytearray(row_words * 2 * rect_h)
    for gi, img in enumerate(images):
        if img is None:
            continue
        assert len(img) == rect_h, f"glyph {gi} has {len(img)} rows, want {rect_h}"
        for y, row in enumerate(img):
            for x, c in enumerate(row):
                if c != "#":
                    continue
                bit = loc[gi] + x
                assert bit < row_bits
                byte = y * row_words * 2 + bit // 8
                strike[byte] |= 0x80 >> (bit % 8)

    loc_table = b"".join(be16(v) for v in loc)
    ow_table = bytearray()
    for i in range(n_chars + 1):
        if i < n_chars:
            code = first + i
            ow_table += bytes([bearings[code], advances[code]])
        else:
            # The missing-character glyph carries a real pair; the terminator
            # after it is 0xFFFF.
            ow_table += bytes([bearings["missing"], advances["missing"]])
    ow_table += b"\xff\xff"
    assert len(ow_table) == 2 * (n_chars + 2)

    # owTLoc counts words from offset 16 to the start of the offset/width
    # table, which follows the header, the strike and the location table.
    ow_at = 26 + len(strike) + len(loc_table)
    assert (ow_at - 16) % 2 == 0
    ow_loc = (ow_at - 16) // 2

    header = (
        be16(0x9000)      # fontType: a bitmap font, no image height table
        + be16(first)
        + be16(last)
        + be16(wid_max)
        + be16(kern_max)  # signed
        + be16(0)         # nDescent
        + be16(rect_w)
        + be16(rect_h)
        + be16(ow_loc)
        + be16(ascent)
        + be16(descent)
        + be16(leading)
        + be16(row_words)
    )
    assert len(header) == 26
    return bytes(header) + bytes(strike) + loc_table + bytes(ow_table)


# FONT 524 — family 4 at 12pt, the id an Infocom Macintosh v6 release uses for
# its body face. 7x15 with the baseline 12 rows down, which is the Macintosh
# Version 6 cell (`mac/xzip.lst`: colWidth := 7; lineHeight := 15).
#
# kernMax is -1 and every offset byte is 2, so the left side bearing is 1: a
# reader that ignores kernMax puts every glyph one column too far right, and
# every expected row below would be wrong. That is the point of choosing it.
FONT_524 = build_font(
    first=0x41,
    last=0x44,
    glyphs=FONT_524_GLYPHS,
    missing=missing_box(15, 9, 3),
    rect_w=7,
    rect_h=15,
    ascent=12,
    descent=3,
    leading=0,
    wid_max=7,
    kern_max=-1,
    advances={0x41: 7, 0x42: 7, 0x43: 7, 0x44: 3, "missing": 7},
    bearings={0x41: 2, 0x42: 2, 0x43: 2, 0x44: 1, "missing": 2},
)

# FONT 1033 — family 8 at 9pt, the id a release uses for its font-3 graphics
# set. Here it is a second, SHORTER face, so that `mac_font::from_fork`'s
# "tallest wins" rule has something to choose between. kernMax 0, offset 1, so
# the bearing is 1 again.
FONT_1033 = build_font(
    first=0x30,
    last=0x31,
    glyphs=FONT_1033_GLYPHS,
    missing=missing_box(12, 8, 2),
    rect_w=7,
    rect_h=12,
    ascent=9,
    descent=3,
    leading=0,
    wid_max=6,
    kern_max=0,
    advances={0x30: 6, 0x31: 6, "missing": 6},
    bearings={0x30: 1, 0x31: 1, "missing": 1},
)


# ---------------------------------------------------------------------------
# The resource fork (Inside Macintosh: More Macintosh Toolbox).
#
# Header, 16 bytes:  data offset, map offset, data length, map length.
# Then 240 reserved bytes, so the data area conventionally begins at 256.
# Each resource in the data area is a 4-byte length followed by its bytes.
#
# The map: a 16-byte copy of the header, 4 bytes of next-map handle, 2 of file
# reference number, 2 of attributes, then the offset to the type list and the
# offset to the name list, both from the START OF THE MAP.
#
# The type list opens with (number of types - 1). Each 8-byte entry is a
# four-character type, (number of resources of that type - 1), and the offset
# to that type's reference list FROM THE START OF THE TYPE LIST.
#
# A reference list entry is 12 bytes: resource id, offset into the name list
# (0xFFFF for none), one attribute byte, a THREE-byte offset into the data
# area, and four bytes of handle.
# ---------------------------------------------------------------------------

RESOURCES = [
    # (type, id, name, bytes)
    (b"FONT", 524, "Lanthorn 12", FONT_524),
    (b"FONT", 1033, "Lanthorn 9", FONT_1033),
]


def build_fork():
    data_off = 256

    data = bytearray()
    data_offsets = []
    for _, _, _, payload in RESOURCES:
        data_offsets.append(len(data))
        data += be32(len(payload)) + payload

    names = bytearray()
    name_offsets = []
    for _, _, name, _ in RESOURCES:
        name_offsets.append(len(names))
        names += pstr(name)

    # One type, `FONT`, holding both resources.
    type_list_off = 28          # straight after the 28-byte map preamble
    n_types = 1
    ref_list_off = 2 + 8 * n_types   # from the start of the type list
    name_list_off = type_list_off + ref_list_off + 12 * len(RESOURCES)

    type_list = be16(n_types - 1)
    type_list += b"FONT" + be16(len(RESOURCES) - 1) + be16(ref_list_off)

    refs = bytearray()
    for i, (_, rid, _, _) in enumerate(RESOURCES):
        off = data_offsets[i]
        refs += be16(rid)
        refs += be16(name_offsets[i])
        refs += bytes([0])                                    # attributes
        refs += bytes([(off >> 16) & 0xFF, (off >> 8) & 0xFF, off & 0xFF])
        refs += be32(0)                                       # handle

    map_body = (
        be32(0)                       # next resource map handle
        + be16(0)                     # file reference number
        + be16(0)                     # fork attributes
        + be16(type_list_off)
        + be16(name_list_off)
        + type_list
        + bytes(refs)
        + bytes(names)
    )
    map_len = 16 + len(map_body)
    map_off = data_off + len(data)

    header = be32(data_off) + be32(map_off) + be32(len(data)) + be32(map_len)
    assert len(header) == 16

    fork = bytearray(data_off)
    put(fork, 0, header)
    fork += data
    fork += header               # the map opens with a copy of the header
    fork += map_body
    assert len(fork) == map_off + map_len
    return bytes(fork)


RESOURCE_FORK = build_fork()


# ---------------------------------------------------------------------------
# A minimal but structurally honest Version 6 story header, so the volume holds
# something a story-finder can identify by content and `from_volume_beside` has
# a story to pair the application with. It is 512 bytes of nothing in
# particular; no Infocom code is involved.
# ---------------------------------------------------------------------------


def story():
    b = bytearray(BLOCK)
    b[0] = 6                       # Version 6
    put(b, 0x04, be16(0x0400))     # base of high memory
    put(b, 0x08, be16(0x0300))     # dictionary
    put(b, 0x0A, be16(0x0100))     # object table
    put(b, 0x0C, be16(0x0200))     # global variables
    put(b, 0x0E, be16(0x0280))     # base of static memory
    put(b, 0x1A, be16(BLOCK // 8))  # file length, in v6 units of 8 bytes
    put(b, 0x12, b"000000")        # serial: not a date, not a release
    return bytes(b)


STORY = story()


# ---------------------------------------------------------------------------
# The HFS volume (Inside Macintosh: Files).
#
# Logical blocks 0-1 are the boot blocks, block 2 is the Master Directory
# Block, block 3 is the volume bitmap. `drAlBlSt` says where the allocation
# blocks begin, counted in 512-byte logical blocks.
#
# Allocation blocks here are one logical block each, which is what an 800K
# floppy uses, and are handed out:
#
#   0-1  the catalog B*-tree (a header node and one leaf)
#   2-3  the application's RESOURCE fork
#   4    the story's data fork
# ---------------------------------------------------------------------------

ALLOC_START = 4          # drAlBlSt
ALLOC_COUNT = 60         # drNmAlBlks
ALLOC_SIZE = BLOCK       # drAlBlkSiz
VOLUME_NAME = "Lanthorn Test"

ROOT_CNID = 2
NODE_SIZE = BLOCK
NODE_HEADER = 14

CDR_DIR = 1
CDR_FILE = 2


def catalog_key(parent, name):
    """ckrKeyLen, ckrResrv1, ckrParID, ckrCName — padded to an even length.

    ckrKeyLen counts the key's bytes NOT including itself.
    """
    body = bytes([0]) + be32(parent) + pstr(name)
    key = bytes([len(body)]) + body
    if len(key) % 2:
        key += b"\x00"
    return key


def dir_record(cnid, valence):
    """A catalog directory record, 70 bytes."""
    d = bytearray(70)
    d[0] = CDR_DIR
    put(d, 2, be16(0))            # dirFlags
    put(d, 4, be16(valence))      # dirVal
    put(d, 6, be32(cnid))         # dirDirID
    return bytes(d)


def file_record(cnid, ftype, creator, data_len, data_extents,
                rsrc_len, rsrc_extents):
    """A catalog file record, 102 bytes.

    Field offsets, from Inside Macintosh: Files:
       0 cdrType   2 filFlags/filTyp   4 filUsrWds(16)   20 filFlNum
      24 filStBlk 26 filLgLen 30 filPyLen
      34 filRStBlk 36 filRLgLen 40 filRPyLen
      44 filCrDat 48 filMdDat 52 filBkDat  56 filFndrInfo(16)
      72 filClpSize  74 filExtRec(12)  86 filRExtRec(12)  98 filResrv
    """
    d = bytearray(102)
    d[0] = CDR_FILE
    put(d, 4, ftype)                   # filUsrWds.fdType
    put(d, 8, creator)                 # filUsrWds.fdCreator
    put(d, 20, be32(cnid))
    put(d, 24, be16(data_extents[0][0] if data_extents else 0))
    put(d, 26, be32(data_len))         # filLgLen
    put(d, 30, be32(round_up(data_len)))
    put(d, 34, be16(rsrc_extents[0][0] if rsrc_extents else 0))
    put(d, 36, be32(rsrc_len))         # filRLgLen
    put(d, 40, be32(round_up(rsrc_len)))
    put(d, 74, extent_record(data_extents))
    put(d, 86, extent_record(rsrc_extents))
    return bytes(d)


def round_up(n):
    return ((n + ALLOC_SIZE - 1) // ALLOC_SIZE) * ALLOC_SIZE


def extent_record(extents):
    """Three (first allocation block, block count) pairs, zero-padded."""
    out = bytearray(12)
    for i, (start, count) in enumerate(extents[:3]):
        put(out, i * 4, be16(start))
        put(out, i * 4 + 2, be16(count))
    return bytes(out)


def btree(records):
    """A header node naming one leaf, then that leaf holding `records`.

    The node descriptor is 14 bytes — ndFLink, ndBLink, ndType, ndNHeight,
    ndNRecs, ndResrv2 — records grow from offset 14, and their offsets grow
    BACKWARDS from the end of the node, one per record plus one more marking
    where the free space starts.

    The header node carries three records: the 106-byte B*-tree header record,
    a 128-byte reserved record, and the node bitmap filling the rest.
    """
    header = bytearray(NODE_SIZE)
    header[8] = 1          # ndType: header node
    header[9] = 0          # ndNHeight
    put(header, 10, be16(3))

    bth = bytearray(106)
    put(bth, 0, be16(1))        # bthDepth
    put(bth, 2, be32(1))        # bthRoot: node 1
    put(bth, 6, be32(len(records)))   # bthNRecs
    put(bth, 10, be32(1))       # bthFNode: the first leaf
    put(bth, 14, be32(1))       # bthLNode: and the last
    put(bth, 18, be16(NODE_SIZE))     # bthNodeSize
    put(bth, 20, be16(37))      # bthKeyLen: a catalog key's maximum
    put(bth, 22, be32(2))       # bthNNodes
    put(bth, 26, be32(0))       # bthFree

    reserved = bytes(128)
    # The node bitmap: nodes 0 and 1 are in use.
    bitmap = bytearray(NODE_SIZE - NODE_HEADER - len(bth) - len(reserved) - 8)
    bitmap[0] = 0xC0

    at = NODE_HEADER
    offsets = []
    for rec in (bytes(bth), reserved, bytes(bitmap)):
        offsets.append(at)
        put(header, at, rec)
        at += len(rec)
    offsets.append(at)
    for i, off in enumerate(offsets):
        put(header, NODE_SIZE - 2 * (i + 1), be16(off))

    leaf = bytearray(NODE_SIZE)
    put(leaf, 0, be32(0))       # ndFLink: no next leaf
    leaf[8] = 0xFF              # ndType: leaf node (-1)
    leaf[9] = 1                 # ndNHeight
    put(leaf, 10, be16(len(records)))
    at = NODE_HEADER
    offsets = []
    for rec in records:
        offsets.append(at)
        put(leaf, at, rec)
        at += len(rec)
    offsets.append(at)
    assert at + 2 * len(offsets) <= NODE_SIZE, "the leaf does not fit in one node"
    for i, off in enumerate(offsets):
        put(leaf, NODE_SIZE - 2 * (i + 1), be16(off))

    return bytes(header) + bytes(leaf)


def build_volume():
    # Allocation blocks 0-1 hold the catalog; 2-3 the resource fork; 4 the
    # story. Records are in catalog key order: by parent CNID, then by name.
    folder_cnid = 16
    app_cnid = 17
    story_cnid = 18

    records = [
        catalog_key(ROOT_CNID, "Test Folder") + dir_record(folder_cnid, 2),
        catalog_key(folder_cnid, "Story.data")
        + file_record(story_cnid, b"INdf", b"LNTH",
                      len(STORY), [(4, 1)], 0, []),
        # The application. Its DATA fork is ZERO BYTES, which is how an Infocom
        # Macintosh release ships: everything is in the resource fork, and a
        # reader that can only reach data forks sees an empty file here.
        catalog_key(folder_cnid, "TestApp")
        + file_record(app_cnid, b"APPL", b"LNTH",
                      0, [], len(RESOURCE_FORK), [(2, 2)]),
    ]
    catalog = btree(records)
    assert len(catalog) == 2 * NODE_SIZE

    volume = bytearray((ALLOC_START + ALLOC_COUNT) * BLOCK)

    def alloc(n):
        return (ALLOC_START + n) * ALLOC_SIZE

    put(volume, alloc(0), catalog)
    put(volume, alloc(2), RESOURCE_FORK)
    put(volume, alloc(4), STORY)

    # The Master Directory Block. Field offsets from Inside Macintosh: Files —
    # note drNxtCNID is at 30 and drFreeBks at 34, which is the one place the
    # field list is easy to get wrong by two bytes and still produce a volume
    # that mounts.
    #
    #   0 drSigWord   2 drCrDate    6 drLsMod   10 drAtrb    12 drNmFls
    #  14 drVBMSt    16 drAllocPtr 18 drNmAlBlks 20 drAlBlkSiz 24 drClpSiz
    #  28 drAlBlSt   30 drNxtCNID  34 drFreeBks 36 drVN(28)  64 drVolBkUp
    #  68 drVSeqNum  70 drWrCnt    74 drXTClpSiz 78 drCTClpSiz 82 drNmRtDirs
    #  84 drFilCnt   88 drDirCnt   92 drFndrInfo(32) 124 drVCSize
    # 126 drVBMCSize 128 drCtlCSize 130 drXTFlSize 134 drXTExtRec(12)
    # 146 drCTFlSize 150 drCTExtRec(12)                        = 162 bytes
    mdb = 2 * BLOCK
    put(volume, mdb + 0, be16(0x4244))            # drSigWord: 'BD'
    put(volume, mdb + 10, be16(0x0100))           # drAtrb: cleanly unmounted
    put(volume, mdb + 12, be16(1))                # drNmFls: files in the root
    put(volume, mdb + 14, be16(3))                # drVBMSt: the volume bitmap
    put(volume, mdb + 18, be16(ALLOC_COUNT))      # drNmAlBlks
    put(volume, mdb + 20, be32(ALLOC_SIZE))       # drAlBlkSiz
    put(volume, mdb + 24, be32(ALLOC_SIZE))       # drClpSiz
    put(volume, mdb + 28, be16(ALLOC_START))      # drAlBlSt
    put(volume, mdb + 30, be32(19))               # drNxtCNID
    put(volume, mdb + 34, be16(ALLOC_COUNT - 5))  # drFreeBks
    put(volume, mdb + 36, pstr(VOLUME_NAME))      # drVN
    put(volume, mdb + 74, be32(ALLOC_SIZE))       # drXTClpSiz
    put(volume, mdb + 78, be32(ALLOC_SIZE))       # drCTClpSiz
    put(volume, mdb + 82, be16(1))                # drNmRtDirs
    put(volume, mdb + 84, be32(2))                # drFilCnt
    put(volume, mdb + 88, be32(1))                # drDirCnt
    put(volume, mdb + 130, be32(0))               # drXTFlSize: empty
    put(volume, mdb + 134, extent_record([]))     # drXTExtRec
    put(volume, mdb + 146, be32(len(catalog)))    # drCTFlSize
    put(volume, mdb + 150, extent_record([(0, 2)]))  # drCTExtRec

    # The volume bitmap in block 3: allocation blocks 0-4 are in use.
    volume[3 * BLOCK] = 0b11111000
    return bytes(volume)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "macfont.hfs"
    image = build_volume()
    with open(out, "wb") as f:
        f.write(image)
    print(f"{out}: {len(image)} bytes")
    print(f"  FONT  524: {len(FONT_524)} bytes, 7x15, ascent 12, 'A'-'D'")
    print(f"  FONT 1033: {len(FONT_1033)} bytes, 7x12, ascent 9, '0'-'1'")
    print(f"  resource fork: {len(RESOURCE_FORK)} bytes")


if __name__ == "__main__":
    main()
