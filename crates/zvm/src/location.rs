// Mapper-facing API — current player location and read-only object tree.
//
// This module provides two signals that the future automapper consumes:
//   1. `current_location` — the object representing where the player is now.
//   2. `object_tree_view` — a read-only enumeration of all objects.
//
// # Location heuristic
//
// The Z-machine specification (ZMSD) has no standard mechanism for identifying
// the player's current location.  We use version-dependent heuristics:
//
// ## v3 (status-line games, ZMSD §8.2.2.1)
// The interpreter status line reads the current room from **global variable 0**
// (variable number 0x10, the first global).  This is the object number of the
// current room.  We read that global; if it is nonzero and within the valid
// object-number range we return its snapshot.
//
// ## v4+ (no status line / Inform games)
// There is no guaranteed status-line global.  Many Inform games still store a
// location-ish object in global 0, so we try the same strategy.  This is a
// best-effort heuristic; the automapper's "unknown direction" mechanism handles
// the occasional wrong or missing value gracefully.
//
// # Object-tree enumeration bounds
//
// The Z-machine does not store the object count explicitly.  We infer it from
// the layout: objects are stored in a compact array immediately after the
// property-defaults table; each object entry contains a pointer to its own
// property table.  The smallest property-table address found across all entries
// marks where the object entries array ends, because property tables are always
// placed after the object entries in well-formed story files.
//
// Concretely: iterate candidate objects starting from 1.  For each candidate,
// read the property-table pointer stored in its entry.  If that pointer is less
// than or equal to the start of the current candidate's own entry (meaning the
// pointer points back into the entry region itself), we have run past the end of
// the real object table.  We also stop if the pointer is zero.  A reasonable
// absolute cap of 2000 objects is applied to guard against malformed data.
//
// **Documented limitations:**
//   - The v4+ location is a best-effort guess; wrong answers are expected
//     occasionally and the automapper is designed to tolerate them.
//   - Object-count inference can be wrong for unusual story layouts (hand-crafted
//     or very old files where property tables are interleaved with entries).
//   - v8 and v7 stories use the same heuristic as v4+ for location.

use crate::cpu::exec::Machine;
use crate::objects::{entries_base, entry_size, get_parent, object_snapshot, prop_table_ptr_offset, short_name, ObjectSnapshot};
use crate::screen::{UpperWindow, V6_FONT_WIDTH};

/// Normalize for matching/hashing: trim, collapse whitespace, lowercase.
pub(crate) fn normalize_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Strip the posture suffix after the first comma, then trim.
fn clean_room_text(s: &str) -> String {
    s.split(',').next().unwrap_or(s).trim().to_string()
}

/// Map Unicode box-drawing (U+2500–U+257F) and block-element (U+2580–U+259F)
/// glyphs to a space for status-line PARSING only (never for display).
///
/// BeyondZork's VT220 mode frames the centered room title with half-block bars
/// (`▐`…`▌`, U+2590/U+258C); without this the leading bar reads as a bogus
/// left-justified "room name" and defeats the centered fallback. Ordinary room
/// names never contain these glyphs, so this is a no-op for every other game.
fn deframe(ch: char) -> char {
    if ('\u{2500}'..='\u{259F}').contains(&ch) {
        ' '
    } else {
        ch
    }
}

/// Extract a candidate room name from the v4+ status-line grid, or None.
///
/// Scans at most the first 2 active rows. Prefers a `Location:` label segment;
/// otherwise takes row 1's first segment (text before the first run of 2+
/// spaces, which separates the left-justified room name from the right-aligned
/// score/moves/time block). Strips a trailing posture suffix after a comma.
pub fn status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String> {
    let scan = active_rows.min(2).min(upper.rows);
    let row_text = |r: u16| -> String {
        let mut s = String::new();
        for c in 1..=upper.cols {
            s.push(deframe(upper.cell(r, c).ch));
        }
        s
    };

    // 1. Label form: any scanned row containing a "Location:" segment.
    for r in 1..=scan {
        let line = row_text(r);
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("location:") {
            let after = line[idx + "location:".len()..].trim_start();
            let value = after.split("  ").next().unwrap_or("").trim();
            let candidate = clean_room_text(value);
            if !candidate.is_empty() {
                return Some(candidate);
            }
        }
    }

    // 2. Common form: row 1's first segment (before the first 2+ space run).
    if scan >= 1 {
        let line = row_text(1);
        let first = line.split("  ").next().unwrap_or("").trim();
        let candidate = clean_room_text(first);
        if !candidate.is_empty() {
            return Some(candidate);
        }
    }

    None
}

/// Centered-title fallback for row 1 when the common form yields nothing.
///
/// Some v4+ games (e.g. BeyondZork) CENTER the room name in row 1 with leading
/// padding and put stats on row 2, so the left-justified first segment (text
/// before the first 2+-space run) is empty and `status_line_room_name` returns
/// `None`. This retries on the TRIMMED line, taking its first 2+-space segment.
///
/// It fires ONLY where the left-justified first segment is empty, so it never
/// changes left-justified behavior. A centered line can be a banner/title
/// rather than a room, so `detect_location` accepts this result only under the
/// strongest validation — the avatar's ancestor chain reaches a room of this
/// name (PlayerParent). A bare name match (StatusName) or unvalidated `NameOnly`
/// is NOT trusted here.
fn centered_status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String> {
    let scan = active_rows.min(2).min(upper.rows);
    if scan < 1 {
        return None;
    }
    let line: String = (1..=upper.cols).map(|c| deframe(upper.cell(1, c).ch)).collect();
    // Only a centered title: the left-justified first segment must be empty
    // (the line begins with 2+ spaces). Otherwise the common form handled it.
    if !line.split("  ").next().unwrap_or("").trim().is_empty() {
        return None;
    }
    let first = line.trim().split("  ").next().unwrap_or("").trim();
    let candidate = clean_room_text(first);
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

// ── v6 (graphical) status-band room extraction ─────────────────────────────────
//
// v6 games never populate the v4+ upper-window GRID; their status text is PAINT
// in the v6 window model (`machine.screen.v6` — each window's `texts` are runs
// with screen-absolute 1-based pixel x/y, ZMSD §8.8). The extractor below reads
// those paint runs and yields ordered room-name candidates for the SAME
// validation ladder the grid path uses.
//
// Grounded in the four real v6 titles at the 640×400 geometry (SQ-0479 re-trace;
// coords roughly double the old 320×200 values):
//   - Zork Zero: window 1 status band at y_coord=1. Room name is the LEFT run on
//     the top text row (y=11, x=71 → dx=70): "Banquet Hall" → "Scullery".
//     Score/Moves on the next row (y=27). The kingdom "Flatheadia" is a
//     RIGHT-anchored field (x=489, dx=488).
//   - Shogun: window 1 (x_coord=47), two rows. Row 1 (y=1): "Erasmus:" (the ship,
//     a LABELLED field — a ":" run abuts it, x=49 dx=2) + "SHOGUN" (a CENTERED
//     banner, x=296 dx=249) + Score (x=504 dx=457). Row 2 (y=17): "Bridge" (the
//     room, left, x=49 dx=2) + Moves. The avatar is NOT parented to the room
//     object here, so only StatusName (resolve_room_object) recovers "Bridge";
//     "Erasmus" and "SHOGUN" must be rejected.
//   - Arthur / Journey: no room-status text painted in the top band → no
//     candidates → None (correct; Journey is menu-driven, Arthur's boot intro
//     paints no status line).

/// Top of the v6 status band: runs whose absolute screen `y` is within the first
/// four native text rows (4 × `V6_FONT_HEIGHT` = 64px). Re-grounded on fresh
/// 640×400 traces (SQ-0479): the status text now paints at 16px row pitch — Zork0
/// room name y=11, Score/Moves y=27; Shogun room y=17, Score/banner y=1 — so the
/// band spans rows 0–1 (y ≤ 32) with headroom to 4 rows. Journey's menu (y≥305)
/// and any deeper content stay well outside.
const V6_STATUS_BAND_MAX_Y: u16 = 64;

/// A run is "left-anchored" (a classic room-name slot, eligible for the weaker
/// StatusName path) when it starts within this many pixels of its window's left
/// edge. Re-grounded on fresh 640×400 traces (SQ-0479): room names sit dx≤~70
/// (Zork0 "Banquet Hall"/"Scullery" x=71 in a x_coord=1 window → dx=70; Shogun
/// "Bridge" x=49 in a x_coord=47 window → dx=2); centered/right fields sit far
/// out (Shogun "SHOGUN" banner dx=249, Score dx=457; Zork0 "Flatheadia" dx=488).
/// 96px (12 cells) divides them with margin — above every room name, below every
/// banner/score.
const V6_LEFT_ANCHOR_MAX_DX: u16 = 96;

/// One v6 status-band candidate: the cleaned room text and whether it was
/// left-anchored. Left-anchored runs are tried for StatusName; centered/other
/// runs are PlayerParent-only (a centered run is usually a banner, e.g. "SHOGUN").
struct V6Candidate {
    name: String,
    left_anchored: bool,
}

/// True for a status FIELD that is never a room name: a score/moves/turns/time
/// label, or a run with no letters at all (bare numbers, ":", padding).
fn is_v6_stat_field(s: &str) -> bool {
    let n = normalize_name(s);
    if n.is_empty() {
        return true;
    }
    if ["score", "moves", "turns", "time"].iter().any(|k| n.contains(k)) {
        return true;
    }
    !n.chars().any(|c| c.is_alphabetic())
}

/// Ordered v6 status-band room candidates: left-anchored runs first (top rows
/// first), then centered/other runs. A pure read of the v6 paint model; empty
/// when the story is not v6 or paints no top-band text.
fn v6_status_candidates(machine: &Machine) -> Vec<V6Candidate> {
    let Some(v6) = machine.screen.v6.as_ref() else {
        return Vec::new();
    };
    use std::collections::BTreeMap;
    // Group top-band runs by row (absolute y), carrying each run's window left
    // edge so left-anchoring is measured relative to the window, not the screen.
    let mut rows: BTreeMap<u16, Vec<(&crate::screen::V6Text, u16)>> = BTreeMap::new();
    for w in v6.windows.iter() {
        for t in w.texts.iter() {
            if t.y <= V6_STATUS_BAND_MAX_Y && !t.text.trim().is_empty() {
                rows.entry(t.y).or_default().push((t, w.x_coord));
            }
        }
    }
    let fw = V6_FONT_WIDTH as i32;
    let mut left = Vec::new();
    let mut other = Vec::new();
    for (_y, mut runs) in rows {
        runs.sort_by_key(|(t, _)| t.x);
        for (i, (t, xc)) in runs.iter().enumerate() {
            let text = t.text.trim();
            if is_v6_stat_field(text) {
                continue;
            }
            // Label field (e.g. Shogun's "Erasmus:"): the next run on the row
            // abuts this one's right edge and begins with ':'. Labels are not
            // rooms — skip, mirroring the grid path's "Location:" handling.
            let is_label = runs.get(i + 1).is_some_and(|(nx, _)| {
                let right_edge = t.x as i32 + t.text.chars().count() as i32 * fw;
                nx.text.trim_start().starts_with(':') && (nx.x as i32 - right_edge).abs() <= fw
            });
            if is_label {
                continue;
            }
            let cand = clean_room_text(&text.chars().map(deframe).collect::<String>());
            if cand.is_empty() {
                continue;
            }
            let left_anchored = t.x.saturating_sub(*xc) <= V6_LEFT_ANCHOR_MAX_DX;
            if left_anchored {
                left.push(V6Candidate { name: cand, left_anchored: true });
            } else {
                other.push(V6Candidate { name: cand, left_anchored: false });
            }
        }
    }
    left.into_iter().chain(other).collect()
}

/// Ordered v6 status-band room-name candidates (left-anchored first). Pure read
/// of the v6 paint model; empty for non-v6 stories or top-band-less screens.
/// Public for introspection/tests; `detect_location` uses the richer internal
/// form that also tracks left-anchoring for the StatusName gate.
pub fn v6_status_room_candidates(machine: &Machine) -> Vec<String> {
    v6_status_candidates(machine).into_iter().map(|c| c.name).collect()
}

/// v6 location via the same validation ladder as the grid path, but sourced from
/// the v6 paint runs (`v6_status_candidates`).
///
/// 1. PlayerParent (strongest) across ALL candidates, left then centered: the
///    first candidate whose name is reached by some avatar's ancestor chain wins.
///    This is what makes Zork Zero work (its avatar sits directly in the room
///    object) and what would reject a banner even if it were centered.
/// 2. StatusName for LEFT-ANCHORED candidates only (mirrors the grid): resolve
///    the name to a real object, preferring the player_room_beside form. This is
///    the ONLY thing that recovers Shogun's "Bridge" (whose avatar is not
///    parented to the room). Centered runs are never StatusName'd — a centered
///    run is a banner far more often than a room.
/// 3. NameOnly is DROPPED for v6: with no backing object a v6 candidate is far
///    more likely a title/character-sheet/banner than a room, and there is no
///    grid-shaped left-justified discipline to lean on. Returning None on a
///    title/menu screen is the correct answer, so an object-less candidate yields
///    None rather than inventing a room.
fn detect_location_v6(machine: &Machine) -> Option<Location> {
    let cands = v6_status_candidates(machine);
    // 1. PlayerParent across all candidates.
    for cand in &cands {
        for player in player_candidates(machine) {
            if let Some(room) = nearest_matching_ancestor(machine, player, &cand.name) {
                return Some(Location::PlayerParent(room));
            }
        }
    }
    // 2. StatusName for left-anchored candidates only.
    for cand in cands.iter().filter(|c| c.left_anchored) {
        if let Some(shown) = resolve_room_object(machine, &cand.name) {
            if let Some(room) = player_room_beside(machine, &shown) {
                return Some(Location::PlayerParent(room));
            }
            return Some(Location::StatusName(shown));
        }
    }
    // 3. No object-backed candidate: do NOT invent a NameOnly room for v6.
    None
}

/// True if `short` names the room shown as `candidate`: equality, or `short` is
/// a leading prefix of `candidate` ending on a word boundary (next char
/// non-alphanumeric, or end of string). Both normalized; `short` non-empty.
pub fn status_name_matches(candidate: &str, short: &str) -> bool {
    let c = normalize_name(candidate);
    let s = normalize_name(short);
    if s.is_empty() {
        return false;
    }
    if c == s {
        return true;
    }
    match c.strip_prefix(&s) {
        Some(rest) => rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric()),
        None => false,
    }
}

/// The current player object: among all objects whose normalized short name is
/// a plausible avatar name (see `PLAYER_NAMES`), prefer the lowest-numbered
/// *situated* one — i.e. one with a non-zero parent, meaning it's actually
/// placed inside a room. Some games (e.g. Zork 1) also have a parentless
/// decorative "you" object (never placed anywhere, no children) that isn't the
/// real avatar; a real avatar always sits inside a room. If no candidate is
/// situated, fall back to the lowest-numbered candidate (preserves prior
/// behavior for games without a decorative object). None if no candidate exists.
pub fn find_player_object(machine: &Machine) -> Option<u16> {
    let cands = player_candidates(machine);
    cands
        .iter()
        .copied()
        .find(|&obj| get_parent(&machine.mem, obj) != 0)
        .or_else(|| cands.first().copied())
}

/// Object short-names that plausibly denote the player avatar, including ZIL's
/// "cretin"/"adventurer".
const PLAYER_NAMES: [&str; 7] = ["yourself", "you", "me", "myself", "self", "cretin", "adventurer"];

/// All objects whose short name plausibly denotes the player avatar, in ascending
/// object order. `detect_location` validates each against the status-line room.
fn player_candidates(machine: &Machine) -> Vec<u16> {
    let n = max_object_number(&machine.mem);
    (1..=n)
        .filter(|&obj| {
            let nm = normalize_name(&short_name(&machine.mem, obj));
            PLAYER_NAMES.contains(&nm.as_str())
        })
        .collect()
}

/// Nearest ancestor of `start` (exclusive) whose short name matches `name` via
/// `status_name_matches`. Depth-bounded (32) to tolerate cycles.
fn nearest_matching_ancestor(machine: &Machine, start: u16, name: &str) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let mut cur = get_parent(mem, start);
    for _ in 0..32 {
        if cur == 0 {
            break;
        }
        if status_name_matches(name, &short_name(mem, cur)) {
            return Some(object_snapshot(mem, cur));
        }
        cur = get_parent(mem, cur);
    }
    None
}

/// The player's real room when the status line has gone STALE — i.e. no avatar's ancestor chain
/// reaches the room the status line names, so the TEXT and the OBJECT TREE disagree (SQ-0358).
///
/// The tree is the game's own state; the status line is a rendering the game may simply not have
/// refreshed. Zork's Loud Room is the case in point: its echo routine intercepts input, so the
/// status line keeps naming the room you *came from* for as long as you stand there, while
/// `cretin`'s parent points squarely at the Loud Room the whole time.
///
/// Nothing about a room's identity is hard-coded. The status line names a room we CAN resolve, so
/// its object teaches us what a room looks like structurally — its CONTAINER — and the player's
/// nearest ancestor sharing that container is the room they are actually in. Zork keeps every room
/// under one container object, and Inform keeps rooms at the top level (container 0); both fall out
/// of the same rule.
///
/// Candidates are tried in order and the first that yields a room wins, which is what rejects
/// decorative avatars: Zork's `you` hangs off `it` at the top level and never reaches the room
/// container, while `cretin` does.
fn player_room_beside(machine: &Machine, shown: &ObjectSnapshot) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    for player in player_candidates(machine) {
        let mut cur = get_parent(mem, player);
        for _ in 0..32 {
            // Depth-bounded like `nearest_matching_ancestor`, to tolerate cycles.
            if cur == 0 {
                break;
            }
            if get_parent(mem, cur) == shown.parent {
                return Some(object_snapshot(mem, cur));
            }
            cur = get_parent(mem, cur);
        }
    }
    None
}

/// The object whose short name matches `name` (longest match wins; ties -> lowest
/// number), or None.
fn resolve_room_object(machine: &Machine, name: &str) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let n = max_object_number(mem);
    let mut best: Option<(usize, u16)> = None; // (normalized short-name length, object)
    for obj in 1..=n {
        let sn = short_name(mem, obj);
        if status_name_matches(name, &sn) {
            let len = normalize_name(&sn).len();
            if best.is_none_or(|(blen, _)| len > blen) {
                best = Some((len, obj));
            }
        }
    }
    best.map(|(_, obj)| object_snapshot(mem, obj))
}

/// How the current room was determined (drives the map indicator label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationMethod {
    GlobalVar0,
    PlayerParent,
    StatusName,
    NameOnly,
    /// Glulx: the room was read from the Inform 7 `Subheader` room heading in
    /// the story buffer (name-based; no backing object). Trusted directly — not
    /// subject to the `NameOnly`-empty-graph gate.
    RoomHeading,
}

/// The mapper-facing location signal for one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    GlobalVar0(ObjectSnapshot),
    PlayerParent(ObjectSnapshot),
    StatusName(ObjectSnapshot),
    NameOnly(String),
}

impl Location {
    /// The backing object snapshot, or None for a name-only room.
    pub fn object(&self) -> Option<&ObjectSnapshot> {
        match self {
            Location::GlobalVar0(s) | Location::PlayerParent(s) | Location::StatusName(s) => Some(s),
            Location::NameOnly(_) => None,
        }
    }
    /// The detection method tag.
    pub fn method(&self) -> LocationMethod {
        match self {
            Location::GlobalVar0(_) => LocationMethod::GlobalVar0,
            Location::PlayerParent(_) => LocationMethod::PlayerParent,
            Location::StatusName(_) => LocationMethod::StatusName,
            Location::NameOnly(_) => LocationMethod::NameOnly,
        }
    }
}

/// Best-effort current room, version-gated:
/// - v3 and below: global variable 0 -> GlobalVar0, or None.
/// - v4+: validated player-parent -> player-parent beside a stale status line -> status-name ->
///   name-only -> None.
///
/// Stateless: a pure function of the machine, re-run each turn.
pub fn detect_location(machine: &Machine) -> Option<Location> {
    if machine.mem.version() <= 3 {
        return current_location(machine).map(Location::GlobalVar0);
    }
    // v6 (graphical) games paint their status text into the v6 window model, not
    // the v4+ grid, so the grid parser below always sees an empty upper window.
    // Source the candidates from the paint runs instead, feeding the SAME ladder.
    if machine.screen.v6.is_some() {
        return detect_location_v6(machine);
    }
    if let Some(name) = status_line_room_name(&machine.screen.upper, machine.screen.upper_window_rows) {
        // Prefer the avatar whose ancestor chain validates against the status-line
        // room name. Trying every plausible player object (and using the first whose
        // parent chain reaches the shown room) distinguishes same-named rooms that a
        // name-only match would collapse — e.g. Zork's several "Forest" rooms — and
        // rejects decorative "you"/"self" objects whose parent never tracks the player.
        for player in player_candidates(machine) {
            if let Some(room) = nearest_matching_ancestor(machine, player, &name) {
                return Some(Location::PlayerParent(room));
            }
        }
        // Nothing validated: the status line does not describe where the player is. That is the
        // signal the text has gone stale, not a reason to trust it — prefer the object tree, which
        // is the game's own state (SQ-0358).
        if let Some(shown) = resolve_room_object(machine, &name) {
            if let Some(room) = player_room_beside(machine, &shown) {
                return Some(Location::PlayerParent(room));
            }
            // No avatar reaches a room at all: the tree has nothing better to offer, so the status
            // line stands — this is what keeps games with no identifiable player object working.
            return Some(Location::StatusName(shown));
        }
        return Some(Location::NameOnly(name));
    }
    // Centered-title fallback (BeyondZork et al.): a centered row-1 name that the
    // left-justified common form can't parse. A centered line is often a banner
    // or story title, not a room (e.g. Photopia's "Photopia by Adam Cadre", whose
    // top-level title object would satisfy a bare name match). So accept it ONLY
    // under the STRONGEST validation — the avatar's own ancestor chain reaches a
    // room of that name (PlayerParent). A mere name match (StatusName) or an
    // unvalidated NameOnly is NOT trusted here; anything else returns None.
    if let Some(name) =
        centered_status_line_room_name(&machine.screen.upper, machine.screen.upper_window_rows)
    {
        for player in player_candidates(machine) {
            if let Some(room) = nearest_matching_ancestor(machine, player, &name) {
                return Some(Location::PlayerParent(room));
            }
        }
    }
    None
}

/// Returns the object representing the player's current location, or `None` if
/// the heuristic cannot determine a plausible location.
///
/// See module-level docs for the version-specific strategy.
pub fn current_location(machine: &Machine) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    // Global variable 0 is at address `global_vars + 0` (var 0x10 maps to
    // global index 0, stored at global_vars base with no offset).
    let global0_addr = mem.global_vars() as u32;
    let obj_num = mem.read_word(global0_addr);

    if obj_num == 0 {
        return None;
    }

    // Validate that obj_num is within the object table.
    // We use the same bound logic as object_tree_view: check the entry would
    // lie before the first property table pointer within the table.
    let max_obj = max_object_number(mem);
    if obj_num > max_obj {
        return None;
    }

    Some(object_snapshot(mem, obj_num))
}

/// Returns a read-only enumeration of all objects in the story as snapshots.
///
/// Object count is inferred from the layout — see module docs for the approach.
pub fn object_tree_view(machine: &Machine) -> Vec<ObjectSnapshot> {
    let mem = &machine.mem;
    let n = max_object_number(mem);
    (1..=n).map(|i| object_snapshot(mem, i)).collect()
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Infer the maximum valid object number from the object table layout.
///
/// Object entries are stored contiguously and are immediately followed by the
/// property tables, so the LOWEST property-table pointer marks the end of the
/// entries region. We track that minimum as we scan and stop once an entry
/// would extend into (or past) it — the bytes there are property data that
/// merely resemble another entry. (The previous "pointer looks plausible"
/// check had no such bound, so it walked straight into the property tables and
/// over-counted, surfacing garbage objects with corrupt names.) Capped at 2000
/// to guard against pathological data.
fn max_object_number(mem: &crate::memory::Memory) -> u16 {
    let version = mem.version();
    let base = entries_base(mem);
    let esize = entry_size(version);
    let prop_ptr_offset = prop_table_ptr_offset(version);

    let mut min_ptbl = u32::MAX;
    let mut n: u16 = 0;
    for candidate in 1u16..=2000 {
        // Address of this candidate's entry.
        let entry_addr = base + (candidate as u32 - 1) * esize;
        // Stop if this entry would run past the end of the story file, or into
        // the first property table (the end of the real entries region).
        if (entry_addr + esize) as usize > mem.len() || entry_addr + esize > min_ptbl {
            break;
        }
        // Property-table pointer is the last word of the entry.
        let ptbl_addr = mem.read_word(entry_addr + prop_ptr_offset) as u32;

        // A valid property-table pointer is nonzero, points after this entry,
        // and lies within the story file (its name-length byte must be
        // readable). Anything else means we've walked past the real object
        // table into unrelated data that merely looks like an entry.
        if ptbl_addr == 0 || ptbl_addr <= entry_addr || ptbl_addr as usize >= mem.len() {
            break;
        }
        min_ptbl = min_ptbl.min(ptbl_addr);
        n = candidate;
    }
    n
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::exec::{Machine, StepResult};
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::screen::UpperWindow;

    fn upper_with(rows: &[&str]) -> UpperWindow {
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
        let mut u = UpperWindow::default();
        u.resize(rows.len() as u16, cols.max(1));
        for (r, line) in rows.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                u.put((r + 1) as u16, (c + 1) as u16, ch, 0, crate::screen::ZColour::Default, crate::screen::ZColour::Default);
            }
        }
        u
    }

    #[test]
    fn status_room_name_common_form_strips_score_and_posture() {
        let u = upper_with(&[" Bedroom, in the bed                              Score: 0     Moves: 1"]);
        assert_eq!(status_line_room_name(&u, 1).as_deref(), Some("Bedroom"));
    }

    #[test]
    fn status_room_name_plain() {
        let u = upper_with(&[" Darkness                                         Score: 0     Moves: 0"]);
        assert_eq!(status_line_room_name(&u, 1).as_deref(), Some("Darkness"));
    }

    #[test]
    fn status_room_name_location_label_form() {
        let u = upper_with(&[
            " Mode:  Communications Mode                                Time:  7:07pm",
            " Location:  Foo Bar                                        Date:  3/16/2031",
        ]);
        assert_eq!(status_line_room_name(&u, 2).as_deref(), Some("Foo Bar"));
    }

    #[test]
    fn status_room_name_empty_grid_is_none() {
        let u = upper_with(&["                                "]);
        assert_eq!(status_line_room_name(&u, 1), None);
    }

    #[test]
    fn status_room_name_centered_defeats_common_form() {
        // A centered title has empty left-justified first segment → None from
        // the common form; the centered fallback recovers the name.
        let u = upper_with(&["                           Hilltop                                     "]);
        assert_eq!(status_line_room_name(&u, 1), None);
        assert_eq!(centered_status_line_room_name(&u, 1).as_deref(), Some("Hilltop"));
    }

    #[test]
    fn centered_fallback_ignores_left_justified() {
        // Left-justified lines must NOT trigger the centered fallback — the
        // common form already handles them, so it returns None here.
        let u = upper_with(&[" Bedroom                                         Score: 0     Moves: 1"]);
        assert_eq!(centered_status_line_room_name(&u, 1), None);
    }

    #[test]
    fn centered_fallback_multiword_and_empty() {
        let multi = upper_with(&["                     Palace Gate                                        "]);
        assert_eq!(centered_status_line_room_name(&multi, 1).as_deref(), Some("Palace Gate"));
        let empty = upper_with(&["                                                                        "]);
        assert_eq!(centered_status_line_room_name(&empty, 1), None);
    }

    #[test]
    fn bordered_centered_title_vt220() {
        // BeyondZork VT220 mode frames the centered room title with half-block
        // bars: `▐  <spaces>  Hilltop  <spaces>  ▌  <trailing spaces>`. The
        // leading bar must NOT be read as a left-justified room name, and the
        // centered fallback must recover the true name.
        let u = upper_with(&["▐                          Hilltop                           ▌                  "]);
        assert_eq!(status_line_room_name(&u, 1), None, "leading bar must not become a bogus room name");
        assert_eq!(centered_status_line_room_name(&u, 1).as_deref(), Some("Hilltop"));
    }

    #[test]
    fn bordered_centered_title_vt220_multiword() {
        let u = upper_with(&["▐                     Palace Gate                            ▌                  "]);
        assert_eq!(status_line_room_name(&u, 1), None);
        assert_eq!(centered_status_line_room_name(&u, 1).as_deref(), Some("Palace Gate"));
    }

    #[test]
    fn status_name_matches_rules() {
        assert!(status_name_matches("Bedroom", "Bedroom"));            // equal
        assert!(status_name_matches("Bedroom (messy)", "Bedroom"));    // trailing decoration
        assert!(status_name_matches("Bedroom, north end", "Bedroom")); // (post-strip safety net)
        assert!(!status_name_matches("Hallway", "Hall"));              // word-boundary guard
        assert!(status_name_matches("hall  ", "Hall"));                // case + whitespace
        assert!(!status_name_matches("Kitchen", "Bedroom"));          // unrelated
        assert!(!status_name_matches("Bedroom", ""));                 // empty short
    }

    #[test]
    fn find_player_object_by_name() {
        // Rename obj3 to "you" so it is the player.
        // "yourself" would be truncated to "yoursel" by v3 Z-char encoding (6 Z-char max),
        // so we use "you" which is in NAMES and encodes without truncation.
        let mut buf = build_v3_story();
        let name = z_name("you");
        buf[PROP3_TBL as usize] = (name.len() / 2) as u8;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 1 + name.len()].copy_from_slice(&name);
        let machine = make_machine(buf);
        assert_eq!(find_player_object(&machine), Some(3));
    }

    #[test]
    fn find_player_object_finds_player_in_minizork() {
        // Real-game check: minizork's player object is #30, short name "you".
        // This is what makes the inventory panel's name-based player lookup reliable.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minizork.z3");
        if !fixture.exists() {
            eprintln!("SKIP: minizork.z3 fixture not found");
            return;
        }
        let data = std::fs::read(&fixture).expect("read minizork.z3");
        let machine = make_machine(data);
        assert_eq!(find_player_object(&machine), Some(30), "minizork player object is #30 (\"you\")");
    }

    #[test]
    fn resolve_room_object_matches_short_name() {
        let machine = make_machine(build_v3_story()); // obj1 "west", obj2 "east", obj3 "hall"
        let r = resolve_room_object(&machine, "hall").expect("hall resolves");
        assert_eq!(r.number, 3);
        assert!(resolve_room_object(&machine, "nowhere").is_none());
    }

    #[test]
    fn nearest_matching_ancestor_walks_up() {
        // obj tree: obj3 (parent obj1), obj2 (parent obj1), obj1 (parent 0).
        // Searching from obj3 for "west" should walk up to obj1.
        let machine = make_machine(build_v3_story());
        let r = nearest_matching_ancestor(&machine, 3, "west").expect("walks up to west");
        assert_eq!(r.number, 1);
        assert!(nearest_matching_ancestor(&machine, 3, "nowhere").is_none());
    }

    #[test]
    fn detect_location_v3_uses_global0() {
        let mut buf = build_v3_story();
        put_word(&mut buf, GLOBAL_VARS as usize, 1); // global 0 = obj 1
        let machine = make_machine(buf);
        match detect_location(&machine) {
            Some(Location::GlobalVar0(s)) => assert_eq!(s.number, 1),
            other => panic!("expected GlobalVar0, got {other:?}"),
        }
        assert_eq!(detect_location(&machine).unwrap().method(), LocationMethod::GlobalVar0);
    }

    #[test]
    fn location_object_and_method_accessors() {
        let s = ObjectSnapshot { number: 5, parent: 0, name: "Hall".into() };
        assert_eq!(Location::StatusName(s.clone()).object().map(|o| o.number), Some(5));
        assert_eq!(Location::NameOnly("X".into()).object(), None);
        assert_eq!(Location::NameOnly("X".into()).method(), LocationMethod::NameOnly);
        assert_eq!(Location::PlayerParent(s).method(), LocationMethod::PlayerParent);
    }

    // We reuse the same object-table layout as objects.rs tests:
    //   object_table = 0x0100, entries_base = 0x013E (v3)
    //   obj1 at 0x013E, obj2 at 0x0147, obj3 at 0x0150
    //   property tables at 0x0200, 0x0220, 0x0230
    //   global_vars = 0x0300

    const OBJ_TABLE: u32 = 0x0100;
    const ENTRIES_V3: u32 = OBJ_TABLE + 31 * 2; // 0x013E
    const OBJ1_ENTRY: u32 = ENTRIES_V3;
    const OBJ2_ENTRY: u32 = ENTRIES_V3 + 9;
    const OBJ3_ENTRY: u32 = ENTRIES_V3 + 18;

    const PROP1_TBL: u16 = 0x0200;
    const PROP2_TBL: u16 = 0x0220;
    const PROP3_TBL: u16 = 0x0230;

    const GLOBAL_VARS: u32 = 0x0300;

    fn put_word(buf: &mut [u8], offset: usize, val: u16) {
        buf[offset]     = (val >> 8) as u8;
        buf[offset + 1] = (val & 0xFF) as u8;
    }

    /// Write `val` as a big-endian word into the buffer.
    fn z_name(text: &str) -> Vec<u8> {
        crate::text::encode::encode_word(text, 3) // 4 bytes = 2 Z-words for v3
    }

    /// Build a minimal v3 story with 3 objects and properly encoded short names.
    /// Object structure: obj1 (root), obj2 (child of obj1), obj3 (sibling of obj2).
    fn build_v3_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        // ── obj1 entry: parent=0 sibling=0 child=2, prop=PROP1_TBL ──────────
        buf[OBJ1_ENTRY as usize + 4] = 0; // parent
        buf[OBJ1_ENTRY as usize + 5] = 0; // sibling
        buf[OBJ1_ENTRY as usize + 6] = 2; // child
        put_word(&mut buf, OBJ1_ENTRY as usize + 7, PROP1_TBL);

        // ── obj2 entry: parent=1 sibling=3 child=0 ───────────────────────────
        buf[OBJ2_ENTRY as usize + 4] = 1;
        buf[OBJ2_ENTRY as usize + 5] = 3;
        buf[OBJ2_ENTRY as usize + 6] = 0;
        put_word(&mut buf, OBJ2_ENTRY as usize + 7, PROP2_TBL);

        // ── obj3 entry: parent=1 sibling=0 child=0 ───────────────────────────
        buf[OBJ3_ENTRY as usize + 4] = 1;
        buf[OBJ3_ENTRY as usize + 5] = 0;
        buf[OBJ3_ENTRY as usize + 6] = 0;
        put_word(&mut buf, OBJ3_ENTRY as usize + 7, PROP3_TBL);

        // ── Property table for obj1: name "west", no properties ──────────────
        // name_words = 2 (encode_word produces 4 bytes = 2 Z-words for v3)
        let name1 = z_name("west");
        assert_eq!(name1.len(), 4);
        buf[PROP1_TBL as usize] = 2; // 2 Z-words in name
        buf[PROP1_TBL as usize + 1..PROP1_TBL as usize + 5].copy_from_slice(&name1);
        buf[PROP1_TBL as usize + 5] = 0x00; // end-of-properties sentinel

        // ── Property table for obj2: name "east" ─────────────────────────────
        let name2 = z_name("east");
        buf[PROP2_TBL as usize] = 2;
        buf[PROP2_TBL as usize + 1..PROP2_TBL as usize + 5].copy_from_slice(&name2);
        buf[PROP2_TBL as usize + 5] = 0x00;

        // ── Property table for obj3: name "hall" ─────────────────────────────
        let name3 = z_name("hall");
        buf[PROP3_TBL as usize] = 2;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 5].copy_from_slice(&name3);
        buf[PROP3_TBL as usize + 5] = 0x00;

        buf
    }

    /// Build a Machine from story bytes.
    fn make_machine(buf: Vec<u8>) -> Machine {
        Machine::new(Memory::new(buf).unwrap())
    }

    // ── v5 avatar-based detection ────────────────────────────────────────────
    // Regression for the Zork1-r52 forest collapse: same-named rooms must be
    // distinguished by the true avatar's parent, not by a name-only match that
    // resolves every "forest" to one scenery object.

    const V5_ENTRIES: u32 = OBJ_TABLE + 63 * 2; // prop-defaults = 63 words (v4+)
    fn v5_entry(obj: u16) -> usize {
        (V5_ENTRIES + (obj as u32 - 1) * 14) as usize
    }

    /// Build a v5 story with 5 objects reproducing the r52 topology:
    ///   #1 "forest" scenery (top-level)   — what StatusName wrongly resolves to
    ///   #2 "you"    decorative, parent 0  — what find_player_object picks
    ///   #3 "forest" a real forest ROOM
    ///   #4 "cretin" the true avatar, child of #3
    ///   #5 "forest" a second real forest ROOM
    /// Lowercase names avoid A1/A2 shift-encoding; the status line matches case-
    /// insensitively.
    fn build_v5_forests() -> Vec<u8> {
        let mut buf = sample_story(5);
        let props: [(u16, &str); 5] =
            [(0x1D2, "forest"), (0x1DA, "you"), (0x1E2, "forest"), (0x1EA, "cretin"), (0x1F2, "forest")];
        let parents: [u16; 5] = [0, 0, 0, 3, 0];
        for (i, (ptbl, name)) in props.iter().enumerate() {
            let obj = (i + 1) as u16;
            let e = v5_entry(obj);
            put_word(&mut buf, e + 6, parents[i]); // parent (word, v4+)
            put_word(&mut buf, e + 8, 0); // sibling
            put_word(&mut buf, e + 10, 0); // child
            put_word(&mut buf, e + 12, *ptbl); // property-table pointer
            let nm = crate::text::encode::encode_word(name, 5);
            let p = *ptbl as usize;
            buf[p] = (nm.len() / 2) as u8; // name length in words
            buf[p + 1..p + 1 + nm.len()].copy_from_slice(&nm);
            buf[p + 1 + nm.len()] = 0x00; // end-of-properties sentinel
        }
        buf
    }

    /// v5 machine with the avatar (#4 cretin) placed in `cretin_parent` and a
    /// status line showing "forest".
    fn machine_in_forest(cretin_parent: u16) -> Machine {
        let mut buf = build_v5_forests();
        put_word(&mut buf, v5_entry(4) + 6, cretin_parent);
        let mut m = make_machine(buf);
        m.screen.upper = upper_with(&[" forest                              Score: 0    Moves: 0"]);
        m.screen.upper_window_rows = 1;
        m
    }

    #[test]
    fn v5_builder_encodes_names() {
        // Guard: distinguish a builder bug from a detection bug.
        let m = machine_in_forest(3);
        assert_eq!(normalize_name(&short_name(&m.mem, 3)), "forest");
        assert_eq!(normalize_name(&short_name(&m.mem, 4)), "cretin");
        assert_eq!(status_line_room_name(&m.screen.upper, m.screen.upper_window_rows).as_deref(), Some("forest"));
    }

    #[test]
    fn v5_same_named_rooms_resolve_via_avatar_not_name() {
        // Avatar #4 is in forest ROOM #3; scenery "forest" is #1. Name-only
        // resolution would return #1; the avatar's parent gives the true #3.
        let m = machine_in_forest(3);
        let loc = detect_location(&m).expect("should detect a location");
        assert_eq!(loc.method(), LocationMethod::PlayerParent, "must validate the avatar, not match by name");
        assert_eq!(loc.object().unwrap().number, 3, "true room is #3, not scenery #1");
    }

    #[test]
    fn find_player_object_prefers_situated_avatar_over_decorative_you() {
        // Zork1-r52 topology: #2 "you" is a decorative object with no parent,
        // #4 "cretin" is the real avatar situated inside forest ROOM #3.
        // Old name-only lookup grabbed #2 (empty child chain -> inventory always
        // empty); the situated-candidate heuristic must select #4 instead.
        let m = machine_in_forest(3);
        assert_eq!(get_parent(&m.mem, 2), 0, "decorative \"you\" #2 is parentless");
        assert_eq!(get_parent(&m.mem, 4), 3, "avatar \"cretin\" #4 sits in room #3");
        assert_eq!(
            find_player_object(&m),
            Some(4),
            "must pick the situated avatar #4 (cretin), not the decorative #2 (you)"
        );
    }

    #[test]
    fn find_player_object_falls_back_to_lowest_when_none_situated() {
        // No candidate is situated (avatar #4 also parentless): fall back to the
        // lowest-numbered candidate so games without a decorative object are
        // unaffected (here that is #2 "you", the first player-named object).
        let m = machine_in_forest(0);
        assert_eq!(get_parent(&m.mem, 2), 0);
        assert_eq!(get_parent(&m.mem, 4), 0);
        assert_eq!(find_player_object(&m), Some(2), "fallback picks the lowest-numbered candidate");
    }

    #[test]
    fn v5_distinct_forests_get_distinct_ids() {
        let a = detect_location(&machine_in_forest(3)).unwrap();
        let b = detect_location(&machine_in_forest(5)).unwrap();
        assert_eq!(a.object().unwrap().number, 3);
        assert_eq!(b.object().unwrap().number, 5);
        assert_ne!(
            a.object().unwrap().number,
            b.object().unwrap().number,
            "two different forest rooms must not collapse to one id"
        );
    }

    // ── v6 (graphical) status-band detection (SQ-0468) ───────────────────────
    // The v6 paint model (`screen.v6`) replaces the v4+ grid as the status-text
    // source. Bands below reproduce the real Zork0/Shogun layouts verified
    // headlessly (see the app-level v6_location_mapper gameplay tests).

    use crate::screen::{V6Text, V6Windows, ZColour};

    /// A v6 status band in window 1 (y_coord=1) painted with the given
    /// `(y, x, text)` runs at their absolute screen pixel positions.
    fn v6_band(runs: &[(u16, u16, &str)]) -> V6Windows {
        let mut v = V6Windows::default();
        v.windows[1].y_coord = 1;
        v.windows[1].x_coord = 1;
        for &(y, x, text) in runs {
            v.windows[1].texts.push(V6Text {
                y,
                x,
                text: text.into(),
                style: 0,
                fg: ZColour::Default,
                bg: ZColour::Default,
            });
        }
        v
    }

    #[test]
    fn v6_candidates_shogun_layout_filters_label_banner_and_stats() {
        // Row 1: "Erasmus:" (label — ":" abuts) + "SHOGUN" (centered banner) +
        // Score. Row 2: "Bridge" (room, left) + Moves. Expect only the real
        // room-ish runs, left-anchored first: Bridge, then the centered SHOGUN.
        // 640×400 coords (SQ-0479 re-trace, window x_coord=1): Erasmus label
        // dx=2, ":" abuts, SHOGUN banner dx=249, Score dx=457; row 2 (y=17):
        // Bridge dx=2, Moves dx=457.
        let v = v6_band(&[
            (1, 3, "Erasmus"),
            (1, 59, ":"),
            (1, 250, "SHOGUN"),
            (1, 458, "Score:"),
            (1, 538, "0"),
            (17, 3, "Bridge"),
            (17, 458, "Moves:"),
            (17, 538, "1"),
        ]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert_eq!(v6_status_room_candidates(&m), vec!["Bridge".to_string(), "SHOGUN".to_string()]);
    }

    #[test]
    fn v6_candidates_zork0_layout_room_left_kingdom_other() {
        // Zork0: room "Banquet Hall" left on the top row; kingdom "Flatheadia"
        // right-anchored; Score/Moves filtered. Left-anchored room comes first.
        // 640×400 coords (SQ-0479 re-trace): room y=11 x=71 (dx=70, left);
        // Flatheadia x=489 (dx=488, right); Score/Moves on the y=27 row.
        let v = v6_band(&[
            (11, 71, "Banquet Hall"),
            (11, 489, "Flatheadia"),
            (27, 71, "Moves:"),
            (27, 489, "Score:"),
        ]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert_eq!(
            v6_status_room_candidates(&m),
            vec!["Banquet Hall".to_string(), "Flatheadia".to_string()]
        );
    }

    #[test]
    fn v6_candidates_below_band_and_blanks_ignored() {
        // Runs below the 64px band (Journey's menu at y=305) and blank padding
        // never become candidates → menu screens yield nothing. y=65 is one pixel
        // past the 4-row (64px) band.
        let v = v6_band(&[(9, 121, " "), (305, 75, "START the game"), (65, 26, "TooLow")]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert!(v6_status_room_candidates(&m).is_empty());
    }

    #[test]
    fn v6_detect_playerparent_validates_left_room() {
        // build_v5_forests: avatar #4 "cretin" sits in forest ROOM #3. A
        // left-anchored "forest" run must validate via the avatar's ancestor
        // chain → PlayerParent(#3), NOT the scenery "forest" #1.
        let mut m = machine_in_forest(3);
        m.screen.v6 = Some(v6_band(&[(11, 71, "forest"), (27, 71, "Score: 0")]));
        let loc = detect_location(&m).expect("v6 left room should detect");
        assert_eq!(loc.method(), LocationMethod::PlayerParent);
        assert_eq!(loc.object().unwrap().number, 3, "true room #3, not scenery #1");
    }

    #[test]
    fn v6_detect_statusname_when_avatar_not_parented_to_room() {
        // Shogun case: the avatar is NOT in the named room's subtree, so
        // PlayerParent can't fire. A LEFT-anchored name that resolves to a real
        // object falls back to StatusName. Here cretin #4 is parentless (0) so no
        // ancestor validates; "forest" resolves to the scenery object #1.
        let mut m = machine_in_forest(0);
        m.screen.v6 = Some(v6_band(&[(17, 71, "forest"), (17, 489, "Moves: 1")]));
        let loc = detect_location(&m).expect("left-anchored name resolves to an object");
        assert_eq!(loc.method(), LocationMethod::StatusName);
        assert_eq!(loc.object().unwrap().number, 1, "\"forest\" resolves to scenery #1");
    }

    #[test]
    fn v6_detect_rejects_centered_banner_without_playerparent() {
        // A CENTERED run that matches no avatar ancestor is never StatusName'd —
        // banners like "SHOGUN" must not become rooms. Even though "forest" is a
        // REAL object (#1), a centered placement with a parentless avatar yields
        // None (StatusName is left-anchored-only).
        let mut m = machine_in_forest(0);
        m.screen.v6 = Some(v6_band(&[(11, 250, "forest"), (11, 489, "Score: 0")]));
        assert_eq!(detect_location(&m), None, "centered object-name must not yield StatusName");
    }

    #[test]
    fn v6_detect_drops_nameonly_for_unknown_left_name() {
        // A left-anchored name with NO backing object and not an ancestor yields
        // None for v6 (NameOnly is dropped) — a title/character-sheet name must
        // never invent a room.
        let mut m = machine_in_forest(0);
        m.screen.v6 = Some(v6_band(&[(11, 71, "Nowhere City"), (11, 489, "Score: 0")]));
        assert_eq!(detect_location(&m), None, "unknown v6 name must not become a NameOnly room");
    }

    #[test]
    fn v6_detect_none_on_blank_menu_band() {
        // Menu/title phase: only blank/padding runs in the band → None.
        let mut m = machine_in_forest(3);
        m.screen.v6 = Some(v6_band(&[(1, 121, " "), (9, 121, " ")]));
        assert_eq!(detect_location(&m), None, "no room text → no location (title/menu is None)");
    }

    // ── TDD Step 1: write the failing tests ───────────────────────────────────
    // (These were written BEFORE the implementation; the RED→GREEN cycle is
    //  documented in the task report.)

    // ── current_location: v3 hit ──────────────────────────────────────────────

    #[test]
    fn v3_current_location_from_global0() {
        let mut buf = build_v3_story();
        // Set global 0 (at GLOBAL_VARS) to object 1.
        put_word(&mut buf, GLOBAL_VARS as usize, 1);
        let machine = make_machine(buf);
        let loc = current_location(&machine).expect("should return a snapshot");
        assert_eq!(loc.number, 1);
        // Name should be "west" (our encoded name for obj1).
        assert!(loc.name.starts_with('w'), "expected name starting with 'w', got {:?}", loc.name);
    }

    // ── current_location: v3 None when global0 == 0 ──────────────────────────

    #[test]
    fn v3_current_location_none_when_global0_zero() {
        let mut buf = build_v3_story();
        // global 0 = 0 → no location
        put_word(&mut buf, GLOBAL_VARS as usize, 0);
        let machine = make_machine(buf);
        assert!(current_location(&machine).is_none());
    }

    // ── current_location: None when global0 exceeds max object ───────────────

    #[test]
    fn v3_current_location_none_when_global0_out_of_range() {
        let mut buf = build_v3_story();
        // 0xFFFF is never a valid object in our tiny tree
        put_word(&mut buf, GLOBAL_VARS as usize, 0xFFFF);
        let machine = make_machine(buf);
        assert!(current_location(&machine).is_none());
    }

    // ── object_tree_view: returns all 3 objects with correct fields ───────────

    #[test]
    fn v3_object_tree_view_count_and_fields() {
        let mut buf = build_v3_story();
        // global 0 = 0 (irrelevant for tree view)
        put_word(&mut buf, GLOBAL_VARS as usize, 0);
        let machine = make_machine(buf);
        let tree = object_tree_view(&machine);

        assert_eq!(tree.len(), 3, "expected exactly 3 objects, got {}", tree.len());

        // obj1: number=1 parent=0 name starts 'w'
        assert_eq!(tree[0].number, 1);
        assert_eq!(tree[0].parent, 0);
        assert!(tree[0].name.starts_with('w'), "obj1 name: {:?}", tree[0].name);

        // obj2: number=2 parent=1 name starts 'e'
        assert_eq!(tree[1].number, 2);
        assert_eq!(tree[1].parent, 1);
        assert!(tree[1].name.starts_with('e'), "obj2 name: {:?}", tree[1].name);

        // obj3: number=3 parent=1 name starts 'h'
        assert_eq!(tree[2].number, 3);
        assert_eq!(tree[2].parent, 1);
        assert!(tree[2].name.starts_with('h'), "obj3 name: {:?}", tree[2].name);
    }

    // ── Fixture-backed test (skips when minizork.z3 absent) ──────────────────

    #[test]
    fn minizork_current_location_returns_something() {
        let Some(story) = crate::fixtures::load("minizork.z3") else {
            // Fixture absent — skip.
            return;
        };
        let mem = Memory::new(story).unwrap();
        let mut machine = Machine::new(mem);
        machine.init_caps();
        // In minizork the opening location is set in global 0 before the first
        // read instruction.  Run until NeedLine (the first READ opcode) so that
        // the init code has had a chance to store the starting room into global 0.
        for _ in 0..100_000u64 {
            match machine.step() {
                StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => break,
                StepResult::NeedChar => { machine.supply_char(b'\n'); }
                StepResult::SaveRequest => { machine.complete_save(false); }
                StepResult::RestoreRequest => { machine.complete_restore_failure(); }
                StepResult::Continue => {}
            }
        }
        let loc = current_location(&machine);
        assert!(loc.is_some(), "minizork: expected a location from global 0, got None");
    }
    #[test]
    fn minizork_object_table_stops_at_the_real_end_no_garbage() {
        // Regression: the object-entry scan used to walk past the real table into
        // the property-table data and over-count, surfacing garbage objects with
        // corrupt names. Bounded by the lowest property-table pointer, minizork
        // has exactly 179 objects; the last is "pseudo" (the pseudo-object), not
        // a garbage-named phantom.
        let Some(story) = crate::fixtures::load("minizork.z3") else {
            return; // fixture absent — skip
        };
        let mem = Memory::new(story).unwrap();
        let machine = Machine::new(mem);
        let tree = object_tree_view(&machine);
        assert_eq!(tree.len(), 179, "minizork has 179 real objects");
        assert_eq!(tree.first().map(|s| s.name.as_str()), Some("forest"));
        assert_eq!(tree.last().map(|s| s.name.as_str()), Some("pseudo"));
    }
}
