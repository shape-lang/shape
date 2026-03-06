//! Direct-mapped lossy cache for megamorphic property lookups.
//!
//! When a property access site has seen too many different schemas
//! (Megamorphic state), we use this global cache as a last-resort
//! optimization before falling back to name-based lookup.

const CACHE_SIZE: usize = 1024;

#[derive(Debug, Clone, Copy)]
pub struct MegamorphicEntry {
    pub key: u64,
    pub field_idx: u16,
    pub field_type_tag: u16,
    pub valid: bool,
}

impl Default for MegamorphicEntry {
    fn default() -> Self {
        Self {
            key: 0,
            field_idx: 0,
            field_type_tag: 0,
            valid: false,
        }
    }
}

pub struct MegamorphicCache {
    entries: Box<[MegamorphicEntry; CACHE_SIZE]>,
}

impl MegamorphicCache {
    /// Creates a new cache with all entries invalid.
    pub fn new() -> Self {
        Self {
            entries: Box::new([MegamorphicEntry::default(); CACHE_SIZE]),
        }
    }

    /// Combines a schema_id and field name into a hash key.
    ///
    /// Uses FNV-1a-inspired mixing: fold the field name bytes into the
    /// schema_id with multiply-xor rounds.
    pub fn hash_key(schema_id: u64, field_name: &str) -> u64 {
        // FNV-1a offset basis mixed with schema_id
        let mut hash: u64 = 0xcbf29ce484222325 ^ schema_id;
        for byte in field_name.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    /// Probes the cache for a matching entry.
    /// Returns `Some((field_idx, field_type_tag))` on hit, `None` on miss.
    pub fn probe(&self, key: u64) -> Option<(u16, u16)> {
        let idx = (key as usize) % CACHE_SIZE;
        let entry = &self.entries[idx];
        if entry.valid && entry.key == key {
            Some((entry.field_idx, entry.field_type_tag))
        } else {
            None
        }
    }

    /// Inserts an entry at the direct-mapped position (key % CACHE_SIZE).
    /// Overwrites any existing entry at that index.
    pub fn insert(&mut self, key: u64, field_idx: u16, field_type_tag: u16) {
        let idx = (key as usize) % CACHE_SIZE;
        self.entries[idx] = MegamorphicEntry {
            key,
            field_idx,
            field_type_tag,
            valid: true,
        };
    }

    /// Marks all entries as invalid.
    pub fn invalidate_all(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.valid = false;
        }
    }

    /// Returns the ratio of valid entries (diagnostic).
    pub fn hit_rate(&self) -> f64 {
        let valid_count = self.entries.iter().filter(|e| e.valid).count();
        valid_count as f64 / CACHE_SIZE as f64
    }
}

impl Default for MegamorphicCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cache_empty() {
        let cache = MegamorphicCache::new();
        assert_eq!(cache.hit_rate(), 0.0);
        // Probe any key should miss
        assert_eq!(cache.probe(12345), None);
    }

    #[test]
    fn test_insert_and_probe_hit() {
        let mut cache = MegamorphicCache::new();
        let key = MegamorphicCache::hash_key(42, "name");
        cache.insert(key, 3, 7);

        let result = cache.probe(key);
        assert_eq!(result, Some((3, 7)));
    }

    #[test]
    fn test_probe_miss() {
        let mut cache = MegamorphicCache::new();
        let key1 = MegamorphicCache::hash_key(42, "name");
        let key2 = MegamorphicCache::hash_key(42, "age");
        cache.insert(key1, 3, 7);

        // Different key at potentially different index
        // Even if same index, key won't match
        assert_eq!(cache.probe(key2), None);
    }

    #[test]
    fn test_hash_key_consistency() {
        let k1 = MegamorphicCache::hash_key(100, "field_a");
        let k2 = MegamorphicCache::hash_key(100, "field_a");
        assert_eq!(k1, k2);

        // Different field name -> different key
        let k3 = MegamorphicCache::hash_key(100, "field_b");
        assert_ne!(k1, k3);

        // Different schema -> different key
        let k4 = MegamorphicCache::hash_key(200, "field_a");
        assert_ne!(k1, k4);
    }

    #[test]
    fn test_invalidate_all() {
        let mut cache = MegamorphicCache::new();
        for i in 0..10u64 {
            let key = MegamorphicCache::hash_key(i, "x");
            cache.insert(key, i as u16, 0);
        }
        assert!(cache.hit_rate() > 0.0);

        cache.invalidate_all();
        assert_eq!(cache.hit_rate(), 0.0);

        // Previously inserted keys should miss
        let key = MegamorphicCache::hash_key(0, "x");
        assert_eq!(cache.probe(key), None);
    }

    #[test]
    fn test_collision_overwrites() {
        let mut cache = MegamorphicCache::new();

        // Two keys that map to the same index
        let key1 = 100u64;
        let key2 = key1 + CACHE_SIZE as u64; // same index: both % 1024 == 100

        cache.insert(key1, 1, 10);
        assert_eq!(cache.probe(key1), Some((1, 10)));

        // Overwrite with key2 at same index
        cache.insert(key2, 2, 20);
        assert_eq!(cache.probe(key2), Some((2, 20)));

        // key1 is now evicted (same slot, different key)
        assert_eq!(cache.probe(key1), None);
    }
}
