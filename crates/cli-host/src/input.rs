//! Reading input, and knowing when there is none left.
//!
//! The whole module exists for one distinction the CLIs each had to learn
//! separately: **a blank line is not EOF**. `read_line` returning `Ok(0)` means
//! the stream is finished; returning `Ok(1)` with `"\n"` means the player
//! pressed Enter on an empty command, which is a perfectly good thing to hand a
//! game. Confuse the two and the host synthesizes a newline forever — the game
//! reprints its prompt, asks again, gets another fabricated newline, and the
//! process has to be killed.
//!
//! That bug shipped twice. `zvm-cli` fixed it in its line path long ago;
//! `gvm-cli` carried the identical defect until SQ-0604, where piping
//! `/dev/null` to Kerkerkruip repeated its "no destination yet" reply until the
//! process was killed. `zvm-cli`'s *char* path, meanwhile, got the EOF check
//! right but exited without restoring the terminal.
//!
//! So: one pair of readers that report EOF honestly, and one exit that always
//! puts the terminal back.

use std::io::{self, BufRead};

use crate::term::restore_and_exit;

/// Read one line, or `None` at true EOF.
///
/// A 0-byte read is EOF. A blank line still yields one byte (`"\n"`) and is a
/// legitimate empty command. A read error is not recoverable input either, so it
/// reports EOF rather than pretending.
pub fn read_line_or_eof<R: BufRead>(r: &mut R) -> Option<String> {
    let mut line = String::new();
    match r.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line),
        Err(_) => None,
    }
}

/// Read one line and return its first byte, or `None` at true EOF.
///
/// A blank line yields `b'\n'` — the byte the player actually typed — which must
/// not be confused with the `None` that means the stream is done.
pub fn read_byte_or_eof<R: BufRead>(r: &mut R) -> Option<u8> {
    read_line_or_eof(r).map(|line| line.bytes().next().unwrap_or(b'\n'))
}

/// [`read_line_or_eof`] against the real stdin.
pub fn read_line_stdin() -> Option<String> {
    read_line_or_eof(&mut io::stdin().lock())
}

/// [`read_byte_or_eof`] against the real stdin.
pub fn read_byte_stdin() -> Option<u8> {
    read_byte_or_eof(&mut io::stdin().lock())
}

/// Stop cleanly at end of piped input: restore the terminal and exit 0.
///
/// The game asked for input and there is none; the only honest answers are to
/// stop or to hang. `prefix` carries any renderer teardown (a scroll-region
/// reset, say) that must go out before the terminal is handed back.
///
/// The restore matters even here, where stdin is a pipe: stdout may still be a
/// terminal — `echo commands | zvm-cli story.z5` — and it is wearing the game's
/// page background and a block cursor.
pub fn exit_at_eof(prefix: &str) -> ! {
    restore_and_exit(prefix, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_line_is_input_not_eof() {
        let mut r = io::Cursor::new(b"\n".to_vec());
        assert_eq!(read_line_or_eof(&mut r), Some("\n".to_string()));
        assert_eq!(read_line_or_eof(&mut r), None, "now it really is EOF");

        // The byte reader must draw the same line: `\n` is the byte the player
        // typed. Reading it as EOF (or reading EOF as `\n`) is the loop that
        // hung Kerkerkruip on /dev/null — SQ-0604.
        let mut r = io::Cursor::new(b"\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut r), Some(b'\n'));
        assert_eq!(read_byte_or_eof(&mut r), None);
    }

    #[test]
    fn empty_stream_is_eof_immediately() {
        assert_eq!(read_line_or_eof(&mut io::Cursor::new(Vec::new())), None);
        assert_eq!(read_byte_or_eof(&mut io::Cursor::new(Vec::new())), None);
    }

    #[test]
    fn lines_are_returned_with_their_terminator_and_first_byte() {
        let mut r = io::Cursor::new(b"look\nnorth\n".to_vec());
        assert_eq!(read_line_or_eof(&mut r), Some("look\n".to_string()));
        assert_eq!(read_line_or_eof(&mut r), Some("north\n".to_string()));
        assert_eq!(read_line_or_eof(&mut r), None);

        let mut r = io::Cursor::new(b"yes\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut r), Some(b'y'));
    }

    #[test]
    fn unterminated_final_line_is_input_then_eof() {
        // A pipe whose last line has no trailing newline still carries a command.
        let mut r = io::Cursor::new(b"quit".to_vec());
        assert_eq!(read_line_or_eof(&mut r), Some("quit".to_string()));
        assert_eq!(read_line_or_eof(&mut r), None);
    }
}
