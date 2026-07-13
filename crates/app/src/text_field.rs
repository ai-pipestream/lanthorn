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
}
