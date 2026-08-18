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
