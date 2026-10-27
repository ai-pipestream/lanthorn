//! What a listener should hear on a screen, for embedding hosts (SQ-1765).
//!
//! A screen reader cannot read every window the same way. A status band is
//! chrome: the host quietens it, or speaks only what changed. The story window
//! is content. A hint menu is a list with a cursor. A picture is nothing to read
//! at all. [`regions`] answers that question once, renderer-neutrally (no
//! ratatui types), so each host does not re-derive it from the window tree.
//!
//! ## Two layers
//!
//! 1. **A generic rule.** A short text window at the top of the screen is
//!    [`RegionRole::Chrome`]; any other text window and the story's buffer
//!    window are [`RegionRole::Content`]; graphics windows contribute nothing.
//!    For the ordinary `Pair { Grid, Buffer }` screen this is the CLIs' rule
//!    (`zvm-cli`'s `screen.rs`: a one-row upper window is status chrome, taller
//!    is content). It fits no Version 6 screen, whose window numbers mean
//!    different things in different games.
//! 2. **Per-release overrides** ([`RELEASES`]) for the Infocom Version 6 titles
//!    whose windows were read out of a running copy. They are keyed on the
//!    story's **release and serial**, never on the title: a disk image is a
//!    different release (`docs/internals/interpreter.md`, "A floppy is a
//!    different release"), and its windows are not assumed to be the bare
//!    file's. A release not in the table gets the generic rule, which is also
//!    what *Journey* gets (SQ-1770).
//!
//! On top of either, a text region whose lines parse as a menu
//! ([`cli_host::MenuBlock::parse`]) is promoted to [`RegionRole::Menu`]; its
//! `text` is then exactly the block that parsed, so the host can call
//! `MenuBlock::parse(&region.text.join("\n"))` and use `MenuBlock::parts()`.
//! A menu the game marks with reverse video rather than a `>` (Zork Zero's
//! InvisiClues) gets a two-column marker margin: every line is prefixed with
//! `"> "` (the highlighted row) or `"  "` so the cursor survives as text.
//!
//! ## Score
//!
//! [`score`] backs `Engine::score` for Z-code: v1-v3 read the standard status
//! globals (ZMSD 8.2.3.1), an override release reads the global its own status
//! band prints, and an unknown Version 6 release falls back to parsing a
//! `Score:` number out of the region classified Chrome. Everything else is
//! `None`.

use blorb::GameIdentifier;
use cli_host::MenuBlock;
use zvm::cpu::exec::Machine;

use crate::engine::{BufferWindow, GridWindow, PositionedWindow, ScreenModel, WinNode};

/// What a host should do with a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionRole {
    /// Status furniture the game repaints every turn: location, score, moves.
    Chrome,
    /// Text the story narrates into, or a text panel it prints.
    Content,
    /// A list with a cursor; see the module docs for the text shape.
    Menu,
}

/// One readable region of the screen.
///
/// `rect` is `(x, y, w, h)` in terminal cells, the same cell rect
/// [`PositionedWindow`] carries; a plain tuple rather than a ratatui `Rect` so
/// the host API stays renderer-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub role: RegionRole,
    /// The Version 6 window number, `None` outside v6.
    pub window: Option<u8>,
    pub rect: (u16, u16, u16, u16),
    /// The region's lines, trailing blanks trimmed. For the story's primary
    /// buffer this is the page as the game painted it; outside Version 6 the
    /// primary buffer's text is the transcript the host already receives each
    /// turn, so it is empty here.
    pub text: Vec<String>,
}

/// Where a release keeps its score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScoreAt {
    /// The game's own status-band paint routine prints `global(index) * scale`
    /// beside `Score:`.
    Global { index: u8, scale: i32 },
    /// The game has no single score (the band prints none).
    None,
}

/// One Version 6 release whose windows and score were established on a running
/// copy. See `docs/internals/architecture.md` for how each row was found.
struct ReleaseRule {
    release: u16,
    serial: &'static str,
    /// Windows that are status chrome.
    chrome: &'static [u8],
    /// Windows that are story content.
    content: &'static [u8],
    /// Frames, poles and picture panels: never read.
    silent: &'static [u8],
    score: ScoreAt,
}

/// Overrides, keyed on release and serial. Three bare story files; the disk
/// images of the same titles are different releases and are NOT listed because
/// their window numbers and score globals were not established.
const RELEASES: &[ReleaseRule] = &[
    // Zork Zero r393 / 890714. Window 1 is the four-row top band (location, Moves,
    // Score) and, in the hint menu, the InvisiClues legend. Window 0 is the story
    // buffer and, with its wrapping cleared, the hint menu's topic grid. Window 7
    // is the full-screen frame. Score: global 0x61 (97), found in the band paint
    // routine at 0x00eb60 (`je g61, g79 ?..; call_1n 0x01bca0`, which does
    // `print_num g61` after `Score:`); g79 is the cache that gates the repaint.
    ReleaseRule {
        release: 393,
        serial: "890714",
        chrome: &[1],
        content: &[0],
        silent: &[7],
        score: ScoreAt::Global { index: 0x61, scale: 1 },
    },
    // Shogun r322 / 890706. Window 1 is the two-row band (Erasmus/SHOGUN/Score,
    // location/Moves). Window 0 is the story buffer (and the title card's grid).
    // Window 2 is the three-row panel the START/RESTORE choice is painted in.
    // Window 7 is the frame. Score: the band routine at 0x01232e prints
    // `mul g66, #05 -> local2; call_vn 0x012594, local2, #01` after `Score:`, i.e.
    // global 0x66 (102) times five, with g6f the repaint cache.
    ReleaseRule {
        release: 322,
        serial: "890706",
        chrome: &[1],
        content: &[0, 2],
        silent: &[7],
        score: ScoreAt::Global { index: 0x66, scale: 5 },
    },
    // Arthur r74 / 890714. The one-row band (location and date) sits BELOW the
    // picture panel at row 12, so no top-of-screen rule finds it: window 1. Window 0
    // is the story buffer; window 3 is the one-row message line at the bottom the
    // game prints its replies into. Window 2 is the picture panel, 5 and 6 the
    // poles, 7 the frame. Arthur keeps four point categories (chivalry, wisdom,
    // experience, quest) and prints none of them on the band: no score.
    ReleaseRule {
        release: 74,
        serial: "890714",
        chrome: &[1],
        content: &[0, 3],
        silent: &[2, 5, 6, 7],
        score: ScoreAt::None,
    },
];

fn rule_for(id: &GameIdentifier) -> Option<&'static ReleaseRule> {
    RELEASES
        .iter()
        .find(|r| r.release == id.release && id.serial == *r.serial.as_bytes())
}

/// The identity a running Z-machine states in its own header (release, serial
/// and checksum), normalised the way [`GameIdentifier::of_story`] does.
pub fn identify(machine: &Machine) -> GameIdentifier {
    let mut serial = [0u8; 6];
    for (i, b) in serial.iter_mut().enumerate() {
        *b = machine.mem.read_byte(0x12 + i as u32) & 0x7f;
    }
    GameIdentifier {
        release: machine.mem.read_word(0x02),
        serial,
        checksum: machine.mem.read_word(0x1c),
    }
}

/// Every readable region of `screen`, top to bottom and then left to right.
///
/// `release` is the story's identity (`GameSession::game_identifier`, or
/// [`GameIdentifier::of_story`]); `None` gets the generic rule everywhere.
/// Graphics windows and windows with nothing to read are left out.
pub fn regions(screen: &ScreenModel, release: Option<&GameIdentifier>) -> Vec<Region> {
    let rule = release.and_then(rule_for);
    let mut out = Vec::new();
    match &screen.root {
        WinNode::Layered(items) => {
            for pw in items {
                layered_region(pw, screen, rule, &mut out);
            }
        }
        other => {
            let mut y = 0u16;
            plain_regions(other, screen, &mut y, &mut out);
        }
    }
    // Stable: windows sharing a row keep their list order.
    out.sort_by_key(|r| (r.rect.1, r.rect.0));
    out
}

/// The CLIs' rule, shared by every screen that is not a v6 composite: a
/// one-row grid is status chrome, a taller one is content.
fn plain_grid_role(rows: u16) -> RegionRole {
    if rows <= 1 { RegionRole::Chrome } else { RegionRole::Content }
}

fn plain_regions(node: &WinNode, screen: &ScreenModel, y: &mut u16, out: &mut Vec<Region>) {
    match node {
        WinNode::Pair { first, second, vertical, .. } => {
            plain_regions(first, screen, y, out);
            if !*vertical {
                *y = 0;
            }
            plain_regions(second, screen, y, out);
        }
        WinNode::Grid(g) => {
            let rows = g.active_rows.min(g.rows);
            let text = grid_rows(g);
            let at = *y;
            *y = y.saturating_add(rows);
            if !text.is_empty() {
                push_text_region(out, plain_grid_role(rows), None, (0, at, g.cols, rows), text);
            }
        }
        WinNode::Buffer(b) => {
            let text = buffer_lines(b);
            if b.primary || !text.is_empty() {
                let h = screen.content_size.1.saturating_sub(*y);
                push_text_region(out, RegionRole::Content, None, (0, *y, screen.content_size.0, h), plain_rows(text));
            }
        }
        WinNode::Layered(_) | WinNode::Graphics(_) | WinNode::Blank => {}
    }
}

fn layered_region(pw: &PositionedWindow, screen: &ScreenModel, rule: Option<&ReleaseRule>, out: &mut Vec<Region>) {
    let rect = (pw.x, pw.y, pw.w, pw.h);
    let forced = pw.window.and_then(|w| {
        let r = rule?;
        if r.silent.contains(&w) {
            Some(None)
        } else if r.chrome.contains(&w) {
            Some(Some(RegionRole::Chrome))
        } else if r.content.contains(&w) {
            Some(Some(RegionRole::Content))
        } else {
            None
        }
    });
    if forced == Some(None) {
        return;
    }
    match &pw.node {
        WinNode::Grid(g) => {
            let text = grid_rows(g);
            if text.is_empty() {
                return;
            }
            let role = match forced {
                Some(Some(role)) => role,
                _ => generic_layered_role(pw, screen),
            };
            push_text_region(out, role, pw.window, rect, text);
        }
        WinNode::Buffer(b) => {
            let text = buffer_lines(b);
            if b.primary || !text.is_empty() {
                let role = match forced {
                    Some(Some(role)) => role,
                    _ => RegionRole::Content,
                };
                push_text_region(out, role, pw.window, rect, plain_rows(text));
            }
        }
        WinNode::Graphics(_) | WinNode::Blank | WinNode::Pair { .. } | WinNode::Layered(_) => {}
    }
}

/// The generic Version 6 rule: a short text window at the top of the screen is
/// chrome, every other text window is content.
fn generic_layered_role(pw: &PositionedWindow, screen: &ScreenModel) -> RegionRole {
    let screen_rows = screen.content_size.1.max(1);
    let at_top = u32::from(pw.y) * 4 < u32::from(screen_rows);
    if at_top && pw.h <= 4 { RegionRole::Chrome } else { RegionRole::Content }
}

/// One non-blank line of a region, and whether the game painted its first
/// visible character in reverse video (a menu's cursor, when it has no marker).
struct Row {
    text: String,
    reverse: bool,
}

fn plain_rows(lines: Vec<String>) -> Vec<Row> {
    lines.into_iter().map(|text| Row { text, reverse: false }).collect()
}

/// Push a text region, promoting it to a Menu when its lines parse as one.
fn push_text_region(
    out: &mut Vec<Region>,
    role: RegionRole,
    window: Option<u8>,
    rect: (u16, u16, u16, u16),
    rows: Vec<Row>,
) {
    let (role, text) = match menu_text(&rows) {
        Some(menu) => (RegionRole::Menu, menu),
        None => (role, rows.into_iter().map(|r| r.text).collect()),
    };
    out.push(Region { role, window, rect, text });
}

/// The text a menu region carries, or `None` when `rows` is not a menu.
///
/// Tried plain first (a game that draws its own `>`), then, when exactly one row
/// is in reverse video (Zork Zero's hint menu highlights the current topic that
/// way), with a marker margin added so the cursor is text.
fn menu_text(rows: &[Row]) -> Option<Vec<String>> {
    if rows.len() < 2 {
        return None;
    }
    let plain: Vec<String> = rows.iter().map(|r| r.text.clone()).collect();
    if MenuBlock::parse(&plain.join("\n")).is_some() {
        return Some(plain);
    }
    if rows.iter().filter(|r| r.reverse).count() != 1 {
        return None;
    }
    let marked: Vec<String> = rows
        .iter()
        .map(|r| format!("{}{}", if r.reverse { "> " } else { "  " }, r.text))
        .collect();
    MenuBlock::parse(&marked.join("\n")).map(|_| marked)
}

/// A grid's non-blank rows. A Version 6 grid carries the game's pixel paint list,
/// whose runs know the character cell they began at; that is the exact page,
/// where the cell mirror can miss text painted at pixel positions (Shogun's
/// START/RESTORE panel). Anything else reads its cells.
fn grid_rows(g: &GridWindow) -> Vec<Row> {
    let mut by_row: std::collections::BTreeMap<u16, Vec<(u16, char, bool)>> = Default::default();
    if g.px_texts.is_empty() {
        for r in 0..g.rows {
            let start = usize::from(r) * usize::from(g.cols);
            for (c, cell) in g.cells[start..start + usize::from(g.cols)].iter().enumerate() {
                by_row.entry(r).or_default().push((c as u16, cell.ch, cell.style & 1 != 0));
            }
        }
    } else {
        for run in &g.px_texts {
            let row = by_row.entry(run.grow).or_default();
            for (i, ch) in run.text.chars().enumerate() {
                let col = run.gcol.saturating_add(i as u16);
                row.retain(|(c, _, _)| *c != col);
                row.push((col, ch, run.style & 1 != 0));
            }
        }
    }
    by_row
        .into_values()
        .filter_map(|mut cells| {
            cells.sort_by_key(|(c, _, _)| *c);
            let end = cells.last()?.0;
            let line: String = (0..=end)
                .map(|c| cells.iter().find(|(cc, _, _)| *cc == c).map_or(' ', |(_, ch, _)| *ch))
                .collect();
            let text = line.trim_end().to_string();
            if text.trim().is_empty() {
                return None;
            }
            let reverse = cells.iter().find(|(_, ch, _)| *ch != ' ').is_some_and(|(_, _, rv)| *rv);
            Some(Row { text, reverse })
        })
        .collect()
}

/// A buffer window's lines: its own `lines` when it has them, otherwise the page
/// its pixel runs paint (the Version 6 primary buffer).
///
/// That paint list shadows every glyph the game printed and never erases (see
/// `ZWindow::record_streamed`), so a short line printed over a long one leaves
/// the long one's tail behind. The newest run on a row is therefore trusted, and
/// older runs only where they touch it from the left; a stale run to its right
/// is dropped.
fn buffer_lines(b: &BufferWindow) -> Vec<String> {
    if !b.lines.is_empty() {
        return b.lines.iter().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect();
    }
    let mut by_row: std::collections::BTreeMap<u16, Vec<(u16, &str)>> = Default::default();
    for run in &b.px_runs {
        by_row.entry(run.grow).or_default().push((run.gcol, run.text.as_str()));
    }
    by_row
        .into_values()
        .filter_map(|runs| {
            let mut cells: std::collections::BTreeMap<u16, char> = Default::default();
            let mut left_edge = u16::MAX;
            for &(start, text) in runs.iter().rev() {
                let end = start.saturating_add(text.chars().count() as u16);
                if left_edge != u16::MAX && start >= left_edge {
                    continue; // older than the newest run and to its right: stale
                }
                if left_edge != u16::MAX && end < left_edge {
                    break; // a gap: not the same line any more
                }
                for (i, ch) in text.chars().enumerate() {
                    let col = start.saturating_add(i as u16);
                    if col < left_edge {
                        cells.entry(col).or_insert(ch);
                    }
                }
                left_edge = left_edge.min(start);
            }
            let last = *cells.keys().next_back()?;
            let first = *cells.keys().next()?;
            let line: String = (first..=last).map(|c| cells.get(&c).copied().unwrap_or(' ')).collect();
            let line = line.trim().to_string();
            (!line.is_empty()).then_some(line)
        })
        .collect()
}

/// The engine-neutral score of a Z-code session, for `Engine::score`.
///
/// * Versions 1-3: the second global is the score (ZMSD 8.2.3.1), `None` for a
///   time game (3: `Flags 1` bit 1 set).
/// * Version 6, an [`RELEASES`] release: the global its own band prints.
/// * Version 6, any other release: the `Score:` number in the region classified
///   Chrome. **This is a fallback**, read off the screen the game painted rather
///   than off its state.
/// * Everything else: `None`.
///
/// `screen` is only called for the fallback.
pub fn score(machine: &Machine, release: &GameIdentifier, screen: impl FnOnce() -> ScreenModel) -> Option<i32> {
    match machine.mem.version() {
        1..=3 => match machine.status_line().right {
            zvm::screen::StatusRight::ScoreTurns { score, .. } => Some(i32::from(score)),
            _ => None,
        },
        6 => match rule_for(release) {
            Some(rule) => match rule.score {
                ScoreAt::Global { index, scale } => Some(i32::from(machine.global(index) as i16) * scale),
                ScoreAt::None => None,
            },
            None => {
                let model = screen();
                regions(&model, Some(release))
                    .iter()
                    .filter(|r| r.role == RegionRole::Chrome)
                    .find_map(|r| r.text.iter().find_map(|l| score_in_text(l)))
            }
        },
        _ => None,
    }
}

/// The integer after `Score` (and an optional colon) in `line`, if any.
fn score_in_text(line: &str) -> Option<i32> {
    let at = line.find("Score")?;
    let rest = line[at + "Score".len()..].trim_start_matches([':', ' ']);
    let end = rest
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || (*i == 0 && *c == '-')))
        .map_or(rest.len(), |(i, _)| i);
    rest[..end].parse().ok()
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;
    use crate::engine::{GridCell, StatusModel};

    fn grid(rows: &[&str], reverse_row: Option<usize>) -> GridWindow {
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
        let mut cells = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let mut line: Vec<char> = r.chars().collect();
            line.resize(usize::from(cols), ' ');
            for ch in line {
                cells.push(GridCell { ch, style: u8::from(reverse_row == Some(i)), ..GridCell::default() });
            }
        }
        GridWindow {
            win: 0,
            cols,
            rows: rows.len() as u16,
            cells,
            active_rows: rows.len() as u16,
            cursor: (1, 1),
            cursor_active: false,
            border: crate::engine::BorderPref::Unspecified,
            bg: None,
            fg: None,
            reverse: false,
            fill: None,
            px_texts: Vec::new(),
        }
    }

    fn pw(window: Option<u8>, y: u16, h: u16, node: WinNode) -> PositionedWindow {
        PositionedWindow {
            window,
            x: 0,
            y,
            w: 80,
            h,
            x_px: 0,
            y_px: y * 16,
            w_px: 640,
            h_px: h * 16,
            left_margin: 0,
            right_margin: 0,
            node,
        }
    }

    fn model(root: WinNode) -> ScreenModel {
        ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (80, 25) }
    }

    fn id(release: u16, serial: &str) -> GameIdentifier {
        let mut s = [0u8; 6];
        s.copy_from_slice(serial.as_bytes());
        GameIdentifier { release, serial: s, checksum: 0 }
    }

    #[test]
    fn plain_pair_follows_the_cli_rule() {
        let one = model(WinNode::Pair {
            vertical: true,
            split: crate::engine::Split { fixed: 1, fixed_px: None, rest: None },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Grid(grid(&[" West of House   Score: 0"], None))),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let r = regions(&one, None);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].role, r[1].role), (RegionRole::Chrome, RegionRole::Content));
        assert!(r[1].text.is_empty(), "the primary buffer's text is the transcript, not ours");

        let tall = model(WinNode::Pair {
            vertical: true,
            split: crate::engine::Split { fixed: 3, fixed_px: None, rest: None },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Grid(grid(&["a", "b", "c"], None))),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        assert_eq!(regions(&tall, None)[0].role, RegionRole::Content);
    }

    #[test]
    fn generic_layered_rule_chrome_at_top_content_below_graphics_skipped() {
        let m = model(WinNode::Layered(vec![
            pw(Some(4), 0, 25, WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 0,
                canvas: std::sync::Arc::new(image::RgbaImage::new(1, 1)),
                version: 0,
                upscale: false,
                undrawn: false,
            })),
            pw(Some(1), 0, 2, WinNode::Grid(grid(&["Bridge   Score: 3", "Moves: 4"], None))),
            pw(Some(2), 10, 3, WinNode::Grid(grid(&["a panel", "of text"], None))),
        ]));
        let r = regions(&m, None);
        assert_eq!(r.len(), 2, "graphics contribute nothing: {r:?}");
        assert_eq!((r[0].role, r[0].window), (RegionRole::Chrome, Some(1)));
        assert_eq!((r[1].role, r[1].window), (RegionRole::Content, Some(2)));
    }

    #[test]
    fn override_is_keyed_on_release_and_serial() {
        // Arthur's band sits at row 12: the generic rule calls it content, the
        // r74/890714 override calls it chrome, and another release gets the generic.
        let m = model(WinNode::Layered(vec![pw(Some(1), 12, 1, WinNode::Grid(grid(&["Churchyard   St Anne's Day"], None)))]));
        assert_eq!(regions(&m, None)[0].role, RegionRole::Content);
        assert_eq!(regions(&m, Some(&id(74, "890714")))[0].role, RegionRole::Chrome);
        assert_eq!(regions(&m, Some(&id(54, "890606")))[0].role, RegionRole::Content, "the Amiga floppy is another release");
        assert_eq!(regions(&m, Some(&id(74, "890715")))[0].role, RegionRole::Content, "serial matters too");
    }

    #[test]
    fn a_marked_grid_is_a_menu_and_a_reverse_row_is_the_marker() {
        let plain = model(WinNode::Layered(vec![pw(Some(5), 3, 4, WinNode::Grid(grid(&["Pick one", "> first", "  second", "  third"], None)))]));
        let r = regions(&plain, None);
        assert_eq!(r[0].role, RegionRole::Menu);
        assert_eq!(MenuBlock::parse(&r[0].text.join("\n")).unwrap().count(), 3);

        let rev = model(WinNode::Layered(vec![pw(Some(5), 3, 3, WinNode::Grid(grid(&["alpha", "beta", "gamma"], Some(1))))]));
        let r = regions(&rev, None);
        assert_eq!(r[0].role, RegionRole::Menu);
        let block = MenuBlock::parse(&r[0].text.join("\n")).unwrap();
        assert_eq!((block.count(), block.current()), (3, 2));

        let band = model(WinNode::Layered(vec![pw(Some(1), 0, 2, WinNode::Grid(grid(&["Bridge   Score: 3", "Moves: 4"], None)))]));
        assert_eq!(regions(&band, None)[0].role, RegionRole::Chrome, "a band is not a menu");
    }

    #[test]
    fn score_text_parses_the_number_after_score() {
        assert_eq!(score_in_text("Moves:  4      Score:   15"), Some(15));
        assert_eq!(score_in_text("Score: -3/100"), Some(-3));
        assert_eq!(score_in_text("Score: none"), None);
        assert_eq!(score_in_text("no label 12"), None);
    }
}
