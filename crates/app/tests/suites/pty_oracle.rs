//! A real terminal emulator's verdict on babelmap's bytes (SQ-0764).
//!
//! `pty_emitted_stream.rs` asserts on what OUR decoder read out of the stream.
//! That decoder and the renderer it audits were written by the same hands, so a
//! shared misreading of the kitty protocol is invisible to both — the harness
//! agrees with the bug. This binary adds the second opinion: the same bytes fed
//! to `qwertty-term-vt` (Ghostty's terminal core, ported), which resolves
//! placements the way a terminal actually would, and disagrees where it must.
//!
//! Two halves, deliberately unequal:
//!
//!   * `protocol` — PORTABLE, always runs, no fixture, no pty. Hand-authored
//!     kitty streams, a few hundred bytes each, pinning the continuation rule
//!     that makes babelmap's placement painting fragile. This is the part that
//!     is a real test rather than a snapshot: it asserts both directions, so it
//!     fails if the rule is broken AND if it is over-applied.
//!   * `emitter` — PORTABLE, always runs. babelmap's real emitter driven through
//!     a real `Terminal` over a byte sink, so the bytes judged are the ones a
//!     player's terminal receives, frame boundaries and buffer diff included.
//!     This is where SQ-0772 lives: the defect was a placement the damage model
//!     could not see, which no amount of hand-authored stream proves anything
//!     about.
//!   * `real_capture` — unix only, drives a real story through the pty, and
//!     asserts the two decoders agree on BOTH backgrounds and image coverage.
//!
//! Slow-test gating follows `pty_emitted_stream.rs`: nothing here is `#[ignore]`
//! (SQ-0368 reserved that for the multi-second full-game walkthroughs), and the
//! gitignored fixture makes the capture half skip vacuously rather than fail.

// Declared once by the group binary (`tests/pty.rs`) and shared by every pty
// suite in it; see `pty_emitted_stream.rs`.
use super::pty_stream;

/// The kitty row/column diacritics, by the value they encode. Index in kitty's
/// `rowcolumn-diacritics.txt` IS the value; these four were read out of
/// `qwertty-term-vt`'s own `src/kitty/unicode.rs` table (which the crate
/// unit-tests as sorted, and spot-checks at indices 30 and 294), not recalled.
const D: [char; 4] = ['\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}'];

/// Index 164 in that same table — the third diacritic's job is the image id's
/// HIGH BYTE, and 164 is a value with bits set, which is the whole point: an id
/// whose high byte is zero survives losing this diacritic.
const HIGH_164: char = '\u{1DC0}';

/// An image id whose high byte is 164. `ESC[38;2;r;g;b` can only carry the low
/// 24 bits, so this id exists ONLY when the high-byte diacritic is present.
const ID_HIGH: u32 = (164 << 24) | 0x00b0_0001;
const ID_LOW_R: u8 = 0xb0;
const ID_LOW_G: u8 = 0x00;
const ID_LOW_B: u8 = 0x01;

/// A terminal wide enough to hold the 4-cell art with room either side, and a
/// cell size in the shape a real one answers `CSI 16 t` with.
const COLS: u16 = 20;
const ROWS: u16 = 6;
const CELL_W: u32 = 8;
const CELL_H: u32 = 16;

/// Where the art is painted: 4 cells wide starting at column 2, on rows 1..2.
const ART_LEFT: u16 = 2;
const ART_COLS: u16 = 4;
const ART_TOP: u16 = 1;
const ART_ROWS: u16 = 2;

/// Base64 without a dependency. `qwertty-term-vt` has one of these inside it;
/// pulling `base64` into this crate to encode 32 bytes of test payload would be
/// a production dependency's worth of ceremony for eight lines.
fn b64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

/// A `a=T,U=1` transmit-and-display of a solid RGBA image, declaring a
/// `ART_COLS x ART_ROWS` cell grid — the shape babelmap sends. `z=3` is
/// deliberately non-default so the authored z can be told apart from the -1
/// upstream reports for every virtual placement.
fn transmit(id: u32) -> String {
    let (w, h) = (u32::from(ART_COLS) * CELL_W, u32::from(ART_ROWS) * CELL_H);
    let rgba = [7u8, 8, 9, 255].repeat((w * h) as usize);
    format!(
        "\x1b_Gq=2,a=T,U=1,i={id},f=32,t=d,s={w},v={h},c={ART_COLS},r={ART_ROWS},z=3,m=0;{}\x1b\\",
        b64(&rgba)
    )
}

/// One row of placeholders in babelmap's own shape: the LEAD cell carries the
/// full diacritic triple (image row, image column, id high byte) and every cell
/// after it is a bare `U+10EEEE` relying on the continuation rule.
fn placeholder_row(row: u16, high: char) -> String {
    let mut s = format!(
        "\x1b[{};{}H\x1b[38;2;{ID_LOW_R};{ID_LOW_G};{ID_LOW_B}m",
        ART_TOP + row + 1,
        ART_LEFT + 1
    );
    s.push('\u{10EEEE}');
    s.push(D[row as usize]);
    s.push(D[0]);
    s.push(high);
    for _ in 1..ART_COLS {
        s.push('\u{10EEEE}');
    }
    s.push_str("\x1b[39m");
    s
}

/// The whole frame: the upload plus both placeholder rows.
fn full_frame(id: u32, high: char) -> String {
    let mut s = transmit(id);
    for row in 0..ART_ROWS {
        s.push_str(&placeholder_row(row, high));
    }
    s
}

/// Overpaint the lead cell of every art row with a plain space, exactly as a
/// later frame drawing a divider down that column would.
fn overpaint_lead_cells(s: &mut String) {
    for row in 0..ART_ROWS {
        let _ = std::fmt::Write::write_fmt(
            s,
            format_args!("\x1b[{};{}H\x1b[0m ", ART_TOP + row + 1, ART_LEFT + 1),
        );
    }
}

mod protocol {
    use super::*;
    use crate::pty_stream::oracle::{self, Origin};

    /// The baseline: a run with its lead cell intact is an image, and the oracle
    /// says where. Without this direction the next test would pass for a
    /// terminal that never resolves anything at all.
    #[test]
    fn a_run_with_its_lead_cell_intact_resolves_to_the_expected_placement() {
        let res = oracle::resolve(full_frame(ID_HIGH, HIGH_164).as_bytes(), COLS, ROWS, CELL_W, CELL_H);

        assert_eq!(
            res.placements.len(),
            1,
            "one image, aggregated from its per-row entries: {}",
            res.describe_placements()
        );
        let p = &res.placements[0];
        assert_eq!(p.image_id, ID_HIGH, "the full 32-bit id, high byte and all");
        assert_eq!(
            (p.top, p.bottom, p.left, p.right),
            (ART_TOP, ART_TOP + ART_ROWS - 1, ART_LEFT, ART_LEFT + ART_COLS - 1),
            "the rect the placeholder cells describe: {}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART_ROWS) * usize::from(ART_COLS));
        assert_eq!(p.origin, Origin::Virtual);
        // The authored z, not the -1 upstream reports for every virtual
        // placement — the difference is the reason `ImageRect::z` reads storage.
        assert_eq!(p.z, 3, "the transmit asked for z=3");

        // Every cell of the rect, and nothing outside it.
        for row in 0..ROWS {
            for col in 0..COLS {
                let inside = (ART_TOP..ART_TOP + ART_ROWS).contains(&row)
                    && (ART_LEFT..ART_LEFT + ART_COLS).contains(&col);
                assert_eq!(
                    res.cell(row, col).image_id,
                    if inside { Some(ID_HIGH) } else { None },
                    "cell ({row},{col})"
                );
            }
        }
    }

    /// The rule that paid for the crate. Overpaint the lead cell and the run
    /// loses its high-byte diacritic; the id truncates to the low 24 bits, the
    /// lookup misses, and a real terminal draws NOTHING — while the surviving
    /// cells are still `U+10EEEE` placeholders our own decoder happily reports
    /// as an image.
    #[test]
    fn a_run_whose_lead_cell_was_overpainted_resolves_to_nothing() {
        let mut bytes = full_frame(ID_HIGH, HIGH_164);
        overpaint_lead_cells(&mut bytes);
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H);

        assert!(
            res.placements.is_empty(),
            "the orphaned run names {:#010x} truncated to its low 24 bits, an image the \
             terminal does not hold — it must draw nothing:\n{}",
            ID_HIGH,
            res.describe_placements()
        );
        for row in 0..ROWS {
            for col in 0..COLS {
                assert_eq!(res.cell(row, col).image_id, None, "cell ({row},{col}) must be bare");
            }
        }

        // And the trap this whole file exists to spring: OUR decoder still sees
        // the placeholder cells and still calls them an image. Both readings are
        // honest about what they measure; only the oracle's is about pixels.
        let mut ours = crate::pty_stream::decode::Term::new(COLS, ROWS);
        ours.feed(bytes.as_bytes());
        let mine = ours.placements();
        assert_eq!(mine.len(), 1, "our decoder reports placeholder cells, which are still there");
        assert_eq!(mine[0].image_id, ID_HIGH & oracle::ID_MASK, "and only their low 24 bits");
        assert!(
            !oracle::disagreements(&ours, &res).is_empty(),
            "so the two decoders MUST disagree on this stream"
        );
    }

    /// The other direction, which is what stops the test above from passing for
    /// the wrong reason: overpaint the lead cells and then RE-EMIT them, as a
    /// correct repaint would, and the placement resolves exactly as before.
    #[test]
    fn a_partial_overpaint_that_re_emits_the_lead_cell_still_resolves() {
        let mut bytes = full_frame(ID_HIGH, HIGH_164);
        overpaint_lead_cells(&mut bytes);
        for row in 0..ART_ROWS {
            bytes.push_str(&placeholder_row(row, HIGH_164));
        }
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H);

        assert_eq!(res.placements.len(), 1, "{}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(p.image_id, ID_HIGH);
        assert_eq!(
            (p.top, p.bottom, p.left, p.right),
            (ART_TOP, ART_TOP + ART_ROWS - 1, ART_LEFT, ART_LEFT + ART_COLS - 1),
            "{}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART_ROWS) * usize::from(ART_COLS));
    }

    /// The failure mode is WORSE when the id's high byte is zero, which is
    /// babelmap's own id range (`0x00B0_xxxx`, `render/graphics.rs`): the
    /// truncated id still names a real image, so the lookup succeeds and the
    /// orphaned run resolves — but with no row diacritic it claims image row 0,
    /// so every row of the art redraws the art's FIRST row, and it starts one
    /// cell right of where it should. Silent corruption instead of a blank.
    #[test]
    fn a_zero_high_byte_id_survives_the_overpaint_but_draws_the_wrong_fragment() {
        let id: u32 = ID_HIGH & oracle::ID_MASK;
        let mut bytes = full_frame(id, D[0]);
        overpaint_lead_cells(&mut bytes);
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H);

        assert_eq!(res.placements.len(), 1, "the truncated id still resolves: {}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.left, p.right),
            (ART_LEFT + 1, ART_LEFT + ART_COLS - 1),
            "the run now starts where the lead cell used to be: {}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART_ROWS) * (usize::from(ART_COLS) - 1));
    }

    /// A painted background is not an image, to the oracle either. The mirror of
    /// `decode.rs`'s own first test, run through the real emulator so the two
    /// models are pinned to the same distinction.
    #[test]
    fn a_painted_background_is_not_a_placement() {
        let mut bytes = String::new();
        for row in 0..ART_ROWS {
            bytes.push_str(&format!(
                "\x1b[{};{}H\x1b[48;2;40;30;90m    \x1b[0m",
                ART_TOP + row + 1,
                ART_LEFT + 1
            ));
        }
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H);

        assert!(res.placements.is_empty(), "paint is not a placement");
        for col in ART_LEFT..ART_LEFT + ART_COLS {
            let c = res.cell(ART_TOP, col);
            assert_eq!(c.bg, crate::pty_stream::decode::Color::Rgb(40, 30, 90));
            assert_eq!(c.image_id, None);
        }

        // And our decoder reads it the same way, so this stream is agreement.
        let mut ours = crate::pty_stream::decode::Term::new(COLS, ROWS);
        ours.feed(bytes.as_bytes());
        let d = oracle::disagreements(&ours, &res);
        assert!(d.is_empty(), "the two decoders must agree on a plain painted fill: {d:#?}");
    }
}

/// The real emitter, the real ratatui diff, a real terminal — no pty, no fixture,
/// no story (SQ-0772).
///
/// The `protocol` module above hand-authors the streams it judges, which pins the
/// RULE but not babelmap's obedience to it. `real_capture` below judges babelmap's
/// own bytes, but needs a pty, a commercial story file and a couple of seconds.
/// This module sits between them: it drives `GraphicsRender` through a real
/// `Terminal` over a byte sink, so the bytes are the ones a player's terminal would
/// receive, and resolves them through the same emulator — every frame boundary,
/// buffer diff and cell-skip decision included, which is exactly where this defect
/// lived. It runs everywhere and takes milliseconds.
mod emitter {
    use ratatui::Terminal;
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};
    use ratatui::widgets::Widget;
    use ratatui::{TerminalOptions, Viewport};

    use app::engine::GraphicsWindow;
    use app::render::graphics::{GraphicsRender, kitty_picker};

    use crate::pty_stream::oracle;

    const COLS: u16 = 40;
    const ROWS: u16 = 12;
    const CELL_W: u16 = 8;
    const CELL_H: u16 = 18;

    /// The graphics window's cell rect. Column 3 is the LEAD column — the one a
    /// divider drawn down the screen's left flank lands on, and the one whose loss
    /// used to orphan the rest of every row.
    const ART: Rect = Rect { x: 3, y: 2, width: 12, height: 6 };

    /// A canvas whose every pixel ROW is a different colour, so a placement that
    /// draws the wrong row of it is distinguishable from one that draws the right
    /// one. A flat canvas would let the corrupt reading pass.
    fn window(version: u64) -> GraphicsWindow {
        let (w, h) = (u32::from(ART.width) * u32::from(CELL_W), u32::from(ART.height) * u32::from(CELL_H));
        let mut canvas = image::RgbaImage::new(w, h);
        for (_, y, p) in canvas.enumerate_pixels_mut() {
            *p = image::Rgba([(y % 251) as u8, 40, 200, 255]);
        }
        GraphicsWindow { win: 1, canvas: std::sync::Arc::new(canvas), version, upscale: false }
    }

    /// The backend's byte sink, kept on our side of the writer: ratatui-crossterm's
    /// own `writer()` accessor is behind an unstable feature gate, and a shared
    /// buffer is a smaller thing to depend on than an unstable API.
    #[derive(Clone, Default)]
    struct Sink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A `Terminal` writing into a byte sink we can hand to the emulator. The
    /// viewport is FIXED so nothing consults the real terminal this test may or may
    /// not be attached to.
    fn terminal() -> (Terminal<CrosstermBackend<Sink>>, Sink) {
        let sink = Sink::default();
        let term = Terminal::with_options(
            CrosstermBackend::new(sink.clone()),
            TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, COLS, ROWS)) },
        )
        .expect("a fixed viewport needs no terminal to size itself against");
        (term, sink)
    }

    /// Resolve everything written so far the way a terminal would.
    fn resolve(sink: &Sink) -> oracle::Resolved {
        oracle::resolve(&sink.0.borrow(), COLS, ROWS, u32::from(CELL_W), u32::from(CELL_H))
    }

    /// Draw the art, and nothing else.
    fn frame_with_art(term: &mut Terminal<CrosstermBackend<Sink>>, gr: &mut GraphicsRender, version: u64) {
        let picker = kitty_picker(CELL_W, CELL_H);
        term.draw(|f| gr.render(&picker, &window(version), ART, Style::default(), f.buffer_mut()))
            .expect("drawing into a byte sink cannot fail");
    }

    /// The art's every cell, and the image pixel row landing on each — the reading
    /// that separates a healthy placement from an orphaned one.
    fn source_rows(res: &oracle::Resolved) -> Vec<Option<u32>> {
        (ART.y..ART.y + ART.height).map(|row| res.cell(row, ART.x).source_y).collect()
    }

    /// The baseline, and the direction that stops the rest passing vacuously: the
    /// emitter's own bytes place the whole rect, and each screen row draws a
    /// DIFFERENT row of the image.
    #[test]
    fn the_emitters_bytes_place_every_cell_of_the_art() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);
        let res = resolve(&sink);

        assert_eq!(res.placements.len(), 1, "{}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.top, p.bottom, p.left, p.right),
            (ART.y, ART.y + ART.height - 1, ART.x, ART.x + ART.width - 1),
            "the whole window rect: {}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART.width) * usize::from(ART.height));

        let rows = source_rows(&res);
        assert!(
            rows.windows(2).all(|w| w[0] < w[1]),
            "each screen row must draw a LOWER row of the image than the one above it, else \
             the placement is redrawing one row over and over: {rows:?}"
        );
    }

    /// SQ-0772's corruption mode, through the real emitter. A later frame draws a
    /// divider down the art's lead column and re-places the art everywhere else —
    /// the shape of Journey's chrome ring trimming the raster composite's left edge.
    /// The survivors must still name their own image rows.
    ///
    /// Before the fix the whole row leaned on that lead cell, so its loss left the
    /// rest of the row anchorless: babelmap's ids have a zero high byte, so the run
    /// still resolved — to the image's FIRST row, on every screen row.
    #[test]
    fn overpainting_the_lead_column_leaves_the_survivors_naming_their_own_rows() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);

        let picker = kitty_picker(CELL_W, CELL_H);
        term.draw(|f| {
            let buf = f.buffer_mut();
            gr.render(&picker, &window(1), ART, Style::default(), buf);
            // …and a divider down the art's first column, drawn after it.
            for y in ART.y..ART.y + ART.height {
                if let Some(cell) = buf.cell_mut((ART.x, y)) {
                    cell.set_symbol("\u{2502}").set_style(Style::default().fg(Color::Rgb(9, 9, 9)));
                }
            }
        })
        .expect("drawing into a byte sink cannot fail");

        let res = resolve(&sink);
        assert_eq!(res.placements.len(), 1, "the art survives the trim: {}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.left, p.right),
            (ART.x + 1, ART.x + ART.width - 1),
            "the divider took the first column and nothing else: {}",
            p.describe()
        );

        let rows: Vec<Option<u32>> =
            (ART.y..ART.y + ART.height).map(|row| res.cell(row, ART.x + 1).source_y).collect();
        assert!(
            rows.windows(2).all(|w| w[0] < w[1]),
            "every surviving row must still draw its OWN row of the image; all-equal means the \
             run lost its anchor and is redrawing the first row down the whole rect: {rows:?}"
        );
    }

    /// The other half of the rule, and the reason the fix is buffer-visible cells
    /// rather than only self-describing ones: a frame that simply STOPS drawing the
    /// art must unpaint every placeholder cell it left behind.
    ///
    /// Honest about its own strength — this one passes on the old emitter too, in
    /// this shape. `Skip` is part of ratatui's cell equality, so a cell that was
    /// `Skip` last frame and plain this frame does diff and does get repainted. What
    /// the old shape could not survive was a placement whose cells stayed `Skip`
    /// frame after frame while its ANCHOR was overpainted (the test above), and this
    /// is the guard that the fix did not trade that away for a leak in the simpler
    /// direction.
    #[test]
    fn a_frame_that_stops_drawing_the_art_unpaints_every_placeholder_cell() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);
        assert_eq!(resolve(&sink).placements.len(), 1, "the art was placed to begin with");

        // The next frame draws ordinary text over the art's left third and leaves
        // the rest of its rows untouched — the ring/text layout that replaced the
        // raster composite in the capture.
        term.draw(|f| {
            let buf = f.buffer_mut();
            ratatui::widgets::Paragraph::new("text").render(Rect::new(ART.x, ART.y, 4, ART.height), buf);
        })
        .expect("drawing into a byte sink cannot fail");

        let res = resolve(&sink);
        assert!(
            res.placements.is_empty(),
            "nothing draws the art any more, so nothing may still be on screen: {}",
            res.describe_placements()
        );
        for row in 0..ROWS {
            for col in 0..COLS {
                assert_eq!(res.cell(row, col).image_id, None, "cell ({row},{col}) still carries an image");
            }
        }

        // And our own decoder must read it the same way, or the harness is lying.
        let mut ours = crate::pty_stream::decode::Term::new(COLS, ROWS);
        ours.feed(&sink.0.borrow());
        let d = oracle::disagreements(&ours, &res);
        assert!(d.is_empty(), "the two decoders must agree that the art is gone: {d:#?}");
    }

    /// The cheapness the old shape bought, kept: re-placing an unchanged image
    /// repaints nothing. A fix that made every cell buffer-visible by repainting it
    /// every frame would satisfy every test above and cost a screenful of
    /// placeholders per frame for ever.
    ///
    /// Measured between the SECOND and THIRD identical frames, because the second
    /// legitimately repaints one cell: the first frame's leading cell carries the
    /// image upload and the second's does not, so that one cell differs. From there
    /// on the buffer is identical and the diff is silent.
    #[test]
    fn re_placing_an_unchanged_image_repaints_no_placeholder_cells() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);
        frame_with_art(&mut term, &mut gr, 1);
        let settled = sink.0.borrow().len();
        assert!(
            String::from_utf8_lossy(&sink.0.borrow()[..settled]).contains('\u{10EEEE}'),
            "the frames so far did paint placeholders, so the next frame's silence means something"
        );

        frame_with_art(&mut term, &mut gr, 1);
        let added = String::from_utf8_lossy(&sink.0.borrow()[settled..]).to_string();
        assert!(
            !added.contains('\u{10EEEE}'),
            "an identical frame must diff to nothing but cursor bookkeeping; it repainted \
             placeholders: {added:?}"
        );
    }
}

/// The rasteriser (SQ-0775): the resolved screen drawn as pixels.
///
/// PORTABLE, always runs. Every stream here is hand-authored so the expected
/// picture can be stated exactly, and every assertion names a COORDINATE and a
/// COLOUR. That shape is the point: the obvious failure mode for a PNG writer is
/// emitting a plausible-looking blank, which "a file appeared" and "the file is
/// 40kB" both accept happily. A blank canvas fails `art_lands_where_the_placement_put_it`
/// on its first pixel, fails the glyph test for want of any foreground pixel,
/// and fails both z-order tests in opposite directions.
mod raster {
    use super::*;
    use crate::pty_stream::{oracle, raster};

    /// The screen's fill where nothing was written: `qwertty-term-vt`'s default
    /// palette entry 0, which is Ghostty's `Name::Black` — NOT pure black. Every
    /// "nothing is here" assertion below is against this, so a rasteriser that
    /// invented its own background would fail them all.
    const DEFAULT_BG: [u8; 4] = [0x1D, 0x1F, 0x21, 255];

    /// An image whose every pixel ROW is a different colour, so a placement that
    /// draws the wrong row of it is distinguishable from one that draws the right
    /// one. Row `r` is `[20 + r, 0, 0]`; the `+ 20` keeps row 0 clear of black,
    /// so "drew the first row" and "drew nothing" cannot be confused.
    fn gradient_transmit(id: u32) -> String {
        let (w, h) = (u32::from(ART_COLS) * CELL_W, u32::from(ART_ROWS) * CELL_H);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for _ in 0..w {
                rgba.extend_from_slice(&[20 + y as u8, 0, 0, 255]);
            }
        }
        format!(
            "\x1b_Gq=2,a=T,U=1,i={id},f=32,t=d,s={w},v={h},c={ART_COLS},r={ART_ROWS},z=3,m=0;{}\x1b\\",
            b64(&rgba)
        )
    }

    /// The gradient art, placed the way babelmap places art: one placeholder run
    /// per row, lead cell carrying the diacritic triple.
    fn gradient_frame() -> String {
        let mut s = gradient_transmit(ID_HIGH);
        for row in 0..ART_ROWS {
            s.push_str(&placeholder_row(row, HIGH_164));
        }
        s
    }

    fn draw(bytes: &str) -> image::RgbaImage {
        raster::render(&oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H))
    }

    fn px(canvas: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
        canvas.get_pixel(x, y).0
    }

    /// The art occupies exactly the pixels the placement resolved to, and each
    /// screen row draws its OWN row of the image.
    ///
    /// The gradient is what makes the second half real: dest and source are both
    /// 32x32 here, so screen row `ART_TOP` must show image rows 0..15 and screen
    /// row `ART_TOP + 1` image rows 16..31. A rasteriser that drew the image once
    /// into its bounding box, or one that lost `source_y` the way SQ-0772's
    /// orphaned runs do, paints the same band twice and fails on the second row.
    #[test]
    fn art_lands_where_the_placement_put_it() {
        let canvas = draw(&gradient_frame());
        assert_eq!(
            (canvas.width(), canvas.height()),
            (u32::from(COLS) * CELL_W, u32::from(ROWS) * CELL_H),
            "the canvas is the screen at its own cell size"
        );

        let (x0, y0) = (u32::from(ART_LEFT) * CELL_W, u32::from(ART_TOP) * CELL_H);
        let (x1, y1) = (x0 + u32::from(ART_COLS) * CELL_W, y0 + u32::from(ART_ROWS) * CELL_H);

        // Top-left corner of the rect, and the row of the image it must show.
        assert_eq!(px(&canvas, x0, y0), [20, 0, 0, 255], "the rect's first pixel is the image's first row");
        // One pixel down is one image row down (1:1 scale).
        assert_eq!(px(&canvas, x0, y0 + 1), [21, 0, 0, 255]);
        // The SECOND screen row of the placement — a different resolved run, with
        // its own source row. This is the assertion an aggregated rect cannot pass.
        assert_eq!(
            px(&canvas, x0, y0 + CELL_H),
            [20 + CELL_H as u8, 0, 0, 255],
            "screen row {} must draw image row {CELL_H}, not the first row again",
            ART_TOP + 1
        );
        // The rect's far corner, one pixel inside.
        assert_eq!(px(&canvas, x1 - 1, y1 - 1), [20 + (2 * CELL_H - 1) as u8, 0, 0, 255]);

        // …and nothing outside it, on all four sides.
        assert_eq!(px(&canvas, x0 - 1, y0), DEFAULT_BG, "one pixel left of the art");
        assert_eq!(px(&canvas, x1, y0), DEFAULT_BG, "one pixel right of the art");
        assert_eq!(px(&canvas, x0, y0 - 1), DEFAULT_BG, "one pixel above the art");
        assert_eq!(px(&canvas, x0, y1), DEFAULT_BG, "one pixel below the art");
    }

    /// A painted background fills its cell, and a glyph paints foreground pixels
    /// inside it without erasing it.
    ///
    /// The direction that matters: a blank-canvas bug passes nothing here. The
    /// space cell is asserted to be UNIFORMLY the painted colour, and the letter
    /// cell to hold both colours — so a rasteriser that skipped backgrounds, or
    /// one that skipped glyphs, fails a different assertion.
    #[test]
    fn a_painted_cell_is_filled_and_its_glyph_is_drawn_over_it() {
        // Row 4 (1-based row 5), from column 0: "A" then a space, on a painted bg.
        let canvas = draw("\x1b[5;1H\x1b[48;2;40;30;90m\x1b[38;2;200;10;20mA \x1b[0m");
        let y = 4 * CELL_H;
        let (bg, fg) = ([40, 30, 90, 255], [200, 10, 20, 255]);

        let letter: Vec<[u8; 4]> =
            (0..CELL_W).flat_map(|x| (0..CELL_H).map(move |dy| (x, dy))).map(|(x, dy)| px(&canvas, x, y + dy)).collect();
        assert!(letter.contains(&fg), "the 'A' painted no foreground pixel — the glyph never drew");
        assert!(letter.contains(&bg), "the 'A' covered its whole cell — the background never drew");
        assert!(
            letter.iter().all(|p| *p == fg || *p == bg),
            "a cell may only hold its own two colours"
        );

        // The space cell beside it: all background, no glyph.
        for dy in 0..CELL_H {
            for x in CELL_W..2 * CELL_W {
                assert_eq!(px(&canvas, x, y + dy), bg, "the space cell at ({x},{})", y + dy);
            }
        }
        // And a cell nothing ever wrote to keeps the screen's own fill.
        assert_eq!(px(&canvas, 0, 0), DEFAULT_BG);
    }

    /// A `z=-1` placement draws UNDER the text; a `z=1` placement draws OVER it.
    ///
    /// Both directions, because either alone passes for a rasteriser that ignores
    /// z entirely and always picks that one order. The image is pin-anchored
    /// rather than virtual so the text can be printed over it without destroying
    /// the placeholder run that positions it (the SQ-0772 failure, which is a
    /// different subject).
    fn pinned_art_under_text(z: i32) -> image::RgbaImage {
        let (w, h) = (2 * CELL_W, CELL_H);
        let rgba = [0u8, 200, 0, 255].repeat((w * h) as usize);
        draw(&format!(
            "\x1b_Gq=2,a=T,i=7,f=32,t=d,s={w},v={h},c=2,r=1,z={z},m=0;{}\x1b\\\
             \x1b[1;1H\x1b[38;2;255;255;255mW\x1b[0m",
            b64(&rgba)
        ))
    }

    #[test]
    fn a_negative_z_placement_draws_under_the_text() {
        let canvas = pinned_art_under_text(-1);
        let cell: Vec<[u8; 4]> =
            (0..CELL_W).flat_map(|x| (0..CELL_H).map(move |y| (x, y))).map(|(x, y)| px(&canvas, x, y)).collect();
        assert!(
            cell.contains(&[255, 255, 255, 255]),
            "the 'W' must be visible over a z=-1 image"
        );
        assert!(cell.contains(&[0, 200, 0, 255]), "and the image must fill the rest of the cell");
        // The cell beside it has no glyph, so it is all image.
        assert!(
            (CELL_W..2 * CELL_W).all(|x| px(&canvas, x, 0) == [0, 200, 0, 255]),
            "the un-lettered half of the placement is all image"
        );
    }

    #[test]
    fn a_positive_z_placement_draws_over_the_text() {
        let canvas = pinned_art_under_text(1);
        for y in 0..CELL_H {
            for x in 0..2 * CELL_W {
                assert_eq!(
                    px(&canvas, x, y),
                    [0, 200, 0, 255],
                    "a z=1 image covers the text under it; pixel ({x},{y}) shows through"
                );
            }
        }
    }

    /// The before/after pair the whole feature is for: two rasters, side by side,
    /// each still readable at its own coordinates.
    #[test]
    fn side_by_side_keeps_both_frames_intact() {
        let before = draw("\x1b[1;1H\x1b[48;2;10;20;30m \x1b[0m");
        let after = draw("\x1b[1;1H\x1b[48;2;90;80;70m \x1b[0m");
        let pair = raster::side_by_side(&before, &after);

        assert_eq!(pair.height(), before.height());
        assert!(pair.width() > before.width() + after.width(), "there is a gutter between them");
        assert_eq!(px(&pair, 0, 0), [10, 20, 30, 255], "the left frame is the before");
        assert_eq!(
            px(&pair, before.width() + (pair.width() - before.width() - after.width()), 0),
            [90, 80, 70, 255],
            "the right frame is the after"
        );
    }
}

#[cfg(not(unix))]
#[test]
fn the_real_capture_half_is_unix_only() {
    eprintln!(
        "SKIP: capturing a real stream needs a pty, which this platform does not have; \
         the hand-authored protocol tests still ran"
    );
}

#[cfg(unix)]
mod real_capture {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::pty_stream::{self, driver, oracle};

    /// Journey release 30, the Amiga disk image — the same fixture
    /// `pty_emitted_stream.rs` drives, so the two binaries measure one frame.
    /// NOT `journey.z6`, which is release 83 and a different build (SQ-0760).
    const STORY: &str = "Journey - The Quest Begins.adf";

    const COLS: u16 = 117;
    const ROWS: u16 = 64;

    fn out_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Both decoders on one real capture, on BOTH axes: which cells carry which
    /// SGR background, and which cells a renderer would put image pixels on.
    ///
    /// Image coverage used to be printed as a finding rather than asserted, because
    /// the two decoders legitimately disagreed: this capture left 33 runs of
    /// placeholder cells over rows 15–46, cols 47–113 that our decoder counted as
    /// the raster composite and a real terminal declined to draw at all, their
    /// anchoring cell having been overpainted by the chrome ring that replaced them
    /// (SQ-0772). With the placement now buffer-visible, the ring's frame unpaints
    /// those cells instead of stranding them, and the two readings coincide
    /// exactly — so the number is a tripwire again.
    #[test]
    fn our_decoder_and_a_real_terminal_agree_on_what_is_on_screen() {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        }
        let user_dir = out_dir().join("oracle-user-dir");
        let _ = std::fs::remove_dir_all(&user_dir);

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_babelmap"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        spec.keys = vec![
            driver::Key::Wait(Duration::from_millis(1200)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(900)),
        ];

        let cap = driver::run(spec).expect("the pty harness should boot babelmap");
        let term = pty_stream::decode_capture(&cap);
        let res = oracle::resolve(
            &cap.bytes,
            cap.spec.cols,
            cap.spec.rows,
            u32::from(cap.spec.cell_w),
            u32::from(cap.spec.cell_h),
        );

        assert!(
            term.printed_cells > 1000,
            "only {} cells were ever printed — the app never drew a frame, so neither \
             decoder measured anything",
            term.printed_cells
        );

        let all = oracle::disagreements(&term, &res);
        let (bg, img): (Vec<&String>, Vec<&String>) =
            all.iter().partition(|d| d.starts_with("background"));

        eprintln!(
            "oracle: {} placement(s) a real terminal would draw\n{}",
            res.placements.len(),
            res.describe_placements()
        );

        for (axis, runs) in [("background", &bg), ("image-coverage", &img)] {
            assert!(
                runs.is_empty(),
                "our decoder and a real terminal read {} {axis} run(s) differently on the \
                 same bytes; one of the two is wrong:\n{}",
                runs.len(),
                runs.iter().take(40).map(|s| format!("  {s}")).collect::<Vec<_>>().join("\n")
            );
        }
    }
}
