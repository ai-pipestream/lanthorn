//! `o=z` on the wire, and the app's own report of it, checked against each other
//! (SQ-0993).
//!
//! SQ-0991 shipped kitty transmission compression on a hand-run pty capture. No
//! suite pinned it, and **its failure mode is silent in both directions**: a
//! terminal that cannot inflate a transmit simply draws nothing (the image is
//! never stored, and every placement naming it disappears — measured, when `o=z`
//! first met the oracle's codec-free terminal core), and the capability quietly
//! reverting to raw looks like nothing at all. Both were live risks: SQ-0992 found
//! that a mid-session font change rebuilt the picker with an EMPTY capability list
//! and dropped compression back to raw until relaunch, and nothing noticed.
//!
//! **The two halves are asserted against EACH OTHER, and that is the real test.**
//! One case reads the wire; the other reads what `/dump-terminal` says about the
//! wire; and the third fails if they disagree. Either alone can be right about
//! nothing — a report that reads its own wish, a wire nobody can explain — and a
//! disagreement between them is a defect neither could catch.
//!
//! `/dump-terminal` is read from its LOG rather than off the screen, which is what
//! the log is for: a v6 pane is drawn out of kitty placeholder glyphs, so the
//! transcript copy of the report is exactly the thing that cannot be read back.
//!
//! Unix only; a pty is. Windows gets an explicit skip.

use super::pty_stream;

#[cfg(not(unix))]
#[test]
fn the_compression_capture_is_unix_only() {
    eprintln!("SKIP: pinning o=z on the wire needs a pty, which this platform does not have");
}

#[cfg(unix)]
mod unix {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::pty_stream::{driver, inflate};

    /// Journey release 30, the Amiga disk image — the same specimen
    /// `pty_emitted_stream` drives, and for the same reason: it is the corpus
    /// frame that reliably uploads real artwork through `ratatui-image`, which is
    /// the path SQ-0991 taught to compress. NOT `journey.z6`, which is release 83
    /// and a different build (SQ-0760).
    const STORY: &str = "Journey - The Quest Begins.adf";

    /// 117x64 cells at 8x18 px, map hidden — `pty_emitted_stream`'s geometry, so
    /// the two captures describe the same screen.
    const COLS: u16 = 117;
    const ROWS: u16 = 64;
    const CELL_W: u16 = 8;
    const CELL_H: u16 = 18;
    /// The cell the mid-run resize moves to. Same GRID, different pixels per cell
    /// — which is a font-size change and nothing else, exactly the event SQ-0992
    /// is about. Divides evenly into `COLS`/`ROWS`, so `TIOCGWINSZ` re-derives it
    /// without rounding and the report's number is unambiguous.
    const CELL_W2: u16 = 9;
    const CELL_H2: u16 = 20;

    /// Four blank lines reach Journey's party menu — the frame with the picture
    /// column, the prose beside it and the menu along the bottom. THEN the font
    /// change, then `/dump-terminal`, so the report describes a session that has
    /// already been resized and every transmit after the resize is on record.
    fn keys() -> Vec<driver::Key> {
        let cr = || driver::Key::Bytes(b"\r".to_vec());
        let wait = |ms| driver::Key::Wait(Duration::from_millis(ms));
        vec![
            wait(1200),
            cr(),
            wait(600),
            cr(),
            wait(600),
            cr(),
            wait(600),
            cr(),
            wait(900),
            driver::Key::Resize { cols: COLS, rows: ROWS, cell_w: CELL_W2, cell_h: CELL_H2 },
            wait(1200),
            // Ctrl+T, bound by the config this run writes. Journey's party menu is
            // a CHAR read, so typed text goes to the game and `/dump-terminal`
            // would be spelled into the menu — the run loop's char-mode gate lets
            // only Ctrl combos past. That is the same route `/dump-windows` and
            // `/dump-cells` are on `DEFAULT_DIRECT_COMMANDS` for.
            driver::Key::Bytes(vec![0x14]),
            wait(1500),
        ]
    }

    /// A `[keymap.global]` binding for the command under test, written into the
    /// throwaway user dir before the app seeds its own template there.
    ///
    /// Deliberately NOT a default binding added to the app: these three dumps are
    /// debugging commands, not play keys, and the tree's position is that they
    /// stay unbound until a user asks for one.
    const KEYMAP: &str = "[keymap.global]\n\"ctrl+t\" = \"dump-terminal\"\n";

    fn out_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// One capture plus the `/dump-terminal` report it wrote, or `None` when the
    /// gitignored story is absent — `stories/` is not in CI, and a real-media case
    /// must skip vacuously rather than fail.
    struct Run {
        cap: driver::Capture,
        report: String,
        report_path: PathBuf,
    }

    fn run(tag: &str) -> Option<Run> {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return None;
        }
        let user_dir = out_dir().join(format!("user-dir-{tag}"));
        let _ = std::fs::remove_dir_all(&user_dir);
        std::fs::create_dir_all(&user_dir).expect("the throwaway lanthorn home");
        std::fs::write(user_dir.join("config.toml"), KEYMAP).expect("seed the keymap");

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        spec.cell_w = CELL_W;
        spec.cell_h = CELL_H;
        spec.keys = keys();

        let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
        let report_path = user_dir.join("dump-terminal.log");
        let report = std::fs::read_to_string(&report_path).unwrap_or_default();
        Some(Run { cap, report, report_path })
    }

    /// The capture is only worth reading if it exercised the kitty path at all —
    /// half-blocks uploads nothing, and every number off such a run is worthless.
    fn require_kitty(cap: &driver::Capture) {
        let neg = cap.negotiated();
        assert!(neg.is_kitty(), "this capture measures nothing: {}", neg.explain());
    }

    /// Whether the report says the `ratatui-image` uploads are compressed. Read
    /// off the one line that names both, so a passing test cannot be satisfied by
    /// the word "COMPRESSED" appearing anywhere else in the report.
    fn report_says_compressed(report: &str) -> Option<bool> {
        let line = report.lines().find(|l| l.contains("ratatui-image uploads"))?;
        if line.contains("COMPRESSED") {
            Some(true)
        } else if line.contains("RAW") {
            Some(false)
        } else {
            None
        }
    }

    /// **The wire.** Every transmit says `o=z`, keeps `f=32`, declares the
    /// UNCOMPRESSED `s`/`v`, and carries a payload that actually inflates to
    /// exactly `s*v*4` bytes.
    ///
    /// The claim under test is not "we wrote `o=z`" — that is a substring, and a
    /// transmit that compressed each chunk separately, or that put the compressed
    /// length in `s`/`v`, or that shipped something other than a zlib stream,
    /// would contain it just the same and draw NOTHING. The terminal sizes its
    /// buffer from `s*v*4` and the inflated payload must be exactly that long, so
    /// that equality is the assertion.
    ///
    /// FALSIFY by reverting the deflate in `kitty_transmit_virtual` and
    /// `ratatui-image`'s `transmit_virtual`: the `o=z` check fails on the first
    /// transmit, and with only one of the two reverted it fails on whichever path
    /// went back to raw.
    #[test]
    fn every_kitty_transmit_is_deflated_and_declares_its_uncompressed_size() {
        let Some(r) = run("compression-wire") else { return };
        require_kitty(&r.cap);

        let transmits = inflate::transmits(&r.cap.bytes);
        assert!(
            !transmits.is_empty(),
            "the run negotiated kitty and emitted no transmit at all — nothing to measure"
        );

        let mut wire = 0usize;
        let mut raw_total = 0usize;
        for t in &transmits {
            assert!(t.compressed, "a transmit went down the wire raw: {}", t.params);
            assert_eq!(t.get("f"), Some("32"), "`o=z` is the ENCODING; `f` is still the format: {}", t.params);
            assert!(!t.params.contains("S="), "`S` is for PNG-plus-compression, and this is f=32: {}", t.params);
            let raw = t.raw.as_ref().unwrap_or_else(|| {
                panic!("a transmit says o=z and its payload is not a zlib stream — a real terminal \
                        would store no image and every placement naming it would draw nothing: {}", t.params)
            });
            let s: u32 = t.get("s").and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("no s= in {}", t.params));
            let v: u32 = t.get("v").and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("no v= in {}", t.params));
            assert_eq!(
                raw.len() as u64,
                u64::from(s) * u64::from(v) * 4,
                "s={s}, v={v} must name the UNCOMPRESSED image: the terminal sizes its buffer from \
                 s*v*4 and inflating this payload gave {} bytes ({})",
                raw.len(),
                t.params
            );
            wire += t.wire_b64;
            raw_total += raw.len().div_ceil(3) * 4;
        }

        // Not a threshold to tune — sixteen-colour flat artwork is what deflate is
        // best at, and SQ-0991 measured 50.7x on this very capture. 4x is the
        // floor below which something has stopped compressing in a way the key
        // check above could not see.
        assert!(
            raw_total / wire.max(1) >= 4,
            "{} transmit(s) went out in {wire} base64 bytes against {raw_total} raw — a ratio of \
             {:.1}x, which is not what deflate does to this artwork",
            transmits.len(),
            raw_total as f64 / wire.max(1) as f64
        );
        eprintln!(
            "wire: {} transmit(s), {wire} b64 bytes compressed against {raw_total} raw ({:.1}x)",
            transmits.len(),
            raw_total as f64 / wire.max(1) as f64
        );
    }

    /// **SQ-0992's property, now observable.** A font-size change re-derives the
    /// cell from `TIOCGWINSZ` and must not disturb the capability list behind it —
    /// the picker is mutated in place precisely so the `o=z` answer survives.
    ///
    /// This is what the harness could not see before `Key::Resize`: the winsize
    /// was set once at `open_pty`, so "compression survives a font change" could
    /// only be argued from the source. `Capture::resizes` records where the
    /// resize split the stream, so the transmits AFTER it can be named — without
    /// which "all of them are compressed" would pass on a run whose tail was
    /// empty.
    ///
    /// FALSIFY by putting `refresh_cell_size` back on the deprecated
    /// `Picker::from_fontsize` rebuild it used before SQ-0992: that constructor
    /// makes `capabilities: Vec::new()`, so every transmit after the resize goes
    /// out raw and this fails while the whole-capture case above still passes on
    /// the transmits that preceded it.
    #[test]
    fn compression_survives_a_font_size_change() {
        let Some(r) = run("compression-resize") else { return };
        require_kitty(&r.cap);

        let resize = *r.cap.resizes.first().unwrap_or_else(|| {
            panic!("the run scripted a resize and the capture recorded none")
        });
        assert_eq!(
            (resize.cell_w, resize.cell_h),
            (CELL_W2, CELL_H2),
            "the resize must be a FONT-SIZE change, not a grid change"
        );

        let after: Vec<_> = inflate::transmits(&r.cap.bytes).into_iter().filter(|t| t.at >= resize.offset).collect();
        assert!(
            !after.is_empty(),
            "the app emitted no image at all after the font change, so this run proves nothing \
             about what happens to compression across one (resize at byte {} of {})",
            resize.offset,
            r.cap.bytes.len()
        );
        for t in &after {
            assert!(
                t.compressed,
                "a transmit after the font change went out RAW — the capability list did not \
                 survive the cell-size refresh, which is SQ-0992 exactly: {}",
                t.params
            );
        }
        eprintln!("{} transmit(s) after the font change, all still o=z", after.len());
    }

    /// **The report.** `/dump-terminal` says compression is on, names the protocol
    /// it detected, and — the distinction the whole command exists for — reports
    /// the post-resize cell as DERIVED from the ioctl rather than as the `CSI 16 t`
    /// measurement it no longer is.
    ///
    /// FALSIFY by deleting the `append_terminal_dump` call in the `DumpTerminal`
    /// arm: the log never appears and every assertion here fails at the read.
    #[test]
    fn dump_terminal_reports_the_session_the_capture_measured() {
        let Some(r) = run("compression-report") else { return };
        require_kitty(&r.cap);

        assert!(
            !r.report.is_empty(),
            "no /dump-terminal report at {} — either the command did not reach the app or it \
             wrote no log",
            r.report_path.display()
        );
        assert!(
            r.report.contains("graphics protocol: kitty (auto-detected)"),
            "the report must describe the same kitty session the capture measured:\n{}",
            r.report
        );
        assert_eq!(
            report_says_compressed(&r.report),
            Some(true),
            "the terminal answered the o=z probe, so the report must say the uploads are \
             compressed:\n{}",
            r.report
        );
        // The cell in force after a font change is the IOCTL's, not the stale
        // `CSI 16 t` answer — and calling that "measured" would be exactly the
        // conflation SQ-0994 exists to end.
        assert!(
            r.report.contains(&format!("cell size: {CELL_W2}x{CELL_H2} px — DERIVED")),
            "after the font change the cell is re-derived from TIOCGWINSZ, and the report must \
             say which of the two it is:\n{}",
            r.report
        );
        assert!(
            r.report.contains(&format!("CSI 16 t answered: {CELL_W}x{CELL_H} px")),
            "…while still showing what the terminal originally measured:\n{}",
            r.report
        );
        eprintln!("/dump-terminal report at {}", r.report_path.display());
    }

    /// **Their agreement, which is the assertion neither half can make alone.**
    ///
    /// A report that claims compression while the wire is raw is a lie a user
    /// would act on; a wire that compresses while the report says raw sends the
    /// next investigator after the wrong thing. Both are defects, and both are
    /// invisible to a test that reads only one side.
    ///
    /// Deliberately an EQUALITY rather than "both true": on a build or a terminal
    /// where compression is genuinely off, the pair must still agree, and a test
    /// that only ever demanded `true` would have nothing to say about that. The
    /// "and it is on here" half is asserted separately, because this harness
    /// answers the `o=z` probe and so knows which answer to expect.
    #[test]
    fn the_wire_and_the_report_agree_about_compression() {
        let Some(r) = run("compression-agree") else { return };
        require_kitty(&r.cap);

        let transmits = inflate::transmits(&r.cap.bytes);
        assert!(!transmits.is_empty(), "no transmit, so neither half has anything to be right about");
        let wire = transmits.iter().all(|t| t.compressed);
        let reported = report_says_compressed(&r.report).unwrap_or_else(|| {
            panic!("the report says nothing about ratatui-image uploads:\n{}", r.report)
        });

        assert_eq!(
            wire, reported,
            "the wire and /dump-terminal disagree: {} transmit(s), {} of them compressed, while \
             the report says compression is {}. One of the two is lying, and neither half of this \
             suite could tell you which on its own.\nreport at {}",
            transmits.len(),
            transmits.iter().filter(|t| t.compressed).count(),
            if reported { "ON" } else { "OFF" },
            r.report_path.display()
        );
        assert!(
            wire,
            "this harness answers the kitty o=z probe, so compression must be ON — the two halves \
             agreeing that it is off means the capability was dropped somewhere between the probe \
             and the encoder"
        );
    }
}
