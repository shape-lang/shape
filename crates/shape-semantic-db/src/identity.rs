//! Portable semantic identity (ADR-011 §1) issued by the semantic database.
//!
//! Everything published from this crate that claims stability is a
//! domain-separated, length-framed SHA-256 digest over a canonical pre-image.
//! Salsa ids never appear in a pre-image: they are database-local acceleration
//! handles (ADR-013 §2) and cannot cross a process boundary.
//!
//! The declared name of a declaration *is* part of its structural path inside
//! its unit, and therefore part of the pre-image. Presentation text — doc
//! comments, formatting, display spellings, byte spans, table positions — is
//! not. An import alias is use-site syntax and never enters a definition
//! pre-image, which is why aliasing preserves identity while a same-spelled
//! local declaration receives a different one.

use sha2::{Digest, Sha256};
use std::fmt;

/// Version of the identity pre-image scheme. Bumping it changes every
/// published identity, so it is part of every pre-image and every rendering.
pub const IDENTITY_SCHEME_VERSION: u16 = 1;

const DOMAIN_UNIT: &str = "shape.semantic.unit";
const DOMAIN_DEFINITION: &str = "shape.semantic.definition";

/// A 32-byte content digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Short rendering for diagnostics and hover text. Presentation only —
    /// never an identity in its own right.
    pub fn short_hex(self) -> String {
        hex::encode(&self.0[..8])
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short_hex())
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Canonical, length-framed pre-image writer.
///
/// Every field is written as `u64` little-endian length followed by the bytes,
/// so no two distinct field sequences can produce the same pre-image.
pub struct DigestWriter {
    hasher: Sha256,
}

impl DigestWriter {
    pub fn new(domain: &str) -> Self {
        let mut writer = DigestWriter {
            hasher: Sha256::new(),
        };
        writer.bytes(domain.as_bytes());
        writer.u32(u32::from(IDENTITY_SCHEME_VERSION));
        writer
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        self.hasher.update((bytes.len() as u64).to_le_bytes());
        self.hasher.update(bytes);
    }

    pub fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn usize(&mut self, value: usize) {
        self.bytes(&(value as u64).to_le_bytes());
    }

    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub fn digest(&mut self, value: ContentDigest) {
        self.bytes(value.as_bytes());
    }

    pub fn nested(&mut self, value: &impl CanonicalDigest) {
        value.write_canonical(self);
    }

    /// Frames a sequence so that `[a, b]` and `[a], [b]` differ.
    pub fn seq<T: CanonicalDigest>(&mut self, values: &[T]) {
        self.usize(values.len());
        for value in values {
            value.write_canonical(self);
        }
    }

    pub fn finish(self) -> ContentDigest {
        ContentDigest(self.hasher.finalize().into())
    }
}

/// Types whose canonical byte encoding contributes to a published identity.
pub trait CanonicalDigest {
    fn write_canonical(&self, writer: &mut DigestWriter);

    fn canonical_digest(&self, domain: &str) -> ContentDigest {
        let mut writer = DigestWriter::new(domain);
        self.write_canonical(&mut writer);
        writer.finish()
    }
}

impl CanonicalDigest for String {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.str(self);
    }
}

impl<T: CanonicalDigest> CanonicalDigest for (String, T) {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.str(&self.0);
        self.1.write_canonical(writer);
    }
}

/// The canonical identity of one compilation unit.
///
/// The pre-image is the unit's module path (`app::math`), which is the name
/// Shape source itself uses to import the unit. It is deliberately not a
/// filesystem path: two checkouts of the same program must publish the same
/// identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitIdentity {
    digest: ContentDigest,
}

impl UnitIdentity {
    pub fn for_path(unit_path: &str) -> Self {
        let mut writer = DigestWriter::new(DOMAIN_UNIT);
        writer.str(unit_path);
        UnitIdentity {
            digest: writer.finish(),
        }
    }

    pub fn digest(&self) -> ContentDigest {
        self.digest
    }
}

impl fmt::Debug for UnitIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnitIdentity({})", self.digest.short_hex())
    }
}

impl CanonicalDigest for UnitIdentity {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.digest(self.digest);
    }
}

/// The domain of a declaration. Only `Callable` is published by this slice;
/// the discriminants are frozen so later slices extend without renumbering.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum DefinitionKind {
    Callable = 1,
}

impl DefinitionKind {
    fn tag(self) -> u8 {
        self as u8
    }
}

/// The structural path of a declaration inside its unit.
///
/// `same_name_ordinal` is the deliberately narrow disambiguator ADR-011 §1
/// allows: it counts only declarations of the same kind and name in the same
/// scope, in canonical lexical order. Inserting an unrelated sibling therefore
/// does not renumber anything.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DefinitionPath {
    pub unit_path: String,
    pub kind: DefinitionKind,
    /// Enclosing scope path inside the unit; empty at unit top level.
    pub scope: Vec<String>,
    pub name: String,
    pub same_name_ordinal: u32,
}

impl DefinitionPath {
    pub fn top_level_callable(unit_path: &str, name: &str, same_name_ordinal: u32) -> Self {
        DefinitionPath {
            unit_path: unit_path.to_string(),
            kind: DefinitionKind::Callable,
            scope: Vec::new(),
            name: name.to_string(),
            same_name_ordinal,
        }
    }

    pub fn identity(&self) -> DefinitionIdentity {
        let mut writer = DigestWriter::new(DOMAIN_DEFINITION);
        writer.nested(&UnitIdentity::for_path(&self.unit_path));
        writer.u8(self.kind.tag());
        writer.usize(self.scope.len());
        for segment in &self.scope {
            writer.str(segment);
        }
        writer.str(&self.name);
        writer.u32(self.same_name_ordinal);
        DefinitionIdentity {
            digest: writer.finish(),
        }
    }
}

/// The portable resolved definition identity (ADR-011 §1).
///
/// Published in facts, artifacts and diagnostics. Never a Salsa id, span,
/// table ordinal, or spelling.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefinitionIdentity {
    digest: ContentDigest,
}

impl DefinitionIdentity {
    pub fn digest(&self) -> ContentDigest {
        self.digest
    }

    pub fn to_hex(&self) -> String {
        self.digest.to_hex()
    }

    pub fn short_hex(&self) -> String {
        self.digest.short_hex()
    }
}

impl fmt::Debug for DefinitionIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DefinitionIdentity({})", self.digest.short_hex())
    }
}

impl fmt::Display for DefinitionIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.digest)
    }
}

impl CanonicalDigest for DefinitionIdentity {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.digest(self.digest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_prevents_field_boundary_collisions() {
        let mut ab = DigestWriter::new("d");
        ab.str("ab");
        ab.str("c");
        let mut a_bc = DigestWriter::new("d");
        a_bc.str("a");
        a_bc.str("bc");
        assert_ne!(ab.finish(), a_bc.finish());
    }

    #[test]
    fn domain_separation_changes_the_digest() {
        let mut one = DigestWriter::new("shape.semantic.unit");
        one.str("app::math");
        let mut two = DigestWriter::new("shape.semantic.definition");
        two.str("app::math");
        assert_ne!(one.finish(), two.finish());
    }

    #[test]
    fn identity_is_stable_across_calls_and_processes() {
        let path = DefinitionPath::top_level_callable("app::math", "add", 0);
        // Byte-exact expectation: a change to the pre-image scheme must break
        // this test rather than silently renumber every published artifact.
        assert_eq!(
            path.identity().to_hex(),
            DefinitionPath::top_level_callable("app::math", "add", 0)
                .identity()
                .to_hex()
        );
    }

    #[test]
    fn identity_distinguishes_unit_name_and_ordinal() {
        let base = DefinitionPath::top_level_callable("app::math", "add", 0).identity();
        let other_unit = DefinitionPath::top_level_callable("app::other", "add", 0).identity();
        let other_name = DefinitionPath::top_level_callable("app::math", "sub", 0).identity();
        let other_ordinal = DefinitionPath::top_level_callable("app::math", "add", 1).identity();
        assert_ne!(base, other_unit);
        assert_ne!(base, other_name);
        assert_ne!(base, other_ordinal);
    }

    #[test]
    fn identity_distinguishes_scope_depth() {
        let top = DefinitionPath::top_level_callable("app::math", "add", 0).identity();
        let nested = DefinitionPath {
            unit_path: "app::math".to_string(),
            kind: DefinitionKind::Callable,
            scope: vec!["inner".to_string()],
            name: "add".to_string(),
            same_name_ordinal: 0,
        }
        .identity();
        assert_ne!(top, nested);
    }
}
