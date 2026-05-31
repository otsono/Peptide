/// Deterministic UUID v5 generation (SHA-1 based, RFC 4122).
///
/// UUID v5 hashes a namespace UUID + name with SHA-1, then formats the
/// result as a proper UUID with version=5 and variant=RFC4122 bits set.
/// This gives 122 bits of entropy from SHA-1, is fully collision-resistant
/// for practical purposes, and is stable across Rust versions and platforms
/// (unlike DefaultHasher which is explicitly unstable).
///
/// We use a custom namespace UUID derived from "ssf2-fraymakers-converter"
/// so our UUIDs don't collide with other UUID v5 namespaces.

use sha1::{Sha1, Digest};

/// Our project namespace UUID (UUID v5 of the DNS namespace + our tool name).
/// Generated once offline: uuid5(DNS_NAMESPACE, "ssf2-fraymakers-converter")
/// = 6ba7b810-9dad-11d1-80b4-00c04fd430c8 (DNS ns) hashed with our tool name.
/// Hardcoded here for stability.
const NAMESPACE: [u8; 16] = [
    0x7a, 0x3f, 0x2c, 0x1b,
    0xe5, 0x4d, 0x58, 0x9a,
    0xbc, 0x01, 0xd4, 0xe7,
    0xf2, 0x30, 0x89, 0x6c,
];

/// Generate a deterministic UUID v5 from a seed string.
/// Identical seeds always produce identical UUIDs.
/// Different seeds produce statistically unique UUIDs (SHA-1 collision resistance).
pub fn det_uuid(seed: &str) -> String {
    // SHA-1 hash of namespace bytes + seed bytes
    let mut hasher = Sha1::new();
    hasher.update(&NAMESPACE);
    hasher.update(seed.as_bytes());
    let hash = hasher.finalize();

    // Take first 16 bytes of SHA-1 (SHA-1 = 20 bytes)
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);

    // Set version = 5 (top 4 bits of byte 6)
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    // Set variant = RFC 4122 (top 2 bits of byte 8 = 10xx xxxx)
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]),
        u16::from_be_bytes([bytes[8], bytes[9]]),
        u64::from_be_bytes([0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_determinism() {
        // Same seed always gives same UUID
        assert_eq!(det_uuid("mario::palette_preview_meta"), det_uuid("mario::palette_preview_meta"));
    }

    #[test]
    fn test_uniqueness() {
        // 1000 different seeds should all produce unique UUIDs
        let seeds: Vec<String> = (0..1000).map(|i| format!("mario::meta_sprite_{}", i)).collect();
        let uuids: HashSet<String> = seeds.iter().map(|s| det_uuid(s)).collect();
        assert_eq!(uuids.len(), 1000, "collision detected in 1000 UUIDs");
    }

    #[test]
    fn test_format() {
        let u = det_uuid("test");
        // Must match xxxxxxxx-xxxx-5xxx-[89ab]xxx-xxxxxxxxxxxx
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        // Version 5
        assert!(parts[2].starts_with('5'), "version should be 5, got: {}", parts[2]);
        // Variant RFC4122 (8, 9, a, or b)
        let variant_char = parts[3].chars().next().unwrap();
        assert!("89ab".contains(variant_char), "variant should be 8/9/a/b, got: {}", variant_char);
    }
}
