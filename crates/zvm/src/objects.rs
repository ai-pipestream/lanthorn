// Z-machine object table — ZMSD §12.
//
// Attributes, parent/child/sibling tree, properties, short names.
// v3: 1-byte object numbers, 32 attrs, 31-word default table, 9-byte entries.
// v4+: 2-byte object numbers, 48 attrs, 63-word default table, 14-byte entries.

use crate::memory::Memory;
use crate::text::decode_string;

// ── Layout helpers ────────────────────────────────────────────────────────────

/// Number of words in the property-defaults table.
fn prop_defaults_count(version: u8) -> u32 {
    if version <= 3 { 31 } else { 63 }
}

/// Number of attribute bytes per object entry.
fn attr_bytes(version: u8) -> u32 {
    if version <= 3 { 4 } else { 6 }
}

/// Size of a single object entry in bytes.
fn entry_size(version: u8) -> u32 {
    if version <= 3 { 9 } else { 14 }
}

/// Base address of the object-entries region (after the property-defaults table).
fn entries_base(mem: &Memory) -> u32 {
    mem.object_table() as u32 + prop_defaults_count(mem.version()) * 2
}

/// Byte address of object `obj`'s entry (object numbers are 1-based).
fn entry_addr(mem: &Memory, obj: u16) -> u32 {
    entries_base(mem) + (obj as u32 - 1) * entry_size(mem.version())
}

// ── Attributes ────────────────────────────────────────────────────────────────

/// Read attribute `attr` of object `obj`.  Attribute N is bit `7-(N%8)` of
/// attribute byte `N/8` (MSB-first, ZMSD §12.3).
pub fn get_attr(mem: &Memory, obj: u16, attr: u8) -> bool {
    let byte_addr = entry_addr(mem, obj) + (attr / 8) as u32;
    let bit = 7 - (attr % 8);
    (mem.read_byte(byte_addr) >> bit) & 1 == 1
}

/// Set attribute `attr` of object `obj`.
pub fn set_attr(mem: &mut Memory, obj: u16, attr: u8) {
    let byte_addr = entry_addr(mem, obj) + (attr / 8) as u32;
    let bit = 7 - (attr % 8);
    let v = mem.read_byte(byte_addr) | (1 << bit);
    mem.write_byte(byte_addr, v);
}

/// Clear attribute `attr` of object `obj`.
pub fn clear_attr(mem: &mut Memory, obj: u16, attr: u8) {
    let byte_addr = entry_addr(mem, obj) + (attr / 8) as u32;
    let bit = 7 - (attr % 8);
    let v = mem.read_byte(byte_addr) & !(1 << bit);
    mem.write_byte(byte_addr, v);
}

// ── Tree pointers ─────────────────────────────────────────────────────────────

/// Byte offset within an entry of the parent/sibling/child fields.
/// They come after the attribute bytes.
fn tree_offset(version: u8, which: u8) -> u32 {
    // which: 0=parent, 1=sibling, 2=child
    if version <= 3 {
        attr_bytes(version) + which as u32
    } else {
        attr_bytes(version) + (which as u32) * 2
    }
}

fn read_tree_ptr(mem: &Memory, obj: u16, which: u8) -> u16 {
    let base = entry_addr(mem, obj) + tree_offset(mem.version(), which);
    if mem.version() <= 3 {
        mem.read_byte(base) as u16
    } else {
        mem.read_word(base)
    }
}

fn write_tree_ptr(mem: &mut Memory, obj: u16, which: u8, val: u16) {
    let base = entry_addr(mem, obj) + tree_offset(mem.version(), which);
    if mem.version() <= 3 {
        mem.write_byte(base, val as u8);
    } else {
        mem.write_word(base, val);
    }
}

pub fn get_parent(mem: &Memory, obj: u16) -> u16 { read_tree_ptr(mem, obj, 0) }
pub fn get_sibling(mem: &Memory, obj: u16) -> u16 { read_tree_ptr(mem, obj, 1) }
pub fn get_child(mem: &Memory, obj: u16) -> u16 { read_tree_ptr(mem, obj, 2) }

pub fn set_parent(mem: &mut Memory, obj: u16, val: u16) { write_tree_ptr(mem, obj, 0, val); }
pub fn set_sibling(mem: &mut Memory, obj: u16, val: u16) { write_tree_ptr(mem, obj, 1, val); }
pub fn set_child(mem: &mut Memory, obj: u16, val: u16) { write_tree_ptr(mem, obj, 2, val); }

// ── Tree manipulation ─────────────────────────────────────────────────────────

/// Remove `obj` from its current parent's child list, leaving `obj`'s own
/// children untouched.  Sets `obj`'s parent and sibling to 0.
pub fn remove_obj(mem: &mut Memory, obj: u16) {
    let parent = get_parent(mem, obj);
    if parent == 0 {
        // Already detached.
        return;
    }
    let first_child = get_child(mem, parent);
    if first_child == obj {
        // obj is the first child — promote its sibling.
        let sib = get_sibling(mem, obj);
        set_child(mem, parent, sib);
    } else {
        // Walk sibling chain to find the predecessor of obj.
        let mut cur = first_child;
        loop {
            let next = get_sibling(mem, cur);
            if next == obj {
                let obj_sib = get_sibling(mem, obj);
                set_sibling(mem, cur, obj_sib);
                break;
            }
            if next == 0 {
                // obj not found in chain — shouldn't happen in a valid story.
                break;
            }
            cur = next;
        }
    }
    set_parent(mem, obj, 0);
    set_sibling(mem, obj, 0);
}

/// Make `obj` the first child of `dest`, first unlinking it from its current
/// parent.  (ZMSD §12.4 insert_obj semantics.)
pub fn insert_obj(mem: &mut Memory, obj: u16, dest: u16) {
    remove_obj(mem, obj);
    let old_first = get_child(mem, dest);
    set_sibling(mem, obj, old_first);
    set_child(mem, dest, obj);
    set_parent(mem, obj, dest);
}

// ── Property table address ────────────────────────────────────────────────────

/// Address of the property-table pointer field within the object entry.
fn prop_table_ptr_offset(version: u8) -> u32 {
    // After attributes (4 or 6 bytes) + 3 tree fields (1-byte each v3, 2-byte each v4+).
    if version <= 3 {
        4 + 3
    } else {
        6 + 6
    }
}

/// Read the property-table address stored in an object's entry.
fn prop_table_addr(mem: &Memory, obj: u16) -> u32 {
    let base = entry_addr(mem, obj) + prop_table_ptr_offset(mem.version());
    mem.read_word(base) as u32
}

// ── Short name ────────────────────────────────────────────────────────────────

/// Decode the short name of object `obj` from its property table.
pub fn short_name(mem: &Memory, obj: u16) -> String {
    let ptbl = prop_table_addr(mem, obj);
    // Byte 0: number of words of Z-text for the short name.
    let name_words = mem.read_byte(ptbl) as u32;
    if name_words == 0 {
        return String::new();
    }
    let (s, _) = decode_string(mem, ptbl + 1);
    s
}

// ── Property walking ──────────────────────────────────────────────────────────

/// Returns the address of the first property entry (just after the name).
fn first_prop_addr(mem: &Memory, obj: u16) -> u32 {
    let ptbl = prop_table_addr(mem, obj);
    let name_words = mem.read_byte(ptbl) as u32;
    ptbl + 1 + name_words * 2
}

/// Parse the size byte(s) at `addr` in a property entry.
/// Returns `(property_number, data_length_in_bytes, header_length_in_bytes)`.
///
/// v3 (ZMSD §12.4.1): one size byte; bits 7–5 = (size-1), bits 4–0 = prop num.
/// v4+ (ZMSD §12.4.2): if bit 7 clear, one byte: bits 6–0 encode prop num,
///   bit 6 set → 2-byte data, else 1-byte data. If bit 7 set, two size bytes:
///   first byte bits 5–0 = prop num; second byte bits 5–0 = data length
///   (0 means 64).
fn parse_prop_header(mem: &Memory, addr: u32) -> (u16, u32, u32) {
    let b0 = mem.read_byte(addr);
    if mem.version() <= 3 {
        let prop_num = (b0 & 0x1F) as u16;
        let data_len = ((b0 >> 5) as u32) + 1;
        (prop_num, data_len, 1)
    } else {
        if b0 & 0x80 != 0 {
            // Two-byte form.
            let prop_num = (b0 & 0x3F) as u16;
            let b1 = mem.read_byte(addr + 1);
            let raw_len = (b1 & 0x3F) as u32;
            let data_len = if raw_len == 0 { 64 } else { raw_len };
            (prop_num, data_len, 2)
        } else {
            // One-byte form.
            let prop_num = (b0 & 0x3F) as u16;
            let data_len = if b0 & 0x40 != 0 { 2 } else { 1 };
            (prop_num, data_len, 1)
        }
    }
}

/// Find the address of property `prop`'s data in `obj`'s property table.
/// Returns 0 if the property is absent.
pub fn get_prop_addr(mem: &Memory, obj: u16, prop: u8) -> u16 {
    let mut addr = first_prop_addr(mem, obj);
    loop {
        let b0 = mem.read_byte(addr);
        if b0 == 0 {
            return 0; // end-of-table sentinel
        }
        let (pnum, data_len, hdr_len) = parse_prop_header(mem, addr);
        if pnum == prop as u16 {
            return (addr + hdr_len) as u16;
        }
        if pnum < prop as u16 {
            // Properties are stored in descending order.
            return 0;
        }
        addr += hdr_len + data_len;
    }
}

/// Read the data length (in bytes) of the property whose data starts at
/// `prop_addr`.  The size byte immediately precedes the data. (ZMSD §12.4.4)
pub fn get_prop_len(mem: &Memory, prop_addr: u16) -> u8 {
    if prop_addr == 0 {
        return 0;
    }
    let size_byte_addr = prop_addr as u32 - 1;
    let b = mem.read_byte(size_byte_addr);
    if mem.version() <= 3 {
        (b >> 5) + 1
    } else {
        if b & 0x80 != 0 {
            // Two-byte form: the byte immediately before prop_addr is the
            // second size byte (bits 5–0 = data length).
            let raw = b & 0x3F;
            if raw == 0 { 64 } else { raw }
        } else {
            // One-byte form.
            if b & 0x40 != 0 { 2 } else { 1 }
        }
    }
}

/// Read property `prop` of object `obj`.  Falls back to the property-defaults
/// table if the property is absent from the object.  (ZMSD §12.1)
pub fn get_prop(mem: &Memory, obj: u16, prop: u8) -> u16 {
    let addr = get_prop_addr(mem, obj, prop);
    if addr == 0 {
        // Property defaults: 0-indexed, prop 1 is at offset 0.
        let default_addr = mem.object_table() as u32 + (prop as u32 - 1) * 2;
        return mem.read_word(default_addr);
    }
    let len = get_prop_len(mem, addr);
    match len {
        1 => mem.read_byte(addr as u32) as u16,
        _ => mem.read_word(addr as u32),
    }
}

/// Write property `prop` of object `obj` to `val`.  Only 1- and 2-byte
/// properties are writable via put_prop. (ZMSD §12.4)
pub fn put_prop(mem: &mut Memory, obj: u16, prop: u8, val: u16) {
    let addr = get_prop_addr(mem, obj, prop);
    if addr == 0 {
        panic!("put_prop: object {} has no property {}", obj, prop);
    }
    let len = get_prop_len(mem, addr);
    match len {
        1 => mem.write_byte(addr as u32, val as u8),
        _ => mem.write_word(addr as u32, val),
    }
}

/// Return the next property number after `prop` in object `obj`'s property
/// table.  If `prop` is 0, returns the first property number.  Returns 0 if
/// there are no more properties. (ZMSD §12.4.3)
pub fn get_next_prop(mem: &Memory, obj: u16, prop: u8) -> u8 {
    let mut addr = first_prop_addr(mem, obj);
    if prop == 0 {
        // Return first property.
        let b0 = mem.read_byte(addr);
        if b0 == 0 {
            return 0;
        }
        let (pnum, _, _) = parse_prop_header(mem, addr);
        return pnum as u8;
    }
    // Scan for prop, then return the next one.
    loop {
        let b0 = mem.read_byte(addr);
        if b0 == 0 {
            return 0; // prop not found or end of table
        }
        let (pnum, data_len, hdr_len) = parse_prop_header(mem, addr);
        if pnum == prop as u16 {
            addr += hdr_len + data_len;
            let next_b0 = mem.read_byte(addr);
            if next_b0 == 0 {
                return 0;
            }
            let (next_pnum, _, _) = parse_prop_header(mem, addr);
            return next_pnum as u8;
        }
        addr += hdr_len + data_len;
    }
}

// ── Mapper snapshot ───────────────────────────────────────────────────────────

/// Stable per-object identity for the automapper.
pub struct ObjectSnapshot {
    pub number: u16,
    pub parent: u16,
    pub name: String,
}

pub fn object_snapshot(mem: &Memory, obj: u16) -> ObjectSnapshot {
    ObjectSnapshot {
        number: obj,
        parent: get_parent(mem, obj),
        name: short_name(mem, obj),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::encode::encode_word;

    // ── Shared layout for hand-built v3 object table ──────────────────────────
    //
    // sample_story places object_table at 0x0100.
    // v3 property-defaults: 31 words = 62 bytes → 0x0100..=0x013D
    // Object entries start at 0x013E.
    // Each v3 entry = 9 bytes:
    //   [0..3] attributes (4 bytes)
    //   [4]    parent
    //   [5]    sibling
    //   [6]    child
    //   [7..8] property-table address (word)
    //
    // We'll place property tables at 0x0200 for obj1, 0x0220 for obj2.
    //
    // Object 1 entry at 0x013E, Object 2 entry at 0x0147.

    const OBJ_TABLE: u32 = 0x0100;
    const ENTRIES_V3: u32 = OBJ_TABLE + 31 * 2; // = 0x013E
    const OBJ1_ENTRY: u32 = ENTRIES_V3;          // 0x013E
    const OBJ2_ENTRY: u32 = ENTRIES_V3 + 9;      // 0x0147
    const OBJ3_ENTRY: u32 = ENTRIES_V3 + 18;     // 0x0150

    const PROP1_TBL: u32 = 0x0200; // property table for object 1
    const PROP2_TBL: u32 = 0x0220; // property table for object 2
    const PROP3_TBL: u32 = 0x0230; // property table for object 3

    /// Write `val` as a big-endian word into a byte buffer at `offset`.
    fn put_word(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset] = (val >> 8) as u8;
        buf[offset + 1] = (val & 0xFF) as u8;
    }

    /// Encode a Z-string for a property table name.
    /// Returns the bytes to write (2 bytes per encoded word).
    fn z_name(text: &str) -> Vec<u8> {
        // We'll just use encode_word as a proxy for a short Z-string.
        // For version 3 encoding (6 Z-chars = 4 bytes = 2 words).
        encode_word(text, 3)
    }

    /// Build a v3 story buffer with a small 2-object tree:
    ///   obj2 is child of obj1; obj3 is sibling of obj2.
    ///   obj1 has attr 0 set; obj2 has attr 7, 8 set (cross-byte boundary).
    fn build_v3_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        // ── Object 1 entry ───────────────────────────────────────────────────
        // attr 0 set: byte 0 = 0x80
        buf[OBJ1_ENTRY as usize] = 0x80; // attr 0 set
        buf[OBJ1_ENTRY as usize + 1] = 0x00;
        buf[OBJ1_ENTRY as usize + 2] = 0x00;
        buf[OBJ1_ENTRY as usize + 3] = 0x00;
        // parent=0 sibling=0 child=2
        buf[OBJ1_ENTRY as usize + 4] = 0; // parent
        buf[OBJ1_ENTRY as usize + 5] = 0; // sibling
        buf[OBJ1_ENTRY as usize + 6] = 2; // child
        // property table at PROP1_TBL
        put_word(&mut buf, OBJ1_ENTRY as usize + 7, PROP1_TBL as u16);

        // ── Object 2 entry ───────────────────────────────────────────────────
        // attr 7 = last bit of byte 0: 0x01
        // attr 8 = first bit of byte 1: 0x80
        buf[OBJ2_ENTRY as usize] = 0x01;  // attr 7 set
        buf[OBJ2_ENTRY as usize + 1] = 0x80; // attr 8 set
        buf[OBJ2_ENTRY as usize + 2] = 0x00;
        buf[OBJ2_ENTRY as usize + 3] = 0x00;
        buf[OBJ2_ENTRY as usize + 4] = 1; // parent = obj1
        buf[OBJ2_ENTRY as usize + 5] = 3; // sibling = obj3
        buf[OBJ2_ENTRY as usize + 6] = 0; // child = none
        put_word(&mut buf, OBJ2_ENTRY as usize + 7, PROP2_TBL as u16);

        // ── Object 3 entry ───────────────────────────────────────────────────
        buf[OBJ3_ENTRY as usize] = 0x00;
        buf[OBJ3_ENTRY as usize + 1] = 0x00;
        buf[OBJ3_ENTRY as usize + 2] = 0x00;
        buf[OBJ3_ENTRY as usize + 3] = 0x00;
        buf[OBJ3_ENTRY as usize + 4] = 1; // parent = obj1
        buf[OBJ3_ENTRY as usize + 5] = 0; // sibling = none
        buf[OBJ3_ENTRY as usize + 6] = 0; // child = none
        put_word(&mut buf, OBJ3_ENTRY as usize + 7, PROP3_TBL as u16);

        // ── Property table for obj1 ──────────────────────────────────────────
        // name: 2 words of Z-text for "west" (4 bytes from encode_word v3).
        let name1 = z_name("west");
        assert_eq!(name1.len(), 4); // 2 Z-words
        buf[PROP1_TBL as usize] = 2; // 2 words of name
        buf[PROP1_TBL as usize + 1..PROP1_TBL as usize + 5].copy_from_slice(&name1);
        // Property 10: 2 bytes of data = 0xABCD
        // size byte: bits 7-5 = (2-1)=1, bits 4-0 = 10 → 0b001_01010 = 0x2A
        buf[PROP1_TBL as usize + 5] = 0x2A; // size byte: size 2, prop 10
        buf[PROP1_TBL as usize + 6] = 0xAB;
        buf[PROP1_TBL as usize + 7] = 0xCD;
        // Property 5: 1 byte of data = 0x42
        // size byte: bits 7-5 = (1-1)=0, bits 4-0 = 5 → 0b000_00101 = 0x05
        buf[PROP1_TBL as usize + 8] = 0x05; // size byte: size 1, prop 5
        buf[PROP1_TBL as usize + 9] = 0x42;
        // End sentinel
        buf[PROP1_TBL as usize + 10] = 0x00;

        // ── Property table for obj2 ──────────────────────────────────────────
        let name2 = z_name("east");
        buf[PROP2_TBL as usize] = 2;
        buf[PROP2_TBL as usize + 1..PROP2_TBL as usize + 5].copy_from_slice(&name2);
        // End sentinel (no properties)
        buf[PROP2_TBL as usize + 5] = 0x00;

        // ── Property table for obj3 ──────────────────────────────────────────
        let name3 = z_name("hall");
        buf[PROP3_TBL as usize] = 2;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 5].copy_from_slice(&name3);
        buf[PROP3_TBL as usize + 5] = 0x00;

        // Property defaults: set prop 10's default to 0x1234 (0-indexed: prop 10 is at offset (10-1)*2 = 18).
        put_word(&mut buf, OBJ_TABLE as usize + (10 - 1) * 2, 0x1234);

        buf
    }

    // ── v3 attribute tests ────────────────────────────────────────────────────

    #[test]
    fn v3_get_attr_set_on_construction() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        assert!(get_attr(&m, 1, 0));   // obj1 attr 0
        assert!(!get_attr(&m, 1, 1));  // obj1 attr 1 clear
    }

    #[test]
    fn v3_set_and_clear_attr() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(!get_attr(&m, 1, 3));
        set_attr(&mut m, 1, 3);
        assert!(get_attr(&m, 1, 3));
        clear_attr(&mut m, 1, 3);
        assert!(!get_attr(&m, 1, 3));
    }

    #[test]
    fn v3_attr_cross_byte_boundary() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj2: attr 7 (byte 0, bit 0) and attr 8 (byte 1, bit 7)
        assert!(get_attr(&m, 2, 7));
        assert!(get_attr(&m, 2, 8));
        assert!(!get_attr(&m, 2, 6));
        assert!(!get_attr(&m, 2, 9));
    }

    #[test]
    fn v3_set_attr_at_boundary() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(!get_attr(&m, 2, 9));
        set_attr(&mut m, 2, 9);
        assert!(get_attr(&m, 2, 9));
        // attr 8 should still be set
        assert!(get_attr(&m, 2, 8));
    }

    // ── v3 tree pointer tests ─────────────────────────────────────────────────

    #[test]
    fn v3_tree_pointers() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_parent(&m, 1), 0);
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_parent(&m, 2), 1);
        assert_eq!(get_sibling(&m, 2), 3);
        assert_eq!(get_parent(&m, 3), 1);
        assert_eq!(get_sibling(&m, 3), 0);
    }

    // ── v3 insert_obj / remove_obj ────────────────────────────────────────────

    #[test]
    fn v3_remove_obj_first_child() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        // Remove obj2 (first child of obj1); obj3 should become first child.
        remove_obj(&mut m, 2);
        assert_eq!(get_child(&m, 1), 3);
        assert_eq!(get_parent(&m, 2), 0);
        assert_eq!(get_sibling(&m, 2), 0);
        // obj3 parent still 1
        assert_eq!(get_parent(&m, 3), 1);
    }

    #[test]
    fn v3_remove_obj_later_sibling() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        // Remove obj3 (sibling of obj2): obj2's sibling should become 0.
        remove_obj(&mut m, 3);
        assert_eq!(get_sibling(&m, 2), 0);
        assert_eq!(get_parent(&m, 3), 0);
    }

    #[test]
    fn v3_insert_obj_makes_first_child() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        // Insert obj3 into obj2 (obj3 currently child of obj1, sibling of obj2).
        insert_obj(&mut m, 3, 2);
        // obj3 is now first child of obj2
        assert_eq!(get_child(&m, 2), 3);
        assert_eq!(get_parent(&m, 3), 2);
        // obj3's sibling should be 0 (obj2 had no children before)
        assert_eq!(get_sibling(&m, 3), 0);
        // obj1's child list: obj2 (obj3 was removed from the sibling chain)
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_sibling(&m, 2), 0); // obj3 is gone from obj1's sibling list
    }

    // ── v3 short_name ─────────────────────────────────────────────────────────

    #[test]
    fn v3_short_name_obj1() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let name = short_name(&m, 1);
        assert!(name.starts_with("west"), "got: {:?}", name);
    }

    #[test]
    fn v3_short_name_obj2() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let name = short_name(&m, 2);
        assert!(name.starts_with("east"), "got: {:?}", name);
    }

    // ── v3 property tests ─────────────────────────────────────────────────────

    #[test]
    fn v3_get_prop_present() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj1 prop 10 = 0xABCD (2 bytes)
        assert_eq!(get_prop(&m, 1, 10), 0xABCD);
    }

    #[test]
    fn v3_get_prop_one_byte() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj1 prop 5 = 0x42 (1 byte)
        assert_eq!(get_prop(&m, 1, 5), 0x42);
    }

    #[test]
    fn v3_get_prop_defaults_fallback() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj2 has no properties; prop 10 falls back to default 0x1234
        assert_eq!(get_prop(&m, 2, 10), 0x1234);
    }

    #[test]
    fn v3_put_prop_two_bytes() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        put_prop(&mut m, 1, 10, 0x5678);
        assert_eq!(get_prop(&m, 1, 10), 0x5678);
    }

    #[test]
    fn v3_put_prop_one_byte() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        put_prop(&mut m, 1, 5, 0xFF);
        assert_eq!(get_prop(&m, 1, 5), 0xFF);
    }

    #[test]
    fn v3_get_prop_addr() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let addr = get_prop_addr(&m, 1, 10);
        assert_ne!(addr, 0);
        assert_eq!(m.read_word(addr as u32), 0xABCD);
    }

    #[test]
    fn v3_get_prop_addr_absent() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj2 has no properties
        assert_eq!(get_prop_addr(&m, 2, 10), 0);
    }

    #[test]
    fn v3_get_prop_len() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let addr = get_prop_addr(&m, 1, 10);
        assert_eq!(get_prop_len(&m, addr), 2);
        let addr5 = get_prop_addr(&m, 1, 5);
        assert_eq!(get_prop_len(&m, addr5), 1);
    }

    #[test]
    fn v3_get_next_prop() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // First prop of obj1 (prop=0 → first)
        let first = get_next_prop(&m, 1, 0);
        assert_eq!(first, 10); // highest prop first
        let next = get_next_prop(&m, 1, 10);
        assert_eq!(next, 5);
        let end = get_next_prop(&m, 1, 5);
        assert_eq!(end, 0); // no more
    }

    // ── v3 snapshot ──────────────────────────────────────────────────────────

    #[test]
    fn v3_object_snapshot() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let snap = object_snapshot(&m, 2);
        assert_eq!(snap.number, 2);
        assert_eq!(snap.parent, 1);
        assert!(snap.name.starts_with("east"), "got: {:?}", snap.name);
    }

    // ── v5 layout tests ───────────────────────────────────────────────────────
    //
    // sample_story(5) uses object_table = 0x0100.
    // v5 property-defaults: 63 words = 126 bytes → entries at 0x0100 + 0x7E = 0x017E.
    // Each v5 entry = 14 bytes:
    //   [0..5]   attributes (6 bytes)
    //   [6..7]   parent (word)
    //   [8..9]   sibling (word)
    //   [10..11] child (word)
    //   [12..13] property-table address (word)

    const ENTRIES_V5: u32 = OBJ_TABLE + 63 * 2; // = 0x017E
    const V5_OBJ1_ENTRY: u32 = ENTRIES_V5;        // 0x017E
    const V5_OBJ2_ENTRY: u32 = ENTRIES_V5 + 14;   // 0x018C
    const V5_OBJ3_ENTRY: u32 = ENTRIES_V5 + 28;   // 0x019A

    const V5_PROP1_TBL: u32 = 0x0280;
    const V5_PROP2_TBL: u32 = 0x02A0;
    const V5_PROP3_TBL: u32 = 0x02C0;

    fn build_v5_story() -> Vec<u8> {
        let mut buf = sample_story(5);

        // ── Object 1 entry ───────────────────────────────────────────────────
        // attr 0 set: byte 0 = 0x80
        buf[V5_OBJ1_ENTRY as usize] = 0x80;
        for i in 1..6 {
            buf[V5_OBJ1_ENTRY as usize + i] = 0;
        }
        // parent=0, sibling=0, child=2 (words)
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 6, 0);   // parent
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 8, 0);   // sibling
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 10, 2);  // child
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 12, V5_PROP1_TBL as u16);

        // ── Object 2 entry ───────────────────────────────────────────────────
        // attr 47 = last bit of byte 5 (bit 0) for v5 (48 attrs, 6 bytes)
        // attr 47: byte = 47/8 = 5, bit = 7 - (47%8) = 7 - 7 = 0 → 0x01
        buf[V5_OBJ2_ENTRY as usize] = 0x00;
        for i in 1..5 {
            buf[V5_OBJ2_ENTRY as usize + i] = 0;
        }
        buf[V5_OBJ2_ENTRY as usize + 5] = 0x01; // attr 47 set
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 6, 1);   // parent = obj1
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 8, 0);   // sibling = none
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 10, 0);  // child = none
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 12, V5_PROP2_TBL as u16);

        // ── Property table for obj1 (v5 two-byte property header) ────────────
        // name: "west" encoded (2 Z-words for v3 encoding; we use same 4-byte output)
        let name1 = encode_word("west", 5); // 6 bytes (3 Z-words) for v5
        let name_words = (name1.len() / 2) as u8; // 3 words
        buf[V5_PROP1_TBL as usize] = name_words;
        buf[V5_PROP1_TBL as usize + 1..V5_PROP1_TBL as usize + 1 + name1.len()]
            .copy_from_slice(&name1);
        let after_name = V5_PROP1_TBL as usize + 1 + name1.len();

        // Property 10, 2-byte data, using v5 one-byte form (bit 7 clear):
        // one-byte form: bit 6 set → 2 bytes of data; bits 5-0 = prop num 10 = 0b00_001010
        // so: bit7=0, bit6=1, bits5-0=10 → 0b0100_1010 = 0x4A
        buf[after_name] = 0x4A; // v5 one-byte size: 2 data bytes, prop 10
        buf[after_name + 1] = 0xAB;
        buf[after_name + 2] = 0xCD;
        // Property 5, 1-byte data, v5 one-byte form:
        // bit7=0, bit6=0, bits5-0=5 → 0b0000_0101 = 0x05
        buf[after_name + 3] = 0x05;
        buf[after_name + 4] = 0x42;
        buf[after_name + 5] = 0x00; // sentinel

        // ── Property table for obj2 ──────────────────────────────────────────
        let name2 = encode_word("east", 5);
        let name_words2 = (name2.len() / 2) as u8;
        buf[V5_PROP2_TBL as usize] = name_words2;
        buf[V5_PROP2_TBL as usize + 1..V5_PROP2_TBL as usize + 1 + name2.len()]
            .copy_from_slice(&name2);
        let after_name2 = V5_PROP2_TBL as usize + 1 + name2.len();
        buf[after_name2] = 0x00; // sentinel

        // Property default for prop 10: 0x9999 (63-word table, prop 10 at offset (10-1)*2=18)
        put_word(&mut buf, OBJ_TABLE as usize + (10 - 1) * 2, 0x9999);

        buf
    }

    #[test]
    fn v5_get_attr_set() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert!(get_attr(&m, 1, 0));
        assert!(!get_attr(&m, 1, 1));
    }

    #[test]
    fn v5_attr_47() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert!(get_attr(&m, 2, 47));
        assert!(!get_attr(&m, 2, 46));
    }

    #[test]
    fn v5_set_clear_attr() {
        let buf = build_v5_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(!get_attr(&m, 1, 31));
        set_attr(&mut m, 1, 31);
        assert!(get_attr(&m, 1, 31));
        clear_attr(&mut m, 1, 31);
        assert!(!get_attr(&m, 1, 31));
    }

    #[test]
    fn v5_tree_pointers() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_parent(&m, 1), 0);
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_parent(&m, 2), 1);
        assert_eq!(get_sibling(&m, 2), 0);
    }

    #[test]
    fn v5_insert_obj() {
        let buf = build_v5_story();
        let mut m = Memory::new(buf).unwrap();
        // obj2 is child of obj1; insert obj2 into obj1 again (no-op tree-wise but exercises path)
        // More useful: insert obj2 into itself? No. Let's just test remove then re-insert.
        remove_obj(&mut m, 2);
        assert_eq!(get_child(&m, 1), 0);
        insert_obj(&mut m, 2, 1);
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_parent(&m, 2), 1);
    }

    #[test]
    fn v5_short_name() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        let name = short_name(&m, 1);
        assert!(name.starts_with("west"), "got: {:?}", name);
    }

    #[test]
    fn v5_get_prop_present() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 1, 10), 0xABCD);
    }

    #[test]
    fn v5_get_prop_one_byte() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 1, 5), 0x42);
    }

    #[test]
    fn v5_get_prop_defaults_fallback() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 2, 10), 0x9999);
    }

    #[test]
    fn v5_put_prop() {
        let buf = build_v5_story();
        let mut m = Memory::new(buf).unwrap();
        put_prop(&mut m, 1, 10, 0xDEAD);
        assert_eq!(get_prop(&m, 1, 10), 0xDEAD);
    }

    #[test]
    fn v5_get_prop_addr_absent() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop_addr(&m, 2, 10), 0);
    }

    #[test]
    fn v5_get_next_prop() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        let first = get_next_prop(&m, 1, 0);
        assert_eq!(first, 10);
        let next = get_next_prop(&m, 1, 10);
        assert_eq!(next, 5);
        let end = get_next_prop(&m, 1, 5);
        assert_eq!(end, 0);
    }

    // ── v5 two-byte property header tests ────────────────────────────────────
    //
    // Builds on build_v5_story() by wiring up obj3 (at V5_OBJ3_ENTRY) with a
    // property table that uses the two-byte header form (ZMSD §12.4.2):
    //
    //   first byte:  0x80 | prop_num  (bit 7 set → two-byte form; bits 5-0 = prop num)
    //   second byte: 0x80 | size      (bit 7 set; bits 5-0 = data length; 0 → 64)
    //
    // Properties (descending order):
    //   prop 20: 4 bytes data (0xDEAD_BEEF)  — normal case, expect get_prop_len == 4
    //   prop 15: 64 bytes data (zeroed)       — escape case (size bits == 0), expect 64

    fn build_v5_two_byte_story() -> Vec<u8> {
        let mut buf = build_v5_story();

        // ── Object 3 entry (v5, 14 bytes) ───────────────────────────────────
        for i in 0..6 {
            buf[V5_OBJ3_ENTRY as usize + i] = 0; // no attributes
        }
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 6, 0);  // parent = none
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 8, 0);  // sibling = none
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 10, 0); // child = none
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 12, V5_PROP3_TBL as u16);

        // ── Property table for obj3 ──────────────────────────────────────────
        // Short name: "room" encoded for v5 (6 bytes, 3 Z-words)
        let name = encode_word("room", 5);
        let name_words = (name.len() / 2) as u8; // 3 words
        let base = V5_PROP3_TBL as usize;
        buf[base] = name_words;
        buf[base + 1..base + 1 + name.len()].copy_from_slice(&name);
        let mut p = base + 1 + name.len(); // first property byte offset

        // Property 20, 4 bytes data: two-byte header
        //   byte 0: 0x80 | 20 = 0x94
        //   byte 1: 0x80 | 4  = 0x84  (bits 5-0 = 4 → data length 4)
        buf[p]     = 0x94; // first size byte: two-byte form, prop 20
        buf[p + 1] = 0x84; // second size byte: length 4
        buf[p + 2] = 0xDE;
        buf[p + 3] = 0xAD;
        buf[p + 4] = 0xBE;
        buf[p + 5] = 0xEF;
        p += 6; // 2 header bytes + 4 data bytes

        // Property 15, 64 bytes data: two-byte header with length-escape
        //   byte 0: 0x80 | 15 = 0x8F
        //   byte 1: 0x80 | 0  = 0x80  (bits 5-0 = 0 → data length 64)
        buf[p]     = 0x8F; // first size byte: two-byte form, prop 15
        buf[p + 1] = 0x80; // second size byte: length escape → 64
        // 64 bytes of data (zeroed — the buffer is already 0 there)
        p += 2 + 64;

        // End sentinel
        buf[p] = 0x00;

        buf
    }

    #[test]
    fn v5_two_byte_property_header() {
        let buf = build_v5_two_byte_story();
        let m = Memory::new(buf).unwrap();

        // ── Normal case: prop 20, 4 bytes ────────────────────────────────────
        let addr20 = get_prop_addr(&m, 3, 20);
        assert_ne!(addr20, 0, "prop 20 should be present in obj3");
        // Data address must point just past the two size bytes.
        // The two size bytes are at addr20-2 (first) and addr20-1 (second).
        assert_eq!(m.read_byte(addr20 as u32 - 2), 0x94, "first size byte for prop 20");
        assert_eq!(m.read_byte(addr20 as u32 - 1), 0x84, "second size byte for prop 20");
        // get_prop_addr returns address of DATA, not header.
        assert_eq!(m.read_byte(addr20 as u32),     0xDE);
        assert_eq!(m.read_byte(addr20 as u32 + 1), 0xAD);
        assert_eq!(m.read_byte(addr20 as u32 + 2), 0xBE);
        assert_eq!(m.read_byte(addr20 as u32 + 3), 0xEF);
        // get_prop_len reads the byte immediately before prop_addr; for two-byte
        // form that is the SECOND size byte (0x84), so len = 0x84 & 0x3F = 4.
        assert_eq!(get_prop_len(&m, addr20), 4, "normal two-byte prop len should be 4");

        // ── Escape case: prop 15, 64 bytes ───────────────────────────────────
        let addr15 = get_prop_addr(&m, 3, 15);
        assert_ne!(addr15, 0, "prop 15 should be present in obj3");
        assert_eq!(m.read_byte(addr15 as u32 - 2), 0x8F, "first size byte for prop 15");
        assert_eq!(m.read_byte(addr15 as u32 - 1), 0x80, "second size byte for prop 15");
        // get_prop_len: second size byte = 0x80; 0x80 & 0x3F = 0 → returns 64.
        assert_eq!(get_prop_len(&m, addr15), 64, "escape two-byte prop len should be 64");
    }

    // ── Fixture test ──────────────────────────────────────────────────────────

    #[test]
    fn reads_object_tree_from_minizork() {
        let Some(story) = crate::fixtures::load("minizork.z3") else { return /* skip */ };
        let m = crate::memory::Memory::new(story).unwrap();
        let snap = object_snapshot(&m, 1);
        assert_eq!(snap.number, 1);
        assert!(!snap.name.is_empty());
    }
}
