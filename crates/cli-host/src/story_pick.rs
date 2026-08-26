//! Resolving `--story <n|name>` — which story to take off a volume that holds
//! several.
//!
//! One rule, in one place, because two front-ends offer the flag: `zvm-cli` has
//! had it since SQ-0834, and lanthorn gained it in SQ-1078 so that a headless
//! instrument can reach any game on a compilation disc instead of only whichever
//! one the format's tiebreak prefers. A flag spelled the same in both and
//! *matching* differently would be its own defect — `--story arthur` finding a
//! game at the prompt and nothing in the TUI is exactly the kind of disagreement
//! `disk_set` and `titles` already exist to prevent.
//!
//! Deliberately not here: what a story IS, or how a volume is read. This module
//! is given rows that a caller has already enumerated and returns which one was
//! meant.

/// One story a volume offers, as the chooser needs to see it.
pub struct Row {
    /// The name the medium stores it under, INCLUDING its directory on a format
    /// that has them (`InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA`).
    pub name: String,
    /// The canonical title, when the build's release and serial are in the
    /// bundled table. Matched as well as `name` because the menu prints it: a
    /// list showing `Zork I: The Great Underground Empire` where `--story
    /// "zork i"` found nothing would make the menu a liar (SQ-0884).
    pub title: Option<String>,
    /// The whole line a menu prints for this row, caller-formatted.
    pub label: String,
}

/// The numbered list a player picks from.
pub fn menu(rows: &[Row]) -> String {
    let mut s = String::new();
    for (i, r) in rows.iter().enumerate() {
        s.push_str(&format!("  {}) {}\n", i + 1, r.label));
    }
    s
}

/// Resolve what `--story` asked for: a 1-based number, or a name to match
/// (case-insensitive, and a substring is enough as long as it picks out one
/// story).
///
/// `subject` names what was searched, for the error text — "this disk" at a
/// prompt pointed at a floppy, "this library" for a directory of story files.
pub fn find(rows: &[Row], want: &str, subject: &str) -> Result<usize, String> {
    let want = want.trim();
    if let Ok(n) = want.parse::<usize>() {
        if (1..=rows.len()).contains(&n) {
            return Ok(n - 1);
        }
        let last = rows.len();
        return Err(format!("no story {n} on {subject} — pick 1 to {last}:\n{}", menu(rows)));
    }
    let lower = want.to_ascii_lowercase();
    let hits: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            r.name.to_ascii_lowercase().contains(&lower)
                || r.title.as_ref().is_some_and(|t| t.to_ascii_lowercase().contains(&lower))
        })
        .collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => Err(format!("no story on {subject} is named '{want}':\n{}", menu(rows))),
        _ => Err(format!("'{want}' matches more than one story on {subject}:\n{}", menu(rows))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Row> {
        vec![
            Row {
                name: "InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA".into(),
                title: Some("Arthur: The Quest for Excalibur".into()),
                label: "Arthur: The Quest for Excalibur  (v6 r54 s890606)".into(),
            },
            Row {
                name: "InfocomMasterpieces/ZORK ZERO/STORY.DATA".into(),
                title: Some("Zork Zero: The Revenge of Megaboz".into()),
                label: "Zork Zero: The Revenge of Megaboz  (v6 r296 s881019)".into(),
            },
            Row { name: "PLANETFALL".into(), title: None, label: "PLANETFALL  (v3 r39 s880501)".into() },
        ]
    }

    #[test]
    fn a_number_picks_that_row_and_out_of_range_says_the_range() {
        assert_eq!(find(&rows(), "2", "this disk"), Ok(1));
        let err = find(&rows(), "9", "this disk").unwrap_err();
        assert!(err.starts_with("no story 9 on this disk — pick 1 to 3:"), "{err}");
        // …and the menu comes with it, so the next attempt is informed.
        assert!(err.contains("3) PLANETFALL"), "{err}");
    }

    #[test]
    fn a_name_matches_the_stored_name_or_the_title_case_insensitively() {
        // The title the menu shows, which is not the name on the volume.
        assert_eq!(find(&rows(), "zork zero", "this disk"), Ok(1));
        // The stored name, for a build the title table does not carry.
        assert_eq!(find(&rows(), "planetfall", "this disk"), Ok(2));
        // A directory component is part of the stored name and matches too —
        // the spelling that cost SQ-1068 an afternoon.
        assert_eq!(find(&rows(), "ARTHUR FOLDER", "this disk"), Ok(0));
    }

    #[test]
    fn an_ambiguous_name_refuses_rather_than_guessing() {
        let err = find(&rows(), "story.data", "this disk").unwrap_err();
        assert!(err.starts_with("'story.data' matches more than one story on this disk:"), "{err}");
    }

    #[test]
    fn no_match_says_so_and_the_subject_is_the_callers() {
        let err = find(&rows(), "trinity", "this library").unwrap_err();
        assert!(err.starts_with("no story on this library is named 'trinity':"), "{err}");
    }
}
