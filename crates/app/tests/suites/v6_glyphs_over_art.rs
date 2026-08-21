//! SQ-0944 — text the game printed ON its artwork is drawn as real GLYPHS on the
//! half-block backend, in a background sampled from the picture behind it.
//!
//! ## Why this is a defect and not a taste
//!
//! A half-block cell is `▀` with a foreground and a background: TWO vertical
//! samples. So a rasterised 8×16 v6 glyph arrives as 8×2 and is not merely ugly,
//! it is unreadable. Every other route to the same text — a `ChromeStrip::Text`
//! row, the story transcript — has always been drawn with real glyphs; only text
//! sitting on artwork was rasterised, because `ChromeStrip::Art` contributed no
//! glyph rows to `TextLayer::SkipGlyphRows`.
//!
//! That matters beyond the odd banner: half-blocks is what a terminal with no
//! graphics protocol falls back to, what tmux gets, and what an asciinema cast
//! records (a cast replays because half-blocks are glyphs plus 24-bit SGR). It is
//! the difference between a cast that shows a legible game and one that shows
//! coloured mush with a picture in it.
//!
//! ## Why ONLY half-blocks — the measurement that shrank this quest
//!
//! The quest also proposed doing this on kitty, on the reading that every
//! placement sits at `z = -1` ("over the backgrounds but under the text") so a
//! glyph would appear on top for free. It does not, and the reason is not about z
//! at all: lanthorn's placements are VIRTUAL (`U=1`), positioned by `U+10EEEE`
//! placeholder characters, so the image IS the cell's content. Printing a glyph
//! into a covered cell deletes the image rather than compositing over it, and
//! takes the rest of the row's run with it. `pty_oracle.rs` pins that directly;
//! `screen::backend_layers_glyphs_over_art` carries the full finding.
//!
//! So the capability is present on exactly one backend, where it is not optional,
//! and absent on the other three. There is no config key: nothing is left to
//! choose. This suite pins BOTH halves of that — the half-block gain, and that
//! kitty is byte-for-byte unmoved.
//!
//! ## The specimen
//!
//! Zork Zero, `zork0-r393-s890714.z6`, release 393 / serial 890714, booted as
//! `IbmPc` at native 640×400 (`art scale (2,2)`), driven **6 keypresses** in
//! (`look` at each Line prompt) to reach a frame whose banner carries a live room
//! name. That frame is the canonical text-over-art case in the corpus: all 29
//! chrome runs sit on the banner ribbon, every one of them `fg = Standard(2)`
//! black with NO explicit background and no reverse bit — the "dark ink straight
//! on the picture" branch of `chrome_run_ink`. `ring_scout --runs --bands` reports
//! its top band as `strip:art 70x6 at (14,0)` carrying "Banquet Hall",
//! "Flatheadia", "Moves: 0" and "Score: 0".
//!
//! Both `honor_game_colours` modes are pinned; the story asset is gitignored, so
//! every case skips cleanly when it is absent and `the_smokes_were_not_vacuous`
//! fails if the fixture is there and nothing measured.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const STORY: &str = "zork0-r393-s890714.z6";

/// The banner labels the game paints onto its ribbon at this turn count.
const BANNER_WORDS: [&str; 4] = ["Banquet Hall", "Flatheadia", "Moves:", "Score:"];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn fixture_present() -> bool {
    stories_dir().join(STORY).exists()
}

/// Boot Zork Zero and play in far enough that the banner carries a live room name
/// — SIX keypresses, the same frame `v6_zork0_icon_backdrop` measures.
fn zork0_in_play(honor_game_colours: bool) -> Option<GameSession> {
    let story_path = stories_dir().join(STORY);
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes,
        honor_game_colours,
        false,
        None,
        false,
        picture_dims,
        picts.std_window(),
        None,
        None,
    )
    .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "Zork0 booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..6 {
        match session.pending_input() {
            app::session::InputKind::Line => {
                let _ = session.submit("look");
            }
            app::session::InputKind::Char => {
                let _ = session.submit_char(b' ');
            }
            app::session::InputKind::Event => {
                let _ = session.submit("");
            }
        }
        let _ = session.take_transcript();
    }
    Some(session)
}

/// A render state on a named backend. `None` = half-blocks; `Some(p)` overrides
/// the protocol on the same picker, which is the only way to name a backend
/// without a live terminal to query (`from_fontsize` is deprecated).
fn render_state(
    honor_game_colours: bool,
    protocol: Option<ratatui_image::picker::ProtocolType>,
) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some({
        let mut picker = ratatui_image::picker::Picker::halfblocks();
        if let Some(p) = protocol {
            picker.set_protocol_type(p);
        }
        picker
    });
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor_game_colours;
    state.push_transcript("Banquet Hall");
    state.push_transcript("The hall is filled to capacity.");
    state
}

/// Render the frame and hand back the buffer plus the story viewport, so a caller
/// can confine itself to the RING — the rows above and below the transcript.
fn frame(
    honor_game_colours: bool,
    protocol: Option<ratatui_image::picker::ProtocolType>,
) -> Option<(Buffer, Rect)> {
    let session = zork0_in_play(honor_game_colours)?;
    let model = session.screen();
    let state = render_state(honor_game_colours, protocol);
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    assert_eq!(state.v6_ring_plan.get(), "frame", "Zork0's enclosed frame takes the hybrid ring path");
    let viewport = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    Some((buf, viewport))
}

/// The text of the ring rows ABOVE the story viewport, one string per row. The
/// banner is up there; the transcript below is prose and proves nothing here.
fn ring_rows_above(buf: &Buffer, viewport: Rect) -> Vec<String> {
    (0..viewport.y)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')))
                .collect()
        })
        .collect()
}

#[test]
fn halfblocks_draw_the_banner_labels_as_real_glyphs() {
    for honor in [true, false] {
        let Some((buf, viewport)) = frame(honor, None) else { return };
        let rows = ring_rows_above(&buf, viewport);
        let banner = rows.join("\n");

        // Non-vacuity guard on the FRAME's shape (CLAUDE.md): if the ring above the
        // story is not several rows deep, this is not the frame the suite is about
        // and every assertion below would be measuring something else.
        assert!(
            viewport.y >= 4,
            "honor={honor}: the banner ring should be several rows deep, got {} (wrong frame?)",
            viewport.y
        );

        for word in BANNER_WORDS {
            assert!(
                banner.contains(word),
                "honor={honor}: the banner label {word:?} must reach the screen as CHARACTERS, \
                 not as an 8x2 smear of half-blocks. Ring rows:\n{banner}"
            );
        }
    }
}

#[test]
fn a_banner_glyph_sits_in_the_picture_not_in_a_box() {
    // The point of sampling: the cell a glyph lands in must keep a background that
    // belongs to the ART around it, not the theme's backdrop. Measured against the
    // glyph's own NEIGHBOURING art cells rather than against a hardcoded colour,
    // because the banner's ribbon colour is the game's to choose.
    for honor in [true, false] {
        let Some((buf, viewport)) = frame(honor, None) else { return };
        let rows = ring_rows_above(&buf, viewport);

        // Find a row carrying a banner word, and the COLUMN its first letter is in
        // — by character, never by `str::find`, whose byte index is not a column
        // once the row holds multi-byte half-block characters.
        let hit = rows.iter().enumerate().find_map(|(y, r)| {
            let chars: Vec<char> = r.chars().collect();
            chars
                .windows("Banquet".len())
                .position(|w| w.iter().collect::<String>() == "Banquet")
                .map(|x| (y as u16, x as u16))
        });
        let Some((row, col)) = hit else {
            panic!("honor={honor}: no ring row carries the banner label; rows:\n{}", rows.join("\n"))
        };

        let cell = buf.cell((col, row)).expect("the glyph's cell is inside the pane");
        assert_eq!(cell.symbol(), "B", "the located cell is the label's first letter");

        // The nearest cell on this row that is still ART — a half-block `▀` — gives
        // the ground the picture actually has there.
        let art_bg = (0..buf.area.width)
            .filter(|&x| buf.cell((x, row)).is_some_and(|c| c.symbol() == "▀"))
            .min_by_key(|&x| x.abs_diff(col))
            .and_then(|x| buf.cell((x, row)))
            .map(|c| (c.fg, c.bg));
        let Some((art_fg, art_lower)) = art_bg else {
            panic!("honor={honor}: the banner row carries no art cell to compare against")
        };

        let rgb = |c: Color| match c {
            Color::Rgb(r, g, b) => Some([i32::from(r), i32::from(g), i32::from(b)]),
            _ => None,
        };
        let (Some(glyph), Some(a), Some(b)) = (rgb(cell.bg), rgb(art_fg), rgb(art_lower)) else {
            panic!("honor={honor}: the ring should be painting true colour on both counts")
        };
        // The picture's own two samples bracket the ground the glyph was given —
        // it is the mean of a cell of the same ribbon, so it cannot be far outside
        // the range those samples span.
        let dist = |p: [i32; 3], q: [i32; 3]| (0..3).map(|i| (p[i] - q[i]).abs()).max().unwrap_or(0);
        assert!(
            dist(glyph, a).min(dist(glyph, b)) <= 24,
            "honor={honor}: the glyph's background {glyph:?} must be the art behind it \
             (neighbouring art cell samples {a:?}/{b:?}), not a box of some other colour"
        );
    }
}

#[test]
fn kitty_sixel_and_iterm2_are_left_exactly_as_they_were() {
    // The capability is absent on all three — on kitty because a virtual placement
    // is positioned by its placeholder CELLS, so a glyph in one deletes the image
    // rather than layering over it (measured; see `pty_oracle.rs`). So none of them
    // may grow a banner glyph, and the art must still be reaching those rows.
    use ratatui_image::picker::ProtocolType;
    for protocol in [ProtocolType::Kitty, ProtocolType::Sixel, ProtocolType::Iterm2] {
        for honor in [true, false] {
            let Some((buf, viewport)) = frame(honor, Some(protocol)) else { return };
            let banner = ring_rows_above(&buf, viewport).join("\n");
            for word in BANNER_WORDS {
                assert!(
                    !banner.contains(word),
                    "{protocol:?} honor={honor}: this backend cannot show a glyph in a cell its \
                     image covers, so {word:?} must stay in the raster. Ring rows:\n{banner}"
                );
            }
        }
    }
}

#[test]
fn no_rasterised_ghost_of_the_label_survives_past_its_glyphs() {
    // The other half of the change, and the half a "the letters are there" test
    // cannot see: the rows these runs sit on are withheld from the chrome canvas
    // (`TextLayer::SkipGlyphRows`) so the band ships the picture WITHOUT a
    // rasterised copy of the text baked into it.
    //
    // It shows because the two renderings are not the same width. Terminal text is
    // one column per character, but a native 8-px character is 1.5 columns at this
    // frame's scale — so "Banquet Hall" occupies 12 glyph columns and its
    // rasterised twin sprawls across 18. The 6 columns past the glyphs are where
    // the ghost lands, and they are ribbon or they are not.
    //
    // Measured at this frame, as the worst channel distance from the ribbon over
    // those six columns: 24 with the rows withheld, 56 with them rasterised. The
    // tolerance sits between, and both numbers are here so a later reader can see
    // how much margin there is.
    for honor in [true, false] {
        let Some((buf, viewport)) = frame(honor, None) else { return };
        let rows = ring_rows_above(&buf, viewport);
        let hit = rows.iter().enumerate().find_map(|(y, r)| {
            let chars: Vec<char> = r.chars().collect();
            chars
                .windows("Banquet Hall".len())
                .position(|w| w.iter().collect::<String>() == "Banquet Hall")
                .map(|x| (y as u16, x as u16))
        });
        let Some((row, col)) = hit else { panic!("honor={honor}: the banner label is not on the screen") };

        let rgb = |c: Color| match c {
            Color::Rgb(r, g, b) => Some([i32::from(r), i32::from(g), i32::from(b)]),
            _ => None,
        };
        let tail = col + "Banquet Hall".len() as u16;
        // The ribbon, measured LOCALLY: the median background of the twelve columns
        // beyond the ghost's reach. Local because the banner is ornate and its
        // brightest pixel is somewhere else entirely on this row; median because one
        // stray cell should not become the reference. Stable across the change — the
        // ribbon out there is untouched in both renderings, which is exactly what a
        // reference has to be if it is to measure anything.
        let mut ribbon = [0i32; 3];
        for ch in 0..3 {
            let mut v: Vec<i32> = (tail + 6..tail + 18)
                .filter_map(|x| buf.cell((x, row)).and_then(|c| rgb(c.bg)))
                .map(|p| p[ch])
                .collect();
            assert!(!v.is_empty(), "honor={honor}: no ribbon beyond the label to measure against");
            v.sort_unstable();
            ribbon[ch] = v[v.len() / 2];
        }

        for x in tail..tail + 6 {
            let Some(bg) = buf.cell((x, row)).and_then(|c| rgb(c.bg)) else { continue };
            let d = (0..3).map(|i| (bg[i] - ribbon[i]).abs()).max().unwrap_or(0);
            assert!(
                d <= 40,
                "honor={honor}: column {x} is past the label's glyphs and must be clean ribbon \
                 {ribbon:?}, but reads {bg:?} ({d} off) — a rasterised ghost of the label is \
                 still baked into the band"
            );
        }
    }
}

/// CI has no `stories/`, so every case above returns early there and this file
/// would pass without measuring anything. Count one real decision and say so.
#[test]
fn the_smokes_were_not_vacuous() {
    let mut seen = 0;
    if let Some((buf, viewport)) = frame(true, None) {
        assert!(ring_rows_above(&buf, viewport).join("\n").contains("Banquet Hall"));
        seen += 1;
    }
    assert!(
        !fixture_present() || seen > 0,
        "the fixture is present but nothing rendered — this suite was vacuous"
    );
}
