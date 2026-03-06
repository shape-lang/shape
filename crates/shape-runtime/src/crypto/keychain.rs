//! Trusted author keychain for module signature verification.
//!
//! The `Keychain` stores a set of trusted Ed25519 public keys with associated
//! trust levels, and provides verification of module signatures against the
//! trust policy.

use super::signing::ModuleSignatureData;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How much trust is granted to a particular author key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Trusted for all modules.
    Full,
    /// Trusted only for modules whose names match one of the listed prefixes.
    Scoped(Vec<String>),
    /// Trusted only for a single specific manifest hash.
    Pinned([u8; 32]),
}

/// A trusted author entry in the keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedAuthor {
    /// Human-readable name for this author.
    pub name: String,
    /// Ed25519 public key (32 bytes).
    pub public_key: [u8; 32],
    /// Trust level governing which modules this key may sign.
    pub trust_level: TrustLevel,
}

/// A keychain managing trusted author keys and module signature verification.
#[derive(Clone)]
pub struct Keychain {
    trusted: HashMap<[u8; 32], TrustedAuthor>,
    require_signatures: bool,
}

/// Result of verifying a module against the keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyResult {
    /// Signature is valid and the author is trusted for this module.
    Trusted,
    /// No signature present and signatures are not required.
    Unsigned,
    /// Verification failed for the given reason.
    Rejected(String),
}

impl Keychain {
    /// Create a new keychain.
    ///
    /// When `require_signatures` is `true`, unsigned modules are rejected.
    pub fn new(require_signatures: bool) -> Self {
        Self {
            trusted: HashMap::new(),
            require_signatures,
        }
    }

    /// Add or replace a trusted author in the keychain.
    pub fn add_trusted(&mut self, author: TrustedAuthor) {
        self.trusted.insert(author.public_key, author);
    }

    /// Remove a trusted author by public key.
    ///
    /// Returns the removed author, or `None` if the key was not in the keychain.
    pub fn remove_trusted(&mut self, public_key: &[u8; 32]) -> Option<TrustedAuthor> {
        self.trusted.remove(public_key)
    }

    /// Check whether the given public key is trusted for a module with the
    /// specified name and manifest hash.
    pub fn is_trusted(
        &self,
        public_key: &[u8; 32],
        module_name: &str,
        manifest_hash: &[u8; 32],
    ) -> bool {
        let Some(author) = self.trusted.get(public_key) else {
            return false;
        };
        match &author.trust_level {
            TrustLevel::Full => true,
            TrustLevel::Scoped(prefixes) => prefixes
                .iter()
                .any(|prefix| module_name.starts_with(prefix)),
            TrustLevel::Pinned(pinned_hash) => pinned_hash == manifest_hash,
        }
    }

    /// Verify a module's signature against the keychain trust policy.
    ///
    /// Checks:
    /// 1. If no signature is present, passes only when signatures are not required.
    /// 2. Cryptographic validity of the Ed25519 signature.
    /// 3. The signing key is in the keychain and trusted for this module.
    pub fn verify_module(
        &self,
        module_name: &str,
        manifest_hash: &[u8; 32],
        signature: Option<&ModuleSignatureData>,
    ) -> VerifyResult {
        let Some(sig) = signature else {
            return if self.require_signatures {
                VerifyResult::Rejected("module is unsigned and signatures are required".into())
            } else {
                VerifyResult::Unsigned
            };
        };

        if !sig.verify(manifest_hash) {
            return VerifyResult::Rejected("invalid signature".into());
        }

        if !self.is_trusted(&sig.author_key, module_name, manifest_hash) {
            return VerifyResult::Rejected(format!(
                "author key {} is not trusted for module '{}'",
                hex::encode(sig.author_key),
                module_name,
            ));
        }

        VerifyResult::Trusted
    }

    /// Whether this keychain requires all modules to be signed.
    pub fn requires_signatures(&self) -> bool {
        self.require_signatures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::signing::generate_keypair;

    fn make_author(name: &str, key: [u8; 32], trust: TrustLevel) -> TrustedAuthor {
        TrustedAuthor {
            name: name.to_string(),
            public_key: key,
            trust_level: trust,
        }
    }

    #[test]
    fn test_unsigned_allowed_when_not_required() {
        let kc = Keychain::new(false);
        let result = kc.verify_module("my_mod", &[0u8; 32], None);
        assert_eq!(result, VerifyResult::Unsigned);
    }

    #[test]
    fn test_unsigned_rejected_when_required() {
        let kc = Keychain::new(true);
        let result = kc.verify_module("my_mod", &[0u8; 32], None);
        assert!(matches!(result, VerifyResult::Rejected(_)));
    }

    #[test]
    fn test_full_trust_verifies() {
        let (signing_key, verifying_key) = generate_keypair();
        let mut kc = Keychain::new(true);
        kc.add_trusted(make_author(
            "alice",
            verifying_key.to_bytes(),
            TrustLevel::Full,
        ));

        let hash = [1u8; 32];
        let sig = ModuleSignatureData::sign(&hash, &signing_key);
        assert_eq!(
            kc.verify_module("anything", &hash, Some(&sig)),
            VerifyResult::Trusted
        );
    }

    #[test]
    fn test_scoped_trust_allows_matching_prefix() {
        let (signing_key, verifying_key) = generate_keypair();
        let mut kc = Keychain::new(true);
        kc.add_trusted(make_author(
            "bob",
            verifying_key.to_bytes(),
            TrustLevel::Scoped(vec!["std::".to_string()]),
        ));

        let hash = [2u8; 32];
        let sig = ModuleSignatureData::sign(&hash, &signing_key);
        assert_eq!(
            kc.verify_module("std::core::math", &hash, Some(&sig)),
            VerifyResult::Trusted
        );
    }

    #[test]
    fn test_scoped_trust_rejects_non_matching() {
        let (signing_key, verifying_key) = generate_keypair();
        let mut kc = Keychain::new(true);
        kc.add_trusted(make_author(
            "bob",
            verifying_key.to_bytes(),
            TrustLevel::Scoped(vec!["std::".to_string()]),
        ));

        let hash = [2u8; 32];
        let sig = ModuleSignatureData::sign(&hash, &signing_key);
        let result = kc.verify_module("vendor::malware", &hash, Some(&sig));
        assert!(matches!(result, VerifyResult::Rejected(_)));
    }

    #[test]
    fn test_pinned_trust_matching_hash() {
        let (signing_key, verifying_key) = generate_keypair();
        let pinned_hash = [5u8; 32];
        let mut kc = Keychain::new(true);
        kc.add_trusted(make_author(
            "carol",
            verifying_key.to_bytes(),
            TrustLevel::Pinned(pinned_hash),
        ));

        let sig = ModuleSignatureData::sign(&pinned_hash, &signing_key);
        assert_eq!(
            kc.verify_module("some_mod", &pinned_hash, Some(&sig)),
            VerifyResult::Trusted
        );
    }

    #[test]
    fn test_pinned_trust_wrong_hash() {
        let (signing_key, verifying_key) = generate_keypair();
        let pinned_hash = [5u8; 32];
        let mut kc = Keychain::new(true);
        kc.add_trusted(make_author(
            "carol",
            verifying_key.to_bytes(),
            TrustLevel::Pinned(pinned_hash),
        ));

        let different_hash = [6u8; 32];
        let sig = ModuleSignatureData::sign(&different_hash, &signing_key);
        let result = kc.verify_module("some_mod", &different_hash, Some(&sig));
        assert!(matches!(result, VerifyResult::Rejected(_)));
    }

    #[test]
    fn test_untrusted_key_rejected() {
        let (signing_key, _) = generate_keypair();
        let kc = Keychain::new(true);

        let hash = [3u8; 32];
        let sig = ModuleSignatureData::sign(&hash, &signing_key);
        let result = kc.verify_module("my_mod", &hash, Some(&sig));
        assert!(matches!(result, VerifyResult::Rejected(_)));
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let (signing_key, verifying_key) = generate_keypair();
        let mut kc = Keychain::new(true);
        kc.add_trusted(make_author(
            "dave",
            verifying_key.to_bytes(),
            TrustLevel::Full,
        ));

        let hash = [4u8; 32];
        let mut sig = ModuleSignatureData::sign(&hash, &signing_key);
        sig.signature[0] ^= 0xFF; // corrupt
        let result = kc.verify_module("mod", &hash, Some(&sig));
        assert!(matches!(result, VerifyResult::Rejected(_)));
    }

    #[test]
    fn test_remove_trusted() {
        let (_, verifying_key) = generate_keypair();
        let mut kc = Keychain::new(false);
        let key_bytes = verifying_key.to_bytes();
        kc.add_trusted(make_author("eve", key_bytes, TrustLevel::Full));
        assert!(kc.is_trusted(&key_bytes, "any", &[0u8; 32]));

        let removed = kc.remove_trusted(&key_bytes);
        assert!(removed.is_some());
        assert!(!kc.is_trusted(&key_bytes, "any", &[0u8; 32]));
    }
}
