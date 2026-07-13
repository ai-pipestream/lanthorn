use std::io;
use std::path::{Path, PathBuf};

/// Resolve an export destination the SQ-0284 way: no dest → `game_dir/<default_name>`;
/// a bare name (no separator) → `game_dir/<name>` with the default's extension appended
/// if the name has none; a value containing a path separator (or absolute) → verbatim.
pub fn resolve_export_path(dest: Option<&str>, game_dir: &Path, default_name: &str) -> PathBuf {
    match dest.map(str::trim) {
        None | Some("") => game_dir.join(default_name),
        Some(d) if d.contains('/') || d.contains('\\') => PathBuf::from(d),
        Some(d) => {
            let name = if Path::new(d).extension().is_some() {
                d.to_string()
            } else if let Some(ext) = Path::new(default_name).extension().and_then(|e| e.to_str()) {
                format!("{d}.{ext}")
            } else {
                d.to_string()
            };
            game_dir.join(name)
        }
    }
}

/// Write `lines` to a file, resolving the destination via [`resolve_export_path`]
/// against `game_dir` with the default name `transcript.txt`.
///
/// Parent directories are created if missing.
/// Returns the path that was written.
pub fn export_transcript(
    lines: &[String],
    dest: Option<&str>,
    game_dir: &Path,
) -> io::Result<PathBuf> {
    let target = resolve_export_path(dest, game_dir, "transcript.txt");
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
    use std::path::{Path, PathBuf};

    #[test]
    fn resolve_none_uses_default_name_in_game_dir() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(None, gd, "map.svg"), PathBuf::from("/data/Zork1.z5/map.svg"));
    }
    #[test]
    fn resolve_bare_name_appends_default_ext_when_missing() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(Some("before"), gd, "map.svg"), PathBuf::from("/data/Zork1.z5/before.svg"));
        // an explicit extension on the bare name is preserved
        assert_eq!(resolve_export_path(Some("before.dot"), gd, "map.svg"), PathBuf::from("/data/Zork1.z5/before.dot"));
    }
    #[test]
    fn resolve_path_bearing_value_is_verbatim() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(Some("/tmp/x.svg"), gd, "map.svg"), PathBuf::from("/tmp/x.svg"));
    }

    #[test]
    fn export_transcript_resolves_dest_and_writes() {
        let dir = std::env::temp_dir().join(format!("babelmap-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lines = vec!["a".to_string(), "b".to_string()];
        let p1 = export_transcript(&lines, None, &dir).unwrap();
        assert_eq!(p1, dir.join("transcript.txt"));
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), "a\nb\n");
        let p2 = export_transcript(&lines, Some("out.txt"), &dir).unwrap();
        assert_eq!(p2, dir.join("out.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
