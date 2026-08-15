//! Scout the v6 hybrid ring's band tiling, current vs. proposed (SQ-0894).
//!
//! The ring is defined as `pane − viewport` by [`v6_layout::chrome_bands`], with
//! the TOP and BOTTOM bands spanning the full pane width — so they own the
//! corners, and each flank only gets the story viewport's vertical extent. That
//! definition is why a flank column is composed of up to three pieces drawn by
//! two different routines at three magnifications (§5 of
//! `docs/superpowers/specs/2026-08-15-v6-render-pipeline.md`, measured).
//!
//! SQ-0894's proposal is to invert which axis is full: flanks span the whole pane
//! height and own the corners, top/bottom span only the viewport's columns. It is
//! still an exact, non-overlapping tiling of `pane − viewport` — but the flank
//! becomes ONE rect, which is the "flank is one object" the quest asks for.
//!
//! This prints both tilings for a real frame so the change can be judged against
//! measurements rather than argued from the diagram. It draws no pixels and
//! negotiates no terminal: it boots the story headlessly, takes the same
//! `ScreenModel` the renderer gets, and runs the same layout primitives.
//!
//! ```sh
//! cargo run -q -p app --example ring_scout -- --story stories/zork0-r393-s890714.z6
//! cargo run -q -p app --example ring_scout -- --all --size 100x40
//! ```
//!
//! `--size` is the PANE in cells (default 98x37, what a 100x40 terminal leaves
//! after the app frame — the size §5's captures were measured at); `--cell` is the
//! terminal cell in pixels (default 8x18, what the capture harness negotiates).

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
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--story" => {
                story = args.get(i + 1).cloned();
                i += 1;
            }
            "--all" => all = true,
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
        CORPUS.iter().map(|(s, k)| (s.to_string(), k.to_string())).collect()
    } else if let Some(s) = story {
        vec![(s, String::new())]
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
        match scout(&path, &keys, pane_cells, cell_px) {
            Ok(()) => {}
            Err(e) => println!("  SKIP: {e}\n"),
        }
    }
}

fn parse_pair(s: &str) -> Option<(u16, u16)> {
    let (a, b) = s.split_once(['x', 'X'])?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn scout(
    path: &str,
    keys: &str,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if bytes.first() != Some(&6) {
        return Err("not a v6 story".into());
    }
    let mut picts = app::graphics::PictSource::resolve(std::path::Path::new(path), None);
    let dims = picts.all_pict_dims();
    let std_win = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_win, None, None)
        .map_err(|e| format!("boot: {e:?}"))?;
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();

    // Tap through the intro to a frame that has a ring on it.
    for _ in 0..6 {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
                break;
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

    let old = v6::chrome_bands(pane, viewport);
    let new = proposed_bands(pane, viewport);

    println!("  CURRENT ({} bands, top/bottom own the corners):", old.len());
    for (role, b) in &old {
        println!("    {:<22} {}", format!("{role:?}"), fmt(*b));
    }
    println!("  PROPOSED ({} bands, flanks own the corners, full pane height):", new.len());
    for (role, b) in &new {
        println!("    {role:<22} {}", fmt(*b));
    }

    // The property that must hold either way: an exact, non-overlapping tiling.
    let pane_area = pane.width as u32 * pane.height as u32;
    let vp_area = viewport.width as u32 * viewport.height as u32;
    let old_area: u32 = old.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
    let new_area: u32 = new.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
    println!(
        "  tiling check: pane {pane_area} − viewport {vp_area} = {} · current {old_area} · proposed {new_area}{}",
        pane_area - vp_area,
        if old_area == pane_area - vp_area && new_area == pane_area - vp_area { " ✓" } else { "  ✗ MISMATCH" },
    );

    // What the flank column gains. Under the current tiling the cells above and
    // below the viewport in a flank's columns belong to the full-width top/bottom
    // bands, drawn by a different routine off a different source — the seam.
    if let (Some(l_old), Some(l_new)) = (
        old.iter().find(|(role, _)| *role == v6::BandRole::LeftFlank).map(|(_, r)| *r),
        new.iter().find(|(r, _)| *r == "left flank"),
    ) {
        let stolen = l_new.1.height.saturating_sub(l_old.height);
        println!(
            "  left flank: {} rows now, {} proposed — {stolen} row(s) currently drawn by the full-width bands",
            l_old.height, l_new.1.height,
        );
    }
    println!();
    Ok(())
}

/// SQ-0894's proposed tiling: flanks span the FULL PANE HEIGHT and own the
/// corners; top/bottom span only the viewport's columns. Same exact tiling of
/// `pane − viewport`, opposite corner ownership.
fn proposed_bands(pane: Rect, viewport: Rect) -> Vec<(&'static str, Rect)> {
    let vx = viewport.x.clamp(pane.x, pane.right());
    let vy = viewport.y.clamp(pane.y, pane.bottom());
    let vr = viewport.right().clamp(vx, pane.right());
    let vb = viewport.bottom().clamp(vy, pane.bottom());
    let mut out = vec![
        ("left flank", Rect::new(pane.x, pane.y, vx - pane.x, pane.height)),
        ("right flank", Rect::new(vr, pane.y, pane.right() - vr, pane.height)),
        ("top", Rect::new(vx, pane.y, vr - vx, vy - pane.y)),
        ("bottom", Rect::new(vx, vb, vr - vx, pane.bottom() - vb)),
    ];
    out.retain(|(_, r)| r.width > 0 && r.height > 0);
    out
}

fn fmt(r: Rect) -> String {
    format!("{}x{} at ({},{})", r.width, r.height, r.x, r.y)
}
