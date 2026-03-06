//! Ed25519 digital signature support for module manifests.
//!
//! Provides signing and verification of content-addressed module manifests
//! using Ed25519 key pairs.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Cryptographic signature data attached to a module manifest.
///
/// Contains the author's public key, the Ed25519 signature over the manifest
/// hash, and a timestamp recording when the signature was produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSignatureData {
    /// Ed25519 public key of the author (32 bytes).
    pub author_key: [u8; 32],
    /// Ed25519 signature bytes (64 bytes). Uses `Vec<u8>` because serde does
    /// not support `[u8; 64]` out of the box.
    pub signature: Vec<u8>,
    /// Unix timestamp (seconds) when the signature was created.
    pub signed_at: u64,
}

impl ModuleSignatureData {
    /// Sign a manifest hash with the given signing key.
    ///
    /// Produces a `ModuleSignatureData` containing the author's public key,
    /// the signature over `manifest_hash`, and the current timestamp.
    pub fn sign(manifest_hash: &[u8; 32], signing_key: &SigningKey) -> Self {
        let signature = signing_key.sign(manifest_hash);
        let author_key = signing_key.verifying_key().to_bytes();
        Self {
            author_key,
            signature: signature.to_bytes().to_vec(),
            signed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Verify that this signature is valid for the given manifest hash.
    ///
    /// Returns `true` if the Ed25519 signature is valid for the embedded
    /// author public key and the provided hash.
    pub fn verify(&self, manifest_hash: &[u8; 32]) -> bool {
        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.author_key) else {
            return false;
        };
        let Ok(sig_bytes): Result<[u8; 64], _> = self.signature.as_slice().try_into() else {
            return false;
        };
        let signature = Signature::from_bytes(&sig_bytes);
        verifying_key.verify(manifest_hash, &signature).is_ok()
    }
}

/// Sign a manifest hash using raw secret key bytes (32 bytes).
///
/// This is a convenience wrapper that avoids callers needing to depend on
/// `ed25519_dalek` directly.
pub fn sign_manifest_hash(
    manifest_hash: &[u8; 32],
    secret_key_bytes: &[u8; 32],
) -> ModuleSignatureData {
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    ModuleSignatureData::sign(manifest_hash, &signing_key)
}

/// Get the public key bytes for a given secret key.
pub fn public_key_from_secret(secret_key_bytes: &[u8; 32]) -> [u8; 32] {
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    signing_key.verifying_key().to_bytes()
}

/// Generate a new Ed25519 signing/verifying key pair.
pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
    let mut secret = [0u8; 32];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut secret);
    let signing_key = SigningKey::from_bytes(&secret);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

/// Generate a new Ed25519 key pair, returning raw byte arrays.
///
/// Returns `(secret_key_bytes, public_key_bytes)`. This avoids callers
/// needing to depend on `ed25519_dalek` types directly.
pub fn generate_keypair_bytes() -> ([u8; 32], [u8; 32]) {
    let (signing, verifying) = generate_keypair();
    (signing.to_bytes(), verifying.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let (signing_key, _) = generate_keypair();
        let manifest_hash = [42u8; 32];
        let sig = ModuleSignatureData::sign(&manifest_hash, &signing_key);
        assert!(sig.verify(&manifest_hash));
    }

    #[test]
    fn test_verify_fails_with_wrong_hash() {
        let (signing_key, _) = generate_keypair();
        let manifest_hash = [42u8; 32];
        let sig = ModuleSignatureData::sign(&manifest_hash, &signing_key);
        let wrong_hash = [99u8; 32];
        assert!(!sig.verify(&wrong_hash));
    }

    #[test]
    fn test_verify_fails_with_corrupt_signature() {
        let (signing_key, _) = generate_keypair();
        let manifest_hash = [42u8; 32];
        let mut sig = ModuleSignatureData::sign(&manifest_hash, &signing_key);
        sig.signature[0] ^= 0xFF;
        assert!(!sig.verify(&manifest_hash));
    }

    #[test]
    fn test_verify_fails_with_wrong_key() {
        let (signing_key, _) = generate_keypair();
        let (other_key, _) = generate_keypair();
        let manifest_hash = [42u8; 32];
        let mut sig = ModuleSignatureData::sign(&manifest_hash, &signing_key);
        sig.author_key = other_key.verifying_key().to_bytes();
        assert!(!sig.verify(&manifest_hash));
    }

    #[test]
    fn test_signed_at_is_nonzero() {
        let (signing_key, _) = generate_keypair();
        let sig = ModuleSignatureData::sign(&[0u8; 32], &signing_key);
        assert!(sig.signed_at > 0);
    }

    #[test]
    fn test_serde_roundtrip() {
        let (signing_key, _) = generate_keypair();
        let manifest_hash = [7u8; 32];
        let sig = ModuleSignatureData::sign(&manifest_hash, &signing_key);

        let json = serde_json::to_string(&sig).expect("serialize");
        let restored: ModuleSignatureData = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.author_key, sig.author_key);
        assert_eq!(restored.signature, sig.signature);
        assert_eq!(restored.signed_at, sig.signed_at);
        assert!(restored.verify(&manifest_hash));
    }
}
