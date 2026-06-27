//! Shared most-recently-used custom-color list for the style editor, persisted
//! to a sidecar so it survives restarts.
use std::path::{Path, PathBuf};

const CAP: usize = 16;
const GLYPH_CAP: usize = 32;
fn sidecar(dir: &Path) -> PathBuf { dir.join("style_editor.toml") }

/// Read the sidecar TOML and return (recent_colors_vec, recent_glyphs_vec).
/// Missing keys return empty vecs; a missing file returns ([], []).
fn read_sidecar(dir: &Path) -> (Vec<String>, Vec<String>) {
    let text = match std::fs::read_to_string(sidecar(dir)) {
        Ok(t) => t,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let table = match text.parse::<toml::Table>() {
        Ok(t) => t,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let colors = table.get("recent_colors")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let glyphs = table.get("recent_glyphs")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    (colors, glyphs)
}

/// Write both keys to the sidecar in one atomic write.
fn write_sidecar(dir: &Path, colors: &[String], glyphs: &[String]) -> std::io::Result<()> {
    let color_arr = colors.iter().filter(|s| is_valid_color_token(s))
        .map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", ");
    let glyph_arr = glyphs.iter()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>().join(", ");
    std::fs::write(
        sidecar(dir),
        format!("recent_colors = [{color_arr}]\nrecent_glyphs = [{glyph_arr}]\n"),
    )
}

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
    if v.iter().any(|x| x == value) {
        return; // already present: keep its position stable
    }
    if v.len() >= CAP {
        v.remove(0); // full: evict the oldest (front) entry
    }
    v.push(value.to_string()); // new color goes to the end
}

pub fn load_mru(dir: &Path) -> Vec<String> {
    read_sidecar(dir).0
}

pub fn save_mru(dir: &Path, v: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let (_, glyphs) = read_sidecar(dir);
    write_sidecar(dir, v, &glyphs)
}

pub fn push_glyph_mru(v: &mut Vec<String>, value: &str) {
    v.retain(|x| x != value);
    v.insert(0, value.to_string());
    v.truncate(GLYPH_CAP);
}

pub fn load_glyph_mru(dir: &Path) -> Vec<String> {
    read_sidecar(dir).1
}

pub fn save_glyph_mru(dir: &Path, v: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let (colors, _) = read_sidecar(dir);
    write_sidecar(dir, &colors, v)
}

pub fn is_valid_glyph(s: &str) -> bool {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => !c.is_whitespace() && !c.is_control()
            && unicode_width::UnicodeWidthChar::width(c) == Some(1),
        _ => false,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_mru_keeps_position_of_existing() {
        let mut v = vec!["#aaaaaa".to_string(), "#bbbbbb".to_string(), "#cccccc".to_string()];
        push_mru(&mut v, "#bbbbbb"); // re-use a NON-front entry
        assert_eq!(
            v,
            vec!["#aaaaaa".to_string(), "#bbbbbb".to_string(), "#cccccc".to_string()],
            "re-using an existing color must not move it to the front (would fail under move-to-front)"
        );
    }

    #[test]
    fn push_mru_appends_new_to_end() {
        let mut v = vec!["#aaaaaa".to_string()];
        push_mru(&mut v, "#bbbbbb");
        assert_eq!(v, vec!["#aaaaaa".to_string(), "#bbbbbb".to_string()],
            "a new color must be appended at the end, not inserted at the front");
    }

    #[test]
    fn push_mru_evicts_oldest_when_full() {
        let mut v: Vec<String> = (0..CAP).map(|i| format!("#0000{:02x}", i)).collect();
        let oldest = v[0].clone();
        let newest = "#ffffff".to_string();
        push_mru(&mut v, &newest);
        assert_eq!(v.len(), CAP, "length stays capped");
        assert!(!v.contains(&oldest), "the oldest (front) entry is evicted when full");
        assert_eq!(v.last().unwrap(), &newest, "the new color lands at the end");
    }

    #[test]
    fn push_dedups_caps_16_newest_last() {
        let mut v = Vec::new();
        for i in 0..20 { push_mru(&mut v, &format!("#{:06x}", i)); }
        assert_eq!(v.len(), 16);
        assert_eq!(v[15], "#000013"); // last pushed is at the end
        push_mru(&mut v, "#000013"); // existing → stays in place, no dup
        assert_eq!(v.iter().filter(|x| *x == "#000013").count(), 1);
        assert_eq!(v[15], "#000013");
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

    #[test]
    fn glyph_mru_caps_32_dedups_newest_first() {
        let mut v = Vec::new();
        for i in 0..40u32 { push_glyph_mru(&mut v, &char::from_u32(0x2500 + i).unwrap().to_string()); }
        assert_eq!(v.len(), 32);
        let item = v[5].clone(); // clone before mutable borrow
        push_glyph_mru(&mut v, &item); // existing → front, no dup
        assert_eq!(v[0], item);
        assert_eq!(v.iter().filter(|x| **x == item).count(), 1);
    }

    #[test]
    fn is_valid_glyph_rejects_multibyte_width_and_empty() {
        assert!(is_valid_glyph("═"));
        assert!(is_valid_glyph("█"));
        assert!(!is_valid_glyph(""), "empty rejected");
        assert!(!is_valid_glyph("ab"), "more than one char rejected");
        assert!(!is_valid_glyph("世"), "double-width rejected");
        assert!(!is_valid_glyph(" "), "space rejected");
    }

    #[test]
    fn glyph_mru_sidecar_round_trips() {
        let dir = std::env::temp_dir().join(format!("bm-gmru-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save_glyph_mru(&dir, &["═".into(), "║".into()]).unwrap();
        assert_eq!(load_glyph_mru(&dir), vec!["═".to_string(), "║".into()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidecar_cross_preserves_both_keys() {
        let dir = std::env::temp_dir().join(format!("bm-xpres-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save_mru(&dir, &["#aabbcc".into()]).unwrap();
        save_glyph_mru(&dir, &["═".into()]).unwrap();
        // saving glyphs must not clobber colors
        assert_eq!(load_mru(&dir), vec!["#aabbcc".to_string()]);
        assert_eq!(load_glyph_mru(&dir), vec!["═".to_string()]);
        // now overwrite colors, glyphs must survive
        save_mru(&dir, &["#112233".into()]).unwrap();
        assert_eq!(load_mru(&dir), vec!["#112233".to_string()]);
        assert_eq!(load_glyph_mru(&dir), vec!["═".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
