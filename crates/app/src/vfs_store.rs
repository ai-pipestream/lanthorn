//! Per-story disk sidecar for the Glulx Glk file VFS (SQ-0278).
//! Path + filesystem only: the bytes are already the gvm sidecar blob
//! (`session.vfs_bytes()`, encoded by `gvm::glk::encode_files`), so this
//! module does not touch the wire format. Mirrors `aux_store` for the
//! Z-machine aux table.

use std::path::{Path, PathBuf};

/// `<save_dir>/<sanitized-ifid>.gvfs`. Uses the same defensive ifid
/// sanitization as `aux_store::aux_path` (`[A-Za-z0-9_-]`, no `.`, so `..`
/// cannot survive) with no separators.
pub fn vfs_path(save_dir: &Path, ifid: &str) -> PathBuf {
    let safe: String = ifid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { c } else { '_' })
        .collect();
    let stem = if safe.is_empty() { "game".to_string() } else { safe };
    save_dir.join(format!("{stem}.gvfs"))
}

/// Read the per-game VFS sidecar (empty bytes if absent or unreadable).
pub fn read_vfs(save_dir: &Path, ifid: &str) -> Vec<u8> {
    std::fs::read(vfs_path(save_dir, ifid)).unwrap_or_default()
}

/// Write the per-game VFS sidecar (creating `save_dir` if needed).
pub fn write_vfs(save_dir: &Path, ifid: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(save_dir)?;
    std::fs::write(vfs_path(save_dir, ifid), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfs_path_sanitizes_and_stays_in_dir() {
        let dir = Path::new("/tmp/saves");
        let p = vfs_path(dir, "../../etc/ZCODE-1-840726");
        assert_eq!(p.parent(), Some(dir), "no path escape");
        let fname = p.file_name().unwrap().to_string_lossy();
        assert!(fname.ends_with(".gvfs"));
        assert!(!fname.contains('/') && !fname.contains("..") && !fname.contains('\\'));
    }

    #[test]
    fn empty_ifid_falls_back_to_game_stem() {
        assert_eq!(vfs_path(Path::new("/tmp"), "").file_name().unwrap(), "game.gvfs");
    }

    #[test]
    fn round_trips_through_temp_dir() {
        let dir = std::env::temp_dir().join(format!("babelmap-vfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ifid = "ZCODE-1-840726-ABCD";
        assert!(read_vfs(&dir, ifid).is_empty(), "absent file → empty");
        let blob = vec![0xDE, 0xAD, 0xBE, 0xEF];
        write_vfs(&dir, ifid, &blob).unwrap();
        assert_eq!(read_vfs(&dir, ifid), blob);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
