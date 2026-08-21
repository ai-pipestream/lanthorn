//! SQ-0976: undo the kitty protocol's `o=z` in a capture, so the placement
//! oracle can still read it.
//!
//! **Why this exists, and why it is not optional.** `qwertty-term-vt` is the
//! terminal core the oracle ([`super::oracle`]) resolves bytes through, and it
//! deliberately links no codecs at all: its `ImageDecoder` is a seam, and
//! `Stream<TerminalHandler>` — the only way to feed it a byte stream — wires the
//! null `NoDecoder`, whose `inflate_zlib` answers `None`. A compressed transmit
//! therefore fails with `EINVAL: decompression failed`, the image is never
//! stored, and every placement that named it silently disappears. Measured the
//! moment `kitty_transmit_virtual` started emitting `o=z`: three `pty_oracle`
//! cases went from one placement to **zero**, reporting "the art was placed to
//! begin with: left 0, right 1" — a screen with no art on it, from a stream that
//! draws art perfectly well on a real terminal.
//!
//! That is a gap in the HARNESS, not a finding about lanthorn, so it is repaired
//! here rather than by declining to compress. Our own decoder ([`super::decode`])
//! never had the problem: it counts payload bytes and does not decode pixels.
//!
//! **This is a transport rewrite and nothing more.** Base64 is undone and redone
//! the same way by every other layer that reads these bytes; `o=z` sits at
//! exactly that level — the kitty spec says the payload *"is now compressed using
//! deflate (this occurs prior to base64 encoding)"* — so replacing a compressed
//! transmit with the uncompressed one it stands for changes no image, no
//! geometry, no placement and no id. What it does change is the byte COUNT, which
//! is why [`super::driver::Capture::bytes`] keeps the wire stream and this runs
//! only where a terminal's reading is wanted.

/// Rewrite every `o=z` kitty transmit in `stream` as the uncompressed `f=32`
/// transmit it stands for, leaving all other bytes untouched.
///
/// Borrows when there is nothing to do, which is every capture from a build that
/// does not compress and every stream with no graphics in it at all.
pub fn kitty_inflate(stream: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if find(stream, b"o=z").is_none() {
        return std::borrow::Cow::Borrowed(stream);
    }
    let mut out: Vec<u8> = Vec::with_capacity(stream.len());
    let mut i = 0usize;
    while i < stream.len() {
        let Some(rel) = find(&stream[i..], b"\x1b_G") else {
            out.extend_from_slice(&stream[i..]);
            break;
        };
        out.extend_from_slice(&stream[i..i + rel]);
        i += rel;
        // The command, and everything that continues it. `None` means the stream
        // was truncated mid-command — copy the tail through verbatim rather than
        // guess at it.
        let Some((params, payload, next)) = take_transmit(stream, i) else {
            out.extend_from_slice(&stream[i..]);
            break;
        };
        let start = i;
        i = next;
        if !params.split(',').any(|kv| kv == "o=z") {
            // Untouched: re-emit exactly the bytes that were there.
            out.extend_from_slice(&stream[start..next]);
            continue;
        }
        let raw = match inflate(&unb64(&payload)) {
            Some(raw) => raw,
            // A payload we cannot inflate is a defect worth SEEING as a missing
            // image rather than papering over, so pass it through unchanged.
            None => {
                out.extend_from_slice(b"\x1b_G");
                out.extend_from_slice(params.as_bytes());
                out.push(b';');
                out.extend_from_slice(&payload);
                out.extend_from_slice(b"\x1b\\");
                continue;
            }
        };
        let head: Vec<&str> = params.split(',').filter(|kv| *kv != "o=z" && !kv.starts_with("m=")).collect();
        let head = head.join(",");
        let chunks: Vec<&[u8]> = raw.chunks(3072).collect();
        let n = chunks.len();
        for (c, chunk) in chunks.into_iter().enumerate() {
            let more = u8::from(c + 1 < n);
            if c == 0 {
                out.extend_from_slice(b"\x1b_G");
                out.extend_from_slice(head.as_bytes());
                out.extend_from_slice(format!(",m={more};").as_bytes());
            } else {
                out.extend_from_slice(format!("\x1b_Gq=2,m={more};").as_bytes());
            }
            out.extend_from_slice(b64(chunk).as_bytes());
            out.extend_from_slice(b"\x1b\\");
        }
    }
    std::borrow::Cow::Owned(out)
}

/// One transmit starting at `at` (which must index an `ESC _ G`), as its first
/// command's parameters, every chunk's base64 concatenated, and the offset just
/// past the last chunk consumed.
///
/// Continuation chunks are the ones the spec allows to carry only `m` and `q`;
/// the run ends at the chunk saying `m=0`, or at the first command that is not a
/// continuation.
fn take_transmit(stream: &[u8], at: usize) -> Option<(String, Vec<u8>, usize)> {
    let (params, payload, mut next) = take_command(stream, at)?;
    let mut all = payload;
    let mut more = params.split(',').any(|kv| kv == "m=1");
    while more {
        if !stream[next..].starts_with(b"\x1b_G") {
            break;
        }
        let (p, pay, after) = take_command(stream, next)?;
        if !p.split(',').all(|kv| kv.starts_with("m=") || kv.starts_with("q=")) {
            break;
        }
        all.extend_from_slice(&pay);
        more = p.split(',').any(|kv| kv == "m=1");
        next = after;
    }
    Some((params, all, next))
}

/// One APC command at `at`: its parameters, its payload bytes, and the offset
/// past its ST.
///
/// A command with no `;` is one with no payload — `a=d,d=I,i=…` deletes are the
/// ones this stream is full of — and must come back as parameters rather than as
/// "unparseable", or a single delete would stop the walk dead and pass the whole
/// rest of the capture through untouched.
fn take_command(stream: &[u8], at: usize) -> Option<(String, Vec<u8>, usize)> {
    let body_start = at + 3;
    let end = body_start + find(&stream[body_start..], b"\x1b\\")?;
    let body = &stream[body_start..end];
    let (params, payload) = match find(body, b";") {
        Some(semi) => (&body[..semi], body[semi + 1..].to_vec()),
        None => (body, Vec::new()),
    };
    Some((String::from_utf8_lossy(params).into_owned(), payload, end + 2))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

fn unb64(data: &[u8]) -> Vec<u8> {
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for &c in data.iter().filter(|&&c| c != b'=') {
        let Some(v) = ALPHABET.iter().position(|&t| t == c) else { continue };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    std::io::copy(&mut flate2::read::ZlibDecoder::new(data), &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deflate(raw: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(raw).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn a_stream_with_no_compressed_transmit_is_handed_straight_back() {
        let plain = b"\x1b[2J\x1b_Gq=2,i=7,a=T,U=1,f=32,t=d,s=2,v=1,r=1,c=2,m=0;AAAAAAAA\x1b\\hello";
        assert!(matches!(kitty_inflate(plain), std::borrow::Cow::Borrowed(_)));
    }

    /// The whole point: a multi-chunk compressed transmit comes back as the
    /// uncompressed one, payload identical and `o=z` gone.
    #[test]
    fn a_chunked_compressed_transmit_becomes_the_uncompressed_transmit_it_stood_for() {
        // Incompressible on purpose: the COMPRESSED stream has to exceed one
        // 3072-byte chunk, or the fixture never exercises continuation chunks —
        // and a canvas's worth of flat artwork deflates to well under that.
        let mut lcg = 0x2545_F491_4F6C_DD1Du64;
        let raw: Vec<u8> = (0..40_000)
            .map(|_| {
                lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (lcg >> 33) as u8
            })
            .collect();
        let z = deflate(&raw);
        let mut stream = b"before".to_vec();
        let chunks: Vec<&[u8]> = z.chunks(3072).collect();
        let n = chunks.len();
        assert!(n > 1, "the fixture must exercise continuation chunks");
        for (i, c) in chunks.into_iter().enumerate() {
            let more = u8::from(i + 1 < n);
            if i == 0 {
                stream.extend_from_slice(
                    format!("\x1b_Gq=2,i=9,a=T,U=1,f=32,o=z,t=d,s=100,v=100,r=2,c=4,m={more};").as_bytes(),
                );
            } else {
                stream.extend_from_slice(format!("\x1b_Gq=2,m={more};").as_bytes());
            }
            stream.extend_from_slice(b64(c).as_bytes());
            stream.extend_from_slice(b"\x1b\\");
        }
        stream.extend_from_slice(b"after");

        let out = kitty_inflate(&stream).into_owned();
        assert!(out.starts_with(b"before") && out.ends_with(b"after"), "surrounding bytes survive");
        assert!(find(&out, b"o=z").is_none(), "nothing still claims to be compressed");

        // Reassemble what a terminal would.
        let (params, payload, _) = take_transmit(&out, find(&out, b"\x1b_G").unwrap()).unwrap();
        assert!(params.contains("i=9") && params.contains("s=100,v=100"), "control keys survive: {params}");
        assert_eq!(unb64(&payload), raw, "the payload is the bytes that were compressed");
    }
}
