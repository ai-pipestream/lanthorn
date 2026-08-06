// Z-machine executor core — ZMSD §14, §15.
//
// Provides `Machine` (memory + CPU state) and `step()` (fetch-decode-execute).
// The pc-advance contract: step() sets state.pc = instr.next_pc BEFORE executing,
// so that call handlers find state.pc already pointing past the call instruction,
// making it the correct return_pc. Branch/jump offsets are relative to next_pc.
//
// Dispatch structure: match on operand_count then opcode number.
// Tasks 10–13 add arms to the same match without restructuring the core.

use crate::cpu::decode::{decode, Branch, Instr, Operand, OperandCount};
use crate::cpu::state::{call_routine, peek_stack, poke_stack, read_var, return_value, write_var, State};
use crate::dictionary;
use crate::io::{BufferOutput, Output};
use crate::memory::Memory;
use crate::objects;
use crate::screen::{advertise_colour, advertise_sound, init_header_caps, write_default_colours, ScreenState, StreamState, V6Windows, GRID_CELL_CAP, V6_FONT_HEIGHT, V6_FONT_WIDTH};
use crate::text::cp437::cp437_to_char;
use crate::text::decode::{decode_string, zscii_to_char};

/// Best-effort mnemonic for a decoded instruction; hex fallback when unknown.
/// Covers the memory/stack opcodes most likely to fault, plus common ones.
fn opcode_name(count: OperandCount, opcode: u8) -> String {
    let name = match (count.clone(), opcode) {
        (OperandCount::Two, 0x0F) => "loadw",
        (OperandCount::Two, 0x10) => "loadb",
        (OperandCount::Two, 0x01) => "je",
        (OperandCount::One, 0x0F) => "call_1n",
        (OperandCount::Var, 0x01) => "storew",
        (OperandCount::Var, 0x02) => "storeb",
        (OperandCount::Var, 0x00) => "call",
        (OperandCount::Var, 0x06) => "print_num",
        _ => return format!("op:{:?}/0x{:02x}", count, opcode),
    };
    name.to_string()
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A Z-machine `sound_effect` event (ZMSD §9.4), recorded for the host to act on.
/// `number` 1/2 are the built-in high/low bleeps; `number >= 3` selects a Blorb
/// `Snd ` resource. `effect`: 1=prepare 2=start 3=stop 4=finish. `volume` is the
/// Z-scale 1..=8 (255 = loudest). `repeats` is the repeat count from the volume
/// word's high byte; 255 = forever, 0/omitted = play once (applied by the host).
/// `routine` (v5+) is the finish-routine the host calls when the sound ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundEvent {
    pub number: u16,
    pub effect: u8,
    pub volume: u8,
    pub repeats: u8,
    pub routine: u16,
}

/// A v6 `draw_picture`/`erase_picture` event (ZMSD §15), recorded for the host
/// to act on in Plan 1b. `number` is the picture number; `window` is the v6
/// window the call targeted (`ScreenState.v6.current` at the time); `x`/`y`
/// are pixel coordinates (of the top-left corner) within that window. Both
/// opcodes share the same `(picture-number, y, x)` operands, so `erase`
/// distinguishes them — `erase_picture` needs the real picture number too
/// (to know the region's dimensions), which rules out a `number: 0`
/// "erase all" sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureEvent {
    pub number: u16,
    pub window: u8,
    pub x: u16,
    pub y: u16,
    pub erase: bool,
    /// Total chars printed to window 0 (the main scrolling window) before this
    /// event — anchors a window-0 inline picture (drop-cap, room icon) to its
    /// exact position in the text stream, so the host can float it beside the
    /// paragraph it belongs to. Meaningless for windows 1–7.
    pub out_chars: u64,
    /// The `left` value of a `set_margins` issued on the same window directly
    /// after this draw, if any — v6 games (Zork Zero's PICINF idiom) set the
    /// text margin right after drawing an inline picture to make text flow
    /// beside it. `None` when no such call followed.
    pub margin_after: Option<u16>,
}

/// Result of executing one instruction.
#[derive(Debug, PartialEq)]
pub enum StepResult {
    /// Normal execution: continue to next instruction.
    Continue,
    /// `quit` opcode — host should stop the run loop.
    Quit,
    /// `restart` opcode — host should reload and restart.
    Restart,
    /// `read` / `sread` — host must supply a line of input.
    NeedLine { text_buf: u32, parse_buf: u32 },
    /// `read_char` — host must supply a single keypress.
    NeedChar,
    /// `save` — host must write interpreter state to a file.
    SaveRequest,
    /// `restore` — host must read interpreter state from a file.
    RestoreRequest,
    /// A runtime fault halted the machine. The host reads `take_fault_trace()`.
    Fault,
}

/// State saved while waiting for player input (`read` / `read_char`).
///
/// When `step()` returns `NeedLine` or `NeedChar` the machine suspends; the
/// host calls `supply_line` / `supply_char` with the input, which uses these
/// fields to complete the operation (write buffers, store result) before the
/// next `step()` call.
#[derive(Clone, Copy)]
struct PendingInput {
    /// Destination variable for the result (store var of the read/read_char
    /// instruction; `None` if the instruction has no store — v3 `read` has none).
    store_var: Option<u8>,
    /// Address of the text buffer (for `supply_line`).
    text_buf: u32,
    /// Address of the parse buffer (for `supply_line`; 0 in v5+ means skip).
    parse_buf: u32,
    /// Timed-input interval in tenths of a second (0 = untimed).
    interrupt_time: u16,
    /// Packed address of the interrupt routine (0 = none).
    interrupt_routine: u16,
    /// Start PC of the read/read_char instruction that suspended here. A host
    /// Save State taken at this prompt records this (via `save_pc`) so restoring
    /// it re-executes the read and re-arms the prompt — otherwise the resume
    /// would land past the read on a stale input buffer.
    instr_pc: u32,
}

/// One in-memory undo snapshot: the Quetzal state blob plus the `save_undo`
/// instruction's store target, so `restore_undo` can write 2 back into it.
#[derive(Debug, Clone)]
pub struct UndoSnapshot {
    pub blob: Vec<u8>,
    pub store: Option<u8>,
}

/// Outcome of running a timed-input interrupt routine.
pub struct TimedInterrupt {
    /// The routine returned nonzero: the host should abort the pending read.
    pub aborted: bool,
}

/// The Z-machine interpreter — ties memory and CPU state together.
/// Fields are `pub` so Tasks 11+ can attach I/O channels.
pub struct Machine {
    pub mem: Memory,
    pub state: State,
    /// Pluggable text output sink. Defaults to `BufferOutput` (Task 11).
    pub out: Box<dyn Output>,
    /// Non-None while the machine is suspended waiting for player input.
    pending_input: Option<PendingInput>,
    /// Screen model: window layout, cursor, text style.
    pub screen: ScreenState,
    /// Output stream routing: streams 1/2/3/4 state.
    pub streams: StreamState,
    /// Snapshot of the original dynamic memory (bytes 0..static_mem_base) taken
    /// at construction time.  Used by Quetzal CMem encoding (XOR diff).
    /// Story files are small (< 256 KB) so the memory cost is acceptable.
    pub original_dynamic: Vec<u8>,
    /// In-memory undo snapshots (newest last). Session-only; not saved.
    pub undo_stack: Vec<UndoSnapshot>,
    /// Max retained undo snapshots; 0 disables undo. Default 16; the app sets it
    /// from `config.undo_levels`.
    pub undo_cap: usize,
    /// Pending save-request context: branch/store info from the save opcode so
    /// complete_save() can deliver the version-appropriate result.
    pending_save: Option<PendingSave>,
    /// Store variable captured from the restore opcode (v4+), used by
    /// complete_restore_failure() to store 0 into the correct variable.
    pending_restore_store: Option<u8>,
    /// True while a game `@restore` is suspended awaiting the host's bytes, in
    /// EVERY version. `pending_restore_store` cannot answer that question on its
    /// own: v3's `@restore` is a BRANCH instruction with no store target, so the
    /// field stays `None` through a perfectly real v3 suspension. Cleared by
    /// `complete_restore_success`/`complete_restore_failure` (the two ways the
    /// suspension resolves), and by `restart`/`restore_file` (the two ways the run
    /// it belonged to is discarded). Read via [`Machine::is_saveload_pending`].
    pending_restore: bool,
    /// PRNG state for the `random` opcode (xorshift32).
    /// Initialised to a fixed nonzero constant; seeded by `random` with negative arg.
    rng_state: u32,
    /// VAR opcodes that have hit the unimplemented fallthrough (warned once each).
    pub(crate) warned_var_opcodes: std::collections::HashSet<u8>,
    /// EXT opcodes that have hit the unimplemented fallthrough (warned once each).
    pub(crate) warned_ext_opcodes: std::collections::HashSet<u8>,
    /// Sound events recorded by `sound_effect` since the host last drained them.
    pub pending_sounds: Vec<SoundEvent>,
    /// Injected picture-dimension table for v6 `picture_data`: `(picture_number,
    /// width_px, height_px)`. Populated by the host (Task 9) before the boot run
    /// from the self-blorb's `Pict` resources; empty for non-v6 stories.
    pub picture_dims: Vec<(u16, u16, u16)>,
    /// Draw/erase events recorded by `draw_picture`/`erase_picture` since the
    /// host last drained them. The engine never rasterizes; the host (Plan
    /// 1b) decodes the Blorb `Pict` resource and renders it — mirrors
    /// `pending_sounds`.
    pub pending_pictures: Vec<PictureEvent>,
    /// Running count of chars printed to v6 window 0 (the main scrolling
    /// window) — stamps `PictureEvent::out_chars` so window-0 inline pictures
    /// anchor to their position in the text stream. Monotonic, never reset.
    pub v6_win0_out_chars: u64,
    /// The v6 window the game last asked for INPUT through, when that window was a
    /// flowing-prose one (SQ-0585). It is the game's main text window by definition
    /// — the one the player types into — so its output is what the host mirrors as
    /// the transcript. Any OTHER prose window is a display panel, and its text goes
    /// to that window's own `prose` buffer instead of being spliced into the same
    /// stream. `0` until the first input request, which is right for boot: window 0
    /// is the classic main window, and text printed before any read (the banner)
    /// belongs to the transcript.
    pub v6_input_window: u8,
    /// Host-facing diagnostic lines (e.g. unimplemented opcodes, sampled sounds)
    /// recorded since the host last drained them. The engine never prints.
    pub diagnostics: Vec<String>,
    /// In-memory auxiliary save table for the v5 `save/restore table` opcodes,
    /// keyed by the game-supplied name string. The host persists/repopulates it
    /// (in the `.babelmap` archive or a per-game global file); the engine itself
    /// never touches the filesystem.
    pub aux_data: std::collections::BTreeMap<String, Vec<u8>>,
    /// Set true whenever an aux `save table` writes the table. The host clears it
    /// after persisting; correctness does not depend on the flag (every archive
    /// write embeds the latest table) — it is a "data changed" notification.
    pub aux_dirty: bool,
    /// Whether to advertise Flags1 bit 0 (colour available) to the game. Default
    /// false; set via `set_honor_game_colours`. v3 stories ignore this entirely.
    pub honor_game_colours: bool,
    /// Whether to advertise sound-effects capability (Flags1 bit 5 v4+, Flags2
    /// bit 7). Default false; set via `set_sound_available`.
    pub sound_available: bool,
    /// Interpreter number to advertise in header byte 0x1E. `None` = auto (Frotz's
    /// rule: 6 for v6, else 1). `Some(n)` overrides. Applied at `init_caps`.
    pub interpreter_number: Option<u8>,
    /// The interpreter's default background/foreground colours, published to the
    /// game in header bytes $2C/$2D (ZMSD §8.3.3). Standard colour numbers
    /// 2..=9; defaults to black-on-white (2/9). Set via `set_default_colours`,
    /// re-applied at every `init_caps` (so `@restart` keeps the host's choice).
    pub default_bg_colour: u8,
    pub default_fg_colour: u8,
    /// Set when `step()` returns `Fault`; the host drains it for display.
    pub fault_trace: Option<crate::cpu::trace::StackTrace>,
    /// Start PC of the instruction currently being executed (set each `step()`).
    /// Captured into `PendingInput.instr_pc` when a `read`/`read_char` suspends,
    /// so `save_pc` can rewind a save-at-input-prompt to the read instruction.
    cur_instr_pc: u32,
    /// When true, screen-control opcodes push a decoded line into `screen_trace`
    /// (the `screen` debug section). Separate from `diagnostics`. (trace feature)
    pub trace_screen: bool,
    /// Accumulated `screen`-trace lines since the host last drained them.
    pub screen_trace: Vec<String>,
    /// When true, `step()` records each instruction's start PC into `exec_pcs`
    /// (the debug inspector's execution-coverage marking).
    pub trace_exec: bool,
    /// Start PCs of instructions executed since the host last cleared them.
    pub exec_pcs: std::collections::HashSet<u32>,
    /// Cumulative start PCs of every instruction ever executed while tracing was
    /// on — NEVER cleared per turn (unlike `exec_pcs`). Drives the permanent
    /// "executed" disassembly colour, and can be pre-seeded from host-persisted
    /// coverage (the debug PC-set sidecar) via [`seed_executed`](Machine::seed_executed).
    pub ever_exec_pcs: std::collections::HashSet<u32>,
    /// v6 `buffer_screen` (EXT:0x1D) state: 0 = update immediately, 1 = the
    /// interpreter may buffer to a backing store. No rendering effect here —
    /// the value is tracked purely so the opcode's store result (the OLD
    /// mode) is correct (ZMSD §15).
    buffer_screen_mode: u16,
    /// True once `output_stream 2` (the transcript FILE stream) has recorded its
    /// "not supported" diagnostic — ZMSD §7.6.5.2 asks for one warning to the
    /// player, not one per request, and games re-select the stream every turn.
    warned_stream2: bool,
    /// v5/v6 mouse state (ZMSD §15/§8). Set by [`set_mouse`](Machine::set_mouse)
    /// when the host reports a click; `read_mouse` (EXT:0x16) reports these back.
    /// Coordinates are game pixels, 1-based (ZMSD §8.8.1 coordinate convention).
    mouse_x: u16,
    mouse_y: u16,
    /// Button bitmask (bit 0 = primary, per ZMSD §15 read_mouse ordering).
    mouse_buttons: u16,
    /// Mouse-confinement window from `mouse_window` (EXT:0x17); −1 = no
    /// constraint. Defaults to 1 per ZMSD §15 ("By default it sits in window 1").
    /// Recorded for observability; the host owns actual pointer confinement.
    mouse_window: i16,
    /// True while a v6 newline-interrupt routine (window prop 8) is running, so a
    /// newline the routine itself emits cannot recursively re-fire the interrupt
    /// (ZMSD §8.8.3.2.2). Frotz relies on the zeroed countdown alone; this flag is
    /// a hang-safety belt for a spec-violating routine that both prints and re-arms
    /// prop 9. See [`newline_interrupt`](Machine::newline_interrupt).
    newline_interrupt_active: bool,
    /// Set by [`restart`](Machine::restart) so the host can drop app-side chrome
    /// (e.g. a v6 picture-canvas cache) that the VM's own screen reset cannot
    /// reach. Cleared by the host when observed. Not part of saved state.
    pub just_restarted: bool,
}

/// Context captured when the `save` opcode fires, needed by `complete_save`.
struct PendingSave {
    /// v3: the branch descriptor; v4+: the store variable number.
    result_dest: SaveDest,
    /// Address of the instruction's result descriptor (Quetzal §5.8): the store
    /// byte (v4+) or the first branch byte (v3). Written into the save file's PC.
    descriptor_pc: u32,
}

enum SaveDest {
    Branch(crate::cpu::decode::Branch),
    Store(u8),
}

/// Build the freshly-booted execution and screen state for `mem`'s version.
///
/// v3–8: header 0x06 is a direct instruction address; execution begins there
/// with an empty frame stack. v6: header 0x06 is the *packed address of `main`*,
/// which the interpreter enters with no args/result when the game starts up
/// (ZMSD §5.4) — `call_routine` pushes main's frame; when main returns, `step`
/// sees an empty v6 frame stack and quits (returning from main is illegal per
/// §5.4, so that only happens on a real `@quit`). The v6 window model is seeded
/// to Frotz's `restart_screen` defaults.
///
/// Shared by [`Machine::with_output`] (boot) and [`Machine::restart`] (@restart
/// re-boot) so both land in byte-identical initial state.
fn boot_state_and_screen(mem: &mut Memory) -> (State, ScreenState) {
    let mut state = State::new(mem.initial_pc());
    let mut screen = ScreenState::default();
    if mem.version() == 6 {
        state = State::new(0);
        let main_packed = mem.read_word(0x06);
        call_routine(&mut state, mem, main_packed, &[], None);
        // ZMSD §8.8.1: coordinates are 1-based with (1,1) top-left, and "all
        // eight windows begin at (1,1)"; each window's cursor likewise starts at
        // its own (1,1). Frotz restart_screen also seeds every window's font
        // props: font = TEXT_FONT (1) and font_size = (font_height << 8) |
        // font_width — games read the width back out of prop 13 for layout math
        // (Shogun sizes its READ input buffer with it; 0 there means zero-length
        // input forever).
        let mut v6 = V6Windows::default();
        for w in v6.windows.iter_mut() {
            w.y_coord = 1;
            w.x_coord = 1;
            w.y_cursor = 1;
            w.x_cursor = 1;
            w.font_number = 1;
            w.font_size =
                (crate::screen::V6_FONT_HEIGHT << 8) | crate::screen::V6_FONT_WIDTH;
            // frotz restart_screen: attribute 8 (buffered) everywhere...
            w.attributes = 8;
        }
        // ...and 15 (wrapping+scrolling+scripting+buffering) on window 0.
        v6.windows[0].attributes = 15;
        // ZMSD §8.8.3.3: "Window 0 occupies the whole screen and is initially
        // selected. Window 1 is as wide as the screen but has zero height.
        // Windows 2 to 7 have zero width and height." So windows 0 and 1 both
        // get the full screen width (pixels in v6 — games read it back via
        // get_wind_prop before ever calling window_size) and window 0 also gets
        // the full screen HEIGHT; window 1 stays at height 0. Reseeded with the
        // real screen size by `set_screen_dims` when the host reports it.
        let width = crate::screen::DEFAULT_SCREEN_COLS as u16 * crate::screen::V6_FONT_WIDTH;
        let height = crate::screen::DEFAULT_SCREEN_ROWS as u16 * crate::screen::V6_FONT_HEIGHT;
        v6.windows[0].x_size = width;
        v6.windows[0].y_size = height;
        v6.windows[1].x_size = width;
        screen.v6 = Some(v6);
    }
    (state, screen)
}

impl Machine {
    /// Create a new `Machine` from story memory, using a `BufferOutput` sink.
    /// `state.pc` is set to the header's `initial_pc` field (direct instruction
    /// address for v3/4/5/7/8; v6 is not supported).
    pub fn new(mem: Memory) -> Machine {
        Machine::with_output(mem, Box::new(BufferOutput::new()))
    }

    /// Create a new `Machine` with a custom output sink.
    pub fn with_output(mut mem: Memory, out: Box<dyn Output>) -> Machine {
        // Capture original dynamic memory for Quetzal CMem XOR diff.
        let dyn_len = mem.static_mem_base() as usize;
        let original_dynamic = mem.raw_bytes()[..dyn_len].to_vec();

        // Freshly-booted execution + screen state (v6 enters `main`; v1–5 start
        // at the initial PC). Shared with `restart` so @restart re-boots identically.
        let (state, screen) = boot_state_and_screen(&mut mem);

        Machine {
            state,
            mem,
            out,
            pending_input: None,
            screen,
            streams: StreamState::new(),
            original_dynamic,
            undo_stack: Vec::new(),
            undo_cap: 16,
            pending_save: None,
            pending_restore_store: None,
            pending_restore: false,
            rng_state: 0x12345678, // fixed nonzero seed
            warned_var_opcodes: std::collections::HashSet::new(),
            warned_ext_opcodes: std::collections::HashSet::new(),
            pending_sounds: Vec::new(),
            picture_dims: Vec::new(),
            pending_pictures: Vec::new(),
            v6_win0_out_chars: 0,
            v6_input_window: 0,
            diagnostics: Vec::new(),
            aux_data: std::collections::BTreeMap::new(),
            aux_dirty: false,
            honor_game_colours: false,
            sound_available: false,
            interpreter_number: None,
            default_bg_colour: crate::screen::DEFAULT_BG_COLOUR,
            default_fg_colour: crate::screen::DEFAULT_FG_COLOUR,
            fault_trace: None,
            cur_instr_pc: 0,
            trace_screen: false,
            screen_trace: Vec::new(),
            trace_exec: false,
            exec_pcs: std::collections::HashSet::new(),
            ever_exec_pcs: std::collections::HashSet::new(),
            buffer_screen_mode: 0,
            warned_stream2: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_buttons: 0,
            mouse_window: 1,
            newline_interrupt_active: false,
            just_restarted: false,
        }
    }

    /// Set interpreter capability bits in the story header (ZMSD §11.1).
    ///
    /// Call this once after loading a real story file and before running.
    /// Not called automatically by `new`/`with_output` because the writes
    /// overlap with address 0x10 (Flags2) which test programs occupy at that
    /// same address — real story files have static programs above 0x40.
    ///
    /// # Caller
    /// Call this from the host after loading a real story file, before the first
    /// `step()`. Not needed for test harnesses built from `sample_story` (whose
    /// buffers may overlap header bytes).
    pub fn init_caps(&mut self) {
        init_header_caps(&mut self.mem, self.honor_game_colours, self.sound_available, self.interpreter_number);
        // Re-apply the host's chosen interpreter defaults over the 2/9 seed
        // `init_header_caps` writes, so they survive `@restart` too.
        write_default_colours(&mut self.mem, self.default_bg_colour, self.default_fg_colour);
        // Communicate the initial buffer_mode state (false = off) to the sink.
        self.out.set_buffer_mode(self.screen.buffer_mode);
    }

    /// Re-boot the machine in place for the `@restart` opcode (ZMSD §6.1.3):
    /// "the entire state is restored from the original story file, and the stack
    /// is emptied; but 'Flags 2' is preserved".
    ///
    /// Concretely: dynamic memory is reloaded from the pristine boot image, the
    /// call/eval stack is emptied, and execution resumes exactly as at boot —
    /// the initial PC (v3–5/7/8) or by re-entering the packed `main` routine
    /// (v6, §5.4). Per §6.1.3 the two game-writable `Flags 2` bits — transcription
    /// (bit 0) and fixed-pitch (bit 1) — survive; the interpreter re-stamps the
    /// rest of the header capability bits (`init_caps`) and re-applies the
    /// host-reported screen size so the reboot lands where boot did (v6's 640×400
    /// unit screen). This mirrors Frotz `z_restart`, which rereads dynamic memory
    /// then calls `restart_header`/`restart_screen`, preserving only those two
    /// bits. The v6 window model and all queued picture/sound events are reset to
    /// boot defaults so no pre-restart screen chrome carries over, and the undo
    /// stack is discarded. Interpreter configuration (colour/sound/interpreter
    /// number/picture dims) and persistent aux data are unchanged, exactly as at
    /// boot.
    pub fn restart(&mut self) {
        // Preserve the two game-set Flags 2 bits that outlive a restart, and the
        // host screen size (init_caps below re-seeds only the generic default).
        let preserved_flags2 = self.mem.read_word(0x10) & 0b11;
        let rows = self.mem.read_byte(0x20);
        let cols = self.mem.read_byte(0x21);

        // Reload dynamic memory to its pristine boot image.
        for (i, &b) in self.original_dynamic.iter().enumerate() {
            self.mem.write_byte(i as u32, b);
        }

        // Fresh execution + screen model, identical to a cold boot.
        let (state, screen) = boot_state_and_screen(&mut self.mem);
        self.state = state;
        self.screen = screen;

        // Empty the stack and drop everything transient so no stale chrome or
        // half-finished I/O survives the reboot.
        self.streams = StreamState::new();
        self.undo_stack.clear();
        self.pending_input = None;
        self.pending_save = None;
        self.pending_restore_store = None;
        self.pending_restore = false;
        self.pending_sounds.clear();
        self.pending_pictures.clear();
        self.buffer_screen_mode = 0;
        self.v6_win0_out_chars = 0;
        self.v6_input_window = 0; // a reboot is back to the classic main window (SQ-0585)
        self.newline_interrupt_active = false;

        // Re-stamp the interpreter capability bits over the pristine header, then
        // restore the preserved Flags 2 bits and the host screen dimensions.
        self.init_caps();
        let f2 = (self.mem.read_word(0x10) & !0b11) | preserved_flags2;
        self.mem.write_word(0x10, f2);
        if rows > 0 && cols > 0 {
            self.set_screen_dims(rows, cols);
        }

        // Tell the host to drop app-side chrome the VM reset cannot reach.
        self.just_restarted = true;
    }

    /// Resolve a v6 window operand: `-3` (0xFFFD) means the CURRENTLY selected
    /// window (ZMSD §8.8.3.2; frotz `winarg0`). Shogun addresses most of its
    /// status-line cursor/property reads through window -3 — leaving it
    /// unresolved made every such read return 0 and scrambled its layout
    /// math. Other values pass through (downstream bounds checks ignore
    /// out-of-range windows).
    fn v6_window_operand(&self, w: u16) -> u16 {
        if w == 0xFFFD {
            if let Some(v6) = self.screen.v6.as_ref() {
                return v6.current as u16;
            }
        }
        w
    }

    /// Keep `screen.current_fg`/`current_bg` mirroring the CURRENT v6 window's
    /// colour pair.
    ///
    /// ZMSD §8.3 gives each Version 6 window its own foreground/background pair,
    /// but the prose stream (window 0's scrolling text) tags its `TextAttrs` from
    /// `current_fg`/`current_bg` — the fields only the non-v6 `set_colour` path
    /// used to write. Without this mirror, v6 prose always printed in the
    /// interpreter default no matter what the game selected. Call with the new
    /// pair whenever the current window's colours change (`set_colour`,
    /// `set_true_colour`) or a different window becomes current (`set_window`,
    /// `erase_window` -1); `None` means "nothing to mirror".
    fn mirror_v6_colours(&mut self, pair: Option<(crate::screen::ZColour, crate::screen::ZColour)>) {
        if let Some((fg, bg)) = pair {
            self.screen.current_fg = fg;
            self.screen.current_bg = bg;
        }
    }

    /// Report the screen size to the story: writes the header dimension fields
    /// (see [`write_screen_dims`]) and, for v6, reseeds windows 0 and 1 with
    /// the new screen width in pixels (frotz restart_screen) so games that
    /// read window widths via `get_wind_prop` before sizing anything see the
    /// real screen, not the boot-time default. Window 0 also takes the new
    /// screen HEIGHT (ZMSD §8.8.3.3: "Window 0 occupies the whole screen");
    /// window 1 keeps its zero height until a `split_window`.
    ///
    /// For v4/v5/v7/v8 a LIVE upper window follows the new WIDTH — see
    /// [`Machine::refit_upper_window_width`].
    pub fn set_screen_dims(&mut self, rows: u8, cols: u8) {
        crate::screen::write_screen_dims(&mut self.mem, rows, cols);
        if self.mem.version() == 6 {
            if let Some(v6) = self.screen.v6.as_mut() {
                let width = cols.max(1) as u16 * crate::screen::V6_FONT_WIDTH;
                let height = rows.max(1) as u16 * crate::screen::V6_FONT_HEIGHT;
                v6.windows[0].x_size = width;
                v6.windows[0].y_size = height;
                v6.windows[1].x_size = width;
            }
            return;
        }
        self.refit_upper_window_width(cols);
    }

    /// Make an already-open v4+ upper window follow a host screen-width change.
    ///
    /// ZMSD §8.4: "The interpreter may change the exact dimensions whenever it
    /// likes but must write the current height (in lines) and width (in
    /// characters) into bytes $20 and $21 in the header." Only `split_window`
    /// used to size the grid (from $21), so a game that splits ONCE at boot and
    /// never re-splits — Sherlock, Trinity — kept its boot-time grid width for
    /// the rest of the session while $20/$21 tracked the resized pane: the
    /// status bar stayed 80 columns in a 100-column pane (or overflowed a
    /// narrowed one). AMFV only self-healed because it re-splits on every mode
    /// change. The grid is the interpreter's own storage for the screen the
    /// spec just let us resize, so it is resized here too.
    ///
    /// WIDTH only. The split HEIGHT is the game's (`split_window`, §8.7.2.1),
    /// so `rows` is left exactly as the game last set it — including any extra
    /// rows `grow_rows` added for writes below the split. Content is preserved
    /// per the §15 v4+ rule that a re-split leaves the upper window on screen:
    /// growing pads right with blanks, shrinking truncates
    /// ([`UpperWindow::resize_preserving`]).
    ///
    /// CURSOR: a shrink can leave `cursor_col` past the new right edge, which
    /// §8.7.2.3 makes illegal ("It is illegal to move the cursor outside the
    /// current size of the upper window"), so it is clamped to the last column
    /// — a minimal sideways move that keeps the row. That follows the one
    /// V4/V5-scoped precedent for an interpreter-forced cursor relocation,
    /// §8.7.2.2: "If a split takes place which would cause the upper window to
    /// swallow the lower window's cursor position, the interpreter should move
    /// the lower window's cursor down to the line just below the upper window's
    /// new size" — i.e. nudge onto the nearest legal position on the axis that
    /// went out of range, don't re-home. (§8.8.3.4's "reset to the left margin
    /// on the top line" is the Version 6 window-property rule and stays with
    /// `window_size`; homing here would silently discard a row the game set
    /// before a resize it never asked for.)
    ///
    /// v3 is out of scope: its status line is recomputed from the globals every
    /// turn, and `write_screen_dims` does not touch $20/$21 below v4 at all.
    fn refit_upper_window_width(&mut self, cols: u8) {
        if self.mem.version() < 4 || self.screen.upper.rows == 0 {
            return;
        }
        let new_cols = cols.max(1) as u16;
        if new_cols == self.screen.upper.cols {
            return;
        }
        let rows = self.screen.upper.rows;
        // A WIDEN continues each row's trailing appearance into the columns that
        // appear (SQ-0679): the game never asked for them and will never paint
        // them, so leaving them at the interpreter default cut the game's own
        // status band short of its right edge. A shrink is plain truncation.
        self.screen.upper.resize_continuing_row_style(rows, new_cols);
        if self.screen.cursor_col > new_cols {
            self.screen.cursor_col = new_cols;
        }
    }

    /// Record a mouse click at game-pixel `(y, x)` (1-based, ZMSD §8.8.1: "the
    /// origin of the screen is at the top left, so that the coordinates of the
    /// top left pixel are (1,1)") with `buttons` as the button bitmask (bit 0 =
    /// primary). Called by the host when the player clicks inside the v6 image.
    ///
    /// Besides recording the state for `read_mouse` (EXT:0x16), this writes the
    /// header extension table so a game that reads the coordinates directly from
    /// the header — rather than via `read_mouse` — also sees the click. Per the
    /// ZMSD §11 header-extension table layout, word 1 holds "X-coordinate of
    /// mouse after a click" and word 2 holds "Y-coordinate of mouse after a
    /// click" (note the X-before-Y order, the reverse of `read_mouse`'s array).
    /// Word 0 is the count of further words; the writes are skipped when the
    /// table is absent or too short (ZMSD §11: "If the interpreter needs to read
    /// a word which is beyond the length of the extension table, or the
    /// extension table doesn't exist at all, then the result is 0").
    pub fn set_mouse(&mut self, y: u16, x: u16, buttons: u16) {
        self.mouse_y = y;
        self.mouse_x = x;
        self.mouse_buttons = buttons;
        // Header 0x36 holds the byte address of the extension table (0 = none).
        let ext = self.mem.read_word(0x36) as u32;
        if ext == 0 {
            return;
        }
        let count = self.mem.read_word(ext); // word 0: number of further words
        if count >= 1 {
            self.mem.write_word(ext + 2, x); // word 1: mouse X-coordinate
        }
        if count >= 2 {
            self.mem.write_word(ext + 4, y); // word 2: mouse Y-coordinate
        }
    }

    /// ZMSD §7.4: games (all Infocom-era ones) turn transcription on by setting
    /// bit 0 of Flags 2 rather than issuing `output_stream 2` — the interpreter
    /// is expected to watch the bit. We support no transcript FILE, so on
    /// seeing the bit set, warn once (same diagnostic as `output_stream 2`) and
    /// CLEAR it — the game then honestly reports scripting as off instead of
    /// believing a transcript is being written. Checked at every input request
    /// (the turn boundary), which is where a SCRIPT verb's effect first
    /// becomes observable.
    fn check_transcript_bit(&mut self) {
        const FLAGS2: u32 = 0x10;
        let flags = self.mem.read_word(FLAGS2);
        if flags & 1 != 0 {
            self.mem.write_word(FLAGS2, flags & !1);
            if !self.warned_stream2 {
                self.warned_stream2 = true;
                self.diagnostics.push(
                    "transcript file output isn't supported — the game's script command will have no effect (the app keeps its own scrollback)".to_string(),
                );
            }
        }
    }

    /// Remember which v6 window the game is reading input through (SQ-0585), at
    /// every `read`/`read_char`.
    ///
    /// Only a flowing-prose window counts: a game that reads a keypress while a
    /// PAINT window is current is running a menu (Shogun's boot menu reads through
    /// its 1px caret window), and that must not redesignate the main text window —
    /// doing so would divert the whole transcript into a window buffer.
    fn note_v6_input_window(&mut self) {
        if let Some(v6) = self.screen.v6.as_ref() {
            let cur = v6.current as usize;
            if v6.windows[cur].prose_window() {
                self.v6_input_window = v6.current;
            }
        }
    }

    /// Enable/disable honoring game-driven colour. Advertises (or clears) the
    /// Flags1 colour bit immediately so a not-yet-run game sees the capability.
    pub fn set_honor_game_colours(&mut self, on: bool) {
        self.honor_game_colours = on;
        advertise_colour(&mut self.mem, on);
    }

    /// Enable/disable advertising sound-effects capability. Advertises (or
    /// clears) the sound header bits immediately so a not-yet-run game sees
    /// the capability.
    pub fn set_sound_available(&mut self, on: bool) {
        self.sound_available = on;
        advertise_sound(&mut self.mem, on);
    }

    /// Inject the v6 picture-dimension table `picture_data` answers from:
    /// `(picture_number, width_px, height_px)` triples. The host builds this
    /// from the self-blorb's `Pict` resources before the boot run (Task 9).
    pub fn set_picture_dims(&mut self, t: Vec<(u16, u16, u16)>) {
        self.picture_dims = t;
    }

    /// Set the interpreter number to advertise (header 0x1E). `None` restores the
    /// auto default (Frotz's rule). Takes effect at the next `init_caps`.
    pub fn set_interpreter_number(&mut self, n: Option<u8>) {
        self.interpreter_number = n;
    }

    /// Publish the interpreter's own default colours to the game.
    ///
    /// ZMSD §8.3.3: a colour-capable interpreter "should ... write its default
    /// background and foreground colours into bytes $2c and $2d of the header"
    /// — i.e. the colours the host actually paints unstyled text in, which games
    /// read back to build their palettes (Beyond Zork does exactly this).
    /// Arguments are standard colour numbers (ZMSD §8.3.1); anything outside
    /// 2..=9 falls back to black background (2) / white foreground (9).
    ///
    /// Writes the header immediately (so it may be called after boot) and is
    /// re-applied at every `init_caps`, including the one `@restart` performs.
    /// No-op below V5, where $2C/$2D are not colour bytes.
    pub fn set_default_colours(&mut self, bg: u8, fg: u8) {
        use crate::screen::{clamp_default_colour, DEFAULT_BG_COLOUR, DEFAULT_FG_COLOUR};
        self.default_bg_colour = clamp_default_colour(bg, DEFAULT_BG_COLOUR);
        self.default_fg_colour = clamp_default_colour(fg, DEFAULT_FG_COLOUR);
        write_default_colours(&mut self.mem, self.default_bg_colour, self.default_fg_colour);
    }

    /// Seed the cumulative "ever executed" set from host-persisted knowledge
    /// (the debug PC-set sidecar) so prior runs' coverage colours immediately.
    /// Independent of `trace_exec` — the host seeds regardless of tracing state.
    pub fn seed_executed(&mut self, pcs: impl IntoIterator<Item = u32>) {
        self.ever_exec_pcs.extend(pcs);
    }

    /// Borrow the default `BufferOutput` sink if that is what `out` holds, else `None`.
    pub fn buffer_output(&self) -> Option<&BufferOutput> {
        self.out.as_any().downcast_ref::<BufferOutput>()
    }

    /// Execute one instruction and return the result.
    ///
    /// Contract (per task spec):
    ///   1. Decode instruction at `state.pc`.
    ///   2. Advance `state.pc` to `instr.next_pc` BEFORE executing.
    ///   3. Execute the instruction.
    ///
    /// This ensures `state.pc` already points past the call site when
    /// `call_routine` is invoked, giving the correct `return_pc`. Branch and
    /// jump offsets are computed relative to `state.pc` (= next_pc) too.
    pub fn step(&mut self) -> StepResult {
        let version = self.mem.version();
        let instr_start_pc = self.state.pc;
        self.cur_instr_pc = instr_start_pc;
        if self.trace_exec {
            self.exec_pcs.insert(instr_start_pc);
            self.ever_exec_pcs.insert(instr_start_pc);
        }
        let instr = decode(&self.mem, self.state.pc, version);
        let op_name = opcode_name(instr.operand_count.clone(), instr.opcode);

        // CRITICAL: advance PC before executing so call/branch targets are correct.
        self.state.pc = instr.next_pc;

        let result = self.execute(instr);

        // A latched OOB access or stack underflow overrides the normal result.
        // Drain BOTH latches unconditionally first so a single instruction that
        // fires both never leaks the undrained one into the next step().
        let mem_fault = self.mem.take_mem_fault();
        let state_fault = self.state.fault.take();
        if let Some((is_write, size, addr)) = mem_fault {
            let kind = if is_write { "write".to_string() } else { format!("read{}", size as u32 * 8) };
            let msg = format!("memory fault: {kind} @{addr:#010x}");
            self.fault_trace = Some(self.build_trace(msg, instr_start_pc, op_name));
            return StepResult::Fault;
        }
        if let Some(msg) = state_fault {
            self.fault_trace = Some(self.build_trace(msg, instr_start_pc, op_name));
            return StepResult::Fault;
        }

        // v6: `main` runs as the base call frame, so an empty frame stack means
        // it returned — the story is over (ZMSD §5.5). v3–8 run frameless and
        // end via @quit, so this only triggers for v6.
        if version == 6 && self.state.frames.is_empty() {
            return StepResult::Quit;
        }
        result
    }

    /// Take and clear the stack trace captured at the last fault.
    pub fn take_fault_trace(&mut self) -> Option<crate::cpu::trace::StackTrace> {
        self.fault_trace.take()
    }

    fn build_trace(&self, fault: String, fault_pc: u32, fault_op: String)
        -> crate::cpu::trace::StackTrace
    {
        use crate::cpu::trace::{StackTrace, TraceFrame};
        let st = &self.state;
        let n = st.frames.len();
        let mut frames = Vec::with_capacity(n);
        // Innermost (last) frame first.
        for i in (0..n).rev() {
            let f = &st.frames[i];
            let upper = st.frames.get(i + 1).map(|nf| nf.eval_base).unwrap_or(st.eval_stack.len());
            // Defensive clamp: a corrupt stack must never panic a trace builder
            // that exists to report a fault. Identical to the unclamped slice
            // under the normal (valid-invariant) case.
            let lo = f.eval_base.min(st.eval_stack.len());
            let hi = upper.min(st.eval_stack.len());
            let hi = hi.max(lo);
            let operands = st.eval_stack[lo..hi]
                .iter().map(|&w| w as i64).collect();
            frames.push(TraceFrame {
                func_addr: f.func_addr,
                return_pc: f.return_pc,
                locals: f.locals.iter().map(|&w| w as i64).collect(),
                operands,
            });
        }
        StackTrace { fault, fault_pc, fault_op, width: 2, frames }
    }

    // -----------------------------------------------------------------------
    // Main dispatch
    // -----------------------------------------------------------------------

    fn execute(&mut self, instr: Instr) -> StepResult {
        // v6 `pull stack -> (result)` (ZMSD §15, frotz z_pull V6 branch): with an
        // operand — of ANY encoding, resolved like every other operand — its value
        // is a *user*-stack address: pop one word off it (bump the free-slot
        // count, read the freed slot). With no operand, pop the game stack. In
        // both cases store the popped value (the decoder always reads the store
        // byte for v6 pull; see decode.rs). The v1-5 form (operand = destination
        // variable) is handled by the normal 0x09 arm below.
        if matches!(instr.operand_count, OperandCount::Var)
            && instr.opcode == 0x09
            && self.mem.version() == 6
        {
            let value = if let Some(op) = instr.operands.first() {
                let addr = self.resolve(op) as u32;
                let size = self.mem.read_word(addr).wrapping_add(1);
                self.mem.write_word(addr, size);
                self.mem.read_word(addr.wrapping_add(2u32.wrapping_mul(size as u32)))
            } else {
                read_var(&mut self.state, &self.mem, 0) // pop the game stack
            };
            self.do_store(instr.store, value);
            return StepResult::Continue;
        }

        // Resolve all operands left-to-right (Var operands can pop the stack).
        let ops: Vec<u16> = instr
            .operands
            .iter()
            .map(|op| self.resolve(op))
            .collect();

        match instr.operand_count {
            OperandCount::Two => self.exec_2op(instr.opcode, &ops, instr.store, instr.branch),
            OperandCount::One => self.exec_1op(instr.opcode, &ops, instr.store, instr.branch),
            OperandCount::Zero => self.exec_0op(instr.opcode, instr.store, instr.branch, instr.text),
            OperandCount::Var => self.exec_var(instr.opcode, &ops, instr.store, instr.branch),
            OperandCount::Ext => self.exec_ext(instr.opcode, &ops, instr.store, instr.branch),
        }
    }

    // -----------------------------------------------------------------------
    // 2OP opcodes
    // -----------------------------------------------------------------------

    fn exec_2op(
        &mut self,
        opcode: u8,
        ops: &[u16],
        store: Option<u8>,
        branch: Option<Branch>,
    ) -> StepResult {
        let a = ops.first().copied().unwrap_or(0);
        let b = ops.get(1).copied().unwrap_or(0);

        match opcode {
            // 0x01 je — branch if a equals ANY of ops[1..]
            // Variable form allows up to 4 operands (ZMSD §14).
            0x01 => {
                let cond = ops.len() > 1 && ops[1..].contains(&a);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x02 jl — branch if a < b (signed)
            0x02 => {
                let cond = (a as i16) < (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x03 jg — branch if a > b (signed)
            0x03 => {
                let cond = (a as i16) > (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x04 dec_chk — decrement variable (ops[0] = var number), branch if new_val < b.
            // ZMSD §14: first operand is a "variable by reference" — its *value* is the
            // variable number to operate on. In the Long form this is a Small constant
            // (the var number); in the Var form it may be a Var (whose contents are
            // the var number). ops[0] already holds the variable number correctly in
            // both cases after normal operand resolution.
            0x04 => {
                let var = a as u8;
                let old = read_var(&mut self.state, &self.mem, var);
                let new_val = (old as i16).wrapping_sub(1) as u16;
                write_var(&mut self.state, &mut self.mem, var, new_val);
                let cond = (new_val as i16) < (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x05 inc_chk — increment variable (ops[0] = var number), branch if new_val > b.
            0x05 => {
                let var = a as u8;
                let old = read_var(&mut self.state, &self.mem, var);
                let new_val = (old as i16).wrapping_add(1) as u16;
                write_var(&mut self.state, &mut self.mem, var, new_val);
                let cond = (new_val as i16) > (b as i16);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x06 jin — branch if obj a is a child of obj b (parent(a) == b)
            0x06 => {
                let parent = objects::get_parent(&self.mem, a);
                self.do_branch(branch, parent == b);
                StepResult::Continue
            }
            // 0x07 test — branch if all bits in b are set in a (bitmap test, ZMSD §15)
            0x07 => {
                self.do_branch(branch, a & b == b);
                StepResult::Continue
            }
            // 0x0A test_attr — branch if object a has attribute b set (ZMSD §14)
            0x0A => {
                let cond = objects::get_attr(&self.mem, a, b as u8);
                self.do_branch(branch, cond);
                StepResult::Continue
            }
            // 0x0B set_attr — set attribute b on object a (side effect only)
            0x0B => {
                objects::set_attr(&mut self.mem, a, b as u8);
                StepResult::Continue
            }
            // 0x0C clear_attr — clear attribute b on object a (side effect only)
            0x0C => {
                objects::clear_attr(&mut self.mem, a, b as u8);
                StepResult::Continue
            }
            // 0x0E insert_obj — make object a the first child of object b
            0x0E => {
                if !objects::insert_obj(&mut self.mem, a, b) {
                    self.state.fault = Some(format!("insert_obj: sibling cycle in object table (object {a})"));
                }
                StepResult::Continue
            }
            // 0x0F loadw — load word from array: result = mem[a + 2*b].
            // The address is computed in 16-bit space and wraps at 0xFFFF (as the
            // reference interpreter does), so a negative word-index (e.g. b=0xFFFF
            // meaning -1) reads the word *before* the array rather than a huge OOB
            // address. Regression-guarded by praxix's "Array loads and stores"
            // group (crates/zvm/tests/regression.rs).
            0x0F => {
                let addr = a.wrapping_add(2u16.wrapping_mul(b)) as u32;
                let result = self.mem.read_word(addr);
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x10 loadb — load byte from array: result = mem[a + b] (16-bit wrap).
            0x10 => {
                let addr = a.wrapping_add(b) as u32;
                let result = self.mem.read_byte(addr) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x11 get_prop — store property b of object a (fallback to default)
            0x11 => {
                let val = objects::get_prop(&self.mem, a, b as u8);
                self.do_store(store, val);
                StepResult::Continue
            }
            // 0x12 get_prop_addr — store address of property b data in object a (0 if absent)
            0x12 => {
                let addr = objects::get_prop_addr(&self.mem, a, b as u8);
                self.do_store(store, addr);
                StepResult::Continue
            }
            // 0x13 get_next_prop — store next property number after b in object a (0=last/first)
            0x13 => {
                let next = objects::get_next_prop(&self.mem, a, b as u8);
                self.do_store(store, next as u16);
                StepResult::Continue
            }
            // 0x08 or — bitwise OR
            0x08 => {
                self.do_store(store, a | b);
                StepResult::Continue
            }
            // 0x09 and — bitwise AND
            0x09 => {
                self.do_store(store, a & b);
                StepResult::Continue
            }
            // 0x0D store — write value b into variable a (by reference).
            // ZMSD §6.3.4: if variable number == 0, REPLACE (do not push) the stack top.
            0x0D => {
                let var = a as u8;
                if var == 0 {
                    poke_stack(&mut self.state, b);
                } else {
                    write_var(&mut self.state, &mut self.mem, var, b);
                }
                StepResult::Continue
            }
            // 0x14 add (signed)
            0x14 => {
                let result = (a as i16).wrapping_add(b as i16) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x15 sub (signed)
            0x15 => {
                let result = (a as i16).wrapping_sub(b as i16) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x16 mul (signed)
            0x16 => {
                let result = (a as i16).wrapping_mul(b as i16) as u16;
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x17 div (signed); division by zero → 0 (ZMSD §15 "interpreter may halt or trap")
            //
            // wrapping_div: i16::MIN / -1 overflows i16 — a plain `/` panics in
            // both debug and release. Reference interpreters (Frotz) compute in
            // C `int` and truncate back to 16 bits, so 0x8000 / 0xFFFF = 32768
            // → 0x8000; `wrapping_div` produces exactly that.
            0x17 => {
                let result = if b == 0 { 0 } else { (a as i16).wrapping_div(b as i16) as u16 };
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x18 mod (signed); mod by zero → 0. wrapping_rem for the same
            // i16::MIN % -1 overflow as div above (result 0).
            0x18 => {
                let result = if b == 0 { 0 } else { (a as i16).wrapping_rem(b as i16) as u16 };
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x19 call_2s — call with one arg, store result (v4+)
            0x19 => {
                call_routine(&mut self.state, &mut self.mem, a, &[b], store);
                StepResult::Continue
            }
            // 0x1A call_2n — call with one arg, discard result (v5+)
            0x1A => {
                call_routine(&mut self.state, &mut self.mem, a, &[b], None);
                StepResult::Continue
            }
            // 2OP:0x1B set_colour (v5+). Per-channel replace with sentinels
            // (ZMSD §8.3): 0 = keep, 1 = default, 2..=12 = palette + v6 greys.
            //
            // v6 (ZMSD §15) extends this to 3 operands (fg, bg, window; window
            // defaults to current). It reuses the SAME opcode number (2OP:27 =
            // 0x1B) rather than a VAR-numbered opcode — the ZMSD says the
            // 3-operand form "uses the same opcode number but in variable-length
            // encoding". Confirmed against decode.rs: a 2OP opcode encoded in
            // variable form (top bits 11, bit5=0) is classified OperandCount::Two
            // but reads its operands via a type byte just like VAR, so it can
            // carry up to 4 operands (see `Form::Variable` in decode.rs and the
            // `je`-with-4-operands handling at 0x01 above). So a 3-operand
            // set_colour decodes as OperandCount::Two and lands HERE in
            // exec_2op with `ops.len() == 3`, never in exec_var.
            0x1B => {
                if let Some(v6) = self.screen.v6.as_mut() {
                    let win = ops.get(2).copied().map(|w| (if w == 0xFFFD { v6.current as u16 } else { w }) as u8).unwrap_or(v6.current);
                    if self.trace_screen {
                        let fg = decode_set_colour_v6(a).map(zscreen_colour_name).unwrap_or_else(|| a.to_string());
                        let bg = decode_set_colour_v6(b).map(zscreen_colour_name).unwrap_or_else(|| b.to_string());
                        self.screen_trace.push(format!("@set_colour(fg={fg}, bg={bg}, window={win})"));
                    }
                    let mut mirror = None;
                    if let Some(w) = v6.windows.get_mut(win as usize) {
                        if let Some(c) = decode_set_colour_v6(a) {
                            w.fg = c;
                        }
                        if let Some(c) = decode_set_colour_v6(b) {
                            w.bg = c;
                        }
                        w.colour_data = pack_colour_data(w.fg, w.bg);
                        if win == v6.current {
                            mirror = Some((w.fg, w.bg));
                        }
                    }
                    self.mirror_v6_colours(mirror);
                } else {
                    if self.trace_screen {
                        let fg = decode_set_colour(a).map(zscreen_colour_name).unwrap_or_else(|| a.to_string());
                        let bg = decode_set_colour(b).map(zscreen_colour_name).unwrap_or_else(|| b.to_string());
                        self.screen_trace.push(format!("@set_colour(fg={fg}, bg={bg})"));
                    }
                    if let Some(c) = decode_set_colour(a) {
                        self.screen.current_fg = c;
                    }
                    if let Some(c) = decode_set_colour(b) {
                        self.screen.current_bg = c;
                    }
                }
                StepResult::Continue
            }
            // 2OP:0x1C throw value stack-frame (v5+) — non-local return. `catch`
            // (0OP:0x09) records the call-stack depth; `throw` unwinds back to that
            // depth and returns `value` from the catching routine (ZMSD §15).
            0x1C => {
                let value = a;
                let target_depth = b as usize;
                self.unwind_to_depth(target_depth);
                // Return `value` from the catching routine itself (defensive: skip
                // if the depth was invalid and nothing remains to return from).
                if !self.state.frames.is_empty() {
                    return_value(&mut self.state, &mut self.mem, value);
                }
                StepResult::Continue
            }
            // Unknown / unimplemented 2OP — no-op seam for Tasks 10+ (object/text ops)
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // 1OP opcodes
    // -----------------------------------------------------------------------

    fn exec_1op(
        &mut self,
        opcode: u8,
        ops: &[u16],
        store: Option<u8>,
        branch: Option<Branch>,
    ) -> StepResult {
        let a = ops.first().copied().unwrap_or(0);

        match opcode {
            // 0x00 jz — branch if a == 0
            0x00 => {
                self.do_branch(branch, a == 0);
                StepResult::Continue
            }
            // 0x01 get_sibling — store sibling of a AND branch if sibling != 0 (ZMSD §14)
            0x01 => {
                let sib = objects::get_sibling(&self.mem, a);
                self.do_store(store, sib);
                self.do_branch(branch, sib != 0);
                StepResult::Continue
            }
            // 0x02 get_child — store child of a AND branch if child != 0 (ZMSD §14)
            0x02 => {
                let child = objects::get_child(&self.mem, a);
                self.do_store(store, child);
                self.do_branch(branch, child != 0);
                StepResult::Continue
            }
            // 0x03 get_parent — store parent of a, no branch (ZMSD §14)
            0x03 => {
                let parent = objects::get_parent(&self.mem, a);
                self.do_store(store, parent);
                StepResult::Continue
            }
            // 0x04 get_prop_len — store length in bytes of property whose data address is a
            0x04 => {
                let len = objects::get_prop_len(&self.mem, a);
                self.do_store(store, len as u16);
                StepResult::Continue
            }
            // 0x07 print_addr — print string at byte address a
            0x07 => {
                let (s, _) = decode_string(&self.mem, a as u32);
                self.print_text(&s);
                StepResult::Continue
            }
            // 0x09 remove_obj — remove object a from its parent's child list
            0x09 => {
                if !objects::remove_obj(&mut self.mem, a) {
                    self.state.fault = Some(format!("remove_obj: sibling cycle in object table (object {a})"));
                }
                StepResult::Continue
            }
            // 0x0A print_obj — print the short name of object a via the output sink
            0x0A => {
                let name = objects::short_name(&self.mem, a);
                self.print_text(&name);
                StepResult::Continue
            }
            // 0x0D print_paddr — print string at packed address a
            0x0D => {
                let byte_addr = self.mem.unpack_string(a);
                let (s, _) = decode_string(&self.mem, byte_addr);
                self.print_text(&s);
                StepResult::Continue
            }
            // 0x05 inc — increment variable by reference (no store/branch)
            0x05 => {
                let var = a as u8;
                let v = read_var(&mut self.state, &self.mem, var);
                write_var(&mut self.state, &mut self.mem, var, v.wrapping_add(1));
                StepResult::Continue
            }
            // 0x06 dec — decrement variable by reference
            0x06 => {
                let var = a as u8;
                let v = read_var(&mut self.state, &self.mem, var);
                write_var(&mut self.state, &mut self.mem, var, v.wrapping_sub(1));
                StepResult::Continue
            }
            // 0x08 call_1s — call routine at packed addr a, no args, store result
            0x08 => {
                call_routine(&mut self.state, &mut self.mem, a, &[], store);
                StepResult::Continue
            }
            // 0x0B ret — return value a from current routine
            0x0B => {
                return_value(&mut self.state, &mut self.mem, a);
                StepResult::Continue
            }
            // 0x0C jump — unconditional; operand is signed i16 offset.
            // ZMSD §14: pc = pc + offset - 2 (where pc is already next_pc).
            0x0C => {
                let offset = a as i16;
                self.state.pc = (self.state.pc as i32 + offset as i32 - 2) as u32;
                StepResult::Continue
            }
            // 0x0E load — read value of variable a, store result.
            // ZMSD §6.3.4: if variable number == 0, PEEK (do not pop) the stack top.
            0x0E => {
                let var = a as u8;
                let val = if var == 0 {
                    peek_stack(&self.state)
                } else {
                    read_var(&mut self.state, &self.mem, var)
                };
                self.do_store(store, val);
                StepResult::Continue
            }
            // 0x0F not (v1–4, stores) / call_1n (v5+, no store)
            0x0F => {
                if self.mem.version() <= 4 {
                    self.do_store(store, !a);
                } else {
                    call_routine(&mut self.state, &mut self.mem, a, &[], None);
                }
                StepResult::Continue
            }
            // Unknown / unimplemented 1OP — no-op seam (object ops in Task 10)
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // 0OP opcodes
    // -----------------------------------------------------------------------

    // Branch and store are threaded through because verify/piracy need do_branch.
    fn exec_0op(
        &mut self,
        opcode: u8,
        store: Option<u8>,
        branch: Option<Branch>,
        text: Option<(String, u32)>,
    ) -> StepResult {
        match opcode {
            // 0x00 rtrue — return 1 from current routine
            0x00 => {
                return_value(&mut self.state, &mut self.mem, 1);
                StepResult::Continue
            }
            // 0x01 rfalse — return 0 from current routine
            0x01 => {
                return_value(&mut self.state, &mut self.mem, 0);
                StepResult::Continue
            }
            // 0x02 print — print the inline string (from Instr.text)
            0x02 => {
                if let Some((s, _)) = text {
                    self.print_text(&s);
                }
                StepResult::Continue
            }
            // 0x03 print_ret — print inline string + newline, then return true
            0x03 => {
                if let Some((s, _)) = text {
                    self.print_text(&s);
                }
                self.print_text("\n");
                return_value(&mut self.state, &mut self.mem, 1);
                StepResult::Continue
            }
            // 0x04 nop — no operation
            0x04 => StepResult::Continue,
            // 0x07 restart
            0x07 => StepResult::Restart,
            // 0x08 ret_popped — pop eval stack and return that value
            0x08 => {
                let val = read_var(&mut self.state, &self.mem, 0); // var 0 = pop
                return_value(&mut self.state, &mut self.mem, val);
                StepResult::Continue
            }
            // 0x09 pop (v1–4) / catch (v5+, stores frame depth)
            0x09 => {
                if self.mem.version() <= 4 {
                    // pop: discard top of eval stack
                    let _ = read_var(&mut self.state, &self.mem, 0);
                } else {
                    // catch: stores current call stack depth (frame count)
                    let depth = self.state.frames.len() as u16;
                    self.do_store(store, depth);
                }
                StepResult::Continue
            }
            // 0x0A quit
            0x0A => StepResult::Quit,
            // 0x0B new_line — print newline
            0x0B => {
                self.print_text("\n");
                StepResult::Continue
            }
            // 0OP:0x0D verify — checksum the story and branch on match.
            0x0D => {
                let header_ck = self.mem.read_word(0x1C);
                // If the header records no checksum (some dev builds), treat as genuine.
                let ok = header_ck == 0 || self.story_checksum() == header_ck;
                self.do_branch(branch, ok);
                StepResult::Continue
            }
            // 0OP:0x0F piracy — the standard says interpreters should behave as if the
            // game is genuine: always take the branch.
            0x0F => {
                self.do_branch(branch, true);
                StepResult::Continue
            }
            // 0x05 save — suspend and let the host serialise state (Task 14).
            // v3: branch on success; v4+: store result (1=ok, 0=fail).
            // PC has already advanced past the instruction (standard step() contract),
            // so state.pc at this point is the correct resume address.
            0x05 => {
                let (dest, descriptor_pc) = if self.mem.version() <= 3 {
                    // v3: save is a branch instruction; branch is present
                    match branch {
                        Some(b) => {
                            let dpc = self.state.pc - b.len as u32;
                            (SaveDest::Branch(b), dpc)
                        }
                        None => (SaveDest::Store(0), self.state.pc.saturating_sub(1)), // shouldn't happen; safe fallback
                    }
                } else {
                    // v4+: save is a store instruction; store is present
                    match store {
                        Some(sv) => (SaveDest::Store(sv), self.state.pc.saturating_sub(1)),
                        None => (SaveDest::Store(0), self.state.pc.saturating_sub(1)),
                    }
                };
                self.pending_save = Some(PendingSave { result_dest: dest, descriptor_pc });
                StepResult::SaveRequest
            }
            // 0x06 restore — suspend and let the host supply bytes (Task 14).
            // v3: branch on success; v4+: store result (2 = restored from save,
            // 0 = failure). The store byte is decoded by the decoder and passed
            // here; capture it so complete_restore_failure() can use it without
            // reading from state.pc (which has already advanced past the store byte).
            0x06 => {
                if self.mem.version() >= 4 {
                    self.pending_restore_store = store;
                }
                // v3 has no store byte to capture, but the suspension is just as
                // real — flag it separately so `is_saveload_pending` sees it.
                self.pending_restore = true;
                StepResult::RestoreRequest
            }
            // 0x0C show_status (v3 only) — signal host to redraw the status line
            0x0C => {
                self.screen.show_status_requested = true;
                StepResult::Continue
            }
            // Unknown / unimplemented 0OP — no-op
            _ => StepResult::Continue,
        }
    }

    // -----------------------------------------------------------------------
    // VAR opcodes
    // -----------------------------------------------------------------------

    fn exec_var(
        &mut self,
        opcode: u8,
        ops: &[u16],
        store: Option<u8>,
        branch: Option<Branch>,
    ) -> StepResult {
        match opcode {
            // 0x00 call / call_vs — call with up to 3 args, store result
            0x00 => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, store);
                StepResult::Continue
            }
            // 0x01 storew — store word: mem[ops[0] + 2*ops[1]] = ops[2].
            // Address computed in 16-bit space (wraps at 0xFFFF), matching the
            // reference interpreter, so negative indices address within memory.
            0x01 => {
                let array = ops.first().copied().unwrap_or(0);
                let index = ops.get(1).copied().unwrap_or(0);
                let val   = ops.get(2).copied().unwrap_or(0);
                self.mem.write_word(array.wrapping_add(2u16.wrapping_mul(index)) as u32, val);
                StepResult::Continue
            }
            // 0x02 storeb — store byte: mem[ops[0] + ops[1]] = ops[2] & 0xFF (16-bit wrap)
            0x02 => {
                let array = ops.first().copied().unwrap_or(0);
                let index = ops.get(1).copied().unwrap_or(0);
                let val   = (ops.get(2).copied().unwrap_or(0) & 0xFF) as u8;
                self.mem.write_byte(array.wrapping_add(index) as u32, val);
                StepResult::Continue
            }
            // 0x03 put_prop — set property ops[1] of object ops[0] to value ops[2].
            // A missing property is illegal (ZMSD §15 put_prop: "the interpreter
            // should halt with a suitable error message") — latch the VM's
            // graceful fault instead of panicking the process.
            0x03 => {
                let obj  = ops.first().copied().unwrap_or(0);
                let prop = ops.get(1).copied().unwrap_or(0) as u8;
                let val  = ops.get(2).copied().unwrap_or(0);
                if !objects::put_prop(&mut self.mem, obj, prop, val) {
                    self.state.fault = Some(format!("put_prop: object {obj} has no property {prop}"));
                }
                StepResult::Continue
            }
            // 0x05 print_char — print a single ZSCII character
            0x05 => {
                let zscii = ops.first().copied().unwrap_or(0);
                if zscii == 0 {
                    // ZSCII 0 has no printed form and must not be sent to any
                    // output stream (ZMSD §3.8) — a true no-op, not a '?'
                    // substitute. Matters for stream 3: praxix's "Memory
                    // stream round-trip" test sends 0 through print_char and
                    // expects it to contribute nothing to the table.
                    return StepResult::Continue;
                }
                if self.streams.stream3_active() {
                    // Stream 3 stores the verbatim ZSCII byte given to print_char
                    // (ZMSD §7.1.2.5), not a display-round-tripped value. Going
                    // through print_char_to_unicode()/print_text() here would
                    // convert e.g. ZSCII 10 to display space (32) and CP437
                    // glyphs with no ZSCII equivalent to '?' (63) before storing —
                    // losing the original byte (SQ-0247). Valid ZSCII output
                    // codes are 0..=255, so the low byte is the verbatim value.
                    self.streams.write_stream3_bytes(&[zscii as u8]);
                    return StepResult::Continue;
                }
                let ch = self.print_char_to_unicode(zscii);
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                self.print_text(s);
                StepResult::Continue
            }
            // 0x06 print_num — print operand as signed decimal
            0x06 => {
                let val = ops.first().copied().unwrap_or(0) as i16;
                let s = format!("{}", val);
                self.print_text(&s);
                StepResult::Continue
            }
            // 0x07 random — ZMSD §15: random number generator
            //   range > 0 → uniform random in 1..=range
            //   range == 0 → reseed from entropy (we use a fixed step; return 0)
            //   range < 0 → seed with |range| (predictable mode); return 0
            0x07 => {
                let range = ops.first().copied().unwrap_or(0) as i16;
                let result = if range > 0 {
                    // xorshift32 step
                    let mut s = self.rng_state;
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    self.rng_state = s;
                    // Map to 1..=range
                    (s % (range as u32) + 1) as u16
                } else if range < 0 {
                    // Predictable seed: use |range| as the new state (nonzero guard)
                    let seed = (-range) as u32;
                    self.rng_state = if seed == 0 { 1 } else { seed };
                    0
                } else {
                    // range == 0: re-randomise (use a fixed increment so no OS calls)
                    self.rng_state = self.rng_state.wrapping_add(0x9E3779B9);
                    if self.rng_state == 0 { self.rng_state = 1; }
                    0
                };
                self.do_store(store, result);
                StepResult::Continue
            }
            // 0x08 push — push value onto eval stack
            0x08 => {
                let val = ops.first().copied().unwrap_or(0);
                write_var(&mut self.state, &mut self.mem, 0, val); // var 0 = push
                StepResult::Continue
            }
            // 0x09 pull — pop from eval stack and store into variable ops[0].
            // ZMSD §14 / frotz semantics: when destination var == 0 (sp),
            // pop the top value, then OVERWRITE the new top with that value
            // (rather than pushing it back). This is the "pull to sp" effect:
            // stack [a, b, TOP] → pop TOP → stack [a, b], then overwrite b
            // → stack [a, TOP]. Net: removes the second-from-top element.
            0x09 => {
                let var = ops.first().copied().unwrap_or(0) as u8;
                let val = read_var(&mut self.state, &self.mem, 0); // pop stack
                if var == 0 {
                    // Destination is sp: overwrite new top (not push-back)
                    poke_stack(&mut self.state, val);
                } else {
                    write_var(&mut self.state, &mut self.mem, var, val);
                }
                StepResult::Continue
            }
            // 0x0C call_vs2 — like call_vs but with 2 type bytes, stores result
            0x0C => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, store);
                StepResult::Continue
            }
            // 0x04 sread/aread/read — pause execution and wait for a line of input.
            // v3: no store var. v4+: has a store var (terminating character).
            // Operands: text_buf, parse_buf, + optional time/routine (v4+).
            0x04 => {
                self.check_transcript_bit();
                self.note_v6_input_window();
                let text_buf = ops.first().copied().unwrap_or(0) as u32;
                let parse_buf = ops.get(1).copied().unwrap_or(0) as u32;
                let interrupt_time = ops.get(2).copied().unwrap_or(0);
                let interrupt_routine = ops.get(3).copied().unwrap_or(0);
                self.pending_input = Some(PendingInput {
                    store_var: store, text_buf, parse_buf, interrupt_time, interrupt_routine,
                    instr_pc: self.cur_instr_pc,
                });
                StepResult::NeedLine { text_buf, parse_buf }
            }
            // 0x16 read_char — pause execution and wait for a single keypress (v4+).
            // Has a store var for the ZSCII code. Operands: device, + optional time/routine.
            0x16 => {
                self.check_transcript_bit();
                self.note_v6_input_window();
                let interrupt_time = ops.get(1).copied().unwrap_or(0);
                let interrupt_routine = ops.get(2).copied().unwrap_or(0);
                self.pending_input = Some(PendingInput {
                    store_var: store, text_buf: 0, parse_buf: 0, interrupt_time, interrupt_routine,
                    instr_pc: self.cur_instr_pc,
                });
                StepResult::NeedChar
            }
            // 0x18 not (VAR form, v5+) — bitwise complement
            0x18 => {
                let val = ops.first().copied().unwrap_or(0);
                self.do_store(store, !val);
                StepResult::Continue
            }
            // 0x19 call_vn — call with up to 3 args, discard result (v5+)
            0x19 => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, None);
                StepResult::Continue
            }
            // 0x1A call_vn2 — like call_vn but with 2 type bytes
            0x1A => {
                let packed = ops.first().copied().unwrap_or(0);
                let args = if ops.len() > 1 { &ops[1..] } else { &[][..] };
                call_routine(&mut self.state, &mut self.mem, packed, args, None);
                StepResult::Continue
            }
            // 0x1F check_arg_count (v5+) — branch if arg_count >= ops[0]
            0x1F => {
                let n = ops.first().copied().unwrap_or(0);
                let arg_count = self.state.frames.last().map(|f| f.arg_count as u16).unwrap_or(0);
                self.do_branch(branch, arg_count >= n);
                StepResult::Continue
            }
            // ── Screen / stream opcodes (Task 13) ────────────────────────────

            // 0x0A split_window — set upper window to N rows (v3+)
            0x0A => {
                let rows = ops.first().copied().unwrap_or(0);
                if self.trace_screen { self.screen_trace.push(format!("@split_window({rows})")); }
                if self.screen.v6.is_some() {
                    // v6: split_window(N) resizes the UPPER window (window 1) to
                    // the full screen width × N *pixels*, anchored top-left, and
                    // gives the LOWER window (window 0) the remaining height — the
                    // classic split adapted to v6's pixel geometry. Zork Zero's
                    // title screen relies on it: it @erase_window(all),
                    // @split_window(400) (the full 400px screen height),
                    // @set_window(upper), then @draw_picture(1) so the ZORK ZERO
                    // splash fills the whole screen while window 0 collapses to
                    // zero height (so no story viewport is carved over the
                    // picture). Without the resize the picture clips to window 1's
                    // 78px banner box and only its top strip shows, and the story
                    // window stays open and paints the transcript over the splash.
                    // Only the sizes change — window 0's ORIGIN (its inset frame
                    // position) is left untouched, so the game's own
                    // window_size(win=0, …) call restores it verbatim once the
                    // splash is dismissed. (SQ-0497)
                    let screen_w = self.mem.read_word(0x22);
                    let screen_h = self.mem.read_word(0x24);
                    if let Some(v6) = self.screen.v6.as_mut() {
                        let upper = &mut v6.windows[1];
                        upper.x_coord = 1;
                        upper.y_coord = 1;
                        if screen_w > 0 {
                            upper.x_size = screen_w;
                        }
                        upper.y_size = rows;
                        v6.windows[0].y_size = screen_h.saturating_sub(rows);
                    }
                } else {
                    // Cap the row count like EXT window_size does (GRID_CELL_CAP):
                    // the operand is game-controlled, and split_window(0xFFFF)
                    // would otherwise allocate rows×cols cells (~400 MB at 80
                    // cols) before the terminal could ever show them.
                    let rows = rows.min(GRID_CELL_CAP);
                    self.screen.upper_window_rows = rows;
                    let cols = self.mem.read_byte(0x21) as u16;
                    // ZMSD §15 split_window: "In Version 3 (only) the upper
                    // window should be cleared after the split." From v4 on the
                    // existing contents survive a re-split (games re-split every
                    // turn and repaint only the fields that changed), so grow
                    // with blanks / shrink by truncation instead of reallocating.
                    if self.mem.version() <= 3 {
                        self.screen.upper.resize(rows, cols.max(1));
                        let bg = self.screen.current_bg;
                        self.screen.upper.clear_to(bg);
                    } else {
                        self.screen.upper.resize_preserving(rows, cols.max(1));
                    }
                    self.screen.cursor_row = 1;
                    self.screen.cursor_col = 1;
                }
                StepResult::Continue
            }
            // 0x0B set_window — select window 0 (lower) or 1 (upper) (v3+).
            //
            // Versions 3–5 (ZMSD §8.6.1, repeated for §8.7's v4/v5 model):
            // "Whenever the upper window is selected, its cursor position is
            // reset to the top left."
            //
            // Version 6 (ZMSD §8.8.3.5): "Each window remembers its own cursor
            // position ... it is legal to move the cursor for an unselected
            // window" — selecting a window must NOT disturb the cursor it
            // remembers. It does become the window whose colour pair the prose
            // stream prints in (§8.3), so mirror that pair.
            0x0B => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0)) as u8;
                if self.trace_screen { self.screen_trace.push(format!("@set_window({})", zscreen_window_name(win as u16))); }
                let mut mirror = None;
                if let Some(v6) = self.screen.v6.as_mut() {
                    let w = win as usize;
                    if w < v6.windows.len() {
                        v6.current = win;
                        mirror = Some((v6.windows[w].fg, v6.windows[w].bg));
                    }
                } else {
                    self.screen.current_window = win;
                    if win == 1 {
                        self.screen.cursor_row = 1;
                        self.screen.cursor_col = 1;
                    }
                }
                self.mirror_v6_colours(mirror);
                StepResult::Continue
            }
            // 0x0D erase_window — clear window (state-tracking only; no render).
            // ZMSD §8.7.3: -1 = erase all + unsplit, -2 = erase all without
            // unsplitting, 0 = lower window, 1 = upper window. The lower window's
            // scrolling contents live in the host, so we flag the request for it
            // to drain (erase_lower_requested), mirroring show_status_requested.
            //
            // v6 keeps -1 and -2 DISTINCT (ZMSD §8.8.5.3.1/§8.8.5.3.2) — see the
            // match arms below; a window number 0–7 clears just that window's
            // rect, grid and cursor.
            0x0D => {
                // -3 = current window (frotz winarg0); -1/-2 keep their own
                // erase-all meanings below.
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0)) as i16;
                if self.trace_screen { self.screen_trace.push(format!("@erase_window({})", zscreen_window_name(win as u16))); }
                let screen_w = self.mem.read_word(0x22);
                let screen_h = self.mem.read_word(0x24);
                let mut mirror = None;
                if let Some(v6) = self.screen.v6.as_mut() {
                    // v6 erase is PAINT: background fills the window's CURRENT
                    // screen rect. Painted text runs (any window's) lose the
                    // covered glyphs — no more, no less: Shogun erases its
                    // 1-px caret window without touching the menu items
                    // painted around it. Picture canvases clear via a
                    // number-0 erase event pushed onto the SAME ordered queue
                    // as draws, so "erase, then draw the borders" replays in
                    // order host-side (the title splash must actually vanish).
                    let out_chars = self.v6_win0_out_chars;
                    let mut clear_canvas: Vec<u8> = Vec::new();
                    match win {
                        -1 => {
                            // ZMSD §8.8.5.3.1: "Erasing window number -1 erases
                            // the entire screen to the background colour of
                            // window 0, unsplits windows 0 and 1 and selects
                            // window 0."
                            // The erase colour is carried to the host by the
                            // number-0 canvas-clear events pushed below (and by
                            // each window's own `bg`, which the model publishes
                            // as the window's background); the CHARACTER GRID
                            // must NOT be stamped with it. A v6 grid is a
                            // compositing layer drawn OVER the picture canvases,
                            // so an explicitly-coloured blank cell is opaque
                            // forever: Zork Zero erases at boot with a white
                            // window-0 background and its full-screen decorative
                            // window 7 would hide every picture for the rest of
                            // the session. Blank cells therefore stay
                            // `ZColour::Default` — transparent to the compositor.
                            for (i, w) in v6.windows.iter_mut().enumerate() {
                                w.grid.clear();
                                w.texts.clear();
                                w.prose.clear(); // live screen state, erased with the pixels (SQ-0585)
                                w.y_cursor = 1;
                                w.x_cursor = 1;
                                clear_canvas.push(i as u8);
                            }
                            // "unsplits windows 0 and 1": window 1 collapses to
                            // zero height, window 0 takes the whole screen
                            // (§8.8.3.3's boot geometry).
                            v6.windows[0].y_coord = 1;
                            v6.windows[0].x_coord = 1;
                            v6.windows[1].y_coord = 1;
                            v6.windows[1].x_coord = 1;
                            v6.windows[1].y_size = 0;
                            if screen_w > 0 {
                                v6.windows[0].x_size = screen_w;
                                v6.windows[1].x_size = screen_w;
                            }
                            if screen_h > 0 {
                                v6.windows[0].y_size = screen_h;
                            }
                            v6.current = 0;
                            mirror = Some((v6.windows[0].fg, v6.windows[0].bg));
                        }
                        -2 => {
                            // ZMSD §8.8.5.3.2: "Erasing window -2 erases the
                            // entire screen to the current background colour.
                            // (It doesn't perform erase_window for all the
                            // individual windows, and it doesn't change any
                            // window attributes or cursor positions.)" — so the
                            // pixels go, but cursors, geometry and the current
                            // window selection all stay exactly as they were.
                            // Grid cells stay transparent for the same
                            // compositing reason as -1 above; the erase colour
                            // travels as the canvas-clear event.
                            for (i, w) in v6.windows.iter_mut().enumerate() {
                                w.grid.clear();
                                w.texts.clear();
                                w.prose.clear(); // live screen state, erased with the pixels (SQ-0585)
                                clear_canvas.push(i as u8);
                            }
                        }
                        n if (0..8).contains(&n) => {
                            let (top, left, h, wd) = {
                                let w = &v6.windows[n as usize];
                                (
                                    w.y_coord.max(1) as i32,
                                    w.x_coord.max(1) as i32,
                                    w.y_size as i32,
                                    w.x_size as i32,
                                )
                            };
                            v6.erase_screen_rect(top, left, h, wd);
                            let w = &mut v6.windows[n as usize];
                            // ZMSD §8.8.5.3: erase "to background colour (even
                            // if the current text style is Reverse Video)" — the
                            // window's `bg` (published as the window background)
                            // and the canvas-clear event below carry that; the
                            // grid cells go back to `ZColour::Default` so the
                            // erased window does not become an opaque layer over
                            // the pictures (see the -1 arm).
                            w.grid.clear();
                            w.prose.clear(); // live screen state, erased with the pixels (SQ-0585)
                            w.y_cursor = 1;
                            w.x_cursor = 1;
                            clear_canvas.push(n as u8);
                        }
                        _ => {}
                    }
                    for window in clear_canvas {
                        self.pending_pictures.push(PictureEvent {
                            number: 0,
                            window,
                            x: 1,
                            y: 1,
                            erase: true,
                            out_chars,
                            margin_after: None,
                        });
                    }
                } else {
                    // ZMSD §8.7.3.2.1: "In Versions 5 and later, the cursor for
                    // the window being erased should be moved to the top left.
                    // In Version 4, the lower window's cursor moves to its
                    // bottom left, while the upper window's cursor moves to top
                    // left." Both versions therefore put the UPPER window's
                    // cursor at (1,1) — the only cursor this model owns; the
                    // lower window's scrolling cursor lives in the host (it
                    // drains `erase_lower_requested` and re-homes its own).
                    let bg = self.screen.current_bg;
                    match win {
                        -1 => {
                            // §8.7.3.3: "Erasing window -1 clears the whole
                            // screen to the background colour of the lower
                            // screen, collapses the upper window to height 0,
                            // moves the cursor of the lower screen to bottom
                            // left (in Version 4) or top left (in Versions 5 and
                            // later) and selects the lower screen."
                            self.screen.upper_window_rows = 0;
                            self.screen.upper.resize(0, self.screen.upper.cols);
                            self.screen.erase_lower_requested = true;
                            self.screen.current_window = 0;
                            self.screen.cursor_row = 1;
                            self.screen.cursor_col = 1;
                        }
                        -2 => {
                            // §15 erase_window: -2 "clears the screen without
                            // unsplitting it" — the upper window is erased, so
                            // §8.7.3.2.1 homes its cursor, but the split height
                            // and the window selection are untouched.
                            self.screen.upper.clear_to(bg);
                            self.screen.erase_lower_requested = true;
                            self.screen.cursor_row = 1;
                            self.screen.cursor_col = 1;
                        }
                        0 => {
                            self.screen.erase_lower_requested = true;
                        }
                        1 => {
                            self.screen.upper.clear_to(bg);
                            self.screen.cursor_row = 1;
                            self.screen.cursor_col = 1;
                        }
                        _ => {}
                    }
                }
                self.mirror_v6_colours(mirror);
                StepResult::Continue
            }
            // 0x0F set_cursor — update cursor position (row, col) in upper window.
            // v6 (ZMSD §15): addresses the CURRENT window's grid cursor; an
            // optional 3rd operand names the window instead. A negative row is
            // the v6 cursor-visibility convention (-1 off, -2 on), not a real
            // position — no visibility state is modeled yet, so it's a no-op
            // beyond not corrupting the stored cursor with the sentinel.
            0x0F => {
                let row = ops.first().copied().unwrap_or(1);
                let col = ops.get(1).copied().unwrap_or(1);
                if let Some(v6) = self.screen.v6.as_mut() {
                    let win = ops.get(2).copied().map(|w| (if w == 0xFFFD { v6.current as u16 } else { w }) as u8).unwrap_or(v6.current);
                    if self.trace_screen {
                        self.screen_trace.push(format!("@set_cursor(row={row}, col={col}, window={win})"));
                    }
                    if (row as i16) >= 0 {
                        if let Some(w) = v6.windows.get_mut(win as usize) {
                            // ZMSD §15 set_cursor: v6 coordinates are in UNITS
                            // (pixels), 1-based, relative to the window's (1,1) —
                            // stored VERBATIM (window props 4/5 are read back in
                            // units). A position outside the margins moves the
                            // cursor to the left margin (§15) — model that as a
                            // clamp to 1 for any zero/negative operand.
                            w.y_cursor = if (row as i16) < 1 { 1 } else { row };
                            w.x_cursor = if (col as i16) < 1 { 1 } else { col };
                        }
                    }
                } else {
                    if self.trace_screen { self.screen_trace.push(format!("@set_cursor(row={row}, col={col})")); }
                    // ZMSD §8.7.2.3: "When the upper window is selected, its
                    // cursor position can be moved with set_cursor. … The
                    // opcode has no effect when the lower window is selected.
                    // It is illegal to move the cursor outside the current size
                    // of the upper window."
                    //
                    // Half one is literal: a lower-window set_cursor is dropped
                    // (harmless either way, since §8.7.2 re-homes the upper
                    // cursor to the top left whenever window 1 is selected).
                    //
                    // Half two is deliberately loosened to the PHYSICAL SCREEN
                    // rather than the split height: Inform's menu library
                    // (LostPig's HELP) splits N lines and then set_cursors below
                    // the split, and real interpreters keep that text — see
                    // `upper_window_write_beyond_split_grows_grid`. So only a
                    // move that no interpreter could honour (row/col 0, or off
                    // the screen entirely) is ignored as "illegal".
                    let screen_rows = self.mem.read_byte(0x20) as u16;
                    let screen_cols = self.mem.read_byte(0x21) as u16;
                    let in_range = row >= 1
                        && col >= 1
                        && (screen_rows == 0 || row <= screen_rows)
                        && (screen_cols == 0 || col <= screen_cols);
                    if self.screen.current_window == 1 && in_range {
                        self.screen.cursor_row = row;
                        self.screen.cursor_col = col;
                    }
                }
                StepResult::Continue
            }
            // 0x11 set_text_style — update text style bitmask (v4+).
            // ZMSD §8.7.1: styles are cumulative — Roman (0) clears all, any
            // nonzero style is OR-ed into the current set. Games (e.g. BeyondZork
            // menus) layer fixed-pitch onto a reverse-video region and rely on
            // the reverse bit persisting; replacing would wipe it.
            0x11 => {
                let style = ops.first().copied().unwrap_or(0) as u8;
                if self.trace_screen { self.screen_trace.push(format!("@set_text_style({})", zscreen_style_name(style as u16))); }
                if style == 0 {
                    self.screen.text_style = 0;
                } else {
                    self.screen.text_style |= style;
                }
                // ZMSD §8.8.3.2.3: "The text style is set just as in Version 4,
                // using set_text_style (which sets that for the current
                // window). The property holds the operand of that instruction
                // (e.g. 4 for italic)." Mirror the resulting bitmask into the
                // CURRENT v6 window's prop 10 so get_wind_prop(w, 10) reads
                // fresh.
                let new_style = self.screen.text_style as u16;
                if let Some(v6) = self.screen.v6.as_mut() {
                    let cur = v6.current as usize;
                    if let Some(w) = v6.windows.get_mut(cur) {
                        w.text_style = new_style;
                    }
                }
                StepResult::Continue
            }
            // 0x12 buffer_mode — toggle output buffering (v4+)
            0x12 => {
                let mode = ops.first().copied().unwrap_or(0);
                let on = mode != 0;
                if self.trace_screen { self.screen_trace.push(format!("@buffer_mode({})", if on { "on" } else { "off" })); }
                self.screen.buffer_mode = on;
                self.out.set_buffer_mode(on);
                StepResult::Continue
            }
            // 0x13 output_stream — select/deselect output streams (ZMSD §7.1.2.5)
            //   +1/-1: stream 1 (screen) on/off
            //   +2/-2: stream 2 (transcript) on/off
            //   +3:    stream 3 on — second operand is table address, third
            //          (v6 only) is the width operand (see below)
            //   -3:    stream 3 off — finalise table, restore routing
            //   +4/-4: stream 4 (commands) on/off
            0x13 => {
                let stream = ops.first().copied().unwrap_or(0) as i16;
                match stream {
                    1  => { self.streams.stream1 = true; }
                    -1 => { self.streams.stream1 = false; }
                    2  => {
                        self.streams.stream2 = true;
                        // ZMSD §7.6.5: "Interpreters are allowed to not support
                        // access to external files (such as with output_stream
                        // 2 …)"; §7.6.5.2: such an attempt "should ideally print
                        // a warning to the user that the functionality is not
                        // available, and otherwise do nothing". Nothing consumes
                        // `stream2` — there is no transcript FILE — so the flag
                        // is the "do nothing" half and this diagnostic is the
                        // warning. The host surfaces diagnostics as Warning
                        // transcript lines; once per session is enough.
                        if !self.warned_stream2 {
                            self.warned_stream2 = true;
                            self.diagnostics.push(
                                "transcript file output isn't supported — the game's script command will have no effect (the app keeps its own scrollback)".to_string(),
                            );
                        }
                    }
                    -2 => { self.streams.stream2 = false; }
                    3  => {
                        let table = ops.get(1).copied().unwrap_or(0) as u32;
                        // ZMSD §15 output_stream: "In Version 6, a width field
                        // may optionally be given: text will then be
                        // justified as if it were in the window with that
                        // number (if width is zero or positive) or a box
                        // -width pixels wide (if negative)." Resolve to a
                        // concrete pixel width here (needs the v6 window
                        // table, which StreamState doesn't have); `None` when
                        // the operand is absent, or on non-v6 stories where
                        // this operand doesn't exist.
                        let width_px = if self.mem.version() == 6 {
                            ops.get(2).map(|&w| {
                                let w = w as i16;
                                if w < 0 {
                                    (-w) as u16
                                } else {
                                    self.screen.v6.as_ref()
                                        .and_then(|v6| v6.windows.get(w as usize))
                                        .map(|win| win.x_size)
                                        .unwrap_or(0)
                                }
                            })
                        } else {
                            None
                        };
                        self.streams.push_stream3(table, width_px);
                    }
                    -3 => {
                        self.streams.pop_stream3(&mut self.mem);
                    }
                    4  => { self.streams.stream4 = true; }
                    -4 => { self.streams.stream4 = false; }
                    _  => {}
                }
                StepResult::Continue
            }
            // VAR:0x14 input_stream — select input source: 0 = keyboard (default), 1 = command
            // file. The engine only records the selection; sourcing input from a file is a host
            // concern (the app drives all reads via supply_line). Other values are ignored per spec.
            0x14 => {
                let stream = ops.first().copied().unwrap_or(0) as i16;
                if stream == 0 || stream == 1 {
                    self.streams.input_stream = stream as u8;
                }
                StepResult::Continue
            }
            // VAR:0x10 get_cursor — write (row, col) of the upper-window cursor into a 2-word array.
            // v6 (frotz z_get_cursor): the CURRENT window's pixel cursor, verbatim —
            // the non-v6 screen cursor fields are never updated by the v6 path.
            //
            // ZMSD §8.8.3.2.7: "If an attempt is made by the game to read the
            // cursor position at a time when text is held unprinted in a
            // buffer, then this text should be flushed first, to ensure that
            // the cursor position is accurate before being read." Nothing is
            // ever held here: `print_text` paints/streams each call as it
            // arrives and moves the window cursor with it
            // (`v6_advance_prose_cursor`, SQ-0536), so the read below is
            // already the flushed position. Any future deferred buffer must
            // flush at this point.
            0x10 => {
                let array = ops.first().copied().unwrap_or(0) as u32;
                if let Some(v6) = self.screen.v6.as_ref() {
                    let w = &v6.windows[(v6.current as usize).min(7)];
                    self.mem.write_word(array, w.y_cursor);
                    self.mem.write_word(array + 2, w.x_cursor);
                } else {
                    self.mem.write_word(array, self.screen.cursor_row);
                    self.mem.write_word(array + 2, self.screen.cursor_col);
                }
                StepResult::Continue
            }
            // VAR:0x17 scan_table — search a table for x; store match address (0 if none), branch if found.
            0x17 => {
                let x = ops.first().copied().unwrap_or(0);
                let table = ops.get(1).copied().unwrap_or(0) as u32;
                let len = ops.get(2).copied().unwrap_or(0);
                let form = ops.get(3).copied().unwrap_or(0x82);
                let is_word = form & 0x80 != 0;
                let step = ((form & 0x7F) as u32).max(1);
                let mut found: u16 = 0;
                for i in 0..len as u32 {
                    let addr = table + i * step;
                    // Compare against the FULL 16-bit search value in both modes: a
                    // byte read is already 0..=255, so a byte-mode search for a value
                    // > 255 correctly never matches (matching the reference terp).
                    // Masking x to its low byte here spuriously matched (praxix
                    // "Bad @scan_table branch").
                    let val = if is_word { self.mem.read_word(addr) } else { self.mem.read_byte(addr) as u16 };
                    if val == x {
                        found = addr as u16;
                        break;
                    }
                }
                self.do_store(store, found);
                self.do_branch(branch, found != 0);
                StepResult::Continue
            }
            // VAR:0x1B tokenise text parse [dictionary] [flag] — lex the text buffer
            // into the parse buffer, like the lexing half of `read`.
            0x1B => {
                let text_buf = ops.first().copied().unwrap_or(0) as u32;
                let parse = ops.get(1).copied().unwrap_or(0) as u32;
                // Operand 2 is an optional custom dictionary address (ZMSD §15);
                // 0 means use the standard story dictionary.
                let dict_addr = ops.get(2).copied().unwrap_or(0);
                let flag = ops.get(3).copied().unwrap_or(0) != 0;
                let text = self.read_text_buffer(text_buf);
                let text_data_start: u8 = if self.mem.version() <= 4 { 1 } else { 2 };
                let dict = if dict_addr != 0 {
                    dictionary::load_at(&self.mem, dict_addr as u32)
                } else {
                    dictionary::load(&self.mem)
                };
                let tokens = dict.tokenise(&self.mem, &text);
                self.write_parse_buffer(parse, &tokens, text_data_start, flag);
                StepResult::Continue
            }
            // VAR:0x1C encode_text zscii-text length from coded-text — encode `length`
            // ZSCII bytes at zscii-text+from to the packed dictionary form at coded-text.
            0x1C => {
                let src = ops.first().copied().unwrap_or(0) as u32;
                let length = ops.get(1).copied().unwrap_or(0) as u32;
                let from = ops.get(2).copied().unwrap_or(0) as u32;
                let coded = ops.get(3).copied().unwrap_or(0) as u32;
                let mut s = String::new();
                for i in 0..length {
                    let b = self.mem.read_byte(src + from + i);
                    s.push(zscii_to_char(b as u16)); // mirror the read path's ZSCII decode
                }
                let packed = crate::text::encode::encode_word_mem(&s, &self.mem);
                for (i, b) in packed.iter().enumerate() {
                    self.mem.write_byte(coded + i as u32, *b);
                }
                StepResult::Continue
            }
            // VAR:0x1D copy_table — copy/zero a memory region (ZMSD §15).
            0x1D => {
                let first = ops.first().copied().unwrap_or(0) as u32;
                let second = ops.get(1).copied().unwrap_or(0) as u32;
                let size = ops.get(2).copied().unwrap_or(0) as i16;
                if second == 0 {
                    for i in 0..size.unsigned_abs() as u32 {
                        self.mem.write_byte(first + i, 0);
                    }
                } else if size < 0 {
                    // forced forward copy; overlap corruption is intentional
                    let n = size.unsigned_abs() as u32;
                    for i in 0..n {
                        let b = self.mem.read_byte(first + i);
                        self.mem.write_byte(second + i, b);
                    }
                } else {
                    // positive: copy avoiding corruption — snapshot the source first
                    let n = size as u32;
                    let src: Vec<u8> = (0..n).map(|i| self.mem.read_byte(first + i)).collect();
                    for (i, &b) in src.iter().enumerate() {
                        self.mem.write_byte(second + i as u32, b);
                    }
                }
                StepResult::Continue
            }
            // VAR:0x1E print_table — print a rectangle of ZSCII text from the current cursor (ZMSD §15).
            0x1E => {
                let mut addr = ops.first().copied().unwrap_or(0) as u32;
                let width = ops.get(1).copied().unwrap_or(0);
                let height = ops.get(2).copied().unwrap_or(1).max(1);
                let skip = ops.get(3).copied().unwrap_or(0) as u32;
                let start_col = self.screen.cursor_col;
                let start_row = self.screen.cursor_row;
                for row in 0..height {
                    // Position each row at the starting column, one line down (correct once the grid exists).
                    self.screen.cursor_row = start_row + row;
                    self.screen.cursor_col = start_col;
                    for _ in 0..width {
                        let ch = zscii_to_char(self.mem.read_byte(addr) as u16);
                        let mut buf = [0u8; 4];
                        self.print_text(ch.encode_utf8(&mut buf));
                        addr += 1;
                    }
                    addr += skip;
                }
                StepResult::Continue
            }
            // 0x0E erase_line — erase from cursor to end of line in the upper window.
            // v6 (ZMSD §15): value 1 erases from the cursor to the end of the
            // line in the CURRENT window; value n erases n-1 pixels. Menu
            // screens (Zork Zero's InvisiClues) erase_line before repainting
            // each highlighted row — a no-op left stale tails behind.
            0x0E => {
                let value = ops.first().copied().unwrap_or(0);
                if self.trace_screen { self.screen_trace.push(format!("@erase_line({value})")); }
                if let Some(v6) = self.screen.v6.as_mut() {
                    let cur = v6.current as usize;
                    let (top, left, width) = {
                        let w = &v6.windows[cur.min(7)];
                        let y_abs = w.y_coord.max(1) as i32 + w.y_cursor.max(1) as i32 - 1;
                        let x_abs = w.x_coord.max(1) as i32 + w.x_cursor.max(1) as i32 - 1;
                        // ZMSD §15 erase_line (v6): the erase is "clipped to
                        // stay inside the right margin" — the window's right
                        // edge less its prop-7 right margin, not the raw edge.
                        let to_edge = (w.x_coord.max(1) as i32 + w.x_size as i32
                            - w.right_margin as i32)
                            .saturating_sub(x_abs);
                        let width = if value == 1 { to_edge } else { (value as i32 - 1).min(to_edge) };
                        (y_abs, x_abs, width)
                    };
                    v6.erase_screen_rect(top, left, crate::screen::V6_FONT_HEIGHT as i32, width);
                    // Cell-grid mirror: blank from the cursor cell rightward.
                    let w = &mut v6.windows[cur.min(7)];
                    let row = (w.y_cursor.max(1) - 1) / crate::screen::V6_FONT_HEIGHT + 1;
                    let start = (w.x_cursor.max(1) - 1) / crate::screen::V6_FONT_WIDTH + 1;
                    let cells = (width.max(0) as u16).div_ceil(crate::screen::V6_FONT_WIDTH);
                    for c in start..(start + cells).min(w.grid.cols + 1) {
                        w.grid.put(row, c, ' ', 0, w.fg, w.bg);
                    }
                } else if value == 1 && self.screen.current_window == 1 {
                    // ZMSD §15 erase_line (v4/5): "erase from the current cursor
                    // position to the end of its line in the CURRENT window."
                    // Only the upper window is a modelled grid — the lower
                    // window is a scrolling stream the host owns — so an
                    // erase_line issued while window 0 is selected must not
                    // scribble a blank row into the upper grid at the upper
                    // window's (unrelated) cursor.
                    let (row, start) = (self.screen.cursor_row, self.screen.cursor_col);
                    let cols = self.screen.upper.cols;
                    let mut c = start;
                    while c <= cols {
                        // ZMSD §8.7.3.4: erase_line clears "to background
                        // colour", and "Even if the text style is Reverse Video
                        // the new blank space should not have reversed colours"
                        // — so the blanks carry style 0, never `text_style`.
                        self.screen.upper.put(row, c, ' ', 0, crate::screen::ZColour::Default, self.screen.current_bg);
                        c += 1;
                    }
                }
                StepResult::Continue
            }
            // 0x15 sound_effect — number effect volume routine (ZMSD §9.4).
            // Record a SoundEvent for every call (including #1/#2 bleeps). The host
            // drains `pending_sounds` and decides what to play / how to visualise.
            0x15 => {
                let number = ops.first().copied().unwrap_or(0);
                if number != 0 {
                    let effect = ops.get(1).copied().unwrap_or(0) as u8;
                    // Volume word: low byte = volume (1..8, 255=loudest), high byte
                    // = repeat count (255 = forever, 0/omitted = play once, applied
                    // by the host). Default 8 when omitted.
                    let vw = ops.get(2).copied().unwrap_or(8);
                    let volume = (vw & 0xFF) as u8;
                    let repeats = (vw >> 8) as u8;
                    let routine = ops.get(3).copied().unwrap_or(0);
                    self.pending_sounds.push(SoundEvent { number, effect, volume, repeats, routine });
                }
                StepResult::Continue
            }
            // Unknown / unimplemented VAR opcode: record once, then ignore.
            _ => {
                if self.warned_var_opcodes.insert(opcode) {
                    self.diagnostics.push(format!(
                        "unimplemented VAR opcode 0x{opcode:02X} (ignored)"
                    ));
                }
                StepResult::Continue
            }
        }
    }

    // -----------------------------------------------------------------------
    // EXT opcodes (v5+)
    // -----------------------------------------------------------------------

    fn exec_ext(&mut self, opcode: u8, ops: &[u16], store: Option<u8>, branch: Option<Branch>) -> StepResult {
        match opcode {
            // EXT:0x00 save — 0 operands: full game-state save (suspend).
            // ≥3 operands: v5 auxiliary "save table bytes name [prompt]".
            0x00 => {
                if ops.len() >= 3 {
                    let table = ops[0] as u32;
                    let len = ops[1] as u32;
                    let name = self.read_aux_name(ops[2] as u32);
                    let mut data = Vec::with_capacity(len.min(self.mem.len() as u32) as usize);
                    for i in 0..len {
                        let a = table + i;
                        if a as usize >= self.mem.len() { break; }
                        data.push(self.mem.read_byte(a));
                    }
                    self.aux_data.insert(name, data);
                    self.aux_dirty = true;
                    self.do_store(store, 1);
                    StepResult::Continue
                } else {
                    let dest = match store {
                        Some(sv) => SaveDest::Store(sv),
                        None => SaveDest::Store(0),
                    };
                    self.pending_save = Some(PendingSave {
                        result_dest: dest,
                        descriptor_pc: self.state.pc.saturating_sub(1),
                    });
                    StepResult::SaveRequest
                }
            }
            // EXT:0x01 restore — 0 operands: full restore (suspend). ≥3 operands:
            // v5 auxiliary "restore table bytes name [prompt]" (stores bytes read).
            0x01 => {
                if ops.len() >= 3 {
                    let table = ops[0] as u32;
                    let len = ops[1] as u32;
                    let name = self.read_aux_name(ops[2] as u32);
                    let written = match self.aux_data.get(&name).cloned() {
                        Some(data) => {
                            let n = (data.len() as u32).min(len);
                            let mut w = 0u16;
                            for i in 0..n {
                                let a = table + i;
                                if a as usize >= self.mem.len() { break; }
                                self.mem.write_byte(a, data[i as usize]);
                                w += 1;
                            }
                            w
                        }
                        None => 0,
                    };
                    self.do_store(store, written);
                    StepResult::Continue
                } else {
                    self.pending_restore_store = store;
                    self.pending_restore = true;
                    StepResult::RestoreRequest
                }
            }
            // EXT:0x02 log_shift — logical (unsigned) shift
            // places > 0 → left shift; places < 0 → right shift (zero-fill)
            0x02 => {
                let n = ops.first().copied().unwrap_or(0);
                let places = ops.get(1).copied().unwrap_or(0) as i16;
                let result = if places >= 16 || places <= -16 {
                    0u16
                } else if places > 0 {
                    n << (places as u16)
                } else if places < 0 {
                    n >> ((-places) as u16)
                } else {
                    n
                };
                self.do_store(store, result);
                StepResult::Continue
            }
            // EXT:0x03 art_shift — arithmetic (signed) shift
            // places > 0 → left shift; places < 0 → arithmetic right shift
            0x03 => {
                let n = ops.first().copied().unwrap_or(0) as i16;
                let places = ops.get(1).copied().unwrap_or(0) as i16;
                let result: i16 = if places >= 16 || places <= -16 {
                    if n < 0 { -1 } else { 0 }
                } else if places > 0 {
                    n << (places as u16)
                } else if places < 0 {
                    n >> ((-places) as u16)
                } else {
                    n
                };
                self.do_store(store, result as u16);
                StepResult::Continue
            }
            // EXT:0x04 set_font (ZMSD §15): operand 0 = query current font without
            // changing; 1 = normal, 3 = character-graphics (Font 3), 4 = Courier/
            // fixed-pitch (rendered like font 1 on a fixed grid). Returns the
            // previously-active font, or 0 if the requested font is unavailable.
            //
            // ZMSD §15 set_font: "In Version 6, set_font has an optional window
            // parameter, as for set_colour." -3 (0xFFFD) means the currently
            // selected window (`v6_window_operand`); when the operand is
            // omitted, mirror `set_colour`'s default (the current window).
            // Mirror the NEW font into that window's prop 12 (font_number) so
            // a subsequent get_wind_prop(win, 12) reads it fresh.
            0x04 => {
                let requested = ops.first().copied().unwrap_or(0);
                if self.trace_screen { self.screen_trace.push(format!("@set_font({})", zscreen_font_name(requested))); }
                let prev = self.screen.current_font as u16;
                let mut changed = false;
                let result = match requested {
                    0 => prev,       // query: return current, no change
                    1 => { self.screen.current_font = 1; changed = true; prev }
                    3 => { self.screen.current_font = 3; changed = true; prev }
                    4 => { self.screen.current_font = 4; changed = true; prev }
                    _ => 0,          // unsupported → 0
                };
                if changed {
                    let new_font = self.screen.current_font as u16;
                    let win = match ops.get(1).copied() {
                        Some(w) => Some(self.v6_window_operand(w)),
                        None => self.screen.v6.as_ref().map(|v6| v6.current as u16),
                    };
                    if let Some(win) = win {
                        if let Some(v6) = self.screen.v6.as_mut() {
                            if let Some(w) = v6.windows.get_mut(win as usize) {
                                w.font_number = new_font;
                            }
                        }
                    }
                }
                self.do_store(store, result);
                StepResult::Continue
            }
            // EXT:0x0B print_unicode — output an arbitrary Unicode codepoint.
            0x0B => {
                let cp = ops.first().copied().unwrap_or(0) as u32;
                let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                let mut b = [0u8; 4];
                self.print_text(ch.encode_utf8(&mut b));
                StepResult::Continue
            }
            // EXT:0x0C check_unicode — bit0: can print, bit1: can input. We render
            // UTF-8, so any valid scalar can print (bit0). We CANNOT input Unicode:
            // the input path is byte-limited (`supply_char(ch: u8)`, no ZSCII
            // mapping), so bit1 stays clear. Report 1 (print-only) for a valid
            // scalar; invalid -> 0. (Bump to 3 once real Unicode input lands.)
            0x0C => {
                let cp = ops.first().copied().unwrap_or(0) as u32;
                let val = if char::from_u32(cp).is_some() { 1 } else { 0 };
                self.do_store(store, val);
                StepResult::Continue
            }
            // EXT:0x09 save_undo — in-memory undo snapshot.
            0x09 => {
                self.do_save_undo(store);
                StepResult::Continue
            }
            // EXT:0x0A restore_undo — restore the newest in-memory undo snapshot.
            0x0A => {
                self.do_restore_undo(store);
                StepResult::Continue
            }
            // EXT:0x0D set_true_colour (v5+). Same channel model as set_colour
            // but signed sentinels: -2 = keep, -1 = default, else 15-bit RGB.
            // v6-only (ZMSD §15): an optional 3rd operand names the window
            // (defaults to current).
            0x0D => {
                let fg_op = ops.first().copied().unwrap_or(0);
                let bg_op = ops.get(1).copied().unwrap_or(0);
                if let Some(v6) = self.screen.v6.as_mut() {
                    let win = ops.get(2).copied().map(|w| (if w == 0xFFFD { v6.current as u16 } else { w }) as u8).unwrap_or(v6.current);
                    if self.trace_screen {
                        let fg = decode_true_colour(fg_op).map(zscreen_colour_name).unwrap_or_else(|| fg_op.to_string());
                        let bg = decode_true_colour(bg_op).map(zscreen_colour_name).unwrap_or_else(|| bg_op.to_string());
                        self.screen_trace.push(format!("@set_true_colour(fg={fg}, bg={bg}, window={win})"));
                    }
                    let mut mirror = None;
                    if let Some(w) = v6.windows.get_mut(win as usize) {
                        if let Some(c) = decode_true_colour(fg_op) {
                            w.fg = c;
                        }
                        if let Some(c) = decode_true_colour(bg_op) {
                            w.bg = c;
                        }
                        w.colour_data = pack_colour_data(w.fg, w.bg);
                        if win == v6.current {
                            mirror = Some((w.fg, w.bg));
                        }
                    }
                    self.mirror_v6_colours(mirror);
                } else {
                    if self.trace_screen {
                        let fg = decode_true_colour(fg_op).map(zscreen_colour_name).unwrap_or_else(|| fg_op.to_string());
                        let bg = decode_true_colour(bg_op).map(zscreen_colour_name).unwrap_or_else(|| bg_op.to_string());
                        self.screen_trace.push(format!("@set_true_colour(fg={fg}, bg={bg})"));
                    }
                    if let Some(c) = decode_true_colour(fg_op) {
                        self.screen.current_fg = c;
                    }
                    if let Some(c) = decode_true_colour(bg_op) {
                        self.screen.current_bg = c;
                    }
                }
                StepResult::Continue
            }
            // ── v6 window/graphics opcodes — Phase 0 stubs (SQ-0186). ──────────
            // Signatures are honoured (store 0 / documented branch sense) so the
            // VM stays in sync; real behaviour lands in later phases.
            // EXT:0x13 get_wind_prop(window, property-number) -> (result) — ZMSD
            // 1.1 §15/§8.8.3.2: reads the addressed window's property array.
            0x13 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0));
                let prop = ops.get(1).copied().unwrap_or(0);
                let (def_fg, def_bg) = (self.default_fg_colour, self.default_bg_colour);
                let val = self.screen.v6.as_ref()
                    .and_then(|v6| v6.windows.get(win as usize))
                    .map(|w| match prop {
                        // ZMSD §8.8.3.2.8: properties 16/17 "show the actual
                        // colour being used for the foreground and background,
                        // whether it was set using set_colour or
                        // set_true_colour" — so they are DERIVED on read from
                        // the window's live channels (a standard number maps
                        // through the §8.3.1 table; a `Default` channel resolves
                        // to the interpreter's own default, the same value
                        // published in header-extension words 5/6).
                        16 => w.fg.true_value(def_fg),
                        17 => w.bg.true_value(def_bg),
                        _ => w.get_prop(prop),
                    })
                    .unwrap_or(0);
                if self.trace_screen {
                    self.screen_trace.push(format!("@get_wind_prop(win={win}, prop={prop}) -> {val}"));
                }
                self.do_store(store, val);
                StepResult::Continue
            }
            // EXT:0x1D buffer_screen(mode) -> (result) — ZMSD §15: "If mode is
            // 0, updates must be made as soon as possible. If mode is 1, the
            // interpreter may make changes to a backing store, and need not
            // update the screen." Issuing buffer_screen(-1) forces an
            // immediate update WITHOUT altering the buffering state. Returns
            // the OLD buffering state (no rendering effect either way).
            0x1D => {
                let mode = ops.first().copied().unwrap_or(0) as i16;
                let prev = self.buffer_screen_mode;
                if self.trace_screen {
                    self.screen_trace.push(format!("@buffer_screen(mode={mode}) -> prev={prev}"));
                }
                if mode != -1 {
                    self.buffer_screen_mode = mode as u16;
                }
                self.do_store(store, prev);
                StepResult::Continue
            }
            // EXT:0x06 picture_data(picture-number, array) [branch] — ZMSD §15:
            // picture-number 0 asks for "number of pictures available" (word 0)
            // and "release number of the picture file" (word 1), branching if any
            // pictures are available. Otherwise: if `picture-number` is in the
            // injected table, write height (word 0) then width (word 1) in
            // pixels and branch true; else leave the array untouched and don't
            // branch.
            0x06 => {
                // v6-only: for a non-v6 story this stays the Phase 0 stub
                // (no array write, no branch) so v1–5 behaviour is byte-identical.
                if self.screen.v6.is_none() {
                    self.do_branch(branch, false);
                    return StepResult::Continue;
                }
                let number = ops.first().copied().unwrap_or(0);
                let array = ops.get(1).copied().unwrap_or(0) as u32;
                if number == 0 {
                    let count = self.picture_dims.len() as u16;
                    // No real picture-file metadata is available yet (Task 9 wires
                    // the self-blorb); the story's own header release number is a
                    // harmless placeholder until then.
                    let release = self.mem.read_word(0x02);
                    self.mem.write_word(array, count);
                    self.mem.write_word(array.wrapping_add(2), release);
                    if self.trace_screen {
                        self.screen_trace.push(format!("@picture_data(0) -> count={count}, release={release}"));
                    }
                    self.do_branch(branch, count > 0);
                } else if let Some(&(_, w, h)) = self.picture_dims.iter().find(|&&(n, _, _)| n == number) {
                    self.mem.write_word(array, h);
                    self.mem.write_word(array.wrapping_add(2), w);
                    if self.trace_screen {
                        self.screen_trace.push(format!("@picture_data({number}) -> h={h}, w={w}"));
                    }
                    self.do_branch(branch, true);
                } else {
                    if self.trace_screen {
                        self.screen_trace.push(format!("@picture_data({number}) -> MISSING"));
                    }
                    self.do_branch(branch, false);
                }
                StepResult::Continue
            }
            0x1B => { self.do_branch(branch, false); StepResult::Continue } // make_menu → failed (stub; SQ-0457 tracks a real implementation)
            0x18 => { // push_stack value stack -> (branch) — v6 user stack (ZMSD §15).
                // The user stack at `addr` stores the number of FREE slots in its
                // first word; entries fill downward from the end. Mirrors frotz
                // z_push_stack exactly, branch included (on the post-push free count).
                let val = ops.first().copied().unwrap_or(0);
                let addr = ops.get(1).copied().unwrap_or(0) as u32;
                let size = self.mem.read_word(addr);
                let free_after = if size != 0 {
                    self.mem.write_word(addr.wrapping_add(2u32.wrapping_mul(size as u32)), val);
                    let ns = size - 1;
                    self.mem.write_word(addr, ns);
                    ns
                } else {
                    0
                };
                self.do_branch(branch, free_after != 0);
                StepResult::Continue
            }
            0x15 => { // pop_stack items [stack] — v6 (ZMSD §15, frotz z_pop_stack).
                // With a stack operand it frees `items` slots on that user stack;
                // without one it discards `items` from the game (eval) stack.
                let items = ops.first().copied().unwrap_or(0);
                if let Some(&addr16) = ops.get(1) {
                    let addr = addr16 as u32;
                    let size = self.mem.read_word(addr);
                    self.mem.write_word(addr, size.wrapping_add(items));
                } else {
                    for _ in 0..items {
                        let _ = read_var(&mut self.state, &self.mem, 0x00);
                    }
                }
                StepResult::Continue
            }
            // EXT:0x10 move_window(window, y, x) — ZMSD §15: reposition the window's
            // top-left corner (pixels). Purely notional — does not change the
            // current display, only where future plotting/geometry lands.
            0x10 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0));
                let y = ops.get(1).copied().unwrap_or(0);
                let x = ops.get(2).copied().unwrap_or(0);
                if self.trace_screen {
                    self.screen_trace.push(format!("@move_window(win={win}, y={y}, x={x})"));
                }
                if let Some(v6) = self.screen.v6.as_mut() {
                    if (win as usize) < v6.windows.len() {
                        let w = &mut v6.windows[win as usize];
                        w.y_coord = y;
                        w.x_coord = x;
                    }
                }
                StepResult::Continue
            }
            // EXT:0x11 window_size(window, y, x) — ZMSD §15: resize the window
            // (pixels). We also quantize to the character grid so the window's
            // text-cell storage (used for grid rendering) matches the new size.
            0x11 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0));
                let y = ops.get(1).copied().unwrap_or(0);
                let x = ops.get(2).copied().unwrap_or(0);
                if self.trace_screen {
                    self.screen_trace.push(format!("@window_size(win={win}, y={y}, x={x})"));
                }
                // Bound the backing grid so a hostile / buggy story requesting
                // window_size(w, 0xFFFF, 0xFFFF) can't force a ~1 GB allocation
                // (8191×8191 cells) — an OOM abort would be worse than the VM's
                // graceful-fault guarantee. GRID_CELL_CAP (1024) far exceeds any
                // real terminal (a 4K screen at 8 px/cell is ~480×270 cells) yet
                // caps worst-case storage at ~1M cells. The pixel sizes (props
                // 2/3) are still stored verbatim; only the cell grid is bounded.
                if let Some(v6) = self.screen.v6.as_mut() {
                    if (win as usize) < v6.windows.len() {
                        let w = &mut v6.windows[win as usize];
                        w.y_size = y;
                        w.x_size = x;
                        // ZMSD §8.8.3.4: "If the window size is reduced so that
                        // its cursor lies outside it, the cursor should be reset
                        // to the left margin on the top line." Cursors are 1-based
                        // pixels within the window, so "left margin" is
                        // left_margin + 1. (Painted text is unaffected —
                        // window_size "does not change the current display".)
                        if w.y_cursor > w.y_size || w.x_cursor > w.x_size {
                            w.y_cursor = 1;
                            w.x_cursor = w.left_margin + 1;
                        }
                        let rows = (y / V6_FONT_HEIGHT).clamp(1, GRID_CELL_CAP);
                        let cols = (x / V6_FONT_WIDTH).clamp(1, GRID_CELL_CAP);
                        w.grid.resize(rows, cols);
                    }
                }
                StepResult::Continue
            }
            // EXT:0x12 window_style(window, flags, operation) — ZMSD §15
            // (confirmed via inform-fiction.org/zmachine/standards/z1point1/sect15.html):
            // operation 0 = replace (attributes := flags), 1 = set bits (OR),
            // 2 = clear bits (AND NOT), 3 = toggle bits (XOR).
            0x12 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0));
                let flags = ops.get(1).copied().unwrap_or(0);
                let operation = ops.get(2).copied().unwrap_or(0);
                if self.trace_screen {
                    self.screen_trace.push(format!(
                        "@window_style(win={win}, flags={flags:#06b}, op={operation})"
                    ));
                }
                if let Some(v6) = self.screen.v6.as_mut() {
                    if (win as usize) < v6.windows.len() {
                        let w = &mut v6.windows[win as usize];
                        w.attributes = match operation {
                            0 => flags,
                            1 => w.attributes | flags,
                            2 => w.attributes & !flags,
                            3 => w.attributes ^ flags,
                            _ => w.attributes,
                        };
                    }
                }
                StepResult::Continue
            }
            // EXT:0x19 put_wind_prop(window, property-number, value) — ZMSD 1.1
            // §15/§8.8.3.2: writes the addressed window's property array.
            0x19 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0));
                let prop = ops.get(1).copied().unwrap_or(0);
                let val = ops.get(2).copied().unwrap_or(0);
                if self.trace_screen {
                    self.screen_trace.push(format!("@put_wind_prop(win={win}, prop={prop}, val={val})"));
                }
                if let Some(v6) = self.screen.v6.as_mut() {
                    if let Some(w) = v6.windows.get_mut(win as usize) {
                        w.put_prop(prop, val);
                    }
                }
                StepResult::Continue
            }
            // EXT:0x05 draw_picture(picture-number, y, x) — ZMSD §15 (confirmed
            // via inform-fiction.org/zmachine/standards/z1point1/sect15.html):
            // y/x are the pixel coords (top-left corner) within the current
            // window; 0 for either means "use the window's cursor position"
            // (left to the host to resolve — the engine forwards the raw
            // operand). Recorded as a PictureEvent for the host to rasterize
            // (Plan 1b), mirroring pending_sounds.
            0x05 => {
                // v6-only: a non-v6 story keeps the Phase 0 no-op (no event).
                if let Some(v6) = self.screen.v6.as_ref() {
                    let window = v6.current;
                    let number = ops.first().copied().unwrap_or(0);
                    // ZMSD §15: (y,x) are each optional — a value of zero means
                    // "the cursor y or x coordinate in the current window" (the
                    // cursor is already stored in 1-based pixels).
                    let (cy, cx) = v6
                        .windows
                        .get(window as usize)
                        .map(|w| (w.y_cursor.max(1), w.x_cursor.max(1)))
                        .unwrap_or((1, 1));
                    let y = match ops.get(1).copied().unwrap_or(0) { 0 => cy, v => v };
                    let x = match ops.get(2).copied().unwrap_or(0) { 0 => cx, v => v };
                    if self.trace_screen {
                        self.screen_trace.push(format!(
                            "@draw_picture(number={number}, window={window}, y={y}, x={x})"
                        ));
                    }
                    let out_chars = self.v6_win0_out_chars;
                    self.pending_pictures.push(PictureEvent { number, window, x, y, erase: false, out_chars, margin_after: None });
                }
                StepResult::Continue
            }
            // EXT:0x07 erase_picture(picture-number, y, x) — ZMSD §15: same
            // operands as draw_picture; paints the picture's region to the
            // window's background colour instead of displaying it. Recorded
            // with `erase: true` (see PictureEvent doc for why not a
            // `number: 0` sentinel).
            0x07 => {
                // v6-only: a non-v6 story keeps the Phase 0 no-op (no event).
                if let Some(v6) = self.screen.v6.as_ref() {
                    let window = v6.current;
                    let number = ops.first().copied().unwrap_or(0);
                    // Zero y/x defaults to the window's pixel cursor, as for
                    // draw_picture.
                    let (cy, cx) = v6
                        .windows
                        .get(window as usize)
                        .map(|w| (w.y_cursor.max(1), w.x_cursor.max(1)))
                        .unwrap_or((1, 1));
                    let y = match ops.get(1).copied().unwrap_or(0) { 0 => cy, v => v };
                    let x = match ops.get(2).copied().unwrap_or(0) { 0 => cx, v => v };
                    if self.trace_screen {
                        self.screen_trace.push(format!(
                            "@erase_picture(number={number}, window={window}, y={y}, x={x})"
                        ));
                    }
                    let out_chars = self.v6_win0_out_chars;
                    self.pending_pictures.push(PictureEvent { number, window, x, y, erase: true, out_chars, margin_after: None });
                }
                StepResult::Continue
            }
            // EXT:0x08 set_margins(left, right, window) — ZMSD §15: set the
            // left/right text margins (pixels) of the window. The main text
            // window (0) uses this to keep its text inside a graphical border
            // frame (e.g. Zork0 insets past its ~36px side columns). Window
            // defaults to the current window when omitted.
            0x08 => {
                let left = ops.first().copied().unwrap_or(0);
                let right = ops.get(1).copied().unwrap_or(0);
                let win = match ops.get(2).copied() {
                    Some(w) => self.v6_window_operand(w),
                    None => self.screen.v6.as_ref().map_or(0, |v| v.current as u16),
                };
                if self.trace_screen {
                    self.screen_trace.push(format!("@set_margins(left={left}, right={right}, win={win})"));
                }
                if let Some(v6) = self.screen.v6.as_mut() {
                    if let Some(w) = v6.windows.get_mut(win as usize) {
                        w.left_margin = left;
                        w.right_margin = right;
                        // ZMSD §15: "If the cursor is overtaken and now lies
                        // outside the margins altogether, move it back to the
                        // left margin of the current line." Outside means past
                        // EITHER edge — left of the left margin, or right of the
                        // text column's right edge (x_size - right). Frotz snaps
                        // on the same two-sided test (`x_cursor <= left ||
                        // x_cursor > x_size - right`); cursor and margins are
                        // pixels.
                        let text_right = w.x_size.saturating_sub(right);
                        if w.x_cursor <= left || w.x_cursor > text_right {
                            w.x_cursor = left + 1;
                        }
                    }
                }
                // A set_margins directly after a draw_picture on the same window
                // is the v6 inline-picture idiom (text flows beside the art):
                // attach the margin to that event so the host can lay the float
                // out with the game's own computed text-start.
                if let Some(ev) = self.pending_pictures.last_mut() {
                    if ev.window as u16 == win && ev.margin_after.is_none() && !ev.erase {
                        ev.margin_after = Some(left);
                    }
                }
                StepResult::Continue
            }
            // EXT:0x14 scroll_window(window, pixels) — ZMSD §15: "Scrolls the
            // given window by the given number of pixels (a negative value
            // scrolls backwards, i.e., down) writing in blank (background
            // colour) pixels in the new lines." No store/branch.
            //
            // Window 0 (the main scrolling window) is owned by the host's
            // transcript renderer, not this grid model, so a scroll of it is a
            // deliberate NO-OP — and a SILENT one. It is not an error case: it
            // is the normal back half of Zork Zero's inline-picture idiom, which
            // reads window 0's cursor, scrolls up to free vertical room for a
            // room icon, homes the cursor into the freed band, draws there, and
            // sets margins so prose flows beside the art. The host lays that
            // picture out as an inline transcript band instead — the transcript
            // has already scrolled by exactly the text it printed — so obeying
            // the pixel scroll would double it. Warning about it would fire on
            // essentially every illustrated room description; the `@scroll_window`
            // trace line above is the diagnostic channel. (See docs/standards.md,
            // "Where we knowingly differ".)
            //
            // Grid windows 1–7 DO shift their pixel-positioned text runs and cell
            // grid, via `ZWindow::scroll_pixels`.
            0x14 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0));
                let pixels = ops.get(1).copied().unwrap_or(0) as i16;
                if self.trace_screen {
                    self.screen_trace.push(format!("@scroll_window(win={win}, pixels={pixels})"));
                }
                if win == 0 {
                    // Intentionally silent — see above.
                } else if let Some(v6) = self.screen.v6.as_mut() {
                    if let Some(w) = v6.windows.get_mut(win as usize) {
                        w.scroll_pixels(pixels);
                    }
                }
                StepResult::Continue
            }
            // EXT:0x1C picture_table(table) — ZMSD §15: "Given a table of
            // picture numbers, the interpreter may if it wishes load or
            // unpack these pictures from disc into a cache for convenient
            // rapid plotting later." This engine's picture pipeline decodes
            // Blorb resources on demand host-side, so pre-caching buys
            // nothing — formal no-op, trace only.
            0x1C => {
                let table = ops.first().copied().unwrap_or(0);
                if self.trace_screen {
                    self.screen_trace.push(format!("@picture_table(table={table:#06x})"));
                }
                StepResult::Continue
            }
            // EXT:0x16 read_mouse(array) — ZMSD §15: "The four words in the array
            // are written with the mouse y coordinate, x coordinate, button bits,
            // and a menu word." Coordinates come from the last `set_mouse` click;
            // menus are unsupported so the menu word is always 0.
            0x16 => {
                let array = ops.first().copied().unwrap_or(0) as u32;
                if self.trace_screen {
                    self.screen_trace.push(format!(
                        "@read_mouse(array={array:#06x}) -> y={} x={} buttons={:#06x}",
                        self.mouse_y, self.mouse_x, self.mouse_buttons
                    ));
                }
                if array != 0 {
                    self.mem.write_word(array, self.mouse_y); // word 0: y
                    self.mem.write_word(array + 2, self.mouse_x); // word 1: x
                    self.mem.write_word(array + 4, self.mouse_buttons); // word 2
                    self.mem.write_word(array + 6, 0); // word 3: menu (unsupported)
                }
                StepResult::Continue
            }
            // EXT:0x17 mouse_window(window) — ZMSD §15: "Constrain the mouse arrow
            // to sit inside the given window. By default it sits in window 1.
            // Setting to -1 takes all restriction away." Recorded for
            // observability; the host owns actual pointer confinement.
            0x17 => {
                let win = self.v6_window_operand(ops.first().copied().unwrap_or(0)) as i16;
                if self.trace_screen {
                    self.screen_trace.push(format!("@mouse_window(window={win})"));
                }
                self.mouse_window = win;
                StepResult::Continue
            }
            0x1A => {
                // print_form — no-op in Phase 0 (a different lane; left untouched).
                // Stub; SQ-0457 tracks a real implementation (would consume the
                // formatted-text table stream-3 close now produces — see
                // `wrap_stream3_text` in screen.rs).
                StepResult::Continue
            }
            // Unknown / unimplemented EXT opcode: record once, then ignore
            // (mirrors the VAR fallthrough for observability parity).
            _ => {
                if self.warned_ext_opcodes.insert(opcode) {
                    self.diagnostics.push(format!(
                        "unimplemented EXT opcode 0x{opcode:02X} (ignored)"
                    ));
                }
                StepResult::Continue
            }
        }
    }

    /// Read the length-prefixed ASCII filename string for the v5 aux opcodes:
    /// byte 0 is the length, followed by that many ASCII bytes. Bounds-safe --
    /// returns an empty string (a valid table key) for a 0 / out-of-range addr.
    fn read_aux_name(&self, addr: u32) -> String {
        if addr == 0 || addr as usize >= self.mem.len() {
            return String::new();
        }
        let len = self.mem.read_byte(addr) as u32;
        let mut s = String::with_capacity(len as usize);
        for i in 0..len {
            let a = addr + 1 + i;
            if a as usize >= self.mem.len() { break; }
            s.push(self.mem.read_byte(a) as char);
        }
        s
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Resolve an `Operand` to a u16 value.
    /// NOTE: `Var` resolution may pop the eval stack — call left-to-right exactly once.
    fn resolve(&mut self, op: &Operand) -> u16 {
        match op {
            Operand::Large(v) => *v,
            Operand::Small(v) => *v as u16,
            Operand::Var(n) => read_var(&mut self.state, &self.mem, *n),
        }
    }

    /// Discard call frames until exactly `target_depth` remain, truncating the
    /// shared eval stack to each popped frame's base. Used by `throw` to unwind
    /// the stack back to the depth a matching `catch` recorded.
    fn unwind_to_depth(&mut self, target_depth: usize) {
        while self.state.frames.len() > target_depth {
            if let Some(f) = self.state.frames.pop() {
                self.state.eval_stack.truncate(f.eval_base);
            }
        }
    }

    /// Execute a branch: if condition matches `branch.on_true`, take the branch.
    ///
    /// Offset 0 → return false (0) from current routine.
    /// Offset 1 → return true (1) from current routine.
    /// Else → pc += offset - 2  (offset is relative to next_pc already in state.pc).
    pub fn do_branch(&mut self, branch: Option<Branch>, cond: bool) {
        let br = match branch {
            Some(b) => b,
            None => return,
        };
        if cond == br.on_true {
            match br.offset {
                0 => return_value(&mut self.state, &mut self.mem, 0),
                1 => return_value(&mut self.state, &mut self.mem, 1),
                off => {
                    self.state.pc = (self.state.pc as i32 + off as i32 - 2) as u32;
                }
            }
        }
    }

    /// Sum (mod 0x10000) of the story bytes [0x40, file_length) — every byte
    /// past the header up to the declared file length (ZMSD §11.1.6). file_length =
    /// header word 0x1A * scale (2 for v3, 4 for v4-5, 8 for v6+).
    pub fn story_checksum(&self) -> u16 {
        let scale: u32 = match self.mem.version() {
            1..=3 => 2,
            4 | 5 => 4,
            _ => 8,
        };
        let file_length = self.mem.read_word(0x1A) as u32 * scale;
        let end = file_length.min(self.mem.len() as u32);
        // Checksum the ORIGINAL story image: the dynamic region [0..static_mem_base)
        // may have been mutated by the running game, so read it from the snapshot
        // captured at load; the static region never changes, so read it from `mem`.
        let dyn_end = self.original_dynamic.len() as u32;
        let mut sum: u16 = 0;
        for addr in 0x40..end {
            let b = if addr < dyn_end {
                self.original_dynamic[addr as usize]
            } else {
                self.mem.read_byte(addr)
            };
            sum = sum.wrapping_add(b as u16);
        }
        sum
    }

    /// Write tokens into a parse buffer in the standard format (ZMSD §15):
    /// [byte0 = max words][byte1 = count]; then per word: 2-byte dict address,
    /// 1-byte length, 1-byte text-buffer position (1-based: `text_data_start +
    /// token.text_pos`). When `flag` is set, slots for words not in the dictionary
    /// (dict_addr == 0) are left untouched.
    fn write_parse_buffer(
        &mut self,
        parse: u32,
        tokens: &[dictionary::Token],
        text_data_start: u8,
        flag: bool,
    ) {
        let max = self.mem.read_byte(parse) as usize;
        let n = tokens.len().min(max);
        self.mem.write_byte(parse + 1, n as u8);
        for (i, t) in tokens.iter().take(max).enumerate() {
            if flag && t.dict_addr == 0 {
                continue;
            }
            let base = parse + 2 + (i as u32) * 4;
            self.mem.write_word(base, t.dict_addr);
            self.mem.write_byte(base + 2, t.len);
            self.mem.write_byte(base + 3, text_data_start + t.text_pos);
        }
    }

    /// Read the already-entered line out of a text buffer (the inverse of the
    /// write done by `supply_line`), as raw ZSCII bytes — the buffer's native
    /// representation, and what `Dictionary::tokenise` consumes. Layout (ZMSD §15):
    ///   v1–4: byte 0 = max; text starts at byte 1, 0-terminated.
    ///   v5+:  byte 0 = max; byte 1 = char count; text starts at byte 2.
    fn read_text_buffer(&self, text_buf: u32) -> Vec<u8> {
        if self.mem.version() <= 4 {
            let mut s = Vec::new();
            let mut addr = text_buf + 1;
            loop {
                // Stop at end-of-memory: a buffer with no 0 terminator would
                // otherwise latch a spurious OOB fault for the next step().
                if addr as usize >= self.mem.len() {
                    break;
                }
                let b = self.mem.read_byte(addr);
                if b == 0 {
                    break;
                }
                s.push(b);
                addr += 1;
            }
            s
        } else {
            let count = self.mem.read_byte(text_buf + 1) as u32;
            (0..count)
                .map(|i| self.mem.read_byte(text_buf + 2 + i))
                .collect()
        }
    }

    /// v5+: does `ch` terminate line input? Enter (13) always does; otherwise, if a
    /// terminating-characters table (header 0x2E) is present, any listed char does
    /// (255 in the table = any function key, i.e. ch >= 129). ZMSD §10.7.
    pub fn is_terminator(&self, ch: u16) -> bool {
        if ch == 13 {
            return true;
        }
        if self.mem.version() < 5 {
            return false;
        }
        let mut p = self.mem.read_word(0x2E) as u32;
        if p == 0 {
            return false;
        }
        loop {
            // Bound the walk at end-of-memory: a malformed table with no 0
            // terminator before EOF would otherwise read past the story image
            // and latch a spurious memory fault (drained — and reported — by
            // the NEXT step(), which had nothing to do with it).
            if p as usize >= self.mem.len() {
                return false;
            }
            let t = self.mem.read_byte(p) as u16;
            if t == 0 {
                return false;
            }
            if t == 255 {
                if ch >= 129 {
                    return true;
                }
            } else if t == ch {
                return true;
            }
            p += 1;
        }
    }

    /// While a *timed* `read`/`read_char` is pending, return `(time_tenths,
    /// packed_routine)`. `None` for an untimed read or when no read is pending.
    /// The clock lives in the host: it polls input for `time_tenths * 100` ms and
    /// calls `run_timed_interrupt` on each timeout.
    pub fn pending_timeout(&self) -> Option<(u16, u16)> {
        let p = self.pending_input?;
        if p.interrupt_time != 0 && p.interrupt_routine != 0 {
            Some((p.interrupt_time, p.interrupt_routine))
        } else {
            None
        }
    }

    /// Run the pending read's interrupt routine to completion and report whether
    /// input should abort. Called by the host once per elapsed timer interval.
    /// The routine's output flows to the normal sink; `pending_input` is left
    /// intact so an un-aborted read resumes. If the routine attempts nested
    /// input/save/restart (unsupported per ZMSD), the interrupt is abandoned and
    /// reported as non-aborting, with engine state restored.
    pub fn run_timed_interrupt(&mut self) -> TimedInterrupt {
        let saved = match self.pending_input {
            Some(p) if p.interrupt_routine != 0 => p, // PendingInput: Copy
            _ => return TimedInterrupt { aborted: false },
        };
        let ret = self.run_routine(saved.interrupt_routine);
        TimedInterrupt { aborted: ret != 0 }
    }

    /// Call `packed_routine` to completion and return its value. Safe whether or
    /// not a read is pending: it snapshots `pending_input` and restores it if the
    /// routine attempts nested input/save/restart (unsupported — the routine is
    /// then abandoned and 0 is returned). On the normal path `pending_input` is
    /// left untouched. Used by timed-input interrupts and by the sound
    /// finish-routine callback.
    pub fn run_routine(&mut self, packed_routine: u16) -> u16 {
        let saved = self.pending_input; // Option<PendingInput>: Copy
        let base_frames = self.state.frames.len();
        let base_stack = self.state.eval_stack.len();
        // Push the routine, storing its return value onto the eval stack (var 0).
        call_routine(&mut self.state, &mut self.mem, packed_routine, &[], Some(0));
        if self.state.frames.len() == base_frames {
            // packed 0 / bad addr: call_routine pushed 0 to the stack already.
            return self.state.eval_stack.pop().unwrap_or(0);
        }
        loop {
            match self.step() {
                StepResult::Continue => {
                    if self.state.frames.len() <= base_frames {
                        break;
                    }
                }
                // Nested input/save/restart/quit inside the routine: unsupported.
                // Unwind and restore, including pending_input (a nested read opcode
                // may have overwritten it).
                _ => {
                    self.state.frames.truncate(base_frames);
                    self.state.eval_stack.truncate(base_stack);
                    self.pending_input = saved;
                    return 0;
                }
            }
        }
        let ret = self.state.eval_stack.pop().unwrap_or(0);
        // Guard: a well-behaved routine leaves the stack where we started.
        self.state.eval_stack.truncate(base_stack);
        ret
    }

    /// v6 newline interrupt (ZMSD §8.8.3.2.2) — the print-path analogue of
    /// Frotz's `countdown()` (`src/common/screen.c`). Each new-line a *scrolling*
    /// window emits decrements that window's interrupt countdown (prop 9); when it
    /// reaches zero the routine whose packed address is in prop 8 is called "before
    /// text printing resumes". Called once per '\n' streamed to the window in
    /// [`print_text`].
    ///
    /// Semantics matched to Frotz: `if countdown != 0 { if --countdown == 0 { call }}`
    /// — the routine fires exactly once, and because the countdown is now 0 any
    /// new-line the routine itself emits is a no-op (Frotz's `!= 0` guard), so the
    /// zeroed prop 9 *is* the re-entrancy guard. The routine "should not attempt to
    /// print anything" (§8.8.3.2.2); Zork0 r393's is three instructions — set a
    /// flag byte, `set_margins 0,0`, `rtrue` — i.e. it rolls prose back inside its
    /// border frame. We run it synchronously via [`run_routine`], which safely
    /// abandons (and restores state) if a spec-violating routine attempts a nested
    /// blocking read our step model cannot suspend for. `newline_interrupt_active`
    /// hard-stops recursion for a pathological routine that both prints and re-arms
    /// prop 9 (Frotz would stack-overflow there; we simply skip).
    ///
    /// No-op below v6 (window props 8/9 do not exist), and no-op unless the
    /// countdown is armed, so it costs a single field read on the hot path.
    fn newline_interrupt(&mut self, win: usize) {
        if self.newline_interrupt_active {
            return;
        }
        let routine = match self.screen.v6.as_mut().and_then(|v6| v6.windows.get_mut(win)) {
            Some(w) if w.interrupt_countdown != 0 => {
                w.interrupt_countdown -= 1;
                if w.interrupt_countdown != 0 {
                    return; // not yet zero — just counted down
                }
                w.interrupt_routine
            }
            _ => return,
        };
        if routine == 0 {
            return;
        }
        self.newline_interrupt_active = true;
        self.run_routine(routine);
        self.newline_interrupt_active = false;
    }

    /// Advance the current v6 window's cursor as `s` streams out through the
    /// scrolling prose path (ZMSD §8.8.3.2.7: the cursor a game reads back
    /// must be accurate for the text already printed — with nothing held in a
    /// buffer here, keeping the cursor current at print time IS the flush).
    ///
    /// Mirrors frotz `screen_char`: a glyph that no longer fits between the
    /// margins wraps first (this regime always has wrapping on — it is what
    /// selects it), then the cursor advances one glyph. The host does the
    /// real, word-aware wrapping of prose, so the model's line breaks are an
    /// approximation of the visible ones; they exist to keep props 4/5 (and
    /// the line count) moving, not to place pixels.
    ///
    /// No-op below v6.
    fn v6_advance_prose_cursor(&mut self, s: &str) {
        let fw = V6_FONT_WIDTH;
        let Some(v6) = self.screen.v6.as_mut() else {
            return;
        };
        let idx = (v6.current as usize).min(7);
        let w = &mut v6.windows[idx];
        for ch in s.chars() {
            if ch == '\n' {
                w.prose_new_line();
                continue;
            }
            let right_edge = w.x_size.saturating_sub(w.right_margin);
            if w.x_cursor.saturating_add(fw).saturating_sub(1) > right_edge {
                w.prose_new_line();
            }
            w.x_cursor = w.x_cursor.saturating_add(fw);
        }
    }

    /// Reload every v6 window's line count (ZMSD §8.8.3.2.2 / §8.8.3.2.6 —
    /// see [`crate::screen::ZWindow::reload_line_count`]). Frotz does this for
    /// all eight windows whenever a keystroke actually arrives (not on a
    /// timeout), in `console_read_input` and `console_read_key`; without it a
    /// long game walks the count down to the -999 floor and silently turns
    /// "[MORE]" off for good. No-op below v6.
    fn v6_reload_line_counts(&mut self) {
        if let Some(v6) = self.screen.v6.as_mut() {
            for w in v6.windows.iter_mut() {
                w.reload_line_count();
            }
        }
    }

    /// The current v6 window's line count (property 15) as a signed number:
    /// how many more lines it prints before "[MORE]" falls due (zero or below
    /// = due; [`crate::screen::NEVER_MORE`] = never). `None` below v6.
    ///
    /// For the host's pager — the engine only maintains the count (decrement
    /// per new-line, floor at -999, reload on input); deciding when to show
    /// "[MORE]" stays a host job.
    pub fn v6_line_count(&self) -> Option<i16> {
        self.screen
            .v6
            .as_ref()
            .map(|v6| v6.windows[(v6.current as usize).min(7)].line_count_signed())
    }

    /// ZMSD §8.8.3.2.6: "A line count of -999 means 'never print [MORE]'."
    /// True only for a v6 story whose current window is parked at the
    /// sentinel — the device Zork Zero's demonstration mode uses (§8 Remarks).
    pub fn v6_suppress_more(&self) -> bool {
        self.v6_line_count() == Some(crate::screen::NEVER_MORE)
    }

    /// Complete a pending timed read as *interrupted* (the interrupt routine
    /// returned true / the host timed out): `read_char` stores ZSCII 0;
    /// `read` writes the partial `typed` line and stores terminator 0 (v5+).
    /// Delegates to `supply_char`/`supply_line`, which clear `pending_input`.
    /// No-op if no read is pending.
    pub fn abort_timed_input(&mut self, typed: &str) {
        match self.pending_input {
            Some(p) if p.text_buf == 0 => {
                // read_char: deliver ZSCII 0.
                self.supply_char(0);
            }
            Some(_) => {
                // read (line): partial buffer, terminator 0.
                self.supply_line(typed, 0);
            }
            None => {}
        }
    }

    /// Store `val` into variable `var` if `var` is Some.
    pub fn do_store(&mut self, var: Option<u8>, val: u16) {
        if let Some(v) = var {
            write_var(&mut self.state, &mut self.mem, v, val);
        }
    }

    /// `save_undo` (EXT:0x09): push an in-memory snapshot and store the result.
    /// Stores -1 (0xFFFF) when undo is disabled (`undo_cap == 0`).
    pub(crate) fn do_save_undo(&mut self, store: Option<u8>) {
        if self.undo_cap == 0 {
            self.do_store(store, 0xFFFF);
            return;
        }
        let blob = self.save_quetzal();
        self.undo_stack.push(UndoSnapshot { blob, store });
        if self.undo_stack.len() > self.undo_cap {
            self.undo_stack.remove(0); // drop oldest
        }
        self.do_store(store, 1);
    }

    /// `restore_undo` (EXT:0x0A): restore the newest snapshot and resume, storing 2
    /// into the original `save_undo`'s target. Stores 0 (into this instruction's
    /// target) when the stack is empty or a restore fails.
    pub(crate) fn do_restore_undo(&mut self, store: Option<u8>) {
        match self.undo_stack.pop() {
            Some(snap) => match self.restore_quetzal(&snap.blob) {
                Ok(()) => self.do_store(snap.store, 2),
                Err(_) => self.do_store(store, 0),
            },
            None => self.do_store(store, 0),
        }
    }

    /// Route text through the output-stream state.
    ///
    /// If stream 3 is active, text goes to the memory table buffer (NOT the
    /// screen).  Otherwise it goes to `self.out` (subject to stream 1 being
    /// active).
    ///
    /// When the active font is Font 3 (character-graphics), character codes
    /// 32–126 are translated through the Font-3 Unicode mapping table before
    /// being stored in the upper window grid or forwarded to the output sink.
    /// With any other font the output is byte-identical to the input.
    pub fn print_text(&mut self, s: &str) {
        // ZMSD 7.1.2.5: when stream 3 is selected it is the ONLY output stream —
        // any future stream-2/4 transcript sink MUST be added below this early
        // return, never above it.
        if self.streams.stream3_active() {
            // Store one ZSCII byte per output char (ZMSD §7.1.2.5). Do the
            // mem-based conversion BEFORE borrowing &mut self.streams to
            // avoid a borrow conflict.
            let bytes: Vec<u8> = s.chars().map(|c| self.mem.zscii_from_unicode(c)).collect();
            self.streams.write_stream3_bytes(&bytes);
            return;
        }
        // Stream 2 (the transcript) takes a copy of everything printed while it
        // is selected — including v6 text that PAINTS rather than streams, which
        // used to fall out through the paint path's early return below (SQ-0537).
        // Which windows contribute is a per-window decision in v6: ZMSD §8.8.3.1
        // attribute "2: text copied to output stream 2 (the transcript, if
        // selected)" — frotz's `enable_scripting`, taken from the CURRENT
        // window's attribute bits. Below v6 the same bit is what keeps upper
        // window text out of transcripts: frotz's restart_screen gives window 0
        // attribute 15 (scripting on) and windows 1-7 attribute 8 (off).
        if self.streams.stream2 {
            let copy = match self.screen.v6.as_ref() {
                Some(v6) => v6.windows[(v6.current as usize).min(7)].scripting(),
                None => self.screen.current_window != 1,
            };
            if copy {
                self.streams.write_stream2(s);
            }
        }
        let font3 = self.screen.current_font == 3;
        // v6: prop-14 attributes route stream-vs-paint UNIFORMLY across all 8
        // windows. A window is a flowing-prose "main" window — its output
        // streams to the buffered stream-1/transcript path below (window 0's
        // classic route) — only when BOTH the wrapping (bit 0) and scrolling
        // (bit 1) attributes are set; otherwise its output PAINTS into the grid
        // at screen-absolute pixels (status lines, menus, graphics captions),
        // where the per-char `wrapping` bit still decides whether a paint run
        // wraps at the window's own width.
        //   * Infocom v6: prose streams via window 0 (default attrs 0b1111 =
        //     wrap+scroll); windows 1-7 stay paint (attrs 0b1000, no wrap/scroll)
        //     — Zork Zero even clears window 0's wrap bit (attrs 0b1110) to paint
        //     its InvisiClues menu.
        //   * Inform 6's v6 library prints prose into WINDOW 7 (its "main"
        //     window), which it explicitly sets to attrs 0b1111 (wrap+scroll);
        //     that diverts win7's prose to the transcript exactly like window 0,
        //     so Inform-library games (advent) show story text in hybrid mode
        //     while Zork0/Shogun (win7 lacks wrap+scroll) stay byte-identical.
        // (SQ-0459)
        if let Some(v6) = self.screen.v6.as_ref() {
            let cur = v6.current;
            let paint_mode = v6.windows[cur as usize].attributes & 0b11 != 0b11;
            if paint_mode {
                let style = self.screen.text_style;
                let idx = cur as usize;
                // Read the header before taking &mut self.screen — mirrors the
                // v1-5 upper-window grow bound below. Screen width (px, word
                // 0x22) is the clip bound for no-wrap printing; 0 (unwritten
                // header) leaves it unclipped.
                let screen_h = self.mem.read_byte(0x20) as u16;
                let screen_w_px = match self.mem.read_word(0x22) {
                    0 => u16::MAX,
                    w => w,
                };
                // Finished runs collect here with SCREEN-ABSOLUTE pixel coords
                // stamped at paint time (window origin + cursor, both 1-based),
                // then paint via `V6Windows::paint_run` after the window borrow
                // ends — v6 text is paint, and painting must trim whatever
                // earlier runs it covers (Shogun overprints its status line at
                // a fixed pixel cursor every turn).
                let mut finished: Vec<crate::screen::V6Text> = Vec::new();
                if let Some(w) = self.screen.v6.as_mut().and_then(|v6| v6.windows.get_mut(idx)) {
                    let fw = crate::screen::V6_FONT_WIDTH;
                    let fh = crate::screen::V6_FONT_HEIGHT;
                    let cols = w.grid.cols.max(1);
                    let (fg, bg) = (w.fg, w.bg);
                    let bound = screen_h.max(w.grid.rows) * fh; // px bound
                    // The cursor is in 1-based PIXELS; the cell it maps to is
                    // `(px-1)/font + 1`. Each printed run is also recorded at
                    // its exact pixel start (`texts`) for pixel-faithful rasters.
                    let mut run: Option<crate::screen::V6Text> = None;
                    // ZMSD §8.8.3.1.2.2: "If 'buffered printing' is on, then
                    // text is wrapped after the last word which could fit on a
                    // line. If not, then text is wrapped after the last
                    // character that could fit." Word wrapping therefore needs
                    // BOTH wrapping (attribute 0) and buffered printing
                    // (attribute 3); with buffering off the per-character wrap
                    // at the end of this loop stands (SQ-0535).
                    let word_wrap = w.wrapping() && w.buffered();
                    let chars: Vec<char> = s.chars().collect();
                    let mut i = 0;
                    while i < chars.len() {
                        let ch = chars[i];
                        i += 1;
                        if ch == '\n' {
                            if let Some(r) = run.take() {
                                finished.push(r);
                            }
                            if w.scrolling() {
                                w.tick_line_count();
                            }
                            w.y_cursor += fh;
                            w.x_cursor = w.left_margin + 1;
                            continue;
                        }
                        // Word wrap: measure the word about to start and break
                        // the line ahead of it if it cannot fit. Frotz buffers
                        // a word together with its leading space and drops that
                        // space at the break (`screen_word`), so the wrapped
                        // line does not end in a stray blank. A word longer than
                        // the whole line is left to the character wrap below.
                        // (Our buffer only spans one print call, so a word split
                        // across two calls still breaks mid-word — real v6 games
                        // print prose through the host path, not here.)
                        if word_wrap {
                            let at_space = ch == ' ';
                            if at_space || i == 1 {
                                let start = if at_space { i } else { i - 1 };
                                let word = chars[start..]
                                    .iter()
                                    .take_while(|c| **c != ' ' && **c != '\n')
                                    .count();
                                let col = ((w.x_cursor.max(1) - 1) / fw + 1) as usize;
                                let need = word + usize::from(at_space);
                                if word > 0
                                    && col > 1
                                    && word <= cols as usize
                                    && col + need - 1 > cols as usize
                                {
                                    if let Some(r) = run.take() {
                                        finished.push(r);
                                    }
                                    if w.scrolling() {
                                        w.tick_line_count();
                                    }
                                    w.y_cursor += fh;
                                    w.x_cursor = w.left_margin + 1;
                                    if at_space {
                                        continue; // the break consumes the space
                                    }
                                }
                            }
                        }
                        let out_ch = if font3 { font3_translate(ch) } else { ch };
                        // Wrapping is the window's attribute bit 0 (ZMSD
                        // §8.8.3.2 prop 14; frotz update_attributes). With it
                        // CLEAR — the boot default for windows 1-7 — text does
                        // NOT wrap at the window's own width: Shogun prints
                        // its boot-menu items through a 1-px caret window and
                        // they must paint rightward on the screen, clipped
                        // only at the screen edge.
                        let wrapping = w.attributes & 1 != 0;
                        let (r, c) = ((w.y_cursor.max(1) - 1) / fh + 1, (w.x_cursor.max(1) - 1) / fw + 1);
                        if !wrapping {
                            let abs_x = w.x_coord.max(1) + w.x_cursor.max(1) - 1;
                            if abs_x + fw - 1 > screen_w_px {
                                continue; // clipped at the screen edge; cursor pinned
                            }
                        }
                        if r > w.grid.rows && w.y_cursor <= bound {
                            w.grid.grow_rows(r);
                        }
                        w.grid.put(r, c, out_ch, style, fg, bg);
                        run.get_or_insert_with(|| crate::screen::V6Text {
                            y: w.y_coord.max(1) + w.y_cursor.max(1) - 1,
                            x: w.x_coord.max(1) + w.x_cursor.max(1) - 1,
                            text: String::new(),
                            style,
                            fg,
                            bg,
                        })
                        .text
                        .push(out_ch);
                        if wrapping && c >= cols {
                            if let Some(r) = run.take() {
                                finished.push(r);
                            }
                            if w.scrolling() {
                                w.tick_line_count();
                            }
                            w.y_cursor += fh;
                            w.x_cursor = w.left_margin + 1;
                        } else {
                            w.x_cursor += fw;
                        }
                    }
                    if let Some(r) = run.take() {
                        finished.push(r);
                    }
                }
                if let Some(v6) = self.screen.v6.as_mut() {
                    for r in finished {
                        v6.paint_run(idx, r);
                    }
                }
                return;
            }
            // wrap+scroll both set: fall through to the buffered stream path
            // below (window 0 normally; Inform's win7 prose window, SQ-0459).
        }
        // Window 1 (upper): write chars into the grid, do not stream.
        if self.screen.current_window == 1 {
            let style = self.screen.text_style;
            let cols = self.screen.upper.cols.max(1);
            // Games may draw in the upper window at rows below the split height
            // (Inform's menu library does this — e.g. LostPig's HELP menu splits
            // to 7 rows then prints 5 items at rows 6–10). Real interpreters keep
            // such writes on screen, so grow the grid to cover the target row,
            // bounded by the physical screen height (header byte 0x20).
            let screen_h = (self.mem.read_byte(0x20) as u16).max(self.screen.upper_window_rows);
            for ch in s.chars() {
                if ch == '\n' {
                    self.screen.cursor_row += 1;
                    self.screen.cursor_col = 1;
                    continue;
                }
                let out_ch = if font3 { font3_translate(ch) } else { ch };
                let (r, c) = (self.screen.cursor_row, self.screen.cursor_col);
                if r > self.screen.upper.rows && r <= screen_h {
                    self.screen.upper.grow_rows(r);
                }
                self.screen.upper.put(r, c, out_ch, style, self.screen.current_fg, self.screen.current_bg);
                if self.screen.cursor_col >= cols {
                    self.screen.cursor_row += 1;
                    self.screen.cursor_col = 1;
                } else {
                    self.screen.cursor_col += 1;
                }
            }
            return;
        }
        // Stream 3 is inactive; streams 1/2/4 apply.
        if self.streams.stream1 {
            // SQ-0585: a v6 game may run SEVERAL flowing-prose windows at once —
            // advent.z6's `style` opens one across the top of the screen and keeps
            // playing in another below it. Both are wrap+scroll, so both reach this
            // stream, and splicing them into one transcript scrolls the top window's
            // text away (the game itself warns that a correct interpreter must not).
            //
            // Which one carries the narrative is not a guess: ZMSD §8.8.3.1 gives every
            // v6 window an attribute 2, "text copied to output stream 2 (the transcript,
            // if selected)", and a game that splits its display sets it on the window
            // whose text is the transcript. advent does exactly that — window 7, where
            // the player types, has it; the window 3 it opens across the top does not.
            // So the game's own declaration decides, corroborated by the window it asks
            // for input through. Text bound for any other prose window is that window's
            // live screen state and goes to its own buffer.
            //
            // Only the DESTINATION changes here. Everything the game can observe —
            // the window cursor (props 4/5) and the §8.8.3.2.2 newline interrupt below
            // — happens either way, exactly as it would if the text had streamed.
            let divert = self.screen.v6.as_ref().and_then(|v6| {
                let cur = v6.current as usize;
                let w = &v6.windows[cur];
                (w.prose_window() && !w.copy_to_transcript() && v6.current != self.v6_input_window)
                    .then_some(cur)
            });
            if let Some(cur) = divert {
                if let Some(v6) = self.screen.v6.as_mut() {
                    v6.windows[cur].push_prose(s);
                }
            } else {
                // v6: this is window 0's (the main scrolling window's) output.
                // Count its chars so window-0 inline pictures can anchor to their
                // exact position in the text stream (PictureEvent::out_chars).
                if self.screen.v6.is_some() {
                    self.v6_win0_out_chars += s.chars().count() as u64;
                }
                let attrs = crate::io::TextAttrs {
                    style: self.screen.text_style,
                    fg: self.screen.current_fg,
                    bg: self.screen.current_bg,
                };
                if font3 {
                    let translated: String = s.chars().map(|ch| {
                        let code = ch as u32;
                        if (32..=126).contains(&code) { font3_translate(ch) } else { ch }
                    }).collect();
                    self.out.print_attr(&translated, attrs);
                } else {
                    self.out.print_attr(s, attrs);
                }
            }
            // The prose regime moves the window cursor too (SQ-0536). Before
            // this, v6 window 0 / win7 printed whole paragraphs without prop
            // 4/5 ever changing, so get_wind_prop and get_cursor reported the
            // position the window had before the print. Runs BEFORE the
            // newline interrupt so a prop-8 routine sees the post-newline
            // cursor, exactly as it would in frotz (whose screen_new_line
            // moves the cursor and only then counts down).
            self.v6_advance_prose_cursor(s);
            // v6 newline interrupt (ZMSD §8.8.3.2.2): this stream path is the
            // *scrolling* regime (v6 window 0 / Inform's wrap+scroll win7), the
            // only place Frotz counts new-lines. Tick the current window's prop-9
            // countdown once per '\n' emitted; `newline_interrupt` fires the prop-8
            // routine when it reaches zero. No-op below v6 and when disarmed.
            // Firing after the line is streamed matches Zork0 r393's fire-at-end
            // quirk (Frotz's `story_id == ZORK_ZERO && h_release == 393` branch) and
            // is invisible for a spec-compliant routine, which prints nothing.
            if self.screen.v6.is_some() && s.contains('\n') {
                let win = self.screen.v6.as_ref().map_or(0, |v6| v6.current as usize);
                for _ in 0..s.matches('\n').count() {
                    self.newline_interrupt(win);
                }
            }
        }
    }

    /// Resolve a `print_char` ZSCII operand to the Unicode scalar to display.
    ///
    /// When interpreter number 6 (IBM PC) is set in the header by
    /// `init_header_caps` (via the `-I 6` / config override; the default is
    /// now 1/DEC-20), Infocom's IBM-PC-aware v4+ games — Beyond Zork in
    /// particular — emit their on-screen graphics (cursor arrows, box-drawing,
    /// block elements) as raw CP437 byte codes through `print_char` instead of
    /// switching to the portable Font 3. Translate those bytes through the CP437
    /// table so they render on a Unicode terminal.
    ///
    /// Gating keeps this faithful and non-regressive:
    ///   * Only when the header interpreter number is actually 6.
    ///   * The Z-machine output codes that mean SPACING rather than a glyph keep
    ///     their ZSCII meaning and are never remapped to CP437 glyphs: NUL (0),
    ///     tab (9), the invisible spacer (10), sentence space (11) and newline
    ///     (13). ZMSD §3.8 defines 9 and 11 for output in Version 6 — a sentence
    ///     space is "a suitable gap between two sentences" — and every v6 story
    ///     gets interpreter 6 by default, so without this Shogun's prose printed
    ///     CP437's 0x0B glyph (♂) between its sentences.
    ///   * Values > 255 (10-bit ZSCII) fall back to the standard mapping.
    ///   * 0x20–0x7E is ASCII either way, so ordinary text is unaffected.
    ///
    /// Z-string text (`print`, `print_paddr`, …) is decoded separately via
    /// `zscii_to_char`/the header Unicode table and is deliberately left on the
    /// standard ZSCII path, so accented prose in other games is never garbled.
    fn print_char_to_unicode(&self, zscii: u16) -> char {
        let interp_ibm_pc = self.mem.read_byte(0x1E) == 6;
        let is_control = matches!(zscii, 0 | 9 | 10 | 11 | 13);
        if interp_ibm_pc && !is_control && zscii <= 255 {
            cp437_to_char(zscii as u8)
        } else {
            zscii_to_char(zscii)
        }
    }

    /// Compute the v3 status line from memory globals.
    pub fn status_line(&self) -> crate::screen::StatusLine {
        crate::screen::compute_status_line(&self.mem)
    }

    /// Read global variable N (0-based). Convenience for tests and Tasks 11+.
    pub fn global(&self, n: u8) -> u16 {
        let base = self.mem.global_vars() as u32;
        self.mem.read_word(base + n as u32 * 2)
    }

    /// Complete a suspended `read` instruction by supplying a line of input.
    ///
    /// This is the natural hook for the future automapper to observe the player's
    /// command — the host calls this method with whatever the player typed, and
    /// could record `input` before forwarding to this function.
    ///
    /// Text-buffer layout (ZMSD §15):
    ///   v1–4: byte 0 = max chars; text starts at byte 1 (lower-cased, 0-terminated).
    ///   v5+:  byte 0 = max chars; byte 1 = actual char count; text starts at byte 2
    ///         (lower-cased, NOT zero-terminated).
    ///
    /// Parse-buffer layout (ZMSD §15):
    ///   byte 0 = max tokens (set by game); byte 1 = token count (we write this);
    ///   then for each token: 2-byte dict addr, 1-byte len, 1-byte text-buf position.
    ///   The text-buf position is 1-based from the start of the text buffer, i.e.
    ///   `text_data_start + token.text_pos` where text_data_start = 1 (v1–4) or 2 (v5+).
    ///
    /// For v5+: stores `terminator` (the ZSCII code of the key that ended the
    /// line — 13 for Enter, or a function-key code the host matched against the
    /// terminating-characters table) into the store variable. v1–4 have no store
    /// variable for `read`, so `terminator` is ignored there.
    /// Skips tokenisation when parse_buf == 0 (v5+ only).
    pub fn supply_line(&mut self, input: &str, terminator: u8) {
        let pending = match self.pending_input.take() {
            Some(p) => p,
            None => return, // no pending read — ignore
        };

        // A key actually arrived, so the v6 windows get a fresh screenful
        // before "[MORE]" is due again (frotz console_read_input, which skips
        // this on ZC_TIME_OUT — our timeout path is terminator 0).
        if terminator != 0 {
            self.v6_reload_line_counts();
        }

        let version = self.mem.version();
        let text_buf = pending.text_buf;
        let parse_buf = pending.parse_buf;

        // Read the max-length cap written by the game (byte 0 of text buffer).
        // ZMSD §15 `read`: in v1–4 byte 0 holds "the maximum number of letters
        // which can be typed, minus 1" — Frotz (input.c z_read) reads byte 0
        // then does `if (h_version <= V4) max--`, so the buffer holds at most
        // byte0−1 letters plus the 0 terminator (writing byte0 letters + NUL
        // overran the game's buffer by one byte). v5+: byte 0 IS the cap.
        let byte0 = self.mem.read_byte(text_buf) as usize;
        let max_len = if version <= 4 { byte0.saturating_sub(1) } else { byte0 };

        // Lower-case the input, convert each character to its ZSCII code
        // (custom Unicode table first, ZMSD §3.8.5.4 — the buffer stores ZSCII
        // bytes, never raw UTF-8: 'é' must land as one byte, code 170, or it
        // can never match a dictionary key), and truncate to max_len
        // CHARACTERS (byte-slicing a UTF-8 string here panicked mid-char).
        let text: Vec<u8> = input
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .take(max_len)
            .map(|c| self.mem.zscii_from_unicode(c))
            .collect();

        // Write the text into the buffer and set the count/terminator bytes.
        if version <= 4 {
            // v1–4: text starts at byte 1, terminated by a 0 byte.
            let text_data_start: u32 = 1;
            for (i, &b) in text.iter().enumerate() {
                self.mem.write_byte(text_buf + text_data_start + i as u32, b);
            }
            // Null-terminate.
            self.mem.write_byte(text_buf + text_data_start + text.len() as u32, 0);
        } else {
            // v5+: byte 1 = char count; text starts at byte 2, no null terminator.
            let text_data_start: u32 = 2;
            self.mem.write_byte(text_buf + 1, text.len() as u8);
            for (i, &b) in text.iter().enumerate() {
                self.mem.write_byte(text_buf + text_data_start + i as u32, b);
            }
        }

        // Tokenise and fill the parse buffer (skip if parse_buf == 0 in v5+).
        if parse_buf != 0 {
            let text_data_start: u8 = if version <= 4 { 1 } else { 2 };
            let dict = dictionary::load(&self.mem);
            let tokens = dict.tokenise(&self.mem, &text);
            self.write_parse_buffer(parse_buf, &tokens, text_data_start, false);
        }

        // v5+: store the terminating character the host supplied. `is_terminator`
        // is the host's oracle for which keys may end line input (ZMSD §10.7); by
        // the time we get here the host has already applied it.
        if version >= 5 {
            self.do_store(pending.store_var, terminator as u16);
        }
    }

    /// Complete a suspended `read_char` instruction by supplying a single keystroke.
    ///
    /// `ch` is the ZSCII code of the key pressed (e.g. 65 = 'A').
    /// The value is written into the instruction's store variable.
    pub fn supply_char(&mut self, ch: u8) {
        let pending = match self.pending_input.take() {
            Some(p) => p,
            None => return,
        };
        // As in `supply_line`: a real keystroke reloads the v6 line counts,
        // ZSCII 0 (our timed-read timeout) does not (frotz console_read_key).
        if ch != 0 {
            self.v6_reload_line_counts();
        }
        self.do_store(pending.store_var, ch as u16);
    }

    // -----------------------------------------------------------------------
    // Quetzal save / restore (Task 14)
    // -----------------------------------------------------------------------

    /// Serialise the current machine state to a Quetzal IFF byte buffer.
    ///
    /// The host (CLI / app) should call this after receiving `StepResult::SaveRequest`,
    /// then write the returned bytes to a file (or wherever), then call `complete_save`.
    pub fn save_quetzal(&self) -> Vec<u8> {
        crate::quetzal::save_quetzal(self)
    }

    /// The PC of the `read`/`read_char` instruction the machine is suspended on
    /// while awaiting input, if any. `step()` advances `state.pc` PAST the read
    /// before it suspends (so the resume lands on the code that consumes the input),
    /// so `state.pc` points to the NEXT instruction; this returns the read itself.
    /// The debugger folds it into the disassembly cache as a confirmed boundary so
    /// the parked read renders correctly instead of being eaten by a stale tiling.
    pub fn pending_read_pc(&self) -> Option<u32> {
        self.pending_input.as_ref().map(|pi| pi.instr_pc)
    }

    /// The program counter to record in a save file. For an in-game `@save`
    /// (pending_save set) this is the result descriptor's address, per Quetzal
    /// §5.8; otherwise (host Save State, undo snapshots) it is the current pc.
    pub(crate) fn save_pc(&self) -> u32 {
        // A game `@save` records the result-descriptor PC (Quetzal §5.8).
        if let Some(p) = self.pending_save.as_ref() {
            return p.descriptor_pc;
        }
        // A host Save State taken at an input prompt is suspended just past a
        // `read`/`read_char`; rewind to that instruction so restoring re-executes
        // the read and re-arms the prompt (otherwise the resume lands past the
        // read on a stale input buffer and replays the previous command).
        if let Some(pi) = self.pending_input.as_ref() {
            return pi.instr_pc;
        }
        self.state.pc
    }

    /// Whether a game-initiated `@save`/`@restore` is currently suspended,
    /// awaiting the host's file I/O (`complete_save` / `complete_restore_success`
    /// / `complete_restore_failure`).
    ///
    /// Hosts guard their unconditional snapshot triggers (exit auto-save, the
    /// quit dialog's "Save State & quit") on this. The Z-machine's hazard is not
    /// Glulx's un-popped call stub, it is [`save_pc`](Self::save_pc): while an
    /// `@save` is suspended, `save_pc` deliberately reports the result-descriptor
    /// address (Quetzal §5.8), so a HOST snapshot taken in that window records a
    /// PC pointing at a branch/store descriptor byte rather than at an
    /// instruction — restoring it later resumes by decoding that byte as an
    /// opcode. The twin of `gvm`'s `Machine::is_saveload_pending` (SQ-0661).
    pub fn is_saveload_pending(&self) -> bool {
        self.pending_save.is_some() || self.pending_restore
    }

    /// Deliver the result of a save operation back to the machine.
    ///
    /// `ok = true`  → save succeeded (v3: branch taken; v4+: store 1).
    /// `ok = false` → save failed    (v3: fall through; v4+: store 0).
    ///
    /// Must be called after `StepResult::SaveRequest` before the next `step()`.
    pub fn complete_save(&mut self, ok: bool) {
        let pending = match self.pending_save.take() {
            Some(p) => p,
            None => return,
        };
        match pending.result_dest {
            SaveDest::Branch(br) => self.do_branch(Some(br), ok),
            SaveDest::Store(sv) => self.do_store(Some(sv), if ok { 1 } else { 0 }),
        }
    }

    /// Restore machine state from a Quetzal byte buffer supplied by the host.
    ///
    /// On success the machine state (dynamic memory, frames, eval stack, PC) is
    /// replaced with the saved state and `Ok(())` is returned.  On failure the
    /// machine is untouched and an error is returned; the host should then call
    /// `complete_restore_failure()` to set the failure result.
    ///
    /// On a successful restore execution continues from the saved PC — the saved
    /// state already contains the correct resume address (the instruction after
    /// the save opcode) so no additional store/branch is needed.
    pub fn restore_quetzal(&mut self, data: &[u8]) -> Result<(), crate::error::ZError> {
        crate::quetzal::restore_quetzal(self, data)?;
        // A restore performed while the machine was suspended at a read must
        // drop that suspension: the restored PC comes from the SAVE, so a
        // stale PendingInput would make save_pc() rewind any subsequent host
        // save to the pre-restore read instruction (and supply_line would
        // write into buffers the restored game never armed). `restart()`
        // clears it for the same reason.
        //
        // A suspended `@save`/`@restore` is deliberately NOT cleared here: this is
        // the primitive `complete_restore_success` and `do_restore_undo` call while
        // their own descriptor is still live. Discarding an abandoned suspension is
        // the HOST restore's business — see `restore_file` (SQ-0661).
        self.pending_input = None;
        Ok(())
    }

    /// Restore from an external save file/archive. Like `restore_quetzal`, but also
    /// clears the in-memory undo stack on success — a file restore invalidates the
    /// undo history (snapshots taken after the save point are no longer coherent
    /// with the restored state). Use this for the host's file/restore paths;
    /// `restore_undo` keeps using `restore_quetzal` directly.
    pub fn restore_file(&mut self, data: &[u8]) -> Result<(), crate::error::ZError> {
        let dims = self.host_screen_dims();
        self.restore_quetzal(data)?; // also clears any stale pending_input
        self.undo_stack.clear();
        self.post_restore_fixups(dims);
        // A host restore REPLACES the run, so any game `@save`/`@restore` that run
        // had suspended on is abandoned along with it — the host will never call
        // `complete_save`/`complete_restore_*` for a descriptor that belongs to a
        // discarded machine. Left set, `pending_save` keeps winning in `save_pc`,
        // so the NEXT host Save State would record the dead run's descriptor PC and
        // a later restore of it would resume by decoding a branch/store descriptor
        // byte as an opcode; a stale `pending_restore_store` would likewise let a
        // later `complete_restore_failure` store 0 into the dead run's variable.
        // Same reasoning as the `pending_input` clear inside `restore_quetzal`.
        // (SQ-0661)
        //
        // This clearing belongs HERE and not in `restore_quetzal`:
        // `complete_restore_success` — the in-game `@restore` — calls
        // `restore_quetzal` *while* its own suspension is live and resolves it
        // afterwards from these very fields, and `do_restore_undo` calls it
        // mid-instruction where there is nothing suspended at all. Only the host
        // path means "that run is gone".
        self.pending_save = None;
        self.pending_restore_store = None;
        self.pending_restore = false;
        Ok(())
    }

    /// The screen dimensions the host last reported (header $20/$21), captured
    /// before a restore overwrites the header with the SAVED session's screen.
    fn host_screen_dims(&self) -> (u8, u8) {
        (self.mem.read_byte(0x20), self.mem.read_byte(0x21))
    }

    /// Re-establish the interpreter's own view of the header and screen after a
    /// restore has replaced dynamic memory with the saved session's copy.
    ///
    /// ZMSD §11.1: "'Rst' means that the interpreter must set it correctly after
    /// loading the game, after a restore or after a restart." Everything in that
    /// column is what [`Machine::init_caps`] writes — capability bits,
    /// interpreter number/version, screen dimensions and font size, the default
    /// colours in $2C/$2D and (Standard 1.1) header-extension words 4–6 — so a
    /// restore re-runs it, then re-applies the dimensions the HOST is actually
    /// showing (the save may have come from a different-sized screen; `init_caps`
    /// only seeds the generic default). This mirrors Frotz, whose `z_restore`
    /// calls `restart_header()` on success for exactly this reason.
    ///
    /// ZMSD §8.6.1.3: "Following a 'restore' of the game, the interpreter should
    /// automatically collapse the upper window to size 0." That clause lives
    /// under §8.6, "The screen model for Version 3", so the collapse is applied
    /// to v3 only — the same scoping Frotz uses (`if (h_version == V3)
    /// split_window(0)`). From v4 on the game owns its upper window across a
    /// restore.
    fn post_restore_fixups(&mut self, (rows, cols): (u8, u8)) {
        self.init_caps();
        if rows > 0 && cols > 0 {
            self.set_screen_dims(rows, cols);
        }
        if self.mem.version() <= 3 {
            self.screen.upper_window_rows = 0;
            let cols = self.mem.read_byte(0x21) as u16;
            self.screen.upper.resize(0, cols.max(1));
            self.screen.current_window = 0;
        }
    }

    /// Signal that a restore operation failed (no data / invalid data).
    ///
    /// v3: fall through (no branch taken); v4+: store 0 into the restore's
    /// store variable.  The store variable was captured into `pending_restore_store`
    /// when the restore opcode fired, so state.pc is already correct (pointing to
    /// the instruction after restore) and must not be modified here.
    pub fn complete_restore_failure(&mut self) {
        self.pending_restore = false;
        if self.mem.version() <= 3 {
            // v3 restore is a branch instruction; on failure just fall through
            // (no state change needed — execution continues at state.pc which
            // is already past the restore instruction).
        } else {
            // v4+: use the store variable captured when the restore opcode fired.
            if let Some(sv) = self.pending_restore_store.take() {
                self.do_store(Some(sv), 0);
            }
        }
    }

    /// Complete a game-initiated restore with the supplied Quetzal bytes.
    ///
    /// On success the machine state (dynamic memory, frames, eval stack, PC) is
    /// replaced with the saved state, whose PC points at the original `@save`'s
    /// result descriptor (Quetzal §5.8). We complete that descriptor forward as
    /// "restore succeeded": v3 takes the `@save` branch as true; v4+ stores 2
    /// into the `@save`'s store variable. A restore invalidates undo history, and
    /// the `@restore`'s own store target is unused on success — both are cleared.
    ///
    /// On `Err` the machine is untouched (the `restore_quetzal` contract); the
    /// caller should then call `complete_restore_failure()`.
    pub fn complete_restore_success(&mut self, data: &[u8]) -> Result<(), crate::error::ZError> {
        let dims = self.host_screen_dims();
        self.restore_quetzal(data)?;
        self.post_restore_fixups(dims);
        if self.mem.version() <= 3 {
            // v3 @save is a branch instruction; resume as if it branched on success.
            let br = crate::cpu::decode::decode_branch_at(&self.mem, self.state.pc);
            self.state.pc += br.len as u32; // advance to next_pc (do_branch uses pc + off - 2)
            self.do_branch(Some(br), true);
        } else {
            // v4+ @save stores its result; the game is being restored, so store 2.
            let store_var = self.mem.read_byte(self.state.pc);
            self.do_store(Some(store_var), 2);
            self.state.pc += 1; // advance past the store byte
        }
        self.undo_stack.clear();
        self.pending_restore_store = None;
        self.pending_restore = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Font-3 character-graphics mapping (ZMSD §16)
// ---------------------------------------------------------------------------

/// Translate a character code through Font 3's character-graphics table.
///
/// Source: Bocfel interpreter (garglk/garglk, terps/bocfel/unicode.cpp,
/// function `build_zscii_to_character_graphics_table`), which faithfully
/// implements the 8×8 bitmap descriptions in Z-Machine Standards Document §16.
/// https://inform-fiction.org/zmachine/standards/z1point1/sect16.html
///
/// Key BeyondZork cursor-arrow mappings (ZMSD §16):
///   code 92 ('\\') → U+2191 ↑  (cursor up)
///   code 93 (']')  → U+2193 ↓  (cursor down)
///
/// Codes not assigned in the table (71–74) map to U+FFFD (replacement character),
/// matching bocfel's treatment of unassigned entries.
/// Codes outside 32..=126 are not handled here; the caller passes them through.
fn font3_translate(ch: char) -> char {
    let code = ch as u32;
    match code {
        32  => '\u{0020}', // (space)
        33  => '\u{2190}', // ←
        34  => '\u{2192}', // →
        35  => '\u{2571}', // ╱
        36  => '\u{2572}', // ╲
        37  => '\u{0020}', // (space)
        38  => '\u{2500}', // ─
        39  => '\u{2500}', // ─
        40  => '\u{2502}', // │
        41  => '\u{2502}', // │
        42  => '\u{2534}', // ┴
        43  => '\u{252C}', // ┬
        44  => '\u{251C}', // ├
        45  => '\u{2524}', // ┤
        46  => '\u{2514}', // └
        47  => '\u{250C}', // ┌
        48  => '\u{2510}', // ┐
        49  => '\u{2518}', // ┘
        // 50-53: room-connection pieces (bocfel default: no alt-graphics → corner chars)
        50  => '\u{2514}', // └
        51  => '\u{250C}', // ┌
        52  => '\u{2510}', // ┐
        53  => '\u{2518}', // ┘
        54  => '\u{2588}', // █
        55  => '\u{2580}', // ▀
        56  => '\u{2584}', // ▄
        57  => '\u{258C}', // ▌
        58  => '\u{2590}', // ▐
        // 59-62: room-connection pieces (bocfel default: filled block corners)
        59  => '\u{2584}', // ▄
        60  => '\u{2580}', // ▀
        61  => '\u{258C}', // ▌
        62  => '\u{2590}', // ▐
        63  => '\u{259D}', // ▝
        64  => '\u{2597}', // ▗
        65  => '\u{2596}', // ▖
        66  => '\u{2598}', // ▘
        // 67-70: room-connection pieces (bocfel default: quarter-block corners)
        67  => '\u{259D}', // ▝
        68  => '\u{2597}', // ▗
        69  => '\u{2596}', // ▖
        70  => '\u{2598}', // ▘
        // 71-74: not assigned in the bocfel/§16 table → replacement character
        71..=74 => '\u{FFFD}',
        75  => '\u{2594}', // ▔
        76  => '\u{2581}', // ▁
        77  => '\u{258F}', // ▏
        78  => '\u{2595}', // ▕
        79  => '\u{0020}', // (space)
        80  => '\u{258F}', // ▏
        81  => '\u{258E}', // ▎
        82  => '\u{258D}', // ▍
        83  => '\u{258C}', // ▌
        84  => '\u{258B}', // ▋
        85  => '\u{258A}', // ▊
        86  => '\u{2589}', // ▉
        87  => '\u{2588}', // █
        88  => '\u{2595}', // ▕
        89  => '\u{258F}', // ▏
        90  => '\u{2573}', // ╳
        91  => '\u{253C}', // ┼
        92  => '\u{2191}', // ↑  (cursor up arrow — used by BeyondZork menu)
        93  => '\u{2193}', // ↓  (cursor down arrow — used by BeyondZork menu)
        94  => '\u{2195}', // ↕
        95  => '\u{2395}', // ⎕
        96  => '\u{003F}', // ?
        // 97-122: Elder Futhark runic letters (used by BeyondZork for atmosphere)
        97  => '\u{16AA}', // ᚪ
        98  => '\u{16D2}', // ᛒ
        99  => '\u{16C7}', // ᛇ
        100 => '\u{16DE}', // ᛞ
        101 => '\u{16D6}', // ᛖ
        102 => '\u{16A0}', // ᚠ
        103 => '\u{16B7}', // ᚷ
        104 => '\u{16BB}', // ᚻ
        105 => '\u{16C1}', // ᛁ
        106 => '\u{16C4}', // ᛄ
        107 => '\u{16E6}', // ᛦ
        108 => '\u{16DA}', // ᛚ
        109 => '\u{16D7}', // ᛗ
        110 => '\u{16BE}', // ᚾ
        111 => '\u{16A9}', // ᚩ
        112 => '\u{15BE}', // ᖾ
        113 => '\u{16B3}', // ᚳ
        114 => '\u{16B1}', // ᚱ
        115 => '\u{16CB}', // ᛋ
        116 => '\u{16CF}', // ᛏ
        117 => '\u{16A2}', // ᚢ
        118 => '\u{16E0}', // ᛠ
        119 => '\u{16B9}', // ᚹ
        120 => '\u{16C9}', // ᛉ
        121 => '\u{16A5}', // ᚥ
        122 => '\u{16DF}', // ᛟ
        // 123-126: reversed variants (per ZMSD §16 note); mirror arrows/? from 92-96
        123 => '\u{2191}', // ↑
        124 => '\u{2193}', // ↓
        125 => '\u{2195}', // ↕
        126 => '\u{003F}', // ?
        _   => ch,         // outside 32..=126: pass through (caller guards this range)
    }
}

// ---------------------------------------------------------------------------
// Memory accessor
// ---------------------------------------------------------------------------

impl Memory {
    /// Initial PC from the story header (direct instruction address for v3–v8).
    pub fn initial_pc(&self) -> u32 {
        self.read_word(0x06) as u32
    }
}

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

/// Decode a `set_colour` operand into a colour-channel update, for the
/// **non-v6** screen model. Returns `None` for 0 ("leave unchanged");
/// `Some(ZColour)` otherwise.
///
/// ZMSD §8.3.1 closes its colour table with "Colours 10, 11, 12, 15 and -1 are
/// available only in Version 6." Below v6 the three greys are not part of the
/// palette, so they are ignored (channel kept) rather than rendered — see
/// [`decode_set_colour_v6`] for the version that honours them.
fn decode_set_colour(v: u16) -> Option<crate::screen::ZColour> {
    use crate::screen::ZColour;
    match v {
        0 => None,                     // keep current channel
        1 => Some(ZColour::Default),   // default
        2..=9 => Some(ZColour::Standard(v as u8)), // the version-independent palette
        _ => None,                     // 10–12/15 (v6-only) / -1 / unknown → keep
    }
}

/// v6 variant of [`decode_set_colour`]: per the ZMSD §8.3.1 colour table,
/// **-1 is "the colour of the pixel under the cursor (if any) (true -3)"**, a
/// Version 6-only entry. (It is NOT transparency — §8.3.6 gives that to colour
/// 15, "true -4".) Zork Zero prints its banner labels under `COLOR 1 -1` so the
/// text draws over the ribbon art. Our compositor's closest equivalent to
/// "whatever is already there" is the inherited `Default` channel (which
/// renders with NO opaque fill), so -1 resolves to `Default` rather than "keep"
/// — keeping an earlier explicit bg (black from the border setup) painted
/// opaque boxes over the banner art.
fn decode_set_colour_v6(v: u16) -> Option<crate::screen::ZColour> {
    use crate::screen::ZColour;
    match v as i16 {
        -1 => Some(ZColour::Default), // pixel under the cursor → inherit (no opaque fill)
        // §8.3.1: light/medium/dark grey exist only from Version 6 on.
        10..=12 => Some(ZColour::Standard(v as u8)),
        _ => decode_set_colour(v),
    }
}

/// Decode a `set_true_colour` operand (signed). Returns `None` for "keep".
///
/// ZMSD §8.3.7 special values: (-1) = default, (-2) = current, (-3) = colour
/// under the cursor (V6 only), (-4) = transparent (V6 only). We map -1 to the
/// `Default` sentinel and -2 to "keep" (current channel, unchanged). -3 (pixel
/// under the cursor) needs render feedback we don't have headless, so it is a
/// no-op keep. -4 (transparent) is a background-only V6 feature we don't model
/// as a distinct state; §8.3.6 says "Interpreters not supporting transparency
/// must ignore any attempt to select colour 15", so ignoring (keep) is
/// conformant. Non-negative values are 15-bit true RGB (bit 15 is always 0).
fn decode_true_colour(v: u16) -> Option<crate::screen::ZColour> {
    use crate::screen::ZColour;
    match v as i16 {
        -1 => Some(ZColour::Default),      // default setting
        -2 => None,                        // current setting → keep channel
        -3 => None,                        // colour under cursor (V6) → keep (no render feedback)
        -4 => None,                        // transparent (V6) → ignore per §8.3.6 (keep)
        n if n >= 0 => Some(ZColour::True((n as u16) & 0x7FFF)),
        _ => None,                         // other negatives (bit15 set / invalid) → keep
    }
}

/// Pack a v6 window's fg/bg into the `colour_data` window property (prop 11,
/// ZMSD §8.4.3): high byte = background colour number, low byte = foreground
/// colour number. `True`/`True24` colours have no discrete colour number and
/// pack as 0 in that channel.
fn pack_colour_data(fg: crate::screen::ZColour, bg: crate::screen::ZColour) -> u16 {
    fn byte(c: crate::screen::ZColour) -> u8 {
        use crate::screen::ZColour::*;
        match c {
            Default => 1,
            Standard(n) => n,
            // ZMSD §8.3.5.2: "If the colour selected was not one of the standard
            // set ... the colour shown in property 11 will be >= 16." The §8
            // Remarks suggest allocating 16–255 to the last 240 distinct
            // non-standard colours; we use a cheaper scheme with the same
            // observable contract — hash the true colour into 16..=255. It is
            // STABLE (the same colour always reads back the same number) and
            // always >= 16; distinct colours may collide, which the spec permits
            // since property 11 is "implementation defined" for true colours
            // (§8.3.5) beyond the >= 16 rule.
            True(v) => 16 + (v % 240) as u8,
            True24(rgb) => 16 + (rgb % 240) as u8,
        }
    }
    ((byte(bg) as u16) << 8) | byte(fg) as u16
}

// ---------------------------------------------------------------------------
// Screen-trace decoders (trace feature, `screen` section)
// ---------------------------------------------------------------------------

fn zscreen_window_name(v: u16) -> String {
    match v as i16 {
        0 => "lower".into(), 1 => "upper".into(),
        -1 => "all(unsplit)".into(), -2 => "all".into(),
        other => format!("win{other}"),
    }
}
fn zscreen_style_name(bits: u16) -> String {
    if bits == 0 { return "roman".into(); }
    let mut p = Vec::new();
    if bits & 1 != 0 { p.push("reverse"); }
    if bits & 2 != 0 { p.push("bold"); }
    if bits & 4 != 0 { p.push("italic"); }
    if bits & 8 != 0 { p.push("fixed"); }
    if p.is_empty() { format!("0x{bits:x}") } else { p.join("|") }
}
fn zscreen_font_name(v: u16) -> String {
    match v { 0 => "query".into(), 1 => "normal".into(), 3 => "graphics".into(), 4 => "fixed".into(), n => format!("font{n}") }
}
fn zscreen_colour_name(c: crate::screen::ZColour) -> String {
    use crate::screen::ZColour::*;
    match c {
        Default => "default".into(),
        Standard(n) => format!("std{n}"),
        True(v) => format!("true(0x{v:04x})"),
        True24(rgb) => format!("#{:06X}", rgb & 0x00FF_FFFF),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::screen::ZColour;

    /// A v6 story whose `main` routine (0 locals) begins with `first_instr`.
    /// main is at byte 0x0040; packed routine addr 0x0010 (4·0x10, offset 0).
    fn v6_boot_story(first_instr: &[u8]) -> Vec<u8> {
        let mut buf = crate::header::tests_support::sample_story(6);
        buf[0x06] = 0x00; buf[0x07] = 0x10; // header 0x06 = packed addr of main
        buf[0x40] = 0x00;                   // routine header: 0 locals
        for (i, b) in first_instr.iter().enumerate() {
            buf[0x41 + i] = *b;             // first instruction(s) at 0x41
        }
        buf
    }

    #[test]
    fn v6_boot_pushes_main_frame() {
        let m = Machine::new(Memory::new(v6_boot_story(&[0xB0])).unwrap());
        assert_eq!(m.state.frames.len(), 1, "v6 boot calls main → one base frame");
    }

    #[test]
    fn v6_returning_from_main_quits() {
        // main body: rtrue (0OP:0x00 = 0xB0) → returns from main → frames empty → Quit
        let mut m = Machine::new(Memory::new(v6_boot_story(&[0xB0])).unwrap());
        assert!(matches!(m.step(), StepResult::Quit), "return from main ends the story");
    }

    #[test]
    fn v6_main_quit_opcode_quits() {
        // main body: @quit (0OP:0x0A = 0xBA) reached via the call → Quit
        let mut m = Machine::new(Memory::new(v6_boot_story(&[0xBA])).unwrap());
        assert!(matches!(m.step(), StepResult::Quit));
    }

    #[test]
    fn non_v6_boot_is_frameless() {
        let m = Machine::new(Memory::new(crate::header::tests_support::sample_story(5)).unwrap());
        assert!(m.state.frames.is_empty(), "v5 starts frameless at initial_pc");
    }

    #[test]
    fn v6_restart_reenters_main_reloads_dynmem_and_preserves_flags2() {
        // @restart (ZMSD §6.1.3 / §5.4): a v6 story must re-boot by re-entering
        // the packed `main` routine with a fresh frame, its dynamic memory reset
        // to the original story image, but the transcription + fixed-pitch bits
        // of Flags 2 preserved.
        let mut m = Machine::new(Memory::new(v6_boot_story(&[0xB0])).unwrap());
        // Snapshot pristine state, then dirty the machine as a mid-game run would.
        let orig_100 = m.mem.read_byte(0x100);
        let pristine_f2 = m.mem.read_word(0x10);
        m.mem.write_byte(0x100, orig_100 ^ 0xFF);       // mutate dynamic memory
        m.mem.write_word(0x10, pristine_f2 ^ 0b11);      // flip transcription+fixed-pitch
        m.state.frames.clear();                          // simulate stack unwound mid-run
        m.state.eval_stack.push(0xDEAD);
        m.undo_stack.push(UndoSnapshot { blob: Vec::new(), store: None });
        m.pending_pictures.push(PictureEvent { number: 1, window: 0, x: 1, y: 1, erase: false, out_chars: 0, margin_after: None });

        m.restart();

        assert_eq!(m.state.frames.len(), 1, "@restart re-enters v6 main → one base frame");
        assert_eq!(m.state.pc, 0x41, "PC resumes at main's first instruction");
        assert!(m.state.eval_stack.is_empty(), "the eval stack is emptied");
        assert!(m.undo_stack.is_empty(), "the undo stack is discarded (§6.1.3)");
        assert!(m.pending_pictures.is_empty(), "no pre-restart picture chrome carries over");
        assert_eq!(m.mem.read_byte(0x100), orig_100, "dynamic memory reloaded from the story image");
        assert_eq!(
            m.mem.read_word(0x10) & 0b11,
            (pristine_f2 ^ 0b11) & 0b11,
            "transcription (bit 0) + fixed-pitch (bit 1) preserved across @restart",
        );
        assert_eq!(
            m.screen.v6.as_ref().unwrap().windows[0].attributes, 15,
            "the v6 window model is re-seeded to boot defaults",
        );
        assert!(m.just_restarted, "restart signals the host to drop app-side chrome");
    }

    #[test]
    fn v1to5_restart_resets_to_initial_pc_and_reloads_dynmem() {
        // In v3–5, @restart resumes frameless at the header's initial PC with
        // dynamic memory reloaded and Flags 2's game bits preserved.
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        let initial_pc = m.mem.initial_pc();
        let orig_100 = m.mem.read_byte(0x100);
        let pristine_f2 = m.mem.read_word(0x10);
        m.mem.write_byte(0x100, orig_100 ^ 0xFF);
        m.mem.write_word(0x10, pristine_f2 ^ 0b11);
        m.state.pc = 0x1234;                             // wander off mid-run
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0, locals: vec![0; 2], eval_base: 0,
            store_var: None, arg_count: 0, func_addr: 0,
        });

        m.restart();

        assert!(m.state.frames.is_empty(), "v1–5 @restart resumes frameless");
        assert_eq!(m.state.pc, initial_pc, "PC resumes at the header initial PC");
        assert_eq!(m.mem.read_byte(0x100), orig_100, "dynamic memory reloaded");
        assert_eq!(
            m.mem.read_word(0x10) & 0b11,
            (pristine_f2 ^ 0b11) & 0b11,
            "transcription + fixed-pitch bits preserved across @restart",
        );
    }

    #[test]
    fn game_set_transcript_bit_warns_once_and_clears() {
        // ZMSD §7.4: Infocom-era games turn transcription on by SETTING Flags 2
        // bit 0 — not via `output_stream 2` — and the interpreter is expected to
        // watch the bit. With no transcript file supported, the bit is cleared
        // at the next input request (so the game honestly reports scripting
        // off) and the unsupported-transcript warning fires ONCE per session.
        // (SQ-0532 TTY-pass finding: typing `script` showed no warning because
        // only the opcode path was hooked.)
        let mut m = Machine::new(Memory::new(sample_story(3)).unwrap());
        let count = |m: &Machine| {
            m.diagnostics.iter().filter(|d| d.contains("transcript file")).count()
        };
        let f2 = m.mem.read_word(0x10);
        m.mem.write_word(0x10, f2 | 1); // the game's SCRIPT verb sets the bit
        m.exec_var(0x04, &[0x200, 0x220], None, None); // read → the turn boundary
        assert_eq!(m.mem.read_word(0x10) & 1, 0, "unsupported transcription bit is cleared");
        assert_eq!(count(&m), 1, "warning surfaced");
        // The game sets it again (SCRIPT after UNSCRIPT): cleared again, but
        // the warning stays once-per-session.
        let f2 = m.mem.read_word(0x10);
        m.mem.write_word(0x10, f2 | 1);
        m.exec_var(0x04, &[0x200, 0x220], None, None);
        assert_eq!(m.mem.read_word(0x10) & 1, 0, "cleared on every sighting");
        assert_eq!(count(&m), 1, "still only one warning");
    }

    #[test]
    fn pending_read_pc_is_the_read_instruction_not_the_advanced_pc() {
        // A read suspends with state.pc advanced PAST the read; pending_read_pc
        // returns the read instruction's own PC so the debugger can render it.
        let mut story = sample_story(3);
        story[0x40] = 0xe4; // VAR sread
        story[0x41] = 0xbf; // types: text_buf (large const), then omitted
        story[0x42] = 0x00; story[0x43] = 0x50; // text_buf = 0x0050
        let mem = crate::memory::Memory::new(story).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        assert_eq!(m.pending_read_pc(), Some(0x40), "read instruction pc");
        assert!(m.state.pc > 0x40, "state.pc advanced past the read");
    }

    // ── In-game restore-success: the original @save "returns 2" on restore ─────
    //
    // v4 story at 0x40:  save -> G0 (0xB5, store byte 0x10 at 0x41), then quit.
    // After step() the @save suspends with SaveRequest; the saved (IFhd) PC points
    // at the store byte 0x41 (Quetzal §5.8). complete_restore_success restores the
    // state, reads the store byte forward, stores 2 into G0, and resumes at 0x42.
    fn save_v4_into_g0_story() -> Vec<u8> {
        let mut buf = sample_story(4);
        buf[0x40] = 0xB5; // 0OP:0x05 save (store form, v4+)
        buf[0x41] = 0x10; // store -> global 0 (var 0x10)
        buf[0x42] = 0xBA; // quit
        buf
    }

    #[test]
    fn complete_restore_success_stores_2_and_resumes_pc() {
        let mem = Memory::new(save_v4_into_g0_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;

        // Execute @save -> SaveRequest; PC is now past the store byte (0x42).
        let r = m.step();
        assert_eq!(r, StepResult::SaveRequest, "save opcode suspends with SaveRequest");
        assert_eq!(m.state.pc, 0x42, "PC is post-instruction (store byte at 0x41)");

        // Host captures the Quetzal at the @save point, then completes the save.
        let blob = m.save_quetzal();
        m.complete_save(true);
        assert_eq!(m.global(0), 1, "save success stores 1 into G0");

        // Clobber G0 and move the PC away so the restore must reset BOTH.
        m.do_store(Some(0x10), 0x99);
        m.state.pc = 0x00AB;

        // Restore success: the ORIGINAL @save returns 2; PC resumes at 0x42.
        m.complete_restore_success(&blob).expect("restore must succeed");
        assert_eq!(m.global(0), 2, "restore makes the original @save 'return' 2");
        assert_eq!(m.state.pc, 0x42, "PC resumed at the post-@save address");
    }

    // v3: @save is a BRANCH instruction. 0x40 save (0xB5) + 1 branch byte (0x41).
    // Branch: on-true, short form, offset 5. next_pc after the branch byte is 0x42,
    // so a taken branch lands at 0x42 + 5 - 2 = 0x45.
    fn save_v3_branch_story() -> Vec<u8> {
        let mut buf = sample_story(3);
        buf[0x40] = 0xB5;          // 0OP:0x05 save (branch form in v3)
        buf[0x41] = 0x80 | 0x40 | 5; // branch: on-true, short form, offset 5
        buf[0x45] = 0xBA;          // quit at the branch-taken target
        buf
    }

    #[test]
    fn v3_branch_save_restore_round_trip() {
        let mem = Memory::new(save_v3_branch_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;

        let r = m.step();
        assert_eq!(r, StepResult::SaveRequest, "v3 save suspends with SaveRequest");
        assert_eq!(m.state.pc, 0x42, "PC post-instruction: opcode 0x40 + 1 branch byte");

        // Standard convention: saved (IFhd) PC points AT the branch descriptor (0x41).
        let blob = m.save_quetzal();
        assert_eq!(crate::quetzal::saved_pc_of(&blob), 0x41, "v3 saved PC = branch byte address");

        // Immediate save success takes the branch -> 0x45.
        m.complete_save(true);
        assert_eq!(m.state.pc, 0x45, "save success branches to 0x45");

        // Move PC away; restore must make the original @save 'succeed' (branch taken).
        m.state.pc = 0x00AB;
        m.complete_restore_success(&blob).expect("v3 restore must succeed");
        assert_eq!(m.state.pc, 0x45, "restore resumes as if the v3 @save branched");
    }

    // v5: @save is EXT:0x00 (0xBE 0x00), VAR types byte (0xFF = 0 operands), store byte.
    fn save_v5_ext_into_g0_story() -> Vec<u8> {
        let mut buf = sample_story(5);
        buf[0x40] = 0xBE; // EXT prefix
        buf[0x41] = 0x00; // EXT:0x00 save
        buf[0x42] = 0xFF; // VAR types: all 4 operands omitted
        buf[0x43] = 0x10; // store byte -> global 0
        buf[0x44] = 0xBA; // quit
        buf
    }

    #[test]
    fn v5_ext_save_restore_round_trip() {
        let mem = Memory::new(save_v5_ext_into_g0_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;

        let r = m.step();
        assert_eq!(r, StepResult::SaveRequest, "v5 EXT save suspends with SaveRequest");
        assert_eq!(m.state.pc, 0x44, "PC post-instruction (store byte at 0x43)");

        let blob = m.save_quetzal();
        assert_eq!(crate::quetzal::saved_pc_of(&blob), 0x43, "v5 saved PC = store byte address");

        m.complete_save(true);
        assert_eq!(m.global(0), 1, "save success stores 1");

        m.do_store(Some(0x10), 0x99);
        m.state.pc = 0x00AB;
        m.complete_restore_success(&blob).expect("v5 restore must succeed");
        assert_eq!(m.global(0), 2, "restore makes the original @save 'return' 2");
        assert_eq!(m.state.pc, 0x44, "PC resumes post-@save");
    }

    #[test]
    fn save_state_host_path_keeps_state_pc() {
        // No @save opcode fired (pending_save is None): save_quetzal must serialize
        // state.pc verbatim — the host "Save State" convention is unchanged.
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0123;
        let blob = m.save_quetzal();
        assert_eq!(crate::quetzal::saved_pc_of(&blob), 0x0123, "host save keeps state.pc");
    }

    #[test]
    fn complete_restore_success_err_on_corrupt_blob_leaves_state() {
        let mem = Memory::new(save_v4_into_g0_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x42;
        m.do_store(Some(0x10), 0x77); // sentinel

        let err = m.complete_restore_success(b"not a quetzal blob");
        assert!(err.is_err(), "corrupt blob must return Err");
        assert_eq!(m.global(0), 0x77, "state untouched on restore failure");
        assert_eq!(m.state.pc, 0x42, "pc untouched on restore failure");
    }

    #[test]
    fn undo_save_restore_round_trip() {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.undo_cap = 4;
        m.state.pc = 0x0040;
        m.do_store(Some(0x11), 1); // G1 = 1 (pre-save value)

        // save_undo storing to G0: snapshot taken (PC 0x0040), G0 := 1, one entry.
        m.do_save_undo(Some(0x10));
        assert_eq!(m.global(0), 1, "save_undo stores 1");
        assert_eq!(m.undo_stack.len(), 1);

        // Move the PC AWAY before restoring, so the PC assertion isolates
        // restore_quetzal's PC write (not a value the snapshot already held).
        m.state.pc = 0x00AB;
        // Mutate G1, then restore_undo storing to G2.
        m.do_store(Some(0x11), 0x99);
        m.do_restore_undo(Some(0x12));
        assert_eq!(m.global(1), 1, "G1 reverted to the snapshot value");
        assert_eq!(m.global(0), 2, "the original save_undo 'returns' 2");
        assert_eq!(m.state.pc, 0x0040, "restore_undo restored the snapshot PC (0x0040, not 0x00AB)");
        assert!(m.undo_stack.is_empty(), "snapshot consumed");
    }

    // ── A host restore ABANDONS the run's suspended @save/@restore (SQ-0661) ───

    /// v4 story: 0x40 `@save -> G0` (store byte at 0x41), 0x42 quit. 0x50 is an
    /// unrelated "somewhere else" the elsewhere-snapshot is taken at.
    fn save_v4_with_an_elsewhere_story() -> Vec<u8> {
        let mut buf = sample_story(4);
        buf[0x40] = 0xB5; // 0OP:0x05 save (store form, v4+)
        buf[0x41] = 0x10; // store -> global 0
        buf[0x42] = 0xBA; // quit
        buf[0x50] = 0xBA; // the elsewhere run's next instruction
        buf
    }

    #[test]
    fn host_restore_over_a_suspended_ingame_save_drops_the_abandoned_descriptor() {
        // SQ-0661: a host Save State restore performed while the game's own @save
        // is suspended REPLACES the run. The abandoned `pending_save` used to
        // survive, and `save_pc` prefers its descriptor address (Quetzal §5.8) —
        // so the NEXT host Save State recorded a PC belonging to the discarded
        // run, and restoring THAT resumed by decoding the @save's store byte as
        // an opcode.
        //
        // The bug is invisible at the moment of the restore (state.pc is already
        // correct); it surfaces on the next save. So: restore, then save again,
        // then restore that — and assert where we land.
        let mem = Memory::new(save_v4_with_an_elsewhere_story()).unwrap();
        let mut m = Machine::new(mem);

        // A snapshot of a DIFFERENT point in the story, taken with nothing pending.
        m.state.pc = 0x50;
        let elsewhere = m.save_quetzal();
        assert_eq!(crate::quetzal::saved_pc_of(&elsewhere), 0x50);

        // Now suspend on the game's own @save.
        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert!(m.is_saveload_pending(), "suspended inside the game's @save");
        assert_eq!(m.save_pc(), 0x41, "while suspended, a save records the descriptor");

        // The player instead loads a host Save State. The @save is abandoned.
        m.restore_file(&elsewhere).expect("host restore");
        assert_eq!(m.state.pc, 0x50, "we are running the restored state");

        // Perturb: take the NEXT host Save State — the one the stale descriptor
        // used to poison.
        let next = m.save_quetzal();
        assert_eq!(
            crate::quetzal::saved_pc_of(&next),
            0x50,
            "the next Save State records the restored run's PC, not the dead @save descriptor (0x41)"
        );

        // And restoring it lands on an instruction, not on the store byte.
        m.state.pc = 0x00AB;
        m.restore_file(&next).expect("second host restore");
        assert_eq!(m.state.pc, 0x50, "restoring it resumes where the player actually was");

        // The suspension is gone too, so the host's snapshot guard is not stuck on.
        assert!(
            !m.is_saveload_pending(),
            "the restore discarded the run the @save belonged to"
        );
    }

    #[test]
    fn host_restore_over_a_suspended_ingame_restore_drops_the_abandoned_store() {
        // The @restore twin of the test above: a stale `pending_restore_store`
        // would let a later `complete_restore_failure` store 0 into a variable of
        // the discarded run, and would keep `is_saveload_pending` stuck true.
        let mut buf = sample_story(4);
        buf[0x40] = 0xB6; // 0OP:0x06 restore (store form, v4+)
        buf[0x41] = 0x10; // store -> global 0
        buf[0x42] = 0xBA; // quit
        buf[0x50] = 0xBA;
        let mut m = Machine::new(Memory::new(buf).unwrap());

        m.state.pc = 0x50;
        let elsewhere = m.save_quetzal();

        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::RestoreRequest);
        assert!(m.is_saveload_pending(), "suspended inside the game's @restore");

        m.restore_file(&elsewhere).expect("host restore");

        // Perturb: a later failure completion must not write the dead run's G0.
        m.do_store(Some(0x10), 0x77);
        m.complete_restore_failure();
        assert_eq!(m.global(0), 0x77, "no store into the discarded run's variable");
        assert!(!m.is_saveload_pending(), "the abandoned @restore is gone");
    }

    #[test]
    fn is_saveload_pending_tracks_the_games_own_save_and_restore() {
        // The host guards its unconditional snapshot triggers (exit auto-save,
        // "Save State & quit") on this, so it must be true for the whole
        // suspension and false either side of it — in every version. (SQ-0661)

        // v4 @save, resolved by complete_save.
        let mut m = Machine::new(Memory::new(save_v4_with_an_elsewhere_story()).unwrap());
        assert!(!m.is_saveload_pending(), "nothing pending at boot");
        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert!(m.is_saveload_pending());
        m.complete_save(true);
        assert!(!m.is_saveload_pending(), "cleared once the host answers the @save");

        // v4 @restore, resolved by complete_restore_success.
        let mem = Memory::new(save_v4_with_an_elsewhere_story()).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal();
        m.complete_save(true);
        let mut buf = save_v4_with_an_elsewhere_story();
        buf[0x40] = 0xB6; // @restore -> G0 instead
        let mut m2 = Machine::new(Memory::new(buf).unwrap());
        m2.state.pc = 0x40;
        assert_eq!(m2.step(), StepResult::RestoreRequest);
        assert!(m2.is_saveload_pending());
        m2.complete_restore_success(&blob).expect("restore succeeds");
        assert!(!m2.is_saveload_pending(), "cleared once the restore completes");

        // v3 @restore is a BRANCH instruction with NO store byte, so
        // `pending_restore_store` stays None right through a real suspension —
        // this is the case a `pending_restore_store.is_some()` test would miss.
        let mut buf = sample_story(3);
        buf[0x40] = 0xB6;            // 0OP:0x06 restore (branch form in v3)
        buf[0x41] = 0x80 | 0x40 | 2; // branch: on-true, short form, offset 2
        buf[0x42] = 0xBA;            // quit
        let mut m3 = Machine::new(Memory::new(buf).unwrap());
        m3.state.pc = 0x40;
        assert_eq!(m3.step(), StepResult::RestoreRequest);
        assert!(m3.pending_restore_store.is_none(), "v3 @restore captures no store var");
        assert!(m3.is_saveload_pending(), "…but the v3 suspension is just as real");
        m3.complete_restore_failure();
        assert!(!m3.is_saveload_pending(), "cleared on the failure completion");

        // A @restart drops any suspension along with the run it belonged to.
        let mut m4 = Machine::new(Memory::new(save_v4_with_an_elsewhere_story()).unwrap());
        m4.state.pc = 0x40;
        assert_eq!(m4.step(), StepResult::SaveRequest);
        m4.restart();
        assert!(!m4.is_saveload_pending(), "a restart discards the suspension");
    }

    #[test]
    fn restore_file_clears_undo_stack() {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.undo_cap = 4;
        m.do_save_undo(Some(0x10));
        m.do_save_undo(Some(0x10));
        assert_eq!(m.undo_stack.len(), 2);
        // A file restore (using one of our own blobs as data) clears the undo stack.
        let blob = m.save_quetzal();
        m.restore_file(&blob).unwrap();
        assert!(m.undo_stack.is_empty(), "file restore invalidates and clears undo history");
    }

    #[test]
    fn restore_collapses_the_v3_upper_window_and_restamps_rst_fields() {
        // ZMSD §8.6.1.3 (Version 3 screen model): "Following a 'restore' of the
        // game, the interpreter should automatically collapse the upper window
        // to size 0." ZMSD §11.1: "'Rst' means that the interpreter must set it
        // correctly after loading the game, after a restore or after a restart."
        let mem = Memory::new(sample_story(3)).unwrap();
        let mut m = Machine::new(mem);
        m.init_caps();

        // Save with a header that claims we cannot draw a status line — a save
        // written by some other interpreter, as far as this one is concerned.
        m.mem.write_byte(0x01, m.mem.read_byte(0x01) | (1 << 4));
        let blob = m.save_quetzal();

        m.init_caps(); // (clears it again locally)
        m.exec_var(0x0A, &[4], None, None); // split_window 4
        m.exec_var(0x0B, &[1], None, None); // set_window 1
        assert_eq!(m.screen.upper_window_rows, 4);

        m.restore_file(&blob).unwrap();

        assert_eq!(m.screen.upper_window_rows, 0, "upper window collapsed to size 0");
        assert_eq!(m.screen.current_window, 0, "back in the lower window");
        assert_eq!(
            m.mem.read_byte(0x01) & (1 << 4),
            0,
            "Rst: Flags 1 capability bits re-stamped over the restored header"
        );
    }

    #[test]
    fn restore_leaves_the_v5_upper_window_to_the_game() {
        // §8.6.1.3 sits under §8.6 "The screen model for Version 3"; from v4 on
        // the game owns its upper window across a restore (Frotz scopes its
        // collapse the same way). The Rst header re-stamp still happens.
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.init_caps();
        m.set_screen_dims(12, 40); // the save was written on a small screen
        let blob = m.save_quetzal();
        m.set_screen_dims(30, 70); // the host is showing a bigger one now
        m.exec_var(0x0A, &[4], None, None); // split_window 4

        m.restore_file(&blob).unwrap();

        assert_eq!(m.screen.upper_window_rows, 4, "v5 upper window survives the restore");
        assert_eq!(
            (m.mem.read_byte(0x20), m.mem.read_byte(0x21)),
            (30, 70),
            "Rst: the HOST's screen size wins over the one baked into the save"
        );
    }

    #[test]
    fn undo_empty_and_disabled_and_cap() {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);

        // Empty stack: restore_undo stores 0 into its own target, no state change.
        m.undo_cap = 4;
        m.do_restore_undo(Some(0x10));
        assert_eq!(m.global(0), 0);

        // Disabled (cap 0): save_undo stores -1 (0xFFFF) and pushes nothing.
        m.undo_cap = 0;
        m.do_save_undo(Some(0x11));
        assert_eq!(m.global(1), 0xFFFF, "cap 0 => -1 (unsupported)");
        assert!(m.undo_stack.is_empty());

        // Cap drop: with cap 2, three saves keep the newest two.
        m.undo_cap = 2;
        m.do_save_undo(Some(0x10));
        m.do_save_undo(Some(0x10));
        m.do_save_undo(Some(0x10));
        assert_eq!(m.undo_stack.len(), 2, "oldest dropped past the cap");
    }

    // -----------------------------------------------------------------------
    // Tiny assembler for test programs
    //
    // `Asm` describes Z-machine instructions; `assemble()` emits bytes.
    // Targets v5 (no local-initial-value words in routine headers).
    // Designed to be extended by Tasks 10–13 executor tests.
    //
    // Usage:
    //   let mut m = build_test_machine(&[Asm::Add(C(2), C(3), DG(0)), Asm::Quit]);
    //   run_until_quit(&mut m);
    //   assert_eq!(m.global(0), 5);
    //
    // Supported forms: Long-form 2OP (small/var operands), Short-form 1OP,
    // Variable-form call_vs (large packed addr + small/var args), 0OP.
    // For Large-operand instructions, write raw bytes directly (see jump_negative_offset test).
    // -----------------------------------------------------------------------

    /// An operand value for test assembly.
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    pub(crate) enum Op {
        /// Small constant (0..=255).
        Const(u8),
        /// Global variable reference (var number 0x10 + n).
        Global(u8),
        /// Local variable reference (var number n, 1-based).
        Local(u8),
    }

    pub(crate) use Op::Const as C;
    pub(crate) use Op::Global as G;

    /// Store destination for test assembly.
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    pub(crate) enum Dest {
        /// Store into global variable N (var number = 0x10 + N).
        Global(u8),
        /// Store into local variable N (var number = N).
        Local(u8),
    }

    pub(crate) use Dest::Global as DG;

    /// Assembler instructions (subset used by Tasks 9–13 executor tests).
    #[allow(dead_code)]
    pub(crate) enum Asm {
        /// add a, b -> dest  (signed)
        Add(Op, Op, Dest),
        /// mul a, b -> dest  (signed)
        Mul(Op, Op, Dest),
        /// sub a, b -> dest  (signed)
        Sub(Op, Op, Dest),
        /// je a, b — branch if a == b, taken branch skips next instruction (offset=2)
        JeTrue(Op, Op),
        /// je a, b — fall through if a == b; branch (skip) if a != b
        JeNot(Op, Op),
        /// jz a — branch if a == 0, taken → skip next instruction (offset=2)
        JzTrue(Op),
        /// jump offset (signed i16; applied as: pc = pc + offset - 2)
        Jump(i16),
        /// inc_chk var_num, val — increment var, branch (skip next) if new_val > val
        IncChk(u8, Op),
        /// dec_chk var_num, val — decrement var, branch (skip next) if new_val < val
        DecChk(u8, Op),
        /// call_vs packed_addr, args, dest — VAR:0x00 with Large packed addr
        CallVs(u16, Vec<Op>, Dest),
        /// ret val — return value from current routine
        Ret(Op),
        /// rtrue — return 1
        Rtrue,
        /// rfalse — return 0
        Rfalse,
        /// quit — halt interpreter
        Quit,
        /// nop — no operation
        Nop,
        /// push val — push onto eval stack
        Push(Op),
    }

    /// Operand type code used in Variable-form type bytes (2-bit ZMSD encoding):
    ///   0b01 = small constant, 0b10 = variable reference.
    fn op_type(op: Op) -> u8 {
        match op {
            Op::Const(_) => 0b01,
            Op::Global(_) | Op::Local(_) => 0b10,
        }
    }

    /// Long-form operand bit: 0 = small constant, 1 = variable reference.
    /// (Long form encodes operand types as single bits, not 2-bit type codes.)
    fn op_long_bit(op: Op) -> u8 {
        match op {
            Op::Const(_) => 0,
            Op::Global(_) | Op::Local(_) => 1,
        }
    }

    /// The byte value to emit for an operand (constant value or variable number).
    fn op_byte(op: Op) -> u8 {
        match op {
            Op::Const(v) => v,
            Op::Global(n) => 0x10 + n,
            Op::Local(n) => n,
        }
    }

    fn dest_var(d: Dest) -> u8 {
        match d {
            Dest::Global(n) => 0x10 + n,
            Dest::Local(n) => n,
        }
    }

    /// Emit a long-form 2OP instruction.
    /// Bits 6 and 5 of the opcode byte encode operand types (0=small const, 1=variable).
    fn emit_long2op(out: &mut Vec<u8>, opcode: u8, a: Op, b: Op, store: Option<u8>, branch: Option<(bool, i16)>) {
        let t1 = op_long_bit(a); // 0=small const, 1=variable
        let t2 = op_long_bit(b);
        let ob = (t1 << 6) | (t2 << 5) | (opcode & 0x1F);
        out.push(ob);
        out.push(op_byte(a));
        out.push(op_byte(b));
        if let Some(sv) = store { out.push(sv); }
        if let Some((on_true, offset)) = branch { emit_branch(out, on_true, offset); }
    }

    /// Emit branch data (single-byte for 0..=63, two-byte otherwise).
    fn emit_branch(out: &mut Vec<u8>, on_true: bool, offset: i16) {
        if (0..=63).contains(&offset) {
            // Single-byte: bit7=on_true, bit6=1 (short form), bits5-0=offset
            out.push(if on_true { 0x80 } else { 0x00 } | 0x40 | (offset as u8 & 0x3F));
        } else {
            // Two-byte: 14-bit signed (bits 13..0 of raw, sign-extended)
            let raw = (offset as u16) & 0x3FFF;
            let high6 = ((raw >> 8) & 0x3F) as u8;
            let low8 = (raw & 0xFF) as u8;
            out.push(if on_true { 0x80 } else { 0x00 } | high6);
            out.push(low8);
        }
    }

    /// Emit VAR-form type byte and operand bytes (up to 4 operands).
    fn emit_var_ops(out: &mut Vec<u8>, ops: &[Op]) {
        // Type byte: MSB pair = first operand type; 0b11 = omitted
        let mut type_byte: u8 = 0xFF;
        for (i, op) in ops.iter().enumerate().take(4) {
            let t = op_type(*op);
            let shift = 6u8.saturating_sub(2 * i as u8);
            type_byte &= !(0b11 << shift);
            type_byte |= (t & 0b11) << shift;
        }
        out.push(type_byte);
        for op in ops.iter().take(4) { out.push(op_byte(*op)); }
    }

    /// Assemble `Asm` instructions into a byte vector.
    pub(crate) fn assemble(instrs: &[Asm]) -> Vec<u8> {
        let mut out = Vec::new();
        for instr in instrs {
            match instr {
                Asm::Add(a, b, d) => emit_long2op(&mut out, 0x14, *a, *b, Some(dest_var(*d)), None),
                Asm::Mul(a, b, d) => emit_long2op(&mut out, 0x16, *a, *b, Some(dest_var(*d)), None),
                Asm::Sub(a, b, d) => emit_long2op(&mut out, 0x15, *a, *b, Some(dest_var(*d)), None),
                Asm::JeTrue(a, b) => emit_long2op(&mut out, 0x01, *a, *b, None, Some((true, 2))),
                Asm::JeNot(a, b)  => emit_long2op(&mut out, 0x01, *a, *b, None, Some((false, 2))),
                Asm::JzTrue(a) => {
                    // Short form 1OP jz (opcode=0) with small constant: 0x90
                    out.push(0x90);
                    out.push(op_byte(*a));
                    emit_branch(&mut out, true, 2);
                }
                Asm::Jump(offset) => {
                    // Short form 1OP jump (0x0C) with large constant: 0x8C + 2 bytes
                    out.push(0x8C);
                    let v = *offset as u16;
                    out.push((v >> 8) as u8);
                    out.push((v & 0xFF) as u8);
                }
                Asm::IncChk(var, b) => {
                    // Long form 2OP inc_chk (0x05), var number as small const, branch taken=skip
                    emit_long2op(&mut out, 0x05, Op::Const(*var), *b, None, Some((true, 2)));
                }
                Asm::DecChk(var, b) => {
                    // Long form 2OP dec_chk (0x04), var number as small const, branch taken=skip
                    emit_long2op(&mut out, 0x04, Op::Const(*var), *b, None, Some((true, 2)));
                }
                Asm::CallVs(packed, args, d) => {
                    // VAR form call_vs (0x00) with Large first operand (packed addr)
                    out.push(0xE0); // 11 1 00000 = VAR class, opcode 0
                    // Type byte: first = large const (0b00), rest from args
                    let mut type_byte: u8 = 0xFF;
                    type_byte &= !(0b11 << 6); // first = large (0b00)
                    for (i, arg) in args.iter().enumerate().take(3) {
                        let t = op_type(*arg);
                        let shift = 4u8.saturating_sub(2 * i as u8);
                        type_byte &= !(0b11 << shift);
                        type_byte |= (t & 0b11) << shift;
                    }
                    out.push(type_byte);
                    out.push((*packed >> 8) as u8);
                    out.push((*packed & 0xFF) as u8);
                    for arg in args.iter().take(3) { out.push(op_byte(*arg)); }
                    out.push(dest_var(*d));
                }
                Asm::Ret(a) => {
                    // Short form 1OP ret (opcode=0x0B) with small constant: 0x9B
                    out.push(0x9B);
                    out.push(op_byte(*a));
                }
                Asm::Rtrue  => out.push(0xB0),
                Asm::Rfalse => out.push(0xB1),
                Asm::Quit   => out.push(0xBA),
                Asm::Nop    => out.push(0xB4),
                Asm::Push(a) => {
                    // VAR form push (0xE8): 11 1 01000
                    out.push(0xE8);
                    emit_var_ops(&mut out, &[*a]);
                }
            }
        }
        out
    }

    /// Build a `Machine` with `instrs` placed at 0x10 in a v5 story.
    /// Overrides `state.pc` to 0x10 (the header's initial_pc is 0x40).
    pub(crate) fn build_test_machine(instrs: &[Asm]) -> Machine {
        let bytes = assemble(instrs);
        let mut buf = sample_story(5);
        for (i, &b) in bytes.iter().enumerate() {
            buf[0x10 + i] = b;
        }
        let mem = Memory::new(buf).unwrap();
        let mut machine = Machine::new(mem);
        machine.state.pc = 0x10;
        machine
    }

    /// Run `step()` until `Quit` (safety limit: 10 000 steps).
    pub(crate) fn run_until_quit(machine: &mut Machine) -> u32 {
        for i in 0..10_000u32 {
            if matches!(machine.step(), StepResult::Quit) {
                return i + 1;
            }
        }
        panic!("step limit exceeded without Quit");
    }

    // -----------------------------------------------------------------------
    // Test (a): (2 + 3) * 4 = 20 stored in global 0
    // -----------------------------------------------------------------------

    #[test]
    fn executes_add_mul_into_global() {
        let mut m = build_test_machine(&[
            Asm::Add(C(2), C(3), DG(0)),  // G0 = 2 + 3 = 5
            Asm::Mul(G(0), C(4), DG(0)),  // G0 = G0 * 4 = 20
            Asm::Quit,
        ]);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 20);
    }

    // -----------------------------------------------------------------------
    // Test (b): je branch taken vs not taken
    //
    // ZMSD §4.7: branch offset is relative to the instruction AFTER the branch
    // bytes (= next_pc). offset=2 → no movement (fall-through). To skip an
    // N-byte instruction, use offset = N + 2.
    //
    // We hand-assemble to control the exact offsets.
    // -----------------------------------------------------------------------

    #[test]
    fn je_branch_taken_and_not_taken() {
        // Layout at 0x10 (hand-assembled bytes):
        //
        //   [TAKEN path]
        //   0x10: je 5, 5  (long form, both small const, opcode=0x01, branch)
        //         Long-form opcode: bit6=t1(small=0), bit5=t2(small=0), opcode=0x01 → 0x01
        //         bytes: 0x01, 0x05, 0x05, branch_byte
        //         branch: on_true=1, skip Add(1,0,G0) which is 4 bytes → offset = 4+2 = 6
        //         branch_byte (single): 0x80 | 0x40 | 6 = 0xC6
        //         → 4 bytes total (0x10–0x13), next_pc = 0x14
        //         branch taken: pc = 0x14 + 6 - 2 = 0x18 (skips Add)
        //   0x14: add 1, 0 → G0  (4 bytes, skipped when branch taken)
        //   0x18: add 0, 7 → G0  (4 bytes: 0x14→G0=1; then 0x18→G0=7)
        //   0x1C: quit (1 byte)

        let mut buf = sample_story(5);
        // 0x10: je 5, 5: opcode=0x01, both small (bits6=0,5=0), branch on_true offset=6
        buf[0x10] = 0x01; // je, small+small
        buf[0x11] = 5;    // a=5
        buf[0x12] = 5;    // b=5
        buf[0x13] = 0xC6; // branch: on_true=1, short form, offset=6
        // 0x14: add 1, 0 → G0 (long form, small+small: opcode=0x14, bit6=0, bit5=0)
        buf[0x14] = 0x14;
        buf[0x15] = 1;
        buf[0x16] = 0;
        buf[0x17] = 0x10; // store → G0
        // 0x18: add 0, 7 → G0
        buf[0x18] = 0x14;
        buf[0x19] = 0;
        buf[0x1A] = 7;
        buf[0x1B] = 0x10;
        // 0x1C: quit
        buf[0x1C] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 7, "je taken: Add(1,0) skipped, G0=7 from Add(0,7)");

        // NOT taken: je 5, 6 → falls through to Add(1,0,G0) → G0=1, then quit
        let mut buf2 = sample_story(5);
        buf2[0x10] = 0x01; // je, small+small
        buf2[0x11] = 5;
        buf2[0x12] = 6;    // b=6 ≠ a, branch NOT taken
        buf2[0x13] = 0xC6; // branch: on_true, offset=6 (irrelevant — branch not taken)
        // fall-through → Add(1,0→G0)
        buf2[0x14] = 0x14;
        buf2[0x15] = 1;
        buf2[0x16] = 0;
        buf2[0x17] = 0x10;
        // quit
        buf2[0x18] = 0xBA;

        let mem2 = Memory::new(buf2).unwrap();
        let mut m2 = Machine::new(mem2);
        m2.state.pc = 0x10;
        run_until_quit(&mut m2);
        assert_eq!(m2.global(0), 1, "je not taken: falls through, G0=1");
    }

    // -----------------------------------------------------------------------
    // Test (c): call/return — routine returns 42 into global 0
    // -----------------------------------------------------------------------

    #[test]
    fn call_and_return_value() {
        // Routine at byte 0x80 (v5 packed addr = 0x80 / 4 = 0x20).
        // Routine header: 0 locals (1 byte), then: ret 42 (2 bytes).
        // Main at 0x10: call_vs packed=0x0020 → G0; quit.
        let mut buf = sample_story(5);

        // Routine header + body at 0x80
        buf[0x80] = 0;    // local count = 0 (v5)
        buf[0x81] = 0x9B; // ret, small const
        buf[0x82] = 42;

        // Main: call_vs 0x0020 → G0 (0x10); quit
        buf[0x10] = 0xE0;               // call_vs (VAR:0x00)
        buf[0x11] = 0b00_11_11_11;      // type byte: large, omit, omit, omit
        buf[0x12] = 0x00;               // packed addr high = 0x0020
        buf[0x13] = 0x20;
        buf[0x14] = 0x10;               // store → global 0 (var 0x10)
        buf[0x15] = 0xBA;               // quit

        let mem = Memory::new(buf).unwrap();
        let mut machine = Machine::new(mem);
        machine.state.pc = 0x10;
        run_until_quit(&mut machine);
        assert_eq!(machine.global(0), 42);
    }

    // -----------------------------------------------------------------------
    // Test (d): jump with negative offset loops correctly
    // -----------------------------------------------------------------------

    #[test]
    fn jump_negative_offset() {
        // Program: increment G0 each iteration, exit when G0 > 3.
        //
        // Byte layout (hand-assembled):
        //   0x10: add G0,1 → G0   (long form, var+small) — 4 bytes, next_pc=0x14
        //   0x14: jg G0,3         (long form, var+small, + branch byte) — 4 bytes, next_pc=0x18
        //         branch on_true, offset=5: target = 0x18 + 5 - 2 = 0x1B (skip 3-byte jump)
        //   0x18: jump -9         (short 1OP large const) — 3 bytes, next_pc=0x1B
        //         offset = 0x10 - 0x1B + 2 = -9 → pc = 0x1B + (-9) - 2 = 0x10 ✓
        //   0x1B: quit

        let mut buf = sample_story(5);

        // 0x10: add G0(var=0x10), 1 → G0; long form var+small
        // t1=var(bit6=1), t2=small(bit5=0), opcode=0x14 → 0b0_1_0_10100 = 0x54
        buf[0x10] = 0x54; // add, var+small
        buf[0x11] = 0x10; // G0
        buf[0x12] = 1;
        buf[0x13] = 0x10; // store → G0

        // 0x14: jg G0, 3; long form var+small, opcode=0x03
        // t1=var(bit6=1), t2=small(bit5=0), opcode=0x03 → 0b0_1_0_00011 = 0x43
        buf[0x14] = 0x43; // jg, var+small
        buf[0x15] = 0x10; // G0
        buf[0x16] = 3;    // const 3
        // branch: on_true=1, single-byte, offset=5 → 0x80|0x40|5 = 0xC5
        // next_pc of jg = 0x18; branch target = 0x18 + 5 - 2 = 0x1B (past the jump) ✓
        buf[0x17] = 0xC5;

        // 0x18: jump -9 (large const); next_pc = 0x1B
        // offset = target(0x10) - next_pc(0x1B) + 2 = 0x10 - 0x1B + 2 = -9
        buf[0x18] = 0x8C;
        let jmp_off: i16 = 0x10i16 - 0x1Bi16 + 2; // = -9
        buf[0x19] = (jmp_off as u16 >> 8) as u8;
        buf[0x1A] = (jmp_off as u16 & 0xFF) as u8;

        // 0x1B: quit
        buf[0x1B] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut machine = Machine::new(mem);
        machine.state.pc = 0x10;
        run_until_quit(&mut machine);
        // Iterations: G0: 0→1(jg F), 1→2(jg F), 2→3(jg F), 3→4(jg 4>3=T→branch→quit)
        assert_eq!(machine.global(0), 4);
    }

    // -----------------------------------------------------------------------
    // Test (e): inc_chk and dec_chk branch behavior
    //
    // We hand-assemble to control branch offsets precisely.
    // ZMSD §4.7: branch offset relative to next_pc; offset N → target = next_pc + N - 2.
    // To skip a 4-byte instruction (long-form add with store): offset = 4 + 2 = 6.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Test: load sp peeks without popping (ZMSD §6.3.4)
    // -----------------------------------------------------------------------

    #[test]
    fn load_sp_peeks_not_pops() {
        // Layout at 0x10:
        //   push 0xAAAA              (VAR:0x08, 3 bytes: E8 type_byte val)
        //   push 0xBEEF              (VAR:0x08, 3 bytes)
        //   load sp -> G0            (1OP:0x0E, small const 0x00, store G0)
        //     Short form 1OP with small const: 0x9E, operand=0x00, store=0x10
        //     3 bytes total
        //   quit
        //
        // After load: G0 == 0xBEEF (top value), stack depth still 2.
        let mut buf = sample_story(5);
        let mut pos = 0x10usize;

        // push 0xAAAA: 0xE8 type_byte(large=0b00...) value_hi value_lo
        // VAR:push uses emit_var_ops but for a large constant we need 2 bytes.
        // Actually looking at the Asm::Push handler: it uses emit_var_ops which emits
        // a 1-byte type byte + 1-byte operand (small const). For a 16-bit value we
        // need a different approach. Use raw bytes: write directly.
        // push 0xBEEF needs a large constant. Emit as VAR:0x08 with large operand:
        //   0xE8 (VAR push), type byte: large=0b00 for first op → 0b00_11_11_11 = 0x3F
        //   then 2-byte value: hi, lo
        buf[pos] = 0xE8; pos += 1;   // VAR push
        buf[pos] = 0x3F; pos += 1;   // type: first=large(0b00), rest=omit(0b11)
        buf[pos] = 0xAA; pos += 1;   // 0xAAAA hi
        buf[pos] = 0xAA; pos += 1;   // 0xAAAA lo

        buf[pos] = 0xE8; pos += 1;   // VAR push
        buf[pos] = 0x3F; pos += 1;
        buf[pos] = 0xBE; pos += 1;   // 0xBEEF hi
        buf[pos] = 0xEF; pos += 1;   // 0xBEEF lo

        // load sp (var 0) -> G0: short 1OP small const, opcode=0x0E → 0x9E
        buf[pos] = 0x9E; pos += 1;   // load, small const
        buf[pos] = 0x00; pos += 1;   // operand = variable number 0 (sp)
        buf[pos] = 0x10; pos += 1;   // store -> G0 (var 0x10)

        buf[pos] = 0xBA;              // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        assert_eq!(m.global(0), 0xBEEF, "load sp: G0 should be top of stack (0xBEEF)");
        assert_eq!(m.state.eval_stack.len(), 2, "load sp: stack depth must be unchanged (peek, not pop)");
        assert_eq!(m.state.eval_stack[1], 0xBEEF, "load sp: top value still on stack");
    }

    // -----------------------------------------------------------------------
    // Test: store sp replaces top without pushing (ZMSD §6.3.4)
    // -----------------------------------------------------------------------

    #[test]
    fn store_sp_replaces_top() {
        // Layout at 0x10:
        //   push 0x1234              (VAR:push, large const)
        //   store sp, 0x56           (2OP:0x0D, a=small const 0x00, b=small const 0x56)
        //     Long form small+small: 0x0D, a=0x00, b=0x56
        //     3 bytes total
        //   quit
        //
        // After store: stack depth still 1, top == 0x0056.
        let mut buf = sample_story(5);
        let mut pos = 0x10usize;

        // push 0x1234 (large const)
        buf[pos] = 0xE8; pos += 1;
        buf[pos] = 0x3F; pos += 1;
        buf[pos] = 0x12; pos += 1;
        buf[pos] = 0x34; pos += 1;

        // store sp, 0x56: 2OP:0x0D long form, both small const
        // Long-form opcode: t1=small(0), t2=small(0), opcode=0x0D → 0x0D
        buf[pos] = 0x0D; pos += 1;   // store, small+small
        buf[pos] = 0x00; pos += 1;   // a = variable number 0 (sp)
        buf[pos] = 0x56; pos += 1;   // b = new value 0x56

        buf[pos] = 0xBA;              // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        assert_eq!(m.state.eval_stack.len(), 1, "store sp: stack depth must be unchanged (replace, not push)");
        assert_eq!(m.state.eval_stack[0], 0x0056, "store sp: top value must be new value");
    }

    // -----------------------------------------------------------------------
    // Test: emit_branch two-byte form encodes correctly (ZMSD §4.7)
    // -----------------------------------------------------------------------

    #[test]
    fn emit_branch_two_byte_form() {
        // ZMSD §4.7: when |offset| >= 64, branch is two bytes.
        // Byte 0: bit7=on_true, bit6=0 (long form), bits5-0 = high 6 bits of 14-bit offset
        // Byte 1: low 8 bits of 14-bit offset
        // Test with offset=100 (>= 64), on_true=true.
        let mut out = Vec::new();
        emit_branch(&mut out, true, 100);
        assert_eq!(out.len(), 2, "long branch must emit exactly 2 bytes");

        // offset=100 = 0x0064; 14-bit raw = 0x0064
        // high6 = (0x0064 >> 8) & 0x3F = 0x00
        // low8  = 0x0064 & 0xFF        = 0x64
        // byte0 = on_true(1<<7) | high6 = 0x80 | 0x00 = 0x80
        // byte1 = 0x64
        assert_eq!(out[0], 0x80, "byte0: on_true bit set, bit6 clear, high6=0");
        assert_eq!(out[1], 0x64, "byte1: low8 of offset 100");

        // Also verify: on_true=false with offset=200 (0x00C8)
        // high6 = (0x00C8 >> 8) & 0x3F = 0x00
        // low8  = 0x00C8 & 0xFF        = 0xC8
        // byte0 = 0x00 | 0x00 = 0x00 (bit7=0, bit6=0)
        let mut out2 = Vec::new();
        emit_branch(&mut out2, false, 200);
        assert_eq!(out2[0], 0x00, "byte0: on_true=false, high6=0");
        assert_eq!(out2[1], 0xC8, "byte1: low8 of offset 200");

        // And offset=64 (boundary: just over single-byte limit)
        // high6 = 0x00, low8 = 0x40
        // byte0 = 0x80 (on_true=true)
        let mut out3 = Vec::new();
        emit_branch(&mut out3, true, 64);
        assert_eq!(out3.len(), 2, "offset=64 uses two-byte form");
        assert_eq!(out3[0], 0x80);
        assert_eq!(out3[1], 0x40);
    }

    #[test]
    fn inc_chk_and_dec_chk() {
        // inc_chk test:
        //   0x10: inc_chk 0x10, 0  (dec_chk: opcode=0x05, both small const, branch)
        //         Long-form: opcode=0x05 (inc_chk), t1=small(0), t2=small(0) → 0x05
        //         operand 1 = 0x10 (var number for G0), operand 2 = 0 (threshold)
        //         branch: on_true, offset=6 to skip 4-byte Add: 0x80|0x40|6 = 0xC6
        //         next_pc = 0x14; branch target = 0x14 + 6 - 2 = 0x18
        //   0x14: add 99, 0 → G0  (4 bytes, 0x14–0x17, skipped when branch taken)
        //   0x18: quit
        let mut buf = sample_story(5);
        // inc_chk 0x10, 0: opcode=0x05, both small
        buf[0x10] = 0x05; // inc_chk, small+small
        buf[0x11] = 0x10; // var number = global 0
        buf[0x12] = 0;    // threshold = 0
        buf[0x13] = 0xC6; // branch on_true, short form, offset=6
        // add 99, 0 → G0 (long form, small+small)
        buf[0x14] = 0x14;
        buf[0x15] = 99;
        buf[0x16] = 0;
        buf[0x17] = 0x10; // store → G0
        // quit
        buf[0x18] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 1, "inc_chk: G0 should be 1 (Add(99,0) skipped)");

        // dec_chk test:
        //   0x10: dec_chk 0x10, 0  (opcode=0x04, both small const)
        //         G0 starts at 0 → decrements to 0xFFFF (-1 signed)
        //         -1 < 0 → branch taken, offset=6 → skip Add → quit
        //   0x14: add 99, 0 → G0  (skipped)
        //   0x18: quit
        let mut buf2 = sample_story(5);
        buf2[0x10] = 0x04; // dec_chk, small+small
        buf2[0x11] = 0x10; // var number = G0
        buf2[0x12] = 0;    // threshold = 0
        buf2[0x13] = 0xC6; // branch on_true, short, offset=6
        buf2[0x14] = 0x14; // add
        buf2[0x15] = 99;
        buf2[0x16] = 0;
        buf2[0x17] = 0x10;
        buf2[0x18] = 0xBA; // quit

        let mem2 = Memory::new(buf2).unwrap();
        let mut m2 = Machine::new(mem2);
        m2.state.pc = 0x10;
        run_until_quit(&mut m2);
        assert_eq!(m2.global(0), 0xFFFF, "dec_chk: G0 should be 0xFFFF (-1 as u16)");
    }

    // -----------------------------------------------------------------------
    // Object / property opcode tests (Task 10)
    //
    // Object table layout (v3, sample_story(3)):
    //   object_table = 0x0100 (set by sample_story)
    //   v3 property-defaults: 31 words = 62 bytes → entries at 0x013E
    //   Each v3 entry = 9 bytes: [0..3] attrs, [4] parent, [5] sibling, [6] child, [7..8] prop_tbl
    //
    //   obj1 at 0x013E: parent=0, sibling=0, child=2, attr0 set, prop_tbl=0x0200
    //   obj2 at 0x0147: parent=1, sibling=3, child=0, attr7+8 set, prop_tbl=0x0220
    //   obj3 at 0x0150: parent=1, sibling=0, child=0, prop_tbl=0x0230
    //
    // Property table for obj1 (at 0x0200):
    //   name: 0 words (empty)
    //   prop 10: 2 bytes 0xABCD → size byte 0x2A, data 0xABCD
    //   prop 5:  1 byte  0x42  → size byte 0x05, data 0x42
    //   sentinel 0x00
    //
    // Property table for obj2/obj3: name 0 words, sentinel 0x00 only.
    //
    // Test programs are placed at 0x10 (pc=0x10) in v3 story buffers.
    // -----------------------------------------------------------------------

    /// Build a v3 story buffer with a small 3-object tree for executor tests.
    fn build_obj_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        const OBJ_TABLE: usize = 0x0100;
        const ENTRIES: usize   = OBJ_TABLE + 31 * 2; // 0x013E
        const OBJ1: usize      = ENTRIES;             // 0x013E
        const OBJ2: usize      = ENTRIES + 9;         // 0x0147
        const OBJ3: usize      = ENTRIES + 18;        // 0x0150

        const PROP1: u16 = 0x0200;
        const PROP2: u16 = 0x0220;
        const PROP3: u16 = 0x0230;

        // obj1: attr0 set, parent=0, sibling=0, child=2
        buf[OBJ1]   = 0x80; // attr0
        buf[OBJ1+1] = 0; buf[OBJ1+2] = 0; buf[OBJ1+3] = 0;
        buf[OBJ1+4] = 0; // parent
        buf[OBJ1+5] = 0; // sibling
        buf[OBJ1+6] = 2; // child
        buf[OBJ1+7] = (PROP1 >> 8) as u8; buf[OBJ1+8] = (PROP1 & 0xFF) as u8;

        // obj2: attr7+attr8 set, parent=1, sibling=3, child=0
        buf[OBJ2]   = 0x01; // attr7
        buf[OBJ2+1] = 0x80; // attr8
        buf[OBJ2+2] = 0; buf[OBJ2+3] = 0;
        buf[OBJ2+4] = 1; // parent
        buf[OBJ2+5] = 3; // sibling
        buf[OBJ2+6] = 0; // child
        buf[OBJ2+7] = (PROP2 >> 8) as u8; buf[OBJ2+8] = (PROP2 & 0xFF) as u8;

        // obj3: no attrs, parent=1, sibling=0, child=0
        buf[OBJ3]   = 0; buf[OBJ3+1] = 0; buf[OBJ3+2] = 0; buf[OBJ3+3] = 0;
        buf[OBJ3+4] = 1; // parent
        buf[OBJ3+5] = 0; // sibling
        buf[OBJ3+6] = 0; // child
        buf[OBJ3+7] = (PROP3 >> 8) as u8; buf[OBJ3+8] = (PROP3 & 0xFF) as u8;

        // prop table obj1: name=0 words, prop10(2B)=0xABCD, prop5(1B)=0x42, sentinel
        let p1 = PROP1 as usize;
        buf[p1]   = 0;    // 0 name words
        buf[p1+1] = 0x2A; // size: (2-1)<<5 | 10 = 0b001_01010
        buf[p1+2] = 0xAB; buf[p1+3] = 0xCD;
        buf[p1+4] = 0x05; // size: (1-1)<<5 | 5  = 0b000_00101
        buf[p1+5] = 0x42;
        buf[p1+6] = 0x00; // sentinel

        // prop table obj2: name=0 words, no props
        let p2 = PROP2 as usize;
        buf[p2] = 0; buf[p2+1] = 0x00; // sentinel

        // prop table obj3: name=0 words, no props
        let p3 = PROP3 as usize;
        buf[p3] = 0; buf[p3+1] = 0x00; // sentinel

        // property default for prop10 = 0x5678
        let def10 = OBJ_TABLE + (10 - 1) * 2;
        buf[def10]   = 0x56;
        buf[def10+1] = 0x78;

        buf
    }

    /// Build a `Machine` from a pre-built story buffer, program at 0x10.
    fn build_obj_machine_raw(buf: Vec<u8>, prog: &[u8]) -> Machine {
        let mut buf = buf;
        for (i, &b) in prog.iter().enumerate() {
            buf[0x10 + i] = b;
        }
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m
    }

    // -----------------------------------------------------------------------
    // (a) jin — branch if parent(obj1)==obj2
    // test_attr — branch if attribute set
    // -----------------------------------------------------------------------

    #[test]
    fn obj_jin_branch_taken() {
        // jin obj2, obj1 → branch taken (parent(2)==1)
        // Long-form 2OP 0x06, both small const: opcode byte = 0x06
        // branch taken → skip 1 byte nop, reaching quit
        // Layout: [jin obj2,obj1 + branch(taken, offset=3)][nop][add 99,0→G0][quit]
        //
        // jin (3 + 1 branch byte = 4 bytes, next_pc=0x14)
        // branch: on_true=1, offset=5 → skip 4-byte add, land on quit
        // nop is 1 byte, add is 4 bytes: to skip both → offset = 5 + 2 = 7?
        // Actually place nop+add at 0x14..0x18, quit at 0x19.
        // From next_pc=0x14: to skip to quit at 0x19 → offset = 0x19-0x14+2 = 7.
        // branch byte: on_true, short, offset=7 → 0x80|0x40|7 = 0xC7
        //
        // After branch skips to quit without running add, G0 stays 0.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x06, 2, 1, 0xC7, // jin obj2,obj1 → branch on_true, offset=7 → skip to quit
            0xB4,              // nop (1 byte)
            0x14, 99, 0, 0x10, // add 99,0 → G0 (4 bytes, skipped)
            0xBA,              // quit
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0, "jin taken: add skipped, G0 remains 0");
    }

    #[test]
    fn obj_jin_branch_not_taken() {
        // jin obj1, obj2 → NOT taken (parent(1)==0, not 2)
        // G0 gets set to 99.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x06, 1, 2, 0xC7, // jin obj1,obj2 → branch on_true, offset=7 (but not taken)
            0xB4,              // nop
            0x14, 99, 0, 0x10, // add 99,0 → G0
            0xBA,              // quit
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 99, "jin not taken: falls through, G0=99");
    }

    #[test]
    fn obj_test_attr_branch_taken() {
        // test_attr obj1, attr0 → taken (attr0 is set on obj1)
        // Long-form 2OP 0x0A, both small: opcode byte = 0x0A
        // branch taken → skip 4-byte add, G0 stays 0
        // next_pc=0x14 after [0x0A,1,0,branch_byte]
        // to skip 4-byte add at 0x14 and land at 0x18 (quit): offset=0x18-0x14+2=6
        // branch: on_true=1, short, offset=6 → 0xC6
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0A, 1, 0, 0xC6, // test_attr obj1,attr0 → branch taken, offset=6
            0x14, 99, 0, 0x10, // add 99,0 → G0 (skipped)
            0xBA,              // quit
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0, "test_attr taken: add skipped");
    }

    #[test]
    fn obj_test_attr_branch_not_taken() {
        // test_attr obj1, attr1 → NOT taken (attr1 is clear)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0A, 1, 1, 0xC6, // test_attr obj1,attr1 → not taken
            0x14, 99, 0, 0x10, // add 99,0 → G0 (runs)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 99, "test_attr not taken: G0=99");
    }

    // -----------------------------------------------------------------------
    // (b) set_attr / clear_attr — verify via get_attr after step()
    // -----------------------------------------------------------------------

    #[test]
    fn obj_set_attr_and_clear_attr() {
        // set_attr obj1, attr3 → then clear_attr obj1, attr3
        // Long-form 2OP: set_attr=0x0B, clear_attr=0x0C
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0B, 1, 3, // set_attr obj1, attr3 (3 bytes)
            0xBA,        // quit
        ];
        let mut m = build_obj_machine_raw(buf.clone(), prog);
        run_until_quit(&mut m);
        assert!(objects::get_attr(&m.mem, 1, 3), "attr3 should be set after set_attr");
        assert!(objects::get_attr(&m.mem, 1, 0), "attr0 still set");

        // Now clear it
        let prog2: &[u8] = &[
            0x0B, 1, 3, // set_attr obj1, attr3
            0x0C, 1, 3, // clear_attr obj1, attr3
            0xBA,
        ];
        let mut m2 = build_obj_machine_raw(buf, prog2);
        run_until_quit(&mut m2);
        assert!(!objects::get_attr(&m2.mem, 1, 3), "attr3 should be clear after clear_attr");
        assert!(objects::get_attr(&m2.mem, 1, 0), "attr0 still set");
    }

    // -----------------------------------------------------------------------
    // (c) insert_obj → get_parent / get_child reflect the change
    // -----------------------------------------------------------------------

    #[test]
    fn obj_insert_obj_updates_tree() {
        // insert_obj obj2, obj3 (move obj2 to be child of obj3)
        // Long-form 2OP: insert_obj=0x0E
        // After: parent(obj2)==3, child(obj3)==2
        // Verify by reading tree directly after running.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x0E, 2, 3, // insert_obj obj2, obj3
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(objects::get_parent(&m.mem, 2), 3, "obj2 parent should be 3");
        assert_eq!(objects::get_child(&m.mem, 3), 2, "obj3 child should be 2");
    }

    // -----------------------------------------------------------------------
    // (d) get_parent — store parent, no branch
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_parent_stores() {
        // get_parent obj2 → G0 (parent of 2 is 1)
        // Short form 1OP: 0x93, operand=2 (small const), store byte = G0 (0x10)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x93, 2, 0x10, // get_parent obj2, store → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 1, "get_parent(obj2) should be 1");
    }

    // -----------------------------------------------------------------------
    // (e) get_sibling — store AND branch on result != 0
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_sibling_stores_and_branches() {
        // get_sibling obj2 → G0 (sibling of 2 is 3); branch taken (3 != 0)
        // Short form 1OP: 0x91, operand=2, store=G0(0x10), branch data
        //
        // Instruction: 0x91, op=2, store=0x10, branch: on_true, skip 4-byte add
        // next_pc = 0x10 + 1+1+1+1_branch = 0x15 (5 bytes: opcode+op+store+1_branch_byte)
        // branch short, on_true=1, offset=6 → skip 4-byte add at 0x15 → land at 0x19
        // branch byte: 0x80|0x40|6 = 0xC6
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x91, 2, 0x10, 0xC6, // get_sibling obj2, store→G0, branch on_true offset=6
            0x14, 0, 0, 0x10,    // add 0,0 → G0 (would set G0=0, skipped)
            0xBA,                 // quit
        ];
        let mut m = build_obj_machine_raw(buf.clone(), prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 3, "get_sibling(2) should store 3");

        // get_sibling obj3 → G0 (sibling of 3 is 0); branch NOT taken
        // Not taken means the add runs, overwriting G0 with 99.
        let prog2: &[u8] = &[
            0x91, 3, 0x10, 0xC6, // get_sibling obj3, store→G0, branch on_true offset=6
            0x14, 99, 0, 0x10,   // add 99,0 → G0 (runs because branch not taken)
            0xBA,
        ];
        let mut m2 = build_obj_machine_raw(buf, prog2);
        run_until_quit(&mut m2);
        // G0 was set to 0 (sibling=0), then overwritten to 99 by add
        assert_eq!(m2.global(0), 99, "get_sibling(3) not taken: add runs, G0=99");
    }

    // -----------------------------------------------------------------------
    // (f) get_child — store AND branch on result != 0
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_child_stores_and_branches() {
        // get_child obj1 → G0 = 2, branch taken
        // Short form 1OP: 0x92, op=1, store=0x10, branch: on_true, offset=6
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x92, 1, 0x10, 0xC6, // get_child obj1 → G0, branch taken (child=2 ≠ 0)
            0x14, 0, 0, 0x10,    // add (skipped)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 2, "get_child(obj1) should store 2 and branch");
    }

    // -----------------------------------------------------------------------
    // (g) get_prop — stores the property value, fallback to default
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_prop_stores_value() {
        // get_prop obj1, prop10 → G0 = 0xABCD
        // Long-form 2OP 0x11, both small const, + store byte
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x11, 1, 10, 0x10, // get_prop obj1,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        // Note: prop10 has 2 bytes: value = 0xABCD; low byte only fits in G0? No, G0 is u16 = 0xABCD.
        assert_eq!(m.global(0), 0xABCD, "get_prop(obj1,10) should be 0xABCD");
    }

    #[test]
    fn obj_get_prop_defaults_fallback() {
        // get_prop obj2, prop10 → G0 = 0x5678 (default, obj2 has no props)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x11, 2, 10, 0x10, // get_prop obj2,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0x5678, "get_prop fallback to default");
    }

    // -----------------------------------------------------------------------
    // (h) get_prop_addr — store the address of property data
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_prop_addr_stores() {
        // get_prop_addr obj1, prop10 → G0 = non-zero address
        // Long-form 2OP 0x12
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x12, 1, 10, 0x10, // get_prop_addr obj1,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        let addr = m.global(0);
        assert_ne!(addr, 0, "get_prop_addr should be non-zero");
        // The data at that address should be 0xAB (high byte of 0xABCD)
        assert_eq!(m.mem.read_byte(addr as u32), 0xAB, "prop data at addr");
    }

    // -----------------------------------------------------------------------
    // (i) get_next_prop — iterate properties
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_next_prop_iterates() {
        // get_next_prop obj1, prop=0 → G0 = first prop (10)
        // Long-form 2OP 0x13
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x13, 1, 0, 0x10, // get_next_prop obj1,0 → G0 (first prop = 10)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 10, "first prop should be 10");
    }

    // -----------------------------------------------------------------------
    // (j) put_prop then get_prop — round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn obj_put_prop_round_trip() {
        // VAR:put_prop obj1, prop10, 0x1234 → then get_prop obj1,10 → G0
        // put_prop: VAR form 0x03 → opcode byte 0b11_1_00011 = 0xE3
        //   type byte: all small const (0b01_01_01_11) = 0x57
        //   operands: obj=1, prop=10, val_hi=0x12, val_lo=0x34
        // Wait: put_prop takes 3 operands and val is u16. Since small const max is 255,
        // can't encode 0x1234 as small. Use large const for val → need Var form.
        // Alternative: use a smaller value that fits in u8 for simplicity: val=0xAA.
        //   type byte: obj=small(01), prop=small(01), val=small(01), omit(11) → 0b01_01_01_11 = 0x57
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0xE3, 0x57, 1, 10, 0xAA, // put_prop obj1,prop10,0xAA (1-byte val, but prop is 2 bytes)
            // Actually put_prop on a 2-byte property writes 2 bytes; 0xAA goes in low byte.
            // Actually the put_prop implementation: len=2 → write_word → writes 0x00AA.
            // Let's just check with get_prop that the value is 0x00AA.
            0x11, 1, 10, 0x10, // get_prop obj1,prop10 → G0
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0x00AA, "put_prop/get_prop round-trip: 0x00AA");
    }

    // -----------------------------------------------------------------------
    // (k) get_prop_len — store length of property data
    // -----------------------------------------------------------------------

    #[test]
    fn obj_get_prop_len_stores() {
        // First get_prop_addr for obj1 prop10 → G0 (addr)
        // Then get_prop_len G0 → G1 (should be 2)
        // Short form 1OP get_prop_len: 0x94, operand = G0 (var 0x10), store = G1 (0x11)
        //   Short form with variable operand: 0b10_10_0100 = 0xA4
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x12, 1, 10, 0x10, // get_prop_addr obj1,prop10 → G0 (4 bytes)
            0xA4, 0x10, 0x11,  // get_prop_len G0 → G1 (3 bytes: 0xA4=short/var, var_num, store)
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        let len = m.mem.read_word(m.mem.global_vars() as u32 + 2);
        assert_eq!(len, 2, "get_prop_len for prop10 (2 bytes) should be 2");
    }

    // -----------------------------------------------------------------------
    // (l) remove_obj — unlinks object from parent
    // -----------------------------------------------------------------------

    #[test]
    fn obj_remove_obj_unlinks() {
        // remove_obj obj2 → obj2's parent becomes 0
        // Short form 1OP: 0x99, op=2 (small const)
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x99, 2, // remove_obj obj2
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        assert_eq!(objects::get_parent(&m.mem, 2), 0, "obj2 parent should be 0 after remove_obj");
        assert_eq!(objects::get_child(&m.mem, 1), 3, "obj1 child should now be 3 (obj2 removed)");
    }

    // -----------------------------------------------------------------------
    // (m) print_obj — writes short name to the output sink
    // -----------------------------------------------------------------------

    #[test]
    fn obj_print_obj_writes_to_output() {
        // print_obj needs an object. Use build_obj_story() which has objects with
        // zero name words (empty name). The important thing is the opcode is wired up
        // and routes to self.out rather than the removed out_buf.
        let buf = build_obj_story();
        let prog: &[u8] = &[
            0x9A, 1, // print_obj obj1
            0xBA,
        ];
        let mut m = build_obj_machine_raw(buf, prog);
        run_until_quit(&mut m);
        // short_name of obj1 with 0 name words returns "" — output was called (just empty).
        // We verify no panic and that the sink is accessible.
        let out = m.buffer_output().expect("default sink is BufferOutput");
        // empty name string was printed — buf is "" (valid)
        let _ = &out.buf;
    }

    // -----------------------------------------------------------------------
    // Task 11: text output opcode tests
    // -----------------------------------------------------------------------

    // Helper: build a test machine from raw bytes placed at 0x10.
    fn build_raw_machine(buf: Vec<u8>) -> Machine {
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m
    }

    /// Test: `print` (inline) + `new_line` + `print_num -7` → sink receives "Hello\n-7".
    ///
    /// Z-encoding "Hello" (A0/A1 alphabets, 3 words):
    ///   H: shift-A1 (Z4) + Z13 (A1[7]='H', index = zchar-6 = 13-6 = 7)
    ///   e: Z10 (A0: a=Z6,b=Z7,...,e=Z10)
    ///   l: Z17, l: Z17, o: Z20; pad Z5,Z5 to fill word 2.
    ///   word0: Z4,Z13,Z10  = (4<<10)|(13<<5)|10
    ///   word1: Z17,Z17,Z20 = (17<<10)|(17<<5)|20
    ///   word2 (last): Z5,Z5,Z5 = 0x8000|(5<<10)|(5<<5)|5
    ///
    /// print_num -7: use Large constant 0xFFF9 (small constants are 0-255, no sign).
    #[test]
    fn text_print_newline_print_num() {
        let mut buf = sample_story(5);

        let w0: u16 = (4u16 << 10) | (13u16 << 5) | 10u16;
        let w1: u16 = (17u16 << 10) | (17u16 << 5) | 20u16;
        let w2: u16 = 0x8000 | (5u16 << 10) | (5u16 << 5) | 5u16;

        buf[0x10] = 0xB2;  // 0OP print
        buf[0x11] = (w0 >> 8) as u8; buf[0x12] = (w0 & 0xFF) as u8;
        buf[0x13] = (w1 >> 8) as u8; buf[0x14] = (w1 & 0xFF) as u8;
        buf[0x15] = (w2 >> 8) as u8; buf[0x16] = (w2 & 0xFF) as u8;

        buf[0x17] = 0xBB;  // 0OP new_line

        // VAR:0x06 print_num with Large const -7 (0xFFF9)
        buf[0x18] = 0xE6;
        buf[0x19] = 0x3F;  // type: large first, rest omit
        buf[0x1A] = 0xFF;
        buf[0x1B] = 0xF9;

        buf[0x1C] = 0xBA;  // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "Hello\n-7");
    }

    /// Test: `print_char` for ZSCII 65 ('A') prints 'A'.
    #[test]
    fn text_print_char_known_zscii() {
        let mut buf = sample_story(5);
        // VAR:0x05 print_char, operand=65 (ZSCII 'A')
        // 0xE5 = 0b11_1_00101 (VAR form, opcode 5)
        // type byte: first=small const(01), rest=omit → 0x7F
        buf[0x10] = 0xE5;
        buf[0x11] = 0x7F;
        buf[0x12] = 65u8; // 'A'
        buf[0x13] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "A");
    }

    /// Test: `print_addr` decodes a string at a byte address and prints it.
    #[test]
    fn text_print_addr_decodes_string() {
        let mut buf = sample_story(5);
        // Z-encode "abc": a=Z6, b=Z7, c=Z8
        // word = 0x8000|(6<<10)|(7<<5)|8 = 0x8000|0x1800|0x00E0|0x08 = 0x98E8
        let abc_word: u16 = 0x8000 | (6u16 << 10) | (7u16 << 5) | 8u16;
        buf[0x0200] = (abc_word >> 8) as u8;
        buf[0x0201] = (abc_word & 0xFF) as u8;

        // 1OP:0x07 print_addr, Large operand 0x0200
        // Short form 1OP large const: 0b10_00_0111 = 0x87
        buf[0x10] = 0x87;
        buf[0x11] = 0x02;
        buf[0x12] = 0x00;
        buf[0x13] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "abc");
    }

    /// Test: `print_paddr` unpacks a packed address and prints the string.
    #[test]
    fn text_print_paddr_decodes_string() {
        let mut buf = sample_story(5);
        // v5: unpack_string(packed) = packed * 4. Use packed=0x0050 → byte 0x0140.
        // sample_story(5) is 1024 bytes (0x400); 0x0140 is within bounds.
        // Z-encode "de": d=Z9, e=Z10, pad=Z5
        // word = 0x8000|(9<<10)|(10<<5)|5
        let de_word: u16 = 0x8000 | (9u16 << 10) | (10u16 << 5) | 5u16;
        buf[0x0140] = (de_word >> 8) as u8;
        buf[0x0141] = (de_word & 0xFF) as u8;

        // 1OP:0x0D print_paddr, Large operand 0x0050
        // Short form 1OP large const: 0b10_00_1101 = 0x8D
        buf[0x10] = 0x8D;
        buf[0x11] = 0x00;
        buf[0x12] = 0x50;
        buf[0x13] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert_eq!(out.buf, "de");
    }

    // -----------------------------------------------------------------------
    // Task 12: input opcode tests
    //
    // Dictionary layout shared by these tests:
    //   Same hand-built v3/v5 dict used in dictionary module tests.
    //   We embed a minimal dictionary containing "north", "open", "mailbox"
    //   at memory address 0x0200 (where sample_story points `dictionary`).
    //
    // Text/parse buffers are placed in dynamic memory away from code:
    //   text_buf  at 0x0300 (before global_vars which starts at 0x0300 —
    //   BUT global_vars are at 0x0300 in sample_story! Use 0x0280 instead.)
    //   parse_buf at 0x02C0
    // -----------------------------------------------------------------------

    /// Build a story buffer with a hand-crafted dictionary at 0x0200.
    /// Entries: "north", "open", "mailbox" (sorted by encoded key, 4-byte keys, v3).
    /// Returns (buf, addr_north, addr_open, addr_mailbox).
    fn build_input_story(version: u8) -> (Vec<u8>, u16, u16, u16) {
        use crate::text::encode::encode_word;

        let mut buf = sample_story(version);

        // We use 4-byte keys for v3 and 6-byte keys for v5.
        let key_len: usize = if version <= 3 { 4 } else { 6 };

        // encode_word takes the story version (not syllable count).
        let key_north   = encode_word("north",   version);
        let key_open    = encode_word("open",    version);
        let key_mailbox = encode_word("mailbox", version);

        // Sort by key bytes for binary search.
        let mut entries: Vec<(&str, Vec<u8>)> = vec![
            ("north",   key_north.clone()),
            ("open",    key_open.clone()),
            ("mailbox", key_mailbox.clone()),
        ];
        entries.sort_by(|a, b| a.1.cmp(&b.1));

        let entry_length: usize = key_len + 2; // key + 2 bytes game data (total ≥ 4 v3, ≥ 6 v4+)

        // Write dictionary header at 0x0200.
        buf[0x0200] = 1;    // 1 separator
        buf[0x0201] = b'.'; // separator = '.'
        buf[0x0202] = entry_length as u8;
        buf[0x0203] = 0;
        buf[0x0204] = 3;    // count = 3

        // Entries start at 0x0205.
        let entries_base: usize = 0x0205;
        for (i, (_word, key)) in entries.iter().enumerate() {
            let base = entries_base + i * entry_length;
            buf[base..base + key_len].copy_from_slice(&key[..key_len]);
        }

        // Compute addresses for each word in sorted order.
        let addr_for = |word: &str| -> u16 {
            for (i, (w, _)) in entries.iter().enumerate() {
                if *w == word {
                    return (entries_base + i * entry_length) as u16;
                }
            }
            panic!("word not found: {}", word);
        };

        let addr_north   = addr_for("north");
        let addr_open    = addr_for("open");
        let addr_mailbox = addr_for("mailbox");

        (buf, addr_north, addr_open, addr_mailbox)
    }

    /// Build a VAR-form `read` instruction (opcode 0x04) at `buf[offset]`.
    /// Operands: two Large constants (text_buf addr, parse_buf addr).
    /// v5+ includes a store byte; v3 does not.
    /// Returns the number of bytes emitted.
    fn emit_read(buf: &mut [u8], offset: usize, text_buf: u16, parse_buf: u16, version: u8, store_var: Option<u8>) -> usize {
        // VAR-form opcode for read: 0b11_1_00100 = 0xE4
        buf[offset] = 0xE4;
        // Type byte: first two = large const (0b00), rest = omit (0b11).
        // 0b00_00_11_11 = 0x0F
        buf[offset + 1] = 0x0F;
        // text_buf (large const, 2 bytes)
        buf[offset + 2] = (text_buf >> 8) as u8;
        buf[offset + 3] = (text_buf & 0xFF) as u8;
        // parse_buf (large const, 2 bytes)
        buf[offset + 4] = (parse_buf >> 8) as u8;
        buf[offset + 5] = (parse_buf & 0xFF) as u8;
        let mut len = 6;
        // v5+ has store byte
        if version >= 5 {
            if let Some(sv) = store_var {
                buf[offset + len] = sv;
                len += 1;
            }
        }
        len
    }

    // -----------------------------------------------------------------------
    // Test (a-v3): v3 read → NeedLine → supply_line("north") → check text/parse buf
    // -----------------------------------------------------------------------
    #[test]
    fn read_v3_need_line_supply_north() {
        let (mut buf, addr_north, _addr_open, _addr_mailbox) = build_input_story(3);

        // Text buffer at 0x0250: byte0=max_len=10, rest zero.
        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 10; // max 10 chars

        // Parse buffer at 0x0260: byte0=max_tokens=8, rest zero.
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8; // max 8 tokens

        // Instruction at 0x0010: read text_buf, parse_buf; quit
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        // Step 1: step() returns NeedLine with correct addresses.
        let result = m.step();
        assert!(
            matches!(result, StepResult::NeedLine { text_buf: tb, parse_buf: pb } if tb == text_buf as u32 && pb == parse_buf as u32),
            "expected NeedLine{{text_buf={:#x}, parse_buf={:#x}}}, got {:?}", text_buf, parse_buf, result
        );

        // Step 2: supply_line("north") and check text buffer (v3 layout).
        m.supply_line("north", 13);

        // v3: byte 0 = max (untouched), text at byte 1, null-terminated.
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'n', "text[1] = 'n'");
        assert_eq!(m.mem.read_byte(tb + 2), b'o', "text[2] = 'o'");
        assert_eq!(m.mem.read_byte(tb + 3), b'r', "text[3] = 'r'");
        assert_eq!(m.mem.read_byte(tb + 4), b't', "text[4] = 't'");
        assert_eq!(m.mem.read_byte(tb + 5), b'h', "text[5] = 'h'");
        assert_eq!(m.mem.read_byte(tb + 6), 0,    "null terminator at text[6]");

        // Parse buffer: token count = 1, first token has correct fields.
        let pb = parse_buf as u32;
        assert_eq!(m.mem.read_byte(pb + 1), 1, "parse buf: 1 token");
        // Token 0: dict_addr (2 bytes), len (1 byte), text_buf_pos (1 byte).
        let tok_dict = m.mem.read_word(pb + 2);
        let tok_len  = m.mem.read_byte(pb + 4);
        let tok_pos  = m.mem.read_byte(pb + 5);
        assert_eq!(tok_dict, addr_north, "token dict addr = addr_north ({:#x})", addr_north);
        assert_eq!(tok_len,  5,          "token len = 5 ('north')");
        assert_eq!(tok_pos,  1,          "token pos = 1 (v3: text starts at byte 1, 'north' at pos 0 in input → buf pos = 1+0 = 1)");

        // Machine continues normally after supply_line.
        let r2 = m.step();
        assert_eq!(r2, StepResult::Quit, "next step is quit");
    }

    // A host Save State is taken while suspended at a `read` prompt. Its saved PC
    // must rewind to the read instruction (not the post-read address) so that
    // restoring re-executes the read and re-arms the prompt on the restored
    // buffers. Otherwise the resume lands past the read and the next line replays
    // whatever command was sitting in the restored input buffer.
    #[test]
    fn save_at_read_prompt_rewinds_pc_so_restore_rearms_the_read() {
        let (mut buf, _n, _o, _m) = build_input_story(3);
        let text_buf: u16 = 0x0250; buf[text_buf as usize] = 10;
        let parse_buf: u16 = 0x0260; buf[parse_buf as usize] = 8;
        let read_pc: u32 = 0x0010;
        let n = emit_read(&mut buf, read_pc as usize, text_buf, parse_buf, 3, None);
        buf[read_pc as usize + n] = 0xBA; // quit after the read

        // Drive to the read prompt, then snapshot a Save State here.
        let mut m = Machine::new(Memory::new(buf.clone()).unwrap());
        m.state.pc = read_pc;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }), "reached the read prompt");
        assert_ne!(m.state.pc, read_pc, "state.pc has advanced past the read");
        let save = m.save_quetzal();

        // The saved PC rewinds to the read instruction, not the post-read PC.
        assert_eq!(crate::quetzal::saved_pc_of(&save), read_pc,
            "Save-State PC must be the read instruction so restore re-executes it");

        // Restoring into a fresh machine lands on the read and re-arms it.
        let mut m2 = Machine::new(Memory::new(buf).unwrap());
        m2.restore_file(&save).expect("restore Save State");
        assert_eq!(m2.state.pc, read_pc, "restored PC is the read instruction");
        assert!(matches!(m2.step(), StepResult::NeedLine { .. }),
            "restore re-executes the read → NeedLine (prompt re-armed), not a fall-through");
    }

    // -----------------------------------------------------------------------
    // Test (a-v5): v5 read → NeedLine → supply_line("north") → check text/parse buf
    // -----------------------------------------------------------------------
    #[test]
    fn read_v5_need_line_supply_north() {
        let (mut buf, addr_north, _addr_open, _addr_mailbox) = build_input_story(5);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 10; // max 10 chars

        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        // v5: read has a store var (terminator char).
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 5, Some(0x10)); // store→G0
        buf[0x0010 + n] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        let result = m.step();
        assert!(
            matches!(result, StepResult::NeedLine { text_buf: tb, parse_buf: pb } if tb == text_buf as u32 && pb == parse_buf as u32),
            "expected NeedLine, got {:?}", result
        );

        m.supply_line("north", 13);

        // v5: byte 0 = max (untouched), byte 1 = char count, text at byte 2.
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), 5,    "v5: char count = 5");
        assert_eq!(m.mem.read_byte(tb + 2), b'n', "text[2] = 'n'");
        assert_eq!(m.mem.read_byte(tb + 6), b'h', "text[6] = 'h'");

        // Parse buffer: 1 token, correct position (text_data_start=2 for v5).
        let pb = parse_buf as u32;
        assert_eq!(m.mem.read_byte(pb + 1), 1, "1 token");
        let tok_dict = m.mem.read_word(pb + 2);
        let tok_len  = m.mem.read_byte(pb + 4);
        let tok_pos  = m.mem.read_byte(pb + 5);
        assert_eq!(tok_dict, addr_north, "v5 token dict addr = addr_north");
        assert_eq!(tok_len,  5,          "v5 token len = 5");
        assert_eq!(tok_pos,  2,          "v5 token pos = 2 (text_data_start=2, text_pos=0 → 2+0=2)");

        // v5: terminator (13 = Enter) stored in G0.
        assert_eq!(m.global(0), 13, "v5 read stores terminator 13 in G0");
    }

    // -----------------------------------------------------------------------
    // Test (b): two-word input "open mailbox" → 2 tokens, correct positions
    // -----------------------------------------------------------------------
    #[test]
    fn read_v3_two_word_input_open_mailbox() {
        let (mut buf, _addr_north, addr_open, addr_mailbox) = build_input_story(3);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 20; // max 20 chars

        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        let result = m.step();
        assert!(matches!(result, StepResult::NeedLine { .. }));

        m.supply_line("open mailbox", 13);

        // v3 layout: text at offset 1.
        // "open mailbox" → 12 chars; null at byte 1+12=13.
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'o', "starts with 'o'");
        assert_eq!(m.mem.read_byte(tb + 13), 0, "null terminator");

        let pb = parse_buf as u32;
        assert_eq!(m.mem.read_byte(pb + 1), 2, "2 tokens");

        // Token 0: "open" at text_pos=0 → buf_pos=1.
        let tok0_dict = m.mem.read_word(pb + 2);
        let tok0_len  = m.mem.read_byte(pb + 4);
        let tok0_pos  = m.mem.read_byte(pb + 5);
        assert_eq!(tok0_dict, addr_open, "tok0 = 'open'");
        assert_eq!(tok0_len,  4,         "tok0 len = 4");
        assert_eq!(tok0_pos,  1,         "tok0 buf_pos = 1 (text_data_start=1, text_pos=0)");

        // Token 1: "mailbox" at text_pos=5 → buf_pos=6.
        let tok1_dict = m.mem.read_word(pb + 6);
        let tok1_len  = m.mem.read_byte(pb + 8);
        let tok1_pos  = m.mem.read_byte(pb + 9);
        assert_eq!(tok1_dict, addr_mailbox, "tok1 = 'mailbox'");
        assert_eq!(tok1_len,  7,            "tok1 len = 7");
        assert_eq!(tok1_pos,  6,            "tok1 buf_pos = 6 (text_data_start=1, text_pos=5 → 1+5=6)");
    }

    // -----------------------------------------------------------------------
    // Test (c): v5 read stores terminator char (already covered in v5 test,
    //   but explicit test for completeness)
    // -----------------------------------------------------------------------
    #[test]
    fn read_v5_stores_terminator() {
        let (mut buf, _addr_north, _addr_open, _addr_mailbox) = build_input_story(5);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 20;

        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        // Store into G1 (var 0x11)
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 5, Some(0x11));
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        m.step(); // returns NeedLine
        m.supply_line("hello", 13);

        // G1 should have been set to 13 (Enter).
        let g1 = m.mem.read_word(m.mem.global_vars() as u32 + 2);
        assert_eq!(g1, 13, "v5 read terminator stored in G1 = 13");
    }

    #[test]
    fn supply_line_v5_stores_function_key_terminator() {
        // v5 read terminated by a cursor key (ZSCII 129) stores 129, not 13.
        let (mut buf, ..) = build_input_story(5);
        let text_buf: u16 = 0x0250; buf[text_buf as usize] = 20;
        let parse_buf: u16 = 0x0260; buf[parse_buf as usize] = 8;
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 5, Some(0x11));
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;
        m.step();
        m.supply_line("look", 129);

        let g1 = m.mem.read_word(m.mem.global_vars() as u32 + 2);
        assert_eq!(g1, 129, "v5 read stores the function-key terminator in G1");
    }

    #[test]
    fn supply_char_stores_delete_and_escape() {
        // ZSCII 8 (delete/backspace) and 27 (ESC) reach the read_char store var.
        for z in [8u8, 27u8] {
            let mut buf = sample_story(5);
            buf[0x0010] = 0xF6; // VAR read_char
            buf[0x0011] = 0x7F; // small const, omit, omit, omit
            buf[0x0012] = 1;    // device = keyboard
            buf[0x0013] = 0x10; // store → G0
            buf[0x0014] = 0xBA; // quit
            let mem = Memory::new(buf).unwrap();
            let mut m = Machine::new(mem);
            m.state.pc = 0x0010;
            assert_eq!(m.step(), StepResult::NeedChar);
            m.supply_char(z);
            assert_eq!(m.global(0), z as u16, "supply_char({z}) stored in G0");
        }
    }

    // -----------------------------------------------------------------------
    // Crafted-story / illegal-instruction hardening (SQ-0619..SQ-0622):
    // every case here used to panic, hang, overrun a buffer, or corrupt a
    // save — all must now either compute the reference-interpreter result or
    // latch a graceful StepResult::Fault.
    // -----------------------------------------------------------------------

    #[test]
    fn div_and_mod_i16_min_by_neg_one_wrap_instead_of_panicking() {
        // 0x8000 / 0xFFFF (= -32768 / -1) overflows i16: a plain `/` panics in
        // debug AND release. Frotz computes in C `int` and truncates → 0x8000;
        // mod likewise → 0. (SQ-0619)
        for (opcode, expect) in [(0xD7u8, 0x8000u16), (0xD8u8, 0u16)] {
            let mut buf = sample_story(5);
            buf[0x10] = opcode; // 2OP div/mod in variable-form encoding
            buf[0x11] = 0x0F;   // types: large, large, omit, omit
            buf[0x12] = 0x80; buf[0x13] = 0x00; // a = -32768
            buf[0x14] = 0xFF; buf[0x15] = 0xFF; // b = -1
            buf[0x16] = 0x10; // store → G0
            buf[0x17] = 0xBA; // quit
            let mut m = Machine::new(Memory::new(buf).unwrap());
            m.state.pc = 0x10;
            assert_eq!(m.step(), StepResult::Continue, "opcode {opcode:#x} must not fault");
            assert_eq!(m.global(0), expect, "opcode {opcode:#x} result");
        }
    }

    #[test]
    fn put_prop_on_missing_property_faults_instead_of_panicking() {
        // obj2 has NO properties. ZMSD §15 put_prop: "the interpreter should
        // halt with a suitable error message" — the VM's halt is a latched
        // Fault, never a process panic. (SQ-0619)
        let buf = build_obj_story();
        // VAR put_prop (0xE3); types small,small,small,omit = 0x57
        let prog: &[u8] = &[0xE3, 0x57, 2, 10, 0x42, 0xBA];
        let mut m = build_obj_machine_raw(buf, prog);
        assert_eq!(m.step(), StepResult::Fault);
        let t = m.take_fault_trace().expect("fault trace present");
        assert!(t.fault.contains("put_prop"), "fault names the opcode: {}", t.fault);
    }

    #[test]
    fn remove_obj_with_sibling_cycle_faults_instead_of_hanging() {
        // Corrupted object table: obj2.sibling=3 and obj3.sibling=2 form a
        // cycle, and obj1 (its own parent) is never in that chain — the
        // predecessor walk used to spin forever. (SQ-0619)
        let mut buf = build_obj_story();
        const ENTRIES: usize = 0x0100 + 31 * 2;
        buf[ENTRIES + 4] = 1;      // obj1.parent = 1 (itself)
        buf[ENTRIES + 18 + 5] = 2; // obj3.sibling = 2 → 2→3→2 cycle
        // 1OP remove_obj (0x09), short form, small const: 0x99
        let prog: &[u8] = &[0x99, 1, 0xBA];
        let mut m = build_obj_machine_raw(buf, prog);
        assert_eq!(m.step(), StepResult::Fault);
        let t = m.take_fault_trace().expect("fault trace present");
        assert!(t.fault.contains("sibling cycle"), "fault: {}", t.fault);
    }

    #[test]
    fn split_window_huge_operand_is_capped() {
        // split_window 0xFFFF used to allocate rows×cols cells straight from
        // the operand (~400 MB at 80 cols); the grid is now capped exactly
        // like EXT window_size (GRID_CELL_CAP). (SQ-0620)
        let mut buf = sample_story(5);
        buf[0x21] = 80; // header: 80 columns
        buf[0x10] = 0xEA; // VAR split_window
        buf[0x11] = 0x3F; // types: large, omit, omit, omit
        buf[0x12] = 0xFF; buf[0x13] = 0xFF; // rows = 0xFFFF
        buf[0x14] = 0xBA; // quit
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.state.pc = 0x10;
        assert_eq!(m.step(), StepResult::Continue);
        assert_eq!(m.screen.upper_window_rows, GRID_CELL_CAP, "rows capped");
        assert_eq!(m.screen.upper.rows, GRID_CELL_CAP, "grid allocation capped");
    }

    #[test]
    fn is_terminator_unterminated_table_does_not_latch_spurious_fault() {
        // A terminating-characters table with no 0 byte before end-of-memory:
        // the walk must stop at EOF without latching an OOB fault that the
        // NEXT step() would report against an innocent instruction. (SQ-0620)
        let mut buf = sample_story(5);
        let len = buf.len();
        let tbl = len - 4;
        buf[0x2E] = (tbl >> 8) as u8;
        buf[0x2F] = (tbl & 0xFF) as u8;
        for b in &mut buf[tbl..] {
            *b = 1; // nonzero, not 255, not a key we ask about
        }
        let m = Machine::new(Memory::new(buf).unwrap());
        assert!(!m.is_terminator(65));
        assert_eq!(m.mem.take_mem_fault(), None, "no spurious OOB fault latched");
    }

    #[test]
    fn supply_line_non_ascii_truncates_by_chars_and_stores_zscii() {
        // 'é' is 2 bytes of UTF-8 but ONE ZSCII character (code 170 in the
        // default table). Truncation must count characters (byte-slicing the
        // UTF-8 input panicked mid-char), and the buffer must receive ZSCII
        // bytes (raw UTF-8 0xC3 0xA9 can never match a dictionary). (SQ-0621)
        let (mut buf, ..) = build_input_story(5);
        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 5; // v5: max 5 chars — "ééééé" is 10 UTF-8 bytes
        let n = emit_read(&mut buf, 0x0010, text_buf, 0, 5, Some(0x11));
        buf[0x0010 + n] = 0xBA;
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.state.pc = 0x0010;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        m.supply_line("ééééé", 13);
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), 5, "count = 5 CHARACTERS");
        for i in 0..5 {
            assert_eq!(m.mem.read_byte(tb + 2 + i), 170, "char {i} is ZSCII 170, not UTF-8 bytes");
        }
    }

    #[test]
    fn supply_line_v14_accepts_byte0_minus_one_letters() {
        // ZMSD §15 read (v1–4): byte 0 holds "the maximum number of letters
        // which can be typed, minus 1"; Frotz reads byte 0 then does `max--`.
        // Writing byte0 letters + NUL overran the game's buffer by one byte.
        // (SQ-0621)
        let (mut buf, ..) = build_input_story(3);
        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 5;        // byte 0 = 5 → at most 4 letters
        buf[text_buf as usize + 6] = 0xAA; // sentinel just past the game's buffer
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA;
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.state.pc = 0x0010;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        m.supply_line("abcdefgh", 13);
        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'a');
        assert_eq!(m.mem.read_byte(tb + 4), b'd', "4 letters accepted");
        assert_eq!(m.mem.read_byte(tb + 5), 0, "NUL terminator inside the buffer");
        assert_eq!(m.mem.read_byte(tb + 6), 0xAA, "byte past the buffer untouched");
    }

    #[test]
    fn restore_at_read_prompt_clears_stale_pending_input() {
        // A restore performed while suspended at a read must drop the stale
        // PendingInput: save_pc() otherwise rewinds any subsequent host save
        // to the PRE-restore read instruction. (SQ-0622)
        let (mut buf, ..) = build_input_story(3);
        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 10;
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;
        let read_pc: u32 = 0x0010;
        let n = emit_read(&mut buf, read_pc as usize, text_buf, parse_buf, 3, None);
        buf[read_pc as usize + n] = 0xBA;
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.state.pc = read_pc;
        // Host save of the plain pre-read state.
        let save = m.save_quetzal();
        // Suspend at the read, then restore the earlier save over it.
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        assert!(m.pending_read_pc().is_some(), "suspended at the read");
        m.restore_file(&save).expect("restore");
        assert_eq!(m.pending_read_pc(), None, "restore cleared the stale PendingInput");
        assert_eq!(m.save_pc(), m.state.pc,
            "a save right after restore records the restored PC, not the dead read's");
    }

    // -----------------------------------------------------------------------
    // Test (d): read_char → NeedChar → supply_char(65) stores 65 in store var
    // -----------------------------------------------------------------------
    #[test]
    fn read_char_need_char_supply_char() {
        let mut buf = sample_story(5);

        // VAR-form read_char: opcode 0x16 → 0b11_1_10110 = 0xF6
        // Operands: first arg = 1 (keyboard, required). Type byte: small const(01), rest omit(11).
        // Store byte → G0 (0x10).
        buf[0x0010] = 0xF6; // VAR read_char
        buf[0x0011] = 0x7F; // type: small(01), omit, omit, omit → 0b01_11_11_11 = 0x7F
        buf[0x0012] = 1;    // operand: device=1 (keyboard)
        buf[0x0013] = 0x10; // store → G0
        buf[0x0014] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;

        let result = m.step();
        assert_eq!(result, StepResult::NeedChar, "read_char returns NeedChar");

        m.supply_char(65); // ZSCII 'A'

        assert_eq!(m.global(0), 65, "supply_char(65) stored in G0");

        let r2 = m.step();
        assert_eq!(r2, StepResult::Quit, "machine resumes after supply_char");
    }

    // -----------------------------------------------------------------------
    // Test: pending_timeout() exposes the read's time/routine operands
    // -----------------------------------------------------------------------
    #[test]
    fn pending_timeout_exposes_time_and_routine() {
        // v5 read with time=10 (1.0s) and routine at packed addr 0x0040.
        // VAR read (0xE4) with four Large-const operands: text_buf, parse_buf,
        // time, routine. Type byte: large,large,large,large -> 0b00_00_00_00 = 0x00.
        let mut buf = sample_story(5);
        buf[0x10] = 0xE4;
        buf[0x11] = 0x00;
        buf[0x12] = 0x02; buf[0x13] = 0x50; // text_buf 0x0250
        buf[0x14] = 0x02; buf[0x15] = 0x60; // parse_buf 0x0260
        buf[0x16] = 0x00; buf[0x17] = 0x0A; // time = 10
        buf[0x18] = 0x00; buf[0x19] = 0x40; // routine = 0x40
        buf[0x1A] = 0x10;                   // store var (v5 read stores terminator) -> G0
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        let r = m.step();
        assert!(matches!(r, StepResult::NeedLine { .. }), "read suspends: {r:?}");
        assert_eq!(m.pending_timeout(), Some((10, 0x40)), "time+routine exposed");
    }

    // -----------------------------------------------------------------------
    // Test: pending_timeout() is None for an untimed read
    // -----------------------------------------------------------------------
    #[test]
    fn pending_timeout_none_when_untimed() {
        // Existing untimed v5 read (no time/routine) -> None.
        let mut buf = sample_story(5);
        let n = emit_read(&mut buf, 0x10, 0x0250, 0x0260, 5, Some(0x10));
        buf[0x10 + n] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        let _ = m.step();
        assert_eq!(m.pending_timeout(), None, "untimed read exposes no timeout");
    }

    // -----------------------------------------------------------------------
    // Helper: v5 story whose `read` at 0x10 is timed (time=5, routine=ROUT).
    // ROUT is placed at 0x0280 — clear of the dictionary (0x0200), the text/
    // parse buffers (0x0250/0x0260), and the global-vars table (0x0300) that
    // sample_story wires up, so writing the routine's bytes there can't
    // corrupt engine state the test later reads (e.g. global 0).
    // -----------------------------------------------------------------------
    fn timed_read_story(routine_body: &[u8]) -> (Vec<u8>, u32) {
        let mut buf = sample_story(5);
        let rout: u32 = 0x0280;
        // v5 packed routine addr = byte addr / 4, routine_offset 0 (confirmed by
        // call_and_return_value above: byte 0x80 -> packed 0x20, 4*0x20=0x80).
        let packed = (rout / 4) as u16;
        buf[0x10] = 0xE4; // VAR read
        buf[0x11] = 0x00; // type byte: large,large,large,large
        buf[0x12] = 0x02;
        buf[0x13] = 0x50; // text_buf 0x0250
        buf[0x14] = 0x02;
        buf[0x15] = 0x60; // parse_buf 0x0260
        buf[0x16] = 0x00;
        buf[0x17] = 0x05; // time = 5 (tenths)
        buf[0x18] = (packed >> 8) as u8;
        buf[0x19] = (packed & 0xFF) as u8; // routine (packed)
        buf[0x1A] = 0x10; // store var (v5 read stores terminator) -> G0
        // routine header: 0 locals (v5: no initial-value words), then body.
        buf[rout as usize] = 0x00;
        for (i, b) in routine_body.iter().enumerate() {
            buf[rout as usize + 1 + i] = *b;
        }
        (buf, rout)
    }

    // -----------------------------------------------------------------------
    // Test: run_timed_interrupt runs the routine; a true return aborts the read
    // -----------------------------------------------------------------------
    #[test]
    fn run_timed_interrupt_abort_when_routine_true() {
        // routine body: rtrue (0OP 0xB0).
        let (buf, _) = timed_read_story(&[0xB0]);
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        let depth_before = m.state.frames.len();
        let out = m.run_timed_interrupt();
        assert!(out.aborted, "routine returned true -> abort");
        assert_eq!(m.state.frames.len(), depth_before, "frame stack restored");
        assert!(
            m.pending_timeout().is_some(),
            "pending_input untouched on abort — the host decides whether to abort the read"
        );
    }

    // -----------------------------------------------------------------------
    // Test: a false return continues the read; side effects and eval-stack
    // depth are preserved.
    // -----------------------------------------------------------------------
    #[test]
    fn run_timed_interrupt_continue_and_side_effect() {
        // routine body: inc G0 (1OP:0x05, short form, small-constant operand
        // 0x10 = variable number for global 0), then rfalse (0OP 0xB1).
        // 0x95 = 0b10_01_0101: short form, small constant, opcode 5 (inc).
        let (buf, _) = timed_read_story(&[0x95, 0x10, 0xB1]);
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        let depth_before = m.state.frames.len();
        let stack_before = m.state.eval_stack.len();
        let g_before = m.global(0);
        let out = m.run_timed_interrupt();
        assert!(!out.aborted, "routine returned false -> continue");
        assert_eq!(
            m.global(0),
            g_before.wrapping_add(1),
            "routine side effect applied"
        );
        assert_eq!(m.state.frames.len(), depth_before, "frame stack restored");
        assert_eq!(m.state.eval_stack.len(), stack_before, "eval stack restored");
        assert!(
            m.pending_timeout().is_some(),
            "read still pending after a non-aborting interrupt"
        );
    }

    // -----------------------------------------------------------------------
    // Test: run_timed_interrupt's no-frame branch (call_routine bails out
    // before pushing a frame) still pops a value and reports non-aborting,
    // leaving frames/eval-stack exactly where they started.
    // -----------------------------------------------------------------------
    #[test]
    fn run_timed_interrupt_no_frame_branch_leaves_stack_intact() {
        // Patch the interrupt routine field to a packed address that unpacks
        // past mem.len() (0x400): call_routine's out-of-bounds guard fires,
        // storing 0 into var 0 (the eval stack) WITHOUT pushing a frame —
        // the same no-frame path taken for packed_addr==0 or local_count>15.
        let (mut buf, _rout) = timed_read_story(&[0xB0]); // body unused; routine never runs
        buf[0x18] = 0xFF;
        buf[0x19] = 0xFF; // packed 0xFFFF -> unpacks to 4*0xFFFF, far past mem.len()
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        let depth_before = m.state.frames.len();
        let stack_before = m.state.eval_stack.len();
        let out = m.run_timed_interrupt();
        assert!(!out.aborted, "call_routine's no-frame guard stores 0 -> not aborted");
        assert_eq!(m.state.frames.len(), depth_before, "no frame pushed or leaked");
        assert_eq!(m.state.eval_stack.len(), stack_before, "eval stack depth unchanged");
        assert!(
            m.pending_timeout().is_some(),
            "read still pending after a non-aborting interrupt"
        );
    }

    #[test]
    fn run_routine_returns_true_value() {
        // routine body: rtrue (0OP 0xB0) -> returns 1.
        let (buf, rout) = timed_read_story(&[0xB0]);
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        let packed = (rout / 4) as u16;
        assert_eq!(m.run_routine(packed), 1, "rtrue routine returns 1");
        assert!(m.pending_input.is_none(), "no read pending -> pending_input stays None");
    }

    #[test]
    fn run_routine_returns_explicit_value() {
        // routine body: ret 7 (1OP:0x0B short form small constant 7): 0x9B 0x07.
        let (buf, rout) = timed_read_story(&[0x9B, 0x07]);
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        let packed = (rout / 4) as u16;
        assert_eq!(m.run_routine(packed), 7, "ret 7 returns 7");
    }

    // -----------------------------------------------------------------------
    // v6 newline interrupt (window props 8/9, ZMSD §8.8.3.2.2). Frotz reference:
    // `countdown()` / `screen_new_line` in src/common/screen.c.
    //
    // Helper: v6 story whose `main` (byte 0x40, packed 0x10) runs `main_body` at
    // 0x41, plus a newline-interrupt routine at byte 0x280 (packed 0xA0, since
    // sample_story's routines_offset is 0 → 4*0xA0 = 0x280) with `rout_body` at
    // 0x281. Window-0 output flows through print_text's scrolling stream path.
    // -----------------------------------------------------------------------
    fn v6_newline_story(main_body: &[u8], rout_body: &[u8]) -> Vec<u8> {
        let mut buf = crate::header::tests_support::sample_story(6);
        buf[0x06] = 0x00; buf[0x07] = 0x10; // header 0x06 = packed addr of main → byte 0x40
        buf[0x40] = 0x00;                   // main: 0 locals
        for (i, b) in main_body.iter().enumerate() { buf[0x41 + i] = *b; }
        buf[0x280] = 0x00;                  // interrupt routine: 0 locals
        for (i, b) in rout_body.iter().enumerate() { buf[0x281 + i] = *b; }
        buf
    }

    // Test: a new-line printed to the scrolling window decrements prop 9 but does
    // NOT fire the routine while the count is still above zero.
    #[test]
    fn newline_interrupt_counts_down_without_firing() {
        let mut m = Machine::new(
            Memory::new(crate::header::tests_support::sample_story(6)).unwrap(),
        );
        {
            let w = &mut m.screen.v6.as_mut().unwrap().windows[0];
            w.interrupt_countdown = 3;
            w.interrupt_routine = 0xA0; // present, but must not fire yet
        }
        m.print_text("hello\n");
        assert_eq!(
            m.screen.v6.as_ref().unwrap().windows[0].interrupt_countdown, 2,
            "one new-line → one decrement",
        );
        assert_eq!(m.global(0), 0, "routine must not run while the count is above zero");
    }

    // Test (SQ-0585): text DIVERTED to a secondary prose window's own buffer must
    // still have every side effect the game can observe. Routing decides only where
    // the text is displayed — ZMSD §8.8.3.2.2's new-line countdown (prop 9, the
    // [MORE]/interrupt mechanism) and the window cursor (props 4/5) belong to the
    // window that was printed to, whichever surface the host shows it on.
    #[test]
    fn diverted_prose_still_ticks_the_newline_interrupt_and_cursor() {
        let mut m = Machine::new(
            Memory::new(crate::header::tests_support::sample_story(6)).unwrap(),
        );
        // Window 3: wrapping+scrolling (a prose window) with attribute 2 CLEAR —
        // ZMSD §8.8.3.1's "text copied to output stream 2" — which is how a game
        // says this window's text is not the transcript's (advent.z6's `style`).
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.current = 3;
            let w = &mut v6.windows[3];
            w.attributes = 0b1011; // wrap + scroll + buffered, NOT copy-to-transcript
            w.y_size = 200;
            w.x_size = 640;
            w.y_cursor = 1;
            w.x_cursor = 1;
            w.interrupt_countdown = 3;
            w.interrupt_routine = 0xA0;
        }
        m.v6_input_window = 7; // the player types elsewhere
        m.print_text("hello\n");

        let w = &m.screen.v6.as_ref().unwrap().windows[3];
        assert_eq!(w.prose, vec!["hello".to_string(), String::new()], "the text went to the window");
        assert_eq!(w.interrupt_countdown, 2, "the new-line still counted down (ZMSD 8.8.3.2.2)");
        assert!(w.y_cursor > 1, "the window cursor still advanced (props 4/5), got {}", w.y_cursor);
        assert_eq!(m.global(0), 0, "no routine fires above zero");
    }

    // Test (SQ-0585): the same window WITH attribute 2 set streams to the transcript
    // as before — the game marks the narrative window, and nothing is diverted from
    // it even when the player is typing elsewhere.
    #[test]
    fn a_copy_to_transcript_window_is_never_diverted() {
        let mut m = Machine::new(
            Memory::new(crate::header::tests_support::sample_story(6)).unwrap(),
        );
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.current = 7;
            let w = &mut v6.windows[7];
            w.attributes = 0b1111; // wrap + scroll + copy-to-transcript + buffered
            w.y_size = 200;
            w.x_size = 640;
        }
        m.v6_input_window = 0; // e.g. before the first read, at boot
        m.print_text("banner\n");
        assert!(
            m.screen.v6.as_ref().unwrap().windows[7].prose.is_empty(),
            "a transcript-marked window streams, so nothing lands in its buffer"
        );
    }

    // Test: when the countdown hits zero the prop-8 routine fires exactly once,
    // synchronously, returning cleanly to the caller's frame.
    #[test]
    fn newline_interrupt_fires_at_zero() {
        // main: new_line (0xBB) then quit (0xBA).
        // routine: inc G0 (0x95 0x10) then rtrue (0xB0).
        let mut m = Machine::new(
            Memory::new(v6_newline_story(&[0xBB, 0xBA], &[0x95, 0x10, 0xB0])).unwrap(),
        );
        {
            let w = &mut m.screen.v6.as_mut().unwrap().windows[0];
            w.interrupt_countdown = 1; // fire after this one new-line
            w.interrupt_routine = 0xA0;
        }
        assert_eq!(m.global(0), 0);
        // Step main's new_line: prints "\n" to window 0 → countdown 1→0 → fire.
        assert_eq!(m.step(), StepResult::Continue);
        assert_eq!(m.global(0), 1, "interrupt routine ran exactly once");
        assert_eq!(
            m.screen.v6.as_ref().unwrap().windows[0].interrupt_countdown, 0,
            "the fired countdown stays at zero (Frotz's re-fire guard)",
        );
        assert_eq!(m.state.frames.len(), 1, "routine returned cleanly to main's frame");
    }

    // Test: re-entrancy guard. The routine re-arms prop 9 to 1 AND emits a
    // new-line of its own; without the guard that new-line would re-fire the
    // interrupt forever. It must run once and leave its re-armed count intact.
    #[test]
    fn newline_interrupt_guards_reentrancy() {
        // routine: put_wind_prop(win0, prop9, 1) [BE 19 57 00 09 01],
        //          new_line [BB], inc G0 [95 10], rtrue [B0].
        let rout = [0xBE, 0x19, 0x57, 0x00, 0x09, 0x01, 0xBB, 0x95, 0x10, 0xB0];
        let mut m = Machine::new(
            Memory::new(v6_newline_story(&[0xBB, 0xBA], &rout)).unwrap(),
        );
        {
            let w = &mut m.screen.v6.as_mut().unwrap().windows[0];
            w.interrupt_countdown = 1;
            w.interrupt_routine = 0xA0;
        }
        // Fires via main's new_line; must terminate (no unbounded recursion).
        assert_eq!(m.step(), StepResult::Continue);
        assert_eq!(m.global(0), 1, "routine ran exactly once despite emitting a new-line");
        assert_eq!(
            m.screen.v6.as_ref().unwrap().windows[0].interrupt_countdown, 1,
            "the routine's own new-line is ignored while it runs (re-entrancy guarded)",
        );
        assert!(!m.newline_interrupt_active, "active flag cleared after the routine returns");
    }

    // Test: writing prop 9 via put_wind_prop (EXT:0x19) arms and later resets the
    // running countdown.
    #[test]
    fn put_wind_prop_9_arms_and_resets_countdown() {
        let mut m = Machine::new(
            Memory::new(crate::header::tests_support::sample_story(6)).unwrap(),
        );
        // put_wind_prop(win0, prop9, 5) arms the countdown.
        m.exec_ext(0x19, &[0, 9, 5], None, None);
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[0].interrupt_countdown, 5);
        // One new-line decrements it.
        m.print_text("x\n");
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[0].interrupt_countdown, 4);
        // A fresh prop-9 write resets the running count.
        m.exec_ext(0x19, &[0, 9, 2], None, None);
        assert_eq!(
            m.screen.v6.as_ref().unwrap().windows[0].interrupt_countdown, 2,
            "put_wind_prop(9, ..) resets the count",
        );
    }

    // -----------------------------------------------------------------------
    // Test: abort_timed_input(read_char) stores ZSCII 0 and clears pending
    // -----------------------------------------------------------------------
    #[test]
    fn abort_timed_input_read_char_stores_zero() {
        // v5 read_char at 0x10 -> NeedChar; abort stores 0 in the store var (G0).
        let mut buf = sample_story(5);
        buf[0x0010] = 0xF6; // VAR read_char
        buf[0x0011] = 0x7F; // type: small(01), omit, omit, omit
        buf[0x0012] = 1;    // operand: device=1 (keyboard)
        buf[0x0013] = 0x10; // store -> G0
        buf[0x0014] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;
        assert_eq!(m.step(), StepResult::NeedChar);
        m.abort_timed_input("");
        assert_eq!(m.global(0), 0, "aborted read_char stores 0");
        assert!(m.pending_timeout().is_none(), "pending cleared after abort");
    }

    // -----------------------------------------------------------------------
    // Test: abort_timed_input(read) writes the partial line + count and
    // stores terminator 0 (v5+)
    // -----------------------------------------------------------------------
    #[test]
    fn abort_timed_input_read_writes_partial_and_terminator_zero() {
        // v5 read at 0x10 with time/routine (timed_read_story); abort writes
        // the partial buffer and stores terminator 0. Routine body is
        // irrelevant here (never run).
        let (mut buf, _rout) = timed_read_story(&[0xB0]);
        // text_buf is 0x0250 (see timed_read_story); byte 0 = max chars.
        buf[0x0250] = 20;
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        assert!(matches!(m.step(), StepResult::NeedLine { .. }));
        m.abort_timed_input("no");
        // v5 text buffer: byte0=max, byte1=count, text from byte2.
        assert_eq!(m.mem.read_byte(0x0251), 2, "count = len('no')");
        assert_eq!(m.mem.read_byte(0x0252), b'n');
        assert_eq!(m.mem.read_byte(0x0253), b'o');
        assert_eq!(m.global(0), 0, "terminator stored is 0");
        assert!(m.pending_timeout().is_none(), "pending cleared after abort");
    }

    // -----------------------------------------------------------------------
    // Test: input is lower-cased before writing to text buffer
    // -----------------------------------------------------------------------
    #[test]
    fn read_lower_cases_input() {
        let (mut buf, _addr_north, _addr_open, _addr_mailbox) = build_input_story(3);

        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 20;
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 8;

        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 3, None);
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;
        m.step();
        m.supply_line("NORTH", 13);

        let tb = text_buf as u32;
        assert_eq!(m.mem.read_byte(tb + 1), b'n', "upper N lower-cased to 'n'");
        assert_eq!(m.mem.read_byte(tb + 5), b'h', "upper H lower-cased to 'h'");
    }

    /// Test: `print_ret` prints inline string + newline + returns true (store var gets 1).
    ///
    /// To test print_ret properly we need a routine that calls another routine
    /// containing print_ret. print_ret returns 1 to the caller; the caller stores
    /// the return value in G0. We verify G0=1 and output ends with "\n".
    #[test]
    fn text_print_ret_returns_true() {
        // Routine at 0x80 (v5 packed addr 0x80/4 = 0x20):
        //   0x80: local_count=0
        //   0x81: print_ret "hi"
        //     0xB3 (0OP opcode 0x03 = print_ret)
        //     Z-encode "hi": h=A1-idx(2)=Z4+Z8+Z13... actually h in A1: A1="ABCDEFGHIJKLMNOPQRSTUVWXYZ^0123456789._,!?_#'"
        //     Let me use a simple 3-char encodable string instead: use Z-char padding.
        //     Simpler: Z-chars for "hi": shift-A1(4), h(A1-idx=7=Z13)... actually A1 index 7 = 'H'.
        //     Even simpler: use 3 pad Z-chars (all 5=shift) → empty string output, but test structure.
        //     Let me use "ab": a(A0,Z6), b(A0,Z7), pad(Z5) → word = 0x8000|(6<<10)|(7<<5)|5 = 0x99C5
        let mut buf = sample_story(5);

        // Routine at 0x80
        buf[0x80] = 0; // local count
        // print_ret: 0xB3 + inline text "ab" (Z-chars 6,7,5)
        buf[0x81] = 0xB3;
        let ab_word: u16 = 0x8000 | (6u16 << 10) | (7u16 << 5) | 5u16;
        buf[0x82] = (ab_word >> 8) as u8;
        buf[0x83] = (ab_word & 0xFF) as u8;
        // No explicit quit needed — print_ret returns to caller

        // Main at 0x10: call_vs packed=0x0020 → G0, then quit
        buf[0x10] = 0xE0;
        buf[0x11] = 0b00_11_11_11; // type: large, omit, omit, omit
        buf[0x12] = 0x00;
        buf[0x13] = 0x20;
        buf[0x14] = 0x10; // store → G0
        buf[0x15] = 0xBA; // quit

        let mut m = build_raw_machine(buf);
        run_until_quit(&mut m);

        assert_eq!(m.global(0), 1, "print_ret should return true (1) to caller");
        let out = m.buffer_output().expect("default sink");
        assert!(out.buf.ends_with('\n'), "print_ret output must end with newline");
        assert!(out.buf.starts_with("ab"), "print_ret output starts with the inline string");
    }

    // -----------------------------------------------------------------------
    // Task 13: screen model, output stream 3, window opcodes
    // -----------------------------------------------------------------------

    /// Helper: emit a VAR-form instruction with up to 4 small-const operands.
    /// `opcode` is the VAR opcode number (0x00–0x1F).
    fn emit_var_instr(buf: &mut Vec<u8>, opcode: u8, ops: &[u8]) {
        // VAR form: 0b11_1_xxxxx
        buf.push(0b1110_0000 | (opcode & 0x1F));
        // Type byte: each op = small-const (0b01), unused = omit (0b11).
        let mut type_byte: u8 = 0xFF;
        for (i, _) in ops.iter().enumerate().take(4) {
            let shift = 6u8.saturating_sub(2 * i as u8);
            type_byte &= !(0b11 << shift);
            type_byte |= 0b01 << shift; // small const
        }
        buf.push(type_byte);
        for &op in ops.iter().take(4) {
            buf.push(op);
        }
    }

    /// Emit an EXT instruction (0xBE prefix) with all operands as Large (16-bit) constants.
    fn emit_ext_instr(buf: &mut Vec<u8>, opcode: u8, ops: &[u16]) {
        buf.push(0xBE); // EXT prefix
        buf.push(opcode);
        // Type byte: each op = large-const (0b00), unused = omit (0b11).
        let mut type_byte: u8 = 0xFF;
        for (i, _) in ops.iter().enumerate().take(4) {
            let shift = 6u8.saturating_sub(2 * i as u8);
            type_byte &= !(0b11 << shift); // clear to 0b00 = large const
        }
        buf.push(type_byte);
        for &op in ops.iter().take(4) {
            buf.push((op >> 8) as u8);
            buf.push((op & 0xFF) as u8);
        }
    }

    /// Emit VAR output_stream (0x13) with a large-const signed stream number.
    /// Z-machine signed stream numbers (e.g. -1, -3) require 16-bit large constants.
    fn emit_output_stream_large(buf: &mut Vec<u8>, stream_val: i16) {
        let v = stream_val as u16;
        buf.push(0b1110_0000 | 0x13);  // VAR:0x13
        // Type byte: first=large(0b00), rest=omit(0b11) → 0b00_11_11_11 = 0x3F
        buf.push(0x3F);
        buf.push((v >> 8) as u8);
        buf.push((v & 0xFF) as u8);
    }

    /// Emit output_stream +3 with a large-const table address.
    /// stream=3 (small const), table_addr (large const).
    /// type_byte: first=small(01), second=large(00), rest=omit(11) → 0b01_00_11_11 = 0x4F
    fn emit_output_stream3_on(buf: &mut Vec<u8>, table_addr: u16) {
        buf.push(0b1110_0000 | 0x13); // VAR:0x13
        // Type byte: op0=small(01), op1=large(00), rest=omit(11)
        buf.push(0b01_00_11_11);      // 0x4F
        buf.push(3u8);                // stream number = 3 (small const)
        buf.push((table_addr >> 8) as u8);
        buf.push((table_addr & 0xFF) as u8);
    }

    // ── (a) set_text_style and split_window update ScreenState ───────────────

    #[test]
    fn screen_set_text_style_and_split_window() {
        // Program at 0x10 (v5):
        //   set_text_style 1  (reverse) → screen.text_style = 1
        //   split_window  3           → screen.upper_window_rows = 3
        //   set_window    1           → screen.current_window = 1
        //   quit
        //
        // set_text_style = VAR:0x11
        // split_window   = VAR:0x0A
        // set_window     = VAR:0x0B
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;

        // set_text_style 1: VAR:0x11, small 1
        let instr = {let mut v = vec![]; emit_var_instr(&mut v, 0x11, &[1]); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // split_window 3: VAR:0x0A, small 3
        let instr = {let mut v = vec![]; emit_var_instr(&mut v, 0x0A, &[3]); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // set_window 1: VAR:0x0B, small 1
        let instr = {let mut v = vec![]; emit_var_instr(&mut v, 0x0B, &[1]); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        assert_eq!(m.screen.text_style, 1, "set_text_style(1) → text_style=1");
        assert_eq!(m.screen.upper_window_rows, 3, "split_window(3) → upper_window_rows=3");
        assert_eq!(m.screen.current_window, 1, "set_window(1) → current_window=1");
    }

    // ── (a2) set_text_style is cumulative: nonzero OR-s in, 0 resets ──────────
    // ZMSD §8.7.1: styles combine; only Roman (0) clears all. BeyondZork's
    // character menus rely on this — the reverse-video box stays reversed while
    // an additional style (fixed-pitch) is layered onto a line. Replace
    // semantics would wipe the reverse bit and break selection highlighting.
    #[test]
    fn screen_set_text_style_is_cumulative() {
        // v5 program:
        //   set_text_style 1  (reverse)        → 1
        //   set_text_style 8  (fixed, OR-ed)   → 9
        //   set_text_style 0  (Roman, resets)  → 0
        //   set_text_style 2  (bold)           → 2
        //   quit
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;
        for operand in [1u8, 8, 0, 2] {
            let instr = {
                let mut v = vec![];
                emit_var_instr(&mut v, 0x11, &[operand]);
                v
            };
            buf[pos..pos + instr.len()].copy_from_slice(&instr);
            pos += instr.len();
        }
        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;

        // Step through and check the bitmask after each set_text_style.
        // After 1 → 1; after 8 → 9 (1|8); after 0 → 0; after 2 → 2.
        m.step(); // set_text_style 1
        assert_eq!(m.screen.text_style, 1, "after set_text_style(1): reverse");
        m.step(); // set_text_style 8
        assert_eq!(m.screen.text_style, 9, "after set_text_style(8): reverse|fixed (cumulative)");
        m.step(); // set_text_style 0
        assert_eq!(m.screen.text_style, 0, "after set_text_style(0): Roman resets all");
        m.step(); // set_text_style 2
        assert_eq!(m.screen.text_style, 2, "after set_text_style(2): bold");
    }

    // ── (b) show_status (v3 0OP:0x0C) sets the flag ─────────────────────────

    #[test]
    fn screen_show_status_v3_sets_flag() {
        // v3 program: show_status (0xBC), quit
        let mut buf = sample_story(3);
        buf[0x10] = 0xBC; // 0OP:0x0C show_status
        buf[0x11] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert!(m.screen.show_status_requested, "show_status should set the flag");
    }

    // ── (c) output_stream 3: text goes to memory table, NOT screen ───────────

    #[test]
    fn output_stream3_redirects_text_to_table() {
        // Program at 0x10 (v5):
        //   output_stream +3 table_addr    → select stream 3
        //   print "ab"                     → goes to table, NOT screen
        //   output_stream -3               → deselect stream 3
        //   print "cd"                     → goes to screen (stream 1)
        //   quit
        //
        // Table at 0x0060 (inside dynamic memory, safely below 0x0400).
        // "ab" Z-encoded: a=Z6, b=Z7, pad=Z5 → word = 0x8000|(6<<10)|(7<<5)|5 = 0x99C5
        // "cd" Z-encoded: c=Z8, d=Z9, pad=Z5 → word = 0x8000|(8<<10)|(9<<5)|5

        let table_addr: u16 = 0x0060;
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;

        // output_stream +3, table_addr
        let instr = {let mut v = vec![]; emit_output_stream3_on(&mut v, table_addr); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print "ab" (0OP:0x02 inline)
        let ab_word: u16 = 0x8000 | (6u16 << 10) | (7u16 << 5) | 5u16;
        buf[pos] = 0xB2; pos += 1; // 0OP print
        buf[pos] = (ab_word >> 8) as u8; pos += 1;
        buf[pos] = (ab_word & 0xFF) as u8; pos += 1;

        // output_stream -3 (deselect stream 3)
        let instr = {let mut v = vec![]; emit_output_stream_large(&mut v, -3); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print "cd" (0OP:0x02 inline)
        let cd_word: u16 = 0x8000 | (8u16 << 10) | (9u16 << 5) | 5u16;
        buf[pos] = 0xB2; pos += 1;
        buf[pos] = (cd_word >> 8) as u8; pos += 1;
        buf[pos] = (cd_word & 0xFF) as u8; pos += 1;

        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        // Screen should only have "cd" (stream 1).
        let screen_text = &m.buffer_output().expect("BufferOutput").buf;
        assert_eq!(screen_text.as_str(), "cd", "screen should only receive 'cd' (not 'ab')");

        // Table at 0x0060: word=length=2, then 'a','b'
        assert_eq!(m.mem.read_word(table_addr as u32), 2, "table length word = 2");
        assert_eq!(m.mem.read_byte(table_addr as u32 + 2), b'a', "table[0] = 'a'");
        assert_eq!(m.mem.read_byte(table_addr as u32 + 3), b'b', "table[1] = 'b'");
    }

    #[test]
    fn output_stream3_stores_single_zscii_byte_for_high_char() {
        // print_char with a high ZSCII operand (195 = 'û' via the default
        // Unicode table) must store exactly ONE byte in the stream-3 table,
        // not the multi-byte UTF-8 encoding of 'û' (SQ-0240).
        let table_addr: u32 = 0x0060;
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.streams.push_stream3(table_addr, None);
        m.exec_var(0x05, &[195], None, None);
        m.streams.pop_stream3(&mut m.mem);

        assert_eq!(m.mem.read_word(table_addr), 1, "length word should be 1, not the UTF-8 byte count");
        assert_eq!(m.mem.read_byte(table_addr + 2), 195);
    }

    #[test]
    fn print_char_zscii10_stored_verbatim_in_stream3() {
        // ZSCII 10 is a DISPLAY-only hack in zscii_to_char (renders as a space,
        // code 32). Stream 3 must store the verbatim print_char operand (10),
        // not the round-tripped display value (SQ-0247).
        let table_addr: u32 = 0x0060;
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.streams.push_stream3(table_addr, None);
        m.exec_var(0x05, &[10], None, None);
        m.streams.pop_stream3(&mut m.mem);

        assert_eq!(m.mem.read_word(table_addr), 1, "length word should be 1");
        assert_eq!(m.mem.read_byte(table_addr + 2), 10, "stream 3 must store verbatim ZSCII 10, not 32");
    }

    #[test]
    fn print_char_high_zscii_verbatim_in_stream3() {
        // Regression: the common high-ZSCII case (already covered by
        // output_stream3_stores_single_zscii_byte_for_high_char) must still
        // store the verbatim byte after routing print_char's stream-3 path
        // directly through write_stream3_bytes instead of print_text.
        let table_addr: u32 = 0x0060;
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.streams.push_stream3(table_addr, None);
        m.exec_var(0x05, &[195], None, None);
        m.streams.pop_stream3(&mut m.mem);

        assert_eq!(m.mem.read_word(table_addr), 1, "length word should be 1");
        assert_eq!(m.mem.read_byte(table_addr + 2), 195);
    }

    // ── (d) stream 1 off: screen receives nothing ─────────────────────────────

    #[test]
    fn output_stream2_warns_the_player_once() {
        // ZMSD §7.6.5.2: "An attempt by the game to use streams to access
        // external files which is not supported by the interpreter should
        // ideally print a warning to the user that the functionality is not
        // available, and otherwise do nothing." The host renders diagnostics as
        // Warning transcript lines.
        let mut m = build_test_machine(&[]);
        m.exec_var(0x13, &[2], None, None); // output_stream 2
        assert_eq!(m.diagnostics.len(), 1, "one warning: {:?}", m.diagnostics);
        assert!(
            m.diagnostics[0].contains("transcript file"),
            "warning names the missing feature: {:?}", m.diagnostics[0]
        );
        assert!(m.streams.stream2, "the selection itself is still recorded");

        // Games re-select the transcript every turn — warn once per session.
        m.exec_var(0x13, &[(-2i16) as u16], None, None);
        m.exec_var(0x13, &[2], None, None);
        assert_eq!(m.diagnostics.len(), 1, "no repeat warning: {:?}", m.diagnostics);
    }

    #[test]
    fn output_stream1_off_suppresses_screen() {
        // output_stream -1 (disable screen), print "x", output_stream +1, print "y", quit
        // Screen should only have "y".
        let mut buf = sample_story(5);
        let mut pos: usize = 0x10;

        // output_stream -1
        let instr = {let mut v = vec![]; emit_output_stream_large(&mut v, -1); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print "x": Z-encode x (A0: z=Z31; x is... wait let's use print_char)
        // print_char ZSCII 120 = 'x': VAR:0x05
        buf[pos] = 0xE5; pos += 1;     // VAR print_char
        buf[pos] = 0x7F; pos += 1;     // type: small, omit, omit, omit
        buf[pos] = 120u8; pos += 1;    // 'x' = ZSCII 120

        // output_stream +1
        let instr = {let mut v = vec![]; emit_output_stream_large(&mut v, 1); v};
        buf[pos..pos+instr.len()].copy_from_slice(&instr); pos += instr.len();

        // print_char 'y' = 121
        buf[pos] = 0xE5; pos += 1;
        buf[pos] = 0x7F; pos += 1;
        buf[pos] = 121u8; pos += 1;    // 'y'

        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);

        let out = m.buffer_output().expect("BufferOutput");
        assert_eq!(out.buf, "y", "only 'y' reaches screen (stream 1 was off for 'x')");
    }

    // ── (e) Machine::init_caps sets header bits correctly ────────────────────

    #[test]
    fn machine_init_caps_sets_header_bits() {
        // Build a machine on a story where the initial_pc is past 0x40
        // (so there's no program at 0x10 that conflicts with Flags2).
        // sample_story sets initial_pc = 0x0040 and programs at 0x40+.
        // But we need programs at a safe location. Let's use 0x80 as initial_pc.
        let mut buf = sample_story(5);
        // Place quit at 0x80 so the machine doesn't crash.
        buf[0x80] = 0xBA;
        // Override initial_pc to 0x0080 in the header.
        buf[0x06] = 0x00;
        buf[0x07] = 0x80;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.init_caps();

        // Check that Flags1 has fixed-space font bit set (bit 4 for v5).
        let f1 = m.mem.read_byte(0x01);
        assert_ne!(f1 & (1 << 4), 0, "Flags1 bit 4 (fixed-space font) should be set");

        // Interpreter number and version.
        assert_eq!(m.mem.read_byte(0x1E), 1, "interpreter number defaults to DEC-20 (1)");
        assert_eq!(m.mem.read_byte(0x1F), b'A', "interpreter version = 'A'");
    }

    #[test]
    fn set_sound_available_advertises_and_clears() {
        let mut buf = sample_story(5);
        buf[0x80] = 0xBA;                 // quit at 0x80
        buf[0x06] = 0x00; buf[0x07] = 0x80; // initial_pc = 0x0080
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);

        m.set_sound_available(true);
        let f1 = m.mem.read_byte(0x01);
        let f2 = m.mem.read_word(0x10);
        assert_ne!(f1 & (1 << 5), 0, "Flags1 bit 5 (sound) should be set");
        assert_ne!(f2 & (1 << 7), 0, "Flags2 bit 7 (sound) should be set");

        m.set_sound_available(false);
        let f1 = m.mem.read_byte(0x01);
        let f2 = m.mem.read_word(0x10);
        assert_eq!(f1 & (1 << 5), 0, "Flags1 bit 5 (sound) should be clear");
        assert_eq!(f2 & (1 << 7), 0, "Flags2 bit 7 (sound) should be clear");
    }

    #[test]
    fn init_caps_forwards_sound_available_field() {
        let mut buf = sample_story(5);
        buf[0x80] = 0xBA;                 // quit at 0x80
        buf[0x06] = 0x00; buf[0x07] = 0x80; // initial_pc = 0x0080
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.sound_available = true;
        m.init_caps();

        let f1 = m.mem.read_byte(0x01);
        let f2 = m.mem.read_word(0x10);
        assert_ne!(f1 & (1 << 5), 0, "Flags1 bit 5 (sound) should be set via init_caps");
        assert_ne!(f2 & (1 << 7), 0, "Flags2 bit 7 (sound) should be set via init_caps");
    }

    #[test]
    fn set_default_colours_publishes_host_defaults_in_the_header() {
        // ZMSD §8.3.3: the interpreter writes ITS default background ($2C) and
        // foreground ($2D) into the header, so games can build a palette from
        // the colours the host actually paints in.
        let mut m = Machine::new(
            Memory::new(crate::header::tests_support::sample_story(5)).unwrap(),
        );
        m.init_caps();
        assert_eq!(
            (m.mem.read_byte(0x2C), m.mem.read_byte(0x2D)),
            (2, 9),
            "unset: black-on-white is the boot-time seed"
        );
        // Callable after boot: the bytes land immediately.
        m.set_default_colours(6, 5); // blue background, yellow foreground
        assert_eq!((m.mem.read_byte(0x2C), m.mem.read_byte(0x2D)), (6, 5));
        // ...and survive a re-init (which is what @restart performs).
        m.init_caps();
        assert_eq!((m.mem.read_byte(0x2C), m.mem.read_byte(0x2D)), (6, 5), "kept across init_caps");
        // Anything outside the §8.3.1 standard colours 2..=9 falls back to 2/9.
        m.set_default_colours(0, 15);
        assert_eq!((m.mem.read_byte(0x2C), m.mem.read_byte(0x2D)), (2, 9));
        assert_eq!((m.default_bg_colour, m.default_fg_colour), (2, 9), "the stored pair is clamped too");
    }

    #[test]
    fn set_interpreter_number_overrides_at_init_caps() {
        let mut buf = sample_story(5);
        buf[0x80] = 0xBA;                 // quit at 0x80
        buf[0x06] = 0x00; buf[0x07] = 0x80; // initial_pc = 0x0080
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.set_interpreter_number(Some(4)); // Amiga
        m.init_caps();
        assert_eq!(m.mem.read_byte(0x1E), 4, "override advertised");
    }

    // -----------------------------------------------------------------------
    // Test: v4+ restore-failure stores 0 into the correct store variable
    // and does not corrupt state.pc.
    //
    // Program layout (v5 story at 0x10):
    //   0x10: 0OP restore (0x06), store byte = 0x10 (global 0)
    //         Encoded as short 0OP: 0xB6, then store byte 0x10
    //         → step() decodes this, captures store=G0, sets state.pc=0x12,
    //           then returns RestoreRequest.
    //   0x12: quit (0xBA)
    //
    // After complete_restore_failure():
    //   global(0) == 0  (failure result stored into G0)
    //   state.pc  == 0x12 (unchanged — points to quit)
    // -----------------------------------------------------------------------

    #[test]
    fn restore_failure_stores_zero_into_correct_var_and_pc_unchanged() {
        let mut buf = sample_story(5);
        // restore opcode: short 0OP form = 0xB6, followed by store byte
        buf[0x10] = 0xB6; // 0OP:0x06 restore
        buf[0x11] = 0x10; // store → global 0 (var 0x10)
        buf[0x12] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;

        // Pre-condition: global 0 is 0 (default), but set it to a non-zero sentinel
        // so we can prove it was written by complete_restore_failure.
        let base = m.mem.global_vars() as u32;
        m.mem.write_word(base, 0xABCD); // G0 = 0xABCD (sentinel)

        // Execute the restore instruction.
        let result = m.step();
        assert_eq!(result, StepResult::RestoreRequest, "restore opcode must return RestoreRequest");

        // After step(): pc must be 0x12 (past the store byte).
        assert_eq!(m.state.pc, 0x12, "state.pc must point to instruction after restore (0x12)");

        // Simulate restore failure (no save data).
        m.complete_restore_failure();

        // G0 must now be 0 (failure result).
        assert_eq!(m.global(0), 0, "restore failure must store 0 into the store variable (G0)");

        // state.pc must still be 0x12 — complete_restore_failure must not advance pc.
        assert_eq!(m.state.pc, 0x12, "state.pc must not be corrupted by complete_restore_failure");
    }

    // -----------------------------------------------------------------------
    // loadw / loadb / storew / storeb round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn loadw_storew_round_trip() {
        // storew G0 #0 G1  — store G1 at dynamic_mem[G0 + 0]
        // loadw G0 #0 → G2  — read back from same address
        //
        // Hand-assembled bytes:
        //   storew: VAR:01, type_byte=[Var,Small,Var,omit], G0, 0, G1
        //     0xC1 (VAR bit5=0=2OP? No, VAR:01 with bit5=1=Var)
        //     Wait: VAR:01 opcode byte = 0b11_1_00001 = 0xE1, type_byte=0b10_01_10_11=0xAB
        //
        // Actually easier to set base address to a known dynamic address (e.g. 0x40)
        // and use raw bytes.
        //
        // storew: 0xE1 (VAR:01), type=0xAB([Var,Small,Var,omit]), G0, 0, G1
        // loadw:  VAR:0F = 0b11_1_01111 = 0xEF, type=0b10_01_11_11=0x9F, G0, 0, store=G2
        let mut buf = sample_story(5);
        // Set up globals: G0=0x40 (base address in dynamic mem), G1=0xBEEF (value to store)
        let gbase = {
            let tmp = Memory::new(buf.clone()).unwrap();
            tmp.global_vars() as usize
        };
        // G0 = 0x40
        buf[gbase]     = 0x00;
        buf[gbase + 1] = 0x40;
        // G1 = 0xBEEF
        buf[gbase + 2] = 0xBE;
        buf[gbase + 3] = 0xEF;

        // storew G0 #0 G1:  E1 AB 10 00 11
        buf[0x10] = 0xE1; // VAR:01 storew
        buf[0x11] = 0xAB; // type: [Var=10, Small=01, Var=10, omit=11]
        buf[0x12] = 0x10; // G0 (var 0x10)
        buf[0x13] = 0x00; // index 0
        buf[0x14] = 0x11; // G1 (var 0x11)
        // loadw G0 #0 → G2:  EF 9F 10 00 12
        buf[0x15] = 0xEF; // VAR:0F (but 0xEF with bit5=1=Var, opcode=0x0F=15)
        // Wait: 0xEF = 0b11_1_01111: VAR form, bit5=1→Var, opcode=0x0F=loadw
        // but loadw is 2OP:0x0F. In VAR form with bit5=0→Two, 0xCF would be loadw.
        // 0xEF has bit5=1→Var, so that's VAR:0x0F (not 2OP). But loadw is 2OP!
        // Use Long form instead: 0x4F (bit6=1=Var, bit5=0=Small, op=0x0F=loadw)
        //   Long: 0b01_0_01111 = 0x4F, G0, 0, store=G2
        buf[0x15] = 0x4F; // long form: Var, Small, opcode=0x0F=loadw
        buf[0x16] = 0x10; // G0
        buf[0x17] = 0x00; // index 0
        buf[0x18] = 0x12; // store → G2
        buf[0x19] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(2), 0xBEEF, "loadw round-trip: should read back 0xBEEF");
    }

    #[test]
    fn loadb_storeb_round_trip() {
        // storeb base #0 val  — write byte at base+0
        // loadb base #0 → G2  — read back the byte
        let mut buf = sample_story(5);
        let gbase = {
            let tmp = Memory::new(buf.clone()).unwrap();
            tmp.global_vars() as usize
        };
        // G0 = 0x40 (base)
        buf[gbase]     = 0x00;
        buf[gbase + 1] = 0x40;
        // G1 = 0x42 (byte value to store)
        buf[gbase + 2] = 0x00;
        buf[gbase + 3] = 0x42;

        // storeb G0 #0 G1:  E2 AB 10 00 11  (VAR:02)
        buf[0x10] = 0xE2; // VAR:02 storeb
        buf[0x11] = 0xAB; // [Var, Small, Var, omit]
        buf[0x12] = 0x10;
        buf[0x13] = 0x00;
        buf[0x14] = 0x11;
        // loadb G0 #0 → G2:  Long form 0x50, Var G0, Small 0, store G2
        //   long: bit6=1(var), bit5=0(small), opcode=0x10=loadb → 0b01_0_10000=0x50
        buf[0x15] = 0x50;
        buf[0x16] = 0x10;
        buf[0x17] = 0x00;
        buf[0x18] = 0x12;
        buf[0x19] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(2), 0x42, "loadb round-trip: should read back 0x42");
    }

    // -----------------------------------------------------------------------
    // random — result must be in [1, range] for positive range
    // -----------------------------------------------------------------------

    #[test]
    fn random_in_range() {
        // random #10 → G0; quit
        // VAR:07 random: 0xE7 (bit5=1→Var, op=7), type_byte=0x7F([Small,omit,omit,omit]),
        //   operand=10, store=G0(0x10)
        let mut buf = sample_story(5);
        buf[0x10] = 0xE7; // VAR:07 random
        buf[0x11] = 0x7F; // type: [Small, omit, omit, omit]
        buf[0x12] = 10;   // range = 10
        buf[0x13] = 0x10; // store → G0
        buf[0x14] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        let result = m.global(0);
        assert!((1..=10).contains(&result),
            "random(10) must be in [1,10], got {result}");
    }

    // -----------------------------------------------------------------------
    // log_shift / art_shift (EXT opcodes)
    // -----------------------------------------------------------------------

    #[test]
    fn log_shift_left_and_right() {
        // log_shift value places → store
        // EXT:02 format: 0xBE 0x02 type_byte [operands] store
        //
        // Type byte encoding (2-bit fields, MSB-first):
        //   00=Large(16-bit), 01=Small(8-bit), 10=Variable, 11=omit
        // For [Small, Small, omit, omit]: 0b01_01_11_11 = 0x5F
        // For [Small, Large, omit, omit]: 0b01_00_11_11 = 0x4F (Large places)
        //
        // Test 1: log_shift 8 places=2 → G0  (8u << 2 = 32)
        // Test 2: log_shift 8 places=-1 → G1 (8u >> 1 = 4, unsigned shift)
        //   places=-1 requires Large constant 0xFFFF (Small only covers 0..255)
        let mut buf = sample_story(5);
        let mut pc = 0x10usize;

        // EXT:02, [Small value=8, Small places=2, omit, omit], store=G0
        buf[pc] = 0xBE; buf[pc+1] = 0x02; buf[pc+2] = 0x5F; // type: [S,S,_,_]
        buf[pc+3] = 8; buf[pc+4] = 2; buf[pc+5] = 0x10; pc += 6;

        // EXT:02, [Small value=8, Large places=0xFFFF(-1), omit, omit], store=G1
        buf[pc] = 0xBE; buf[pc+1] = 0x02; buf[pc+2] = 0x4F; // type: [S,L,_,_]
        buf[pc+3] = 8; buf[pc+4] = 0xFF; buf[pc+5] = 0xFF; buf[pc+6] = 0x11; pc += 7;

        buf[pc] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 32, "log_shift 8 << 2 = 32");
        assert_eq!(m.global(1), 4, "log_shift 8 >> 1 = 4 (unsigned)");
    }

    #[test]
    fn art_shift_preserves_sign() {
        // art_shift value places → store
        // EXT:03 format: 0xBE 0x03 type_byte [operands] store
        //
        // Test 1: art_shift 0xFFF8 places=-1 → G0  (-8 >> 1 = -4 = 0xFFFC, sign-extended)
        // Test 2: art_shift 0xFFF8 places=2  → G1  (-8 << 2 = -32 = 0xFFE0)
        // Both need Large(0xFFF8) and either Large(0xFFFF=-1) or Small(2).
        // Type [Large, Large, omit, omit]: 0b00_00_11_11 = 0x0F
        // Type [Large, Small, omit, omit]: 0b00_01_11_11 = 0x1F
        let mut buf = sample_story(5);
        let mut pc = 0x10usize;

        // EXT:03, [Large value=0xFFF8, Large places=0xFFFF(-1), omit, omit], store=G0
        buf[pc] = 0xBE; buf[pc+1] = 0x03; buf[pc+2] = 0x0F; // type: [L,L,_,_]
        buf[pc+3] = 0xFF; buf[pc+4] = 0xF8; // value = 0xFFF8
        buf[pc+5] = 0xFF; buf[pc+6] = 0xFF; // places = 0xFFFF = -1
        buf[pc+7] = 0x10; // store → G0
        pc += 8;

        // EXT:03, [Large value=0xFFF8, Small places=2, omit, omit], store=G1
        buf[pc] = 0xBE; buf[pc+1] = 0x03; buf[pc+2] = 0x1F; // type: [L,S,_,_]
        buf[pc+3] = 0xFF; buf[pc+4] = 0xF8; // value = 0xFFF8
        buf[pc+5] = 0x02; // places = 2
        buf[pc+6] = 0x11; // store → G1
        pc += 7;

        buf[pc] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        // 0xFFF8 = -8 signed. arithmetic right-shift by 1 = -4 = 0xFFFC
        assert_eq!(m.global(0), 0xFFFC, "art_shift(-8, -1) = -4 = 0xFFFC");
        // arithmetic left-shift by 2: -8 << 2 = -32 = 0xFFE0
        assert_eq!(m.global(1), 0xFFE0, "art_shift(-8, 2) = -32 = 0xFFE0");
    }

    // -----------------------------------------------------------------------
    // pull sp — overwrite-new-top semantics (frotz §z_pull)
    // -----------------------------------------------------------------------

    #[test]
    fn pull_sp_overwrites_new_top() {
        // Stack before: [10, 20, 30] (10=bottom, 30=top)
        // pull #0 (pull Small(0) = destination is sp):
        //   pop 30 (value), stack=[10, 20], poke_stack(30) → stack=[10, 30]
        // pull #0 again:
        //   pop 30, stack=[10], poke_stack(30) → stack=[30]
        // pull Small(G0) = pop 30 into G0.
        // Then G0 should be 30, and the stack should have one item (30).
        //
        // We push 10, 20, 30 using push opcodes, then do two pull-sp, then
        // do a normal pull into G0, then quit.
        //
        // push: VAR:08 = 0xE8, type_byte=[Small,omit,omit,omit]=0x7F, value
        // pull Small(0): VAR:09 = 0xE9, type_byte=[Small,omit,omit,omit]=0x7F, 0
        // pull Small(G0_var=0x10): 0xE9, 0x7F, 0x10
        let mut buf = sample_story(5);
        let prog: &[u8] = &[
            0xE8, 0x7F, 10,   // push 10
            0xE8, 0x7F, 20,   // push 20
            0xE8, 0x7F, 30,   // push 30  — stack = [10, 20, 30]
            0xE9, 0x7F, 0x00, // pull #0 (sp)  — pops 30, pokes 30 over 20 → [10, 30]
            0xE9, 0x7F, 0x00, // pull #0 (sp)  — pops 30, pokes 30 over 10 → [30]
            0xE9, 0x7F, 0x10, // pull Small(G0=var 0x10)  — pops 30 into G0
            0xBA,             // quit
        ];
        for (i, &b) in prog.iter().enumerate() {
            buf[0x10 + i] = b;
        }
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 30, "pull sp: final value pulled into G0 should be 30");
    }

    // -----------------------------------------------------------------------
    // scan_table (VAR:0x17) — search table for value, store address, branch if found
    // -----------------------------------------------------------------------

    #[test]
    fn scan_table_word_finds_and_stores_address() {
        let mut m = build_test_machine(&[]);
        // Word table at 0x0200: [0x1111, 0x2222, 0x3333]
        m.mem.write_word(0x0200, 0x1111);
        m.mem.write_word(0x0202, 0x2222);
        m.mem.write_word(0x0204, 0x3333);
        // scan_table 0x2222, table=0x0200, len=3, form=0x82 (word, step 2) -> G0
        m.exec_var(0x17, &[0x2222, 0x0200, 3, 0x82], Some(16), None);
        assert_eq!(m.global(0), 0x0202, "address of the matching word entry");
    }

    #[test]
    fn scan_table_not_found_stores_zero() {
        let mut m = build_test_machine(&[]);
        m.mem.write_word(0x0200, 0x1111);
        // Pre-seed G0 nonzero so the assertion proves the opcode actively wrote 0
        // (not that the store path was skipped on a default-0 global).
        m.do_store(Some(16), 0xBEEF);
        m.exec_var(0x17, &[0x9999, 0x0200, 1, 0x82], Some(16), None);
        assert_eq!(m.global(0), 0, "no match -> store 0");
    }

    #[test]
    fn scan_table_byte_form_matches_full_byte_value() {
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x0200, 0x05);
        m.mem.write_byte(0x0201, 0x07);
        // form=0x01 -> byte entries, step 1. Search value 7 (<= 255) matches.
        m.exec_var(0x17, &[0x0007, 0x0200, 2, 0x01], Some(16), None);
        assert_eq!(m.global(0), 0x0201, "byte form matches the byte value at the second entry");
    }

    #[test]
    fn scan_table_byte_form_search_value_over_255_never_matches() {
        // Regression (SQ-0241): a byte-mode search for a value > 255 must NOT match
        // any byte. The old code masked x to its low byte (0x0100 -> 0x00) and
        // spuriously matched a zero byte, storing a nonzero address and taking the
        // branch (praxix "Bad @scan_table branch"). Since found == 0 here, the
        // branch (cond = found != 0) is correctly NOT taken.
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x0200, 0x00); // a zero byte the old low-byte mask would hit
        m.mem.write_byte(0x0201, 0x07);
        m.do_store(Some(16), 0xBEEF); // pre-seed to prove the opcode actively writes 0
        m.exec_var(0x17, &[0x0100, 0x0200, 2, 0x01], Some(16), None);
        assert_eq!(m.global(0), 0, "byte-mode search for a value > 255 never matches");
    }

    // -----------------------------------------------------------------------
    // copy_table (VAR:0x1D) — copy/zero memory region
    // -----------------------------------------------------------------------

    #[test]
    fn copy_table_copies_forward() {
        let mut m = build_test_machine(&[]);
        for i in 0..4u32 { m.mem.write_byte(0x0200 + i, (i + 1) as u8); } // 1,2,3,4
        m.exec_var(0x1D, &[0x0200, 0x0300, 4], None, None);
        for i in 0..4u32 { assert_eq!(m.mem.read_byte(0x0300 + i), (i + 1) as u8); }
    }

    #[test]
    fn copy_table_zeroes_when_second_is_zero() {
        let mut m = build_test_machine(&[]);
        for i in 0..3u32 { m.mem.write_byte(0x0200 + i, 0xFF); }
        m.exec_var(0x1D, &[0x0200, 0, 3], None, None);
        for i in 0..3u32 { assert_eq!(m.mem.read_byte(0x0200 + i), 0); }
    }

    #[test]
    fn copy_table_positive_size_overlap_is_noncorrupting() {
        let mut m = build_test_machine(&[]);
        for i in 0..4u32 { m.mem.write_byte(0x0200 + i, (i + 1) as u8); } // 1,2,3,4
        // Overlapping forward copy by 1 (dest > src). Positive size must NOT corrupt:
        // result at 0x0201..=0x0204 should be the ORIGINAL 1,2,3,4.
        m.exec_var(0x1D, &[0x0200, 0x0201, 4], None, None);
        assert_eq!(m.mem.read_byte(0x0201), 1);
        assert_eq!(m.mem.read_byte(0x0202), 2);
        assert_eq!(m.mem.read_byte(0x0203), 3);
        assert_eq!(m.mem.read_byte(0x0204), 4);
    }

    #[test]
    fn get_cursor_writes_row_and_col() {
        let mut m = build_test_machine(&[]);
        m.screen.cursor_row = 3;
        m.screen.cursor_col = 7;
        m.exec_var(0x10, &[0x0200], None, None); // array at 0x0200
        assert_eq!(m.mem.read_word(0x0200), 3, "word 0 = row");
        assert_eq!(m.mem.read_word(0x0202), 7, "word 1 = col");
    }

    // print_table (VAR:0x1E)
    fn captured_output(m: &Machine) -> String {
        m.buffer_output().expect("default sink is BufferOutput").buf.clone()
    }

    #[test]
    fn print_table_emits_each_row_chars() {
        let mut m = build_test_machine(&[]);
        // 2x2 region of ASCII at 0x0200: "AB" / "CD"
        m.mem.write_byte(0x0200, b'A');
        m.mem.write_byte(0x0201, b'B');
        m.mem.write_byte(0x0202, b'C');
        m.mem.write_byte(0x0203, b'D');
        m.exec_var(0x1E, &[0x0200, 2, 2, 0], None, None); // width 2, height 2, skip 0
        let out = captured_output(&m);
        assert!(out.contains('A') && out.contains('B') && out.contains('C') && out.contains('D'),
            "all rectangle characters are printed");
    }

    #[test]
    fn sound_effect_records_high_and_low_bleeps() {
        let mut m = build_test_machine(&[]);
        m.exec_var(0x15, &[1], None, None);
        m.exec_var(0x15, &[2], None, None);
        assert_eq!(m.pending_sounds.len(), 2);
        // No volume operand -> vw defaults to 8: volume 8, repeats 0.
        assert_eq!(m.pending_sounds[0], SoundEvent { number: 1, effect: 0, volume: 8, repeats: 0, routine: 0 });
        assert_eq!(m.pending_sounds[1], SoundEvent { number: 2, effect: 0, volume: 8, repeats: 0, routine: 0 });
        assert!(m.diagnostics.is_empty(), "bleeps must not record diagnostics");
    }

    #[test]
    fn sound_effect_records_sampled_sound_event_no_diagnostic() {
        let mut m = build_test_machine(&[]);
        // number 5, effect 2 (start), volume word 0xFF03 -> volume 3, repeats 255 (forever), routine 0x1234
        m.exec_var(0x15, &[5, 2, 0xFF03, 0x1234], None, None);
        assert_eq!(
            m.pending_sounds,
            vec![SoundEvent { number: 5, effect: 2, volume: 3, repeats: 255, routine: 0x1234 }]
        );
        assert!(m.diagnostics.is_empty(), "sampled sounds are recorded, not dropped as diagnostics");
    }

    #[test]
    fn sound_effect_zero_records_nothing() {
        let mut m = build_test_machine(&[]);
        m.exec_var(0x15, &[0], None, None);
        assert!(m.pending_sounds.is_empty());
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn unimplemented_var_opcode_records_diagnostic_not_stderr() {
        let mut m = build_test_machine(&[]);
        // Every VAR opcode number 0x00..=0x1F now has an arm, so probe the defensive
        // fallthrough with an out-of-range number no valid VAR encoding can produce.
        assert!(m.diagnostics.is_empty());
        m.exec_var(0xFF, &[], None, None);
        assert_eq!(m.diagnostics.len(), 1, "fallthrough records one diagnostic line");
        assert!(m.diagnostics[0].contains("0xFF"), "diagnostic names the opcode");
        m.exec_var(0xFF, &[], None, None); // second call must not duplicate
        assert_eq!(m.diagnostics.len(), 1, "warn-once: no duplicate diagnostic");
    }

    #[test]
    fn unimplemented_var_opcode_is_warned_once() {
        let mut m = build_test_machine(&[]);
        // Out-of-range VAR opcode number: no arm, hits the defensive fallthrough.
        assert!(m.warned_var_opcodes.is_empty());
        m.exec_var(0xFF, &[], None, None);
        assert!(m.warned_var_opcodes.contains(&0xFF), "fallthrough records the opcode");
        m.exec_var(0xFF, &[], None, None); // second call must not duplicate
        assert_eq!(m.warned_var_opcodes.len(), 1, "warned at most once per opcode");
    }

    #[test]
    fn input_stream_records_selection_without_warning() {
        let mut m = build_test_machine(&[]);
        assert_eq!(m.streams.input_stream, 0, "defaults to keyboard");
        // Select the command-file input stream.
        m.exec_var(0x14, &[1], None, None);
        assert_eq!(m.streams.input_stream, 1, "input_stream 1 selects the command file");
        // Back to the keyboard.
        m.exec_var(0x14, &[0], None, None);
        assert_eq!(m.streams.input_stream, 0, "input_stream 0 selects the keyboard");
        // Out-of-spec values are ignored, leaving the selection unchanged.
        m.exec_var(0x14, &[7], None, None);
        assert_eq!(m.streams.input_stream, 0, "out-of-range stream ignored");
        // The opcode is implemented, so it must not record an unimplemented diagnostic.
        assert!(m.diagnostics.is_empty(), "input_stream is implemented, no warning");
        assert!(!m.warned_var_opcodes.contains(&0x14));
    }

    #[test]
    fn unimplemented_ext_opcode_is_warned_once() {
        let mut m = build_test_machine(&[]);
        // 0xFE has no arm in exec_ext -> hits the unimplemented fallthrough.
        assert!(m.warned_ext_opcodes.is_empty());
        assert!(m.diagnostics.is_empty());
        m.exec_ext(0xFE, &[], None, None);
        assert!(m.warned_ext_opcodes.contains(&0xFE), "fallthrough records the opcode");
        assert_eq!(m.diagnostics.len(), 1, "records one diagnostic line");
        assert!(
            m.diagnostics[0].contains("EXT") && m.diagnostics[0].contains("0xFE"),
            "diagnostic names EXT + opcode: {:?}", m.diagnostics[0]
        );
        m.exec_ext(0xFE, &[], None, None); // second call must not duplicate
        assert_eq!(m.warned_ext_opcodes.len(), 1, "warned at most once per opcode");
        assert_eq!(m.diagnostics.len(), 1, "warn-once: no duplicate diagnostic");
    }

    #[test]
    fn erase_line_is_recognized_noop_without_warning() {
        let mut m = build_test_machine(&[]);
        let r = m.exec_var(0x0E, &[1], None, None);
        assert!(matches!(r, StepResult::Continue));
        // It must be an explicit arm, not the unknown-opcode fallthrough (Task 6),
        // so it is NOT recorded as a warned opcode.
        assert!(!m.warned_var_opcodes.contains(&0x0E),
            "erase_line is a recognized arm, not an unimplemented fallthrough");
    }

    #[test]
    fn erase_line_clears_to_end_of_row_in_upper() {
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 5);
        m.exec_var(0x0A, &[1], None, None); // split 1 row, 5 cols
        m.screen.current_window = 1;
        m.screen.cursor_row = 1; m.screen.cursor_col = 1;
        m.print_text("ABCDE");
        // move cursor back to col 3 and erase to end of line
        m.screen.cursor_row = 1; m.screen.cursor_col = 3;
        m.exec_var(0x0E, &[1], None, None);
        assert_eq!(m.screen.upper.cell(1, 2).ch, 'B', "before cursor untouched");
        assert_eq!(m.screen.upper.cell(1, 3).ch, ' ', "from cursor cleared");
        assert_eq!(m.screen.upper.cell(1, 5).ch, ' ', "to end of line cleared");
    }

    #[test]
    fn erase_line_in_lower_window_leaves_the_upper_grid_alone() {
        // ZMSD §15 erase_line (v4/5): "erase from the current cursor position to
        // the end of its line in the current window." With window 0 selected the
        // upper grid is not the current window and must not be touched.
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 5);
        m.exec_var(0x0A, &[1], None, None); // split 1 row, 5 cols
        m.exec_var(0x0B, &[1], None, None); // set_window 1
        m.print_text("ABCDE");
        m.exec_var(0x0B, &[0], None, None); // set_window 0 (lower)
        m.screen.cursor_row = 1;
        m.screen.cursor_col = 3;
        m.exec_var(0x0E, &[1], None, None); // erase_line 1
        assert_eq!(m.screen.upper.cell(1, 3).ch, 'C', "upper row survives a lower-window erase_line");
        assert_eq!(m.screen.upper.cell(1, 5).ch, 'E');
    }

    #[test]
    fn erase_window_minus_two_clears_grid_without_unsplitting() {
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 5); // screen width = 5 cols
        m.exec_var(0x0A, &[2], None, None); // split 2 rows
        m.screen.current_window = 1;
        m.screen.cursor_row = 1; m.screen.cursor_col = 1;
        m.print_text("ABCDE");
        assert_eq!(m.screen.upper.cell(1, 1).ch, 'A');
        // erase_window(-2): erase all WITHOUT unsplitting (-2 = 0xFFFE as u16).
        m.exec_var(0x0D, &[0xFFFE], None, None);
        assert_eq!(m.screen.upper_window_rows, 2, "split preserved (no unsplit)");
        assert_eq!(m.screen.upper.cell(1, 1).ch, ' ', "upper grid cleared");
        assert!(m.screen.erase_lower_requested, "lower-window erase requested");
    }

    #[test]
    fn set_window_upper_homes_the_cursor_below_v6() {
        // ZMSD §8.6.1: "Whenever the upper window is selected, its cursor
        // position is reset to the top left." set_window used to only assign
        // current_window.
        let mut m = build_test_machine(&[]);
        m.exec_var(0x0A, &[3], None, None); // split_window 3
        m.screen.cursor_row = 3;
        m.screen.cursor_col = 7;
        m.exec_var(0x0B, &[1], None, None); // set_window(1) — the upper window
        assert_eq!((m.screen.cursor_row, m.screen.cursor_col), (1, 1), "upper cursor homed");
        // Selecting the LOWER window is not covered by that sentence: the upper
        // window keeps whatever position it had.
        m.screen.cursor_row = 2;
        m.screen.cursor_col = 4;
        m.exec_var(0x0B, &[0], None, None); // set_window(0)
        assert_eq!((m.screen.cursor_row, m.screen.cursor_col), (2, 4), "lower select leaves it");
    }

    #[test]
    fn set_cursor_is_ignored_in_the_lower_window_and_off_screen() {
        // ZMSD §8.7.2.3: "The opcode has no effect when the lower window is
        // selected. It is illegal to move the cursor outside the current size
        // of the upper window."
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x20, 10); // screen height = 10 lines
        m.mem.write_byte(0x21, 20); // screen width  = 20 cols
        m.exec_var(0x0A, &[3], None, None); // split_window 3
        m.exec_var(0x0B, &[1], None, None); // set_window 1 (upper)
        m.exec_var(0x0F, &[2, 5], None, None); // legal move inside the split
        assert_eq!((m.screen.cursor_row, m.screen.cursor_col), (2, 5));

        // Lower window selected → no effect.
        m.exec_var(0x0B, &[0], None, None); // set_window 0
        m.exec_var(0x0F, &[1, 1], None, None);
        assert_eq!(
            (m.screen.cursor_row, m.screen.cursor_col),
            (2, 5),
            "set_cursor has no effect while the lower window is selected"
        );

        // Back in the upper window, an off-screen target is illegal → ignored.
        m.exec_var(0x0B, &[1], None, None); // homes the cursor to (1,1)
        m.exec_var(0x0F, &[11, 1], None, None); // row past the 10-line screen
        m.exec_var(0x0F, &[1, 21], None, None); // col past the 20-col screen
        m.exec_var(0x0F, &[0, 0], None, None); // 0 is outside the 1-based grid
        assert_eq!((m.screen.cursor_row, m.screen.cursor_col), (1, 1), "illegal moves ignored");
    }

    #[test]
    fn split_window_preserves_upper_contents_from_v4() {
        // ZMSD §15 split_window: "In Version 3 (only) the upper window should be
        // cleared after the split." From v4 a re-split must leave the existing
        // contents on screen; our resize used to reallocate blank in every
        // version.
        let mut m = screen_machine(5);
        m.exec_var(0x0A, &[2], None, None); // split 2 rows
        m.screen.current_window = 1;
        m.screen.cursor_row = 1;
        m.screen.cursor_col = 1;
        m.print_text("HELLO");
        m.exec_var(0x0A, &[3], None, None); // re-split, taller
        assert_eq!(m.screen.upper.cell(1, 1).ch, 'H', "v5 re-split keeps the old contents");
        assert_eq!(m.screen.upper.rows, 3, "and still resizes");
        assert_eq!(m.screen.upper.cell(3, 1).ch, ' ', "the new row is blank");
        m.exec_var(0x0A, &[1], None, None); // shrink
        assert_eq!(m.screen.upper.rows, 1, "shrinking truncates");
        assert_eq!(m.screen.upper.cell(1, 1).ch, 'H', "the surviving row survives");
    }

    #[test]
    fn split_window_clears_upper_in_v3_only() {
        // The other half of the same §15 sentence.
        let mut m = screen_machine(3);
        m.exec_var(0x0A, &[2], None, None);
        m.screen.current_window = 1;
        m.screen.cursor_row = 1;
        m.screen.cursor_col = 1;
        m.print_text("HELLO");
        assert_eq!(m.screen.upper.cell(1, 1).ch, 'H');
        m.exec_var(0x0A, &[2], None, None); // re-split
        assert_eq!(m.screen.upper.cell(1, 1).ch, ' ', "v3 clears the upper window after a split");
    }

    #[test]
    fn erase_window_blanks_to_background_and_never_reverse_video() {
        // ZMSD §8.7.3.2: a window is erased "to background colour"; §8.7.3.4:
        // "Even if the text style is Reverse Video the new blank space should
        // not have reversed colours." The blanks used to be Cell::default()
        // (Default background) regardless of the selected colours.
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 5);
        m.exec_var(0x0A, &[1], None, None);
        m.exec_2op(0x1B, &[3, 6], None, None); // set_colour(fg=red, bg=blue)
        m.screen.text_style = 1; // reverse video ON
        m.exec_var(0x0D, &[1], None, None); // erase_window(1)
        let c = m.screen.upper.cell(1, 1);
        assert_eq!(c.bg, ZColour::Standard(6), "blank carries the current background");
        assert_eq!(c.style & 1, 0, "blank space is never reverse-video");
    }

    #[test]
    fn erase_line_blank_is_never_reverse_video() {
        // Same §8.7.3.4 sentence, via erase_line: the blanks used to be stamped
        // with `text_style`, reverse bit and all.
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 5);
        m.exec_var(0x0A, &[1], None, None);
        m.exec_2op(0x1B, &[3, 6], None, None); // set_colour(fg=red, bg=blue)
        m.screen.current_window = 1;
        m.screen.cursor_row = 1;
        m.screen.cursor_col = 1;
        m.screen.text_style = 1; // reverse video ON
        m.exec_var(0x0E, &[1], None, None); // erase_line 1
        let c = m.screen.upper.cell(1, 1);
        assert_eq!(c.bg, ZColour::Standard(6), "erased cell clears to background colour");
        assert_eq!(c.style & 1, 0, "erased cell is never reverse-video");
    }

    #[test]
    fn erase_window_homes_the_upper_cursor_and_minus_one_selects_lower() {
        // ZMSD §8.7.3.2.1: "In Versions 5 and later, the cursor for the window
        // being erased should be moved to the top left. In Version 4, the lower
        // window's cursor moves to its bottom left, while the upper window's
        // cursor moves to top left." §8.7.3.3 adds that erasing -1 "selects the
        // lower screen". erase_window used to do neither.
        for v in [4u8, 5] {
            let mut m = screen_machine(v);
            m.exec_var(0x0A, &[3], None, None);
            m.screen.current_window = 1;
            m.screen.cursor_row = 3;
            m.screen.cursor_col = 7;
            m.exec_var(0x0D, &[1], None, None); // erase_window(1)
            assert_eq!(
                (m.screen.cursor_row, m.screen.cursor_col),
                (1, 1),
                "v{v}: erasing the upper window homes its cursor"
            );

            m.screen.current_window = 1;
            m.screen.cursor_row = 2;
            m.screen.cursor_col = 5;
            m.exec_var(0x0D, &[0xFFFF], None, None); // erase_window(-1)
            assert_eq!(m.screen.current_window, 0, "v{v}: -1 selects the lower window");
            assert_eq!(m.screen.upper_window_rows, 0, "v{v}: -1 collapses the upper window");
            assert_eq!(
                (m.screen.cursor_row, m.screen.cursor_col),
                (1, 1),
                "v{v}: -1 homes the upper cursor too"
            );
        }
    }

    #[test]
    fn erase_window_zero_requests_lower_erase() {
        let mut m = build_test_machine(&[]);
        assert!(!m.screen.erase_lower_requested);
        m.exec_var(0x0D, &[0], None, None); // erase lower window
        assert!(m.screen.erase_lower_requested, "erase_window(0) requests a lower-window clear");
    }

    #[test]
    fn print_to_upper_window_lands_in_grid_not_stream() {
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 10); // screen width = 10 cols
        m.exec_var(0x0A, &[2], None, None);     // split_window 2
        m.exec_var(0x0B, &[1], None, None);     // set_window 1 (upper)
        m.screen.cursor_row = 1; m.screen.cursor_col = 1;
        m.print_text("Hi");
        assert_eq!(m.screen.upper.cell(1, 1).ch, 'H');
        assert_eq!(m.screen.upper.cell(1, 2).ch, 'i');
        assert_eq!(m.screen.cursor_col, 3, "cursor advanced past the text");
        // Nothing went to the lower-window output sink:
        assert_eq!(m.buffer_output().expect("sink").buf, "");
    }

    #[test]
    fn upper_window_write_beyond_split_grows_grid() {
        // De-facto Z-machine behaviour (relied on by Inform's menu library, e.g.
        // LostPig's HELP menu): a game may split_window(N) then draw in the upper
        // window at rows *beyond* N via set_cursor. Real interpreters (Frotz) keep
        // that content on screen; our grid must grow to hold writes past the split
        // (bounded by the header screen height at 0x20) rather than dropping them.
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x20, 10); // screen height = 10 lines
        m.mem.write_byte(0x21, 10); // screen width  = 10 cols
        m.exec_var(0x0A, &[2], None, None); // split_window 2
        m.exec_var(0x0B, &[1], None, None); // set_window 1 (upper)
        m.screen.cursor_row = 4; // below the 2-row split
        m.screen.cursor_col = 1;
        m.print_text("X");
        assert_eq!(m.screen.upper.cell(4, 1).ch, 'X', "write past split kept, not dropped");
        assert!(m.screen.upper.rows >= 4, "grid grew to include row 4");
    }

    #[test]
    fn upper_window_write_beyond_screen_height_is_clipped() {
        // Growth is bounded by the header screen height: a write below the
        // physical screen is dropped, matching a real interpreter's clip.
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x20, 5); // screen height = 5 lines
        m.mem.write_byte(0x21, 10);
        m.exec_var(0x0A, &[2], None, None); // split_window 2
        m.exec_var(0x0B, &[1], None, None); // set_window 1 (upper)
        m.screen.cursor_row = 9; // beyond the 5-line screen
        m.screen.cursor_col = 1;
        m.print_text("Z");
        assert!(m.screen.upper.rows <= 5, "grid does not grow past screen height");
        assert_eq!(m.screen.upper.cell(9, 1).ch, ' ', "off-screen write dropped");
    }

    #[test]
    fn lower_window_still_streams() {
        let mut m = build_test_machine(&[]);
        m.screen.current_window = 0;
        m.print_text("ok");
        assert_eq!(m.buffer_output().expect("sink").buf, "ok");
    }

    #[test]
    fn split_window_sizes_grid_from_header_cols() {
        let mut m = build_test_machine(&[]);
        m.mem.write_byte(0x21, 12);
        m.exec_var(0x0A, &[3], None, None);
        assert_eq!(m.screen.upper.rows, 3);
        assert_eq!(m.screen.upper.cols, 12);
    }

    /// The row of the upper grid as a string (trailing blanks included).
    fn upper_row_text(m: &Machine, row: u16) -> String {
        (1..=m.screen.upper.cols).map(|c| m.screen.upper.cell(row, c).ch).collect()
    }

    /// SQ-0533: a v5 game that splits ONCE (Sherlock, Trinity) must still get a
    /// grid as wide as the resized screen — §8.4 lets the interpreter change the
    /// dimensions, so the live upper window follows $21.
    #[test]
    fn set_screen_dims_widens_a_live_upper_window_preserving_content() {
        let mut m = build_test_machine(&[]);
        m.set_screen_dims(24, 80);
        m.exec_var(0x0A, &[1], None, None); // split_window 1 (the only split)
        assert_eq!(m.screen.upper.cols, 80, "boot-time grid width");
        for (i, ch) in "West of House".chars().enumerate() {
            m.screen.upper.put(1, i as u16 + 1, ch, 0, ZColour::Default, ZColour::Default);
        }

        m.set_screen_dims(24, 100); // the terminal got wider

        assert_eq!(m.screen.upper.cols, 100, "the live grid follows the new width");
        assert_eq!(m.screen.upper.rows, 1, "the split height is the game's, untouched");
        assert_eq!(m.screen.upper_window_rows, 1, "split rows untouched");
        let row = upper_row_text(&m, 1);
        assert!(row.starts_with("West of House"), "content preserved left-aligned: {row:?}");
        assert_eq!(row.len(), 100, "row spans the new width");
        assert!(row[13..].chars().all(|c| c == ' '), "grown columns are blank: {row:?}");
    }

    /// The shrink half: columns past the new width are truncated, and a cursor
    /// left beyond the right edge clamps to the last column (§8.7.2.3 makes an
    /// out-of-window cursor illegal; §8.7.2.2's precedent is a minimal nudge on
    /// the axis that went out of range, keeping the row).
    #[test]
    fn set_screen_dims_shrinks_a_live_upper_window_and_clamps_the_cursor() {
        let mut m = build_test_machine(&[]);
        m.set_screen_dims(24, 80);
        m.exec_var(0x0A, &[2], None, None); // split_window 2
        for (i, ch) in "0123456789".chars().enumerate() {
            m.screen.upper.put(2, i as u16 + 1, ch, 0, ZColour::Default, ZColour::Default);
        }
        m.screen.current_window = 1;
        m.screen.cursor_row = 2;
        m.screen.cursor_col = 70;

        m.set_screen_dims(24, 60); // the terminal got narrower

        assert_eq!(m.screen.upper.cols, 60, "the live grid follows the new width");
        assert_eq!(m.screen.upper.rows, 2, "the split height is the game's, untouched");
        let row = upper_row_text(&m, 2);
        assert_eq!(row.len(), 60, "row truncated to the new width");
        assert!(row.starts_with("0123456789"), "surviving content preserved: {row:?}");
        assert_eq!(m.screen.cursor_col, 60, "cursor clamped to the last column");
        assert_eq!(m.screen.cursor_row, 2, "the row the game set is kept");
    }

    /// No live upper window (never split, or unsplit) → nothing to refit; the
    /// grid stays empty rather than being conjured at the new width.
    #[test]
    fn set_screen_dims_leaves_an_unsplit_upper_window_alone() {
        let mut m = build_test_machine(&[]);
        m.set_screen_dims(24, 80);
        assert_eq!(m.screen.upper.rows, 0, "no split yet");
        m.set_screen_dims(24, 100);
        assert_eq!(m.screen.upper.rows, 0, "still no upper window");
        assert_eq!(m.screen.upper.cols, 0, "no grid conjured by a resize");
        // …and the NEXT split still adopts the new width (wave 2 behaviour).
        m.exec_var(0x0A, &[1], None, None);
        assert_eq!(m.screen.upper.cols, 100);
    }

    // -----------------------------------------------------------------------
    // Task 1: verify (real checksum) + piracy
    // -----------------------------------------------------------------------

    #[test]
    fn verify_branches_true_on_correct_checksum() {
        let mut buf = sample_story(5);
        // Give the story a non-empty checksum region so the match path is real.
        buf[0x1A] = 0x00; buf[0x1B] = 0x20; // file-length word = 0x20 -> 0x80 bytes (v5 *4)
        buf[0x40] = 0xAB;                   // a marker byte inside [0x40, 0x80)
        // 0x10: verify (0OP:0x0D = 0xBD), branch on_true offset 6 -> skip the add.
        buf[0x10] = 0xBD;
        buf[0x11] = 0xC6; // branch: on_true (bit7), short (bit6), offset 6
        // add 0,7 -> G0 (2OP:0x14 long form, two small consts), skipped if branch taken.
        buf[0x12] = 0x14; buf[0x13] = 0x00; buf[0x14] = 0x07; buf[0x15] = 0x10;
        buf[0x16] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        let ck = m.story_checksum();
        assert_ne!(ck, 0, "checksum region non-empty so the compare path is exercised");
        m.mem.write_word(0x1C, ck); // header checksum = computed -> verify must branch true
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 0, "verify branched true (skipped the add)");
    }

    #[test]
    fn verify_branches_false_on_bad_checksum() {
        let mut buf = sample_story(5);
        buf[0x1A] = 0x00; buf[0x1B] = 0x20; buf[0x40] = 0xAB;
        buf[0x10] = 0xBD; buf[0x11] = 0xC6;
        buf[0x12] = 0x14; buf[0x13] = 0x00; buf[0x14] = 0x07; buf[0x15] = 0x10;
        buf[0x16] = 0xBA;
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.mem.write_word(0x1C, 0x0001); // deliberately wrong checksum
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 7, "verify branched false (ran the add)");
    }

    // -----------------------------------------------------------------------
    // Task 2: set_font + set_colour + set_true_colour (graceful)
    // -----------------------------------------------------------------------

    #[test]
    fn set_font_reports_current_or_unavailable() {
        // EXT:0x04 set_font font -> (store). font 1 (or 0=query) -> 1; other -> 0.
        let mut buf = sample_story(5);
        // set_font 1 -> G0
        buf[0x10]=0xBE; buf[0x11]=0x04; buf[0x12]=0x7F; buf[0x13]=1; buf[0x14]=0x10;
        // set_font 4 -> G1
        buf[0x15]=0xBE; buf[0x16]=0x04; buf[0x17]=0x7F; buf[0x18]=4; buf[0x19]=0x11;
        buf[0x1A]=0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 1, "font 1 available -> previous font (1)");
        assert_eq!(m.global(1), 1, "font 4 (Courier/fixed-pitch) accepted -> previous font (1)");
    }

    #[test]
    fn catch_then_throw_unwinds_and_returns_value() {
        // catch (0OP:0x09 v5+) records the call-stack depth; throw (2OP:0x1C)
        // unwinds back to that depth and returns the given value from the
        // catching routine, as a non-local return (ZMSD §15).
        let mut m = build_test_machine(&[]);

        // Frame A: the routine that calls catch; its result lands in global 0.
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0x4242, locals: vec![], eval_base: 0,
            store_var: Some(0x10), arg_count: 0, func_addr: 0,
        });
        // catch -> store depth in global 1.
        m.exec_0op(0x09, Some(0x11), None, None);
        let caught = m.global(1);
        assert_eq!(caught, 1, "catch records the depth (1 frame on the stack)");

        // A calls deeper into B then C.
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0x0010, locals: vec![], eval_base: 0,
            store_var: Some(0x12), arg_count: 0, func_addr: 0,
        });
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0x0020, locals: vec![], eval_base: 0,
            store_var: Some(0x13), arg_count: 0, func_addr: 0,
        });
        assert_eq!(m.state.frames.len(), 3);

        // throw 0x1234 back to the caught frame.
        m.exec_2op(0x1C, &[0x1234, caught], None, None);

        assert_eq!(m.state.frames.len(), 0, "unwound past the catching routine's frame");
        assert_eq!(m.state.pc, 0x4242, "pc restored to the catching routine's return_pc");
        assert_eq!(m.global(0), 0x1234, "thrown value returned to the catching routine's caller");
    }

    #[test]
    fn set_colour_and_true_colour_are_graceful_noops() {
        // Neither stores nor branches; just must not warn/crash and must Continue.
        let mut buf = sample_story(5);
        // set_colour 2,3 (2OP:0x1B long form, both small)
        buf[0x10]=0x1B; buf[0x11]=2; buf[0x12]=3;
        // draw_picture 0,0 (EXT:0x05, [Large,Large]) — graceful no-op
        let ext05 = { let mut v = vec![]; emit_ext_instr(&mut v, 0x05, &[0, 0]); v }; // 7 bytes
        buf[0x13..0x13 + ext05.len()].copy_from_slice(&ext05);
        // add 0,5 -> G0 (proves execution continued)
        let add_off = 0x13 + ext05.len(); // 0x1A
        buf[add_off]=0x14; buf[add_off+1]=0x00; buf[add_off+2]=0x05; buf[add_off+3]=0x10;
        buf[add_off+4]=0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        assert_eq!(m.global(0), 5, "execution continued past set_colour/set_true_colour");
        assert!(m.diagnostics.iter().all(|d| !d.contains("0x1B") && !d.contains("0x05")),
            "graceful arms must not emit unimplemented diagnostics");
    }

    #[test]
    fn set_colour_honors_sentinels() {
        // Helper: run "set_colour fg,bg" (2OP:0x1B long form, two smalls) at 0x10.
        fn run_set_colour(fg: u8, bg: u8) -> (ZColour, ZColour) {
            let mut buf = sample_story(5);
            // 2OP long form, both operands Small: opcode byte 0x1B, fg, bg.
            buf[0x10] = 0x1B;
            buf[0x11] = fg;
            buf[0x12] = bg;
            buf[0x13] = 0xBA; // quit
            let mem = Memory::new(buf).unwrap();
            let mut m = Machine::new(mem);
            m.state.pc = 0x10;
            m.step(); // set_colour
            (m.screen.current_fg, m.screen.current_bg)
        }

        // start both non-default, then 0 must KEEP each channel
        let (fg, bg) = run_set_colour(3, 6);
        assert_eq!(fg, ZColour::Standard(3));
        assert_eq!(bg, ZColour::Standard(6));

        // 1 = default
        assert_eq!(run_set_colour(1, 1), (ZColour::Default, ZColour::Default));

        // ZMSD §8.3.1: "Colours 10, 11, 12, 15 and -1 are available only in
        // Version 6." This is a v5 story, so the greys are not a legal palette
        // entry — each channel keeps whatever it had (here: 3 / 6, set above
        // by the same instruction sequence… run fresh, so Default).
        assert_eq!(
            run_set_colour(10, 12),
            (ZColour::Default, ZColour::Default),
            "greys 10–12 are v6-only and must be ignored in v5"
        );
        assert_eq!(
            run_set_colour(11, 9),
            (ZColour::Default, ZColour::Standard(9)),
            "an illegal fg leaves its channel alone without disturbing a legal bg"
        );
    }

    #[test]
    fn set_colour_zero_keeps_channel() {
        // set fg=3,bg=6 then set fg=0,bg=4: fg keeps 3, bg becomes 4.
        let mut buf = sample_story(5);
        buf[0x10] = 0x1B; buf[0x11] = 3; buf[0x12] = 6;      // set_colour 3,6
        buf[0x13] = 0x1B; buf[0x14] = 0; buf[0x15] = 4;      // set_colour 0,4
        buf[0x16] = 0xBA;                                     // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m.step(); m.step();
        assert_eq!(m.screen.current_fg, ZColour::Standard(3), "fg=0 kept prior fg");
        assert_eq!(m.screen.current_bg, ZColour::Standard(4), "bg updated to 4");
    }

    // -----------------------------------------------------------------------
    // Task 3 (series): set_true_colour (EXT:0x0D) sentinel handling
    // -----------------------------------------------------------------------

    #[test]
    fn set_true_colour_honors_sentinels() {
        fn run_true(fg: i16, bg: i16) -> (ZColour, ZColour) {
            let mut buf = sample_story(5);
            let instr = {
                let mut v = vec![];
                emit_ext_instr(&mut v, 0x0D, &[fg as u16, bg as u16]);
                v
            };
            buf[0x10..0x10 + instr.len()].copy_from_slice(&instr);
            buf[0x10 + instr.len()] = 0xBA; // quit
            let mem = Memory::new(buf).unwrap();
            let mut m = Machine::new(mem);
            m.state.pc = 0x10;
            m.step();
            (m.screen.current_fg, m.screen.current_bg)
        }

        assert_eq!(run_true(0x7FFF, -1), (ZColour::True(0x7FFF), ZColour::Default));

        // -2 keeps. Pre-set fg=3, then true_colour(-2,-1): fg stays Standard(3).
        let mut buf = sample_story(5);
        buf[0x10] = 0x1B; buf[0x11] = 3; buf[0x12] = 6;   // set_colour 3,6
        let mut pos = 0x13;
        let instr = { let mut v = vec![]; emit_ext_instr(&mut v, 0x0D, &[(-2i16) as u16, (-1i16) as u16]); v };
        buf[pos..pos + instr.len()].copy_from_slice(&instr); pos += instr.len();
        buf[pos] = 0xBA;
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m.step(); m.step();
        assert_eq!(m.screen.current_fg, ZColour::Standard(3), "-2 kept fg");
        assert_eq!(m.screen.current_bg, ZColour::Default, "-1 set bg default");
    }

    #[test]
    fn decode_true_colour_spec_sentinels() {
        // ZMSD §8.3.7: (-1) default, (-2) current, (-3) under cursor (V6),
        // (-4) transparent (V6). -1 → Default; -2/-3/-4 → keep (None). -3 (no
        // render feedback) and -4 (transparency unsupported, §8.3.6 says ignore)
        // are conformant no-ops. Non-negative = 15-bit RGB (bit15 masked off).
        assert_eq!(decode_true_colour((-1i16) as u16), Some(ZColour::Default));
        assert_eq!(decode_true_colour((-2i16) as u16), None, "-2 current → keep");
        assert_eq!(decode_true_colour((-3i16) as u16), None, "-3 under-cursor → keep");
        assert_eq!(decode_true_colour((-4i16) as u16), None, "-4 transparent → ignore/keep");
        assert_eq!(decode_true_colour(0x0000), Some(ZColour::True(0x0000)), "black");
        assert_eq!(decode_true_colour(0x7FFF), Some(ZColour::True(0x7FFF)), "white");
    }

    // -----------------------------------------------------------------------
    // Task 4 (colour series): upper-window cells capture active colour
    // -----------------------------------------------------------------------

    #[test]
    fn upper_window_cells_capture_active_colour() {
        // split_window 1; set_window 1; set_colour 3,6; print "H".
        let mut buf = sample_story(5);
        let mut pos = 0x10usize;
        let mut v = vec![];
        emit_var_instr(&mut v, 0x0A, &[1]); // split_window 1
        buf[pos..pos + v.len()].copy_from_slice(&v); pos += v.len();
        v.clear();
        emit_var_instr(&mut v, 0x0B, &[1]); // set_window 1
        buf[pos..pos + v.len()].copy_from_slice(&v); pos += v.len();
        // set_colour 3,6 (2OP long form)
        buf[pos] = 0x1B; buf[pos+1] = 3; buf[pos+2] = 6; pos += 3;
        v.clear();
        emit_var_instr(&mut v, 0x05, &[72]); // print_char 'H'
        buf[pos..pos + v.len()].copy_from_slice(&v); pos += v.len();
        buf[pos] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        let cell = m.screen.upper.cell(1, 1);
        assert_eq!(cell.ch, 'H');
        assert_eq!(cell.fg, ZColour::Standard(3));
        assert_eq!(cell.bg, ZColour::Standard(6));
    }

    // -----------------------------------------------------------------------
    // Sectioned debug trace (Task 3): screen-control opcodes decode into
    // `screen_trace` when `trace_screen` is enabled.
    // -----------------------------------------------------------------------

    #[test]
    fn screen_trace_records_decoded_display_opcodes_when_enabled() {
        // set_colour(fg=std5, bg=std2); set_text_style(reverse|bold); split_window(1); quit.
        fn build() -> Machine {
            let mut buf = sample_story(5);
            let mut pos = 0x10usize;
            // set_colour 5,2 (2OP:0x1B long form, both small consts)
            buf[pos] = 0x1B; buf[pos + 1] = 5; buf[pos + 2] = 2; pos += 3;
            let mut v = vec![];
            emit_var_instr(&mut v, 0x11, &[3]); // set_text_style reverse|bold
            buf[pos..pos + v.len()].copy_from_slice(&v); pos += v.len();
            v.clear();
            emit_var_instr(&mut v, 0x0A, &[1]); // split_window 1
            buf[pos..pos + v.len()].copy_from_slice(&v); pos += v.len();
            buf[pos] = 0xBA; // quit
            let mem = Memory::new(buf).unwrap();
            let mut m = Machine::new(mem);
            m.state.pc = 0x10;
            m
        }

        let mut m = build();
        m.trace_screen = true;
        run_until_quit(&mut m);
        assert!(m.screen_trace.iter().any(|l| l.starts_with("@set_colour(")), "{:?}", m.screen_trace);
        assert!(m.screen_trace.iter().any(|l| l.contains("@set_text_style(") && l.contains("reverse")), "{:?}", m.screen_trace);
        assert!(m.screen_trace.iter().any(|l| l == "@split_window(1)"), "{:?}", m.screen_trace);

        // Disabled → nothing accumulates.
        let mut m2 = build();
        m2.trace_screen = false;
        run_until_quit(&mut m2);
        assert!(m2.screen_trace.is_empty());
    }

    #[test]
    fn trace_exec_records_executed_instruction_start_pcs() {
        // Two 2OP `add` instructions (long form: opcode, op1, op2, store -> sp),
        // then quit.
        let mut buf = sample_story(5);
        let mut pos = 0x10usize;
        buf[pos] = 0x14; buf[pos + 1] = 1; buf[pos + 2] = 1; buf[pos + 3] = 0; pos += 4; // add 1,1 -> sp
        let first_pc = 0x10u32;
        let second_pc = pos as u32;
        buf[pos] = 0x14; buf[pos + 1] = 2; buf[pos + 2] = 2; buf[pos + 3] = 0; pos += 4; // add 2,2 -> sp
        buf[pos] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m.trace_exec = true;
        m.step();
        m.step();
        assert!(m.exec_pcs.contains(&first_pc), "{:?}", m.exec_pcs);
        assert!(m.exec_pcs.contains(&second_pc), "{:?}", m.exec_pcs);
        // The cumulative set mirrors the per-turn set as instructions run…
        assert!(m.ever_exec_pcs.contains(&first_pc), "{:?}", m.ever_exec_pcs);
        assert!(m.ever_exec_pcs.contains(&second_pc), "{:?}", m.ever_exec_pcs);
        // …but a host clear of the per-turn set leaves the cumulative set intact.
        m.exec_pcs.clear();
        assert!(m.exec_pcs.is_empty());
        assert!(m.ever_exec_pcs.contains(&first_pc), "{:?}", m.ever_exec_pcs);
        assert!(m.ever_exec_pcs.contains(&second_pc), "{:?}", m.ever_exec_pcs);
    }

    #[test]
    fn seed_executed_extends_the_cumulative_set() {
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.seed_executed([0x0400u32, 0x0500]);
        assert!(m.ever_exec_pcs.contains(&0x0400));
        assert!(m.ever_exec_pcs.contains(&0x0500));
        // Seeding does not touch the per-turn set (only cumulative coverage).
        assert!(m.exec_pcs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Task 3: print_unicode + check_unicode (EXT:0x0B / 0x0C)
    // -----------------------------------------------------------------------

    #[test]
    fn print_unicode_outputs_codepoint() {
        let mut buf = sample_story(5);
        // print_unicode 0x00E9 ('é'): EXT:0x0B, [Large operand 0x00E9]
        // type byte 0b00_11_11_11 = 0x3F ([Large, omit, omit, omit]); large = 2 bytes.
        buf[0x10]=0xBE; buf[0x11]=0x0B; buf[0x12]=0x3F; buf[0x13]=0x00; buf[0x14]=0xE9;
        buf[0x15]=0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        let out = m.buffer_output().expect("default sink");
        assert!(out.buf.contains('é'), "é reached the output sink: {:?}", out.buf);
    }

    #[test]
    fn check_unicode_reports_printable_only() {
        let mut buf = sample_story(5);
        // check_unicode 0x00E9 -> G0  (EXT:0x0C, [Large], store)
        buf[0x10]=0xBE; buf[0x11]=0x0C; buf[0x12]=0x3F; buf[0x13]=0x00; buf[0x14]=0xE9; buf[0x15]=0x10;
        buf[0x16]=0xBA;
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        // Printable (bit0) but NOT receivable (bit1) — input is byte-limited.
        assert_eq!(m.global(0), 1, "valid scalar: printable only = 1");
    }

    // -----------------------------------------------------------------------
    // Task 4: encode_text (VAR:0x1C)
    // -----------------------------------------------------------------------

    #[test]
    fn encode_text_writes_packed_word() {
        let mut buf = sample_story(5);
        // Lay out a ZSCII source word "sword" at 0x40 (dynamic memory), and a 6-byte
        // coded-text buffer at 0x50. encode_text 0x40, 5, 0, 0x50.
        for (i, b) in b"sword".iter().enumerate() { buf[0x40 + i] = *b; }
        // encode_text (VAR:0x1C). opcode byte = 0xE0 | 0x1C = 0xFC.
        // 4 operands [text,length,from,coded]: type byte 0b01_01_01_01 = 0x55.
        buf[0x10]=0xFC; buf[0x11]=0x55; buf[0x12]=0x40; buf[0x13]=5; buf[0x14]=0; buf[0x15]=0x50;
        buf[0x16]=0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        let expected = crate::text::encode::encode_word("sword", 5);
        for (i, b) in expected.iter().enumerate() {
            assert_eq!(m.mem.read_byte(0x50 + i as u32), *b, "coded byte {i}");
        }
    }

    // -----------------------------------------------------------------------
    // Task 5: tokenise (VAR:0x1B)
    // -----------------------------------------------------------------------

    #[test]
    fn tokenise_parses_a_dictionary_word_into_parse_buffer() {
        // build_input_story embeds a dict containing "north" at addr_north.
        let (mut buf, addr_north, _open, _mailbox) = build_input_story(5);
        // v5 text buffer at 0x0250: [max][count][chars...]. "north" already entered.
        let text_buf: u16 = 0x0250;
        buf[text_buf as usize] = 10;     // max chars
        buf[text_buf as usize + 1] = 5;  // current length
        for (i, b) in b"north".iter().enumerate() {
            buf[text_buf as usize + 2 + i] = *b;
        }
        // parse buffer at 0x0260: byte0 = max words.
        let parse_buf: u16 = 0x0260;
        buf[parse_buf as usize] = 4;
        // tokenise text parse (dict=0, flag=0): VAR:0x1B = 0xFB, [Large, Large, omit, omit].
        buf[0x10]=0xFB; buf[0x11]=0x0F;
        buf[0x12]=(text_buf>>8) as u8; buf[0x13]=(text_buf&0xFF) as u8;
        buf[0x14]=(parse_buf>>8) as u8; buf[0x15]=(parse_buf&0xFF) as u8;
        buf[0x16]=0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        run_until_quit(&mut m);
        let pb = parse_buf as u32;
        assert!(m.mem.read_byte(pb + 1) >= 1, "at least one token parsed");
        assert_eq!(m.mem.read_word(pb + 2), addr_north, "known word resolved to its dict entry");
    }

    #[test]
    fn tokenise_honours_custom_dictionary_operand() {
        // VAR:0x1B operand 2 is a custom dictionary address; the word must be
        // resolved against it, not the standard story dictionary (ZMSD §15).
        let mut m = build_test_machine(&[]);
        // Custom dictionary at 0x02A0: 0 separators, entry_length 6, count -1
        // (unsorted, one entry) — all in dynamic memory below global_vars (0x300).
        let dict: u32 = 0x02A0;
        m.mem.write_byte(dict, 0);          // 0 separators
        m.mem.write_byte(dict + 1, 6);      // entry_length = 6 (v5 key length)
        m.mem.write_word(dict + 2, 0xFFFF); // count = -1 -> abs 1, unsorted
        let entry = dict + 4;
        let key = crate::text::encode::encode_word("frotz", 5);
        assert_eq!(key.len(), 6, "v5 dictionary key is 6 bytes");
        for (i, b) in key.iter().enumerate() {
            m.mem.write_byte(entry + i as u32, *b);
        }
        // v5 text buffer at 0x0250: [max][count][chars...] holding "frotz".
        let text_buf: u32 = 0x0250;
        m.mem.write_byte(text_buf, 10);
        m.mem.write_byte(text_buf + 1, 5);
        for (i, b) in b"frotz".iter().enumerate() {
            m.mem.write_byte(text_buf + 2 + i as u32, *b);
        }
        // parse buffer at 0x0270: byte0 = max words.
        let parse: u32 = 0x0270;
        m.mem.write_byte(parse, 4);
        // tokenise text parse custom-dict flag=0.
        m.exec_var(0x1B, &[text_buf as u16, parse as u16, dict as u16, 0], None, None);
        assert_eq!(m.mem.read_byte(parse + 1), 1, "one token parsed");
        assert_eq!(m.mem.read_word(parse + 2), entry as u16,
            "word resolved against the CUSTOM dictionary entry, not the standard dict");
    }

    // -----------------------------------------------------------------------
    // Task 7: terminating-characters table (header 0x2E, v5+)
    // -----------------------------------------------------------------------

    #[test]
    fn terminating_chars_table_is_honoured() {
        // Build a v5 story with header 0x2E -> a table [0x81, 0x00] (function key 129).
        let mut buf = sample_story(5);
        let tbl: u32 = 0x0200;
        buf[0x2E] = (tbl >> 8) as u8; buf[0x2F] = (tbl & 0xFF) as u8;
        buf[tbl as usize] = 0x81; buf[tbl as usize + 1] = 0x00;
        let mem = Memory::new(buf).unwrap();
        let m = Machine::new(mem);
        assert!(m.is_terminator(13), "Enter always terminates");
        assert!(m.is_terminator(0x81), "listed function key terminates");
        assert!(!m.is_terminator(b'a' as u16), "ordinary char does not terminate");
    }

    // ── v5 auxiliary save/restore table form (EXT:0x00 / EXT:0x01, ≥3 operands) ──
    //
    // Lays a name string at 0x300 ("AB", length-prefixed) and a 4-byte data region
    // at 0x310, then drives exec_ext directly. The in-memory table round-trips and
    // the game-visible store values follow the spec (save→1, restore→bytes-read).
    fn aux_machine() -> Machine {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        // name string "AB" at 0x280: [len=2]['A']['B']
        // (0x280 is safely below global_vars=0x300, avoiding a collision
        // when the store variable 0x10 = G0 writes to 0x300.)
        m.mem.write_byte(0x280, 2);
        m.mem.write_byte(0x281, b'A');
        m.mem.write_byte(0x282, b'B');
        // data region at 0x310: 0xDE 0xAD 0xBE 0xEF
        for (i, b) in [0xDE, 0xAD, 0xBE, 0xEF].into_iter().enumerate() {
            m.mem.write_byte(0x310 + i as u32, b);
        }
        m
    }

    #[test]
    fn aux_save_table_stores_one_and_fills_table() {
        let mut m = aux_machine();
        // save table=0x310 bytes=4 name=0x280 -> store G0
        let r = m.exec_ext(0x00, &[0x310, 4, 0x280], Some(0x10), None);
        assert_eq!(r, StepResult::Continue, "aux save never suspends");
        assert_eq!(m.global(0), 1, "aux save stores 1 (success)");
        assert!(m.aux_dirty, "aux save marks the table dirty");
        assert_eq!(m.aux_data.get("AB").map(|v| v.as_slice()), Some(&[0xDE,0xAD,0xBE,0xEF][..]));
    }

    #[test]
    fn aux_restore_table_round_trips_and_stores_count() {
        let mut m = aux_machine();
        m.exec_ext(0x00, &[0x310, 4, 0x280], Some(0x10), None); // save first
        // clobber the region
        for i in 0..4 { m.mem.write_byte(0x310 + i, 0); }
        // restore table=0x310 bytes=4 name=0x280 -> store G0
        let r = m.exec_ext(0x01, &[0x310, 4, 0x280], Some(0x10), None);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(m.global(0), 4, "restore stores the number of bytes read");
        assert_eq!(m.mem.read_byte(0x310), 0xDE);
        assert_eq!(m.mem.read_byte(0x313), 0xEF);
    }

    #[test]
    fn aux_restore_missing_name_stores_zero() {
        let mut m = aux_machine();
        let r = m.exec_ext(0x01, &[0x310, 4, 0x280], Some(0x10), None);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(m.global(0), 0, "restoring an unsaved name stores 0");
    }

    #[test]
    fn aux_save_out_of_bounds_does_not_panic() {
        let mut m = aux_machine();
        let huge = (m.mem.len() as u16).wrapping_sub(2);
        // table near EOF, bytes huge, name near EOF -- must clamp, not panic.
        let r = m.exec_ext(0x00, &[huge, 0xFFFF, huge], Some(0x10), None);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(m.global(0), 1);
    }

    // ── mouse input: set_mouse / read_mouse (EXT:0x16) / mouse_window (EXT:0x17) ──

    // Machine with a header extension table of `count` words at 0x340, its byte
    // address planted in header word 0x36.
    fn mouse_machine(count: u16) -> Machine {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.mem.write_word(0x36, 0x0340); // header extension table address (bytes)
        m.mem.write_word(0x0340, count); // word 0: number of further words
        m
    }

    #[test]
    fn set_mouse_writes_header_extension_x_then_y() {
        // ZMSD §11 extension table: word 1 = X-coordinate, word 2 = Y-coordinate.
        let mut m = mouse_machine(2);
        m.set_mouse(10, 172, 1); // (y, x, buttons)
        assert_eq!(m.mem.read_word(0x0342), 172, "ext word 1 = mouse X");
        assert_eq!(m.mem.read_word(0x0344), 10, "ext word 2 = mouse Y");
    }

    #[test]
    fn set_mouse_short_table_writes_only_available_words() {
        // count=1 -> only word 1 (X) exists; word 2 (Y) must not be written.
        let mut m = mouse_machine(1);
        m.mem.write_word(0x0344, 0xBEEF); // sentinel in the (absent) Y slot
        m.set_mouse(10, 172, 1);
        assert_eq!(m.mem.read_word(0x0342), 172, "X still written");
        assert_eq!(m.mem.read_word(0x0344), 0xBEEF, "Y slot beyond table untouched");
    }

    #[test]
    fn set_mouse_no_extension_table_is_silent() {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.mem.write_word(0x36, 0); // no extension table
        m.set_mouse(10, 172, 1); // must not panic / write anywhere
        assert_eq!(m.mouse_x, 172);
        assert_eq!(m.mouse_y, 10);
    }

    #[test]
    fn read_mouse_fills_four_words_y_x_buttons_menu() {
        // ZMSD §15: array words are y, x, button bits, menu word (menu = 0 here).
        let mut m = mouse_machine(2);
        m.set_mouse(10, 172, 0b1);
        let r = m.exec_ext(0x16, &[0x0380], None, None);
        assert_eq!(r, StepResult::Continue, "read_mouse never suspends/stores");
        assert_eq!(m.mem.read_word(0x0380), 10, "word 0 = y");
        assert_eq!(m.mem.read_word(0x0382), 172, "word 1 = x");
        assert_eq!(m.mem.read_word(0x0384), 0b1, "word 2 = buttons");
        assert_eq!(m.mem.read_word(0x0386), 0, "word 3 = menu (unsupported)");
    }

    #[test]
    fn read_mouse_default_state_is_zero() {
        let mut m = mouse_machine(2);
        m.exec_ext(0x16, &[0x0380], None, None);
        for off in [0, 2, 4, 6] {
            assert_eq!(m.mem.read_word(0x0380 + off), 0, "unclicked read_mouse -> 0");
        }
    }

    #[test]
    fn mouse_window_records_constraint() {
        let mut m = mouse_machine(2);
        assert_eq!(m.mouse_window, 1, "default constraint is window 1 (ZMSD §15)");
        m.exec_ext(0x17, &[3], None, None);
        assert_eq!(m.mouse_window, 3);
        m.exec_ext(0x17, &[0xFFFF], None, None); // -1 = remove restriction
        assert_eq!(m.mouse_window, -1);
    }

    #[test]
    fn ext_save_restore_zero_operands_still_suspend() {
        let mut m = aux_machine();
        assert_eq!(m.exec_ext(0x00, &[], Some(0x10), None), StepResult::SaveRequest);
        assert_eq!(m.exec_ext(0x01, &[], Some(0x10), None), StepResult::RestoreRequest);
    }

    // ── Font 3 character-graphics translation (EXT:0x04 set_font + print_text) ──

    /// Helper: drain the lower-window BufferOutput.
    fn buf_output(m: &Machine) -> &str {
        m.out.as_any().downcast_ref::<BufferOutput>().unwrap().buf.as_str()
    }

    #[test]
    fn font3_translates_up_down_arrows_lower_window() {
        // Codes 92 ('\') → ↑ (U+2191) and 93 (']') → ↓ (U+2193) in Font 3.
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        assert_eq!(m.screen.current_font, 1, "default font is 1");
        m.screen.current_font = 3;
        m.streams.stream1 = true;
        m.print_text("\\]"); // ASCII 92, 93
        assert_eq!(buf_output(&m), "\u{2191}\u{2193}", "font 3: '\\' → ↑ and ']' → ↓");
    }

    #[test]
    fn font1_output_byte_identical_no_translation() {
        // With font 1 (default), print_text must be byte-identical — no translation.
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        assert_eq!(m.screen.current_font, 1);
        m.streams.stream1 = true;
        m.print_text("\\]"); // ASCII 92, 93
        assert_eq!(buf_output(&m), "\\]", "font 1: output must be byte-identical");
    }

    #[test]
    fn font3_translates_upper_window_cells() {
        // Same arrow codes through the upper-window grid path.
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        m.screen.upper.resize(2, 10);
        m.screen.current_window = 1;
        m.screen.cursor_row = 1;
        m.screen.cursor_col = 1;
        m.screen.current_font = 3;
        m.print_text("\\]");
        assert_eq!(m.screen.upper.cell(1, 1).ch, '\u{2191}', "col 1 → ↑");
        assert_eq!(m.screen.upper.cell(1, 2).ch, '\u{2193}', "col 2 → ↓");
    }

    #[test]
    fn set_font_tracks_current_font_and_returns_previous() {
        let mem = Memory::new(sample_story(5)).unwrap();
        let mut m = Machine::new(mem);
        // Default font is 1; set_font(3) should return 1 (previous) and switch to 3.
        m.exec_ext(0x04, &[3], Some(0x10), None);
        assert_eq!(m.global(0), 1, "set_font(3) returns previous font 1");
        assert_eq!(m.screen.current_font, 3, "current_font updated to 3");
        // Query with 0: returns current (3) without changing.
        m.exec_ext(0x04, &[0], Some(0x10), None);
        assert_eq!(m.global(0), 3, "set_font(0) returns current font 3");
        assert_eq!(m.screen.current_font, 3, "current_font unchanged after query");
        // Unsupported font: returns 0 (unavailable).
        m.exec_ext(0x04, &[2], Some(0x10), None);
        assert_eq!(m.global(0), 0, "set_font(2) returns 0 (unavailable)");
        assert_eq!(m.screen.current_font, 3, "current_font unchanged after failed set");
    }

    // ── CP437 print_char translation under interpreter number 6 (IBM PC) ──────

    /// VAR:0x05 print_char with the given ZSCII operand, into the lower window.
    fn run_print_char(m: &mut Machine, zscii: u16) {
        m.streams.stream1 = true;
        m.exec_var(0x05, &[zscii], None, None);
    }

    #[test]
    fn print_char_cp437_under_ibm_pc() {
        // Beyond Zork's menu arrows (0x18 ↑ / 0x19 ↓) and the map's box-drawing
        // (0xDA ┌, 0xC4 ─) come through print_char as raw CP437 bytes when the
        // interpreter number is 6 (IBM PC).
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.mem.write_byte(0x1E, 6); // IBM PC
        run_print_char(&mut m, 0x18);
        run_print_char(&mut m, 0x19);
        run_print_char(&mut m, 0xDA);
        run_print_char(&mut m, 0xC4);
        run_print_char(&mut m, 0x82); // é
        assert_eq!(buf_output(&m), "\u{2191}\u{2193}\u{250C}\u{2500}\u{00E9}");
    }

    #[test]
    fn print_char_ascii_and_newline_unaffected_under_ibm_pc() {
        // ASCII passes through (CP437 0x20–0x7E == ASCII); newline stays a newline.
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.mem.write_byte(0x1E, 6);
        for c in b"Hi" { run_print_char(&mut m, *c as u16); }
        run_print_char(&mut m, 13); // newline, NOT CP437 ♪
        run_print_char(&mut m, b'!' as u16);
        assert_eq!(buf_output(&m), "Hi\n!");
    }

    #[test]
    fn print_char_spacing_codes_survive_cp437_under_ibm_pc() {
        // ZMSD §3.8: tab (9) and SENTENCE SPACE (11) are defined for output in
        // Version 6 — 11 is "a suitable gap between two sentences" — and 10 is
        // the invisible spacer Beyond Zork uses. They mean SPACING, not glyphs,
        // so the CP437 table must never claim them. Every v6 story gets
        // interpreter 6 by default, and Shogun prints 11 between its sentences,
        // so before this Shogun's cabin read "…sea chest here.♂Sitting on the
        // desk…" — CP437's 0x0B glyph (user report at the TTY, 2026-07-28).
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.mem.write_byte(0x1E, 6); // IBM PC → CP437 active
        run_print_char(&mut m, b'.' as u16);
        run_print_char(&mut m, 11); // sentence space, NOT ♂ (U+2642)
        run_print_char(&mut m, b'S' as u16);
        run_print_char(&mut m, 9); // tab, NOT ○
        run_print_char(&mut m, 10); // invisible spacer, NOT ◙
        run_print_char(&mut m, b'!' as u16);
        // '.' + one space (11) + 'S' + two spaces (9, 10) + '!'
        assert_eq!(buf_output(&m), ". S  !");
        assert!(!buf_output(&m).contains('\u{2642}'), "no CP437 ♂ for the sentence space");
    }

    #[test]
    fn print_char_zscii_zero_is_a_no_op() {
        // ZSCII 0 has no printed form (ZMSD §3.8) and must contribute nothing
        // to any output stream — not even a '?' placeholder.
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        run_print_char(&mut m, 65); // 'A'
        run_print_char(&mut m, 0);
        run_print_char(&mut m, 66); // 'B'
        assert_eq!(buf_output(&m), "AB");
    }

    #[test]
    fn print_char_not_cp437_under_other_interpreter() {
        // Under a non-IBM-PC interpreter number the bytes are NOT CP437-mapped:
        // standard ZSCII applies, so the graphics codes fall back to '?'.
        let mut m = Machine::new(Memory::new(sample_story(5)).unwrap());
        m.mem.write_byte(0x1E, 4); // Amiga (takes the Font 3 path, not CP437)
        run_print_char(&mut m, 0x18);
        run_print_char(&mut m, 0x82);
        assert_eq!(buf_output(&m), "??", "non-IBM-PC: graphics codes stay standard ZSCII ('?')");
    }

    // ── Task 3: StepResult::Fault + stack trace capture ──

    #[test]
    fn loadw_out_of_bounds_faults_with_trace() {
        // loadw (2OP:0x0F), variable-form encoding with two Large operands:
        // array=0xFFFF index=0xFFFF -> addr = 0xFFFF + 2*0xFFFF = 0x2FFFD,
        // far past the 0x400-byte sample story's memory.
        let mut buf = sample_story(5);
        buf[0x10] = 0xCF; // variable form, bit5=0 -> 2OP, opcode=0x0F (loadw)
        buf[0x11] = 0x0F; // type byte: large, large, omitted, omitted
        buf[0x12] = 0xFF; buf[0x13] = 0xFF; // operand a (array) = 0xFFFF
        buf[0x14] = 0xFF; buf[0x15] = 0xFF; // operand b (index) = 0xFFFF
        buf[0x16] = 0x00; // store var 0x00 = push onto stack
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        // A non-empty call stack so the trace has at least one frame.
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0,
            locals: vec![],
            eval_base: 0,
            store_var: None,
            arg_count: 0,
            func_addr: 0,
        });

        let start_pc = m.state.pc;
        let r = m.step();
        assert_eq!(r, StepResult::Fault);
        let t = m.take_fault_trace().expect("fault trace present");
        assert!(t.fault.starts_with("memory fault: read16 @"), "fault: {}", t.fault);
        assert_eq!(t.fault_op, "loadw");
        assert_eq!(t.fault_pc, start_pc, "fault_pc is the instruction start, not next_pc");
        assert_eq!(t.width, 2);
        assert!(!t.frames.is_empty());
    }

    #[test]
    fn mem_fault_drains_state_fault_latch_too() {
        // A single instruction that fires BOTH latches in one step():
        // operand a is a Variable reference to local var 2, resolved with an
        // empty call-frame stack — read_var's 0x01..=0x0F arm sets
        // state.fault ("stack underflow") and yields 0 as the operand value.
        // operand b is the Large constant 0xFFFF, so addr = 0 + 2*0xFFFF =
        // 0x1FFFE, far past the 0x400-byte sample story — read_word latches
        // mem.mem_fault too. Under the old early-return code, the mem-fault
        // branch returns before draining state.fault, leaking "stack
        // underflow" into the NEXT step() and misattributing it there.
        let mut buf = sample_story(5);
        buf[0x10] = 0xCF; // variable form, bit5=0 -> 2OP, opcode=0x0F (loadw)
        buf[0x11] = 0x8F; // types: variable, large, omitted, omitted
        buf[0x12] = 0x02; // operand a: Variable ref to local var 2
        buf[0x13] = 0xFF; buf[0x14] = 0xFF; // operand b (large const) = 0xFFFF
        buf[0x15] = 0x00; // store var 0x00 = push onto stack
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        // Deliberately no call frame pushed, so the Variable operand resolves
        // through the frames-empty arm of read_var and sets state.fault.

        let r = m.step();
        assert_eq!(r, StepResult::Fault);
        let t = m.take_fault_trace().expect("fault trace present");
        // Mem fault takes precedence in the reported trace.
        assert!(t.fault.starts_with("memory fault:"), "fault: {}", t.fault);
        // But the state-fault latch must ALWAYS be drained, or it leaks into
        // the next step() and misattributes a fault to a later instruction.
        assert!(m.state.fault.is_none(), "state.fault leaked past step()");
    }

    #[test]
    fn clean_quit_produces_no_fault_trace() {
        let mut buf = sample_story(5);
        buf[0x10] = 0xBA; // quit (0OP:0x0A short form)
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;

        let r = m.step();
        assert_eq!(r, StepResult::Quit);
        assert!(m.take_fault_trace().is_none());
    }

    /// A v6 machine with one base frame (eval_base 0) for exec-body tests.
    fn v6_exec_machine() -> Machine {
        let mut m = Machine::new(Memory::new(
            crate::header::tests_support::sample_story(6)).unwrap());
        // sample_story(6) boots by "calling main" at a garbage addr → may push 0 or
        // 1 frame; normalise to exactly one clean frame for deterministic stores.
        m.state.frames.clear();
        m.state.eval_stack.clear();
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0, locals: vec![0; 4], eval_base: 0,
            store_var: None, arg_count: 0, func_addr: 0,
        });
        m
    }

    #[test]
    fn v6_get_wind_prop_reads_default_zero() {
        let mut m = v6_exec_machine();
        // pre-set local 1 to a nonzero value
        crate::cpu::state::write_var(&mut m.state, &mut m.mem, 0x01, 0xBEEF);
        m.exec_ext(0x13, &[1, 2], Some(0x01), None); // get_wind_prop win=1 prop=2 -> L1
        // window 1's y-size (prop 2) defaults to 0 until put_wind_prop/window_size sets it.
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 0);
    }

    #[test]
    fn v6_true_colour_props_16_17_report_the_actual_colours() {
        // ZMSD §8.8.3.2.8: props 16/17 "show the actual colour being used for
        // the foreground and background, whether it was set using set_colour or
        // set_true_colour".
        let mut m = v6_exec_machine();
        let read = |m: &mut Machine, prop: u16| -> u16 {
            m.exec_ext(0x13, &[1, prop], Some(0x01), None);
            crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01)
        };

        // Untouched window: both channels are Default → the interpreter's own
        // defaults (2 black / 9 white), as §8.3.1 values.
        assert_eq!(read(&mut m, 16), 0x7FFF, "default fg = white");
        assert_eq!(read(&mut m, 17), 0x0000, "default bg = black");

        // set_colour with standard numbers → the §8.3.1 table values.
        m.exec_2op(0x1B, &[3, 6, 1], None, None); // fg=red, bg=blue, window 1
        assert_eq!(read(&mut m, 16), 0x001D, "red");
        assert_eq!(read(&mut m, 17), 0x59A0, "blue");

        // set_true_colour → the exact 15-bit values back.
        m.exec_ext(0x0D, &[0x1234, 0x0456, 1], None, None);
        assert_eq!(read(&mut m, 16), 0x1234);
        assert_eq!(read(&mut m, 17), 0x0456);

        // §8.8.3.2: "must not be written by put_wind_prop".
        m.exec_ext(0x19, &[1, 16, 0x7FFF], None, None);
        m.exec_ext(0x19, &[1, 17, 0x7FFF], None, None);
        assert_eq!(read(&mut m, 16), 0x1234, "prop 16 is not writeable");
        assert_eq!(read(&mut m, 17), 0x0456, "prop 17 is not writeable");
    }

    #[test]
    fn v6_colour_data_prop_11_reports_16_or_more_for_true_colours() {
        // ZMSD §8.3.5.1/.2: a standard colour shows as itself in property 11;
        // a non-standard (true) colour shows as >= 16.
        let mut m = v6_exec_machine();
        m.exec_2op(0x1B, &[3, 6, 1], None, None); // standard red on blue
        assert_eq!(
            m.screen.v6.as_ref().unwrap().windows[1].get_prop(11),
            0x0603,
            "standard colours report their own numbers (bg high, fg low)"
        );

        m.exec_ext(0x0D, &[0x1234, 0x0456, 1], None, None); // true colours
        let data = m.screen.v6.as_ref().unwrap().windows[1].get_prop(11);
        assert!(data & 0xFF >= 16, "true fg reports >= 16, got {}", data & 0xFF);
        assert!(data >> 8 >= 16, "true bg reports >= 16, got {}", data >> 8);

        // Stable: re-selecting the same colours reads back the same numbers.
        m.exec_ext(0x0D, &[0x7FFF, 0x7FFF, 1], None, None);
        m.exec_ext(0x0D, &[0x1234, 0x0456, 1], None, None);
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[1].get_prop(11), data, "stable numbering");
    }

    // ── Task 6: get_wind_prop / put_wind_prop over the property array ───────

    #[test]
    fn v6_get_put_wind_prop_round_trip() {
        let mut m = v6_exec_machine();
        // put via opcode: window 1, prop 2 (y-size) = 40
        m.exec_ext(0x19, &[1, 2, 40], None, None); // put_wind_prop(win, prop, val)
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[1].get_prop(2), 40);

        // get via opcode: window 1, prop 2 -> store
        crate::cpu::state::write_var(&mut m.state, &mut m.mem, 0x01, 0xBEEF);
        m.exec_ext(0x13, &[1, 2], Some(0x01), None); // get_wind_prop(win, prop) -> L1
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 40);
    }

    #[test]
    fn v6_put_wind_prop_out_of_range_window_is_ignored() {
        let mut m = v6_exec_machine();
        // window 8 is out of range (table is 0-7) — must not panic, must be a no-op.
        let before: Vec<u16> =
            m.screen.v6.as_ref().unwrap().windows.iter().map(|w| w.get_prop(2)).collect();
        m.exec_ext(0x19, &[8, 2, 99], None, None);
        let after: Vec<u16> =
            m.screen.v6.as_ref().unwrap().windows.iter().map(|w| w.get_prop(2)).collect();
        assert_eq!(before, after, "no window was touched by an out-of-range put");
        assert!(!after.contains(&99), "the out-of-range value landed nowhere");
    }

    #[test]
    fn v6_get_wind_prop_out_of_range_window_stores_zero() {
        let mut m = v6_exec_machine();
        crate::cpu::state::write_var(&mut m.state, &mut m.mem, 0x01, 0xBEEF);
        m.exec_ext(0x13, &[8, 2], Some(0x01), None); // window 8 out of range
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 0);
    }

    #[test]
    fn v6_set_colour_minus_one_is_transparent_not_keep() {
        // ZMSD §8.3.4: in v6, colour -1 = transparent (the pixel under the
        // cursor). Zork0 prints banner labels under COLOR 1 -1 so text draws
        // over the ribbon art — a "keep" reading here left an earlier explicit
        // black bg active and painted boxes over the banner.
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[1], None, None); // current = 1
        m.exec_2op(0x1B, &[4, 2, 1], None, None); // COLOR 4 2 (explicit green on black)
        {
            let w = &m.screen.v6.as_ref().unwrap().windows[1];
            assert_eq!(w.bg, ZColour::Standard(2), "explicit bg stored");
        }
        m.exec_2op(0x1B, &[1, (-1i16) as u16, 1], None, None); // COLOR 1 -1
        let w = &m.screen.v6.as_ref().unwrap().windows[1];
        assert_eq!(w.fg, ZColour::Default, "fg 1 = default");
        assert_eq!(w.bg, ZColour::Default, "bg -1 = transparent → inherited Default, NOT the kept black");
    }

    #[test]
    fn v6_draw_picture_stamps_out_chars_and_attaches_following_margin() {
        // The event records how many chars had been printed to window 0 (its
        // anchor in the text stream), and a set_margins directly after the draw
        // on the same window attaches its left value (the inline-picture idiom).
        let mut m = v6_exec_machine();
        m.print_text("hello\n"); // 6 chars to window 0
        m.exec_ext(0x05, &[7, 3, 3], None, None); // draw_picture(7, y=3, x=3)
        m.exec_ext(0x08, &[56, 0, 0], None, None); // set_margins(left=56, right=0, win=0)
        {
            let ev = m.pending_pictures.last().unwrap();
            assert_eq!(ev.out_chars, 6, "event anchored after the 6 printed chars");
            assert_eq!(ev.margin_after, Some(56), "following set_margins attached");
        }
        // A LATER set_margins must not overwrite the attached value.
        m.exec_ext(0x08, &[32, 0, 0], None, None);
        assert_eq!(m.pending_pictures.last().unwrap().margin_after, Some(56));
    }

    #[test]
    fn v6_draw_picture_zero_coords_resolve_to_window_cursor() {
        // ZMSD §15: zero y/x means the cursor coordinate in the current window
        // (the cursor is stored in 1-based pixels and used verbatim).
        let mut m = v6_exec_machine();
        m.screen.v6.as_mut().unwrap().windows[0].y_cursor = 9;
        m.screen.v6.as_mut().unwrap().windows[0].x_cursor = 17;
        m.exec_ext(0x05, &[7, 0, 0], None, None);
        let ev = m.pending_pictures.last().unwrap();
        assert_eq!((ev.y, ev.x), (9, 17));
    }

    #[test]
    fn v6_set_margins_stores_and_snaps_cursor() {
        let mut m = v6_exec_machine();
        // Cursor sits at pixel 9, inside the new 20px left margin.
        m.screen.v6.as_mut().unwrap().windows[1].x_cursor = 9;
        m.exec_ext(0x08, &[20, 8, 1], None, None); // set_margins(left, right, window)
        let w = &m.screen.v6.as_ref().unwrap().windows[1];
        assert_eq!(w.left_margin, 20, "left margin stored (prop 6)");
        assert_eq!(w.right_margin, 8, "right margin stored (prop 7)");
        assert_eq!(w.x_cursor, 21, "cursor snapped forward to the new left margin (px)");
    }

    #[test]
    fn v6_set_margins_snaps_cursor_past_right_edge() {
        // ZMSD §15: the cursor is snapped back to the left margin when it lies
        // outside the margins on EITHER side. Shogun's opening flows text past a
        // right-placed picture with a large right margin; a cursor left beyond the
        // new text column (x_size - right) must snap home. (matches Frotz's
        // two-sided `x_cursor <= left || x_cursor > x_size - right`.)
        let mut m = v6_exec_machine();
        m.screen.v6.as_mut().unwrap().windows[0].x_size = 548;
        // Cursor at pixel 300 — past the new text column (548 - 328 = 220).
        m.screen.v6.as_mut().unwrap().windows[0].x_cursor = 300;
        m.exec_ext(0x08, &[2, 328, 0], None, None); // set_margins(left=2, right=328, win=0)
        let w = &m.screen.v6.as_ref().unwrap().windows[0];
        assert_eq!(w.right_margin, 328, "right margin stored (prop 7)");
        assert_eq!(w.x_cursor, 3, "cursor past the right edge snapped to left margin+1 (px)");
    }

    #[test]
    fn v6_set_margins_out_of_range_window_is_ignored() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x08, &[10, 10, 8], None, None); // window 8 out of range
        for w in &m.screen.v6.as_ref().unwrap().windows {
            assert_eq!(w.left_margin, 0, "no window touched by an out-of-range set_margins");
        }
    }

    #[test]
    fn v6_put_wind_prop_all_16_props_readable_back() {
        let mut m = v6_exec_machine();
        for n in 0..16u16 {
            m.exec_ext(0x19, &[3, n, 100 + n], None, None);
        }
        let w = &m.screen.v6.as_ref().unwrap().windows[3];
        for n in 0..16u16 {
            assert_eq!(w.get_prop(n), 100 + n, "prop {n}");
        }
    }

    /// Position window `win` at `(y, x)` with size `h`×`w` px and select it.
    fn v6_place_window(m: &mut Machine, win: u8, y: u16, x: u16, h: u16, w: u16) {
        m.exec_ext(0x10, &[win as u16, y, x], None, None); // move_window
        m.exec_ext(0x11, &[win as u16, h, w], None, None); // window_size
        m.exec_var(0x0B, &[win as u16], None, None); // set_window
    }

    #[test]
    fn v6_paint_runs_are_screen_absolute_and_survive_window_moves() {
        // Shogun prints its boot menu into window 2 while it is wide at
        // (24,169), then MOVES it to (159,169) and shrinks it to 1 px as a
        // caret. "window_size does not change the current display" (ZMSD §15):
        // the painted runs keep their screen positions.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 2, 169, 24, 24, 274);
        m.print_text("START the game");
        m.exec_ext(0x10, &[2, 169, 159], None, None); // move_window
        m.exec_ext(0x11, &[2, 24, 1], None, None); // window_size 1px wide
        let w2 = &m.screen.v6.as_ref().unwrap().windows[2];
        assert_eq!(w2.texts.len(), 1);
        assert_eq!((w2.texts[0].y, w2.texts[0].x), (169, 24), "run keeps its painted screen position");
        assert_eq!(w2.texts[0].text, "START the game");
    }

    #[test]
    fn v6_win0_wrap_off_routes_output_to_paint_runs() {
        // Zork Zero's hint menu clears window 0's wrapping attribute
        // (window_style op 2) and paints topics via set_cursor, one row per
        // item. With wrapping OFF, win0 output is positioned PAINT like
        // windows 1-7 — runs at the cursor, nothing to the stream (the flat
        // transcript strung all topics together). Restoring wrap resumes
        // streaming.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 0, 17, 1, 160, 320);
        m.exec_var(0x0B, &[0], None, None); // set_window(0)
        m.exec_ext(0x12, &[0, 1, 2], None, None); // window_style(0, wrapping, CLEAR)
        // Menu rows are one 16px cell apart (set_cursor y is in units/pixels):
        // row 9 then row 25 so the two 16px-tall glyph rows don't overlap-trim.
        m.exec_var(0x0F, &[9, 1, 0], None, None); // set_cursor(row 9, col 1, win 0)
        m.print_text("PROLOGUE");
        m.exec_var(0x0F, &[25, 1, 0], None, None);
        m.print_text("EAST WING");
        {
            let w0 = &m.screen.v6.as_ref().unwrap().windows[0];
            let mut runs: Vec<(u16, u16, &str)> =
                w0.texts.iter().map(|t| (t.y, t.x, t.text.as_str())).collect();
            runs.sort();
            assert_eq!(
                runs,
                vec![(25, 1, "PROLOGUE"), (41, 1, "EAST WING")],
                "menu items are positioned paint runs (abs y = win 17 + cursor - 1)"
            );
        }
        // Restore wrapping: output streams again, runs stay untouched.
        m.exec_ext(0x12, &[0, 1, 1], None, None); // window_style(0, wrapping, OR)
        m.print_text("back to prose");
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[0].texts.len(), 2, "streamed output adds no runs");
    }

    #[test]
    fn v6_erase_line_erases_to_window_right_edge() {
        // ZMSD §15 erase_line (v6): value 1 erases from the cursor to the end
        // of the line in the current window; value n erases n-1 pixels. The
        // hint menu repaints highlighted items over erase_line'd rows — a
        // no-op left stale tails of longer items behind.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 1, 1, 1, 16, 320);
        m.print_text("GENERAL QUESTIONS");
        m.exec_var(0x0F, &[1, 1, 1], None, None); // cursor back to row 1 col 1
        m.exec_var(0x0E, &[1], None, None); // erase_line(1): to end of line
        assert!(
            m.screen.v6.as_ref().unwrap().windows[1].texts.is_empty(),
            "erase_line(1) clears the painted row: {:?}",
            m.screen.v6.as_ref().unwrap().windows[1].texts
        );
        // Partial form: erase exactly the first glyph's 8 px.
        m.print_text("ABC");
        m.exec_var(0x0F, &[1, 1, 1], None, None);
        m.exec_var(0x0E, &[9], None, None); // erase 8 px
        let joined: String =
            m.screen.v6.as_ref().unwrap().windows[1].texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(joined, "BC", "erase_line(9) erases 8 px = one glyph");
    }

    #[test]
    fn v6_erase_line_is_clipped_inside_the_right_margin() {
        // ZMSD §15 erase_line (v6): the erase is "clipped to stay inside the
        // right margin" — it used to run to the raw window edge, wiping text
        // that lives in the margin strip.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 1, 1, 1, 16, 320); // x 1..320
        m.exec_ext(0x08, &[0, 160, 1], None, None); // set_margins(left=0, right=160)
        m.exec_var(0x0F, &[1, 1, 1], None, None); // cursor to (1,1)
        m.print_text("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcd"); // 40 glyphs = 320 px
        m.exec_var(0x0F, &[1, 1, 1], None, None);
        m.exec_var(0x0E, &[1], None, None); // erase_line(1): to end of line
        let joined: String =
            m.screen.v6.as_ref().unwrap().windows[1].texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            joined, "UVWXYZ0123456789abcd",
            "erase stops at the right margin (320 - 160 px), leaving the margin strip"
        );
    }

    #[test]
    fn v6_stream3_close_stores_text_width_in_header_30() {
        // ZMSD §7.1.2.1: v6 stream-3 deselect stores "the total width of
        // printing (in units)" at header $30. Infocom games measure string
        // widths this way (Shogun right-aligns and centres its whole status
        // line off $30 readbacks).
        let mut m = v6_exec_machine();
        m.exec_var(0x13, &[3, 0x0100], None, None); // output_stream 3 -> table
        m.print_text("Score:");
        m.exec_var(0x13, &[0xFFFD], None, None); // output_stream -3 (close)
        assert_eq!(m.mem.read_word(0x0100), 6, "table word 0 = char count");
        assert_eq!(
            m.mem.read_word(0x30),
            6 * crate::screen::V6_FONT_WIDTH,
            "header $30 = printed width in units"
        );
    }

    #[test]
    fn v6_output_stream3_width_operand_resolves_window_number_to_pixels() {
        // ZMSD §15 output_stream: "In Version 6, a width field may optionally
        // be given: text will then be justified as if it were in the window
        // with that number (if width is zero or positive) ...". window 2's
        // x_size (40px = 5 chars) is too narrow for "AAAA BBBB" (72px) on one
        // line, so the wrap point must land between the words.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 2, 0, 0, 10, 40); // window 2, x_size=40px
        m.exec_var(0x13, &[3, 0x0100, 2], None, None); // output_stream 3 -> table, width=window 2
        m.print_text("AAAA BBBB");
        m.exec_var(0x13, &[0xFFFD], None, None); // output_stream -3 (close)
        assert_eq!(m.mem.read_word(0x0100), 9, "byte count unchanged (space -> newline)");
        let bytes: Vec<u8> = (0..9).map(|i| m.mem.read_byte(0x0100 + 2 + i)).collect();
        assert_eq!(bytes, b"AAAA\rBBBB", "wraps at the window-2 width");
    }

    #[test]
    fn v6_output_stream3_negative_width_operand_is_literal_pixel_box() {
        // ZMSD §15 output_stream: "... or a box -width pixels wide (if
        // negative)." A negative operand is the box width directly, no
        // window lookup — here -40 means a 40px box, same wrap as above.
        let mut m = v6_exec_machine();
        m.exec_var(0x13, &[3, 0x0100, 0xFFD8], None, None); // width = -40 (0xFFD8)
        m.print_text("AAAA BBBB");
        m.exec_var(0x13, &[0xFFFD], None, None);
        let bytes: Vec<u8> = (0..9).map(|i| m.mem.read_byte(0x0100 + 2 + i)).collect();
        assert_eq!(bytes, b"AAAA\rBBBB", "negative operand wraps to -width pixels, no window lookup");
    }

    #[test]
    fn v6_window_operand_minus_three_is_current_window() {
        // ZMSD §8.8.3.2 / frotz winarg0: window -3 (0xFFFD) = the currently
        // selected window. Shogun reads its status-line cursor via
        // get_wind_prop(-3, 4/5); an unresolved -3 returned 0 and scrambled
        // its right-aligned layout math.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 3, 41, 25, 24, 100);
        m.screen.v6.as_mut().unwrap().windows[3].y_cursor = 7;
        m.screen.v6.as_mut().unwrap().windows[3].x_cursor = 33;
        // get_wind_prop(-3, prop) must read window 3 (the current window).
        m.exec_ext(0x13, &[0xFFFD, 4], Some(0), None); // y_cursor -> sp
        m.exec_ext(0x13, &[0xFFFD, 5], Some(0), None); // x_cursor -> sp
        assert_eq!(m.state.eval_stack.pop(), Some(33), "x_cursor via window -3");
        assert_eq!(m.state.eval_stack.pop(), Some(7), "y_cursor via window -3");
        // put_wind_prop(-3, ...) writes the current window too.
        m.exec_ext(0x19, &[0xFFFD, 15, 42], None, None); // line_count = 42
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[3].line_count, 42);
    }

    #[test]
    fn v6_set_font_mirrors_into_window_prop_12() {
        // ZMSD §15 set_font: "In Version 6, set_font has an optional window
        // parameter, as for set_colour." Changing the font for window 3 must
        // be visible via get_wind_prop(3, 12) (font_number) afterwards.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 3, 1, 1, 10, 100);
        m.exec_ext(0x04, &[3, 3], Some(0x10), None); // set_font(3, window=3)
        assert_eq!(m.global(0), 1, "returns previous font (1)");
        m.exec_ext(0x13, &[3, 12], Some(0), None); // get_wind_prop(3, font_number)
        assert_eq!(m.state.eval_stack.pop(), Some(3), "window 3's prop 12 mirrors the new font");
    }

    #[test]
    fn v6_set_font_window_minus_three_targets_current_window() {
        // -3 (0xFFFD) resolves to the currently selected window via
        // `v6_window_operand`.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 5, 1, 1, 10, 100); // selects window 5 as current
        m.exec_ext(0x04, &[4, 0xFFFD], Some(0x10), None); // set_font(4, window=-3)
        m.exec_ext(0x13, &[5, 12], Some(0), None); // get_wind_prop(5, font_number)
        assert_eq!(m.state.eval_stack.pop(), Some(4), "current window (5) gets the new font");
    }

    #[test]
    fn v6_set_text_style_mirrors_into_current_window_prop_10() {
        // ZMSD §8.8.3.2.3: "The text style is set just as in Version 4, using
        // set_text_style (which sets that for the current window). The
        // property holds the operand of that instruction (e.g. 4 for
        // italic)."
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 2, 1, 1, 10, 100); // selects window 2 as current
        m.exec_var(0x11, &[4], None, None); // set_text_style(italic=4)
        m.exec_ext(0x13, &[2, 10], Some(0), None); // get_wind_prop(2, text_style)
        assert_eq!(m.state.eval_stack.pop(), Some(4), "window 2's prop 10 mirrors the style bitmask");
    }

    #[test]
    fn v6_no_wrap_window_prints_past_its_width() {
        // Shogun's boot menu: items print into a 1-px-wide caret window
        // (wrapping OFF, the boot default — frotz seeds attribute=8) and must
        // paint rightward across the screen, clipped only at the screen edge —
        // NOT wrap at the window's own width (which turned each item into a
        // vertical column of glyphs).
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 2, 169, 159, 24, 1);
        m.print_text("START the game");
        let w2 = &m.screen.v6.as_ref().unwrap().windows[2];
        assert_eq!(w2.texts.len(), 1, "one horizontal run, got {:?}", w2.texts);
        assert_eq!((w2.texts[0].y, w2.texts[0].x), (169, 159));
        assert_eq!(w2.texts[0].text, "START the game");
    }

    #[test]
    fn v6_no_wrap_window_clips_at_screen_edge() {
        // sample_story(6) has no screen-dims written; write 320 px wide.
        let mut m = v6_exec_machine();
        m.mem.write_word(0x22, 320);
        v6_place_window(&mut m, 2, 1, 305, 24, 1);
        m.print_text("ABCDEF"); // 6 glyphs from x=305 → only 305,313 fit fully
        let w2 = &m.screen.v6.as_ref().unwrap().windows[2];
        let joined: String = w2.texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(joined, "AB", "chars past the screen edge are dropped, got {:?}", w2.texts);
    }

    #[test]
    fn v6_wrapping_attribute_restores_window_width_wrap() {
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 1, 1, 1, 32, 32); // 4 cols wide
        m.exec_ext(0x12, &[1, 1, 0], None, None); // window_style(win1, wrapping, set)
        m.print_text("ABCDEF");
        let w1 = &m.screen.v6.as_ref().unwrap().windows[1];
        assert_eq!(w1.texts.len(), 2, "wrapped into two runs: {:?}", w1.texts);
        assert_eq!((w1.texts[0].y, w1.texts[0].x, w1.texts[0].text.as_str()), (1, 1, "ABCD"));
        // The wrap advances the cursor one 16px cell down (was 8): y = 1 + 16.
        assert_eq!((w1.texts[1].y, w1.texts[1].x, w1.texts[1].text.as_str()), (17, 1, "EF"));
    }

    #[test]
    fn v6_windows_boot_with_frotz_attributes() {
        // frotz restart_screen: every window boots with attribute 8 (buffered);
        // window 0 gets 15 (wrapping+scrolling+scripting+buffering).
        let m = v6_exec_machine();
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[0].attributes, 15, "window 0 attributes");
        for i in 1..8 {
            assert_eq!(v6.windows[i].attributes, 8, "window {i} attributes");
        }
    }

    #[test]
    fn v6_overprint_replaces_covered_glyphs() {
        // Shogun re-prints its status at the same pixel cursor every turn; the
        // new glyphs must replace the old ones, not stack on top of them.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 1, 1, 1, 16, 320);
        m.print_text("Bridge");
        // Reset the cursor to the same spot and overprint.
        m.exec_var(0x0F, &[1, 1], None, None); // set_cursor(y=1, x=1)
        m.print_text("Chapel");
        let w1 = &m.screen.v6.as_ref().unwrap().windows[1];
        let joined: String = w1.texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(joined, "Chapel", "old run fully covered → removed, got runs: {:?}", w1.texts);
    }

    #[test]
    fn v6_overprint_trims_partial_overlap_into_remnant() {
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 1, 1, 1, 16, 320);
        m.print_text("ABCDEF");
        // Overprint just the first three glyph cells.
        m.exec_var(0x0F, &[1, 1], None, None);
        m.print_text("xyz");
        let w1 = &m.screen.v6.as_ref().unwrap().windows[1];
        let mut runs: Vec<(u16, String)> = w1.texts.iter().map(|t| (t.x, t.text.clone())).collect();
        runs.sort();
        assert_eq!(
            runs,
            vec![(1, "xyz".into()), (1 + 3 * crate::screen::V6_FONT_WIDTH, "DEF".into())],
            "right remnant keeps its screen x"
        );
    }

    #[test]
    fn v6_erase_window_erases_rect_and_emits_canvas_clear() {
        // erase_window paints background over the window's CURRENT rect only:
        // Shogun erases its 1-px caret window without disturbing the menu
        // items painted around it — but erasing the big window takes them out.
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 2, 169, 24, 24, 274);
        m.print_text("START the game");
        // Shrink to the 1-px caret and erase it: runs survive minus one glyph
        // column at most.
        m.exec_ext(0x10, &[2, 169, 159], None, None);
        m.exec_ext(0x11, &[2, 24, 1], None, None);
        m.pending_pictures.clear();
        m.exec_var(0x0D, &[2], None, None); // erase_window(2)
        {
            let w2 = &m.screen.v6.as_ref().unwrap().windows[2];
            let joined: String = w2.texts.iter().map(|t| t.text.as_str()).collect();
            assert!(
                joined.contains("START"),
                "menu items painted earlier must survive the caret-rect erase, got {joined:?}"
            );
            assert_eq!(
                m.pending_pictures.last(),
                Some(&PictureEvent { number: 0, window: 2, x: 1, y: 1, erase: true, out_chars: 0, margin_after: None }),
                "erase_window rides the picture queue as a number-0 erase"
            );
        }
        // Now grow the window back over the menu and erase: runs go away.
        m.exec_ext(0x10, &[2, 169, 24], None, None);
        m.exec_ext(0x11, &[2, 24, 274], None, None);
        m.exec_var(0x0D, &[2], None, None);
        let w2 = &m.screen.v6.as_ref().unwrap().windows[2];
        assert!(w2.texts.is_empty(), "full-rect erase removes the painted runs: {:?}", w2.texts);
    }

    #[test]
    fn v6_get_cursor_reads_current_window_pixel_cursor() {
        // Frotz z_get_cursor: in V6 the current window's pixel cursor is stored
        // verbatim (no grid conversion). Before the fix this always wrote the
        // non-v6 screen cursor fields, which v6 never updates — (0,0).
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[3], None, None); // set_window(3)
        m.screen.v6.as_mut().unwrap().windows[3].y_cursor = 57;
        m.screen.v6.as_mut().unwrap().windows[3].x_cursor = 123;
        m.exec_var(0x10, &[0x0100], None, None); // get_cursor -> array at 0x0100
        assert_eq!(m.mem.read_word(0x0100), 57, "word 0 = y cursor in pixels");
        assert_eq!(m.mem.read_word(0x0102), 123, "word 1 = x cursor in pixels");
    }

    // -----------------------------------------------------------------------
    // SQ-0535: v6 word wrap when "buffered printing" (attribute 3) is on.
    // ZMSD §8.8.3.1.2.2: "If 'buffered printing' is on, then text is wrapped
    // after the last word which could fit on a line. If not, then text is
    // wrapped after the last character that could fit." The spec's own example
    // prints "Here is an abacus" in a narrow window; with wrapping on the two
    // buffering settings must give "Here is an" / "abacus" and
    // "Here is an aba" / "cus" respectively. Infocom's v6 windows 1-7 boot with
    // wrapping OFF and window 0's prose leaves through the host path, so only a
    // synthetic window exercises this.
    // -----------------------------------------------------------------------

    /// Place `win` as a paint-regime window `cols` characters wide, with
    /// wrapping on (attribute 0) and buffered printing per `buffered`.
    fn v6_wrap_window(m: &mut Machine, win: u8, cols: u16, buffered: bool) {
        let w = cols * crate::screen::V6_FONT_WIDTH;
        v6_place_window(m, win, 1, 1, 4 * crate::screen::V6_FONT_HEIGHT, w);
        m.exec_ext(0x12, &[win as u16, 0b0001, 1], None, None); // window_style: set wrapping
        // Attribute 3 (buffering) is on for every window at boot; clear it for
        // the character-wrap half of the pair.
        if !buffered {
            m.exec_ext(0x12, &[win as u16, 0b1000, 2], None, None); // clear buffering
        }
    }

    #[test]
    fn v6_buffered_window_wraps_after_the_last_whole_word() {
        let mut m = v6_exec_machine();
        v6_wrap_window(&mut m, 2, 14, true);
        m.print_text("Here is an abacus");
        let w = &m.screen.v6.as_ref().unwrap().windows[2];
        let runs: Vec<(u16, u16, &str)> =
            w.texts.iter().map(|t| (t.y, t.x, t.text.as_str())).collect();
        assert_eq!(
            runs,
            vec![(1, 1, "Here is an"), (1 + crate::screen::V6_FONT_HEIGHT, 1, "abacus")],
            "buffered printing breaks at the space, which the break consumes"
        );
        // "abacus^" — the cursor sits just after the carried word.
        assert_eq!(w.x_cursor, 1 + 6 * crate::screen::V6_FONT_WIDTH, "cursor after 'abacus'");
    }

    #[test]
    fn v6_unbuffered_window_wraps_after_the_last_character() {
        let mut m = v6_exec_machine();
        v6_wrap_window(&mut m, 2, 14, false);
        m.print_text("Here is an abacus");
        let w = &m.screen.v6.as_ref().unwrap().windows[2];
        let runs: Vec<(u16, u16, &str)> =
            w.texts.iter().map(|t| (t.y, t.x, t.text.as_str())).collect();
        assert_eq!(
            runs,
            vec![(1, 1, "Here is an aba"), (1 + crate::screen::V6_FONT_HEIGHT, 1, "cus")],
            "with buffering off the break falls wherever the 14th column does"
        );
    }

    // -----------------------------------------------------------------------
    // SQ-0536: the v6 prose path advances the window cursor, so get_cursor
    // (ZMSD §8.8.3.2.7) reports where the text actually left it.
    // -----------------------------------------------------------------------

    #[test]
    fn v6_prose_print_advances_the_window_cursor_for_get_cursor() {
        let mut m = v6_exec_machine();
        // Window 0 boots wrap+scroll (attributes 15) → the streaming prose path.
        m.print_text("Hello");
        m.exec_var(0x10, &[0x0100], None, None); // get_cursor
        assert_eq!(m.mem.read_word(0x0100), 1, "still on the first line");
        assert_eq!(
            m.mem.read_word(0x0102),
            1 + 5 * crate::screen::V6_FONT_WIDTH,
            "x cursor advanced one glyph per printed character"
        );
        m.print_text("\n");
        m.exec_var(0x10, &[0x0100], None, None);
        assert_eq!(
            m.mem.read_word(0x0100),
            1 + crate::screen::V6_FONT_HEIGHT,
            "the new-line dropped the cursor one line"
        );
        assert_eq!(m.mem.read_word(0x0102), 1, "and returned it to the left margin");
    }

    #[test]
    fn v6_prose_cursor_stops_at_the_bottom_line_of_a_scrolling_window() {
        // frotz screen_new_line: on the last line a scrolling window scrolls
        // under a stationary cursor. Without this the cursor would walk past
        // the window (and overflow prop 4 outright in a long game).
        let mut m = v6_exec_machine();
        let fh = crate::screen::V6_FONT_HEIGHT;
        let y_size = m.screen.v6.as_ref().unwrap().windows[0].y_size;
        m.print_text(&"\n".repeat(y_size as usize / fh as usize + 20));
        let w = &m.screen.v6.as_ref().unwrap().windows[0];
        assert_eq!(w.y_cursor, y_size - fh + 1, "parked on the bottom line");
    }

    // -----------------------------------------------------------------------
    // SQ-0537: ZMSD §8.8.3.1 attribute "2: text copied to output stream 2
    // (the transcript, if selected)". Painted text used to return before any
    // stream handling at all.
    // -----------------------------------------------------------------------

    #[test]
    fn v6_attribute_2_copies_painted_text_to_stream_2() {
        let mut m = v6_exec_machine();
        v6_place_window(&mut m, 2, 1, 1, 16, 320);
        m.exec_var(0x13, &[2], None, None); // output_stream 2 (transcript on)
        m.exec_ext(0x12, &[2, 0b0100, 1], None, None); // window_style: set attribute 2
        m.print_text("copied");
        assert_eq!(m.streams.stream2_text(), "copied", "attr 2 + stream 2 → transcript");
        m.exec_ext(0x12, &[2, 0b0100, 2], None, None); // clear attribute 2
        m.print_text("silent");
        assert_eq!(
            m.streams.stream2_text(),
            "copied",
            "with attribute 2 clear the window's text stays out of the transcript"
        );
        // And deselecting stream 2 stops the copy even with the attribute set.
        m.exec_ext(0x12, &[2, 0b0100, 1], None, None);
        m.exec_var(0x13, &[(-2i16) as u16], None, None); // output_stream -2
        m.print_text("offline");
        assert_eq!(m.streams.stream2_text(), "copied", "unselected stream 2 takes nothing");
    }

    // -----------------------------------------------------------------------
    // SQ-0534 (zvm half): the per-window line count (property 15) lives.
    // ZMSD §8.8.3.2.2 "the line count is decremented on each new-line",
    // §8.8.3.2.2.3 "A line count is never decremented below -999",
    // §8.8.3.2.6 "A line count of -999 means 'never print [MORE]'".
    // -----------------------------------------------------------------------

    #[test]
    fn v6_line_count_decrements_on_each_new_line() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x19, &[0, 15, 3], None, None); // put_wind_prop(win0, line count, 3)
        m.print_text("one\ntwo\n");
        assert_eq!(m.v6_line_count(), Some(1), "two new-lines, two decrements");
        // A window without the scrolling attribute never pages, so it never
        // counts (frotz gates on `enable_scrolling`).
        v6_place_window(&mut m, 2, 1, 1, 64, 320);
        m.exec_ext(0x19, &[2, 15, 5], None, None);
        m.print_text("no\ncount\n");
        assert_eq!(m.v6_line_count(), Some(5), "non-scrolling window keeps its count");
    }

    #[test]
    fn v6_line_count_floors_at_minus_999_and_sticks_there() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x19, &[0, 15, (-998i16) as u16], None, None);
        assert!(!m.v6_suppress_more(), "-998 still pages");
        m.print_text("\n\n\n\n");
        assert_eq!(m.v6_line_count(), Some(-999), "never decremented below -999");
        assert!(m.v6_suppress_more(), "-999 means never print [MORE]");
        m.print_text("\n\n");
        assert_eq!(m.v6_line_count(), Some(-999), "and it stays parked there");
    }

    #[test]
    fn v6_line_count_reloads_on_input_but_not_on_a_timeout() {
        let mut m = v6_exec_machine();
        let full = m.screen.v6.as_ref().unwrap().windows[0].more_interval();
        m.exec_ext(0x19, &[0, 15, 1], None, None);
        m.exec_var(0x16, &[0], Some(0x01), None); // read_char (arms pending input)
        m.supply_char(0); // ZSCII 0 = timed out
        assert_eq!(m.v6_line_count(), Some(1), "a timeout is not a keystroke");
        m.exec_var(0x16, &[0], Some(0x01), None);
        m.supply_char(b'x');
        assert_eq!(m.v6_line_count(), Some(full), "a real key reloads a full screen");
    }

    #[test]
    fn v6_windows_boot_seed_screen_width() {
        // Frotz restart_screen: windows 0 and 1 boot with x_size = screen width
        // (pixels in v6); games read it back via get_wind_prop(win, 3) for
        // layout math before ever calling window_size.
        let m = v6_exec_machine();
        let v6 = m.screen.v6.as_ref().unwrap();
        let width = crate::screen::DEFAULT_SCREEN_COLS as u16 * crate::screen::V6_FONT_WIDTH;
        assert_eq!(v6.windows[0].x_size, width, "window 0 x_size = screen width px");
        assert_eq!(v6.windows[1].x_size, width, "window 1 x_size = screen width px");
        assert_eq!(v6.windows[2].x_size, 0, "other windows stay unsized (frotz)");
    }

    #[test]
    fn v6_windows_boot_with_font_number_and_size() {
        // Frotz restart_screen: every v6 window starts with font = TEXT_FONT (1)
        // and font_size = (font_height << 8) | font_width. Shogun reads the
        // width out of window prop 13 at boot to size its READ input buffer —
        // a zero here becomes max-input-length 0 and every command turns into
        // "[I beg your pardon?]".
        let m = v6_exec_machine();
        let expected = (crate::screen::V6_FONT_HEIGHT << 8) | crate::screen::V6_FONT_WIDTH;
        for (i, w) in m.screen.v6.as_ref().unwrap().windows.iter().enumerate() {
            assert_eq!(w.get_prop(12), 1, "window {i} font number");
            assert_eq!(w.get_prop(13), expected, "window {i} font size (height<<8 | width)");
        }
    }

    #[test]
    fn v6_push_stack_pop_stack_user_stack() {
        // ZMSD §15 v6 user stack: word[addr] holds the free-slot count; entries
        // fill downward from the end. Mirrors frotz z_push_stack / z_pop_stack.
        let mut m = v6_exec_machine();
        let addr: u32 = 0x0100; // dynamic memory (static base = 0x0400)
        m.mem.write_word(addr, 3); // 3 free slots
        m.exec_ext(0x18, &[0xAAAA, addr as u16], None, None); // push_stack 0xAAAA, stack=addr
        assert_eq!(m.mem.read_word(addr), 2, "free count decremented after push");
        assert_eq!(m.mem.read_word(addr + 2 * 3), 0xAAAA, "value stored at addr + 2*size");
        assert!(m.state.eval_stack.is_empty(), "user-stack push must not touch the game stack");
        m.exec_ext(0x15, &[1, addr as u16], None, None); // pop_stack 1, stack=addr
        assert_eq!(m.mem.read_word(addr), 3, "free count restored after pop");
    }

    #[test]
    fn v6_push_stack_full_stack_branches_false() {
        // A stack with 0 free slots: push does nothing and reports failure.
        let mut m = v6_exec_machine();
        let addr: u32 = 0x0100;
        m.mem.write_word(addr, 0);
        // Branch omitted here (None) — just assert no write and no game-stack use.
        m.exec_ext(0x18, &[0x55, addr as u16], None, None);
        assert_eq!(m.mem.read_word(addr), 0, "full stack unchanged");
        assert!(m.state.eval_stack.is_empty());
    }

    #[test]
    fn v6_pop_stack_game_stack_form_discards() {
        // pop_stack with a single operand still targets the game stack.
        let mut m = v6_exec_machine();
        write_var(&mut m.state, &mut m.mem, 0x00, 0x11);
        write_var(&mut m.state, &mut m.mem, 0x00, 0x22);
        m.exec_ext(0x15, &[1], None, None);
        assert_eq!(m.state.eval_stack.len(), 1, "one value discarded from the game stack");
    }

    #[test]
    fn v6_graphics_opcodes_are_noops_and_stay_continue() {
        let mut m = v6_exec_machine();
        for op in [0x08u8, 0x14, 0x16, 0x17, 0x1A, 0x1C] {
            assert!(matches!(m.exec_ext(op, &[0, 0, 0], None, None), StepResult::Continue),
                "op {op:#04x} no-op → Continue");
        }
        assert!(m.state.eval_stack.is_empty(), "no-op graphics ops must not touch the stack");
    }

    // ── Task 8: draw_picture / erase_picture → pending_pictures events ──────

    #[test]
    fn v6_draw_picture_records_event_for_current_window() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[7], None, None); // set_window(7)
        let r = m.exec_ext(0x05, &[5, 1, 1], None, None); // draw_picture(5, y=1, x=1)
        assert!(matches!(r, StepResult::Continue));
        assert_eq!(m.pending_pictures, vec![
            PictureEvent { number: 5, window: 7, x: 1, y: 1, erase: false, out_chars: 0, margin_after: None }
        ]);
    }

    #[test]
    fn v6_erase_picture_records_event_with_erase_flag() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[3], None, None); // set_window(3)
        let r = m.exec_ext(0x07, &[5, 2, 4], None, None); // erase_picture(5, y=2, x=4)
        assert!(matches!(r, StepResult::Continue));
        assert_eq!(m.pending_pictures, vec![
            PictureEvent { number: 5, window: 3, x: 4, y: 2, erase: true, out_chars: 0, margin_after: None }
        ]);
    }

    // ── Task 7: picture_data from the injected dimension table ──────────────

    #[test]
    fn v6_picture_data_reports_injected_dims() {
        let mut m = v6_exec_machine();
        m.set_picture_dims(vec![(5, 100, 60)]); // picture 5 = 100w x 60h
        let array = 0x0060u16; // 2-word array in dynamic memory
        let pc_before = m.state.pc;
        let branch = Branch { on_true: true, offset: 10, len: 1 };
        let result = m.exec_ext(0x06, &[5, array], None, Some(branch));
        assert!(matches!(result, StepResult::Continue));
        assert_eq!(m.mem.read_word(array as u32), 60, "word 0 = height");
        assert_eq!(m.mem.read_word(array as u32 + 2), 100, "word 1 = width");
        assert_eq!(m.state.pc, pc_before + 10 - 2, "picture found → branch taken");
    }

    #[test]
    fn v6_picture_data_unknown_picture_does_not_branch() {
        let mut m = v6_exec_machine();
        m.set_picture_dims(vec![(5, 100, 60)]);
        let array = 0x0060u16;
        m.mem.write_word(array as u32, 0xDEAD);
        m.mem.write_word(array as u32 + 2, 0xBEEF);
        let pc_before = m.state.pc;
        let branch = Branch { on_true: true, offset: 10, len: 1 };
        m.exec_ext(0x06, &[99, array], None, Some(branch));
        assert_eq!(m.state.pc, pc_before, "picture not found → branch not taken");
        assert_eq!(m.mem.read_word(array as u32), 0xDEAD, "array left untouched");
        assert_eq!(m.mem.read_word(array as u32 + 2), 0xBEEF, "array left untouched");
    }

    #[test]
    fn v6_picture_data_number_zero_reports_count_and_branches() {
        let mut m = v6_exec_machine();
        m.set_picture_dims(vec![(5, 100, 60), (9, 20, 30)]);
        let array = 0x0060u16;
        let pc_before = m.state.pc;
        let branch = Branch { on_true: true, offset: 10, len: 1 };
        m.exec_ext(0x06, &[0, array], None, Some(branch));
        assert_eq!(m.mem.read_word(array as u32), 2, "word 0 = number of pictures available");
        assert_eq!(m.state.pc, pc_before + 10 - 2, "pictures available → branch taken");
    }

    // ── Task 4: move_window / window_size / window_style bodies ─────────────

    #[test]
    fn v6_move_window_sets_window_coords() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x10, &[1, 6, 6], None, None); // move_window(1, y=6, x=6)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].y_coord, 6);
        assert_eq!(v6.windows[1].x_coord, 6);
    }

    #[test]
    fn v6_window_size_sets_size_and_resizes_grid() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x11, &[1, 40, 80], None, None); // window_size(1, y=40, x=80)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].y_size, 40);
        assert_eq!(v6.windows[1].x_size, 80);
        assert_eq!(v6.windows[1].grid.rows, 2, "40 / V6_FONT_HEIGHT(16) = 2 rows");
        assert_eq!(v6.windows[1].grid.cols, 10, "80 / V6_FONT_WIDTH(8) = 10 cols");
    }

    #[test]
    fn v6_window_size_clamps_hostile_dimensions() {
        // A story requesting a max-pixel window must not force a ~1 GB grid
        // allocation; the cell grid is capped (pixel sizes still stored verbatim).
        let mut m = v6_exec_machine();
        m.exec_ext(0x11, &[1, 0xFFFF, 0xFFFF], None, None);
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].y_size, 0xFFFF, "pixel size stored verbatim");
        assert_eq!(v6.windows[1].x_size, 0xFFFF);
        assert!(v6.windows[1].grid.rows <= 1024, "grid rows capped: {}", v6.windows[1].grid.rows);
        assert!(v6.windows[1].grid.cols <= 1024, "grid cols capped: {}", v6.windows[1].grid.cols);
    }

    #[test]
    fn v6_window_size_rehomes_a_cursor_left_outside() {
        // ZMSD §8.8.3.4: "If the window size is reduced so that its cursor lies
        // outside it, the cursor should be reset to the left margin on the top
        // line." (Shogun shrinks its menu window to a 1-px caret after printing.)
        let mut m = v6_exec_machine();
        m.exec_ext(0x11, &[1, 160, 320], None, None); // window_size(1, 160x320)
        m.exec_ext(0x08, &[24, 0, 1], None, None); // set_margins(left=24, right=0)
        m.exec_var(0x0F, &[100, 200, 1], None, None); // set_cursor deep inside
        m.exec_ext(0x11, &[1, 16, 320], None, None); // shrink height: cursor now outside
        {
            let w = &m.screen.v6.as_ref().unwrap().windows[1];
            assert_eq!((w.y_cursor, w.x_cursor), (1, 25), "re-homed to the left margin, top line");
        }
        // A resize that still contains the cursor leaves it alone.
        m.exec_var(0x0F, &[9, 40, 1], None, None);
        m.exec_ext(0x11, &[1, 160, 320], None, None);
        let w = &m.screen.v6.as_ref().unwrap().windows[1];
        assert_eq!((w.y_cursor, w.x_cursor), (9, 40), "cursor still inside → untouched");
    }

    #[test]
    fn non_v6_picture_data_is_a_noop_stub() {
        // v1–5 byte-identical: picture_data on a non-v6 machine must not write
        // to the array or branch (the Phase 0 stub behaviour).
        let mut m = Machine::new(Memory::new(crate::header::tests_support::sample_story(5)).unwrap());
        assert!(m.screen.v6.is_none());
        // array at 0x0300 (globals region, writable); pre-fill sentinels.
        m.mem.write_word(0x0300, 0xDEAD);
        m.mem.write_word(0x0302, 0xBEEF);
        m.exec_ext(0x06, &[5, 0x0300], None, None); // picture_data(5, array)
        assert_eq!(m.mem.read_word(0x0300), 0xDEAD, "array untouched for non-v6");
        assert_eq!(m.mem.read_word(0x0302), 0xBEEF);
    }

    #[test]
    fn v6_window_style_operations() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x12, &[1, 0b0101, 0], None, None); // replace
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[1].attributes, 0b0101);
        m.exec_ext(0x12, &[1, 0b0010, 1], None, None); // set bits
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[1].attributes, 0b0111);
        m.exec_ext(0x12, &[1, 0b0100, 2], None, None); // clear bits
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[1].attributes, 0b0011);
        m.exec_ext(0x12, &[1, 0b1001, 3], None, None); // toggle bits
        assert_eq!(m.screen.v6.as_ref().unwrap().windows[1].attributes, 0b1010);
    }

    // ── Lane Z: scroll_window (EXT:0x14) ──────────────────────────────────────

    #[test]
    fn v6_scroll_window_shifts_grid_window_text_and_grid() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x11, &[1, 24, 80], None, None); // window_size(1, y=24, x=80) -> 3x10 grid
        m.exec_var(0x0B, &[1], None, None); // set_window(1)
        m.exec_var(0x0F, &[1, 1], None, None); // set_cursor: pixel y=1,x=1 -> grid row 1
        m.print_text("A");
        m.exec_var(0x0F, &[9, 1], None, None); // set_cursor: pixel y=9,x=1 -> grid row 2 (one 8px line down)
        m.print_text("B");
        // scroll_window(1, 8) — one line forward/up.
        m.exec_ext(0x14, &[1, 8], None, None);
        let v6 = m.screen.v6.as_ref().unwrap();
        // "A" started at pixel y=1: 1-8=-7, bottom=-7+8-1=0 <1 -> dropped.
        // "B" started at pixel y=9: 9-8=1, bottom=1+8-1=8 >=1 -> kept at y=1.
        assert_eq!(v6.windows[1].texts.len(), 1, "run scrolled fully off top is dropped");
        assert_eq!(v6.windows[1].texts[0].text, "B");
        assert_eq!(v6.windows[1].texts[0].y, 1);
        // Grid also shifted up by one row (8px / 8px-per-row = 1 row).
        assert_eq!(v6.windows[1].grid.cell(1, 1).ch, 'B', "grid row 2 moved to row 1");
    }

    #[test]
    fn v6_scroll_window_on_window_zero_no_ops_silently() {
        // Window 0's scrolling belongs to the host transcript, so the opcode is
        // a no-op — and must stay SILENT. It is not an error path: it is the
        // back half of Zork Zero's inline-picture idiom (read cursor → scroll up
        // to free room → home into the freed band → draw the room icon → set
        // margins so prose flows beside it), which runs on essentially every
        // illustrated room description. The warning it used to push surfaced as
        // a player-facing Warning line mid-game (user report, 2026-07-28); the
        // `@scroll_window` trace line is the diagnostic channel instead.
        let mut m = v6_exec_machine();
        m.exec_ext(0x14, &[0, 8], None, None);
        m.exec_ext(0x14, &[0, 8], None, None);
        assert_eq!(
            m.diagnostics.iter().filter(|d| d.contains("scroll_window")).count(),
            0,
            "a window-0 scroll must not surface a player-facing diagnostic"
        );
    }

    #[test]
    fn v6_scroll_window_traces_when_enabled() {
        let mut m = v6_exec_machine();
        m.trace_screen = true;
        m.exec_ext(0x14, &[1, -8i16 as u16], None, None);
        assert!(m.screen_trace.iter().any(|l| l.contains("@scroll_window")));
    }

    // ── Lane Z: buffer_screen (EXT:0x1D) ──────────────────────────────────────

    #[test]
    fn v6_buffer_screen_tracks_mode_and_returns_previous() {
        let mut m = v6_exec_machine();
        // Default (unset) mode is 0 (update immediately, ZMSD §15).
        m.exec_ext(0x1D, &[1], Some(0x01), None); // switch to buffered (1), store prev
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 0, "prev mode was 0");
        m.exec_ext(0x1D, &[1], Some(0x01), None); // still 1 -> 1, store prev (now 1)
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 1, "prev mode was 1");
    }

    #[test]
    fn v6_buffer_screen_negative_one_forces_update_without_altering_state() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x1D, &[1], None, None); // set mode to buffered (1)
        m.exec_ext(0x1D, &[0xFFFFu16], Some(0x01), None); // -1: force update, don't change state
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 1, "returns current mode (1) unchanged");
        // A subsequent buffer_screen(1) must still report prev=1 (state untouched by -1).
        m.exec_ext(0x1D, &[1], Some(0x01), None);
        assert_eq!(crate::cpu::state::read_var(&mut m.state, &m.mem, 0x01), 1);
    }

    // ── Lane Z: picture_table (EXT:0x1C) ──────────────────────────────────────

    #[test]
    fn v6_picture_table_is_a_traced_formal_noop() {
        let mut m = v6_exec_machine();
        m.trace_screen = true;
        let result = m.exec_ext(0x1C, &[0x0300], None, None);
        assert!(matches!(result, StepResult::Continue));
        assert!(m.screen_trace.iter().any(|l| l.contains("@picture_table")));
        // Must not be routed through the unimplemented-opcode diagnostic path.
        assert!(m.diagnostics.iter().all(|d| !d.contains("0x1C")));
    }

    // ── Task 1 (Phase 1b): v6 print_text routes to the current window's grid ──

    #[test]
    fn v6_print_text_routes_to_current_window_grid() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x11, &[1, 40, 80], None, None); // window_size(1, y=40, x=80) -> 5x10 grid
        m.exec_var(0x0B, &[1], None, None); // set_window(1)
        m.exec_var(0x0F, &[1, 1], None, None); // set_cursor(row=1, col=1)
        m.print_text("HI");
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].grid.cell(1, 1).ch, 'H', "first char lands at (1,1)");
        assert_eq!(v6.windows[1].grid.cell(1, 2).ch, 'I', "second char lands at (1,2)");
        // Cursor is in PIXELS: two 8px glyphs from px 1 → px 17.
        assert_eq!(v6.windows[1].x_cursor, 17, "pixel cursor advanced past the printed text");
        // The run is also recorded at its exact pixel start for the raster.
        assert_eq!(v6.windows[1].texts.len(), 1);
        let t = &v6.windows[1].texts[0];
        assert_eq!((t.y, t.x, t.text.as_str()), (1, 1, "HI"));
    }

    #[test]
    fn v6_print_text_window_zero_streams_to_output() {
        let mut m = v6_exec_machine();
        // v6.current defaults to 0 (main/buffered window) — text must reach the
        // output sink, not a grid.
        assert_eq!(m.screen.v6.as_ref().unwrap().current, 0);
        m.print_text("hello");
        assert_eq!(buf_output(&m), "hello");
        // No grid window received the text.
        for w in &m.screen.v6.as_ref().unwrap().windows {
            assert_eq!(w.grid.cell(1, 1).ch, ' ', "window 0 text must not land in any grid");
        }
    }

    // ── Task 3: v6 split/set/erase_window over the window table ─────────────

    #[test]
    fn v6_set_window_selects_current_window() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[7], None, None); // set_window(7)
        let v6 = m.screen.v6.as_ref().expect("v6 story has a window table");
        assert_eq!(v6.current, 7, "set_window(7) selects window 7");
    }

    #[test]
    fn v6_set_window_preserves_each_windows_own_cursor() {
        // ZMSD §8.8.3.5: "Each window remembers its own cursor position
        // (relative to its own coordinates ...). These can be changed using
        // set_cursor (and it is legal to move the cursor for an unselected
        // window)." Selecting a window must therefore NOT home its cursor.
        // (This test replaces an assertion that pinned the opposite.)
        let mut m = v6_exec_machine();
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.windows[7].y_cursor = 33;
            v6.windows[7].x_cursor = 57;
        }
        m.exec_var(0x0B, &[7], None, None); // set_window(7)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.current, 7);
        assert_eq!(v6.windows[7].y_cursor, 33, "window 7 remembers its own cursor row");
        assert_eq!(v6.windows[7].x_cursor, 57, "window 7 remembers its own cursor col");
    }

    #[test]
    fn v6_set_window_adopts_target_windows_colour_pair() {
        // ZMSD §8.3: in Version 6 each window carries its own colour pair, and
        // prose printed to the current window must be tagged with it.
        let mut m = v6_exec_machine();
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.windows[4].fg = ZColour::Standard(3);
            v6.windows[4].bg = ZColour::Standard(6);
        }
        m.exec_var(0x0B, &[4], None, None); // set_window(4)
        assert_eq!(m.screen.current_fg, ZColour::Standard(3), "prose fg follows the window");
        assert_eq!(m.screen.current_bg, ZColour::Standard(6), "prose bg follows the window");
    }

    #[test]
    fn v6_set_colour_on_current_window_reaches_the_prose_stream() {
        // ZMSD §8.3: v6 set_colour targets a window; the prose stream tags its
        // TextAttrs from current_fg/current_bg, so a colour set on the CURRENT
        // window has to land there too (it used to be dropped, leaving all v6
        // prose in the interpreter default).
        let mut m = v6_exec_machine();
        m.exec_2op(0x1B, &[4, 6], None, None); // set_colour(green, blue) — current window
        assert_eq!(m.screen.current_fg, ZColour::Standard(4));
        assert_eq!(m.screen.current_bg, ZColour::Standard(6));
        // A colour aimed at some OTHER window must not disturb the prose pair.
        m.exec_2op(0x1B, &[3, 9, 5], None, None); // set_colour(red, white, window 5)
        assert_eq!(m.screen.current_fg, ZColour::Standard(4), "window 5's colour is not window 0's");
        assert_eq!(m.screen.current_bg, ZColour::Standard(6));
    }

    #[test]
    fn v6_set_true_colour_on_current_window_reaches_the_prose_stream() {
        let mut m = v6_exec_machine();
        m.exec_ext(0x0D, &[0x1234, 0x5678], None, None);
        assert_eq!(m.screen.current_fg, ZColour::True(0x1234));
        assert_eq!(m.screen.current_bg, ZColour::True(0x5678));
    }

    #[test]
    fn v6_erase_window_clears_target_window_grid() {
        let mut m = v6_exec_machine();
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.windows[3].grid.resize(2, 2);
            v6.windows[3].grid.put(1, 1, 'X', 0, ZColour::Default, ZColour::Default);
            v6.windows[3].y_cursor = 5;
            v6.windows[3].x_cursor = 5;
        }
        m.exec_var(0x0D, &[3], None, None); // erase_window(3)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[3].grid.cell(1, 1).ch, ' ', "window 3's grid cleared");
        assert_eq!(v6.windows[3].y_cursor, 1, "cursor reset to top-left row");
        assert_eq!(v6.windows[3].x_cursor, 1, "cursor reset to top-left col");
    }

    /// A machine on a bare story of the requested version, with a 10-column
    /// screen in the header so the upper-window grid has a width.
    fn screen_machine(version: u8) -> Machine {
        let mut m = Machine::new(
            Memory::new(crate::header::tests_support::sample_story(version)).unwrap(),
        );
        m.mem.write_byte(0x21, 10); // screen width = 10 cols
        m
    }

    #[test]
    fn v6_window0_boots_occupying_the_whole_screen() {
        // ZMSD §8.8.3.3: "Window 0 occupies the whole screen and is initially
        // selected. Window 1 is as wide as the screen but has zero height."
        // Window 0's height used to boot at 0.
        let m = v6_exec_machine();
        let v6 = m.screen.v6.as_ref().unwrap();
        let full_w = crate::screen::DEFAULT_SCREEN_COLS as u16 * V6_FONT_WIDTH;
        let full_h = crate::screen::DEFAULT_SCREEN_ROWS as u16 * V6_FONT_HEIGHT;
        assert_eq!(v6.windows[0].x_size, full_w, "window 0 is as wide as the screen");
        assert_eq!(v6.windows[0].y_size, full_h, "window 0 is as tall as the screen");
        assert_eq!(v6.windows[1].x_size, full_w, "window 1 is as wide as the screen");
        assert_eq!(v6.windows[1].y_size, 0, "window 1 has zero height");
        assert_eq!(v6.current, 0, "window 0 is initially selected");
    }

    #[test]
    fn v6_set_screen_dims_gives_window0_the_whole_screen() {
        // Same §8.8.3.3 clause, re-applied when the host reports the real size.
        let mut m = v6_exec_machine();
        m.set_screen_dims(25, 80);
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[0].x_size, 80 * V6_FONT_WIDTH);
        assert_eq!(v6.windows[0].y_size, 25 * V6_FONT_HEIGHT);
        assert_eq!(v6.windows[1].y_size, 0, "window 1 stays unsplit");
    }

    /// SQ-0533: the v4/v5 "live upper window follows the screen width" refit is
    /// v6-exempt — v6 lays out in its own pixel screen and sizes its window grids
    /// through `window_size`, so a `set_screen_dims` must leave every v6 window
    /// grid (and the unused classic `screen.upper`) exactly as it was.
    #[test]
    fn set_screen_dims_leaves_v6_window_grids_alone() {
        let mut m = v6_exec_machine();
        m.set_screen_dims(25, 80);
        m.exec_ext(0x11, &[1, 160, 320], None, None); // window_size(1) -> 10x40 grid
        let before = {
            let w = &m.screen.v6.as_ref().unwrap().windows[1];
            (w.grid.rows, w.grid.cols)
        };
        assert_eq!(before, (10, 40));

        m.set_screen_dims(25, 100);

        let w = &m.screen.v6.as_ref().unwrap().windows[1];
        assert_eq!((w.grid.rows, w.grid.cols), before, "v6 window grid untouched by a resize");
        assert_eq!(m.screen.upper.cols, 0, "the classic upper grid is unused in v6");
        assert_eq!(m.screen.upper.rows, 0);
    }

    #[test]
    fn v6_erase_window_minus_one_unsplits_and_selects_window_zero() {
        // ZMSD §8.8.5.3.1: "Erasing window number -1 erases the entire screen to
        // the background colour of window 0, unsplits windows 0 and 1 and
        // selects window 0."
        let mut m = v6_exec_machine();
        m.set_screen_dims(25, 80);
        m.exec_var(0x0A, &[160], None, None); // split_window(160px) -> window 1 has height
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.current = 3;
            v6.windows[0].bg = ZColour::Standard(6);
            v6.windows[2].y_cursor = 40;
        }
        m.exec_var(0x0D, &[0xFFFF], None, None); // erase_window(-1)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.current, 0, "-1 selects window 0");
        assert_eq!(v6.windows[1].y_size, 0, "-1 unsplits: window 1 collapses to zero height");
        assert_eq!(v6.windows[0].y_size, 25 * V6_FONT_HEIGHT, "-1 gives window 0 the screen");
        assert_eq!(v6.windows[2].y_cursor, 1, "-1 is a full per-window erase: cursors home");
        assert_eq!(
            m.screen.current_bg,
            ZColour::Standard(6),
            "the screen is erased to window 0's background, which becomes current"
        );
    }

    #[test]
    fn v6_erase_window_minus_two_changes_no_attributes_or_cursors() {
        // ZMSD §8.8.5.3.2: "Erasing window -2 erases the entire screen to the
        // current background colour. (It doesn't perform erase_window for all
        // the individual windows, and it doesn't change any window attributes or
        // cursor positions.)" -1 and -2 used to be handled identically.
        let mut m = v6_exec_machine();
        m.set_screen_dims(25, 80);
        m.exec_var(0x0A, &[160], None, None); // split_window(160px)
        {
            let v6 = m.screen.v6.as_mut().unwrap();
            v6.current = 3;
            v6.windows[2].grid.resize(1, 2);
            v6.windows[2].grid.put(1, 1, 'X', 0, ZColour::Default, ZColour::Default);
            v6.windows[2].y_cursor = 40;
            v6.windows[2].x_cursor = 24;
        }
        m.exec_var(0x0D, &[0xFFFE], None, None); // erase_window(-2)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.current, 3, "-2 does not change the selected window");
        assert_eq!(v6.windows[1].y_size, 160, "-2 does not unsplit");
        assert_eq!(v6.windows[2].y_cursor, 40, "-2 leaves cursor positions alone");
        assert_eq!(v6.windows[2].x_cursor, 24, "-2 leaves cursor positions alone");
        assert_eq!(v6.windows[2].grid.cell(1, 1).ch, ' ', "-2 still erases the whole screen");
    }

    #[test]
    fn v6_set_window_leaves_v5_classic_path_untouched() {
        // Regression: a v5 (non-v6) machine must still route set_window through
        // the classic ScreenState.current_window field, not the window table.
        let mut m = build_test_machine(&[]);
        assert!(m.screen.v6.is_none(), "v5 keeps the classic 2-window model");
        m.exec_var(0x0B, &[1], None, None); // set_window(1)
        assert_eq!(m.screen.current_window, 1, "v5 set_window still sets current_window");
    }

    // ── Task 5: v6 per-window set_cursor / set_colour / set_true_colour ─────

    #[test]
    fn v6_set_cursor_targets_current_window_by_default() {
        // v6 set_cursor coords are 1-based PIXELS (ZMSD §8.8.1: (1,1) is top-
        // left) and are stored verbatim — window props 4/5 read back in units.
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[2], None, None); // set_window(2)
        m.exec_var(0x0F, &[16, 24], None, None); // set_cursor(y=16px, x=24px)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[2].y_cursor, 16);
        assert_eq!(v6.windows[2].x_cursor, 24);
    }

    #[test]
    fn v6_set_cursor_third_operand_selects_window() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[2], None, None); // current = 2
        m.exec_var(0x0F, &[33, 41, 4], None, None); // set_cursor(y=33px, x=41px, window=4)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[4].y_cursor, 33, "pixel row stored verbatim");
        assert_eq!(v6.windows[4].x_cursor, 41, "pixel col stored verbatim");
        assert_eq!(v6.windows[2].y_cursor, 1, "current window (2) untouched");
    }

    #[test]
    fn v6_set_cursor_clamps_zero_and_negative_to_left_margin() {
        // ZMSD §15: a cursor position outside the margins moves to the left
        // margin — zero/negative operands clamp to (1,1) rather than wrapping
        // (a game computing garbage coords must not land at the right edge).
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[1], None, None); // current = 1
        m.screen.v6.as_mut().unwrap().windows[1].x_size = 320; // window width, px
        m.exec_var(0x0F, &[9, 17], None, None); // set_cursor(y=9px, x=17px)
        {
            let w = &m.screen.v6.as_ref().unwrap().windows[1];
            assert_eq!(w.y_cursor, 9, "pixel row stored verbatim");
            assert_eq!(w.x_cursor, 17, "pixel col stored verbatim");
        }
        m.exec_var(0x0F, &[9, (-16i16) as u16], None, None); // negative x → clamp
        let w = &m.screen.v6.as_ref().unwrap().windows[1];
        assert_eq!(w.x_cursor, 1, "negative x clamps to pixel 1, not the right edge");
    }

    #[test]
    fn v6_set_cursor_negative_row_does_not_corrupt_position() {
        // ZMSD v6: row -1 turns the cursor off, -2 turns it back on — these
        // are not literal positions and must not be written through.
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[1], None, None); // current = 1
        m.exec_var(0x0F, &[17, 25], None, None); // baseline (17px, 25px)
        m.exec_var(0x0F, &[(-1i16) as u16, 0], None, None); // cursor off
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].y_cursor, 17, "negative row leaves prior position");
        assert_eq!(v6.windows[1].x_cursor, 25, "negative row leaves prior position");
    }

    #[test]
    fn v6_set_cursor_leaves_v5_classic_path_untouched() {
        let mut m = build_test_machine(&[]);
        assert!(m.screen.v6.is_none());
        m.exec_var(0x0A, &[4], None, None); // split_window 4
        m.exec_var(0x0B, &[1], None, None); // set_window 1 — §8.7.2.3 precondition
        m.exec_var(0x0F, &[3, 4], None, None); // set_cursor(3, 4)
        assert_eq!(m.screen.cursor_row, 3);
        assert_eq!(m.screen.cursor_col, 4);
    }

    #[test]
    fn v6_set_colour_no_window_operand_applies_to_current_window() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[5], None, None); // current = 5
        m.exec_2op(0x1B, &[3, 6], None, None); // set_colour(fg=3, bg=6)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[5].fg, ZColour::Standard(3));
        assert_eq!(v6.windows[5].bg, ZColour::Standard(6));
    }

    #[test]
    fn v6_set_colour_still_accepts_the_v6_only_greys() {
        // The other half of §8.3.1's "Colours 10, 11, 12, 15 and -1 are
        // available only in Version 6": in v6 they ARE the palette.
        let mut m = v6_exec_machine();
        m.exec_2op(0x1B, &[10, 12], None, None); // set_colour(light grey, dark grey)
        let v6 = m.screen.v6.as_ref().unwrap();
        let cur = v6.current as usize;
        assert_eq!(v6.windows[cur].fg, ZColour::Standard(10));
        assert_eq!(v6.windows[cur].bg, ZColour::Standard(12));
    }

    #[test]
    fn v6_set_colour_window_operand_targets_named_window() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[5], None, None); // current = 5
        m.exec_2op(0x1B, &[3, 6, 2], None, None); // set_colour(3, 6, window=2)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[2].fg, ZColour::Standard(3));
        assert_eq!(v6.windows[2].bg, ZColour::Standard(6));
        assert_eq!(v6.windows[2].colour_data, 0x0603, "bg (6) high byte, fg (3) low byte");
        assert_eq!(v6.windows[5].fg, ZColour::Default, "current window (5) untouched");
    }

    #[test]
    fn v6_set_colour_3operand_dispatches_via_2op_variable_form() {
        // Verify the dispatch, not just assume it: ZMSD §15 says the v6
        // 3-operand set_colour "uses the same opcode number (2OP:27 = 0x1B)
        // but in variable-length encoding" — i.e. a 2OP opcode encoded with
        // top bits 11 (variable form) and bit5=0 (2OP, not VAR). decode.rs's
        // Form::Variable handling reads such an instruction's operands from a
        // type byte exactly like VAR, so it can carry >2 operands, and
        // classifies it OperandCount::Two — meaning it dispatches to
        // exec_2op, never exec_var. This test encodes exactly that byte
        // pattern and drives it through `Machine::step` end-to-end.
        let mut buf = sample_story(6);
        let mut pos = 0x10usize;
        buf[pos] = 0xDB; pos += 1; // variable form (11), 2OP (bit5=0), opcode 0x1B
        buf[pos] = 0b01_01_01_11; pos += 1; // types: small, small, small, omitted
        buf[pos] = 5; pos += 1; // fg = std5
        buf[pos] = 2; pos += 1; // bg = std2
        buf[pos] = 3; pos += 1; // window = 3
        buf[pos] = 0xBA; // quit

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m.step();

        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[3].fg, ZColour::Standard(5));
        assert_eq!(v6.windows[3].bg, ZColour::Standard(2));
    }

    #[test]
    fn v6_set_colour_leaves_v5_classic_path_untouched() {
        let mut m = build_test_machine(&[]);
        assert!(m.screen.v6.is_none());
        m.exec_2op(0x1B, &[3, 6], None, None); // set_colour(3, 6)
        assert_eq!(m.screen.current_fg, ZColour::Standard(3));
        assert_eq!(m.screen.current_bg, ZColour::Standard(6));
    }

    #[test]
    fn v6_set_true_colour_window_operand_targets_named_window() {
        let mut m = v6_exec_machine();
        m.exec_var(0x0B, &[5], None, None); // current = 5
        m.exec_ext(0x0D, &[0x7FFF, (-1i16) as u16, 2], None, None); // (fg, bg, window=2)
        let v6 = m.screen.v6.as_ref().unwrap();
        assert_eq!(v6.windows[2].fg, ZColour::True(0x7FFF));
        assert_eq!(v6.windows[2].bg, ZColour::Default);
        assert_eq!(v6.windows[5].fg, ZColour::Default, "current window (5) untouched");
    }

    #[test]
    fn v6_set_true_colour_leaves_v5_classic_path_untouched() {
        let mut m = build_test_machine(&[]);
        assert!(m.screen.v6.is_none());
        m.exec_ext(0x0D, &[0x7FFF, (-1i16) as u16], None, None);
        assert_eq!(m.screen.current_fg, ZColour::True(0x7FFF));
        assert_eq!(m.screen.current_bg, ZColour::Default);
    }
}
