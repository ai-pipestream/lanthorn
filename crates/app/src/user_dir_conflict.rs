//! The startup question when two sets of files compete (SQ-1776).
//!
//! `~/.lanthorn` (legacy) and the platform folders can both hold real settings
//! or saves. [`crate::user_dirs::UserDirs::conflict`] detects it; [`resolve`]
//! here decides what a launch does about it, over injected streams so tests drive
//! it without a terminal. [`run_at_startup`] is the one function that reads the
//! real environment, and it runs BEFORE the config is loaded.
//!
//! Interactive (stdin and stdout both terminals): show both sets and ask which to
//! use, then offer to rename the other to `<name>.unused-YYYY-MM-DD` beside
//! itself so the question cannot recur. Nothing is ever deleted. Declining the
//! rename uses the chosen set for this launch only; the question returns next time.
//!
//! Non-interactive: never reads stdin; uses the platform folders and prints one
//! line to stderr saying how to resolve it.

use crate::user_dirs::{pin_choice, Choice, Conflict, Inputs, Summary, UserDirs};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// What [`resolve`] decided, and what it renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub choice: Choice,
    /// `(from, to)` for each folder renamed out of the way.
    pub renamed: Vec<(PathBuf, PathBuf)>,
}

fn when(t: Option<std::time::SystemTime>) -> String {
    match t.and_then(|t| jiff::Timestamp::try_from(t).ok()) {
        Some(ts) => format!(
            "config.toml modified {}",
            ts.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d %H:%M")
        ),
        None => "no config.toml".to_string(),
    }
}

fn describe(out: &mut dyn Write, n: usize, label: &str, s: &Summary, single: bool) {
    let _ = writeln!(out, "  {n}) {label}");
    if single {
        let _ = writeln!(out, "       folder: {}", s.dirs.config().display());
    } else {
        let _ = writeln!(out, "       config: {}", s.dirs.config().display());
        if s.dirs.data() != s.dirs.config() {
            let _ = writeln!(out, "       data:   {}", s.dirs.data().display());
        }
    }
    let _ = writeln!(
        out,
        "       {}; {} save file{}",
        when(s.config_modified),
        s.save_files,
        if s.save_files == 1 { "" } else { "s" }
    );
}

/// The folders of `set` that exist, outermost only (Windows keeps `logs` inside
/// the cache folder, and renaming the cache folder carries it along).
fn folders(set: &Summary, single: bool) -> Vec<PathBuf> {
    let d = &set.dirs;
    let all: Vec<&Path> = if single { vec![d.config()] } else { vec![d.config(), d.data(), d.cache(), d.logs()] };
    let mut v: Vec<PathBuf> = Vec::new();
    for p in all {
        if p.exists() && !v.iter().any(|q| q == p) {
            v.push(p.to_path_buf());
        }
    }
    let snapshot = v.clone();
    v.retain(|p| !snapshot.iter().any(|q| q != p && p.starts_with(q)));
    v
}

/// `<name>.unused-<date>` beside `from`, or with `-2`, `-3`... if that is taken.
fn unused_name(from: &Path, date: &str) -> PathBuf {
    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = format!("{name}.unused-{date}");
    let mut to = from.with_file_name(&base);
    let mut n = 2;
    while to.symlink_metadata().is_ok() {
        to = from.with_file_name(format!("{base}-{n}"));
        n += 1;
    }
    to
}

/// What renaming `set` would do, as `(from, to)` pairs.
fn rename_plan(set: &Summary, single: bool, date: &str) -> Vec<(PathBuf, PathBuf)> {
    folders(set, single)
        .into_iter()
        .map(|f| {
            let t = unused_name(&f, date);
            (f, t)
        })
        .collect()
}

/// Ask the person (over `input`/`out`) or, when not `interactive`, take the
/// platform folders and say so on `err`. `date` is `YYYY-MM-DD`, for the rename.
pub fn resolve(
    conflict: &Conflict,
    interactive: bool,
    date: &str,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Decision {
    if !interactive {
        let _ = writeln!(
            err,
            "lanthorn: both {} and {} hold your files; using {} (rename one of them, or run `lanthorn --migrate-user-dir`, to settle it)",
            conflict.legacy.dirs.config().display(),
            conflict.platform.dirs.config().display(),
            conflict.platform.dirs.config().display(),
        );
        return Decision { choice: Choice::Platform, renamed: Vec::new() };
    }
    let _ = writeln!(out, "lanthorn found two sets of your files, and both hold settings or saves:");
    let _ = writeln!(out);
    describe(out, 1, "the older single folder (~/.lanthorn)", &conflict.legacy, true);
    describe(out, 2, "the standard platform folders", &conflict.platform, false);
    let _ = writeln!(out);
    let mut choice = None;
    for _ in 0..3 {
        let _ = write!(out, "Which should lanthorn use? [1/2] ");
        let _ = out.flush();
        let mut line = String::new();
        if input.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        match line.trim() {
            "1" => choice = Some(Choice::Legacy),
            "2" => choice = Some(Choice::Platform),
            _ => continue,
        }
        break;
    }
    let Some(choice) = choice else {
        let _ = writeln!(out, "\nNo answer; using the standard platform folders for this launch and nothing was changed.");
        return Decision { choice: Choice::Platform, renamed: Vec::new() };
    };
    let (other, other_single) = match choice {
        Choice::Legacy => (&conflict.platform, false),
        Choice::Platform => (&conflict.legacy, true),
    };
    let plan = rename_plan(other, other_single, date);
    if plan.is_empty() {
        return Decision { choice, renamed: Vec::new() };
    }
    let _ = writeln!(out, "\nTo stop this question coming back, the other set can be renamed (nothing is deleted):");
    for (from, to) in &plan {
        let _ = writeln!(out, "  {} -> {}", from.display(), to.display());
    }
    let _ = write!(out, "Rename it? [y/N] ");
    let _ = out.flush();
    let mut line = String::new();
    let _ = input.read_line(&mut line);
    if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        let _ = writeln!(out, "Not renamed; using your choice for this launch only. You will be asked again next time.");
        return Decision { choice, renamed: Vec::new() };
    }
    let mut renamed = Vec::new();
    for (from, to) in plan {
        match std::fs::rename(&from, &to) {
            Ok(()) => renamed.push((from, to)),
            Err(e) => {
                let _ = writeln!(err, "lanthorn: could not rename {}: {e}; left in place", from.display());
            }
        }
    }
    for (from, to) in &renamed {
        let _ = writeln!(out, "Renamed {} -> {}", from.display(), to.display());
    }
    Decision { choice, renamed }
}

/// Today's date, `YYYY-MM-DD`, local time.
fn today() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d").to_string()
}

/// The launch-time check against the real environment, streams and terminal.
/// Does nothing under an explicit `--user-dir`. Pins the answer for the process.
pub fn run_at_startup(explicit_user_dir: Option<&Path>) {
    let Some(conflict) = UserDirs::conflict(&Inputs::from_environment(explicit_user_dir)) else { return };
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let stdin = std::io::stdin();
    let d = resolve(&conflict, interactive, &today(), &mut stdin.lock(), &mut std::io::stdout(), &mut std::io::stderr());
    pin_choice(d.choice);
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use crate::scratch_dir;
    use crate::user_dirs::{Platform, LEGACY_DIR};
    use std::fs;

    /// Home with a real legacy and a real platform (Linux layout) set.
    fn both(tag: &str) -> (PathBuf, Inputs) {
        let home = scratch_dir(tag);
        fs::create_dir_all(home.join(".lanthorn/saves/g")).unwrap();
        fs::write(home.join(".lanthorn/saves/g/a.lanthorn"), "s").unwrap();
        fs::create_dir_all(home.join(".config/lanthorn")).unwrap();
        fs::write(home.join(".config/lanthorn/config.toml"), "x").unwrap();
        fs::create_dir_all(home.join(".local/share/lanthorn")).unwrap();
        let i = Inputs { home: Some(home.clone()), ..Inputs::empty(Platform::Linux) };
        (home, i)
    }

    fn go(c: &Conflict, interactive: bool, answers: &str) -> (Decision, String, String) {
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let d = resolve(c, interactive, "2026-10-12", &mut answers.as_bytes(), &mut o, &mut e);
        (d, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap())
    }

    /// A reader that fails the test if anything reads it.
    struct NoRead;
    impl std::io::Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("a non-interactive launch must never read stdin");
        }
    }

    #[test]
    fn interactive_pick_legacy_then_rename_platform_folders() {
        let (home, i) = both("conflict-pick-legacy");
        fs::create_dir_all(home.join(".cache/lanthorn")).unwrap();
        let c = UserDirs::conflict(&i).expect("both hold content");
        let (d, out, _) = go(&c, true, "1\ny\n");
        assert_eq!(d.choice, Choice::Legacy);
        assert!(out.contains("Which should lanthorn use? [1/2]") && out.contains("Rename it? [y/N]"), "{out}");
        assert!(out.contains("1 save file") && out.contains("config.toml modified"), "{out}");
        assert!(out.contains("no config.toml"), "legacy has none: {out}");
        assert!(!home.join(".config/lanthorn").exists());
        assert!(home.join(".config/lanthorn.unused-2026-10-12/config.toml").exists());
        assert!(home.join(".local/share/lanthorn.unused-2026-10-12").exists());
        assert!(home.join(".cache/lanthorn.unused-2026-10-12").exists());
        assert!(!home.join(".local/state/lanthorn.unused-2026-10-12").exists(), "missing folders are skipped");
        assert!(home.join(".lanthorn/saves/g/a.lanthorn").exists(), "the chosen set is untouched");
        assert_eq!(d.renamed.len(), 3);
        assert!(UserDirs::conflict(&i).is_none(), "the conflict cannot recur");
    }

    #[test]
    fn interactive_pick_platform_renames_legacy_and_suffixes_on_collision() {
        let (home, i) = both("conflict-pick-platform");
        fs::create_dir_all(home.join(".lanthorn.unused-2026-10-12")).unwrap();
        fs::create_dir_all(home.join(".lanthorn.unused-2026-10-12-2")).unwrap();
        let c = UserDirs::conflict(&i).unwrap();
        let (d, _, _) = go(&c, true, "2\nyes\n");
        assert_eq!(d.choice, Choice::Platform);
        assert_eq!(d.renamed, vec![(home.join(LEGACY_DIR), home.join(".lanthorn.unused-2026-10-12-3"))]);
        assert!(home.join(".lanthorn.unused-2026-10-12-3/saves/g/a.lanthorn").exists(), "nothing deleted");
        assert!(!home.join(".lanthorn").exists());
    }

    #[test]
    fn declining_the_rename_changes_nothing_and_the_question_returns() {
        let (home, i) = both("conflict-decline");
        let c = UserDirs::conflict(&i).unwrap();
        for no in ["n\n", "\n", ""] {
            let (d, out, _) = go(&c, true, &format!("1\n{no}"));
            assert_eq!(d.choice, Choice::Legacy);
            assert!(d.renamed.is_empty() && out.contains("asked again next time"), "{out}");
        }
        assert!(home.join(".lanthorn").exists() && home.join(".config/lanthorn/config.toml").exists());
        assert!(UserDirs::conflict(&i).is_some());
    }

    #[test]
    fn bad_answers_are_reasked_and_eof_falls_back_to_platform_untouched() {
        let (home, i) = both("conflict-eof");
        let c = UserDirs::conflict(&i).unwrap();
        let (d, out, _) = go(&c, true, "x\n3\n2\nn\n");
        assert_eq!(d.choice, Choice::Platform);
        assert_eq!(out.matches("Which should lanthorn use?").count(), 3, "{out}");
        let (d, out, _) = go(&c, true, "");
        assert_eq!(d, Decision { choice: Choice::Platform, renamed: vec![] });
        assert!(out.contains("No answer"), "{out}");
        assert!(home.join(".lanthorn").exists());
    }

    #[test]
    fn non_interactive_never_reads_stdin_and_warns_once_on_stderr() {
        let (home, i) = both("conflict-noninteractive");
        let c = UserDirs::conflict(&i).unwrap();
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let mut r = std::io::BufReader::new(NoRead);
        let d = resolve(&c, false, "2026-10-12", &mut r, &mut o, &mut e);
        assert_eq!(d, Decision { choice: Choice::Platform, renamed: vec![] });
        assert!(o.is_empty());
        let e = String::from_utf8(e).unwrap();
        assert_eq!(e.lines().count(), 1, "{e}");
        assert!(e.contains(&c.legacy.dirs.config().display().to_string()), "{e}");
        assert!(e.contains(&c.platform.dirs.config().display().to_string()), "{e}");
        assert!(e.contains("--migrate-user-dir"), "{e}");
        assert!(home.join(".lanthorn").exists(), "nothing renamed");
    }
}
