//! One argument scanner, three flag tables.
//!
//! The flags themselves genuinely differ — `zvm-cli` has `--volume` and
//! `-I`, `gvm-cli` has `--accel`, `scott-cli` has `--seed` — but the *rules*
//! for reading them do not, and each CLI had its own loop applying them. That
//! went the way these things go: `scott-cli` rejected unknown flags, `zvm-cli`
//! ended its match in `_ => {}` and `gvm-cli` scanned inline, so a mistyped
//! `--no-statu` did nothing at all and exited 0 (SQ-0614). `zvm-cli` also fell
//! through single-dash arguments to its story-path arm, so `-x` was reported as
//! a missing file rather than a bad option.
//!
//! So the loop lives here and each CLI declares only its table. Four rules, in
//! one place:
//!
//! - an option that takes a value consumes the next argument, and it is an error
//!   if there isn't one;
//! - anything else that looks like a flag is an error, named in the message;
//! - everything else is positional, in order;
//! - `--` ends option parsing, so a story file really called `-x` is reachable.

use crate::flags::looks_like_flag;

/// One accepted option. The first name is canonical — the one
/// [`Matches::has`] and [`Matches::value`] answer to — and the rest are aliases.
pub struct Opt {
    pub names: &'static [&'static str],
    /// Does it consume the following argument?
    pub value: bool,
}

impl Opt {
    /// A switch: present or absent.
    pub const fn flag(names: &'static [&'static str]) -> Self {
        Opt { names, value: false }
    }

    /// An option that takes the next argument as its value.
    pub const fn valued(names: &'static [&'static str]) -> Self {
        Opt { names, value: true }
    }

    fn canonical(&self) -> &'static str {
        self.names[0]
    }
}

/// What a scan found.
#[derive(Debug)]
pub struct Matches {
    flags: Vec<&'static str>,
    values: Vec<(&'static str, String)>,
    /// Non-option arguments, in the order given.
    pub positional: Vec<String>,
}

impl Matches {
    /// Was this option present? Ask by its canonical (first) name.
    pub fn has(&self, canonical: &str) -> bool {
        self.flags.contains(&canonical) || self.values.iter().any(|(n, _)| *n == canonical)
    }

    /// The value given for this option, if any. Last occurrence wins, so a
    /// repeated flag behaves the way a shell user expects.
    pub fn value(&self, canonical: &str) -> Option<&str> {
        self.values.iter().rev().find(|(n, _)| *n == canonical).map(|(_, v)| v.as_str())
    }

    /// The first positional argument — the story path, for all three CLIs.
    pub fn first_positional(&self) -> Option<&str> {
        self.positional.first().map(String::as_str)
    }
}

/// Scan `argv` (including argv[0], which is skipped) against `opts`.
///
/// The error is a bare sentence — no program name, no help — so the caller can
/// present it however it likes; all three hand it to
/// [`crate::flags::usage_error`].
pub fn scan(argv: &[String], opts: &[Opt]) -> Result<Matches, String> {
    let mut m = Matches { flags: Vec::new(), values: Vec::new(), positional: Vec::new() };
    let mut it = argv.iter().skip(1);
    let mut options_done = false;
    while let Some(arg) = it.next() {
        if options_done {
            m.positional.push(arg.clone());
            continue;
        }
        if arg == "--" {
            options_done = true;
            continue;
        }
        match opts.iter().find(|o| o.names.contains(&arg.as_str())) {
            Some(o) if o.value => match it.next() {
                Some(v) => m.values.push((o.canonical(), v.clone())),
                None => return Err(format!("{arg} needs a value")),
            },
            Some(o) => m.flags.push(o.canonical()),
            None if looks_like_flag(arg) => return Err(format!("unknown option: {arg}")),
            None => m.positional.push(arg.clone()),
        }
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTS: &[Opt] = &[
        Opt::flag(&["--screen-reader", "--plain"]),
        Opt::flag(&["--story-only", "--lower-only"]),
        Opt::valued(&["--data-dir"]),
        Opt::valued(&["--interpreter", "-I"]),
    ];

    fn scan_of(args: &[&str]) -> Result<Matches, String> {
        let v: Vec<String> =
            std::iter::once("prog").chain(args.iter().copied()).map(String::from).collect();
        scan(&v, OPTS)
    }

    #[test]
    fn flags_answer_to_their_canonical_name_whichever_alias_was_typed() {
        for alias in ["--screen-reader", "--plain"] {
            let m = scan_of(&[alias, "game.z5"]).unwrap();
            assert!(m.has("--screen-reader"), "{alias} should set --screen-reader");
            assert_eq!(m.first_positional(), Some("game.z5"));
        }
        assert!(scan_of(&["--lower-only"]).unwrap().has("--story-only"));
        assert!(!scan_of(&["game.z5"]).unwrap().has("--screen-reader"));
    }

    #[test]
    fn a_valued_option_swallows_its_value_and_not_the_story() {
        let m = scan_of(&["--data-dir", "/tmp/x", "game.z5"]).unwrap();
        assert_eq!(m.value("--data-dir"), Some("/tmp/x"));
        assert_eq!(m.first_positional(), Some("game.z5"), "the value is not the story path");
        // Aliases carry values too, under the canonical name.
        assert_eq!(scan_of(&["-I", "6"]).unwrap().value("--interpreter"), Some("6"));
        // Last occurrence wins.
        assert_eq!(scan_of(&["-I", "1", "-I", "6"]).unwrap().value("--interpreter"), Some("6"));
    }

    #[test]
    fn unknown_options_are_named_rather_than_ignored() {
        // The reported bug: this used to be silently dropped and exit 0.
        assert_eq!(scan_of(&["--no-statu", "g"]).unwrap_err(), "unknown option: --no-statu");
        // Single-dash forms too — `-x` used to be taken as a story path.
        assert_eq!(scan_of(&["-x"]).unwrap_err(), "unknown option: -x");
    }

    #[test]
    fn a_valued_option_at_the_end_is_an_error_not_a_silent_none() {
        assert_eq!(scan_of(&["game.z5", "--data-dir"]).unwrap_err(), "--data-dir needs a value");
    }

    #[test]
    fn a_bare_dash_is_positional_and_double_dash_ends_options() {
        assert_eq!(scan_of(&["-"]).unwrap().first_positional(), Some("-"), "stdin convention");
        // After `--`, a story file really called `-x` is reachable.
        let m = scan_of(&["--", "-x"]).unwrap();
        assert_eq!(m.first_positional(), Some("-x"));
        assert!(m.positional.len() == 1, "the -- itself is not positional");
    }

    #[test]
    fn positionals_keep_their_order_so_the_caller_can_reject_extras() {
        let m = scan_of(&["a.z5", "b.z5"]).unwrap();
        assert_eq!(m.positional, vec!["a.z5".to_string(), "b.z5".to_string()]);
    }
}
