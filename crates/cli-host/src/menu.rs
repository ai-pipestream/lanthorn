//! Grid-drawn menus, heard rather than seen.
//!
//! A menu in interactive fiction is a rectangle the game repaints: a list of
//! items with a marker — `>` almost always — parked on the current one, and a
//! legend saying which keys move it. Sighted, the marker jumps and nothing else
//! appears to happen. Linearised for a listener (SQ-0607 for Glk grids,
//! zvm-cli's inline upper-window block for the Z-machine), *every repaint is a
//! fresh block of text*: Planetfall's InvisiClues menu is sixteen lines, and
//! before this module pressing `N` read all sixteen out again to say that a `>`
//! had moved down one row. Arthur's is twenty-two. That is followable — SQ-0609
//! deliberately shipped it that way rather than guess — but it is not usable.
//!
//! So the host recognises the repaint and says the one thing that changed.
//!
//! ## Detection is a diff, not a heuristic about content
//!
//! Nothing here knows what a menu "looks like" in the abstract, and nothing
//! pattern-matches game titles. A block is compared with the last block from the
//! same source, and if the two differ *only* in where the marker sits — same
//! items, same headers, same legend, marker on a different line — that is a
//! **navigation** event and it announces one line. Any other difference is
//! content and emits normally. That asymmetry is the whole safety argument: a
//! status line whose text changed, a menu that scrolled, a form that gained a
//! field — all of them differ somewhere other than the marker column, so all of
//! them take the ordinary path. Menu detection must never eat a legitimate
//! update that happens to repeat.
//!
//! ## Which lines are items
//!
//! Only the lines the marker could land on get numbered, and the rule is
//! **shape, not observation**: an item is a non-blank line whose text begins at
//! the same column as the marked line's text, with nothing but blanks and marker
//! characters before it. Measured against the three real menus:
//!
//! - **Arthur** (`arthrizm.z5`) — a centred title at column 20, an
//!   `N = next item` legend at column 1, items at column 3, and a `(more)`
//!   pagination hint at column 4. Only the items match.
//! - **Planetfall InvisiClues** — title at column 31, two legend rows at column
//!   1, twelve chapter headings at column 3.
//! - **Counterfeit Monkey**'s `ABOUT` menu — seven items at column 3, preceded
//!   by the echoed command at column 0.
//!
//! Observation would work too — number the lines the marker has been seen on —
//! but the set only grows as the player explores, so the numbering would shift
//! under them mid-menu. The column rule is fixed the moment the menu opens, and
//! it errs towards numbering more lines rather than fewer: an over-numbered
//! header is a small annoyance, an unreachable item is a dead end.
//!
//! Blank lines *within* the run are tolerated (Arthur groups its items with
//! them) but never numbered.
//!
//! ## Repainted twice
//!
//! Counterfeit Monkey draws its whole item list twice per keypress. A block
//! whose item text is one list repeated k times is reduced to a single period
//! before anything else looks at it — the game repainted, and a repaint is not
//! fourteen items.

use std::fmt::Write as _;

/// The line that asks the host — not the game — to re-read the open menu.
///
/// Same shape and the same argument as [`crate::input::STATUS_COMMAND`]: no IF
/// parser gives a leading `/` a meaning, so intercepting it cannot shadow a verb
/// the game defines.
pub const MENU_COMMAND: &str = "/menu";

/// Is this input line a menu re-read request rather than a game command?
pub fn is_menu_request(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case(MENU_COMMAND)
}

/// The header a numbered menu is introduced with.
pub const MENU_HINT: &str = "[menu — type a number to jump, Enter to select]";

/// Characters accepted as a selection marker.
///
/// Deliberately short and deliberately not `-`, `+` or `=`: those open key
/// legends (`N = next item`) and rules, and a marker set is only safe while its
/// members do not appear at the start of lines that are not items.
const MARKERS: [char; 6] = ['>', '*', '→', '»', '•', '▶'];

/// One line of a block, split at the first character that is neither blank nor a
/// marker. `None` for a line with no text at all (blank, or pure decoration).
fn anatomy(line: &str) -> Option<(usize, bool)> {
    let mut col = 0usize;
    let mut marked = false;
    for ch in line.chars() {
        if ch.is_whitespace() {
            col += 1;
        } else if MARKERS.contains(&ch) {
            col += 1;
            marked = true;
        } else {
            return Some((col, marked));
        }
    }
    None
}

/// The text of a line with its marker/indent prefix removed.
fn body(line: &str) -> &str {
    line.trim_start_matches(|c: char| c.is_whitespace() || MARKERS.contains(&c))
}

/// A block of emitted text that parsed as a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuBlock {
    /// The block as emitted, one string per line.
    lines: Vec<String>,
    /// Indices into `lines` of the item lines, in order. Length is always a
    /// whole multiple of `period`.
    items: Vec<usize>,
    /// Distinct items — `items.len()` divided by however many times the game
    /// repainted the same list into one block.
    period: usize,
    /// 1-based ordinal of the marked item within a period.
    current: usize,
}

impl MenuBlock {
    /// Parse `block` (a `\n`-separated emitted block) as a menu, or `None`.
    ///
    /// `None` is the common answer and the safe one: a status line, a form, a
    /// compass rose and an ordinary paragraph all fail here and are emitted
    /// unchanged by the caller.
    pub fn parse(block: &str) -> Option<MenuBlock> {
        let lines: Vec<String> = block
            .trim_end_matches('\n')
            .split('\n')
            .map(|l| l.trim_end().to_string())
            .collect();
        let anat: Vec<Option<(usize, bool)>> = lines.iter().map(|l| anatomy(l)).collect();

        // Exactly one column of marked lines, all wearing the marker at the same
        // indent. A block with markers at two different indents is a page of
        // bullets, not a menu with a cursor on it.
        let marks: Vec<usize> = anat
            .iter()
            .enumerate()
            .filter_map(|(i, a)| a.and_then(|(_, m)| m.then_some(i)))
            .collect();
        let &first = marks.first()?;
        let col = anat[first]?.0;
        if marks.iter().any(|&i| anat[i].map(|a| a.0) != Some(col)) {
            return None;
        }

        // Items: every line whose text starts in the marked line's column.
        let items: Vec<usize> = anat
            .iter()
            .enumerate()
            .filter_map(|(i, a)| (a.map(|(c, _)| c) == Some(col)).then_some(i))
            .collect();
        if items.len() < 2 {
            return None;
        }
        // The run must be unbroken but for blanks. Anything else between the
        // first and last item means these lines are not one list.
        let (lo, hi) = (items[0], items[items.len() - 1]);
        for (i, line) in lines.iter().enumerate().take(hi + 1).skip(lo) {
            if !items.contains(&i) && !line.trim().is_empty() {
                return None;
            }
        }

        let bodies: Vec<&str> = items.iter().map(|&i| body(&lines[i])).collect();
        let period = smallest_period(&bodies);
        if period < 2 {
            return None;
        }
        // A repainted list must carry the marker at the same ordinal in every
        // copy, or the copies are not copies and the block is something else.
        let ordinals: Vec<usize> = marks
            .iter()
            .map(|m| items.iter().position(|i| i == m).unwrap_or(0) % period)
            .collect();
        if ordinals.iter().any(|o| *o != ordinals[0]) {
            return None;
        }
        Some(MenuBlock { lines, items, period, current: ordinals[0] + 1 })
    }

    /// How many distinct items the menu offers.
    pub fn count(&self) -> usize {
        self.period
    }

    /// 1-based ordinal of the marked item.
    pub fn current(&self) -> usize {
        self.current
    }

    /// The text of item `n` (1-based), without its marker or indent.
    pub fn item(&self, n: usize) -> Option<&str> {
        let idx = *self.items.get(n.checked_sub(1)?)?;
        Some(body(&self.lines[idx]))
    }

    /// Everything in the block that is not an item line, in order — the header,
    /// the legend, the blanks. Two menus are the same menu when these agree.
    fn furniture(&self) -> Vec<&str> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.items.contains(i))
            .map(|(_, l)| l.as_str())
            .collect()
    }

    /// Item text, one period's worth.
    fn bodies(&self) -> Vec<&str> {
        self.items[..self.period].iter().map(|&i| body(&self.lines[i])).collect()
    }

    /// Is `self` the same menu as `prev` with the marker somewhere else?
    ///
    /// Everything must agree except the marked ordinal: same items in the same
    /// order, same headers and legend, same repaint count. This is the one test
    /// that decides whether a repaint is announced in a line or read out in
    /// full, so it is stated as equality and not as a tolerance.
    pub fn is_navigation_from(&self, prev: &MenuBlock) -> bool {
        self.period == prev.period
            && self.current != prev.current
            && self.bodies() == prev.bodies()
            && self.furniture() == prev.furniture()
    }

    /// The whole menu, host-numbered — what a listener gets when it opens.
    ///
    /// Item lines are replaced by their ordinal; everything else (title, legend,
    /// blanks) is passed through untouched and unnumbered, because a number in
    /// front of `N = next item` is an invitation to type a key that does
    /// nothing. Repainted copies of the list are dropped.
    pub fn listing(&self) -> String {
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            // Whatever came before the first item — a title, a legend, or in
            // Counterfeit Monkey's case the echo of the command that opened the
            // menu — is the game's own preamble and stays in front. The hint
            // then introduces the numbers it is about, rather than the preamble.
            if i == self.items[0] {
                out.push_str(MENU_HINT);
                out.push('\n');
            }
            match self.items.iter().position(|&x| x == i) {
                Some(n) if n < self.period => {
                    let _ = writeln!(
                        out,
                        "{}{}. {}",
                        if n + 1 == self.current { '>' } else { ' ' },
                        n + 1,
                        body(line)
                    );
                }
                Some(_) => {} // a repaint of a list already numbered above
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// The one line a marker move is worth: `>3. What about the bear? (3 of 7)`.
    pub fn announcement(&self) -> String {
        format!(
            ">{}. {} ({} of {})",
            self.current,
            self.item(self.current).unwrap_or(""),
            self.current,
            self.period
        )
    }
}

/// The smallest `p` such that `xs` is `xs[..p]` repeated. `xs.len()` when there
/// is no shorter one.
fn smallest_period<T: PartialEq>(xs: &[T]) -> usize {
    let n = xs.len();
    (1..=n).find(|&p| n.is_multiple_of(p) && (0..n).all(|i| xs[i] == xs[i % p])).unwrap_or(n)
}

/// A navigation keystroke the host must translate into its own engine's code.
///
/// Kept abstract because ZSCII and Glk disagree on every one of them (ZMSD §3.8
/// gives cursor-up 129 and cursor-down 130; Glk's `keycode_Up`/`keycode_Down`
/// live at the top of the 32-bit range), and this crate has no business knowing
/// either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    /// A literal character the menu's own legend names.
    Char(char),
    /// Cursor down — the fallback when the legend names no keys.
    Down,
    /// Cursor up.
    Up,
}

/// The keys a menu says move its marker, read out of its own legend.
///
/// A menu that documents `N = next item` is a menu that wants `n`, and pressing
/// Down at it does nothing. Only when the block names no such keys does the host
/// fall back to the cursor keys, which is what a menu with no legend is most
/// likely listening for.
///
/// Matches `X = next…` / `X = previous…` in any case, on either side of a line,
/// which covers all three measured legends: Arthur's
/// `N = next item / P = previous item`, Planetfall's `N = Next / P = Previous`,
/// and Counterfeit Monkey's `N = Next / P = Previous` split across two rows.
pub fn nav_keys_from_legend(text: &str) -> (NavKey, NavKey) {
    let mut next = None;
    let mut prev = None;
    for (at, _) in text.match_indices('=') {
        let before = text[..at].trim_end();
        let key = before.chars().next_back().filter(|c| c.is_ascii_alphanumeric());
        // The key must be a lone token, not the tail of a word: `ENTER = select`
        // names Enter, not `R`.
        let lone = before.len() == before.trim_end_matches(|c: char| c.is_alphanumeric()).len() + 1;
        let after = text[at + 1..].trim_start().to_ascii_lowercase();
        let (Some(key), true) = (key, lone) else { continue };
        if after.starts_with("next") && next.is_none() {
            next = Some(key);
        } else if after.starts_with("prev") && prev.is_none() {
            prev = Some(key);
        }
    }
    match (next, prev) {
        (Some(n), Some(p)) => (NavKey::Char(n), NavKey::Char(p)),
        // Half a legend is not a legend: a menu that names only one direction is
        // as likely to be prose that happened to contain `= next`.
        _ => (NavKey::Down, NavKey::Up),
    }
}

/// What a host should emit for the block it just produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Emission {
    /// Emit this text in place of the block (already newline-terminated).
    Block(String),
    /// Emit this single line instead of the block.
    Line(String),
    /// Emit nothing at all.
    Nothing,
}

/// What the host should do with a line typed at a char prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    /// A jump was queued; feed the game [`MenuTracker::next_key`] instead of
    /// whatever the player typed.
    Jump,
    /// The marker is already on the item named. Say so and read again; the game
    /// must not be handed a digit it never asked for.
    Here(String),
    /// Not a menu jump. Hand the game the keystroke, as always.
    Passthrough,
}

/// A number jump in progress: press, look, press again.
///
/// Counting the presses up front would be wrong, and measurably so. Arthur's
/// menu carries unselectable section headings — `Documentation`, `Invisiclues` —
/// at the items' own indent, and its `N` steps straight over them: measured, the
/// marker goes Credits → Sample Transcript → The Churchyard, skipping one line
/// each time. Any rule that decides the item lines by shape will number lines
/// the game will not stop on (and the alternative, numbering only lines the
/// marker has been seen on, renumbers the menu under the player as they
/// explore). So the walk steers instead of counting: one key, read where the
/// marker actually went, decide again.
#[derive(Clone, Copy, Debug)]
struct Walk {
    goal: usize,
    next: NavKey,
    prev: NavKey,
    /// Presses left before giving up. A menu that wraps, or one whose marker
    /// cannot reach the named ordinal at all, must not spin forever.
    budget: usize,
}

/// Per-host menu state: the last block seen from each source, and any
/// host-driven walk in progress.
///
/// One tracker per host, not per source, because a walk belongs to the host: a
/// number typed at a menu is answered by synthesizing the game's own keys, and
/// while those are in flight every source must stay quiet.
#[derive(Debug, Default)]
pub struct MenuTracker {
    /// Last parsed block per source id. A source with no menu holds `None` so
    /// its closing is remembered.
    last: Vec<(u32, Option<MenuBlock>)>,
    /// Source of the menu currently open, if any.
    active: Option<u32>,
    /// The jump being walked, if any.
    walk: Option<Walk>,
}

impl MenuTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe the block `source` is about to emit, and say what to emit instead.
    pub fn observe(&mut self, source: u32, block: &str) -> Emission {
        let parsed = MenuBlock::parse(block);
        let previous = self.slot(source).take();
        match (previous, parsed) {
            (Some(prev), Some(now)) if now.is_navigation_from(&prev) => {
                let line = now.announcement();
                let arrived = now.current();
                *self.slot(source) = Some(now);
                self.active = Some(source);
                match self.walk {
                    // Still steering: the player asked for one landing, not five.
                    Some(w) if w.goal != arrived && w.budget > 0 => Emission::Nothing,
                    Some(_) => {
                        self.walk = None;
                        Emission::Line(line)
                    }
                    None => Emission::Line(line),
                }
            }
            (_, Some(now)) => {
                let listing = now.listing();
                *self.slot(source) = Some(now);
                self.active = Some(source);
                // The menu changed under the walk (it scrolled, or opened a
                // submenu). Steering by ordinal means nothing now.
                self.walk = None;
                Emission::Block(listing)
            }
            (_, None) => {
                if self.active == Some(source) {
                    self.active = None;
                    self.walk = None;
                }
                Emission::Block(block.to_string())
            }
        }
    }

    /// The source produced nothing new this stop (its own dedupe swallowed it).
    ///
    /// That is the end of a walk: the marker did not move, so pressing again
    /// would only press again. It happens whenever a jump asks for an ordinal
    /// the game will not stop on — a section heading, or the far side of a list
    /// that does not wrap — and it must still report where the marker ended up,
    /// or a player who typed a number hears nothing at all.
    pub fn unchanged(&mut self, source: u32) -> Emission {
        // Only the source the menu is on can end a walk. A host has as many
        // sources as the game has windows, and the ones that are not the menu go
        // unchanged every single turn — Counterfeit Monkey's legend grid does —
        // so answering for them would cancel every walk after its first step.
        if self.active != Some(source) || self.walk.take().is_none() {
            return Emission::Nothing;
        }
        match self.active().map(MenuBlock::announcement) {
            Some(line) => Emission::Line(line),
            None => Emission::Nothing,
        }
    }

    /// The menu currently open, if any.
    pub fn active(&self) -> Option<&MenuBlock> {
        let id = self.active?;
        self.last.iter().find(|(s, _)| *s == id)?.1.as_ref()
    }

    /// The open menu re-listed on demand (`/menu`), or `None` when none is open.
    pub fn listing(&self) -> Option<String> {
        self.active().map(MenuBlock::listing)
    }

    /// Plan a jump to item `target` (1-based), given the legend text to read the
    /// menu's own navigation keys out of.
    ///
    /// Returns `false` — and queues nothing — when no menu is open, the target
    /// is out of range, or the marker is already there. The host walks the menu
    /// with the game's own keys rather than teleporting because there is no way
    /// to teleport: the game owns the marker, and the only interface it exposes
    /// is the keystroke it documents.
    pub fn plan_jump(&mut self, target: usize, legend: &str) -> bool {
        let Some(menu) = self.active() else { return false };
        if target == 0 || target > menu.count() || target == menu.current() {
            return false;
        }
        let (next, prev) = nav_keys_from_legend(legend);
        // Generous, because a step may skip a heading, and bounded, because a
        // wrapping menu asked for an unreachable ordinal would otherwise press
        // forever.
        let budget = 2 * menu.count() + 4;
        self.walk = Some(Walk { goal: target, next, prev, budget });
        true
    }

    /// What a line typed at a char prompt means while a menu is open.
    ///
    /// Screen-reader mode leaves the terminal cooked, so a "keypress" arrives as
    /// a whole line — which is the only reason a *multi-digit* jump is possible
    /// at all. Raw mode would deliver `1` then `2` with no way to tell `12` from
    /// item 1 followed by item 2; a cooked read is already terminated by Enter,
    /// so that is the termination rule, chosen by the shape of the read rather
    /// than invented on top of it.
    ///
    /// Anything that is not a bare number — including a digit when no menu is
    /// open, because plenty of menus select by number themselves — passes
    /// straight through to the game.
    pub fn typed(&mut self, line: &str, legend: &str) -> Typed {
        let line = line.trim();
        if line.is_empty() || !line.chars().all(|c| c.is_ascii_digit()) {
            return Typed::Passthrough;
        }
        let Ok(target) = line.parse::<usize>() else { return Typed::Passthrough };
        let Some(menu) = self.active() else { return Typed::Passthrough };
        if target == 0 || target > menu.count() {
            return Typed::Passthrough;
        }
        if target == menu.current() {
            return Typed::Here(menu.announcement());
        }
        self.plan_jump(target, legend);
        Typed::Jump
    }

    /// The next synthesized keystroke, or `None` when the game should be given
    /// whatever the player types next.
    ///
    /// Called at the input stop, *after* the block for that stop has been
    /// observed — so the marker's current position is already known and the
    /// direction is decided fresh each time.
    pub fn next_key(&mut self) -> Option<NavKey> {
        let walk = self.walk.as_mut()?;
        if walk.budget == 0 {
            self.walk = None;
            return None;
        }
        walk.budget -= 1;
        let (goal, next, prev) = (walk.goal, walk.next, walk.prev);
        let current = self.active()?.current();
        match goal.cmp(&current) {
            std::cmp::Ordering::Greater => Some(next),
            std::cmp::Ordering::Less => Some(prev),
            std::cmp::Ordering::Equal => {
                self.walk = None;
                None
            }
        }
    }

    /// Is a host-driven walk in progress?
    pub fn walking(&self) -> bool {
        self.walk.is_some()
    }

    fn slot(&mut self, source: u32) -> &mut Option<MenuBlock> {
        if let Some(i) = self.last.iter().position(|(s, _)| *s == source) {
            return &mut self.last[i].1;
        }
        self.last.push((source, None));
        &mut self.last.last_mut().expect("just pushed").1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Planetfall's InvisiClues menu, measured from the real game.
    fn planetfall(selected: usize) -> String {
        let items = [
            "THE FEINSTEIN",
            "THE POD TRIP",
            "THE DORMITORY",
            "ADMIN/MECH AREA",
        ];
        let mut b = String::from("                               INVISICLUES (tm)\n");
        b.push_str(" N = Next                                              P = Previous\n");
        b.push_str(" RETURN = See hint                                 Q = Resume story\n");
        b.push('\n');
        for (i, item) in items.iter().enumerate() {
            b.push_str(if i + 1 == selected { " > " } else { "   " });
            b.push_str(item);
            b.push('\n');
        }
        b
    }

    #[test]
    fn a_real_menu_parses_into_items_and_furniture() {
        let m = MenuBlock::parse(&planetfall(1)).expect("parses");
        assert_eq!(m.count(), 4, "four items, not seven lines");
        assert_eq!(m.current(), 1);
        assert_eq!(m.item(2), Some("THE POD TRIP"));
        // The title is centred at column 31 and the legends start at column 1;
        // neither shares the items' column, so neither is numbered.
        assert!(m.listing().contains("N = Next"), "the legend survives unnumbered");
        assert!(!m.listing().contains("1. N = Next"), "and is never given an ordinal");
        assert!(m.listing().contains(">1. THE FEINSTEIN"));
        assert!(m.listing().contains(" 2. THE POD TRIP"));
    }

    #[test]
    fn a_marker_that_moves_is_navigation_and_nothing_else_is() {
        let one = MenuBlock::parse(&planetfall(1)).unwrap();
        let two = MenuBlock::parse(&planetfall(2)).unwrap();
        assert!(two.is_navigation_from(&one));
        assert!(one.is_navigation_from(&two), "and back up again");
        assert!(!one.is_navigation_from(&one), "standing still is not a move");

        // Content: the same marker position, different text. This is the case
        // that must NOT be swallowed — a menu that scrolled, or a second page.
        let scrolled = planetfall(2).replace("THE DORMITORY", "THE PLATFORM");
        let scrolled = MenuBlock::parse(&scrolled).unwrap();
        assert!(!scrolled.is_navigation_from(&one), "an item changed: that is content");

        // Furniture: the legend changed, the marker did too. Still content.
        let relegended = planetfall(2).replace("Q = Resume story", "Q = Quit");
        let relegended = MenuBlock::parse(&relegended).unwrap();
        assert!(!relegended.is_navigation_from(&one), "the legend changed: that is content");
    }

    #[test]
    fn the_announcement_names_the_item_and_its_place() {
        let m = MenuBlock::parse(&planetfall(3)).unwrap();
        assert_eq!(m.announcement(), ">3. THE DORMITORY (3 of 4)");
    }

    #[test]
    fn things_that_are_not_menus_do_not_parse() {
        // A v3 status line: one line, no marker.
        assert!(MenuBlock::parse("West of House          Score: 0  Moves: 1").is_none());
        // Counterfeit Monkey's status grid during ordinary play.
        assert!(MenuBlock::parse("Back Alley, noon\nGoals: 1\nScore: 0").is_none());
        // Prose that happens to contain a marker character.
        assert!(MenuBlock::parse("You see a sign.\n> a cross-reference\nIt is blank.").is_none());
        // A marker but only one item at its column: not a list.
        assert!(MenuBlock::parse("Title\n > Only one\nfooter").is_none());
        // Markers at two different columns: a page of bullets, not a cursor.
        assert!(MenuBlock::parse(" > One\n   Two\n     > Three").is_none());
        // A run broken by a line that is neither an item nor blank.
        assert!(MenuBlock::parse(" > One\nx\n   Two").is_none());
    }

    #[test]
    fn a_list_the_game_repainted_twice_counts_once() {
        // Counterfeit Monkey redraws its whole item list on every keypress, so
        // the block a listener would hear carries fourteen lines and seven items.
        let once = " > Alpha\n   Beta\n   Gamma\n";
        let twice = format!("{once}{once}");
        let m = MenuBlock::parse(&twice).expect("parses");
        assert_eq!(m.count(), 3, "three items, not six");
        assert_eq!(m.current(), 1);
        assert_eq!(m.listing().matches("Alpha").count(), 1, "listed once");
        assert_eq!(m.announcement(), ">1. Alpha (1 of 3)");

        // Both copies must agree about where the marker is, or they are not
        // copies and the block is something this module has no business folding.
        assert!(MenuBlock::parse(" > Alpha\n   Beta\n   Alpha\n > Beta\n").is_none());
    }

    #[test]
    fn a_legend_names_the_keys_the_menu_listens_for() {
        assert_eq!(
            nav_keys_from_legend(" N = next item    P = previous item"),
            (NavKey::Char('N'), NavKey::Char('P'))
        );
        assert_eq!(
            nav_keys_from_legend("N = Next\nP = Previous\nQ = Quit Menu\nENTER = Select"),
            (NavKey::Char('N'), NavKey::Char('P'))
        );
        // No legend at all: the cursor keys are the only remaining guess.
        assert_eq!(nav_keys_from_legend(" > One\n   Two"), (NavKey::Down, NavKey::Up));
        // Half a legend is not a legend.
        assert_eq!(nav_keys_from_legend("N = next item"), (NavKey::Down, NavKey::Up));
        // `ENTER = next` names Enter, not R — a multi-letter token is not a key
        // this module will press.
        assert_eq!(
            nav_keys_from_legend("ENTER = next page   BACK = previous page"),
            (NavKey::Down, NavKey::Up)
        );
    }

    #[test]
    fn the_tracker_reads_a_menu_out_once_then_announces_moves() {
        let mut t = MenuTracker::new();
        match t.observe(0, &planetfall(1)) {
            Emission::Block(b) => {
                // The game's own preamble, then the how-to line, then numbers.
                assert!(b.starts_with("                               INVISICLUES"));
                assert!(b.contains(&format!("\n{MENU_HINT}\n>1. THE FEINSTEIN")), "{b:?}");
            }
            other => panic!("a menu opening is read out in full: {other:?}"),
        }
        assert_eq!(
            t.observe(0, &planetfall(2)),
            Emission::Line(">2. THE POD TRIP (2 of 4)".into()),
            "a marker move is one line"
        );
        assert_eq!(t.observe(0, &planetfall(3)), Emission::Line(">3. THE DORMITORY (3 of 4)".into()));
    }

    #[test]
    fn a_menu_that_closes_gives_the_source_back() {
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        assert!(t.active().is_some());
        let status = "Deck Nine                    Score: 0";
        assert_eq!(t.observe(0, status), Emission::Block(status.to_string()), "verbatim");
        assert!(t.active().is_none(), "no menu to re-list");
        assert!(t.listing().is_none());
        // ...and re-opening reads out in full again rather than announcing a
        // move relative to a menu that is no longer on screen.
        assert!(matches!(t.observe(0, &planetfall(2)), Emission::Block(_)));
    }

    #[test]
    fn one_source_menu_does_not_silence_another() {
        // gvm-cli has as many sources as the game has grid windows, plus the
        // story stream. A status bar updating while a menu is open is content.
        let mut t = MenuTracker::new();
        t.observe(7, &planetfall(1));
        assert_eq!(
            t.observe(2, "Back Alley  Score: 3"),
            Emission::Block("Back Alley  Score: 3".into())
        );
        assert!(t.active().is_some(), "another source's status does not close the menu");
        assert_eq!(t.observe(7, &planetfall(2)), Emission::Line(">2. THE POD TRIP (2 of 4)".into()));
    }

    #[test]
    fn a_number_jump_walks_the_menu_with_its_own_keys() {
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        assert!(t.plan_jump(3, &planetfall(1)), "1 -> 3 is a jump");
        assert!(t.walking());
        // The menu's own `N`, not Down — its legend says so.
        assert_eq!(t.next_key(), Some(NavKey::Char('N')));
        assert_eq!(t.observe(0, &planetfall(2)), Emission::Nothing, "silent in transit");
        assert_eq!(t.next_key(), Some(NavKey::Char('N')));
        assert_eq!(
            t.observe(0, &planetfall(3)),
            Emission::Line(">3. THE DORMITORY (3 of 4)".into()),
            "only the landing is announced"
        );
        assert_eq!(t.next_key(), None, "the walk is over");
    }

    #[test]
    fn a_step_that_skips_a_line_does_not_overshoot() {
        // Arthur's `N` steps over its unselectable section headings. A walk that
        // counted presses up front would sail past the item the player named;
        // steering by where the marker actually landed does not.
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        assert!(t.plan_jump(3, &planetfall(1)));
        assert_eq!(t.next_key(), Some(NavKey::Char('N')));
        // One press, two items.
        assert_eq!(t.observe(0, &planetfall(3)), Emission::Line(">3. THE DORMITORY (3 of 4)".into()));
        assert_eq!(t.next_key(), None, "arrived: no second press");
        assert!(!t.walking());
    }

    #[test]
    fn a_backwards_jump_presses_the_previous_key() {
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(3));
        assert!(t.plan_jump(1, &planetfall(3)));
        assert_eq!(t.next_key(), Some(NavKey::Char('P')));
        t.observe(0, &planetfall(2));
        assert_eq!(t.next_key(), Some(NavKey::Char('P')), "still above the goal");
        t.observe(0, &planetfall(1));
        assert_eq!(t.next_key(), None);
    }

    #[test]
    fn a_walk_that_can_never_arrive_gives_up_rather_than_pressing_forever() {
        // A menu that wraps: `N` at the bottom returns to the top, so an ordinal
        // the marker never stops on would be chased round and round.
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        t.plan_jump(3, "");
        let mut presses = 0;
        // Feed it a marker that only ever alternates 1 <-> 2, never reaching 3.
        while t.next_key().is_some() {
            presses += 1;
            assert!(presses < 100, "the budget must stop this");
            t.observe(0, &planetfall(if presses % 2 == 0 { 1 } else { 2 }));
        }
        assert!(!t.walking());
        assert_eq!(presses, 2 * 4 + 4, "bounded by the budget, then it stops");
    }

    #[test]
    fn a_legendless_menu_falls_back_to_the_cursor_keys() {
        let bare = " > Alpha\n   Beta\n   Gamma\n";
        let mut t = MenuTracker::new();
        t.observe(0, bare);
        assert!(t.plan_jump(2, bare));
        assert_eq!(t.next_key(), Some(NavKey::Down));
    }

    #[test]
    fn a_menu_that_changes_under_a_walk_cancels_it() {
        // It scrolled, or a submenu opened: an ordinal from the old block names
        // nothing in the new one.
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        t.plan_jump(4, "");
        assert!(t.walking());
        let elsewhere = " > Hint 1\n   Hint 2\n   Hint 3\n";
        assert!(matches!(t.observe(0, elsewhere), Emission::Block(_)));
        assert!(!t.walking(), "the walk cannot steer by an ordinal that moved");
    }

    #[test]
    fn a_jump_that_cannot_move_queues_nothing() {
        let mut t = MenuTracker::new();
        assert!(!t.plan_jump(2, ""), "no menu open");
        t.observe(0, &planetfall(2));
        assert!(!t.plan_jump(2, ""), "already there");
        assert!(!t.plan_jump(0, ""), "there is no item zero");
        assert!(!t.plan_jump(9, ""), "past the end");
        assert!(!t.walking());
    }

    #[test]
    fn a_walk_that_does_not_move_the_marker_still_lands() {
        // `n` at the bottom of a list that does not wrap: the game redraws the
        // same block, the host's own dedupe swallows it, and without this the
        // player who typed a number hears nothing at all.
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        t.plan_jump(2, "");
        t.next_key();
        assert_eq!(
            t.unchanged(0),
            Emission::Line(">1. THE FEINSTEIN (1 of 4)".into()),
            "the landing is reported even when it is where we started"
        );
        assert_eq!(t.unchanged(0), Emission::Nothing, "and only once");
        assert!(!t.walking(), "no progress means no more presses");
    }

    #[test]
    fn silence_stays_silence_outside_a_walk() {
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(1));
        assert_eq!(t.unchanged(0), Emission::Nothing, "an unchanged menu says nothing");
    }

    #[test]
    fn a_typed_number_is_a_jump_only_when_a_menu_can_answer_it() {
        let mut t = MenuTracker::new();
        // No menu: every digit belongs to the game — plenty of menus, and plenty
        // of ordinary prompts, select by number themselves.
        assert_eq!(t.typed("3", ""), Typed::Passthrough);

        t.observe(0, &planetfall(1));
        assert_eq!(t.typed("3", &planetfall(1)), Typed::Jump);
        assert!(t.walking());
        assert_eq!(t.next_key(), Some(NavKey::Char('N')));
        assert_eq!(t.next_key(), Some(NavKey::Char('N')));

        // Out of range, not a number, empty: all the game's business.
        for line in ["0", "9", "n", "", "3 ", "3a", "-1", "/menu"] {
            let mut t = MenuTracker::new();
            t.observe(0, &planetfall(1));
            let got = t.typed(line, "");
            if line == "3 " {
                assert_eq!(got, Typed::Jump, "surrounding blanks are trimmed");
            } else {
                assert_eq!(got, Typed::Passthrough, "{line:?} must reach the game");
            }
        }
    }

    #[test]
    fn typing_the_number_you_are_already_on_repeats_it_rather_than_typing_a_digit() {
        let mut t = MenuTracker::new();
        t.observe(0, &planetfall(2));
        assert_eq!(t.typed("2", ""), Typed::Here(">2. THE POD TRIP (2 of 4)".into()));
        assert!(!t.walking(), "nothing to walk");
    }

    #[test]
    fn a_multi_digit_jump_is_read_as_one_number() {
        // Cooked input is what makes this possible: the line ends at Enter, so
        // `12` is item twelve and not item 1 followed by item 2.
        let long = |sel: usize| {
            (1..=12).fold(String::new(), |mut s, i| {
                s.push_str(if i == sel { " > " } else { "   " });
                s.push_str(&format!("CHAPTER {i}\n"));
                s
            })
        };
        let mut t = MenuTracker::new();
        t.observe(0, &long(1));
        assert_eq!(t.typed("12", ""), Typed::Jump, "twelve, not one then two");
        let mut presses = 0;
        while let Some(key) = t.next_key() {
            assert_eq!(key, NavKey::Down);
            presses += 1;
            t.observe(0, &long(1 + presses));
        }
        assert_eq!(presses, 11, "one press per item between here and there");
        assert_eq!(t.active().map(MenuBlock::current), Some(12));
    }

    #[test]
    fn menu_request_is_recognised_leniently_but_never_greedily() {
        assert!(is_menu_request("/menu"));
        assert!(is_menu_request("  /MENU \n"));
        for line in ["menu", "/menus", "/menu 3", "x /menu", "/", ""] {
            assert!(!is_menu_request(line), "{line:?} must reach the game");
        }
    }

    #[test]
    fn the_period_of_a_list_is_the_shortest_one() {
        assert_eq!(smallest_period(&["a", "b", "a", "b"]), 2);
        assert_eq!(smallest_period(&["a", "a", "a"]), 1);
        assert_eq!(smallest_period(&["a", "b", "c"]), 3);
        assert_eq!(smallest_period::<&str>(&[]), 0);
    }
}
