//! The two flags every CLI answers the same way.
//!
//! `--help` and `--version` are handled before anything else is set up: no
//! story is loaded, no terminal is touched, and the exit is a plain return so a
//! `--help` in a pipeline stays clean.
//!
//! The name and version are passed in rather than read here, because
//! `env!("CARGO_PKG_NAME")` and `buildinfo::LONG` must expand in the *binary's*
//! crate — a copy baked into this one would report `cli-host` for all three.

/// Handle `--help`/`-h` and `--version`/`-V` if present.
///
/// Returns `true` when one was handled and the caller should return from `main`
/// immediately. Help wins over version when both are given, matching the order
/// the CLIs checked them in before.
pub fn handled_common_flags(argv: &[String], help: &str, name: &str, version: &str) -> bool {
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print!("{help}");
        return true;
    }
    if argv.iter().any(|a| a == "--version" || a == "-V") {
        println!("{name} {version}");
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
