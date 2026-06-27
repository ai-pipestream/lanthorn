//! Zero-dependency parser for the IFF "Blorb" interactive-fiction resource
//! container. Exposes the embedded executable and a generic resource accessor.

/// Errors that can arise while parsing a Blorb container.
#[derive(Debug, PartialEq, Eq)]
pub enum BlorbError {
    /// The bytes are not a Blorb (no `FORM`…`IFRS` magic).
    NotBlorb,
    /// A length or offset ran past the end of the data.
    Truncated,
    /// No `RIdx` resource-index chunk was found.
    NoResourceIndex,
    /// A resource-index entry points at an offset outside the file.
    BadOffset,
    /// No usable `Exec` executable chunk was found.
    NoExecutable,
}

/// One parsed resource-index entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEntry {
    /// Usage tag (e.g. `b"Exec"`, `b"Pict"`, `b"Snd "`, `b"Data"`).
    pub usage: [u8; 4],
    /// Resource number within its usage class.
    pub number: u32,
    /// File offset of the resource's chunk header.
    pub start: usize,
    /// The 4-byte chunk type at `start`.
    pub chunk_type: [u8; 4],
    /// Chunk data length (excludes the 8-byte header and any pad byte).
    pub len: usize,
}

/// The kind of executable embedded in a Blorb's `Exec` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecKind {
    /// Z-machine story (`ZCOD` chunk).
    ZCode,
    /// Glulx story (`GLUL` chunk).
    Glulx,
}

/// A parsed Blorb container: owns the file bytes and the resource index.
#[derive(Debug)]
pub struct Blorb {
    bytes: Vec<u8>,
    index: Vec<ResourceEntry>,
}

fn be_u32(b: &[u8], off: usize) -> Result<u32, BlorbError> {
    let s = b
        .get(off..off + 4)
        .ok_or(BlorbError::Truncated)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

impl Blorb {
    /// Cheap magic check: `bytes[0..4] == b"FORM" && bytes[8..12] == b"IFRS"`.
    ///
    /// Returns `false` (rather than parsing) so callers can fall back to
    /// treating the bytes as a raw story file.
    pub fn is_blorb(b: &[u8]) -> bool {
        b.len() >= 12 && &b[0..4] == b"FORM" && &b[8..12] == b"IFRS"
    }

    /// Parse the bytes as a Blorb, eagerly building the resource index.
    ///
    /// Returns [`BlorbError::NotBlorb`] when the data isn't a Blorb, and other
    /// [`BlorbError`] variants for malformed/truncated containers. All offsets
    /// and lengths are bounds-checked, so malformed input never panics.
    pub fn parse(bytes: Vec<u8>) -> Result<Blorb, BlorbError> {
        if !Self::is_blorb(&bytes) {
            return Err(BlorbError::NotBlorb);
        }
        let end = bytes.len();
        // Walk top-level chunks (start at 12, after FORM+len+IFRS) to find RIdx.
        let mut ridx: Option<(usize, usize)> = None; // (entries_start, count)
        let mut pos = 12;
        while pos + 8 <= end {
            let ctype = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
            let clen = be_u32(&bytes, pos + 4)? as usize;
            let data_start = pos + 8;
            if data_start + clen > end {
                return Err(BlorbError::Truncated);
            }
            if &ctype == b"RIdx" {
                let count = be_u32(&bytes, data_start)? as usize;
                ridx = Some((data_start + 4, count));
                break;
            }
            pos = data_start + clen + (clen & 1);
        }
        let (mut p, count) = ridx.ok_or(BlorbError::NoResourceIndex)?;
        let mut index = Vec::with_capacity(count);
        for _ in 0..count {
            let u = bytes.get(p..p + 4).ok_or(BlorbError::Truncated)?;
            let usage = [u[0], u[1], u[2], u[3]];
            let number = be_u32(&bytes, p + 4)?;
            let start = be_u32(&bytes, p + 8)? as usize;
            // Read the pointed-at chunk header.
            let t = bytes.get(start..start + 4).ok_or(BlorbError::BadOffset)?;
            let chunk_type = [t[0], t[1], t[2], t[3]];
            let len = be_u32(&bytes, start + 4).map_err(|_| BlorbError::BadOffset)? as usize;
            if start + 8 + len > end {
                return Err(BlorbError::BadOffset);
            }
            index.push(ResourceEntry { usage, number, start, chunk_type, len });
            p += 12;
        }
        Ok(Blorb { bytes, index })
    }

    /// The parsed resource index (for enumeration).
    pub fn resources(&self) -> &[ResourceEntry] {
        &self.index
    }

    fn chunk_data(&self, e: &ResourceEntry) -> &[u8] {
        &self.bytes[e.start + 8..e.start + 8 + e.len]
    }

    /// The embedded executable: its [`ExecKind`] plus a slice of its chunk data.
    ///
    /// Finds the `Exec` resource, mapping a `ZCOD` chunk to [`ExecKind::ZCode`]
    /// and `GLUL` to [`ExecKind::Glulx`]. Returns [`BlorbError::NoExecutable`]
    /// when there is no `Exec` entry or its chunk type is neither.
    pub fn executable(&self) -> Result<(ExecKind, &[u8]), BlorbError> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == b"Exec")
            .ok_or(BlorbError::NoExecutable)?;
        let kind = match &e.chunk_type {
            b"ZCOD" => ExecKind::ZCode,
            b"GLUL" => ExecKind::Glulx,
            _ => return Err(BlorbError::NoExecutable),
        };
        Ok((kind, self.chunk_data(e)))
    }

    /// A resource by `usage` + `number` (e.g. `(b"Snd ", 3)`): its chunk type
    /// and data slice, or `None` when no such resource exists.
    pub fn resource(&self, usage: &[u8; 4], number: u32) -> Option<(&[u8; 4], &[u8])> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == usage && r.number == number)?;
        Some((&e.chunk_type, self.chunk_data(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an IFF chunk: type + BE len + data + pad-to-even.
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// Build a Blorb with the given resources. Each resource is
    /// (usage, number, chunk_type, data). Returns the file bytes.
    fn build_blorb(res: &[(&[u8; 4], u32, &[u8; 4], &[u8])]) -> Vec<u8> {
        // Lay out the resource chunks after the RIdx chunk to compute offsets.
        let count = res.len() as u32;
        let ridx_data_len = 4 + 12 * res.len();
        // RIdx chunk header (8) sits at file offset 12 (after FORM+len+IFRS).
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_u, _n, ty, data) in res {
            offsets.push(cursor as u32);
            let c = chunk(ty, data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        // RIdx data
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&count.to_be_bytes());
        for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
            ridx.extend_from_slice(*usage);
            ridx.extend_from_slice(&number.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let ridx_chunk = chunk(b"RIdx", &ridx);
        // Assemble FORM
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn is_blorb_detects_magic() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        assert!(Blorb::is_blorb(&b));
        assert!(!Blorb::is_blorb(b"not a blorb at all"));
        assert!(!Blorb::is_blorb(&[]));
    }

    #[test]
    fn parse_indexes_resources() {
        let b = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 1, b"FORM", b"xyz"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.resources().len(), 2);
        let exec = blorb
            .resources()
            .iter()
            .find(|r| &r.usage == b"Exec")
            .unwrap();
        assert_eq!(&exec.chunk_type, b"ZCOD");
        assert_eq!(exec.len, 4);
    }

    #[test]
    fn parse_rejects_malformed() {
        assert_eq!(Blorb::parse(b"junk".to_vec()).unwrap_err(), BlorbError::NotBlorb);
        // FORM/IFRS but truncated before any chunk
        let mut t = b"FORM".to_vec();
        t.extend_from_slice(&4u32.to_be_bytes());
        t.extend_from_slice(b"IFRS");
        assert_eq!(Blorb::parse(t).unwrap_err(), BlorbError::NoResourceIndex);
    }

    #[test]
    fn executable_returns_zcode_data() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        let blorb = Blorb::parse(b).unwrap();
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::ZCode);
        assert_eq!(data, b"abcd");
    }

    #[test]
    fn executable_detects_glulx() {
        let b = build_blorb(&[(b"Exec", 0, b"GLUL", b"glul")]);
        assert_eq!(Blorb::parse(b).unwrap().executable().unwrap().0, ExecKind::Glulx);
    }

    #[test]
    fn executable_missing_is_error() {
        let b = build_blorb(&[(b"Snd ", 1, b"FORM", b"x")]);
        assert_eq!(
            Blorb::parse(b).unwrap().executable().unwrap_err(),
            BlorbError::NoExecutable
        );
    }

    #[test]
    fn resource_fetches_by_usage_number() {
        let b = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 3, b"OGGV", b"oggdata"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        let (ty, data) = blorb.resource(b"Snd ", 3).unwrap();
        assert_eq!(ty, b"OGGV");
        assert_eq!(data, b"oggdata");
        assert!(blorb.resource(b"Snd ", 99).is_none());
    }

    #[test]
    fn resource_handles_odd_length_pad_byte() {
        // First chunk has odd-length data ("oggdata" = 7); the pad byte must be
        // skipped so the following resource is still found at its real offset.
        let b = build_blorb(&[
            (b"Snd ", 3, b"OGGV", b"oggdata"),
            (b"Exec", 0, b"ZCOD", b"abcd"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.resource(b"Snd ", 3).unwrap().1, b"oggdata");
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::ZCode);
        assert_eq!(data, b"abcd");
    }

    #[test]
    fn parse_rejects_bad_offset() {
        // A valid RIdx whose single entry points past EOF → BadOffset.
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // number
        ridx.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // start past EOF
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        assert_eq!(Blorb::parse(file).unwrap_err(), BlorbError::BadOffset);
    }
}
