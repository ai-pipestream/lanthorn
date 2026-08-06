//! Startup / boot sequence: parse args, load config, resolve and load the story,
//! build the engine, load the mapper/archive, seed the initial UI state, and set
//! up the terminal. Extracted verbatim from `main.rs` (SQ-0306) as `main()`'s
//! linear setup phase (originally "steps 1-4"). Split for SQ-0435 into
//! [`resolve_launch`] (the one-time arg/config resolution, run once by `main`)
//! and [`boot_story`] (the per-story build, run for each chosen story), so a
//! directory launch can replay the build across the picker→play loop. `main`
//! calls `resolve_launch`, then per story `boot_story` and the event loop over
//! the returned [`BootResult`]; helper fns they rely on stay in `main.rs`
//! (referenced via `crate::`) because they are shared with the loop or exercised
//! by `main.rs` tests.

use std::io::{stdout, Stdout};

use crossterm::event::{EnableBracketedPaste, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
use mapper::mapper::Mapper;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use clap::Parser;

use app::archive::load_archive;
use app::config::{config_path, resolve, Cli, Config};
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::hints;
use app::ifid::compute_ifid;
use app::session::{apply_turn, GameSession, TurnResult};
use app::state::AppState;
use app::storage::{default_state_path, game_dir as story_game_dir, story_key};

use crate::engine_helpers::{restore_error_msg, zvm_session_opt_mut};
use crate::{
    install_panic_hook, loading_line, picker_ui, resolve_pict_blorb, restore_terminal, saves_dir,
};

/// Everything [`boot`] produces that `main()`'s event loop then owns: the boxed
/// engine, the mapper, the UI state, the terminal handle, and the per-story
/// paths/identity the loop threads into save/restore/reset calls.
pub(crate) struct BootResult {
    pub session: Box<dyn Engine>,
    pub mapper: Mapper,
    pub state: AppState,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub game_dir: std::path::PathBuf,
    pub ifid: String,
    pub arc_file: std::path::PathBuf,
    pub story_bytes: Vec<u8>,
    pub story_path: std::path::PathBuf,
    pub data_base: std::path::PathBuf,
}

/// The one-time launch context resolved before the picker→play loop: parsed
/// args, config, the saves/sidecar base dir, and whether babelmap was launched
/// against a directory (a story library) or a single file. `resolve_launch`
/// builds this ONCE; `boot_story` consumes it (by reference) per story so a
/// library launch can replay the build for each chosen story. (SQ-0435)
pub(crate) struct LaunchCtx {
    pub cli: Cli,
    pub cfg: Config,
    pub data_base: std::path::PathBuf,
    /// The story directory when launched from a library (the picker source),
    /// else `None`.
    pub library_dir: Option<std::path::PathBuf>,
    /// The single story file when launched with a file argument, else `None`.
    pub single_file: Option<std::path::PathBuf>,
}

/// Resolve the one-time launch context: parse args + config, seed the style
/// template, apply the `default_story_dir` fallback (plus the first-use prompt,
/// which runs exactly ONCE), compute the data base, and classify the launch as a
/// story library (a directory) or a single file. May `std::process::exit(2)`
/// when there's nothing to open. Signal-handler registration lives in `main`
/// (before the loop); the per-story build lives in [`boot_story`]. (SQ-0435)
pub(crate) fn resolve_launch() -> LaunchCtx {
    // ── 1. Parse args + load config ───────────────────────────────────────────

    let cli = Cli::parse();
    let mut cfg = resolve(&cli);

    // Auto-seed a fresh style.toml (SQ-0309, Task 6b) on every startup — before the
    // story picker — so browsing (even without launching a story) leaves the fully
    // commented, registry-derived template, and the picker reads the same file the
    // game does. Never overwrites an existing file; best-effort (a read-only home
    // must not crash startup).
    app::theme::template::auto_seed(&cfg.user_dir);

    // …and the same treatment for config.toml (SQ-0573): a fully commented template
    // listing EVERY setting at its default, so what babelmap can be told to do is
    // discoverable from the file rather than only from the source. Same contract as
    // the style seed — never overwrites, best-effort. Seeded at the RESOLVED config
    // path (`--config`/`--user-dir`/default), not `user_dir`, so the file we seed is
    // the file we read (SQ-0574). Runtime edits still go through `write_config_file`,
    // which is format-preserving and keeps these comments.
    app::config_template::auto_seed(&cfg.config_file);

    // A path may be omitted; fall back to the configured default story dir.
    // With neither, there's nothing to open — tell the user how to fix it.
    let story_path = match cli.story.clone().or_else(|| cfg.default_story_dir.clone()) {
        Some(p) => p,
        None => {
            eprintln!(
                "babelmap: no story given. Pass a story file or directory, or set \
                 `default_story_dir` in {}.",
                config_path(&cli).display(),
            );
            std::process::exit(2);
        }
    };

    // First time a directory is passed on the command line with no default set,
    // offer to remember it as the default story directory (persisted to config).
    if cfg.default_story_dir.is_none()
        && cli.story.as_deref().map(|p| p.is_dir()).unwrap_or(false)
        && prompt_yes_no(&format!(
            "Set {} as your default story directory?",
            story_path.display()
        ))
    {
        // Store an absolute path so a later bare `babelmap` resolves the same
        // directory regardless of the working dir it's launched from. The dir
        // exists (is_dir passed), so canonicalize should succeed; fall back to
        // the supplied path if it somehow doesn't.
        let to_store = std::fs::canonicalize(&story_path).unwrap_or_else(|_| story_path.clone());
        cfg.default_story_dir = Some(to_store.clone());
        match app::config::write_config_file(&cfg) {
            Ok(()) => eprintln!("babelmap: saved default story directory ({}).", to_store.display()),
            Err(e) => eprintln!("babelmap: could not save config: {e}"),
        }
    }

    // Storage base for saves/sidecars (SQ-0284): `--data-dir` overrides the
    // default `<user_dir>/saves`. Each story gets `<data_base>/<story-key>/`.
    let data_base = cli.data_dir.clone().unwrap_or_else(|| saves_dir(&cfg.user_dir));

    // A directory launches the pre-game picker (a library); a file plays directly.
    let (library_dir, single_file) = if story_path.is_dir() {
        (Some(story_path), None)
    } else {
        (None, Some(story_path))
    };

    LaunchCtx { cli, cfg, data_base, library_dir, single_file }
}

/// The real story-pane `(rows, cols)` a v1–8 Z-machine session should be BOOTED
/// with — measured BEFORE the engine exists, so a v4/v5 story's boot-time
/// status-bar layout (Zork 1: paints its reverse bar once, at whatever width
/// header byte $21 held at that moment, then only re-cursors to the two field
/// columns it derived from it) already targets the real pane instead of the
/// zvm 80×24 fallback `init_caps` seeds absent a hint (SQ-0679/SQ-0680).
///
/// Runs the SAME split this frame's [`compute_pane_layout`]/[`story_screen_dims`]
/// would, against a throwaway [`AppState`] carrying only what those two
/// functions read: the resolved theme (border sides, for the upper-window
/// frame `story_screen_dims` insets), the resolved config (margins, the
/// `virtual_screen_cols`/`rows` pin — pinned wins here exactly as it wins in
/// the live pane measurement, since this reuses the very same call), the
/// garglk.ini margin overlay, and the pane-split sizes. Command band / inventory
/// dock are left at their true boot-time state — closed; both open only after
/// this session already exists (`band_auto_open`, further down) — and neither
/// affects the WIDTH this seeds anyway, only rows, which the SQ-0679 floor
/// never gates.
///
/// `None` when the terminal size can't be queried (piped/non-terminal stdout,
/// e.g. some test harnesses) or the query reports a zero-area frame; the
/// constructor then falls back to the 80×24 boot default exactly as before this
/// change.
fn pre_boot_host_screen(
    cfg: &Config,
    cs: &app::colors::ColorScheme,
    garglk_overlay: &Option<app::garglk_ini::GarglkOverlay>,
) -> Option<(u16, u16)> {
    let (term_cols, term_rows) = crossterm::terminal::size().ok()?;
    let frame = Rect::new(0, 0, term_cols, term_rows);
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let mut boot_state = AppState::default();
    boot_state.colors = cs.clone();
    boot_state.config = cfg.clone();
    boot_state.garglk_overlay = garglk_overlay.clone();
    boot_state.pane_sizes = app::state::PaneSizes {
        split_ratio: cfg.split_ratio,
        band_height: cfg.command_band.height,
        inv_dock_pct: cfg.inv_dock_pct,
    };
    let pane_layout = app::layout::compute_pane_layout(frame, &boot_state, 0);
    app::render::screen::story_screen_dims(pane_layout.story, &boot_state)
}

/// Build the per-story engine + mapper + UI state + terminal for `story_path`,
/// using the one-time [`LaunchCtx`]. This is the per-story half of the old
/// `boot()` (load the story, build the engine, load the mapper/archive, seed the
/// state, and enter the alternate screen); a library launch calls it once per
/// chosen story. May `std::process::exit` on an unrecoverable per-story error
/// (unreadable/invalid story, terminal init failure) exactly as before.
///
/// `cfg` is cloned from `ctx` per story because the per-game overlays (garglk.ini
/// colours, per-game honor/borderless) mutate it — each story must start from the
/// pristine launch config. (SQ-0435)
pub(crate) fn boot_story(ctx: &LaunchCtx, story_path: std::path::PathBuf) -> BootResult {
    let cli = &ctx.cli;
    let mut cfg = ctx.cfg.clone();
    let data_base = ctx.data_base.clone();

    let loaded = match hints::load_story(&story_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("babelmap: cannot read '{}': {}", story_path.display(), e);
            std::process::exit(1);
        }
    };
    // Raw executable bytes (for the IFID / map-dir key), independent of engine.
    let story_bytes = loaded.bytes().to_vec();

    // Booting a large story to its first prompt can take several seconds, and this
    // happens before the alternate screen is entered — so the normal terminal would
    // otherwise sit frozen. Spin a tiny indicator on a side thread; it only starts
    // drawing after a short grace period, so quick loads never flicker.
    let loading_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let loading_spinner = {
        use std::io::Write as _;
        use std::sync::atomic::Ordering;
        let done = loading_done.clone();
        let name = story_path.display().to_string();
        let bytes = story_bytes.len();
        std::thread::spawn(move || {
            const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
            const TICK_MS: u64 = 60;
            let (mut waited, mut i, mut shown) = (0u64, 0usize, false);
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
                waited += TICK_MS;
                if waited >= 180 {
                    eprint!("\r{}", loading_line(&name, bytes, FRAMES[i % FRAMES.len()]));
                    let _ = std::io::stderr().flush();
                    i += 1;
                    shown = true;
                }
            }
            if shown {
                eprint!("\r\x1b[2K"); // erase the spinner line before the UI starts
                let _ = std::io::stderr().flush();
            }
        })
    };

    // In-game graphics Picker (None when --no-images or unavailable). Built once
    // and reused both for the Glulx session's char-cell pixel size and, below,
    // AppState.game_picker (the render side already tolerates None).
    let game_picker = if cfg.images { picker_ui::build_cover_picker(cfg.image_protocol) } else { None };
    // Probe the terminal's own default fg/bg (OSC 10/11) in the same pre-UI query
    // window as the image-protocol Picker above (SQ-0510). Seeds the v6 raster
    // canvas's default ink/page so "terminal default" theme colours follow the
    // real terminal instead of a hardcoded light-grey-on-black. Never hangs;
    // terminals that don't answer leave both as None and keep today's fallbacks.
    let term_default_colors = app::term_colors::query_terminal_default_colors();
    let char_px = game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width as u32, f.height as u32)
        })
        .unwrap_or((8, 16));
    // SQ-0593: divide out the terminal's scale before the game sees it. A Glk game's
    // graphics-window sizes are pixel constants its author picked against a
    // conventional screen; a cell twice the reference height turns the same request
    // into half the rows, shrinking the game's artwork against unchanged text. See
    // `GlkPixelScale::resolve` for why this keys off the cell size rather than the
    // display's DPI. No-op at `auto` on an unscaled display with a normal font.
    let char_px = cfg.glk_pixel_scale.apply(char_px);
    // Pixel-precise mouse reporting (SQ-0563) is NOT switched on here. The probe
    // works — terminals answer "set" — but the cell size to divide the reported
    // pixels by does not: the Picker's `font_size` above is in logical points,
    // while SGR-Pixels reports DEVICE pixels, so on a 2× display every click came
    // out at twice its true column and row. That broke click-drag selection and
    // made even cell-granular game buttons unhittable. Until the cell size is
    // derived from the same pixel space the mouse reports in, coordinates stay
    // cells and `pixel_mouse::normalise` is a no-op. Leaving the mode UNSET also
    // matters: a terminal left in PixelMode would report pixels that nothing
    // divides. See `pixel_mouse` for the plumbing, which is otherwise complete.

    // Storage (SQ-0284): saves/sidecars live in `<data_base>/<story-key>/`,
    // keyed by the story filename. Compute the per-game dir and read the Glk
    // file VFS sidecar BEFORE building the engine, so a Glulx boot that reads or
    // writes a Glk file (e.g. CM's init cache) sees the sidecar in place (SQ-0290).
    let game_dir = story_game_dir(&data_base, &story_key(&story_path));
    let _ = std::fs::create_dir_all(&game_dir);
    let vfs_sidecar = app::vfs_store::read_vfs(&game_dir);
    app::trace::hostio(&cfg.user_dir, cfg.trace.hostio,
        format!("vfs_read({} bytes)", vfs_sidecar.len()));

    // Resolve the look from style.toml (the single styling source) BEFORE the
    // engine builds: a Glulx game may probe glk_style_measure for the host's
    // rendered colours during boot (Kerkerkruip's dark-background detection,
    // SQ-0315), so the theme pairs must be in the backend first. `state.colors`
    // is assigned from these below.
    let (style_doc, style_w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (mut cs, set, style_w2) = app::style::resolve(&style_doc, &cfg.user_dir);
    // SQ-0319: discover a per-game garglk.ini beside the story and overlay its
    // colours onto the resolved theme BEFORE the backend snapshot below, so the
    // imported look is in the backend for glk_style_measure and painted from
    // turn one. The overlay is stashed in `state` further down so the post-IFID
    // reload_style (and any live /reload) re-applies it. `stylehint` gates
    // honor_game_colours, which the engine build below reads. Precedence: global
    // theme < garglk.ini < per-game <game_dir>/style.toml.
    // SQ-0318: the global config default is the honor base; garglk.ini's
    // `stylehint` gate and the user's per-game override layer on top (per-game
    // wins). Capture the base before garglk mutates `cfg` so `reload_style` can
    // recompute the precedence and `auto` can fall back to it.
    let honor_game_colours_base = cfg.honor_game_colours;
    let garglk_overlay = app::garglk_ini::discover(&story_path);
    let garglk_line = garglk_overlay.as_ref().map(|ov| {
        let summary = ov.apply(&mut cs);
        if let Some(h) = ov.honor_game_colours {
            cfg.honor_game_colours = h;
        }
        summary.console_line()
    });
    // SQ-0318: apply the user's persisted per-game honor override (if any) ON TOP
    // of garglk/global, so the engine builds — and turn one renders — with the
    // user's explicit choice in force. The IFID is computed here (from the raw
    // bytes) and reused for the map dir / identity below.
    let ifid = compute_ifid(&story_bytes);
    if let Some(v) = app::styles::read_per_game_honor(&game_dir) {
        cfg.honor_game_colours = v;
    }
    // SQ-0341: per-game borderless-windows override (default off → honor the Glk
    // border hint). Applies to Glulx layout from the first relayout at boot.
    // SQ-0344: precedence mirrors honor_game_colours — an explicit per-game
    // `config.toml` value wins, else a discovered garglk.ini's `wborderx`/
    // `wbordery` (0 → borderless), else off.
    let borderless = app::styles::read_per_game_borderless(&game_dir)
        .or_else(|| garglk_overlay.as_ref().and_then(|o| o.borderless))
        .unwrap_or(false);
    // SQ-0304: per-game map-panel visibility. `Some(false)` → start with the map
    // hidden (captured here before `cfg` is moved into the engine build below).
    let start_map_hidden = app::styles::read_per_game_show_map(&game_dir) == Some(false);
    let theme_colours = app::glk_backend::theme_style_colours(&cs);
    // ZMSD §8.3.3 (SQ-0532/A-F2): publish OUR default page + ink in header bytes
    // $2C/$2D, as the nearest §8.3.1 standard colour numbers, so a game that asks
    // "what does 'default' look like here?" gets an honest answer instead of a
    // fixed black-on-white. Resolved from the same layering the renderer uses
    // (theme when it supplies both channels concretely, else the OSC 10/11 probe)
    // and passed into the constructor so it is in force BEFORE the game boots.
    // `honor_game_colours = false` means the interpreter declares itself
    // colourless to the story, so the VM's §8.3.2 seed is left alone.
    let host_default_colours = if cfg.honor_game_colours {
        app::colors::host_default_colour_pair(
            cs.theme.get("transcript").style,
            term_default_colors.fg.map(|c| (c.0[0], c.0[1], c.0[2])),
            term_default_colors.bg.map(|c| (c.0[0], c.0[1], c.0[2])),
        )
    } else {
        None
    };
    // SQ-0679/SQ-0680: the real story-pane `(rows, cols)`, measured before the
    // engine exists, so a v4/v5 story's boot-time status-bar layout already
    // targets it instead of the zvm 80×24 fallback. `None` (size query failed,
    // or a zero-area frame) leaves the constructor's existing fallback in place.
    let host_screen = pre_boot_host_screen(&cfg, &cs, &garglk_overlay);

    // Build the engine: a Z-machine GameSession for Z-code, a GlulxSession for
    // Glulx — both boxed behind the neutral Engine trait. Z-machine-specific
    // setup (screen dims, undo cap) runs in its arm before boxing.
    let mut session: Box<dyn Engine> = match loaded {
        app::hints::LoadedStory::ZCode(bytes) => {
            // v6 Pict dimension table (Plan 1a, SQ-0186): resolve the story's
            // resource Blorb the same way sound resources are resolved below —
            // a self-blorb, a same-stem sidecar, or (Zork0's actual release
            // layout: `zork0-r393-s890714.z6` beside `Zork0.blb`, a resources-
            // only Blorb with no `Exec` of its own) a dir-scan stem-prefix match
            // — and header-sniff every Pict's size (no full decode). This MUST
            // run before `new_with_trace`: `picture_data` is called during boot,
            // which happens inside `new_with_trace` itself (Phase 0 lesson).
            let mut picts = app::graphics::PictSource::new(
                if cfg.images { blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b) } else { None }
            );
            let picture_dims = picts.all_pict_dims();
            // v6: the Blorb `Reso` standard window (e.g. Zork0 → 320×200) is the
            // game's native ART resolution. `new_with_trace` advertises 2× it —
            // the reference-authentic 640×400 unit screen (SQ-0479) — before boot
            // so windows + hardcoded art align. `None` (no Reso / non-v6) falls
            // back to 320×200 art → 640×400 screen inside.
            let v6_screen_px = picts.std_window();

            // `--debug` (SQ-0449): trace from the first boot instruction so the
            // game's initialisation code is captured (a later `/debug` can't).
            let mut s = match GameSession::new_with_trace(bytes, cfg.honor_game_colours, cfg.enable_sound, cfg.interpreter_number, cli.debug, picture_dims, v6_screen_px, host_default_colours, host_screen) {
                Ok(s) => s,
                Err(e) => {
                    use zvm::error::ZError;
                    let msg = match e {
                        ZError::UnsupportedVersion(v) => format!("unsupported Z-machine version {v}"),
                        ZError::NotAStoryFile => "file is not a valid Z-machine story file".to_string(),
                        ZError::Truncated => "story file is truncated".to_string(),
                        _ => format!("{e:?}"),
                    };
                    eprintln!("babelmap: {msg}");
                    std::process::exit(1);
                }
            };
            // v6 Pict source (Plan 1b Task 2, SQ-0186): retained on the session
            // (not `AppState`) so `drain_turn` can rasterize `draw_picture`/
            // `erase_picture` events into `pictures_canvas` self-contained —
            // Plan 1a's dimension table above is a separate, boot-time-only use.
            s.set_pict_source(Some(picts));
            // v6 boot-picture flush (Plan 1b Task 5): a v6 game draws its opening
            // art during boot, inside `new_with_trace` above, before the Pict
            // source existed to rasterize it — drain that backlog once now so the
            // very first `screen()` (before the player's first turn) already
            // shows the boot graphics instead of a blank window.
            s.flush_boot_pictures();
            // Pinned virtual screen dimensions, if the user set either key.
            // `pre_boot_host_screen` above already resolves this pin (it's what
            // `story_screen_dims` reads first) and passed it into the constructor,
            // so a v4/v5 story that lays its status bar out at boot already saw the
            // pinned width. This re-write is now a safety net only — it still fires
            // when `host_screen` came back `None` (no terminal to query), and is a
            // harmless no-op re-write of the same value otherwise. An UNSET key is
            // left for the story pane's real measurement to keep following at every
            // later frame (`poll_zvm_resize`, ZMSD §8.4 — SQ-0532/A-F1).
            // v6 stories run at their NATIVE picture resolution (advertised before
            // boot in new_with_trace); the virtual screen is a v1–5 concern, so
            // leave the v6 native dims untouched here (SQ-0186).
            if s.machine.mem.version() != 6
                && (cfg.virtual_screen_rows.is_some() || cfg.virtual_screen_cols.is_some())
            {
                let rows = cfg.virtual_screen_rows.unwrap_or(s.machine.mem.read_byte(0x20) as u16);
                let cols = cfg.virtual_screen_cols.unwrap_or(s.machine.mem.read_byte(0x21) as u16);
                zvm::screen::write_screen_dims(
                    &mut s.machine.mem,
                    rows.clamp(1, 255) as u8,
                    cols.clamp(1, 255) as u8,
                );
            }
            s.machine.undo_cap = cfg.undo_levels;
            Box::new(s)
        }
        app::hints::LoadedStory::Glulx(bytes) => {
            let pict_blorb = resolve_pict_blorb(&story_path, cfg.images);
            match GlulxSession::new_in(
                game_dir.clone(),
                bytes,
                cfg.virtual_screen_cols.unwrap_or(app::config::FALLBACK_SCREEN_COLS) as u32,
                cfg.virtual_screen_rows.unwrap_or(app::config::FALLBACK_SCREEN_ROWS) as u32,
                cfg.acceleration,
                cfg.images,
                cfg.enable_sound,
                borderless,
                char_px,
                pict_blorb,
                &vfs_sidecar,
                theme_colours,
                // `--debug` (SQ-0465): trace from the first boot instruction so the
                // game's initialisation code is captured (a later `/debug` can't).
                cli.debug,
            ) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!("babelmap: cannot load Glulx story: {e:?}");
                    std::process::exit(1);
                }
            }
        }
        app::hints::LoadedStory::Scott(bytes) => match app::scott_session::ScottSession::new_with_trace(
            bytes,
            resolve_pict_blorb(&story_path, cfg.images),
            // `--debug` (SQ-0449/SQ-0464): trace from boot so the opening
            // occurrence pass (run inside the VM constructor) is captured.
            cli.debug,
        ) {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("babelmap: cannot load Scott Adams story: {e}");
                std::process::exit(1);
            }
        },
    };
    // Strip the game's own inline read prompt only when the dedicated command
    // bar is on (SQ-0264); otherwise inline-prompt mode keeps the game's ">".
    session.set_strip_prompt(cfg.command_bar);

    // `--debug` (SQ-0449): tracing is already on from the first boot instruction
    // (the Z-machine arm used `GameSession::new_with_trace` above), so the boot
    // PCs are already in the cumulative set. Here we just seed prior runs' coverage
    // from the per-story sidecar so those lines colour immediately too.
    if cli.debug {
        let loaded = app::pcset_store::read_pcs(&game_dir);
        if !loaded.is_empty() {
            session.seed_executed_pcs(&loaded);
        }
    }

    // Engine is up — stop the loading spinner and let it erase its line.
    loading_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = loading_spinner.join();

    // SQ-0319: announce the imported garglk config (after the spinner erased its
    // line, so the message isn't clobbered). Printed only when a sidecar applied.
    if let Some(line) = &garglk_line {
        eprintln!("babelmap: {line}");
    }

    // ── 2. IFID + map dir + load/create mapper ────────────────────────────────

    // `ifid` was computed above (before the engine build) so the per-game honor
    // override could feed the engine; `game_dir` (per-story storage) was computed
    // and created before the engine build too. The IFID stays for title/hint/
    // display and the per-game style reload below.
    let arc_file = default_state_path(&game_dir);

    // Load mapper (and optionally restore the game save) from the archive.
    let mut startup_transcript: app::state::LoadedTranscript = None;
    // Rewind/replay history carried from the archive when the game is auto-restored.
    let mut startup_history: Vec<app::history::TurnRecord> = Vec::new();
    // Command history (Up/Down recall) carried from the archive, always loaded.
    let mut startup_command_history: Vec<String> = Vec::new();
    // Turn counter carried from the archive when the game is auto-restored, so a
    // later save records the cumulative count rather than only post-resume moves.
    let mut startup_turns: Option<u32> = None;
    // When auto_load is false but a save exists and prompt_load_on_launch is true,
    // stash the save for the launch dialog instead of discarding it.
    let mut pending_resume_stash: app::state::PendingResume = None;
    let mut mapper = if arc_file.exists() {
        match load_archive(&arc_file) {
            Ok(ac) => {
                // Restore the machine from the saved game state only when auto_load is enabled.
                if cfg.auto_load {
                    match session.restore_state(&ac.engine_save()) {
                        Ok(()) => {
                            if let Some(scr) = ac.screen.clone() {
                                if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                                    app::session::restore_screen(zs, scr);
                                }
                            }
                            // The v6 screen: display list where the archive has one
                            // (SQ-0588), else canvas PNGs. No-op for non-v6 archives.
                            crate::engine_helpers::apply_v6_pictures(&mut *session, &ac);
                            // Hand Glulx back the room it was saved in (SQ-0523);
                            // no-op for zvm.
                            crate::engine_helpers::seed_resumed_location(&mut *session, &ac.meta);
                            startup_transcript = Some((ac.transcript, ac.transcript_kinds, ac.transcript_runs, ac.transcript_para, ac.transcript_images));
                            startup_history = ac.history;
                            // Restore the turn counter from the same archive (SQ-0429):
                            // the auto_load resume path mirrors the interactive restore,
                            // which sets state.turns = ac.meta.turns. Without this, a
                            // resumed game's later save records only post-resume moves.
                            startup_turns = Some(ac.meta.turns);
                        }
                        Err(e) => {
                            eprintln!("babelmap: warning: could not restore game from archive: {}; starting fresh", restore_error_msg(e));
                        }
                    }
                } else if cfg.prompt_load_on_launch && !ac.save.is_empty() {
                    pending_resume_stash = Some((ac.engine_save(), ac.transcript, ac.transcript_kinds, ac.screen));
                }
                if cfg.aux_storage != app::config::AuxStorage::Global {
                    session.set_aux_data(ac.aux.clone());
                }
                startup_command_history = ac.command_history;
                // The map is part of the game's state: it loads only when the state is
                // auto-resumed here. When auto_load is off it either rides the launch-resume
                // dialog (adopted on accept, see apply_launch_resume) or stays blank.
                if cfg.auto_load { ac.mapper } else { Mapper::default() }
            }
            Err(e) => {
                eprintln!("babelmap: warning: could not load archive {}: {}", arc_file.display(), e);
                Mapper::default()
            }
        }
    } else {
        Mapper::default()
    };

    // Startup: pre-load the per-game aux table from the global file when in
    // global mode.  In archive mode the table was populated above from the
    // loaded archive (if any).
    if cfg.aux_storage == app::config::AuxStorage::Global {
        session.set_aux_data(app::aux_store::read_global_aux(&game_dir));
    }

    // The per-story Glk file VFS sidecar was loaded into the VM before boot
    // (GlulxSession::new). A Glulx game may write a Glk file during boot (e.g.
    // CM's init cache); flush it now so it persists before the first turn and
    // survives an immediate quit (SQ-0290). For a Z-machine session vfs_dirty()
    // is always false, so this is a no-op there.
    if session.vfs_dirty() {
        let _ = app::vfs_store::write_vfs(&game_dir, &session.vfs_bytes());
        session.clear_vfs_dirty();
    }

    // ── 3. Seed initial transcript + starting room ────────────────────────────

    let mut state = AppState::default();
    // Apply the look resolved from style.toml above (before the engine build).
    state.colors = cs;
    state.symbols = set;
    // Stash the garglk.ini overlay (already folded into `cs` above) so the
    // post-IFID reload_style below — and every later /reload — re-applies it.
    state.garglk_overlay = garglk_overlay;
    for w in style_w1.into_iter().chain(style_w2) {
        state.push_notice(&format!("[{}]", w));
    }
    let (keymap, keymap_warnings) = app::keymap::KeyMap::resolve(&cfg.keymap);
    state.keymap = keymap;
    // Surface any keymap conflict warnings once in the transcript.
    for w in keymap_warnings {
        state.push_notice(&format!("[{}]", w));
    }
    let (hotkeys, hotkey_warnings) = app::keymap::HotkeyLayout::resolve(&cfg.hotkeys);
    state.hotkeys = hotkeys;
    for w in hotkey_warnings {
        state.push_notice(&format!("[{}]", w));
    }
    state.show_room_numbers = cfg.show_room_numbers;
    state.show_status_bar = cfg.show_status_bar;
    state.game_picker = game_picker;
    state.term_default_colors = term_default_colors;
    state.pane_sizes = app::state::PaneSizes {
        split_ratio: cfg.split_ratio,
        band_height: cfg.command_band.height,
        inv_dock_pct: cfg.inv_dock_pct,
    };
    // `[command_band] auto_open` — open the band with the story, for players who
    // want it as their default input surface rather than a thing to summon.
    let band_auto_open = cfg.command_band.auto_open;
    // SQ-0318: remember the global honor base so reload_style can recompute the
    // per-game > garglk > global precedence (and `auto` can fall back here).
    state.honor_game_colours_base = honor_game_colours_base;
    state.config = cfg;

    // Debug trace (trace feature): start a fresh log for this run and arm the
    // engine's screen-trace buffer per config; no-op when no section is active.
    if state.config.trace.any() {
        app::trace::truncate(&state.config.user_dir);
    }
    session.set_trace_screen(state.config.trace.screen);

    // Resolve the sound container + construct the audio backend (silent if the
    // feature is off, there is no device, or sound is disabled in config).
    // The load line prints here, before the alternate screen is entered, so it
    // stays in the normal terminal scrollback for verification after exit.
    state.sound_blorb = match blorb::resolve_resource_blorb(&story_path) {
        Some((b, path)) => {
            let count = |usage: &[u8; 4]| b.resources().iter().filter(|r| &r.usage == usage).count();
            let (sounds, images) = (count(b"Snd "), count(b"Pict"));
            let own = path == story_path;
            eprintln!(
                "babelmap: loaded resources from {}{} ({} sound{}, {} image{})",
                path.display(),
                if own { " (self)" } else { " (sidecar)" },
                sounds, if sounds == 1 { "" } else { "s" },
                images, if images == 1 { "" } else { "s" },
            );
            Some(b)
        }
        None => None,
    };
    if state.config.enable_sound {
        state.audio = Some(audio::AudioBackend::new(state.config.volume));
    }

    // Seed autocomplete with the story's parser vocabulary (room nouns are added live).
    state.dict_words = session.introspect().map(|i| i.vocabulary()).unwrap_or_default();

    // `[command_band] auto_open`: open the band with the story. Instant (no
    // slide) so the first frame is already the settled layout.
    if band_auto_open {
        let mut mapper_noop = mapper::mapper::Mapper::default();
        app::input::apply_action(
            app::input::Action::OpenCommandBand,
            &mut state,
            &mut mapper_noop,
        );
        state.band_dock.toggle_to(true, true);
    }

    // Push the game's opening banner and capture the title from it. Glulx returns
    // ordered elements (text + any startup/cover images); the Z-machine returns
    // empty here and falls back to the flat string path. Either way `banner` is the
    // banner text for title extraction (the elems' concatenated Text equals it).
    let banner_elems = session.take_transcript_elems();
    let banner: String = if banner_elems.is_empty() {
        session.take_transcript()
    } else {
        banner_elems
            .iter()
            .filter_map(|e| match e {
                app::session::TranscriptElem::Text { text, .. } => Some(text.as_str()),
                app::session::TranscriptElem::Image(_) => None,
            })
            .collect()
    };
    let banner_title = app::session::title_from_banner(&banner);
    state.title = app::session::resolve_title(None, &ifid, banner_title.as_deref(), &story_path);
    state.ifid = ifid.clone();
    state.game_dir = game_dir.clone();
    // Restore the per-game map-panel visibility (SQ-0304): if the user last hid
    // the map for this story, start with it hidden.
    if start_map_hidden {
        state.layout = app::state::Layout::TranscriptFull;
    }
    // Now that game_dir is set, re-resolve through reload_style so the per-game
    // override (<game_dir>/style.toml) is merged over the global at startup — the
    // initial resolve above is global-only (game_dir wasn't set yet). On a per-game
    // parse error the global look already set above stands.
    let _ = app::reload::reload_style(&mut state);
    if banner_elems.is_empty() {
        state.push_transcript(&banner);
    } else {
        app::state::apply_transcript_elems(&mut state, &banner_elems);
    }

    // A config.toml that doesn't load — bad syntax or a value of the wrong type — is
    // ignored WHOLESALE: TOML is one document, so a single stray character costs every
    // setting in the file. Say so, with the error TOML reported, rather than letting
    // the user wonder why their config has no effect (SQ-0580, SQ-0645). Saving is
    // refused while it's broken, so nothing overwrites it.
    if let Some(err) = state.config.config_error.clone() {
        let msg = format!(
            "{} could not be loaded ({err}) — running on defaults, and settings will \
             not be saved until it is fixed",
            state.config.config_file.display(),
        );
        state.push_transcript_internal(&msg, app::state::TranscriptKind::Warning);
    }

    // SQ-0663: the theme just rebuilt by reload_style above may carry its own
    // non-fatal diagnostics (a `parent` re-root naming an unknown selector, or
    // a cycle — see `Theme::warnings`), which used to fall back to registry
    // defaults with no visible sign anything was wrong. Surface them the same
    // way the broken-config.toml case just above is surfaced: one transcript
    // Warning line per issue (collapsed to a summary beyond a few — see
    // `describe_theme_warnings`), so a typo like `parent = "acent"` in
    // style.toml no longer degrades silently.
    for line in app::theme::resolve::describe_theme_warnings(state.colors.theme.warnings()) {
        state.push_transcript_internal(&line, app::state::TranscriptKind::Warning);
    }

    // One-time notice: config.toml no longer carries style — those moved to style.toml.
    if let Ok(raw_cfg) = std::fs::read_to_string(app::config::config_path(cli)) {
        if app::config::config_has_style_sections(&raw_cfg) {
            state.push_transcript_internal(
                "config.toml [colors]/[symbols] are no longer used — styling lives in style.toml ([colors] there; map glyph presets are now [map] keys)",
                app::state::TranscriptKind::Warning,
            );
        }
    }

    // Observe the starting room so it appears on the map immediately.
    let start_loc = session.current_location();
    if let Some(snap) = start_loc {
        let snap_number = snap.number;
        let seed_result = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(snap),
            quit: session.has_quit(),
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        apply_turn(&mut mapper, "", &seed_result, &mut state.death_watch);
        crate::turn::flush_screen_trace(&state.config.user_dir, &mut *session, state.config.trace.screen);
        crate::turn::flush_v6_trace(&state.config.user_dir, &mut *session, state.config.trace.v6);
        if state.config.trace.any() {
            let ptr = format!(
                "[trace → {}: {}]",
                state.config.user_dir.join("trace.log").display(),
                state.config.trace.active_list(),
            );
            state.push_transcript_internal(&ptr, app::state::TranscriptKind::Meta);
        }
        let rid = snap_number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        // Recenter using a default pane size; will be corrected after first draw.
        state.recenter_on(
            mapper
                .graph
                .room(rid)
                .and_then(|r| r.pos)
                .unwrap_or((0, 0)),
            40,
            24,
        );
    }

    // [more] pager for the OPENING BANNER (SQ-0532 wave-5). The banner is one
    // batch of game output exactly like a turn's, and a v6 story box is small —
    // Zork Zero's window 0 holds 20 rows and its prologue wraps to 23, so the
    // view pinned to the newest rows and the illuminated drop-cap that opens the
    // game scrolled off before it was ever seen. Arm the pager the way
    // `finish_command_turn` arms it for a turn: the first frame measures the rows
    // the banner actually produced and engages ONLY if it overflowed the story
    // viewport, parking the view on the first screenful. Same rule as a turn
    // (SQ-0539): a boot that ends on a `read_char` — a splash "press any key", a
    // startup menu — pages too, and the paging keys are swallowed by the pager
    // until the view catches up rather than answering that read. Skipped entirely
    // for a resumed transcript (below): that scrollback was already read, and
    // paging it would park a returning player mid-history.
    if startup_transcript.is_none()
        && app::pager::should_arm(session.pending_input(), app::pager::more_suppressed(&*session))
    {
        state.pager.arm(0);
    }

    // If an archived transcript was loaded on startup, replace the fresh one.
    if let Some((lines, kinds, runs, para, images)) = startup_transcript {
        state.transcript = lines;
        state.clear_anchor = None;
        state.transcript_kinds = kinds;
        state.transcript_runs = runs;
        state.transcript_para = para;
        state.reset_transcript_sidecars();
        // Re-attach inline images after the sidecar reset so an auto-resumed
        // transcript renders its embedded art (SQ-0518).
        state.transcript_images = images;
    }
    if !startup_history.is_empty() {
        state.history = startup_history;
    }
    state.command_history = startup_command_history;
    if let Some(turns) = startup_turns {
        state.turns = turns;
    }

    // If a save was found but auto_load is off and prompt_load_on_launch is on,
    // open the launch dialog so the user can choose to resume or start fresh.
    if let Some(stash) = pending_resume_stash {
        state.pending_resume = Some(stash);
        state.overlays.launch_dialog = true;
        state.overlays.dialog_focus = 0;
    }

    // If the game quit immediately (e.g. czech.z5 or the glk-dev self-checking
    // file tests), bail without entering raw mode. Such stories run to completion
    // and quit before ever asking for input, so their output IS the point — print
    // the captured transcript to stdout instead of discarding it.
    if session.has_quit() {
        for line in &state.transcript {
            println!("{}", line);
        }
        eprintln!("babelmap: story ended without asking for input.");
        std::process::exit(0);
    }

    // `--debug` (SQ-0449): persist the cumulative coverage on story-end, and
    // auto-open the debug inspector now (mirrors `/debug`'s open recipe). Tracing
    // was already enabled above; `set_debug_trace(true)` here is idempotent.
    state.persist_debug_trace = cli.debug;
    if cli.debug && session.debugger().is_some() {
        session.set_debug_trace(true);
        let dbg = session.debugger().expect("checked above");
        let mut panel = app::debug_panel::DebugPanelState::new(dbg.pc());
        panel.apply_engine_layout(dbg);
        panel.refresh(dbg);
        state.debug = Some(panel);
        state.focus = app::state::Focus::Map;
    }

    // ── 4. Terminal setup ─────────────────────────────────────────────────────

    // Install the panic hook FIRST so that any panic after this point (including
    // one between enable_raw_mode and EnterAlternateScreen) restores the terminal.
    install_panic_hook(state.config.user_dir.clone());

    // SQ-0586: from here until teardown, fd 2 goes to <user_dir>/stderr.log instead
    // of the terminal. C libraries (libasound through rodio/cpal) write there
    // directly — no Rust hook can catch them — and an ALSA underrun repeated during
    // power-save lands mid-frame and corrupts the render. Installed AFTER the panic
    // hook, whose `restore_terminal` puts fd 2 back before it prints, and after the
    // CLI/picker phases so ordinary terminal output is unaffected. A failure here is
    // not worth refusing to start over: the game runs, the chatter just stays visible.
    if let Err(e) = app::stderr_redirect::install(&state.config.user_dir.join("stderr.log")) {
        eprintln!("babelmap: could not redirect OS error output ({e}); it may corrupt the display");
    }

    if let Err(e) = enable_raw_mode() {
        eprintln!("babelmap: cannot enable raw mode (not a TTY?): {}", e);
        std::process::exit(1);
    }

    // From here on, raw mode is active — MUST restore on every exit path.

    if let Err(e) = execute!(stdout(), EnterAlternateScreen) {
        restore_terminal();
        eprintln!("babelmap: cannot enter alternate screen: {}", e);
        std::process::exit(1);
    }
    // Bracketed paste (SQ-0653). Without it the terminal replays a paste as raw
    // keystrokes, and the app cannot tell them from typing: a Tab fired
    // autocomplete, a leading '/' opened the command palette, and every newline
    // SUBMITTED a line to the game — so pasting a walkthrough played it. With the
    // mode on, the paste arrives as one `Event::Paste` and lands in the focused
    // field as literal text. Best-effort: a terminal that ignores the sequence
    // simply never sends `Event::Paste`, which is exactly today's behavior.
    // `restore_terminal()` always issues DisableBracketedPaste.
    let _ = execute!(stdout(), EnableBracketedPaste);

    // Mouse capture is opt-in (config `mouse = true`). Capture puts the terminal
    // in any-motion reporting mode, so every mouse movement wakes the event loop
    // and forces a full redraw; leaving it off keeps idle/scroll responsive and
    // preserves the terminal's native text selection. restore_terminal() always
    // issues DisableMouseCapture, which is a harmless no-op when it was never on.
    if state.config.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }

    let terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(e) => {
            restore_terminal();
            eprintln!("babelmap: cannot create terminal: {}", e);
            std::process::exit(1);
        }
    };

    BootResult {
        session,
        mapper,
        state,
        terminal,
        game_dir,
        ifid,
        arc_file,
        story_bytes,
        story_path,
        data_base,
    }
}

/// Cooked-mode y/N prompt on the normal terminal (before the alt-screen is
/// entered). A non-interactive stdin (piped or EOF) reads as "no".
fn prompt_yes_no(question: &str) -> bool {
    use std::io::Write as _;
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(line.trim().chars().next(), Some('y') | Some('Y')),
    }
}
