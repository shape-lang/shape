//! Tests for the `crypto` stdlib module.
//!
//! The crypto module provides: crypto::sha256, crypto::hmac_sha256,
//! crypto::base64_encode, crypto::base64_decode, crypto::hex_encode,
//! crypto::hex_decode. Imported via `use std::core::crypto`.

mod encoding;
mod hashing;
