//! FNV-1a, 128 bit. Content IDs for records.
//!
//! Chosen for the same reason minbin chooses fixed-width integers: anyone can
//! audit it in a minute and reproduce it in any language.

const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
const PRIME: u128 = 0x0000000001000000000000000000013b;

/// FNV-1a over `bytes`.
pub fn fnv1a_128(bytes: &[u8]) -> u128 {
    let mut h = OFFSET;
    for &b in bytes {
        h ^= u128::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::fnv1a_128;

    #[test]
    fn reference_vectors() {
        assert_eq!(fnv1a_128(b""), 0x6c62272e07bb014262b821756295c58d);
        assert_eq!(fnv1a_128(b"a"), 0xd228cb696f1a8caf78912b704e4a8964);
    }
}
