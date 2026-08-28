# README, staged

**What this is.** README prose for features that are in `main` but **not in a
released build**. The README describes the *released* build — someone reads it
having just downloaded the latest release, and a README that describes `main`
tells them to type flags their binary rejects. So the prose is written when the
work lands, parked here, and applied when a version is cut.

**What this is not.** A copy of the README. Only deltas live here, so this file
cannot drift out of sync with anything — it is not a second source of truth
about the shipped build, and there is nothing here to keep updated. A staged
*whole* README would become exactly that, and rot.

**How to use it.**

- **Landing a user-visible feature?** Write the README prose now, in README
  voice, and add an entry below. The changelog entry goes in `CHANGELOG.md`
  under `## Unreleased`; that is a different genre for a different reader —
  the changelog tells an existing user what changed, the README tells a
  newcomer what lanthorn does.
- **Every entry names its destination**: which section, and what it replaces or
  follows. Applying this file at release should be mechanical. An entry that
  says only "mention the new flag" is a re-reading exercise for whoever cuts the
  release, and half of those get missed.
- **Cutting a release?** Apply every entry, then empty this file back to this
  header. Draining it is a release task, like removing a "coming next release"
  caveat.

Entries are grouped by README section, in the order those sections appear.

---

## Quick start

**Replaces** the flag list in the paragraph ending *"`lanthorn --help` has the
flags; the ones people reach for are …"*.

> `lanthorn --help` has the flags; the ones people reach for are `--sound off`,
> `--images off` and `--image-protocol`.

**Follows** the same paragraph — a URL is a launching shape, so it belongs
beside the directory and the disk image:

> ```bash
> lanthorn https://ifarchive.org/if-archive/games/zcode/curses.z5
> ```
>
> A web address works anywhere a path does. lanthorn fetches it, opens it like
> any other file — story, Blorb, disk image, zip — and then offers to keep it in
> your library so the next launch finds it without fetching again.

---

## Try these first

**Add** to the **In the story picker** table, after the `/` row:

> | **Shift+U** | Downloads a story straight into your library from a web
> address you paste. |

---

## What it does

**Follows** the *"A real terminal UI"* bullet, as the last bullet in the list —
it is a thing lanthorn does for the player rather than a surface it draws:

> - **A light held up while you play** — Lanthorn's Guiding Light offers the
>   words this story's parser knows, the noun you were reaching for, and a
>   caution before a move that cannot be taken back. It says so once, then marks
>   every later line with one glyph in the margin — never in the story's own
>   voice, and never a spoiler. `--guidance off`, `/set-guidance`, or the
>   settings screen turns it off.
>   → [customization](docs/features/customization.md)

---

## Play the original disks

**Add** to the media table:

> | Commodore 1541, GCR bitstream | `.g64` | Commodore 128 (7) |

A `.g64` is the raw bitstream a 1541's head reads rather than decoded sectors —
the format archives use when a disk's protection lives in how the bits are laid
down. lanthorn decodes it to sectors and plays it, and the protection is not a
problem because Infocom's lived in the loader, which lanthorn never runs.

**Replaces** whatever the README says about zips (currently in the section on
what a zip carries):

> **A zip is opened like a volume.** What is inside is identified by its
> *contents*, not its name, so a zip carries anything lanthorn runs — every
> Z-machine version including graphical v6, Glulx, Scott Adams, Blorb
> containers — and a Blorb or a hints file packed beside the story is found and
> used. A zip holding two games plays the first one.
>
> **And a downloaded zip of release floppies** is offered to your library: say
> yes and the whole release is unpacked where the picker will find it and
> launched; say no and lanthorn tells you why rather than failing obscurely.
> Only the disk images come out of the archive — never a readme, a cover or
> anything else that happened to be in it.

---

## Terminal support

**Replaces** *"or turn images off with `--no-images`"*:

> or turn images off with `--images off`.

---

## Configuration

**Follows** the paragraph beginning *"lanthorn reads
`~/.lanthorn/config.toml`…"*:

> An **exported transcript** is not quite what is on screen: lanthorn's own
> guidance is marked in the margin while you play, and written out with the word
> `Lanthorn:` in front of it, because a file has no margin and no colour.

---

## Anywhere the old flag spellings appear

The whole `--no-x` surface is gone across all four front-ends. **Grep the README
for `--no-` before cutting the release** — the two occurrences known today are
listed above, but any written between now and then need the same treatment.

| was | is |
|---|---|
| `--no-sound` | `--sound on\|off` |
| `--no-images` | `--images on\|off` |
| `--no-accel` | `--accel on\|off` |
| `--no-game-colours` | `--game-colours on\|off` |
| `--no-aux` | `--aux on\|off` |
| `--no-timed-input` | `--timed-input on\|off` |
| `--no-more` / `--no-page` | `--pager on\|off` |
| `--system-colours` | `--colour machine` |
| `--no-status` | removed — `--story-only` was already its name |

Worth a line somewhere if there is a natural home: `--colour
terminal|theme|machine` is new, and picks which of the three sources the story's
default page and ink come from.
