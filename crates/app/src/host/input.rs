//! Player input a host delivers to the game that is not a typed line (SQ-1568).
//!
//! - [`deliver_v6_click`] — a mouse click on a Version 6 game's own screen.
//! - [`deliver_line_key_terminator`] — a special key (arrow / function key) that
//!   ends a pending LINE read via the story's own terminating-characters table,
//!   on either engine (Z-machine gated by [`withhold_arrow_from_v6`], SQ-1610;
//!   Glulx added SQ-1613).
//!
//! The rule for what a v6 click DOES — and what a line-terminator key DOES —
//! used to live only inside the terminal binary's event loop, so a host that is
//! not a terminal could not reach it. What stays in the binary is the terminal's
//! own gesture handling — press, drag, release, which cell maps to which game
//! pixel, and turning a raw key event into a neutral [`KeyInput`]; this module is
//! what happens once a host has decided a click landed on game pixel `(x, y)` or
//! that a given `KeyInput` was pressed.

use mapper::mapper::Mapper;

use crate::engine::{Engine, KeyInput};
use crate::engine_helpers::{glulx_session_opt_mut, zvm_session_opt_mut};
use crate::session::InputKind;
use crate::state::{AppState, V6ClickRead};

use super::turn::{apply_game_driven_result, finish_command_turn, TurnCtx, TurnOutcome};

// ── Arrow-key withholding (SQ-0460) ──────────────────────────────────────────

/// Whether an arrow keypress should be forwarded to the story as a ZSCII
/// cursor code (129-132; ZMSD §3.8). Some v6 games bind arrows to movement;
/// `v6_arrow_keys = false` withholds them so the key falls through to
/// app-side handling (scrollback / map panning) instead. Only v6 is gated —
/// v1-5 and Glulx stories always get arrows, regardless of `version`'s value
/// for a non-Z-machine session (callers pass a version of 0 in that case).
pub fn forward_arrow_to_v6(v6_arrow_keys: bool, version: u8) -> bool {
    version != 6 || v6_arrow_keys
}

/// Whether `ki` is an arrow that `v6_arrow_keys = false` withholds from a v6
/// story. Withholding applies ONLY at a line (`>`) prompt (`is_line_input`) —
/// that's where movement-vs-panning conflicts, and v6 games list arrows in
/// their terminating-characters table (SQ-0188), so an arrow would otherwise
/// move the player from the prompt regardless of the setting. During CHAR
/// input (`is_line_input = false`: menus, "press any key") arrows are NEVER
/// withheld — those screens are unnavigable without them, so the setting has
/// no say there and arrows always reach a v6 story (SQ-0483).
pub fn withhold_arrow_from_v6(
    ki: Option<KeyInput>,
    v6_arrow_keys: bool,
    version: u8,
    is_line_input: bool,
) -> bool {
    is_line_input
        && ki.is_some_and(|ki| {
            matches!(ki, KeyInput::Up | KeyInput::Down | KeyInput::Left | KeyInput::Right)
                && !forward_arrow_to_v6(v6_arrow_keys, version)
        })
}

/// The ZSCII single-click code (ZMSD §3.8).
const SINGLE_CLICK: u8 = 254;

/// Deliver a left click on 1-based game pixel `game_px` = `(x, y)` to the pending
/// read, and apply the turn it produces.
///
/// `None` — and nothing touched — when the pending read takes no click: not a
/// Z-machine story, no read pending, or a LINE read whose terminating-characters
/// table lists no click (Journey's; see [`crate::input::v6_click_read`]).
///
/// - A CHAR read gets ZSCII 254 with the coordinates recorded first, so the
///   game's `read_mouse` reports them; the result is applied as a game-driven
///   turn, which is not a counted player turn.
/// - A LINE read ends with whatever the player has typed (`AppState::take_input`)
///   and the click as its terminator, and goes through [`finish_command_turn`]
///   like a typed command: history, turn count, mapping, autosave. A compass click
///   types nothing, but the game echoes the command it synthesized ("north") at
///   the head of its output, and that echo is adopted as the turn's command so the
///   move maps exactly like the typed word (SQ-0576).
pub fn deliver_v6_click(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    ctx: &mut TurnCtx<'_>,
    game_px: (u16, u16),
) -> Option<TurnOutcome> {
    let (gx, gy) = game_px;
    let z = zvm_session_opt_mut(session)?;
    let read = crate::input::v6_click_read(Some(z.pending_input()), z.mouse_click_terminator())?;
    match read {
        V6ClickRead::Char => {
            z.set_mouse(gy, gx); // engine stores (y, x)
            let result = z.submit_char(SINGLE_CLICK);
            Some(apply_game_driven_result(
                state,
                mapper,
                &result,
                ctx.game_dir,
                ctx.map_view,
                &*session,
                crate::pager::Driver::PlayerInput,
            ))
        }
        V6ClickRead::Line { terminator } => {
            let cmd = state.take_input();
            z.set_mouse(gy, gx); // engine stores (y, x)
            let result = z.submit_line_with_terminator(&cmd, terminator);
            let cmd = if cmd.is_empty() {
                crate::session::echoed_direction_command(&result.transcript)
                    .unwrap_or_default()
                    .to_string()
            } else {
                cmd
            };
            Some(finish_command_turn(
                &cmd,
                true,
                result,
                state,
                mapper,
                session,
                ctx.game_dir,
                ctx.ifid,
                ctx.arc_file,
                ctx.map_view,
                ctx.bg_tidy_counter,
            ))
        }
    }
}

/// Deliver a key that ends a pending *line* read via the story's own
/// terminating-characters table — Z-machine (v5+ table,
/// [`crate::session::GameSession::line_key_terminator`], ZMSD §10.7) or Glulx
/// (`glk_set_terminators_line_event`, [`crate::glulx_session::GlulxSession::line_key_terminator`],
/// Glk spec §11.2). The Z-machine path is gated by [`withhold_arrow_from_v6`]
/// (SQ-0460): a v6 story that lists arrows as line terminators would otherwise
/// let an arrow move the player from the prompt regardless of `v6_arrow_keys`.
/// Glulx needs no such gate: only `Escape` and function keys can ever be a
/// registered Glk terminator ([`gvm::glk::keycode::is_terminator`]) — arrows
/// are structurally excluded there, so `line_key_terminator` alone rejects
/// them, independent of `v6_arrow_keys` (SQ-1613).
///
/// `None` — and nothing touched — when there is nothing to submit: neither
/// engine has a LINE read pending, `ki` is a v6 arrow currently withheld
/// (Z-machine only), or `ki` is not one the story/game lists as a terminator.
///
/// Otherwise ends the current input like a typed command
/// (`AppState::take_input` + [`finish_command_turn`]): history, turn count,
/// mapping, autosave — with `ended_on_newline = false`, since the read ended on
/// a listed terminating character, not Enter (SQ-0881).
pub fn deliver_line_key_terminator(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    ctx: &mut TurnCtx<'_>,
    ki: KeyInput,
) -> Option<TurnOutcome> {
    if let Some(z) = zvm_session_opt_mut(session) {
        if z.pending_input() != InputKind::Line {
            return None;
        }
        let version = z.machine.mem.version();
        if withhold_arrow_from_v6(Some(ki), state.config.v6_arrow_keys, version, true) {
            return None;
        }
        let term = z.line_key_terminator(&ki)?;
        let cmd = state.take_input();
        let result = z.submit_line_with_terminator(&cmd, term);
        return Some(finish_command_turn(
            &cmd,
            false,
            result,
            state,
            mapper,
            session,
            ctx.game_dir,
            ctx.ifid,
            ctx.arc_file,
            ctx.map_view,
            ctx.bg_tidy_counter,
        ));
    }
    let g = glulx_session_opt_mut(session)?;
    if g.pending_input() != InputKind::Line {
        return None;
    }
    let term = g.line_key_terminator(&ki)?;
    let cmd = state.take_input();
    let result = g.submit_line_with_terminator(&cmd, term);
    Some(finish_command_turn(
        &cmd,
        false,
        result,
        state,
        mapper,
        session,
        ctx.game_dir,
        ctx.ifid,
        ctx.arc_file,
        ctx.map_view,
        ctx.bg_tidy_counter,
    ))
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::session::GameSession;
    use crate::state::AppState;
    use mapper::mapper::Mapper;

    // ── SQ-0460: arrow-key withholding (relocated from main.rs, SQ-1610) ─────

    #[test]
    fn forward_arrow_to_v6_gates_only_v6_when_disabled() {
        // v6_arrow_keys = true: every version forwards arrows.
        assert!(forward_arrow_to_v6(true, 6));
        assert!(forward_arrow_to_v6(true, 5));
        assert!(forward_arrow_to_v6(true, 0));

        // v6_arrow_keys = false (the default, SQ-1087): only version 6 is
        // withheld; v1-5 and the Glulx/no-session placeholder (version 0) still
        // forward arrows.
        assert!(!forward_arrow_to_v6(false, 6));
        assert!(forward_arrow_to_v6(false, 5));
        assert!(forward_arrow_to_v6(false, 3));
        assert!(forward_arrow_to_v6(false, 0));
    }

    #[test]
    fn withhold_arrow_from_v6_covers_all_arrows_and_only_arrows() {
        // The SQ-0188 line-terminator gate uses this predicate with
        // is_line_input = true — v6 games list arrows as line terminators for
        // movement, so gating read_char alone left arrows moving the player.
        for arrow in [KeyInput::Up, KeyInput::Down, KeyInput::Left, KeyInput::Right] {
            assert!(withhold_arrow_from_v6(Some(arrow), false, 6, true), "{arrow:?} withheld on v6 when off");
            assert!(!withhold_arrow_from_v6(Some(arrow), true, 6, true), "{arrow:?} forwarded when on");
            assert!(!withhold_arrow_from_v6(Some(arrow), false, 5, true), "{arrow:?} forwarded on v5");
        }
        // Non-arrows and no-input keys are never withheld.
        assert!(!withhold_arrow_from_v6(Some(KeyInput::Enter), false, 6, true));
        assert!(!withhold_arrow_from_v6(Some(KeyInput::Func(1)), false, 6, true));
        assert!(!withhold_arrow_from_v6(None, false, 6, true));
    }

    #[test]
    fn withhold_arrow_from_v6_never_withholds_during_char_input() {
        // SQ-0483: the char-input (read_char) gate calls the predicate with
        // is_line_input = false. Menus (Shogun's startup menu, hint menus,
        // "press any key") are unnavigable without arrows, so v6 arrows are
        // ALWAYS delivered there — the setting has no say during char input.
        for arrow in [KeyInput::Up, KeyInput::Down, KeyInput::Left, KeyInput::Right] {
            // (a) setting off + char input pending → arrow IS delivered.
            assert!(
                !withhold_arrow_from_v6(Some(arrow), false, 6, false),
                "{arrow:?} must reach a v6 menu even with the setting off",
            );
            // (c) setting on → delivered during char input too.
            assert!(!withhold_arrow_from_v6(Some(arrow), true, 6, false));
        }
        // Contrast (b): the SAME arrow + setting off IS withheld at a line
        // prompt — that path is covered above with is_line_input = true.
        assert!(withhold_arrow_from_v6(Some(KeyInput::Up), false, 6, true));
    }

    // ── SQ-1610: deliver_line_key_terminator ──────────────────────────────────

    /// A v5/v6 story with a real `read` (v5+ `aread`, VAR opcode 0x04 = byte
    /// 0xE4) so `pending_input()` is genuinely `Line` — not the no-request-made
    /// fallback `pending_input_is_line_after_new_on_quitting_story` exercises in
    /// `session.rs` — followed by `quit` once the line is answered. Its header
    /// lists ZSCII 129 (cursor Up) in its terminating-characters table (0x2E),
    /// the same shape `line_key_terminator`'s own fixture uses
    /// (`story_v5_with_up_terminator` in `session.rs`).
    ///
    /// `version` 5 or 6: v6 wraps the same `read`+`quit` body in a routine (a
    /// leading local-count byte, called through the packed main-routine
    /// address, matching `v6_boot_stub_story`'s convention in `session.rs`);
    /// v5 runs it directly as the entry point, matching `read_char_story_v5`'s.
    fn story_with_up_terminator(version: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = version;
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        // v3-5: initial_pc = 0x0040 (raw address). v6: header 0x06/0x07 is
        // main's PACKED address; routines_offset (0x28/0x29) is 0, so
        // unpack_routine(p) = 4*p — 0x0040 unpacks to 0x0100.
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dictionary = 0x0080 (empty: word-sep=0, entry-size=4, entry-count=0)
        buf[0x08] = 0x00; buf[0x09] = 0x80;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100 (unused)
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        // Terminating-characters table at 0x0090: Up (129), then table terminator.
        buf[0x2E] = 0x00; buf[0x2F] = 0x90;
        buf[0x0090] = 0x81; buf[0x0091] = 0x00;

        // `read` (VAR:0x04, byte 0xE4): two Large-const operands (text_buf,
        // parse_buf) then a v5+ store byte (-> G0), mirroring zvm's own
        // `emit_read` fixture (crates/zvm/src/cpu/exec.rs).
        let read = [0xE4u8, 0x0F, 0x02, 0x50, 0x02, 0x60, 0x10];
        buf[0x0250] = 0x20; // text buffer max length
        buf[0x0260] = 0x04; // parse buffer max words

        if version == 6 {
            buf[0x0100] = 0; // local count
            buf[0x0101..0x0101 + read.len()].copy_from_slice(&read);
            buf[0x0101 + read.len()] = 0xBA; // quit
        } else {
            buf[0x0040..0x0040 + read.len()].copy_from_slice(&read);
            buf[0x0040 + read.len()] = 0xBA; // quit
        }
        buf
    }

    fn line_session(version: u8) -> GameSession {
        let s = GameSession::new(story_with_up_terminator(version), true, false, None)
            .expect("GameSession::new");
        assert_eq!(s.pending_input(), InputKind::Line, "premise: a real read suspends on Line");
        s
    }

    /// (a) A listed arrow terminator submits and returns `Some` on a non-V6
    /// story regardless of `v6_arrow_keys` — only v6 is ever gated.
    #[test]
    fn deliver_line_key_terminator_forwards_arrow_on_non_v6_regardless_of_setting() {
        for v6_arrow_keys in [false, true] {
            let mut session = line_session(5);
            let mut state = AppState::default();
            state.config.v6_arrow_keys = v6_arrow_keys;
            let mut mapper = Mapper::default();
            let game_dir = std::path::PathBuf::from("nonexistent");
            let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
            let mut tidy = 0u32;
            let mut ctx = TurnCtx {
                game_dir: &game_dir,
                ifid: "test-ifid",
                arc_file: &arc_file,
                map_view: None,
                bg_tidy_counter: &mut tidy,
            };
            let turns_before = state.turns;
            let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Up);
            assert!(out.is_some(), "v6_arrow_keys={v6_arrow_keys}: a v5 story submits the terminator");
            assert_eq!(state.turns, turns_before + 1, "v6_arrow_keys={v6_arrow_keys}: a counted turn");
        }
    }

    /// (b) The SAME arrow on a V6 story with `v6_arrow_keys = false` (the
    /// default) is withheld: `None`, and nothing about the session moves.
    #[test]
    fn deliver_line_key_terminator_withholds_the_same_arrow_from_v6_by_default() {
        let mut session = line_session(6);
        let mut state = AppState::default();
        state.config.v6_arrow_keys = false;
        let mut mapper = Mapper::default();
        let game_dir = std::path::PathBuf::from("nonexistent");
        let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
        let mut tidy = 0u32;
        let mut ctx = TurnCtx {
            game_dir: &game_dir,
            ifid: "test-ifid",
            arc_file: &arc_file,
            map_view: None,
            bg_tidy_counter: &mut tidy,
        };
        let turns_before = state.turns;
        let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Up);
        assert!(out.is_none(), "a v6 story withholds the arrow when v6_arrow_keys is off");
        assert_eq!(state.turns, turns_before, "no turn was consumed");
        assert_eq!(session.pending_input(), InputKind::Line, "the read is left exactly as it was");
    }

    /// (c) With `v6_arrow_keys = true` the v6 story behaves like (a) again.
    #[test]
    fn deliver_line_key_terminator_forwards_the_arrow_once_v6_arrow_keys_is_on() {
        let mut session = line_session(6);
        let mut state = AppState::default();
        state.config.v6_arrow_keys = true;
        let mut mapper = Mapper::default();
        let game_dir = std::path::PathBuf::from("nonexistent");
        let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
        let mut tidy = 0u32;
        let mut ctx = TurnCtx {
            game_dir: &game_dir,
            ifid: "test-ifid",
            arc_file: &arc_file,
            map_view: None,
            bg_tidy_counter: &mut tidy,
        };
        let turns_before = state.turns;
        let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Up);
        assert!(out.is_some(), "v6_arrow_keys = true forwards the arrow to a v6 story too");
        assert_eq!(state.turns, turns_before + 1, "a counted turn");
    }

    // ── SQ-1613: deliver_line_key_terminator on Glulx ─────────────────────────

    use crate::glulx_session::GlulxSession;
    use gvm::glk::keycode;

    // A tiny Glulx instruction encoder (mirrors the one `glulx_session.rs`'s own
    // test module carries — that file's comment notes it in turn mirrors
    // gvm-cli's; each test module keeps its own trimmed copy rather than
    // sharing one across crates/test-binaries).
    #[derive(Clone, Copy)]
    enum E {
        Imm(u32),
        LocLoad(u8),
        LocStore(u8),
        Push,
        Discard,
    }
    fn emode(e: E) -> u8 {
        match e {
            E::Imm(_) => 3,
            E::LocLoad(_) | E::LocStore(_) => 9,
            E::Push => 8,
            E::Discard => 0,
        }
    }
    fn edata(e: E) -> Vec<u8> {
        match e {
            E::Imm(v) => v.to_be_bytes().to_vec(),
            E::LocLoad(o) | E::LocStore(o) => vec![o],
            E::Push | E::Discard => vec![],
        }
    }
    fn enc(op: u32, args: &[E]) -> Vec<u8> {
        let mut out = Vec::new();
        if op <= 0x7f {
            out.push(op as u8);
        } else {
            out.extend_from_slice(&((op | 0x8000) as u16).to_be_bytes());
        }
        let mut modes = vec![0u8; args.len().div_ceil(2)];
        for (i, &a) in args.iter().enumerate() {
            let m = emode(a);
            if i % 2 == 0 { modes[i / 2] |= m } else { modes[i / 2] |= m << 4 }
        }
        out.extend_from_slice(&modes);
        for &a in args {
            out.extend(edata(a));
        }
        out
    }

    // RAM layout (RAMSTART 0x400, ENDMEM 0x500).
    const EVENT: u32 = 0x400;
    const TERMS: u32 = 0x410;
    const LINEBUF: u32 = 0x480;

    fn image_for(body: Vec<u8>, nlocals: u8) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, nlocals, 0x00, 0x00]; // type C1; nlocals 4-byte
        func.extend(body);
        let (ramstart, endmem) = (0x400u32, 0x500u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    /// Open a TextBuffer (id → local0) and make it current.
    fn open_buffer_prelude() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        for v in [Imm(0), Imm(3), Imm(0), Imm(0), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // rock, wintype=3, size, method, split
        }
        b.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(0)])); // window_open → local0
        b.extend(enc(0x40, &[LocLoad(0), Push]));
        b.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(local0)
        b
    }

    /// Open a buffer window, register `Func1` (only) as its line terminator via
    /// `glk_set_terminators_line_event`, then request a line and select. A
    /// single pending line request is enough to probe several keys: a call that
    /// returns `None` (unregistered key, or an arrow that is never even a
    /// candidate) never touches the machine, so the SAME request is still
    /// pending for the next probe. Quits once the line is finally submitted.
    fn line_image_with_func1_terminator() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude(); // local0 = buffer win, current
        body.extend(enc(0x4C, &[Imm(TERMS), Imm(0), Imm(keycode::FUNC1)])); // astore TERMS[0] = Func1
        for v in [Imm(1), Imm(TERMS), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // count, ptr, win (reverse arg order)
        }
        body.extend(enc(0x130, &[Imm(0x151), Imm(3), Discard])); // set_terminators_line_event
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // initlen, maxlen, buf, win
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    fn glulx_line_session() -> GlulxSession {
        let s = GlulxSession::new(line_image_with_func1_terminator(), 80, 24, true, false, false, (1.0, 1.0), None, &[])
            .expect("GlulxSession::new");
        assert_eq!(s.pending_input(), InputKind::Line, "premise: the fixture suspends on a line request");
        s
    }

    fn glulx_ctx<'a>(game_dir: &'a std::path::Path, arc_file: &'a std::path::Path, tidy: &'a mut u32) -> TurnCtx<'a> {
        TurnCtx { game_dir, ifid: "test-ifid", arc_file, map_view: None, bg_tidy_counter: tidy }
    }

    /// (a) A registered Func-key terminator submits the line and drives the game
    /// (which quits right after), regardless of `v6_arrow_keys` — that setting
    /// has no say on this engine.
    #[test]
    fn deliver_line_key_terminator_submits_a_registered_glulx_terminator() {
        for v6_arrow_keys in [false, true] {
            let mut session = glulx_line_session();
            let mut state = AppState::default();
            state.config.v6_arrow_keys = v6_arrow_keys;
            let mut mapper = Mapper::default();
            let game_dir = std::path::PathBuf::from("nonexistent");
            let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
            let mut tidy = 0u32;
            let mut ctx = glulx_ctx(&game_dir, &arc_file, &mut tidy);
            let turns_before = state.turns;
            let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Func(1));
            assert!(out.is_some(), "v6_arrow_keys={v6_arrow_keys}: a registered Func1 submits the line");
            assert_eq!(state.turns, turns_before + 1, "a counted turn");
        }
    }

    /// (b) A key that IS a structurally-valid Glk terminator (Escape) but was
    /// never registered by this game returns `None`, and the pending line
    /// request is left exactly as it was.
    #[test]
    fn deliver_line_key_terminator_leaves_an_unregistered_glulx_key_untouched() {
        let mut session = glulx_line_session();
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        let game_dir = std::path::PathBuf::from("nonexistent");
        let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
        let mut tidy = 0u32;
        let mut ctx = glulx_ctx(&game_dir, &arc_file, &mut tidy);
        let turns_before = state.turns;
        let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Escape);
        assert!(out.is_none(), "Escape was never registered as a terminator by this game");
        assert_eq!(state.turns, turns_before, "no turn was consumed");
        assert_eq!(session.pending_input(), InputKind::Line, "the read is left exactly as it was");
    }

    /// (c) An arrow key is never even a candidate on Glulx — Escape/Func-keys
    /// are the only keys `gvm::glk::keycode::is_terminator` admits, so an arrow
    /// is rejected by that structural check alone. Confirm the result does not
    /// depend on `v6_arrow_keys` at all (unlike the Z-machine branch): no
    /// `withhold_arrow_from_v6` call happens on this path.
    #[test]
    fn deliver_line_key_terminator_never_treats_a_glulx_arrow_as_a_candidate() {
        for v6_arrow_keys in [false, true] {
            let mut session = glulx_line_session();
            let mut state = AppState::default();
            state.config.v6_arrow_keys = v6_arrow_keys;
            let mut mapper = Mapper::default();
            let game_dir = std::path::PathBuf::from("nonexistent");
            let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
            let mut tidy = 0u32;
            let mut ctx = glulx_ctx(&game_dir, &arc_file, &mut tidy);
            let turns_before = state.turns;
            let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Up);
            assert!(out.is_none(), "v6_arrow_keys={v6_arrow_keys}: an arrow is structurally never a Glk terminator");
            assert_eq!(state.turns, turns_before, "no turn was consumed");
            assert_eq!(session.pending_input(), InputKind::Line, "the read is left exactly as it was");
        }
    }

    /// (d) SQ-1616: a registered Glulx line terminator must survive an
    /// `Engine::save_state`/`restore_state` round trip — the same round trip
    /// `GlulxSession::silent_look` performs on every `look`-driven room-name
    /// probe (`glulx_session.rs`), so it fires far more often than a
    /// player-triggered Save State. Before the fix, `gvm::glk::Model::deserialize`
    /// unconditionally rebuilt every window with an empty terminator list,
    /// discarding whatever `glk_set_terminators_line_event` had registered — so
    /// `is_line_terminator(Func1)` read true before the round trip and false
    /// after it.
    #[test]
    fn deliver_line_key_terminator_survives_a_save_restore_round_trip_sq1616() {
        let mut session = glulx_line_session();
        assert!(
            session.line_key_terminator(&KeyInput::Func(1)).is_some(),
            "premise: Func1 is a registered terminator before any round trip"
        );

        let snapshot = Engine::save_state(&session);
        Engine::restore_state(&mut session, &snapshot).expect("restore the session's own save");

        assert!(
            session.line_key_terminator(&KeyInput::Func(1)).is_some(),
            "Func1 must still be recognized as a line terminator after a save/restore round trip (SQ-1616)"
        );

        // And the end-to-end gate still submits the line after the round trip.
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        let game_dir = std::path::PathBuf::from("nonexistent");
        let arc_file = std::path::PathBuf::from("nonexistent.lanthorn");
        let mut tidy = 0u32;
        let mut ctx = glulx_ctx(&game_dir, &arc_file, &mut tidy);
        let out = deliver_line_key_terminator(&mut state, &mut mapper, &mut session, &mut ctx, KeyInput::Func(1));
        assert!(out.is_some(), "the terminator still submits the line after the round trip");
    }
}
