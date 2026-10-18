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

/// The host-neutral half of a report: what any embedding of the engines and the
/// mapper can say. Every privacy rule that is not about a host's own lines is
/// applied to these in [`build`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SharedFacts {
    /// The host's version string (the TUI's is `buildinfo::LONG`).
    pub version: String,
    pub os: String,
    pub arch: String,
    pub timestamp: String,
    /// The home directory, scrubbed to `~` wherever it appears in either text.
    pub home: Option<String>,
    /// `None` with no game loaded.
    pub game: Option<GameFacts>,
    pub map: Option<MapFacts>,
    /// The `lanthorn-map` JSON (the one `/export-json` writes).
    pub map_json: Option<String>,
    /// The commands typed this session.
    pub commands: Vec<String>,
    /// The full report's file, already `~`-abbreviated: what the player attaches.
    pub report_file: String,
    /// Why the full report could not be written, if it could not be. The summary's
    /// `attach:` line then says so instead of pointing at a file that is not there.
    pub save_error: Option<String>,
}

/// What a host adds to the report: its name, its own summary lines and its own
/// full-report sections. [`build`] scrubs the home directory from all of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostPart {
    /// The product name for the header lines: `lanthorn` gives
    /// `lanthorn <version>` and `lanthorn bug report, <timestamp>`.
    pub product: String,
    /// Labelled summary lines, shown between `system:` and the game.
    pub summary: Vec<(String, String)>,
    /// Full-report sections, `(heading, lines)`, placed after the summary and
    /// before the map and the commands.
    pub sections: Vec<(String, Vec<String>)>,
}

/// Everything [`build`] reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BugReportFacts {
    pub shared: SharedFacts,
    pub host: HostPart,
}

/// The terminal UI's own facts: the source of its [`HostPart`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalHostFacts {
    pub term_program: Option<String>,
    pub term_program_version: Option<String>,
    pub term: Option<String>,
    /// The raw `TMUX` value; only its presence is reported.
    pub tmux: Option<String>,
    /// The raw `SSH_CONNECTION` / `SSH_TTY` value; only its presence is reported.
    pub ssh: Option<String>,
    pub graphics: GraphicsFacts,
    pub settings: SettingsFacts,
    /// `/dump-terminal`'s plain text.
    pub terminal_dump: Vec<String>,
    /// `/dump-windows`'s lines; `None` with no game running.
    pub window_dump: Option<Vec<String>>,
}

impl TerminalHostFacts {
    /// The terminal, graphics and settings lines and the two dump sections.
    pub fn into_host_part(self) -> HostPart {
        let pair = |l: &str, v: String| (l.to_string(), v);
        let summary = vec![
            pair("terminal", terminal_value(&self)),
            pair("graphics", graphics_value(&self.graphics)),
            pair("settings", settings_value(&self.settings)),
        ];
        let mut sections = vec![("/dump-terminal".to_string(), self.terminal_dump)];
        if let Some(w) = self.window_dump {
            sections.push(("window layout (/dump-windows)".to_string(), w));
        }
        HostPart { product: "lanthorn".to_string(), summary, sections }
    }
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

fn terminal_value(f: &TerminalHostFacts) -> String {
    let mut program = match (&f.term_program, &f.term_program_version) {
        (Some(p), Some(v)) => format!("{p} {v}"),
        (Some(p), None) => p.clone(),
        (None, _) => "unknown terminal".to_string(),
    };
    if let Some(t) = &f.term {
        program.push_str(&format!(" (TERM={t})"));
    }
    format!("{program} · tmux: {} · ssh: {}", yes_no(f.tmux.is_some()), yes_no(f.ssh.is_some()))
}

fn graphics_value(g: &GraphicsFacts) -> String {
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
    format!("{proto} · {cell} · pane {}x{}", g.pane.0, g.pane.1)
}

fn settings_value(s: &SettingsFacts) -> String {
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
        // `AppState::audio` is opened lazily on the first sound that plays, so
        // `None` means "not started yet"; nothing records a failed open, so a
        // device failure is never claimed here.
        (true, false) => "sound on (not started yet)".to_string(),
        (false, _) => "sound off".to_string(),
    });
    parts.join(" · ")
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

/// What a host has on hand about the session, for [`gather_session_facts`].
pub struct SessionInputs<'a> {
    pub session: &'a dyn crate::engine::Engine,
    pub mapper: &'a mapper::mapper::Mapper,
    pub story_bytes: &'a [u8],
    /// The story file's path; empty means no game is loaded. Only its file name
    /// ever reaches the report.
    pub story_path: &'a std::path::Path,
    pub title: &'a str,
    pub ifid: &'a str,
    pub ifdb_tuid: Option<String>,
    /// Which story on a multi-story disk image.
    pub disk_entry: Option<String>,
    pub profile: crate::interpreter::InterpreterProfile,
    pub profile_source: crate::interpreter::ProfileSource,
    /// The map pane in cells; `None` when it is hidden.
    pub map_pane: Option<(u16, u16)>,
    /// The map zoom, spelled the way the host names it.
    pub zoom: String,
    /// The commands typed this session.
    pub commands: &'a [String],
}

/// The game, map and command facts of a running session, plus the map JSON.
/// Everything else in [`SharedFacts`] (version, system, timestamp, home, report
/// file) is the host's to fill in, so this is returned as a value to merge:
/// `SharedFacts { game, map, map_json, commands, ..host_basics }`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionFacts {
    pub game: Option<GameFacts>,
    pub map: Option<MapFacts>,
    pub map_json: Option<String>,
    pub commands: Vec<String>,
}

/// Gather the game, map and command facts from a running session. With no story
/// path the game, map and map JSON are `None`. Reuses the producers
/// `/export-json` uses; a game is named by its file name only.
pub fn gather_session_facts(i: &SessionInputs) -> SessionFacts {
    use crate::interpreter::{InterpreterProfile, ProfileSource};

    let commands = i.commands.to_vec();
    if i.story_path.as_os_str().is_empty() {
        return SessionFacts { commands, ..Default::default() };
    }
    let walked = crate::export_json::build_walked_story(i.session, i.story_bytes, i.story_path);
    let ident = &walked.ident;
    let is_z = ident.engine == "z-machine";
    let game = GameFacts {
        title: i.title.to_string(),
        file: ident.file.clone(),
        engine: ident.engine.to_string(),
        format_version: if is_z { i.story_bytes.first().copied() } else { None },
        release: ident.release,
        serial: ident.serial.clone(),
        checksum: ident.checksum.clone(),
        ifid: Some(i.ifid.to_string()),
        ifdb_tuid: i.ifdb_tuid.clone(),
        disk_entry: i.disk_entry.clone(),
        // The player-facing ZMSD name; the IBM PC advertises no number (the VM's
        // own rule is 6 on v6, 1 elsewhere), so it is named as machine 6.
        machine: is_z.then(|| match i.profile.interpreter_number() {
            Some(n) => crate::launch_options::interpreter_name(n).to_string(),
            None if i.profile == InterpreterProfile::IbmPc => crate::launch_options::interpreter_name(6).to_string(),
            None => format!("{:?}", i.profile),
        }),
        machine_source: is_z.then_some(match i.profile_source {
            ProfileSource::Medium => "from the disk",
            ProfileSource::Asked => "asked for",
            ProfileSource::Fallback => "default",
        }),
    };
    let graph = &i.mapper.graph;
    let map = MapFacts {
        rooms: graph.rooms().count(),
        connections: graph.connections().len(),
        layers: graph.layers().len(),
        current_room: graph.current().and_then(|id| graph.room(id)).map(|r| r.name.clone()),
        pane: i.map_pane,
        zoom: i.zoom.clone(),
        shown: i.map_pane.is_some(),
    };
    SessionFacts {
        game: Some(game),
        map: Some(map),
        map_json: Some(crate::export_json::render_walked_json(graph, &walked)),
        commands,
    }
}

/// Build the summary and the full report from `f`.
pub fn build(f: &BugReportFacts) -> BugReport {
    let (sh, host) = (&f.shared, &f.host);
    let mut lines = vec![
        format!("{} {}", host.product, sh.version),
        labelled("system", &format!("{} {}", sh.os, sh.arch)),
    ];
    lines.extend(host.summary.iter().map(|(l, v)| labelled(l, v)));
    if let Some(g) = &sh.game {
        lines.extend(game_lines(g));
    }
    if let Some(m) = &sh.map {
        lines.extend(map_lines(m));
    }
    lines.push(labelled(
        "attach",
        &match &sh.save_error {
            None => format!("{} (the full report: drag it into the issue)", sh.report_file),
            Some(e) => format!("could not save {}: {e}", sh.report_file),
        },
    ));
    let summary = lines.join("\n");

    let mut full = format!("{} bug report, {}\n\n{summary}\n", host.product, sh.timestamp);
    for (heading, body) in &host.sections {
        full.push_str(&format!("\n== {heading} ==\n"));
        for l in body {
            full.push_str(l);
            full.push('\n');
        }
    }
    if let Some(j) = &sh.map_json {
        full.push_str("\n== map (lanthorn-map JSON) ==\n");
        full.push_str(j);
        if !j.ends_with('\n') {
            full.push('\n');
        }
    }
    full.push_str("\n== commands typed this session ==\n");
    if sh.commands.is_empty() {
        full.push_str("(none)\n");
    }
    for c in &sh.commands {
        full.push_str(c);
        full.push('\n');
    }

    let scrub = |s: String| match sh.home.as_deref().filter(|h| !h.is_empty()) {
        Some(h) => s.replace(h, "~"),
        None => s,
    };
    BugReport { summary: scrub(summary), full: scrub(full) }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    /// The terminal UI's facts, as the pre-split tests built them.
    struct Tui {
        shared: SharedFacts,
        term: TerminalHostFacts,
    }

    fn build(f: &Tui) -> BugReport {
        super::build(&BugReportFacts { shared: f.shared.clone(), host: f.term.clone().into_host_part() })
    }

    fn base() -> Tui {
        Tui {
            shared: SharedFacts {
                version: "0.9.7 (d488ae4ef)".into(),
                os: "macos".into(),
                arch: "aarch64".into(),
                report_file: "~/.local/state/lanthorn/report-bug.txt".into(),
                timestamp: "2026-10-18T12:00:00Z".into(),
                ..Default::default()
            },
            term: TerminalHostFacts {
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
                terminal_dump: vec!["[dump-terminal]".into(), "  graphics protocol: kitty".into()],
                ..Default::default()
            },
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
        f.shared.game = Some(game());
        f.shared.map = Some(map());
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
attach:   ~/.local/state/lanthorn/report-bug.txt (the full report: drag it into the issue)";
        assert_eq!(build(&f).summary, expect);
    }

    #[test]
    fn counts_are_singular_when_there_is_one() {
        let mut f = base();
        f.shared.map = Some(MapFacts { rooms: 1, connections: 1, layers: 1, zoom: "Boxes".into(), ..Default::default() });
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
        f.shared.game = Some(GameFacts { file: "foo.ulx".into(), engine: "glulx".into(), ..Default::default() });
        f.term.term_program = None;
        f.term.term_program_version = None;
        f.term.term = None;
        f.term.settings.style = None;
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
        f.term.ssh = Some("203.0.113.9 51234 198.51.100.2 22".into());
        f.term.tmux = Some("/private/tmp/tmux-501/default,1234,0".into());
        let r = build(&f);
        assert!(r.summary.contains("tmux: yes · ssh: yes"), "{}", r.summary);
        for text in [&r.summary, &r.full] {
            assert!(!text.contains("203.0.113.9") && !text.contains("tmux-501"), "{text}");
        }
    }

    #[test]
    fn the_home_directory_never_survives_in_either_text() {
        let mut f = base();
        f.shared.home = Some("/Users/someone".into());
        f.shared.game = Some(game());
        f.shared.report_file = "/Users/someone/.local/state/lanthorn/report-bug.txt".into();
        f.term.window_dump = Some(vec!["source=/Users/someone/games/zork0.z6".into()]);
        f.shared.commands = vec!["open /Users/someone/x".into()];
        let r = build(&f);
        assert!(!r.summary.contains("/Users/someone"), "{}", r.summary);
        assert!(!r.full.contains("/Users/someone"), "{}", r.full);
        assert!(r.summary.contains("attach:   ~/.local/state/lanthorn/report-bug.txt"));
        assert!(!r.summary.contains("logs:"));
        assert!(r.summary.contains("zork0.z6"), "the game is named by its file name");
    }

    #[test]
    fn the_full_report_carries_every_section_and_leads_with_the_summary() {
        let mut f = base();
        f.shared.game = Some(game());
        f.shared.map = Some(map());
        f.term.window_dump = Some(vec!["window 0: text".into()]);
        f.shared.map_json = Some("{\"format\":\"lanthorn-map\"}".into());
        f.shared.commands = vec!["look".into(), "north".into()];
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
        f.term.graphics.forced = true;
        assert!(build(&f).summary.contains("kitty (forced)"));
        f.term.graphics.images_off = true;
        f.term.graphics.protocol = None;
        assert!(build(&f).summary.contains("none (images off)"));
    }

    #[test]
    fn a_failed_save_replaces_the_attach_promise() {
        let mut f = base();
        f.shared.save_error = Some("permission denied".into());
        let s = build(&f).summary;
        assert!(
            s.ends_with("attach:   could not save ~/.local/state/lanthorn/report-bug.txt: permission denied"),
            "{s}"
        );
        assert!(!s.contains("drag it"), "{s}");
    }

    #[test]
    fn sound_without_a_sink_is_not_called_no_device() {
        let mut f = base();
        f.term.settings.audio_device = false;
        let s = build(&f).summary;
        assert!(s.contains("sound on (not started yet)") && !s.contains("no device"), "{s}");
    }

    #[test]
    fn no_commands_reads_as_none() {
        assert!(build(&base()).full.contains("== commands typed this session ==\n(none)\n"));
    }

    /// SQ-1797: a host that is not a terminal supplies its own name, lines and
    /// sections and gets the shared game, map and command parts, the attach line
    /// and the privacy rules, with nothing of the terminal's.
    #[test]
    fn a_non_terminal_host_gets_the_shared_parts_and_its_own_lines() {
        let f = BugReportFacts {
            shared: SharedFacts {
                version: "1.0.0".into(),
                os: "windows".into(),
                arch: "x86_64".into(),
                timestamp: "2026-10-18T12:00:00Z".into(),
                home: Some("/Users/someone".into()),
                // A host that hands over a full path as the file gets it as given:
                // `GameFacts::file` is the NAME, and `gather_session_facts` is what
                // reduces a path to one. Here the file is already a name.
                game: Some(GameFacts { file: "zork0.z6".into(), engine: "z-machine".into(), ..Default::default() }),
                map: Some(map()),
                map_json: Some("{\"format\":\"lanthorn-map\"}".into()),
                commands: vec!["look".into()],
                report_file: "~/reports/bug.txt".into(),
                save_error: None,
            },
            host: HostPart {
                product: "Golden Lanthorn".into(),
                summary: vec![
                    ("window".into(), "1280x800 at /Users/someone/Games".into()),
                    ("renderer".into(), "wgpu".into()),
                ],
                sections: vec![("GPU".into(), vec!["adapter: test /Users/someone/cache".into()])],
            },
        };
        let r = super::build(&f);
        assert!(r.summary.starts_with("Golden Lanthorn 1.0.0\nsystem:   windows x86_64\nwindow:   1280x800 at ~/Games\nrenderer: wgpu\n"), "{}", r.summary);
        assert!(r.summary.ends_with("attach:   ~/reports/bug.txt (the full report: drag it into the issue)"), "{}", r.summary);
        assert!(r.full.starts_with("Golden Lanthorn bug report, 2026-10-18T12:00:00Z\n"), "{}", r.full);
        for needle in ["== GPU ==\nadapter: test ~/cache\n", "== map (lanthorn-map JSON) ==", "== commands typed this session ==\nlook\n"] {
            assert!(r.full.contains(needle), "{needle:?} missing: {}", r.full);
        }
        for text in [&r.summary, &r.full] {
            assert!(!text.contains("/Users/someone"), "{text}");
            for terminal in ["terminal:", "TERM", "tmux", "ssh", "graphics:", "settings:", "/dump-terminal", "/dump-windows"] {
                assert!(!text.contains(terminal), "{terminal:?} leaked into {text}");
            }
        }
    }

    /// SQ-1797, real-game smoke: the public gatherer names the game by FILE NAME
    /// whatever path it is handed, and carries the commands and the map JSON.
    #[test]
    fn gather_session_facts_names_the_game_by_file_name_only() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/stories/minizork-r34-s871124.z3");
        let Ok(bytes) = std::fs::read(&fixture) else { return }; // fixture absent: vacuous, like the other real-game suites
        let session = crate::session::GameSession::new(bytes.clone(), true, false, None).expect("boots");
        let mapper = mapper::mapper::Mapper::default();
        let commands = vec!["look".to_string()];
        let inputs = SessionInputs {
            session: &session,
            mapper: &mapper,
            story_bytes: &bytes,
            story_path: std::path::Path::new("/Users/someone/private/minizork-r34-s871124.z3"),
            title: "Mini-Zork",
            ifid: "ZCODE-34-871124-A3C7",
            ifdb_tuid: None,
            disk_entry: None,
            profile: crate::interpreter::InterpreterProfile::IbmPc,
            profile_source: crate::interpreter::ProfileSource::Fallback,
            map_pane: Some((60, 40)),
            zoom: "Compact".into(),
            commands: &commands,
        };
        let facts = gather_session_facts(&inputs);
        let game = facts.game.as_ref().expect("game");
        assert_eq!(game.file, "minizork-r34-s871124.z3");
        assert_eq!((game.engine.as_str(), game.format_version, game.release), ("z-machine", Some(3), Some(34)));
        assert_eq!(game.machine.as_deref(), Some("IBM PC"));
        assert_eq!(facts.map.as_ref().map(|m| (m.pane, m.shown)), Some((Some((60, 40)), true)));
        assert_eq!(facts.commands, commands);
        let json = facts.map_json.as_deref().expect("map json");
        assert!(!json.contains("/Users/someone"), "{json}");
        assert!(!format!("{facts:?}").contains("/Users/someone"));

        let none = gather_session_facts(&SessionInputs { story_path: std::path::Path::new(""), ..inputs });
        assert!(none.game.is_none() && none.map.is_none() && none.map_json.is_none());
    }
}
