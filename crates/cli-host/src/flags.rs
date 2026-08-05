//! The two flags every CLI answers the same way.
//!
//! `--help` and `--version` are handled before anything else is set up: no
//! story is loaded, no terminal is touched, and the exit is a plain return so a
//! `--help` in a pipeline stays clean.
//!
//! The name and version are passed in rather than read here, because
//! `env!("CARGO_PKG_NAME")` and `buildinfo::LONG` must expand in the *binary's*
//! crate — a copy baked into this one would report `cli-host` for all three.

/// The arguments before the first `--`, which ends option parsing.
///
/// `args::scan` already honours `--` — everything after it is positional even
/// when it is spelled like a flag — and the pre-scans that run before `scan`
/// (this module's [`handled_common_flags`], `mode::plain_from`) must agree, or
/// `zvm-cli -- --help` prints the help instead of opening a story literally
/// named `--help`.
pub fn before_double_dash(argv: &[String]) -> impl Iterator<Item = &String> {
    argv.iter().take_while(|a| a.as_str() != "--")
}

/// Handle `--help`/`-h` and `--version`/`-V` if present.
///
/// Returns `true` when one was handled and the caller should return from `main`
/// immediately. Help wins over version when both are given, matching the order
/// the CLIs checked them in before. Stops at `--`: past it these spellings are
/// story paths, not flags.
pub fn handled_common_flags(argv: &[String], help: &str, name: &str, version: &str) -> bool {
    if before_double_dash(argv).any(|a| a == "--help" || a == "-h") {
        print!("{help}");
        return true;
    }
    if before_double_dash(argv).any(|a| a == "--version" || a == "-V") {
        println!("{name} {version}");
        return true;
    }
    false
}

/// Exit code for a command-line usage error.
///
/// 2 is the shell convention for "you invoked this wrongly" (and what
/// getopt-based tools use), distinct from 1, which these binaries already use
/// for a story that could not be read or parsed. Worth keeping apart: one means
/// fix your command, the other means fix your file.
pub const EXIT_USAGE: i32 = 2;

/// Report a usage error, print the help, and exit.
///
/// Showing the help matters more than it looks. A mistyped flag used to be
/// silently ignored by `zvm-cli` and `gvm-cli` — `--no-statu` did nothing at all
/// and exited 0 — so the failure mode was a player concluding the feature was
/// broken rather than the spelling. Naming the offending argument and then
/// listing what is actually accepted answers both questions at once.
pub fn usage_error(name: &str, message: &str, help: &str) -> ! {
    eprintln!("{name}: {message}");
    eprintln!();
    eprint!("{help}");
    std::process::exit(EXIT_USAGE)
}

/// Does `arg` look like a flag rather than a positional argument?
///
/// A bare `-` is conventionally stdin, not a flag, so it is left to the caller.
/// Everything else starting with `-` is a flag — including single-dash forms,
/// which `zvm-cli` previously fell through to its story-path arm, so `-x` was
/// taken as a filename and reported as a missing file.
pub fn looks_like_flag(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_dash_is_not_a_flag() {
        assert!(!looks_like_flag("-"), "bare - is conventionally stdin");
        assert!(!looks_like_flag(""));
        assert!(!looks_like_flag("story.z5"));
        assert!(looks_like_flag("-x"), "single-dash forms are flags, not filenames");
        assert!(looks_like_flag("--plain"));
        assert!(looks_like_flag("--"), "-- is not a path either");
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn recognises_both_spellings_of_each_flag() {
        for a in ["--help", "-h", "--version", "-V"] {
            assert!(
                handled_common_flags(&argv(&["prog", a]), "", "", ""),
                "{a} should be handled"
            );
        }
    }

    #[test]
    fn ordinary_invocation_is_not_handled() {
        assert!(!handled_common_flags(&argv(&["prog", "story.z5"]), "", "", ""));
        assert!(!handled_common_flags(&argv(&["prog"]), "", "", ""));
        // A flag that merely contains one of the names is not it.
        assert!(!handled_common_flags(&argv(&["prog", "--help-me"]), "", "", ""));
        // Nor is a story file that happens to be spelled like one.
        assert!(!handled_common_flags(&argv(&["prog", "-help"]), "", "", ""));
    }

    #[test]
    fn a_flag_anywhere_in_the_line_counts() {
        assert!(handled_common_flags(&argv(&["prog", "story.z5", "--help"]), "", "", ""));
    }

    /// SQ-0636. `args::scan` treats everything after `--` as positional, so the
    /// pre-scan must too — `zvm-cli -- --help` is a request to open a story
    /// literally named `--help`, not to print the help.
    #[test]
    fn double_dash_ends_the_pre_scan() {
        for a in ["--help", "-h", "--version", "-V"] {
            assert!(
                !handled_common_flags(&argv(&["prog", "--", a]), "", "", ""),
                "{a} after -- is a story path, not a flag"
            );
        }
        // Before the `--` it is still the flag.
        assert!(handled_common_flags(&argv(&["prog", "--help", "--", "x"]), "", "", ""));
    }
}
