//! Scout what the v6 hybrid ring is working from, for a real frame (SQ-0894/0892).
//!
//! It draws no pixels and negotiates no terminal: it boots the story headlessly,
//! takes the same `ScreenModel` the renderer gets, and runs the same layout
//! primitives. That makes it the cheapest of the three render-testing layers — reach
//! for `examples/pty_capture` when the model looks right and the screen does not.
//!
//! It boots through the same profile chain `startup.rs` does — the medium the mount
//! resolved picks the machine, which picks the palette, the interpreter number, the
//! standard window and the default colours (SQ-0901). It prints the profile and the
//! RELEASE it is holding on every line of output, because a disk image is a different
//! build and not the same story on other media (CLAUDE.md), and a disagreement
//! between this and a `/dump-windows` capture should be visible rather than deduced.
//!
//! What it reports:
//!
//! * the story viewport, by the declared window BOX and by what the art leaves
//!   CLEAR (SQ-0894 step (b)) — the two agreed on every corpus frame, and this is
//!   how that is re-measured rather than taken on trust;
//! * `--runs`, every chrome text run with its native origin, the sub-cell remainder
//!   of its mapped device column, and the column per-run rounding puts it in. This
//!   is the SQ-0892 view: a run is POSITIONED through the scale but ADVANCES one
//!   terminal column per character, and these are the numbers that argument turns
//!   on;
//! * `--bands`, the ring AS DRAWN: the strips the renderer classified, the band
//!   log the graphics layer wrote, and — the reason this exists — each band's
//!   MAGNIFICATION beside the frame's own letterbox scale (SQ-0898). One frame,
//!   one magnification: a column drawn in two pieces at two factors is a seam, and
//!   it is invisible in the rects. Do not read the band log's `native` field for
//!   this; on a crop it is a hash footprint carrying the area filter's halo, so
//!   below scale 1 it reads several pixels wide and neighbouring bands look like
//!   they overlap where they partition the canvas exactly;
//! * the GEOMETRIC ring, `pane − viewport`, which is the baseline the shipped
//!   content-carved ring is read against — not the ring itself.
//!
//! ```sh
//! cargo run -q -p app --example ring_scout -- --story stories/zork0-r393-s890714.z6
//! cargo run -q -p app --example ring_scout -- --all --size 100x40
//! cargo run -q -p app --example ring_scout -- --story "stories/James Clavell's Shogun.adf" --taps 1 --runs
//! cargo run -q -p app --example ring_scout -- --story stories/arthur-r74-s890714.z6 \
//!     --keys n --taps 12 --bands --size 70x19
//! ```
//!
//! `--keys` is the byte answered to a CHARACTER read while tapping through the
//! intro; `--all` takes each title's own from the corpus table unless this
//! overrides it. Arthur needs `n` for his "restore a saved position?" question.
//!
//! `--size` is the PANE in cells (default 98x37, what a 100x40 terminal leaves
//! after the app frame — the size §5's captures were measured at); `--cell` is the
//! terminal cell in pixels (default 8x18, what the capture harness negotiates).
//! `--taps N` presses exactly N keys, which is how a screen BETWEEN the splash and
//! gameplay is reached (Shogun's credits/menu is one tap in); `--no-tap` reports the
//! boot frame; `--turns N` then plays N turns, reporting the viewport at each.
//!
//! `--no-game-colours` renders `--bands` with `honor_game_colours` off. The two modes
//! are separate baselines and always have been (CLAUDE.md), and the ring is not the
//! same picture in both: honouring the game's colours floods every window's PAGE, and
//! SQ-0883 lived entirely in the flooded one — the shipped default — while the other
//! mode was correct throughout. An instrument that can only see one of them will
//! report a frame as clean at the very moment it is broken for the user.

use app::engine::Engine;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};
use ratatui::layout::Rect;

/// The corpus `--all` sweeps: every v6 title in `stories/` that draws a ring,
/// with the keys needed to get past its intro to a frame worth measuring.
const CORPUS: &[(&str, &str)] = &[
    ("stories/zork0-r393-s890714.z6", ""),
    ("stories/arthur-r74-s890714.z6", "n"),
    ("stories/shogun-r322-s890706.z6", ""),
    ("stories/journey-r83-s890706.z6", ""),
    ("stories/mysterious01.z6", "n"),
    ("stories/fmvpoker.z6", ""),
    ("stories/scopa.z6", ""),
    ("stories/advent.z6", ""),
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut story: Option<String> = None;
    let mut all = false;
    let mut pane_cells = (98u16, 37u16);
    let mut cell_px = (8u16, 18u16);
    let mut turns = 0usize;
    let mut no_tap = false;
    let mut honor_colours = true;
    let mut runs = false;
    let mut bands = false;
    let mut keys_override: Option<String> = None;
    let mut taps: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--story" => {
                story = args.get(i + 1).cloned();
                i += 1;
            }
            "--all" => all = true,
            "--keys" => {
                keys_override = args.get(i + 1).cloned();
                i += 1;
            }
            "--no-tap" => no_tap = true,
            "--no-game-colours" => honor_colours = false,
            "--runs" => runs = true,
            "--bands" => bands = true,
            "--taps" => {
                if let Some(v) = args.get(i + 1) {
                    taps = v.parse().ok();
                }
                i += 1;
            }
            "--turns" => {
                if let Some(v) = args.get(i + 1) {
                    turns = v.parse().unwrap_or(0);
                }
                i += 1;
            }
            "--size" => {
                if let Some(v) = args.get(i + 1) {
                    pane_cells = parse_pair(v).unwrap_or(pane_cells);
                }
                i += 1;
            }
            "--cell" => {
                if let Some(v) = args.get(i + 1) {
                    cell_px = parse_pair(v).unwrap_or(cell_px);
                }
                i += 1;
            }
            other => eprintln!("ring_scout: ignoring `{other}`"),
        }
        i += 1;
    }

    let targets: Vec<(String, String)> = if all {
        CORPUS
            .iter()
            .map(|(s, k)| (s.to_string(), keys_override.clone().unwrap_or_else(|| k.to_string())))
            .collect()
    } else if let Some(s) = story {
        vec![(s, keys_override.clone().unwrap_or_default())]
    } else {
        eprintln!("ring_scout: pass --story <file> or --all");
        std::process::exit(2);
    };

    println!(
        "pane {}x{} cells, cell {}x{} px  →  {}x{} device px\n",
        pane_cells.0, pane_cells.1, cell_px.0, cell_px.1,
        pane_cells.0 as u32 * cell_px.0 as u32,
        pane_cells.1 as u32 * cell_px.1 as u32,
    );

    for (path, keys) in targets {
        println!("═══ {path}");
        match scout(&path, &keys, pane_cells, cell_px, turns, no_tap, runs, bands, taps, honor_colours) {
            Ok(()) => {}
            Err(e) => println!("  SKIP: {e}\n"),
        }
    }
}

/// The cell rect the CONTENT leaves for story text — SQ-0894 step (b).
///
/// `v6_layout::story_viewport` used to be this, and SQ-0894 deleted it after
/// measuring the answer to be identical to the declared window box on every corpus
/// frame. Kept here, against the surviving native-space `story_clear_native`, so
/// the measurement can be re-run rather than taken on trust.
fn clear_viewport_cells(
    story: Option<&app::engine::PositionedWindow>,
    gfx: &image::RgbaImage,
    scale: &v6::Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> Rect {
    let Some((left, top, w, h)) = v6::story_clear_native(story, gfx) else {
        return Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let (cw, ch) = (cell_px.0.max(1) as f32, cell_px.1.max(1) as f32);
    let dev = |v: u32, off: u32, per: f32| (off as f32 + v as f32 * scale.s) / per;
    let cell_left = dev(left, scale.off_x, cw).ceil() as u16;
    let cell_top = dev(top, scale.off_y, ch).ceil() as u16;
    let cell_right = dev(left + w, scale.off_x, cw).floor() as u16;
    let cell_bottom = dev(top + h, scale.off_y, ch).floor() as u16;
    let width = cell_right.saturating_sub(cell_left).max(1).min(pane_cells.0.saturating_sub(cell_left));
    let height = cell_bottom.saturating_sub(cell_top).max(1).min(pane_cells.1.saturating_sub(cell_top));
    Rect { x: cell_left, y: cell_top, width, height }
}

fn parse_pair(s: &str) -> Option<(u16, u16)> {
    let (a, b) = s.split_once(['x', 'X'])?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

#[allow(clippy::too_many_arguments)]
fn scout(
    path: &str,
    keys: &str,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
    turns: usize,
    no_tap: bool,
    want_runs: bool,
    want_bands: bool,
    taps: Option<usize>,
    honor_colours: bool,
) -> Result<(), String> {
    // Disk images (.adf/.po/.2mg/.dsk) are mounted, not read — a medium carries a
    // different RELEASE, not the same story on other media (CLAUDE.md). The mount's
    // second answer is the medium THIS story came off, which on a hybrid disc is not
    // the image's own format (SQ-0876) — so it is what the profile resolves from.
    let p = std::path::Path::new(path);
    let (bytes, disk_image) = match app::hints::load_mounted_story(p) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(e) => return Err(format!("{path}: {e:?}")),
    };
    if bytes.first() != Some(&6) {
        return Err("not a v6 story".into());
    }
    // SQ-0901: boot the way `startup.rs` does. Passing `interpreter_number: None`
    // and skipping `set_palette` measured a frame the renderer never draws — a
    // 172-native-px flank on the Amiga Arthur where the app renders a 30px pole —
    // and an instrument that silently disagrees with the app on a whole class of
    // media is worse than no instrument at all.
    let mut picts = app::graphics::PictSource::resolve(p, None);
    // No tier-3 archive is named here, so the machine comes from the medium alone.
    let profile = app::interpreter::InterpreterProfile::resolve(p, None, None, disk_image);
    zvm::screen::set_palette(profile.palette());
    let dims = picts.all_pict_dims();
    let std_win = picts.std_window().or_else(|| profile.std_window());
    println!(
        "  booted as {:?}{}  ·  release {} serial {}",
        profile,
        disk_image.map(|m| format!(" off {m:?}")).unwrap_or_default(),
        u16::from_be_bytes([bytes[2], bytes[3]]),
        String::from_utf8_lossy(&bytes[0x12..0x18]),
    );
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        dims,
        std_win,
        profile.default_colours(),
        None,
    )
    .map_err(|e| format!("boot: {e:?}"))?;
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();

    // Tap through the intro to a frame that has a ring on it — unless asked to
    // report the BOOT frame as it stands, which is the only way to see a screen the
    // router sends to the composite before a viewport is ever computed.
    let tap_count = taps.unwrap_or(if no_tap { 0 } else { 6 });
    let stop_at_line = taps.is_none();
    for _ in 0..tap_count {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
                if stop_at_line {
                    break;
                }
            }
            InputKind::Char => {
                let b = keys.bytes().next().unwrap_or(13);
                let _ = s.submit_char(b);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }

    // …then as many further turns as asked, reporting (b) at each: a declared box
    // that avoids the art at BOOT may stop doing so once the game draws into its
    // own story window.
    for t in 0..=turns {
        if t > 0 {
            match s.pending_input() {
                InputKind::Line => { let _ = s.submit("look"); }
                InputKind::Char => { let _ = s.submit_char(13); }
                InputKind::Event => { let _ = s.submit(""); }
            }
            let _ = s.take_transcript();
        }
        let m = s.screen();
        let app::engine::WinNode::Layered(it) = &m.root else { continue };
        let nat = v6::native_extent(it.as_slice());
        let lay = v6::classify_windows(it.as_slice());
        let pd = (pane_cells.0 as u32 * cell_px.0 as u32, pane_cells.1 as u32 * cell_px.1 as u32);
        let sc = v6::uniform_scale(nat, pd);
        let vp_box = v6::story_viewport_box(lay.story, &sc, pane_cells, cell_px);
        let g = v6::build_graphics_canvas(&lay.chrome, nat);
        let vp_clear = clear_viewport_cells(lay.story, &g, &sc, pane_cells, cell_px);
        {
            println!(
                "  turn {t}: {} box {}x{}@({},{}) content {}x{}@({},{}); win0 {:?} -> clear {:?}",
                if vp_box == vp_clear { "same  " } else { "DIFFERS" },
                vp_box.width, vp_box.height, vp_box.x, vp_box.y,
                vp_clear.width, vp_clear.height, vp_clear.x, vp_clear.y,
                lay.story.map(|w| (w.x_px, w.y_px, w.w_px, w.h_px)),
                v6::story_clear_native(lay.story, &g),
            );
        }
    }

    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else {
        return Err("not a Layered v6 frame".into());
    };

    let native = v6::native_extent(items.as_slice());
    let layout = v6::classify_windows(items.as_slice());
    let pane_dev = (pane_cells.0 as u32 * cell_px.0 as u32, pane_cells.1 as u32 * cell_px.1 as u32);
    let scale = v6::uniform_scale(native, pane_dev);
    let pane = Rect::new(0, 0, pane_cells.0, pane_cells.1);
    let viewport = v6::story_viewport_box(layout.story, &scale, pane_cells, cell_px);

    println!(
        "  native {}x{}  scale {:.4} off ({},{})  viewport {}x{} at ({},{})  chrome windows {}",
        native.0, native.1, scale.s, scale.off_x, scale.off_y,
        viewport.width, viewport.height, viewport.x, viewport.y,
        layout.chrome.len(),
    );

    // SQ-0894 step (b): the text region the PANELS leave, versus the raw window box
    // hybrid takes today. `story_viewport` is the shrink-until-clear-then-quantize
    // wrapper that has never had a production caller; measured here against the
    // ART-ONLY canvas, which is the oracle raster uses and the one §3(b) says it
    // needs (against the full chrome canvas — which carries rasterised TEXT as
    // opaque pixels — Shogun's declared 548x64 box comes back 548x16).
    let gfx = v6::build_graphics_canvas(&layout.chrome, native);
    let clear = clear_viewport_cells(layout.story, &gfx, &scale, pane_cells, cell_px);
    let native_clear = v6::story_clear_native(layout.story, &gfx);
    println!(
        "  (b) viewport by BOX   {}x{} at ({},{})\n      viewport by CONTENT {}x{} at ({},{}){}",
        viewport.width, viewport.height, viewport.x, viewport.y,
        clear.width, clear.height, clear.x, clear.y,
        if clear == viewport { "   [no change]" } else { "   <-- DIFFERS" },
    );
    if let Some((l, t, w, h)) = native_clear {
        let sw = layout.story.map(|s| (s.x_px, s.y_px, s.w_px, s.h_px));
        println!("      native: declared {sw:?} -> clear ({l},{t},{w},{h})");
    }

    // SQ-0892: every chrome run with the numbers the quantization argument turns on
    // — native origin, the sub-cell remainder of its mapped device x, and the cell
    // per-run rounding puts it in. Grouped by native text row, which is the key the
    // grouping rule is a refinement of.
    if want_runs {
        use std::collections::BTreeMap;
        let cw = cell_px.0.max(1) as f32;
        let mut rows: BTreeMap<u16, Vec<(&app::engine::PxText, u16)>> = BTreeMap::new();
        for it in &layout.chrome {
            if let app::engine::WinNode::Grid(g) = &it.node {
                for t in &g.px_texts {
                    rows.entry(t.y).or_default().push((t, it.w_px));
                }
            }
        }
        for it in items.iter() {
            let kind = match &it.node {
                app::engine::WinNode::Grid(g) => format!("Grid({} runs)", g.px_texts.len()),
                app::engine::WinNode::Buffer(b) => format!("Buffer(primary={})", b.primary),
                app::engine::WinNode::Graphics(_) => "Graphics".into(),
                _ => "other".into(),
            };
            println!("    win {}x{}@({},{}) {kind}", it.w_px, it.h_px, it.x_px, it.y_px);
        }
        println!("  RUNS ({} native rows):", rows.len());
        for (y, mut rr) in rows {
            rr.sort_by_key(|(t, _)| t.x);
            println!("    native y={y}  (y-1)%16={}", (y.max(1) - 1) % 16);
            for (t, win_w) in rr {
                let px = t.x.max(1) as f32 - 1.0;
                let dev = (scale.off_x as f32 + px * scale.s) / cw;
                println!(
                    "      x={:<4} n={:<3} win_w={:<4}({:>3}%) dev={:<8.3} cell={:<4} {:?}",
                    t.x,
                    t.text.chars().count(),
                    win_w,
                    win_w as u32 * 100 / native.0.max(1) as u32,
                    dev,
                    dev.round() as i32,
                    t.text,
                );
            }
        }
    }

    // SQ-0898: the ring as it is actually DRAWN — the strips the renderer classified
    // and the band log the graphics layer wrote, at this pane. The band log is the
    // only place a piece's native source and its device destination are reported
    // side by side, which is the pair the two extents must agree on: `native WxH`
    // (a crop of the shared scaled canvas, at the frame's one scale) against
    // `source WxH · resample A->B` (a caller-composed image, resized into the
    // band's cell box). A magnification that differs between two pieces of one
    // column is the defect this flag exists to catch, so it must be measurable
    // without a terminal.
    if want_bands {
        let mut st = app::state::AppState::default();
        st.colors = app::colors::ColorScheme::terminal_default();
        st.game_picker = Some(app::render::graphics::kitty_picker(cell_px.0, cell_px.1));
        st.config.v6_render = app::config::V6RenderMode::Hybrid;
        st.config.honor_game_colours = honor_colours;
        let mut buf = ratatui::buffer::Buffer::empty(pane);
        app::render::screen::render_story_pane(&model, false, None, &st, pane, &mut buf);
        println!("  RING AS DRAWN:");
        for c in st.v6_cell_map.borrow().iter() {
            if c.label.starts_with("strip:") || c.label.starts_with("menu:") {
                let (x, y, w, h) = c.cells;
                println!("    {} {}", c.label, fmt(Rect::new(x, y, w, h)));
            }
        }
        for line in &st.graphics_render.borrow().band_log {
            println!("    {line}");
        }
        // ONE FRAME, ONE MAGNIFICATION. Every band's device-per-native factor against
        // the frame's own letterbox scale, with the worst offender named: a column
        // drawn in two pieces at two magnifications is the seam SQ-0894 removed and
        // the corner fragment SQ-0898 is about, and neither is visible in the rects.
        let mags = st.graphics_render.borrow().band_mags.clone();
        for (r, fit, src, dst) in &mags {
            // How far the piece's far edge sits from where the frame's one scale puts
            // it, in device pixels: `|dst − src·s|` on the worse axis. Whole native
            // pixels cost up to half of one, so the honest allowance is one native
            // pixel (`s` device px, floored at 1 for a minifying pane).
            let off = (dst.0 as f32 - src.0 as f32 * scale.s)
                .abs()
                .max((dst.1 as f32 - src.1 as f32 * scale.s).abs());
            let bad = fit.on_the_letterbox_grid() && off > scale.s.max(1.0);
            println!(
                "    mag {:<20} {}x{}px from {}x{}n = {:.4}/{:.4} vs s={:.4}  → {off:.2} device px  [{fit:?}]{}",
                fmt(*r), dst.0, dst.1, src.0, src.1,
                dst.0 as f32 / src.0 as f32, dst.1 as f32 / src.1 as f32, scale.s,
                if bad { "   <-- DRIFT" } else { "" },
            );
        }
    }

    // The GEOMETRIC ring, `pane − viewport`. This is NOT what ships: SQ-0894 carves
    // the ring from CONTENT (`screen::content_ring_bands`, private to the render
    // module), so this is the baseline that rule is read against, not the answer.
    //
    // This block used to print an "axis-inverted" tiling beside it as PROPOSED —
    // flanks full pane height owning the corners. SQ-0894 MEASURED that proposal and
    // rejected it: it tiles exactly on all eight corpus frames and still breaks two,
    // cutting Arthur's full-width status row three ways and swallowing the left 37
    // columns of Journey's verb menu. It is gone rather than left printing, because a
    // scout that offers a known-wrong answer under the heading "PROPOSED" is worse
    // than one that offers none (SQ-0892).
    let bands = v6::chrome_bands(pane, viewport);
    println!("  GEOMETRIC ring ({} bands, pane − viewport):", bands.len());
    for (role, b) in &bands {
        println!("    {:<22} {}", format!("{role:?}"), fmt(*b));
    }
    let pane_area = pane.width as u32 * pane.height as u32;
    let vp_area = viewport.width as u32 * viewport.height as u32;
    let area: u32 = bands.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
    println!(
        "  tiling check: pane {pane_area} − viewport {vp_area} = {} · bands {area}{}",
        pane_area - vp_area,
        if area == pane_area - vp_area { " ✓" } else { "  ✗ MISMATCH" },
    );
    println!();
    Ok(())
}

fn fmt(r: Rect) -> String {
    format!("{}x{} at ({},{})", r.width, r.height, r.x, r.y)
}
