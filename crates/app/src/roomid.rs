//! RoomId policy for name-only rooms (no backing Z-machine object).
//!
//! RoomIds with the high bit set are synthetic: derived from a room's displayed
//! name when it could not be resolved to a game object. The high bit guarantees
//! no collision with real object numbers (no IF game has >= 32768 objects).

/// Set on a RoomId to mark it a name-only (non-object) room.
pub const SYNTHETIC_ROOM_FLAG: u16 = 0x8000;

/// True when `id` denotes a name-only room (high bit set).
pub fn is_synthetic_room(id: u16) -> bool {
    id & SYNTHETIC_ROOM_FLAG != 0
}

/// Deterministic, save/reload-stable RoomId for a name-only room. Normalizes the
/// name (trim, collapse whitespace, lowercase) then FNV-1a hashes it into the
/// low 15 bits, with the high bit set.
pub fn synthetic_room_id(name: &str) -> u16 {
    let norm: String = name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let mut h: u32 = 0x811c_9dc5;
    for b in norm.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    SYNTHETIC_ROOM_FLAG | (h as u16 & 0x7FFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_id_high_bit_set_and_deterministic() {
        let a = synthetic_room_id("Bedroom");
        assert_eq!(a & SYNTHETIC_ROOM_FLAG, SYNTHETIC_ROOM_FLAG, "high bit set");
        assert_eq!(a, synthetic_room_id("Bedroom"), "deterministic");
        assert!(is_synthetic_room(a));
        assert!(!is_synthetic_room(150)); // a real object number
    }

    #[test]
    fn synthetic_id_normalizes_name() {
        assert_eq!(synthetic_room_id("Bedroom"), synthetic_room_id("  bedroom  "));
        assert_eq!(synthetic_room_id("Foo Bar"), synthetic_room_id("foo   bar"));
    }

    #[test]
    fn synthetic_id_differs_for_distinct_names() {
        assert_ne!(synthetic_room_id("Bedroom"), synthetic_room_id("Kitchen"));
    }
}
