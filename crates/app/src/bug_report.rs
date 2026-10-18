//! `/report-bug`: everything a bug report needs, as one pure function of facts.
//!
//! The handler (`slash_dispatch.rs`) gathers a [`BugReportFacts`] — the terminal,
//! the settings, the game, the map — and [`build`] turns it into the two texts the
//! command hands out: a short [`BugReport::summary`] (shown in the transcript and
//! copied to the clipboard) and the [`BugReport::full`] file body. Every formatting
//! and privacy rule lives HERE, so it can be asserted with no terminal and no disk.
//!
//! Privacy, enforced in [`build`] and nowhere else:
//! * a game is named by its file NAME ([`GameFacts::file`]); the builder never sees
//!   its path;
//! * SSH and tmux are reported as yes/no — the facts carry the raw environment
//!   values so that this module, not each caller, is what refuses to print them;
//! * the only environment variables that appear are `TERM`, `TERM_PROGRAM` and
//!   `TERM_PROGRAM_VERSION`;
//! * the home directory is replaced by `~` everywhere in both texts
//!   ([`BugReportFacts::home`]).

use crate::terminal_dump::{CellSource, Probe, TerminalSnapshot};

/// The graphics half of the summary, read off the same snapshot `/dump-terminal` uses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphicsFacts {
    /// The protocol in force, or `None` when images are off / no picker was built.
    pub protocol: Option<String>,
    /// `true` when `--image-protocol` named it rather than detection.
    pub forced: bool,
    pub images_off: bool,
    pub cell: Option<(u16, u16)>,
    /// A short phrase for where the cell size came from.
    pub cell_source: &'static str,
    /// The story pane in terminal cells.
    pub pane: (u16, u16),
}

impl GraphicsFacts {
    pub fn from_snapshot(s: &TerminalSnapshot) -> Self {
        GraphicsFacts {
            protocol: s.protocol.clone(),
            forced: s.forced_protocol.is_some(),
            images_off: s.probe == Probe::NotAskedImagesOff,
            cell: s.cell,
            cell_source: match s.cell_source {
                CellSource::Measured => "measured",
                CellSource::Derived => "derived from the tty",
                CellSource::Assumed => "assumed default",
                CellSource::Unexplained => "unexplained",
                CellSource::None => "unknown",
            },
            pane: s.pane_cells,
        }
    }
}

/// The settings that change what a player sees.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsFacts {
    pub images: bool,
    pub v6_render: String,
    pub pixel_lock: bool,
    pub game_colours: bool,
    pub style: Option<String>,
    pub sound: bool,
    /// An audio device is actually open.
    pub audio_device: bool,
}

/// The running game. Every optional fact is omitted from the report when absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GameFacts {
    pub title: String,
    /// The story file's NAME, never its path.
    pub file: String,
    /// `z-machine`, `glulx` or `scott`, as `mapgen::StoryIdent::engine` spells it.
    pub engine: String,
    /// The Z-machine version byte.
    pub format_version: Option<u8>,
    pub release: Option<u16>,
    pub serial: Option<String>,
    pub checksum: Option<String>,
    pub ifid: Option<String>,
    pub ifdb_tuid: Option<String>,
    /// Which story on a multi-story disk image.
    pub disk_entry: Option<String>,
    /// The machine lanthorn presents itself as, e.g. `Amiga`.
    pub machine: Option<String>,
    /// Where that machine came from: `from the disk`, `asked for`, `default`.
    pub machine_source: Option<&'static str>,
}

/// The automap, summarised.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapFacts {
    pub rooms: usize,
    pub connections: usize,
    pub layers: usize,
    pub current_room: Option<String>,
    /// Map pane size in cells; `None` when the pane is hidden.
    pub pane: Option<(u16, u16)>,
    pub zoom: String,
    pub shown: bool,
}

/// Everything [`build`] reads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BugReportFacts {
    /// `buildinfo::LONG`.
    pub version: String,
    pub os: String,
    pub arch: String,
    pub term_program: Option<String>,
    pub term_program_version: Option<String>,
    pub term: Option<String>,
    /// The raw `TMUX` value; only its presence is reported.
    pub tmux: Option<String>,
    /// The raw `SSH_CONNECTION` / `SSH_TTY` value; only its presence is reported.
    pub ssh: Option<String>,
    pub graphics: GraphicsFacts,
    pub settings: SettingsFacts,
    /// `None` with no game loaded.
    pub game: Option<GameFacts>,
    pub map: Option<MapFacts>,
    /// The logs folder, already `~`-abbreviated.
    pub logs_dir: String,
    /// The home directory, scrubbed to `~` wherever it appears in either text.
    pub home: Option<String>,
    pub timestamp: String,
    /// `/dump-terminal`'s plain text.
    pub terminal_dump: Vec<String>,
    /// `/dump-windows`'s lines; `None` with no game running.
    pub window_dump: Option<Vec<String>>,
    /// The `lanthorn-map` JSON (the one `/export-json` writes).
    pub map_json: Option<String>,
    /// The commands typed this session.
    pub commands: Vec<String>,
}

/// The two texts `/report-bug` hands out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BugReport {
    /// Short; shown in the transcript and copied to the clipboard.
    pub summary: String,
    /// The file body: a header, the summary, then the full sections.
    pub full: String,
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn yes_no(present: bool) -> &'static str {
    if present { "yes" } else { "no" }
}

/// Align `label:` to the summary's value column (10 wide).
fn labelled(label: &str, value: &str) -> String {
    format!("{:<10}{value}", format!("{label}:"))
}

const CONTINUATION: &str = "          ";

fn engine_label(g: &GameFacts) -> String {
    match g.engine.as_str() {
        "z-machine" => match g.format_version {
            Some(v) => format!("Z-machine v{v}"),
            None => "Z-machine".to_string(),
        },
        "glulx" => "Glulx".to_string(),
        "scott" => "Scott Adams".to_string(),
        other => other.to_string(),
    }
}

fn terminal_line(f: &BugReportFacts) -> String {
    let mut program = match (&f.term_program, &f.term_program_version) {
        (Some(p), Some(v)) => format!("{p} {v}"),
        (Some(p), None) => p.clone(),
        (None, _) => "unknown terminal".to_string(),
    };
    if let Some(t) = &f.term {
        program.push_str(&format!(" (TERM={t})"));
    }
    labelled(
        "terminal",
        &format!("{program} · tmux: {} · ssh: {}", yes_no(f.tmux.is_some()), yes_no(f.ssh.is_some())),
    )
}

fn graphics_line(g: &GraphicsFacts) -> String {
    let proto = match (&g.protocol, g.images_off) {
        (_, true) => "none (images off)".to_string(),
        (None, false) => "none".to_string(),
        (Some(p), false) if g.forced => format!("{p} (forced)"),
        (Some(p), false) => format!("{p} (auto-detected)"),
    };
    let cell = match g.cell {
        Some((w, h)) => format!("cell {w}x{h} {}", g.cell_source),
        None => "cell unknown".to_string(),
    };
    labelled("graphics", &format!("{proto} · {cell} · pane {}x{}", g.pane.0, g.pane.1))
}

fn settings_line(s: &SettingsFacts) -> String {
    let mut parts = vec![
        format!("images {}", on_off(s.images)),
        format!("v6 {}", s.v6_render),
        format!("pixel lock {}", on_off(s.pixel_lock)),
        format!("game colours {}", on_off(s.game_colours)),
    ];
    if let Some(style) = &s.style {
        parts.push(format!("style \"{style}\""));
    }
    parts.push(match (s.sound, s.audio_device) {
        (true, true) => "sound on".to_string(),
        (true, false) => "sound on (no device)".to_string(),
        (false, _) => "sound off".to_string(),
    });
    labelled("settings", &parts.join(" · "))
}

fn game_lines(g: &GameFacts) -> Vec<String> {
    let mut head: Vec<String> = Vec::new();
    if !g.title.is_empty() {
        head.push(g.title.clone());
    }
    if !g.file.is_empty() {
        head.push(g.file.clone());
    }
    let mut engine = engine_label(g);
    if let Some(r) = g.release {
        engine.push_str(&format!(" r{r}"));
    }
    if let Some(s) = g.serial.as_ref().filter(|s| !s.is_empty()) {
        engine.push_str(&format!(" s{s}"));
    }
    if let Some(c) = &g.checksum {
        engine.push_str(&format!(" {c}"));
    }
    head.push(engine);

    let mut second: Vec<String> = Vec::new();
    if let Some(i) = g.ifid.as_ref().filter(|s| !s.is_empty()) {
        second.push(format!("IFID {i}"));
    }
    if let Some(t) = g.ifdb_tuid.as_ref().filter(|s| !s.is_empty()) {
        second.push(format!("IFDB {t}"));
    }
    if let Some(d) = &g.disk_entry {
        second.push(format!("disk entry {d}"));
    }
    if let Some(m) = &g.machine {
        second.push(match g.machine_source {
            Some(src) => format!("machine {m} ({src})"),
            None => format!("machine {m}"),
        });
    }

    let mut out = vec![labelled("game", &head.join(" · "))];
    if !second.is_empty() {
        out.push(format!("{CONTINUATION}{}", second.join(" · ")));
    }
    out
}

fn map_lines(m: &MapFacts) -> Vec<String> {
    let n = |count: usize, noun: &str| format!("{count} {noun}{}", if count == 1 { "" } else { "s" });
    let mut first = format!("{} · {} · {}", n(m.rooms, "room"), n(m.connections, "connection"), n(m.layers, "layer"));
    if let Some(r) = &m.current_room {
        first.push_str(&format!(" · here: \"{r}\""));
    }
    let mut second: Vec<String> = Vec::new();
    if let Some((w, h)) = m.pane {
        second.push(format!("pane {w}x{h}"));
    }
    second.push(format!("zoom {}", m.zoom));
    second.push(if m.shown { "map shown" } else { "map hidden" }.to_string());
    vec![labelled("map", &first), format!("{CONTINUATION}{}", second.join(" · "))]
}

/// Build the summary and the full report from `f`.
pub fn build(f: &BugReportFacts) -> BugReport {
    let mut lines = vec![
        format!("lanthorn {}", f.version),
        labelled("system", &format!("{} {}", f.os, f.arch)),
        terminal_line(f),
        graphics_line(&f.graphics),
        settings_line(&f.settings),
    ];
    if let Some(g) = &f.game {
        lines.extend(game_lines(g));
    }
    if let Some(m) = &f.map {
        lines.extend(map_lines(m));
    }
    lines.push(labelled("logs", &f.logs_dir));
    let summary = lines.join("\n");

    let mut full = format!("lanthorn bug report, {}\n\n{summary}\n", f.timestamp);
    full.push_str("\n== /dump-terminal ==\n");
    for l in &f.terminal_dump {
        full.push_str(l);
        full.push('\n');
    }
    if let Some(w) = &f.window_dump {
        full.push_str("\n== window layout (/dump-windows) ==\n");
        for l in w {
            full.push_str(l);
            full.push('\n');
        }
    }
    if let Some(j) = &f.map_json {
        full.push_str("\n== map (lanthorn-map JSON) ==\n");
        full.push_str(j);
        if !j.ends_with('\n') {
            full.push('\n');
        }
    }
    full.push_str("\n== commands typed this session ==\n");
    if f.commands.is_empty() {
        full.push_str("(none)\n");
    }
    for c in &f.commands {
        full.push_str(c);
        full.push('\n');
    }

    let scrub = |s: String| match f.home.as_deref().filter(|h| !h.is_empty()) {
        Some(h) => s.replace(h, "~"),
        None => s,
    };
    BugReport { summary: scrub(summary), full: scrub(full) }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    fn base() -> BugReportFacts {
        BugReportFacts {
            version: "0.9.7 (d488ae4ef)".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            term_program: Some("ghostty".into()),
            term_program_version: Some("1.2.0".into()),
            term: Some("xterm-ghostty".into()),
            graphics: GraphicsFacts {
                protocol: Some("kitty".into()),
                forced: false,
                images_off: false,
                cell: Some((9, 18)),
                cell_source: "measured",
                pane: (98, 60),
            },
            settings: SettingsFacts {
                images: true,
                v6_render: "hybrid".into(),
                pixel_lock: true,
                game_colours: true,
                style: Some("dusk".into()),
                sound: true,
                audio_device: true,
            },
            logs_dir: "~/.local/state/lanthorn/logs".into(),
            timestamp: "2026-10-18T12:00:00Z".into(),
            terminal_dump: vec!["[dump-terminal]".into(), "  graphics protocol: kitty".into()],
            ..Default::default()
        }
    }

    fn game() -> GameFacts {
        GameFacts {
            title: "Zork Zero".into(),
            file: "zork0.z6".into(),
            engine: "z-machine".into(),
            format_version: Some(6),
            release: Some(393),
            serial: Some("890714".into()),
            checksum: Some("0x1234".into()),
            ifid: Some("ZCODE-393-890714-1234".into()),
            ifdb_tuid: Some("abc123".into()),
            machine: Some("Amiga".into()),
            machine_source: Some("from the disk"),
            ..Default::default()
        }
    }

    fn map() -> MapFacts {
        MapFacts {
            rooms: 42,
            connections: 61,
            layers: 3,
            current_room: Some("West of House".into()),
            pane: Some((60, 40)),
            zoom: "Compact".into(),
            shown: true,
        }
    }

    #[test]
    fn a_full_summary_has_the_shape_the_design_promises() {
        let mut f = base();
        f.game = Some(game());
        f.map = Some(map());
        let expect = "\
lanthorn 0.9.7 (d488ae4ef)
system:   macos aarch64
terminal: ghostty 1.2.0 (TERM=xterm-ghostty) · tmux: no · ssh: no
graphics: kitty (auto-detected) · cell 9x18 measured · pane 98x60
settings: images on · v6 hybrid · pixel lock on · game colours on · style \"dusk\" · sound on
game:     Zork Zero · zork0.z6 · Z-machine v6 r393 s890714 0x1234
          IFID ZCODE-393-890714-1234 · IFDB abc123 · machine Amiga (from the disk)
map:      42 rooms · 61 connections · 3 layers · here: \"West of House\"
          pane 60x40 · zoom Compact · map shown
logs:     ~/.local/state/lanthorn/logs";
        assert_eq!(build(&f).summary, expect);
    }

    #[test]
    fn counts_are_singular_when_there_is_one() {
        let mut f = base();
        f.map = Some(MapFacts { rooms: 1, connections: 1, layers: 1, zoom: "Boxes".into(), ..Default::default() });
        let s = build(&f).summary;
        assert!(s.contains("1 room · 1 connection · 1 layer\n"), "{s}");
    }

    #[test]
    fn no_game_omits_the_game_and_map_sections() {
        let r = build(&base());
        assert!(!r.summary.contains("game:"), "{}", r.summary);
        assert!(!r.summary.contains("map:"), "{}", r.summary);
        assert!(!r.full.contains("== window layout"));
        assert!(!r.full.contains("== map"));
        assert!(r.full.contains("== /dump-terminal =="));
    }

    #[test]
    fn absent_fields_are_omitted_cleanly() {
        let mut f = base();
        f.game = Some(GameFacts { file: "foo.ulx".into(), engine: "glulx".into(), ..Default::default() });
        f.term_program = None;
        f.term_program_version = None;
        f.term = None;
        f.settings.style = None;
        let s = build(&f).summary;
        assert!(s.contains("game:     foo.ulx · Glulx\n"), "{s}");
        assert!(!s.contains("None") && !s.contains("IFID") && !s.contains("IFDB") && !s.contains("machine"), "{s}");
        assert!(s.contains("terminal: unknown terminal · tmux"), "{s}");
        assert!(!s.contains("style"), "{s}");
        assert!(!s.contains("(TERM="), "{s}");
    }

    #[test]
    fn ssh_and_tmux_print_only_yes_or_no() {
        let mut f = base();
        f.ssh = Some("203.0.113.9 51234 198.51.100.2 22".into());
        f.tmux = Some("/private/tmp/tmux-501/default,1234,0".into());
        let r = build(&f);
        assert!(r.summary.contains("tmux: yes · ssh: yes"), "{}", r.summary);
        for text in [&r.summary, &r.full] {
            assert!(!text.contains("203.0.113.9") && !text.contains("tmux-501"), "{text}");
        }
    }

    #[test]
    fn the_home_directory_never_survives_in_either_text() {
        let mut f = base();
        f.home = Some("/Users/someone".into());
        f.game = Some(game());
        f.logs_dir = "/Users/someone/.local/state/lanthorn/logs".into();
        f.window_dump = Some(vec!["source=/Users/someone/games/zork0.z6".into()]);
        f.commands = vec!["open /Users/someone/x".into()];
        let r = build(&f);
        assert!(!r.summary.contains("/Users/someone"), "{}", r.summary);
        assert!(!r.full.contains("/Users/someone"), "{}", r.full);
        assert!(r.summary.contains("logs:     ~/.local/state/lanthorn/logs"));
        assert!(r.summary.contains("zork0.z6"), "the game is named by its file name");
    }

    #[test]
    fn the_full_report_carries_every_section_and_leads_with_the_summary() {
        let mut f = base();
        f.game = Some(game());
        f.map = Some(map());
        f.window_dump = Some(vec!["window 0: text".into()]);
        f.map_json = Some("{\"format\":\"lanthorn-map\"}".into());
        f.commands = vec!["look".into(), "north".into()];
        let r = build(&f);
        assert!(r.full.starts_with("lanthorn bug report, 2026-10-18T12:00:00Z\n\n"), "{}", r.full);
        assert!(r.full.contains(&format!("\n\n{}\n", r.summary)), "summary follows the header");
        let order = [
            "== /dump-terminal ==",
            "graphics protocol: kitty",
            "== window layout (/dump-windows) ==",
            "window 0: text",
            "== map (lanthorn-map JSON) ==",
            "{\"format\":\"lanthorn-map\"}",
            "== commands typed this session ==",
            "look\nnorth\n",
        ];
        // Skip past the summary, which also says "kitty".
        let mut at = r.full.find("== /dump-terminal ==").unwrap();
        for needle in order {
            let i = r.full[at..].find(needle).unwrap_or_else(|| panic!("{needle:?} missing or out of order"));
            at += i;
        }
    }

    #[test]
    fn forced_protocol_and_images_off_are_not_called_detected() {
        let mut f = base();
        f.graphics.forced = true;
        assert!(build(&f).summary.contains("kitty (forced)"));
        f.graphics.images_off = true;
        f.graphics.protocol = None;
        assert!(build(&f).summary.contains("none (images off)"));
    }

    #[test]
    fn no_commands_reads_as_none() {
        assert!(build(&base()).full.contains("== commands typed this session ==\n(none)\n"));
    }
}
