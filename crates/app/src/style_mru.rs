//! Shared most-recently-used custom-color list for the style editor, persisted
//! to a sidecar so it survives restarts.
use std::path::{Path, PathBuf};

const CAP: usize = 16;
fn sidecar(dir: &Path) -> PathBuf { dir.join("style_editor.toml") }

/// The 16 ANSI names the swatch grid offers (must match colors::parse_color_value).
pub const ANSI_NAMES: &[&str] = &[
    "black","red","green","yellow","blue","magenta","cyan","white",
    "dark-gray","light-red","light-green","light-yellow","light-blue",
    "light-magenta","light-cyan","gray",
];

pub fn is_valid_color_token(s: &str) -> bool {
    if s == "default" || ANSI_NAMES.contains(&s) { return true; }
    match s.strip_prefix('#') {
        Some(hex) => hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

pub fn push_mru(v: &mut Vec<String>, value: &str) {
    v.retain(|x| x != value);
    v.insert(0, value.to_string());
    v.truncate(CAP);
}

pub fn load_mru(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(sidecar(dir)) else { return Vec::new() };
    text.parse::<toml::Table>().ok()
        .and_then(|t| t.get("recent_colors").and_then(|v| v.as_array()).map(|a|
            a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

pub fn save_mru(dir: &Path, v: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let arr = v.iter().filter(|s| is_valid_color_token(s))
        .map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", ");
    std::fs::write(sidecar(dir), format!("recent_colors = [{arr}]\n"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_dedups_caps_16_newest_first() {
        let mut v = Vec::new();
        for i in 0..20 { push_mru(&mut v, &format!("#{:06x}", i)); }
        assert_eq!(v.len(), 16);
        assert_eq!(v[0], "#000013"); // last pushed is first
        push_mru(&mut v, "#000013"); // existing → moves to front, no dup
        assert_eq!(v.iter().filter(|x| *x == "#000013").count(), 1);
        assert_eq!(v[0], "#000013");
    }

    #[test]
    fn valid_color_token_accepts_ansi_hex_default() {
        assert!(is_valid_color_token("yellow"));
        assert!(is_valid_color_token("#a1b2c3"));
        assert!(is_valid_color_token("default"));
        assert!(!is_valid_color_token("#xyz"));
        assert!(!is_valid_color_token("notacolor"));
    }

    #[test]
    fn is_valid_requires_hash_for_hex() {
        assert!(is_valid_color_token("#a1b2c3"));
        assert!(!is_valid_color_token("a1b2c3"), "bare hex without # is rejected");
        assert!(is_valid_color_token("yellow"));
        assert!(is_valid_color_token("default"));
    }

    #[test]
    fn mru_sidecar_round_trips() {
        let dir = std::env::temp_dir().join(format!("bm-mru-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save_mru(&dir, &["#112233".into(), "#445566".into()]).unwrap();
        assert_eq!(load_mru(&dir), vec!["#112233".to_string(), "#445566".into()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
