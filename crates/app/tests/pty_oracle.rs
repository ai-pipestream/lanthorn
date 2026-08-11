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
//!   * `real_capture` — unix only, drives a real story through the pty, and
//!     asserts the two decoders agree on BACKGROUNDS. It deliberately does not
//!     assert they agree on image coverage: they legitimately disagree there
//!     today (a placement our decoder reports and a real terminal declines to
//!     draw is the finding, not the failure), which is filed as SQ-0772.
//!
//! Slow-test gating follows `pty_emitted_stream.rs`: nothing here is `#[ignore]`
//! (SQ-0368 reserved that for the multi-second full-game walkthroughs), and the
//! gitignored fixture makes the capture half skip vacuously rather than fail.

mod pty_stream;

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

    /// Both decoders on one real capture. The assertion is BACKGROUNDS ONLY:
    /// which cells carry which SGR background is a question both models answer
    /// from the same evidence, so a difference there is a bug in one of them.
    /// Image coverage is not asserted — the two legitimately disagree today (our
    /// decoder counts placeholder cells; the oracle counts pixels that land),
    /// and that disagreement is SQ-0772, not this test's business. It is printed
    /// as a finding so the number is visible without being a tripwire.
    #[test]
    fn our_decoder_and_a_real_terminal_agree_on_backgrounds() {
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

        // The finding, not an assertion (SQ-0772).
        eprintln!(
            "oracle: {} placement(s) a real terminal would draw\n{}",
            res.placements.len(),
            res.describe_placements()
        );
        eprintln!("oracle: {} image-coverage disagreement run(s) (SQ-0772, not asserted)", img.len());
        for d in img.iter().take(20) {
            eprintln!("  {d}");
        }

        assert!(
            bg.is_empty(),
            "our decoder and a real terminal read {} background run(s) differently on the \
             same bytes; one of the two is wrong:\n{}",
            bg.len(),
            bg.iter().take(40).map(|s| format!("  {s}")).collect::<Vec<_>>().join("\n")
        );
    }
}
