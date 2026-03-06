//! SHA256 hashing utilities for reproducibility artifacts
//!
//! Provides deterministic hashing for:
//! - Script content
//! - Data files/checksums
//! - Parameter configurations
//! - Combined artifact hashes

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

/// Hash result as hex string with prefix
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HashDigest(pub String);

impl HashDigest {
    /// Create from raw hex string (adds sha256: prefix if missing)
    pub fn from_hex(hex: &str) -> Self {
        if hex.starts_with("sha256:") {
            HashDigest(hex.to_string())
        } else {
            HashDigest(format!("sha256:{}", hex))
        }
    }

    /// Get the raw hex string without prefix
    pub fn hex(&self) -> &str {
        self.0.strip_prefix("sha256:").unwrap_or(&self.0)
    }

    /// Get the full string with prefix
    pub fn full(&self) -> &str {
        &self.0
    }

    /// Check if this hash matches another (prefix-agnostic)
    pub fn matches(&self, other: &HashDigest) -> bool {
        self.hex() == other.hex()
    }
}

impl std::fmt::Display for HashDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Hash a string (script content, etc.)
pub fn hash_string(content: &str) -> HashDigest {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    HashDigest::from_hex(&hex::encode(result))
}

/// Hash a byte slice
pub fn hash_bytes(data: &[u8]) -> HashDigest {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    HashDigest::from_hex(&hex::encode(result))
}

/// Hash a file by reading its contents
pub fn hash_file(path: &Path) -> io::Result<HashDigest> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let result = hasher.finalize();
    Ok(HashDigest::from_hex(&hex::encode(result)))
}

/// Hash parameters in a deterministic way (sorted keys)
pub fn hash_parameters(params: &BTreeMap<String, serde_json::Value>) -> HashDigest {
    // Serialize to JSON with sorted keys for determinism
    let json = serde_json::to_string(params).unwrap_or_default();
    hash_string(&json)
}

/// Combine multiple hashes into one
pub fn combine_hashes(hashes: &[&HashDigest]) -> HashDigest {
    let mut hasher = Sha256::new();
    for hash in hashes {
        hasher.update(hash.hex().as_bytes());
        hasher.update(b"|"); // Separator
    }
    let result = hasher.finalize();
    HashDigest::from_hex(&hex::encode(result))
}

/// Hash multiple files and combine
pub fn hash_data_files(paths: &[&Path]) -> io::Result<HashDigest> {
    let mut hashes = Vec::new();
    for path in paths {
        hashes.push(hash_file(path)?);
    }

    let refs: Vec<&HashDigest> = hashes.iter().collect();
    Ok(combine_hashes(&refs))
}

/// Hex encoding/decoding (simple implementation to avoid extra dependency)
mod hex {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(data: impl AsRef<[u8]>) -> String {
        let bytes = data.as_ref();
        let mut result = String::with_capacity(bytes.len() * 2);
        for &byte in bytes {
            result.push(HEX_CHARS[(byte >> 4) as usize] as char);
            result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_string() {
        let hash = hash_string("hello world");
        assert!(hash.full().starts_with("sha256:"));
        assert_eq!(hash.hex().len(), 64); // SHA256 produces 32 bytes = 64 hex chars
    }

    #[test]
    fn test_hash_deterministic() {
        let hash1 = hash_string("test content");
        let hash2 = hash_string("test content");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_different_content() {
        let hash1 = hash_string("content a");
        let hash2 = hash_string("content b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_combine_hashes() {
        let h1 = hash_string("first");
        let h2 = hash_string("second");
        let combined = combine_hashes(&[&h1, &h2]);

        // Combined should be different from individual
        assert_ne!(combined, h1);
        assert_ne!(combined, h2);

        // Combining should be deterministic
        let combined2 = combine_hashes(&[&h1, &h2]);
        assert_eq!(combined, combined2);
    }

    #[test]
    fn test_hash_parameters() {
        let mut params = BTreeMap::new();
        params.insert("period".to_string(), serde_json::json!(20));
        params.insert("threshold".to_string(), serde_json::json!(0.5));

        let hash1 = hash_parameters(&params);
        let hash2 = hash_parameters(&params);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_digest_matches() {
        let h1 = HashDigest::from_hex("abc123");
        let h2 = HashDigest::from_hex("sha256:abc123");
        assert!(h1.matches(&h2));
    }
}
