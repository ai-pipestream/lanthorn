//! Macintosh resource forks (SQ-0911).
//!
//! A Macintosh file is two forks. Infocom keeps its story and its artwork in the
//! DATA fork — which is why `hfs.rs` read only that one for a long time, and why
//! `iso9660.rs` still skips the associated files that carry forks on a hybrid CD.
//! But the v6 releases keep their **bitmap fonts** in the resource fork, so it has
//! to be readable to get at them.
//!
//! # Format
//!
//! A 16-byte header giving the offsets and lengths of the data area and the map.
//! The map repeats the header, then carries a type list: a count, then per type a
//! four-character code, a count, and the offset to that type's reference list. Each
//! reference gives a resource id, an offset into the name list, and a three-byte
//! offset into the data area, where the resource is stored as a four-byte length
//! followed by its bytes.
//!
//! All counts are stored one less than the real number, which is the classic trap
//! here: a fork with exactly one type stores zero.

/// One resource: its id, its name when it has one, and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// The resource id. Signed, and negative ids are ordinary.
    pub id: i16,
    /// The resource name, when the fork carries one for it.
    pub name: Option<String>,
    /// The resource's own bytes, without the four-byte length that precedes them.
    pub data: Vec<u8>,
}

/// Every resource in a fork, grouped by four-character type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceFork {
    /// `(type, resources)`, in the order the map lists them.
    pub types: Vec<([u8; 4], Vec<Resource>)>,
}

impl ResourceFork {
    /// Every resource of one type, in map order. Empty when the fork has none.
    pub fn of_type(&self, ty: &[u8; 4]) -> &[Resource] {
        self.types.iter().find(|(t, _)| t == ty).map_or(&[], |(_, r)| r.as_slice())
    }

    /// One resource by type and id.
    pub fn get(&self, ty: &[u8; 4], id: i16) -> Option<&Resource> {
        self.of_type(ty).iter().find(|r| r.id == id)
    }

    /// Parse a resource fork, or `None` when the bytes are not one.
    ///
    /// Every offset is bounds-checked against the buffer, so this can be pointed at
    /// any fork — including an empty one, which yields no types rather than an error.
    pub fn parse(raw: &[u8]) -> Option<ResourceFork> {
        let be16 = |o: usize| -> Option<u16> { Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?])) };
        let be32 = |o: usize| -> Option<u32> {
            Some(u32::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?, *raw.get(o + 2)?, *raw.get(o + 3)?]))
        };
        if raw.len() < 16 {
            return None;
        }
        let data_off = usize::try_from(be32(0)?).ok()?;
        let map_off = usize::try_from(be32(4)?).ok()?;
        if map_off + 30 > raw.len() {
            return None;
        }
        let type_list = map_off.checked_add(usize::from(be16(map_off + 24)?))?;
        let name_list = map_off.checked_add(usize::from(be16(map_off + 26)?))?;
        // Counts are stored one less than the truth, so an empty list is 0xFFFF.
        let n_types = match be16(type_list)? {
            0xFFFF => 0,
            n => usize::from(n) + 1,
        };

        let mut types = Vec::with_capacity(n_types);
        for i in 0..n_types {
            let e = type_list.checked_add(2)?.checked_add(i.checked_mul(8)?)?;
            let ty = [*raw.get(e)?, *raw.get(e + 1)?, *raw.get(e + 2)?, *raw.get(e + 3)?];
            let count = usize::from(be16(e + 4)?).checked_add(1)?;
            let ref_list = type_list.checked_add(usize::from(be16(e + 6)?))?;
            let mut out = Vec::with_capacity(count);
            for j in 0..count {
                let r = ref_list.checked_add(j.checked_mul(12)?)?;
                let id = i16::from_be_bytes([*raw.get(r)?, *raw.get(r + 1)?]);
                let name = match be16(r + 2)? {
                    0xFFFF => None,
                    n => {
                        let at = name_list.checked_add(usize::from(n))?;
                        let len = usize::from(*raw.get(at)?);
                        raw.get(at + 1..at + 1 + len)
                            .map(|b| String::from_utf8_lossy(b).into_owned())
                    }
                };
                // A three-byte offset with the attribute byte in front of it.
                let off = usize::try_from(be32(r + 4)? & 0x00FF_FFFF).ok()?;
                let at = data_off.checked_add(off)?;
                let len = usize::try_from(be32(at)?).ok()?;
                let data = raw.get(at + 4..at + 4 + len)?.to_vec();
                out.push(Resource { id, name, data });
            }
            types.push((ty, out));
        }
        Some(ResourceFork { types })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The synthetic Macintosh volume `unit_tests/mk_macfont_hfs.py` builds.
    ///
    /// Committed, and redistributable by construction — it holds nothing but
    /// bytes this repository authored — so unlike every fixture under
    /// `stories/` it is present on CI, where these paths were untested
    /// (SQ-1015). `include_bytes!` also makes a vanished fixture a COMPILE
    /// error rather than a vacuous skip.
    pub(crate) const VOLUME: &[u8] = include_bytes!("../../../unit_tests/macfont.hfs");

    /// Where the application's resource fork sits in that volume, and how long
    /// it is. Hand-computed from the generator's own layout — allocation
    /// blocks are 512 bytes and start at logical block 4, and the fork is at
    /// allocation block 2 — so this reaches the fork WITHOUT going through
    /// `crate::hfs`, and the two can be checked against each other.
    pub(crate) const FORK_AT: usize = (4 + 2) * 512;
    pub(crate) const FORK_LEN: usize = 525;

    pub(crate) fn fork_bytes() -> &'static [u8] {
        &VOLUME[FORK_AT..FORK_AT + FORK_LEN]
    }

    /// The fixture is the one these expectations were written against.
    ///
    /// Non-vacuity: every other case here would still pass against a fork that
    /// had quietly become something else, because they assert what they find.
    /// This one asserts the bytes, straight out of the file, at offsets read
    /// off the hex dump by hand.
    #[test]
    fn the_fixture_is_the_volume_these_tests_were_written_against() {
        assert_eq!(VOLUME.len(), 32_768, "the synthetic volume is 64 logical blocks");
        assert_eq!(&VOLUME[1024..1026], b"BD", "an HFS Master Directory Block at block 2");
        let fork = fork_bytes();
        // The 16-byte resource header: data at 256, map at 440, 184 bytes of
        // data, 85 bytes of map.
        assert_eq!(&fork[0..16], &[0, 0, 1, 0, 0, 0, 1, 0xB8, 0, 0, 0, 0xB8, 0, 0, 0, 0x55]);
        // The map repeats it, which is what makes 440 the map offset.
        assert_eq!(&fork[440..456], &fork[0..16], "the map opens with a copy of the header");
        // And the first resource's four-byte length, at the data area's start.
        assert_eq!(&fork[256..260], &[0, 0, 0, 110], "FONT 524 is 110 bytes");
    }

    #[test]
    fn parses_the_synthetic_fork() {
        let fork = ResourceFork::parse(fork_bytes()).expect("a resource fork");
        assert_eq!(fork.types.len(), 1, "one type, `FONT` — and a count stored as zero");
        assert_eq!(fork.types[0].0, *b"FONT");

        let fonts = fork.of_type(b"FONT");
        assert_eq!(fonts.len(), 2, "two resources — and a count stored as one");
        assert_eq!(fonts[0].id, 524);
        assert_eq!(fonts[0].name.as_deref(), Some("Lanthorn 12"));
        assert_eq!(fonts[0].data.len(), 110);
        assert_eq!(fonts[1].id, 1033);
        assert_eq!(fonts[1].name.as_deref(), Some("Lanthorn 9"));
        assert_eq!(fonts[1].data.len(), 66);

        // The three-byte data offset is what places the second resource: 114
        // bytes past the first, which is its own four-byte length plus 110.
        assert_eq!(fonts[1].data, fork_bytes()[256 + 114 + 4..256 + 114 + 4 + 66]);
    }

    #[test]
    fn finds_a_resource_by_type_and_id() {
        let fork = ResourceFork::parse(fork_bytes()).expect("a resource fork");
        assert_eq!(fork.get(b"FONT", 524).map(|r| r.data.len()), Some(110));
        assert_eq!(fork.get(b"FONT", 1033).map(|r| r.data.len()), Some(66));
        assert!(fork.get(b"FONT", 525).is_none(), "no such id");
        assert!(fork.get(b"NFNT", 524).is_none(), "no such type");
        assert!(fork.of_type(b"snd ").is_empty());
    }

    /// Truncation is refused rather than half-read, at every offset the format
    /// makes a reader follow.
    #[test]
    fn refuses_forks_that_are_not_forks() {
        assert_eq!(ResourceFork::parse(b""), None);
        assert_eq!(ResourceFork::parse(&[0u8; 15]), None, "shorter than the header");
        // A header pointing at a map that is not there.
        let short = &fork_bytes()[..440 + 29];
        assert_eq!(ResourceFork::parse(short), None, "the map does not fit");
        // A resource whose three-byte data offset leaves the fork is refused
        // outright — the four-byte length it points at is not there to read.
        // The second reference list entry sits 50 bytes into the map and its
        // offset is the three bytes after the attribute byte.
        let mut broken = fork_bytes().to_vec();
        broken[440 + 50 + 5..440 + 50 + 8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        assert_eq!(ResourceFork::parse(&broken), None, "a data offset off the end");
    }

    /// A NAME that runs off the end is not the same failure, and does not
    /// cost the resource: the fork still parses and the resource is nameless.
    /// Names are a convenience — the id is what selects a resource — so a
    /// truncated name list must not take the font down with it.
    #[test]
    fn a_name_that_does_not_fit_leaves_the_resource_nameless() {
        let cut = &fork_bytes()[..fork_bytes().len() - 1];
        let fork = ResourceFork::parse(cut).expect("still a fork");
        let fonts = fork.of_type(b"FONT");
        assert_eq!(fonts.len(), 2);
        assert_eq!(fonts[0].name.as_deref(), Some("Lanthorn 12"), "the first name is whole");
        assert_eq!(fonts[1].name, None, "the second ran off the end");
        assert_eq!(fonts[1].data.len(), 66, "and its bytes are untouched");
    }

    /// An empty fork is a legitimate thing to be handed, and yields no types.
    #[test]
    fn an_empty_fork_yields_no_types() {
        let mut raw = vec![0u8; 46];
        raw[3] = 16; // data at 16
        raw[7] = 16; // map at 16
        raw[15] = 30; // 30 bytes of map
        raw[16 + 24] = 0;
        raw[16 + 25] = 28; // type list at map + 28
        raw[16 + 26] = 0;
        raw[16 + 27] = 30; // name list past it
        raw[16 + 28] = 0xFF;
        raw[16 + 29] = 0xFF; // no types at all
        assert_eq!(ResourceFork::parse(&raw), Some(ResourceFork::default()));
    }
}
