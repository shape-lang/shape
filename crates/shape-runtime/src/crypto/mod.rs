//! Cryptographic utilities for module signing and trust verification.
//!
//! This module provides Ed25519 digital signatures for content-addressed module
//! manifests and a keychain-based trust model for verifying module authors.

pub mod keychain;
pub mod signing;

pub use keychain::{Keychain, TrustLevel, TrustedAuthor, VerifyResult};
pub use signing::{
    ModuleSignatureData, generate_keypair, generate_keypair_bytes, public_key_from_secret,
    sign_manifest_hash,
};
