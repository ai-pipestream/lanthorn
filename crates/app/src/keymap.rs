//! Configurable keymap — `KeySpec`, `Context`, and `KeyMap`.
//!
//! Commands are identified by their registry command-string (see `crate::slash`).
//! `KeySpec` is a parsed keystroke (key code + modifier flags). `Context`
//! partitions bindings into Global, Map, and Anim layers. `KeyMap` holds the
//! full binding table and exposes lookup / resolve / primary-key queries.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ── Context ────────────────────────────────────────────────────────────────────

/// Which dispatch layer a binding belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Context {
    /// Reached in any focus when no prompt or anim sub-mode is active.
    Global,
    /// Map-focus bindings (also fall through to Global on miss).
    Map,
    /// Tidy-animation sub-mode (does NOT fall through).
    Anim,
}

// ── KeySpec ────────────────────────────────────────────────────────────────────

/// A parsed keystroke: key code plus modifier flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeySpec {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeySpec {
    /// Normalize a live crossterm `KeyEvent` into a `KeySpec` for lookups.
    pub fn from_key_event(k: KeyEvent) -> KeySpec {
        KeySpec {
            code: k.code,
            ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
            shift: k.modifiers.contains(KeyModifiers::SHIFT),
            alt: k.modifiers.contains(KeyModifiers::ALT),
        }
    }

    /// Human-readable label for the hint bar / help screen.
    /// Examples: "Ctrl+S", "Shift+←", "h", "F1", "Space", "Shift+Tab".
    pub fn label(&self) -> String {
        // BackTab is always "Shift+Tab" regardless of modifier flags.
        if self.code == KeyCode::BackTab {
            let mut s = String::new();
            if self.ctrl { s.push_str("Ctrl+"); }
            if self.alt { s.push_str("Alt+"); }
            s.push_str("Shift+Tab");
            return s;
        }
        let mut s = String::new();
        if self.ctrl { s.push_str("Ctrl+"); }
        if self.alt { s.push_str("Alt+"); }
        if self.shift { s.push_str("Shift+"); }
        let key_str = match self.code {
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => unreachable!("handled above"),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::Char(c) => c.to_uppercase().to_string(),
            _ => format!("{:?}", self.code),
        };
        s.push_str(&key_str);
        s
    }
}

impl std::str::FromStr for KeySpec {
    type Err = String;

    /// Parse a key spec string like "ctrl+s", "shift+left", "+", "f1", "space".
    ///
    /// Modifiers (ctrl, shift, alt) may appear in any order before the key
    /// token, separated by '+'. A lone '+' character parses as Char('+').
    fn from_str(s: &str) -> Result<KeySpec, String> {
        let lower = s.trim().to_lowercase();

        // Special case: a lone "+" (would otherwise produce empty tokens when split).
        if lower == "+" {
            return Ok(KeySpec { code: KeyCode::Char('+'), ctrl: false, shift: false, alt: false });
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key_token: Option<String> = None;

        let parts: Vec<&str> = lower.split('+').collect();
        let n = parts.len();

        // Walk tokens: modifier keywords consume early slots; the last
        // non-empty, non-modifier token is the key. Handle "+" embedded in
        // the split: a trailing empty part after '+' means '+' was the last char.
        let mut i = 0;
        while i < n {
            let p = parts[i].trim();
            match p {
                "ctrl" | "control" => { ctrl = true; i += 1; }
                "shift" => { shift = true; i += 1; }
                "alt" => { alt = true; i += 1; }
                "" => {
                    // An empty segment from split means a literal '+' was there.
                    // E.g. "shift++" splits as ["shift", "", ""] — the key is '+'.
                    // We treat the first empty token after modifiers as the '+' key.
                    key_token = Some("+".to_string());
                    i += 1;
                }
                other => {
                    key_token = Some(other.to_string());
                    i += 1;
                }
            }
        }

        let tok = key_token.ok_or_else(|| format!("empty key spec: '{s}'"))?;

        let code = match tok.as_str() {
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "tab" => {
                // "shift+tab" parses as shift=true + token "tab"; map to BackTab.
                if shift {
                    shift = false; // BackTab encodes the shift itself
                    KeyCode::BackTab
                } else {
                    KeyCode::Tab
                }
            }
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "esc" | "escape" => KeyCode::Esc,
            "enter" | "return" => KeyCode::Enter,
            "backspace" => KeyCode::Backspace,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
            "f1" => KeyCode::F(1),
            "f2" => KeyCode::F(2),
            "f3" => KeyCode::F(3),
            "f4" => KeyCode::F(4),
            "f5" => KeyCode::F(5),
            "f6" => KeyCode::F(6),
            "f7" => KeyCode::F(7),
            "f8" => KeyCode::F(8),
            "f9" => KeyCode::F(9),
            "f10" => KeyCode::F(10),
            "f11" => KeyCode::F(11),
            "f12" => KeyCode::F(12),
            s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
            other => return Err(format!("unknown key token: '{other}'")),
        };

        Ok(KeySpec { code, ctrl, shift, alt })
    }
}

// ── KeyMap ─────────────────────────────────────────────────────────────────────

/// The full binding table. Each entry is `(KeySpec, String, Context)`.
/// Multiple specs may map to the same command string (multi-bind defaults).
#[derive(Debug)]
pub struct KeyMap {
    pub bindings: Vec<(KeySpec, String, Context)>,
}

impl Default for KeyMap {
    /// Build the default keymap from today's `key_to_action` dispatch.
    ///
    /// This is the single source of truth for back-compat. Every binding here
    /// must match `input.rs` exactly.
    fn default() -> Self {
        use KeyCode::*;

        // Shorthand constructors.
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        let plain = |code| g(code, false, false);
        let ctrl = |code| g(code, true, false);
        let shift = |code| g(code, false, true);

        let mut b: Vec<(KeySpec, String, Context)> = Vec::new();

        macro_rules! bind {
            ($spec:expr, $cmd:expr, $ctx:expr) => {
                b.push(($spec, $cmd.to_string(), $ctx));
            };
        }

        // ── Global ────────────────────────────────────────────────────────────
        // Tab → toggle-focus (the Tab KEY itself stays hardwired in key_to_action;
        // this entry lets the keymap advertise it for hints/help).
        bind!(plain(Tab), "toggle-focus", Context::Global);

        bind!(ctrl(Char('s')), "save-state", Context::Global);
        bind!(ctrl(Char('r')), "restore-state", Context::Global);
        bind!(ctrl(Char('e')), "export-svg", Context::Global);
        bind!(ctrl(Char('g')), "export-dot", Context::Global);
        bind!(ctrl(Char('d')), "export-dump", Context::Global);
        bind!(ctrl(Char('l')), "cycle-layout", Context::Global);
        bind!(ctrl(Char('t')), "tidy-map", Context::Global);
        bind!(ctrl(Char('y')), "animate-tidy", Context::Global);
        bind!(ctrl(Char('a')), "toggle-alignment", Context::Global);
        bind!(ctrl(Char('p')), "toggle-portal-labels", Context::Global);
        // Ctrl+O → open saves manager (free key; not used by any other command).
        bind!(ctrl(Char('o')), "open-saves", Context::Global);
        // v → toggle inventory strip (free key; not used in any context).
        bind!(plain(Char('v')), "toggle-inventory", Context::Global);
        // m → open verb/item token-palette modal (free key; not used by any other command).
        bind!(plain(Char('m')), "open-verb-menu", Context::Global);
        // F2 → open config screen.
        bind!(plain(F(2)), "open-config", Context::Global);
        // F3 → open style editor (F3 is unbound; F2/F4–F9 are taken).
        bind!(plain(F(3)), "open-style-editor", Context::Global);
        // Shift+Tab (BackTab) → cycle layout in reverse (inverse of Ctrl+L forward cycle).
        // BackTab is delivered by crossterm as KeyCode::BackTab, typically with no SHIFT modifier.
        bind!(KeySpec { code: BackTab, ctrl: false, shift: false, alt: false }, "cycle-layout reverse", Context::Global);
        // F5 → reset game (free key; opens a confirmation prompt before acting).
        bind!(plain(F(5)), "reset-game", Context::Global);
        // F4 → open rewind/replay history modal (free function key).
        bind!(plain(F(4)), "open-history", Context::Global);

        // F6-F9 → Nudge (plain function keys; ctrl+arrow removed so all direct
        // bindings remain modifier-free).
        bind!(plain(F(6)), "nudge-room -1 0", Context::Global);
        bind!(plain(F(7)), "nudge-room 1 0", Context::Global);
        bind!(plain(F(8)), "nudge-room 0 -1", Context::Global);
        bind!(plain(F(9)), "nudge-room 0 1", Context::Global);

        // ── Map ───────────────────────────────────────────────────────────────
        // Pan: plain arrows + hjkl (two sets; shift-arrows removed so all
        // direct bindings remain modifier-free).
        bind!(plain(Left), "pan-map -1 0", Context::Map);
        bind!(plain(Right), "pan-map 1 0", Context::Map);
        bind!(plain(Up), "pan-map 0 -1", Context::Map);
        bind!(plain(Down), "pan-map 0 1", Context::Map);

        bind!(plain(Char('h')), "pan-map -1 0", Context::Map);
        bind!(plain(Char('l')), "pan-map 1 0", Context::Map);
        bind!(plain(Char('k')), "pan-map 0 -1", Context::Map);
        bind!(plain(Char('j')), "pan-map 0 1", Context::Map);

        // Zoom: + / = (shift(+) alias removed; plain alternatives cover it).
        bind!(plain(Char('+')), "zoom-map in", Context::Map);
        bind!(plain(Char('=')), "zoom-map in", Context::Map);
        bind!(plain(Char('-')), "zoom-map out", Context::Map);
        // '0' (zero) resets zoom to default (Boxes) and clears char-pan offset.
        bind!(plain(Char('0')), "zoom-map reset", Context::Map);

        // Map commands
        bind!(plain(Char('c')), "center-map", Context::Map);
        bind!(plain(Char('n')), "select-room next", Context::Map);
        bind!(plain(Char('p')), "select-room prev", Context::Map);
        bind!(shift(Char('N')), "rename-layer", Context::Map);
        bind!(shift(Char('P')), "peel-layer", Context::Map);
        bind!(shift(Char('M')), "merge-layer", Context::Map);
        bind!(shift(Char('R')), "tidy-map", Context::Map);
        bind!(plain(Char(']')), "cycle-layer next", Context::Map);
        bind!(plain(Char('[')), "cycle-layer prev", Context::Map);
        bind!(plain(Char('r')), "rename-room", Context::Map);
        bind!(plain(Char('o')), "edit-notes", Context::Map);
        bind!(plain(Char('d')), "delete-connection", Context::Map);
        bind!(plain(Char('e')), "relabel-edge", Context::Map);
        bind!(plain(Char('i')), "toggle-inspector", Context::Map);
        bind!(plain(Char('g')), "open-gallery", Context::Map);
        // Esc → toggle-focus (in map context)
        bind!(plain(Esc), "toggle-focus", Context::Map);

        // ── Anim ──────────────────────────────────────────────────────────────
        // Pan in anim: hjkl only (plain arrows are bound to step; shift-arrows
        // removed so all direct bindings remain modifier-free).
        bind!(plain(Char('h')), "pan-map -1 0", Context::Anim);
        bind!(plain(Char('l')), "pan-map 1 0", Context::Anim);
        bind!(plain(Char('k')), "pan-map 0 -1", Context::Anim);
        bind!(plain(Char('j')), "pan-map 0 1", Context::Anim);

        // Zoom in anim
        bind!(plain(Char('+')), "zoom-map in", Context::Anim);
        bind!(plain(Char('=')), "zoom-map in", Context::Anim);
        bind!(plain(Char('-')), "zoom-map out", Context::Anim);

        // Step / play / exit
        bind!(plain(Left), "anim-step back", Context::Anim);
        bind!(plain(Right), "anim-step forward", Context::Anim);
        bind!(plain(Char(' ')), "anim-play", Context::Anim);
        bind!(plain(Esc), "anim-exit", Context::Anim);
        bind!(plain(Enter), "anim-exit", Context::Anim);

        KeyMap { bindings: b }
    }
}

impl KeyMap {
    /// Look up a key in the given context.
    ///
    /// - `Context::Map` also searches `Context::Global` on miss (fall-through).
    /// - `Context::Global` and `Context::Anim` do not fall through.
    pub fn lookup(&self, spec: &KeySpec, ctx: Context) -> Option<&str> {
        // Exact context match first.
        for (s, cmd, c) in &self.bindings {
            if c == &ctx && s == spec {
                return Some(cmd.as_str());
            }
        }
        // Map falls through to Global.
        if ctx == Context::Map {
            for (s, cmd, c) in &self.bindings {
                if c == &Context::Global && s == spec {
                    return Some(cmd.as_str());
                }
            }
        }
        None
    }

    /// Return the first (primary) `KeySpec` whose command string starts with `command_name`.
    pub fn primary_key(&self, command_name: &str) -> Option<KeySpec> {
        self.bindings.iter()
            .find(|(_, s, _)| s.split_whitespace().next() == Some(command_name))
            .map(|(spec, _, _)| *spec)
    }

    /// Look up a key across ALL contexts (Global → Map → Anim) and return the
    /// first match. Used by the hotkey dialog so that commands in any context
    /// can be triggered from the dialog.
    pub fn lookup_any(&self, spec: &KeySpec) -> Option<&str> {
        for ctx in [Context::Global, Context::Map, Context::Anim] {
            // Use exact context match only (no Map→Global fallthrough here,
            // since we already iterate Global first).
            for (s, cmd, c) in &self.bindings {
                if c == &ctx && s == spec {
                    return Some(cmd.as_str());
                }
            }
        }
        None
    }

    /// Iterate all `(KeySpec, &str)` pairs that belong to `ctx`
    /// (for the help screen's per-context listing).
    pub fn for_context(&self, ctx: Context) -> impl Iterator<Item = (&KeySpec, &str)> {
        self.bindings.iter()
            .filter(move |(_, _, c)| *c == ctx)
            .map(|(s, cmd, _)| (s, cmd.as_str()))
    }

    /// Build a keymap from config overrides.
    ///
    /// Returns the resolved `KeyMap` and a list of warning strings for
    /// overrides that were rejected (unknown name, parse error, conflict).
    pub fn resolve(cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String>) {
        let mut km = if cfg.use_defaults { KeyMap::default() } else { KeyMap { bindings: Vec::new() } };
        let mut warnings = Vec::new();
        for (ctx, section) in [
            (Context::Global, &cfg.global),
            (Context::Map, &cfg.map),
            (Context::Anim, &cfg.anim),
        ] {
            for (key, command) in section {
                let spec = match key.parse::<KeySpec>() {
                    Ok(s) => s,
                    Err(e) => { warnings.push(format!("keymap: cannot parse key '{key}': {e}; skipped")); continue; }
                };
                let cmd_name = command.split_whitespace().next().unwrap_or("");
                if crate::slash::find_command(cmd_name).is_none() {
                    warnings.push(format!("keymap: unknown command '{command}'; skipped"));
                    continue;
                }
                km.bindings.retain(|(s, _, c)| !(*s == spec && *c == ctx));
                km.bindings.push((spec, command.clone(), ctx));
            }
        }
        (km, warnings)
    }
}

// ── HotkeyLayout ──────────────────────────────────────────────────────────────

/// Default full command-strings for the direct (always-available) command set.
const DEFAULT_DIRECT_COMMANDS: &[&str] = &[
    "quit",
    "save-state",
    "restore-state",
    "pan-map -1 0",
    "pan-map 1 0",
    "pan-map 0 -1",
    "pan-map 0 1",
    "zoom-map in",
    "zoom-map out",
    "zoom-map reset",
    "select-room next",
    "select-room prev",
    "center-map",
    "toggle-focus",
    "cycle-layout reverse",
    "nudge-room -1 0",
    "nudge-room 1 0",
    "nudge-room 0 -1",
    "nudge-room 0 1",
];

/// Default groups for the hotkey dialog (title, authored leader-key + full command-string).
const DEFAULT_GROUPS: &[(&str, &[(char, &str)])] = &[
    ("Layout", &[('t', "tidy-map"), ('a', "animate-tidy"), ('l', "cycle-layout")]),
    ("Layers", &[('p', "peel-layer"), ('m', "merge-layer"), ('c', "cycle-layer next"), ('n', "rename-layer")]),
    ("Edit", &[('r', "rename-room"), ('o', "edit-notes"), ('d', "delete-connection"), ('e', "relabel-edge")]),
    ("Files", &[
        ('s', "open-saves"), ('h', "open-history"), ('x', "reset-game"),
        ('v', "export-svg"), ('g', "export-dot"), ('u', "export-dump"),
    ]),
    ("View", &[
        ('i', "toggle-inspector"), ('f', "open-gallery"), ('b', "open-verb-menu"),
        ('w', "open-config"), ('y', "toggle-inventory"), ('j', "toggle-alignment"),
        ('q', "toggle-portal-labels"),
    ]),
];

/// Runtime layout for the hotkey dialog.
///
/// Controls which key opens the dialog (`prefix`), which commands are always
/// reachable without the dialog (`direct`), and how commands are grouped inside
/// the dialog (`groups`).
#[derive(Debug)]
pub struct HotkeyLayout {
    /// The key that opens (and closes) the dialog.
    pub prefix: KeySpec,
    /// Full command-strings that are always available without opening the dialog.
    pub direct: std::collections::HashSet<String>,
    /// Groups of commands shown in the dialog: (group title, [(leader letter, command-string)]).
    pub groups: Vec<(String, Vec<(char, String)>)>,
}

impl Default for HotkeyLayout {
    /// Build the built-in default layout.
    fn default() -> Self {
        let prefix: KeySpec = "ctrl+k".parse().expect("ctrl+k must parse");

        let direct = DEFAULT_DIRECT_COMMANDS.iter().map(|s| s.to_string()).collect();

        let groups = DEFAULT_GROUPS
            .iter()
            .map(|(title, entries)| {
                (title.to_string(), entries.iter().map(|(letter, cmd)| (*letter, cmd.to_string())).collect())
            })
            .collect();

        HotkeyLayout { prefix, direct, groups }
    }
}

impl HotkeyLayout {
    /// Resolve a `HotkeyLayout` from config, producing warnings for unknown command names.
    ///
    /// Fields that are `None` in the config use the built-in defaults.
    pub fn resolve(cfg: &crate::config::HotkeysConfig) -> (HotkeyLayout, Vec<String>) {
        let mut layout = HotkeyLayout::default();
        let mut warnings: Vec<String> = Vec::new();

        // Override prefix if specified.
        if let Some(prefix_str) = &cfg.prefix {
            match prefix_str.parse::<KeySpec>() {
                Ok(spec) => layout.prefix = spec,
                Err(e) => warnings.push(format!("hotkeys: prefix '{}': {e}; using default", prefix_str)),
            }
        }

        // Override direct set if specified. Each entry is a full command-string;
        // its first token is validated against the registry.
        if let Some(direct_cmds) = &cfg.direct {
            let mut direct_set = std::collections::HashSet::new();
            for cmd in direct_cmds {
                let name = cmd.split_whitespace().next().unwrap_or("");
                if crate::slash::find_command(name).is_some() {
                    direct_set.insert(cmd.clone());
                } else {
                    warnings.push(format!("hotkeys: direct: unknown command '{cmd}'; skipped"));
                }
            }
            layout.direct = direct_set;
        }

        // Override groups if any are specified.
        if !cfg.group.is_empty() {
            let mut groups: Vec<(String, Vec<(char, String)>)> = Vec::new();
            let mut used_letters: std::collections::HashSet<char> = std::collections::HashSet::new();

            for g in &cfg.group {
                let mut cmds: Vec<(char, String)> = Vec::new();
                for entry in &g.commands {
                    let tokens: Vec<&str> = entry.split_whitespace().collect();
                    if tokens.is_empty() {
                        continue;
                    }

                    // Try letter-prefixed form, e.g. "t tidy-map".
                    let mut parsed: Option<(char, String)> = None;
                    if tokens[0].chars().count() == 1 && tokens.len() > 1 {
                        let letter = tokens[0].chars().next().unwrap();
                        if crate::slash::find_command(tokens[1]).is_some() {
                            parsed = Some((letter, tokens[1..].join(" ")));
                        }
                    }

                    let (letter, cmd) = if let Some(lc) = parsed {
                        lc
                    } else {
                        // Whole entry is the command-string; auto-assign a free letter.
                        if crate::slash::find_command(tokens[0]).is_none() {
                            warnings.push(format!("hotkeys: group '{}': unknown command '{entry}'; dropped", g.title));
                            continue;
                        }
                        match ('a'..='z').find(|c| !used_letters.contains(c)) {
                            Some(letter) => (letter, entry.clone()),
                            None => {
                                warnings.push(format!("hotkeys: group '{}': no free letter for '{entry}'; dropped", g.title));
                                continue;
                            }
                        }
                    };

                    if used_letters.contains(&letter) {
                        warnings.push(format!("hotkeys: group '{}': letter '{}' already used; dropped '{}'", g.title, letter, cmd));
                        continue;
                    }
                    used_letters.insert(letter);
                    cmds.push((letter, cmd));
                }
                groups.push((g.title.clone(), cmds));
            }
            layout.groups = groups;
        }

        (layout, warnings)
    }

    /// Return the command-string bound to leader letter `key`, if any.
    pub fn leader_command(&self, key: char) -> Option<&str> {
        self.groups.iter()
            .flat_map(|(_, cmds)| cmds.iter())
            .find(|(letter, _)| *letter == key)
            .map(|(_, cmd)| cmd.as_str())
    }

    /// Check whether a full keymap command-string resolves to a direct command.
    ///
    /// `cmd_str` is the full binding string as returned by `KeyMap::lookup`
    /// (e.g. `"zoom-map in"`, `"save-state"`). Matched as a whole against the
    /// direct set, so a command with arguments is matched exactly (e.g.
    /// `"cycle-layout"` is not direct even though `"cycle-layout reverse"` is).
    pub fn is_direct_name(&self, cmd_str: &str) -> bool {
        self.direct.contains(cmd_str)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Action;

    // Task 2: KeySpec parsing and labels
    #[test]
    fn keyspec_parse_and_label_roundtrip() {
        let s: KeySpec = "ctrl+s".parse().unwrap();
        assert_eq!((s.ctrl, s.code), (true, KeyCode::Char('s')));
        assert_eq!("shift+left".parse::<KeySpec>().unwrap().code, KeyCode::Left);
        assert_eq!("+".parse::<KeySpec>().unwrap().code, KeyCode::Char('+'));
        assert_eq!("f1".parse::<KeySpec>().unwrap().code, KeyCode::F(1));
        assert_eq!("space".parse::<KeySpec>().unwrap().code, KeyCode::Char(' '));
        assert!("nope".parse::<KeySpec>().is_err());
        assert_eq!("ctrl+s".parse::<KeySpec>().unwrap().label(), "Ctrl+S");
    }

    // Task 3a: default keymap matches today's bindings
    #[test]
    fn default_keymap_matches_todays_bindings() {
        let km = KeyMap::default();
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        use KeyCode::*;
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Global), Some("save-state"));
        assert_eq!(km.lookup(&g(Char('n'), false, false), Context::Map), Some("select-room next"));
        assert_eq!(km.lookup(&g(Char('h'), false, false), Context::Map), Some("pan-map -1 0"));
        // Map falls through to Global:
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Map), Some("save-state"));
        // shift-arrow pan aliases were removed; plain arrow still works:
        assert_eq!(km.lookup(&g(Left, false, false), Context::Map), Some("pan-map -1 0"));
        // shift-arrow is no longer bound in map context:
        assert_eq!(km.lookup(&g(Left, false, true), Context::Map), None);
    }

    // ── HotkeyLayout tests ────────────────────────────────────────────────────

    #[test]
    fn hotkey_layout_default_direct_and_indirect() {
        let layout = HotkeyLayout::default();
        // Direct commands
        assert!(layout.is_direct_name("center-map"), "center-map should be direct");
        assert!(layout.is_direct_name("quit"), "quit should be direct");
        assert!(layout.is_direct_name("toggle-focus"), "toggle-focus should be direct");
        // Non-direct (dialog-only) commands
        assert!(!layout.is_direct_name("tidy-map"), "tidy-map should NOT be direct");
        assert!(!layout.is_direct_name("open-gallery"), "open-gallery should NOT be direct");
        // Groups
        assert_eq!(layout.groups.len(), 5, "default layout should have 5 groups");
        assert_eq!(layout.groups[0].0, "Layout", "first group title should be Layout");
    }

    #[test]
    fn hotkey_layout_resolve_custom_direct_and_unknown_name() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: Some(vec!["save-state".into(), "quit".into(), "not-a-command".into()]),
            group: vec![HotkeyGroupConfig { title: "T".into(), commands: vec!["tidy-map".into()] }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        // Specified direct commands are direct
        assert!(layout.is_direct_name("save-state"), "save-state should be direct");
        assert!(layout.is_direct_name("quit"), "quit should be direct");
        // center-map is NOT in custom direct list
        assert!(!layout.is_direct_name("center-map"), "center-map should NOT be direct with custom list");
        // Unknown command produces a warning
        assert!(!warnings.is_empty(), "unknown command in direct should produce warning");
        assert!(warnings.iter().any(|w| w.contains("not-a-command")), "warning should mention not-a-command");
    }

    #[test]
    fn hotkey_layout_resolve_unknown_group_command_dropped() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["tidy-map".into(), "totally-fake-cmd".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].1.len(), 1, "unknown command should be dropped from group");
        assert_eq!(layout.groups[0].1[0].1, "tidy-map");
        assert!(!warnings.is_empty(), "unknown group command should produce warning");
        assert!(warnings.iter().any(|w| w.contains("totally-fake-cmd")));
    }

    #[test]
    fn backtab_keyevent_maps_to_cycle_layout_reverse() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use crate::input::{key_to_action, Action};
        use crate::state::AppState;

        let mut state = AppState::default();
        // BackTab is typically delivered with no modifiers.
        let backtab = KeyEvent {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        // With no mid-word suggestions, the BackTab autocomplete intercept does not
        // apply, so BackTab falls through to its Global binding: cycle-layout reverse.
        state.focus = crate::state::Focus::Game;
        let action = key_to_action(&state, backtab);
        assert!(
            matches!(action, Action::CycleLayoutReverse),
            "BackTab in Game focus should produce CycleLayoutReverse, got {:?}",
            action
        );

        state.focus = crate::state::Focus::Map;
        let action_map = key_to_action(&state, backtab);
        assert!(
            matches!(action_map, Action::CycleLayoutReverse),
            "BackTab in Map focus should produce CycleLayoutReverse, got {:?}",
            action_map
        );
    }

    #[test]
    fn backtab_keyspec_label_is_shift_tab() {
        let spec = KeySpec { code: KeyCode::BackTab, ctrl: false, shift: false, alt: false };
        assert_eq!(spec.label(), "Shift+Tab");
    }

    #[test]
    fn shift_tab_parses_to_backtab() {
        let spec: KeySpec = "shift+tab".parse().unwrap();
        assert_eq!(spec.code, KeyCode::BackTab);
        // shift flag should be false (BackTab encodes the shift itself)
        assert!(!spec.shift);
    }

    #[test]
    fn backtab_token_parses_to_backtab() {
        let spec: KeySpec = "backtab".parse().unwrap();
        assert_eq!(spec.code, KeyCode::BackTab);
    }

    #[test]
    fn reset_game_default_key_is_f5() {
        // F5 is the default key
        let km = KeyMap::default();
        let f5 = KeySpec { code: KeyCode::F(5), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&f5, Context::Global), Some("reset-game"));
    }

    #[test]
    fn open_history_default_key_and_group() {
        // F4 is the default key.
        let km = KeyMap::default();
        let f4 = KeySpec { code: KeyCode::F(4), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&f4, Context::Global), Some("open-history"));
        // It appears in the Files hotkey group.
        let layout = HotkeyLayout::default();
        let files = layout.groups.iter().find(|(t, _)| t == "Files").expect("Files group");
        assert!(files.1.iter().any(|c| c.1 == "open-history"), "open-history in Files group");
    }

    #[test]
    fn reset_game_in_files_dialog_group() {
        let layout = HotkeyLayout::default();
        let files_group = layout.groups.iter().find(|(title, _)| title == "Files");
        assert!(files_group.is_some(), "Files group should exist");
        let (_, cmds) = files_group.unwrap();
        assert!(cmds.iter().any(|c| c.1 == "reset-game"), "reset-game should be in Files group");
    }

    #[test]
    fn toggle_inventory_key_is_v() {
        let km = KeyMap::default();
        let spec = KeySpec { code: KeyCode::Char('v'), ctrl: false, shift: false, alt: false };
        let cmd = km.lookup(&spec, Context::Global);
        assert_eq!(cmd, Some("toggle-inventory"), "v should be bound to toggle-inventory");
    }

    #[test]
    fn toggle_inventory_in_view_group() {
        let layout = HotkeyLayout::default();
        let view_group = layout.groups.iter().find(|(title, _)| title == "View");
        assert!(view_group.is_some(), "View group should exist");
        let (_, cmds) = view_group.unwrap();
        assert!(cmds.iter().any(|c| c.1 == "toggle-inventory"), "toggle-inventory should be in View group");
    }

    #[test]
    fn apply_action_toggle_inventory_flips_bool() {
        use mapper::mapper::Mapper;
        use crate::input::apply_action;
        use crate::state::AppState;
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        assert!(!state.show_inventory);
        apply_action(Action::ToggleInventory, &mut state, &mut mapper);
        assert!(state.show_inventory);
        apply_action(Action::ToggleInventory, &mut state, &mut mapper);
        assert!(!state.show_inventory);
    }

    // ── Item 2: ZoomReset command wiring ─────────────────────────────────────

    #[test]
    fn zoom_reset_bound_to_zero_key() {
        let km = KeyMap::default();
        let zero = KeySpec { code: KeyCode::Char('0'), ctrl: false, shift: false, alt: false };
        assert_eq!(
            km.lookup(&zero, Context::Map),
            Some("zoom-map reset"),
            "'0' key must be bound to zoom-map reset in Map context"
        );
    }

    #[test]
    fn zoom_reset_is_in_direct_set() {
        let layout = HotkeyLayout::default();
        assert!(
            layout.is_direct_name("zoom-map reset"),
            "zoom-map reset must be in the direct set (accessible without the hotkey dialog)"
        );
    }

    #[test]
    fn zoom_reset_action_resets_level() {
        use mapper::mapper::Mapper;
        use crate::input::apply_action;
        use crate::state::{AppState, Zoom};
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        // Zoom all the way out first.
        for _ in 0..8 {
            apply_action(Action::ZoomOut, &mut state, &mut mapper);
        }
        assert!(matches!(state.zoom, Zoom::Overview));
        // Reset
        apply_action(Action::ZoomReset, &mut state, &mut mapper);
        assert_eq!(state.zoom_level, 7, "ZoomReset must restore zoom_level to 7");
        assert!(matches!(state.zoom, Zoom::Boxes), "ZoomReset must restore Zoom::Boxes");
    }

    /// Every default binding for a DIRECT command (excluding save-state and
    /// restore-state, which intentionally use Ctrl) must have ctrl=false and
    /// shift=false. This invariant ensures that direct commands are reachable
    /// with plain (unmodified) keystrokes.
    #[test]
    fn direct_default_bindings_have_no_modifiers() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();

        // Commands excluded from this invariant by design.
        let excluded = ["save-state", "restore-state"];

        let mut violations: Vec<String> = Vec::new();
        for (spec, cmd_str, _ctx) in &km.bindings {
            let first = cmd_str.split_whitespace().next().unwrap_or("");
            if excluded.contains(&first) {
                continue;
            }
            if !layout.is_direct_name(cmd_str) {
                continue;
            }
            if spec.ctrl || spec.shift {
                violations.push(format!(
                    "{} ({}): ctrl={} shift={}",
                    cmd_str,
                    spec.label(),
                    spec.ctrl,
                    spec.shift,
                ));
            }
        }

        assert!(
            violations.is_empty(),
            "direct bindings with modifier keys found:\n  {}",
            violations.join("\n  ")
        );
    }

    #[test]
    fn hotkey_defaults_use_registry_names() {
        // DEFAULT_DIRECT_COMMANDS are full command-strings; validate the first token.
        for cmd in DEFAULT_DIRECT_COMMANDS {
            let name = cmd.split_whitespace().next().unwrap_or("");
            assert!(crate::slash::find_command(name).is_some(), "direct command not in registry: {cmd}");
        }
        // DEFAULT_GROUPS hold (letter, full command-string) pairs; validate the first token.
        for (_title, entries) in DEFAULT_GROUPS {
            for (_letter, cmd) in *entries {
                let name = cmd.split_whitespace().next().unwrap_or("");
                assert!(crate::slash::find_command(name).is_some(), "group command not in registry: {cmd}");
            }
        }
    }

    #[test]
    fn keymap_default_and_resolve_command_strings() {
        use crate::config::KeymapConfig;
        let km = KeyMap::default();
        let plus: KeySpec = "+".parse().unwrap();
        assert_eq!(km.lookup(&plus, Context::Map), Some("zoom-map in"));
        let left: KeySpec = "left".parse().unwrap();
        assert_eq!(km.lookup(&left, Context::Map), Some("pan-map -1 0"));

        // use_defaults=false → empty base; only the user binding exists.
        let mut cfg = KeymapConfig { use_defaults: false, ..Default::default() };
        cfg.global.insert("ctrl+s".into(), "save-state".into());
        let (km2, warns) = KeyMap::resolve(&cfg);
        let cs: KeySpec = "ctrl+s".parse().unwrap();
        assert_eq!(km2.lookup(&cs, Context::Global), Some("save-state"));
        assert!(km2.lookup(&plus, Context::Map).is_none(), "no defaults loaded");
        assert!(warns.is_empty());

        // Unknown command name → skip + warn.
        let mut cfg3 = KeymapConfig::default();
        cfg3.global.insert("ctrl+z".into(), "frobnicate".into());
        let (_km3, warns3) = KeyMap::resolve(&cfg3);
        assert!(warns3.iter().any(|w| w.contains("frobnicate")));
    }

    // ── SQ-0202: authored leader letters ─────────────────────────────────────

    #[test]
    fn default_leader_letters_are_unique() {
        let layout = HotkeyLayout::default();
        let letters: Vec<char> = layout.groups.iter()
            .flat_map(|(_, cmds)| cmds.iter().map(|(letter, _)| *letter))
            .collect();
        let unique: std::collections::HashSet<char> = letters.iter().copied().collect();
        assert_eq!(letters.len(), unique.len(), "leader letters must be unique");
        assert_eq!(letters.len(), 24, "expected 24 authored leader letters");
    }

    #[test]
    fn leader_command_resolves_authored_letter() {
        let layout = HotkeyLayout::default();
        assert_eq!(layout.leader_command('t'), Some("tidy-map"));
        assert_eq!(layout.leader_command('c'), Some("cycle-layer next"));
        assert_eq!(layout.leader_command('z'), None);
    }

    #[test]
    fn resolve_parses_letter_prefixed_config_entry() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["t tidy-map".into()],
            }],
        };
        let (layout, _warnings) = HotkeyLayout::resolve(&cfg);
        assert_eq!(layout.groups.len(), 1);
        assert!(layout.groups[0].1.iter().any(|(letter, cmd)| *letter == 't' && cmd == "tidy-map"));
    }

    #[test]
    fn resolve_autoassigns_when_letter_omitted() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["tidy-map".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert!(warnings.is_empty(), "auto-assigning a free letter should not warn: {warnings:?}");
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].1.len(), 1);
        assert_eq!(layout.groups[0].1[0].1, "tidy-map");
    }

    #[test]
    fn resolve_warns_on_duplicate_letter() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["t tidy-map".into(), "t animate-tidy".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert!(!warnings.is_empty(), "duplicate letter should produce a warning");
        assert_eq!(layout.leader_command('t'), Some("tidy-map"), "first occurrence wins");
    }
}
