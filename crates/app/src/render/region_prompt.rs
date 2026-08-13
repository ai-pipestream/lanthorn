//! The region prompt (SQ-0439): the one modal that asks which rooms move, and where to.
//!
//! Three quite different questions come through here and they are all the same two questions —
//! *which rooms*, and *onto what layer* — so they share one piece of chrome:
//!
//! * the map's own suggestion that a set of rooms wants a layer of its own, whose options are the
//!   destinations and whose buttons are the three settled outcomes (separate now / ask again next
//!   crossing / never ask);
//! * a manual `move-region` where several passages lead into the selected room, whose options are
//!   those passages — including the case a direction cannot express at all, because two of them
//!   share one;
//! * a manual `move-region` whose rooms are settled and whose destination is not.
//!
//! Drawing takes only [`AppState`], so every string the player reads is composed against the graph
//! when the prompt opens (see `app::input::open_*`) and carried here. That is what keeps the modal
//! out of the mapper's way.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::{AppState, RegionOption, RegionPrompt, RegionPromptAct, RegionPromptKind};

/// Below this the modal cannot show its list and its buttons at once, so it does not draw.
const MIN_W: u16 = 34;
const MIN_H: u16 = 8;

/// Widest the modal grows, however long a layer name is. Room names are elided into it.
const MAX_W: u16 = 72;

// ── RegionPromptRects ─────────────────────────────────────────────────────────

pub struct RegionPromptRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect per option row, in `options` order. Clicking one chooses it.
    pub options: Vec<Rect>,
    /// The confirm button — `Separate` for a suggestion, `Move` for a pick.
    pub accept: Option<Rect>,
    /// Suggestion only: put it off, and ask again next crossing.
    pub later: Option<Rect>,
    /// Suggestion only: never ask about this passage again.
    pub never: Option<Rect>,
    /// Pick only: close without moving anything.
    pub cancel: Option<Rect>,
}

// ── Buttons ───────────────────────────────────────────────────────────────────

/// The buttons this prompt shows, left to right. A suggestion offers the three outcomes the
/// design settled on; a pick is an ordinary confirm/cancel because declining to pick decides
/// nothing and remembers nothing.
fn buttons_for(prompt: &RegionPrompt) -> &'static [DialogButton] {
    const SUGGEST: &[DialogButton] = &[
        DialogButton { id: ButtonId::Separate, label: "Separate" },
        DialogButton { id: ButtonId::Later, label: "Not now" },
        DialogButton { id: ButtonId::Never, label: "Never" },
    ];
    const PICK: &[DialogButton] = &[
        DialogButton { id: ButtonId::MoveRegion, label: "Move" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    match prompt.kind {
        RegionPromptKind::Suggest { .. } => SUGGEST,
        _ => PICK,
    }
}

/// The label of one option row, with its radio marker.
fn option_line(opt: &RegionOption, chosen: bool) -> String {
    let mark = if chosen { '•' } else { ' ' };
    let label = match opt {
        RegionOption::Dest { label, .. } | RegionOption::Seam { label, .. } => label,
    };
    format!("({mark}) {label}")
}

// ── draw_region_prompt ────────────────────────────────────────────────────────

/// Draw the region prompt centered over `area`, or `None` when it is closed or will not fit.
pub fn draw_region_prompt(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<RegionPromptRects> {
    let prompt = state.overlays.region_prompt.as_ref()?;

    // Content rows: the body lines, the room list, a blank, then one row per option. The button
    // row and the two border rows are on top of that.
    let rooms_rows = u16::from(!prompt.rooms.is_empty());
    let content_rows = prompt.body.len() as u16 + rooms_rows + 1 + prompt.options.len() as u16;
    let want_h = (content_rows + 3).max(MIN_H);
    let widest = prompt
        .body
        .iter()
        .map(|s| s.chars().count())
        .chain(prompt.options.iter().map(|o| option_line(o, true).chars().count()))
        .max()
        .unwrap_or(0) as u16;
    let want_w = (widest + 4).clamp(MIN_W, MAX_W);

    let modal_w = want_w.min(area.width.saturating_sub(4));
    let modal_h = want_h.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    let buttons = buttons_for(prompt);
    // The focus ring runs options first, then buttons, so only the tail of it selects a button.
    let button_focus = state.overlays.dialog_focus.checked_sub(prompt.options.len());
    let spec = DialogSpec {
        title: &prompt.title,
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(buttons[0].id),
        focus: button_focus,
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    let body_style = state.colors.theme.get("dialog.region_prompt.body").style;
    let rooms_style = state.colors.theme.get("dialog.region_prompt.rooms").style;
    let option_style = state.colors.theme.get("dialog.region_prompt.option").style;
    let chosen_style = state.colors.theme.get("dialog.region_prompt.option:chosen").style;

    let mut y = content.y;
    let put = |buf: &mut Buffer, text: &str, style, y: u16| {
        if y < content.bottom() {
            crate::render::draw_str_clipped(buf, content.x, y, text, style, content);
        }
    };
    for line in &prompt.body {
        put(buf, line, body_style, y);
        y += 1;
    }
    if !prompt.rooms.is_empty() {
        put(buf, &prompt.rooms, rooms_style, y);
        y += 1;
    }
    y += 1; // a blank row between what is being asked and the answers

    let mut option_rects = Vec::with_capacity(prompt.options.len());
    for (i, opt) in prompt.options.iter().enumerate() {
        let chosen = i == prompt.choice;
        let line = option_line(opt, chosen);
        let style = if chosen { chosen_style } else { option_style };
        put(buf, &line, style, y);
        let w = (line.chars().count() as u16).min(content.width);
        option_rects.push(if y < content.bottom() {
            Rect::new(content.x, y, w, 1)
        } else {
            Rect::new(content.x, content.y, 0, 0)
        });
        y += 1;
    }

    let find = |id: ButtonId| rects.buttons.iter().find(|(b, _)| *b == id).map(|(_, r)| *r);
    Some(RegionPromptRects {
        area: rects.area,
        close: rects.close,
        options: option_rects,
        accept: find(ButtonId::Separate).or_else(|| find(ButtonId::MoveRegion)),
        later: find(ButtonId::Later),
        never: find(ButtonId::Never),
        cancel: find(ButtonId::Cancel),
    })
}

// ── Keyboard routing ──────────────────────────────────────────────────────────

/// Decode one key press against the focus ring. `None` means the prompt swallowed it.
///
/// Tab / Shift-Tab (and the arrows) are the caller's, as in every other modal here; this is only
/// the part that depends on where the ring is resting. Enter on an OPTION confirms rather than
/// merely selecting, because focusing an option already selects it — there would be nothing left
/// for a second keystroke to do.
pub fn region_prompt_key_focused(
    code: crossterm::event::KeyCode,
    prompt: &RegionPrompt,
    focus: usize,
) -> Option<RegionPromptAct> {
    use crossterm::event::KeyCode;
    let suggest = matches!(prompt.kind, RegionPromptKind::Suggest { .. });
    match code {
        // Esc on a suggestion is not "no": it is "not now", which re-arms the seam. On a pick
        // there is nothing to remember, so it just closes.
        KeyCode::Esc => Some(if suggest { RegionPromptAct::Defer } else { RegionPromptAct::Dismiss }),
        KeyCode::Enter => match focus.checked_sub(prompt.options.len()) {
            None | Some(0) => Some(RegionPromptAct::Accept),
            Some(1) => Some(if suggest { RegionPromptAct::Defer } else { RegionPromptAct::Dismiss }),
            Some(2) if suggest => Some(RegionPromptAct::Never),
            _ => None,
        },
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegionPromptKind;
    use mapper::layer::{MoveTarget, Region, MAIN_LAYER};
    use mapper::suggest::{SeamKey, Trigger};
    use std::collections::BTreeSet;

    fn suggestion_prompt() -> RegionPrompt {
        RegionPrompt {
            kind: RegionPromptKind::Suggest {
                trigger: Trigger::Structural,
                seam: SeamKey { from: 1, dir: mapper::direction::Direction::Up },
                region: Region { anchor: 3, rooms: BTreeSet::from([3, 4, 5, 6]) },
            },
            title: "Give these rooms a layer?".to_string(),
            body: vec!["Four rooms sit behind a portal.".to_string()],
            rooms: "Cellar, Wine Cellar, Vault, Crypt".to_string(),
            options: vec![
                RegionOption::Dest { label: "a new layer".to_string(), target: MoveTarget::New },
                RegionOption::Dest {
                    label: "Main".to_string(),
                    target: MoveTarget::Existing(MAIN_LAYER),
                },
            ],
            choice: 0,
        }
    }

    fn pick_prompt() -> RegionPrompt {
        RegionPrompt {
            kind: RegionPromptKind::PickDest {
                region: Region { anchor: 3, rooms: BTreeSet::from([3, 4]) },
                cut: None,
            },
            title: "Where do these rooms go?".to_string(),
            body: vec!["Two rooms could go to either layer.".to_string()],
            rooms: String::new(),
            options: vec![
                RegionOption::Dest { label: "a new layer".to_string(), target: MoveTarget::New }
            ],
            choice: 0,
        }
    }

    /// The modal draws its title, its body, its room list and one marked radio row per option,
    /// and hands back a hit-rect for each of them plus the three outcome buttons.
    #[test]
    fn suggestion_renders_options_and_three_outcomes() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.region_prompt = Some(suggestion_prompt());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("an open prompt draws");
        assert_eq!(r.options.len(), 2, "one hit-rect per option");
        assert!(r.accept.is_some() && r.later.is_some() && r.never.is_some());
        assert!(r.cancel.is_none(), "a suggestion has no Cancel — every outcome is a decision");
        let all: String =
            terminal.backend().buffer().content().iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Give these rooms a layer?"), "title");
        assert!(all.contains("Four rooms sit behind a portal."), "body");
        assert!(all.contains("Cellar, Wine Cellar, Vault, Crypt"), "the rooms that would move");
        assert!(all.contains("(•) a new layer"), "the chosen option is marked");
        assert!(all.contains("( ) Main"), "the unchosen one is not");
        assert!(all.contains("Separate") && all.contains("Not now") && all.contains("Never"));
    }

    /// A pick is an ordinary confirm/cancel: Move and Cancel, no memory buttons.
    #[test]
    fn a_pick_offers_move_and_cancel() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.region_prompt = Some(pick_prompt());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("an open prompt draws");
        assert!(r.accept.is_some() && r.cancel.is_some());
        assert!(r.later.is_none() && r.never.is_none(), "nothing is remembered about a pick");
    }

    /// Esc means "not now" on a suggestion — the answer that re-arms — and a plain close on a
    /// pick. Enter confirms from an option row and from the first button; the second and third
    /// buttons carry the other two outcomes.
    #[test]
    fn keys_map_to_the_three_outcomes() {
        use crossterm::event::KeyCode;
        let s = suggestion_prompt();
        assert_eq!(region_prompt_key_focused(KeyCode::Esc, &s, 0), Some(RegionPromptAct::Defer));
        // Focus 0 and 1 are the two options; 2/3/4 are Separate / Not now / Never.
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 1), Some(RegionPromptAct::Accept));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 2), Some(RegionPromptAct::Accept));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 3), Some(RegionPromptAct::Defer));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 4), Some(RegionPromptAct::Never));
        // Space stays widget-reserved: it decides nothing here.
        assert_eq!(region_prompt_key_focused(KeyCode::Char(' '), &s, 0), None);

        let p = pick_prompt();
        assert_eq!(region_prompt_key_focused(KeyCode::Esc, &p, 0), Some(RegionPromptAct::Dismiss));
        // Focus 0 is the lone option; 1 = Move, 2 = Cancel.
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &p, 1), Some(RegionPromptAct::Accept));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &p, 2), Some(RegionPromptAct::Dismiss));
    }

    /// Both option styles are themed, not hard-coded: overriding the two selectors repaints the
    /// chosen and unchosen rows independently.
    #[test]
    fn option_rows_take_their_style_from_style_toml() {
        use ratatui::style::Color;
        use ratatui::{backend::TestBackend, Terminal};
        for honor in [true, false] {
            let mut state = AppState::default();
            state.config.honor_game_colours = honor;
            let parsed = crate::theme::toml_schema::parse(
                "[dialog]\n\
                 \"region_prompt.option\" = { fg = \"blue\" }\n\
                 \"region_prompt.option:chosen\" = { fg = \"green\" }\n",
            )
            .unwrap();
            state.colors.theme = crate::theme::resolve::resolve_theme(
                &crate::colors::GhosttyScheme::default(),
                &parsed,
            );
            state.overlays.region_prompt = Some(suggestion_prompt());
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut rects = None;
            terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
            let r = rects.expect("an open prompt draws");
            let buf = terminal.backend().buffer();
            let fg_at = |rect: Rect| buf.cell((rect.x + 1, rect.y)).unwrap().style().fg;
            assert_eq!(
                fg_at(r.options[0]),
                Some(Color::Green),
                "the chosen row uses region_prompt.option:chosen (honor_game_colours={honor})"
            );
            assert_eq!(
                fg_at(r.options[1]),
                Some(Color::Blue),
                "an unchosen row uses region_prompt.option (honor_game_colours={honor})"
            );
        }
    }
}
