use std::io;
use std::path::{Path, PathBuf};

/// Write `lines` to a file, resolving the destination as follows:
/// - `dest=None` → `exports_dir/transcript-<stamp>.txt`
/// - `dest=Some(name)` with no `/` → `exports_dir/name`
/// - `dest=Some(path)` containing `/` → that path as-is
///
/// Parent directories are created if missing.
/// Returns the path that was written.
pub fn export_transcript(
    lines: &[String],
    dest: Option<&str>,
    exports_dir: &Path,
    stamp: &str,
) -> io::Result<PathBuf> {
    let target: PathBuf = match dest {
        None => exports_dir.join(format!("transcript-{}.txt", stamp)),
        Some(name) if !name.contains('/') => exports_dir.join(name),
        Some(path) => PathBuf::from(path),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = format!("{}\n", lines.join("\n"));
    std::fs::write(&target, content)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_transcript_resolves_dest_and_writes() {
        let dir = std::env::temp_dir().join(format!("babelmap-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lines = vec!["a".to_string(), "b".to_string()];
        let p1 = export_transcript(&lines, None, &dir, "20260624-120000").unwrap();
        assert_eq!(p1, dir.join("transcript-20260624-120000.txt"));
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), "a\nb\n");
        let p2 = export_transcript(&lines, Some("out.txt"), &dir, "x").unwrap();
        assert_eq!(p2, dir.join("out.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
