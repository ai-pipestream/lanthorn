//! A minimal single-line editable text buffer with a caret.
//!
//! The app's other text entries (map-editing prompts, the game input line) are
//! cursor-less append/pop buffers. `TextField` adds full caret editing — arrow
//! movement, Home/End, mid-string insert, Backspace/Delete — as a small reusable
//! widget core. All operations work on CHAR boundaries so multi-byte/UTF-8 input
//! stays valid, and the cursor is always clamped to `0..=char_len`.

/// A single-line editable buffer. `cursor` is a CHAR index in `0..=value.chars().count()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextField {
    pub value: String,
    /// Caret position as a char index (0 = before first char, len = after last).
    pub cursor: usize,
}

impl TextField {
    /// A field prefilled with `value`, caret at the end.
    pub fn new(value: impl Into<String>) -> TextField {
        let value = value.into();
        let cursor = value.chars().count();
        TextField { value, cursor }
    }

    /// Number of chars (not bytes) in the buffer.
    pub fn char_len(&self) -> usize {
        self.value.chars().count()
    }

    /// Byte offset of char index `idx` (== `value.len()` when `idx >= char_len`).
    fn byte_of(&self, idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(idx)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    /// Insert `c` at the caret and advance past it.
    pub fn insert(&mut self, c: char) {
        let at = self.byte_of(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the char before the caret (no-op at the start).
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_of(self.cursor - 1);
        let end = self.byte_of(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// Delete the char at the caret (no-op at the end).
    pub fn delete(&mut self) {
        if self.cursor >= self.char_len() {
            return;
        }
        let start = self.byte_of(self.cursor);
        let end = self.byte_of(self.cursor + 1);
        self.value.replace_range(start..end, "");
    }

    /// Move the caret one char left (clamped at 0).
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the caret one char right (clamped at end).
    pub fn right(&mut self) {
        if self.cursor < self.char_len() {
            self.cursor += 1;
        }
    }

    /// Move the caret to the start.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Move the caret to the end.
    pub fn end(&mut self) {
        self.cursor = self.char_len();
    }

    /// Replace the whole buffer; caret to the end when `cursor_end`, else to the start.
    pub fn set(&mut self, value: impl Into<String>, cursor_end: bool) {
        self.value = value.into();
        self.cursor = if cursor_end { self.char_len() } else { 0 };
    }

    /// The buffer as a string slice.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// True when the buffer holds no characters.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Empty the buffer and put the caret back at the start.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Take the buffer's contents, leaving it empty with the caret at the start.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    /// Insert `s` at the caret and advance past it.
    pub fn insert_str(&mut self, s: &str) {
        let at = self.byte_of(self.cursor);
        self.value.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    /// Delete from the start of the buffer up to (not including) the caret;
    /// the caret moves to 0. Readline's Ctrl+U.
    pub fn delete_to_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_of(self.cursor);
        self.value.replace_range(0..end, "");
        self.cursor = 0;
    }

    /// Delete from the caret to the end of the buffer; the caret is unchanged.
    /// Readline's Ctrl+K.
    pub fn delete_to_end(&mut self) {
        let start = self.byte_of(self.cursor);
        self.value.truncate(start);
    }

    /// Delete the word behind the caret, readline style: first eat any run of
    /// whitespace immediately before the caret, then eat the run of
    /// non-whitespace before that. No-op at the start. Readline's Ctrl+W.
    pub fn delete_prev_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.value.chars().collect();
        let mut start_idx = self.cursor;
        while start_idx > 0 && chars[start_idx - 1].is_whitespace() {
            start_idx -= 1;
        }
        while start_idx > 0 && !chars[start_idx - 1].is_whitespace() {
            start_idx -= 1;
        }
        let start = self.byte_of(start_idx);
        let end = self.byte_of(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor = start_idx;
    }

    /// Keep the first `n` CHARS, dropping the rest. The caret is clamped to the new end.
    ///
    /// Chars, not bytes: `String::truncate` panics on a non-char boundary, which a byte count
    /// derived from a multi-byte buffer can easily land on.
    pub fn truncate_chars(&mut self, n: usize) {
        let at = self.byte_of(n);
        self.value.truncate(at);
        self.cursor = self.cursor.min(self.char_len());
    }
}

impl From<&str> for TextField {
    fn from(value: &str) -> TextField {
        TextField::new(value)
    }
}

impl From<String> for TextField {
    fn from(value: String) -> TextField {
        TextField::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_places_caret_at_end() {
        let f = TextField::new("abc");
        assert_eq!(f.value, "abc");
        assert_eq!(f.cursor, 3);
        assert_eq!(TextField::new("").cursor, 0);
    }

    #[test]
    fn insert_at_caret_and_midstring() {
        let mut f = TextField::new("ac");
        f.cursor = 1;
        f.insert('b');
        assert_eq!(f.value, "abc");
        assert_eq!(f.cursor, 2);
    }

    #[test]
    fn backspace_and_delete_clamp_at_boundaries() {
        let mut f = TextField::new("abc");
        f.home();
        f.backspace(); // at start: no-op
        assert_eq!(f.value, "abc");
        assert_eq!(f.cursor, 0);
        f.delete(); // removes 'a'
        assert_eq!(f.value, "bc");
        assert_eq!(f.cursor, 0);
        f.end();
        f.delete(); // at end: no-op
        assert_eq!(f.value, "bc");
        f.backspace();
        assert_eq!(f.value, "b");
        assert_eq!(f.cursor, 1);
    }

    #[test]
    fn movement_clamps() {
        let mut f = TextField::new("ab");
        f.left();
        f.left();
        f.left(); // clamps at 0
        assert_eq!(f.cursor, 0);
        f.right();
        f.right();
        f.right(); // clamps at len
        assert_eq!(f.cursor, 2);
        f.home();
        assert_eq!(f.cursor, 0);
        f.end();
        assert_eq!(f.cursor, 2);
    }

    #[test]
    fn set_replaces_and_positions_caret() {
        let mut f = TextField::new("old");
        f.set("newer", true);
        assert_eq!(f.value, "newer");
        assert_eq!(f.cursor, 5);
        f.set("x", false);
        assert_eq!(f.value, "x");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn utf8_multibyte_is_char_safe() {
        // "café" — 'é' is 2 bytes; "日本" — each char is 3 bytes.
        let mut f = TextField::new("café");
        assert_eq!(f.char_len(), 4);
        assert_eq!(f.cursor, 4);
        f.backspace(); // removes 'é' cleanly, not a partial byte
        assert_eq!(f.value, "caf");
        assert_eq!(f.cursor, 3);

        let mut g = TextField::new("日本");
        g.home();
        g.right(); // between the two chars
        g.insert('X');
        assert_eq!(g.value, "日X本");
        assert_eq!(g.cursor, 2);
        g.home();
        g.delete(); // removes '日'
        assert_eq!(g.value, "X本");
    }

    #[test]
    fn delete_to_start_and_end() {
        let mut f = TextField::new("hello world");
        f.cursor = 5; // between "hello" and " world"
        f.delete_to_end();
        assert_eq!(f.value, "hello");
        assert_eq!(f.cursor, 5);

        let mut g = TextField::new("hello world");
        g.cursor = 5;
        g.delete_to_start();
        assert_eq!(g.value, " world");
        assert_eq!(g.cursor, 0);

        // No-ops at the boundaries.
        let mut h = TextField::new("abc");
        h.home();
        h.delete_to_start();
        assert_eq!(h.value, "abc");
        h.end();
        h.delete_to_end();
        assert_eq!(h.value, "abc");
    }

    #[test]
    fn delete_prev_word_eats_trailing_space_then_the_word() {
        let mut f = TextField::new("go north  ");
        f.end();
        f.delete_prev_word(); // eats trailing spaces + "north"
        assert_eq!(f.value, "go ");
        assert_eq!(f.cursor, 3);
        f.delete_prev_word(); // eats "go "
        assert_eq!(f.value, "");
        assert_eq!(f.cursor, 0);
        f.delete_prev_word(); // no-op at start
        assert_eq!(f.value, "");
    }

    #[test]
    fn utf8_multibyte_ops_are_char_safe() {
        // "héllo wörld" — 'é' and 'ö' are each 2 bytes.
        let mut f = TextField::new("héllo wörld");
        assert_eq!(f.char_len(), 11);
        f.cursor = 6; // just after "héllo "
        f.delete_to_start();
        assert_eq!(f.value, "wörld");
        assert_eq!(f.cursor, 0);

        let mut g = TextField::new("héllo wörld");
        g.cursor = 6;
        g.delete_to_end();
        assert_eq!(g.value, "héllo ");
        assert_eq!(g.cursor, 6);

        let mut h = TextField::new("héllo wörld");
        h.end();
        h.delete_prev_word();
        assert_eq!(h.value, "héllo ");
        assert_eq!(h.cursor, 6);
    }
}
