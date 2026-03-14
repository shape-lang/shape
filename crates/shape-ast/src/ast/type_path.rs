//! Module-qualified type path for Shape AST
//!
//! `TypePath` represents a potentially module-qualified type reference as structured
//! segments. For example, `foo::Bar` is represented as `["foo", "Bar"]` and plain
//! `Bar` as `["Bar"]`.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

/// A potentially module-qualified type reference.
///
/// Stores structured segments (e.g. `["foo", "Bar"]` for `foo::Bar`) along with
/// a cached `qualified` string (`"foo::Bar"`).
///
/// Key trait impls make migration mechanical:
/// - `Deref<Target=str>` returns `&self.qualified`
/// - `Borrow<str>` enables `HashMap<String,..>::get(&type_path)`
/// - `PartialEq<str>`, `PartialEq<&str>`, `PartialEq<String>` for comparisons
/// - `From<String>`, `From<&str>` for construction
/// - Serializes as plain string for backward compatibility
#[derive(Clone, Debug)]
pub struct TypePath {
    segments: Vec<String>,
    qualified: String,
}

impl TypePath {
    /// Create a single-segment (unqualified) type path.
    pub fn simple(name: impl Into<String>) -> Self {
        let name = name.into();
        TypePath {
            segments: vec![name.clone()],
            qualified: name,
        }
    }

    /// Create a multi-segment (potentially qualified) type path.
    pub fn from_segments(segments: Vec<String>) -> Self {
        let qualified = segments.join("::");
        TypePath {
            segments,
            qualified,
        }
    }

    /// Create from a qualified string, splitting on `::`.
    pub fn from_qualified(s: impl Into<String>) -> Self {
        let s = s.into();
        let segments: Vec<String> = s.split("::").map(|seg| seg.to_string()).collect();
        TypePath {
            segments,
            qualified: s,
        }
    }

    /// The type's own name (last segment).
    pub fn name(&self) -> &str {
        self.segments.last().map(|s| s.as_str()).unwrap_or("")
    }

    /// Module segments (everything before the last).
    pub fn module_segments(&self) -> &[String] {
        if self.segments.len() > 1 {
            &self.segments[..self.segments.len() - 1]
        } else {
            &[]
        }
    }

    /// Whether this path has more than one segment.
    pub fn is_qualified(&self) -> bool {
        self.segments.len() > 1
    }

    /// The full qualified string.
    pub fn as_str(&self) -> &str {
        &self.qualified
    }

    /// The individual segments.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

// ---- Deref to &str ----

impl Deref for TypePath {
    type Target = str;
    fn deref(&self) -> &str {
        &self.qualified
    }
}

impl Borrow<str> for TypePath {
    fn borrow(&self) -> &str {
        &self.qualified
    }
}

impl AsRef<str> for TypePath {
    fn as_ref(&self) -> &str {
        &self.qualified
    }
}

// ---- Equality / Hash (based on qualified string) ----

impl PartialEq for TypePath {
    fn eq(&self, other: &Self) -> bool {
        self.qualified == other.qualified
    }
}

impl Eq for TypePath {}

impl Hash for TypePath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.qualified.hash(state);
    }
}

impl PartialEq<str> for TypePath {
    fn eq(&self, other: &str) -> bool {
        self.qualified == other
    }
}

impl PartialEq<&str> for TypePath {
    fn eq(&self, other: &&str) -> bool {
        self.qualified.as_str() == *other
    }
}

impl PartialEq<String> for TypePath {
    fn eq(&self, other: &String) -> bool {
        self.qualified == *other
    }
}

impl PartialEq<TypePath> for str {
    fn eq(&self, other: &TypePath) -> bool {
        self == other.qualified
    }
}

impl PartialEq<TypePath> for &str {
    fn eq(&self, other: &TypePath) -> bool {
        *self == other.qualified.as_str()
    }
}

impl PartialEq<TypePath> for String {
    fn eq(&self, other: &TypePath) -> bool {
        *self == other.qualified
    }
}

// ---- Display ----

impl fmt::Display for TypePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.qualified)
    }
}

// ---- From conversions ----

impl From<String> for TypePath {
    fn from(s: String) -> Self {
        TypePath::from_qualified(s)
    }
}

impl From<&str> for TypePath {
    fn from(s: &str) -> Self {
        TypePath::from_qualified(s)
    }
}

// ---- Serialize as plain string, Deserialize from plain string ----

impl Serialize for TypePath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.qualified.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TypePath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(TypePath::from_qualified(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_path() {
        let p = TypePath::simple("Foo");
        assert_eq!(p.as_str(), "Foo");
        assert_eq!(p.name(), "Foo");
        assert!(!p.is_qualified());
        assert!(p.module_segments().is_empty());
    }

    #[test]
    fn test_qualified_path() {
        let p = TypePath::from_segments(vec!["foo".into(), "Bar".into()]);
        assert_eq!(p.as_str(), "foo::Bar");
        assert_eq!(p.name(), "Bar");
        assert!(p.is_qualified());
        assert_eq!(p.module_segments(), &["foo".to_string()]);
    }

    #[test]
    fn test_deeply_qualified() {
        let p = TypePath::from_segments(vec!["a".into(), "b".into(), "C".into()]);
        assert_eq!(p.as_str(), "a::b::C");
        assert_eq!(p.name(), "C");
        assert_eq!(p.module_segments(), &["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn test_deref_str() {
        let p = TypePath::simple("Foo");
        let s: &str = &p;
        assert_eq!(s, "Foo");
    }

    #[test]
    fn test_eq_str() {
        let p = TypePath::simple("Foo");
        assert!(p == "Foo");
        assert!("Foo" == p);
        assert!(p == "Foo".to_string());
    }

    #[test]
    fn test_from_string() {
        let p: TypePath = "foo::Bar".to_string().into();
        assert!(p.is_qualified());
        assert_eq!(p.name(), "Bar");
    }

    #[test]
    fn test_from_str() {
        let p: TypePath = "Baz".into();
        assert!(!p.is_qualified());
    }

    #[test]
    fn test_serde_roundtrip() {
        let p = TypePath::from_segments(vec!["mod".into(), "Type".into()]);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"mod::Type\"");
        let p2: TypePath = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn test_hashmap_lookup() {
        use std::collections::HashMap;
        let mut map: HashMap<String, i32> = HashMap::new();
        map.insert("foo::Bar".to_string(), 42);
        let p = TypePath::from_qualified("foo::Bar");
        // Use Borrow<str> to look up
        assert_eq!(map.get(p.as_str()), Some(&42));
    }
}
