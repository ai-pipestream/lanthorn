//! What babelmap actually WRITES to the terminal, not what it computed (SQ-0762).
//!
//! Every other harness here renders into a `Buffer` and asserts on cells. That
//! cannot see a defect that lives between the model and the wire, and this
//! session produced several that were "not in the model" and plainly visible on
//! screen. This one boots the real binary under a pty, answers the terminal
//! queries a kitty terminal answers, feeds it keys, and decodes the bytes that
//! come back.
//!
//! WHAT IT ASSERTS, AND WHAT IT DELIBERATELY DOES NOT. It asserts that the
//! harness is measuring the right thing: that kitty negotiated (a run that fell
//! back to half-blocks exercises the wrong backend and every number it produces
//! is worthless), that a real upload happened, and that the placement rect can be
//! read back off the stream. It does NOT pin any particular defect's presence:
//! a test that fails when a bug is FIXED is a trap for the next person. The
//! defect reading — image extent versus painted fill — is printed as a finding
//! and written to the report file instead.
//!
//! Unix only; a pty is. Windows gets an explicit skip, below.

// Not gated: the decoder half is portable and its unit tests are worth having on
// every platform. Only the pty half inside it is `#[cfg(unix)]`.
//
// The harness module is declared ONCE by this suite's group binary (`tests/pty.rs`)
// and shared by every pty suite in it; declaring it here as well would compile a
// second copy of ~2900 lines into the same binary.
use super::pty_stream;

#[cfg(not(unix))]
#[test]
fn the_pty_capture_is_unix_only() {
    eprintln!("SKIP: capturing the stream needs a pty, which this platform does not have; \
               the decoder's own tests still ran");
}

#[cfg(unix)]
mod unix {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::pty_stream::{self, driver};

    /// Journey release 30, the Amiga disk image. NOT `journey.z6` — that is
    /// release 83 and a genuinely different build, and a finding measured on one
    /// does not transfer to the other (SQ-0760).
    const STORY: &str = "Journey - The Quest Begins.adf";

    /// 117x64 terminal, map pane hidden: the frame border takes a column each
    /// side and the help row takes the bottom, which leaves the story pane the
    /// 115x61 the defect was measured at.
    const COLS: u16 = 117;
    const ROWS: u16 = 64;

    fn out_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn the_emitted_stream_shows_which_rows_an_image_covers_and_which_are_painted() {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        }
        let user_dir = out_dir().join("user-dir");
        let _ = std::fs::remove_dir_all(&user_dir);

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_babelmap"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        // Journey's intro wants a few keypresses before the party menu — the frame
        // with the picture column, the prose beside it and the menu along the
        // bottom, which is the frame the defect lives on.
        spec.keys = vec![
            driver::Key::Wait(Duration::from_millis(1200)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(900)),
        ];

        let cap = driver::run(spec).expect("the pty harness should boot babelmap");
        let term = pty_stream::decode_capture(&cap);
        let report = pty_stream::report(&cap, &term);
        let path = out_dir().join("journey-r30-115x61.txt");
        let _ = std::fs::write(&path, &report);

        let neg = cap.negotiated();
        assert!(
            neg.is_kitty(),
            "the capture must exercise the kitty path or it measures nothing: {}\n\
             (full report at {})",
            neg.explain(),
            path.display()
        );
        assert!(
            !term.apc.is_empty(),
            "a kitty run uploads at least one image; none was decoded (report at {})",
            path.display()
        );
        let placements = term.placements();
        assert!(
            !placements.is_empty(),
            "the stream carried APC uploads but no U+10EEEE placeholder cells, so nothing was \
             actually placed on screen (report at {})",
            path.display()
        );
        assert!(
            term.printed_cells > 1000,
            "only {} cells were ever printed — the app did not get as far as drawing a frame \
             (report at {})",
            term.printed_cells,
            path.display()
        );

        // The finding, not an assertion: which rows are image and which are paint.
        eprintln!("emitted-stream report: {}", path.display());
        eprint!("{}", pty_stream::overrun_finding(&term));
    }
}
