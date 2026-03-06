//! Strongly-typed newtype wrappers for numeric identifiers.
//!
//! These newtypes prevent accidental misuse of raw `u16`/`u32`/`u64` values
//! in different identifier domains (function IDs, string pool indices, schema IDs, etc.).

use serde::{Deserialize, Serialize};

/// A function identifier. Indexes into `BytecodeProgram::functions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct FunctionId(pub u16);

impl FunctionId {
    /// Create a new FunctionId from a raw u16.
    #[inline]
    pub const fn new(id: u16) -> Self {
        Self(id)
    }

    /// Get the raw u16 value.
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Convert to usize for indexing.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<u16> for FunctionId {
    #[inline]
    fn from(id: u16) -> Self {
        Self(id)
    }
}

impl From<FunctionId> for u16 {
    #[inline]
    fn from(id: FunctionId) -> u16 {
        id.0
    }
}

impl From<FunctionId> for usize {
    #[inline]
    fn from(id: FunctionId) -> usize {
        id.0 as usize
    }
}

impl std::fmt::Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fn#{}", self.0)
    }
}

/// A string pool identifier. Indexes into `BytecodeProgram::strings`.
///
/// Using `StringId` instead of a heap-allocated `String` makes
/// `Operand` (and therefore `Instruction`) `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct StringId(pub u32);

impl StringId {
    /// Create a new StringId from a raw u32.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw u32 value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Convert to usize for indexing.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for StringId {
    #[inline]
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl From<StringId> for u32 {
    #[inline]
    fn from(id: StringId) -> u32 {
        id.0
    }
}

impl From<StringId> for usize {
    #[inline]
    fn from(id: StringId) -> usize {
        id.0 as usize
    }
}

impl std::fmt::Display for StringId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "str#{}", self.0)
    }
}

/// A type schema identifier. Indexes into the type schema registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct SchemaId(pub u32);

impl SchemaId {
    /// Create a new SchemaId from a raw u32.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw u32 value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Convert to usize for indexing.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for SchemaId {
    #[inline]
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl From<SchemaId> for u32 {
    #[inline]
    fn from(id: SchemaId) -> u32 {
        id.0
    }
}

impl From<SchemaId> for usize {
    #[inline]
    fn from(id: SchemaId) -> usize {
        id.0 as usize
    }
}

impl std::fmt::Display for SchemaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "schema#{}", self.0)
    }
}

/// A stack slot index. Used for local variables and temporary values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct StackSlotIdx(pub usize);

impl StackSlotIdx {
    /// Create a new StackSlotIdx.
    #[inline]
    pub const fn new(idx: usize) -> Self {
        Self(idx)
    }

    /// Get the raw usize value.
    #[inline]
    pub const fn raw(self) -> usize {
        self.0
    }
}

impl From<usize> for StackSlotIdx {
    #[inline]
    fn from(idx: usize) -> Self {
        Self(idx)
    }
}

impl From<StackSlotIdx> for usize {
    #[inline]
    fn from(idx: StackSlotIdx) -> usize {
        idx.0
    }
}

impl std::fmt::Display for StackSlotIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "slot#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_id_conversions() {
        let id = FunctionId::new(42);
        assert_eq!(id.raw(), 42u16);
        assert_eq!(id.index(), 42usize);
        assert_eq!(u16::from(id), 42u16);
        assert_eq!(usize::from(id), 42usize);
        assert_eq!(FunctionId::from(42u16), id);
    }

    #[test]
    fn test_string_id_conversions() {
        let id = StringId::new(100);
        assert_eq!(id.raw(), 100u32);
        assert_eq!(id.index(), 100usize);
        assert_eq!(u32::from(id), 100u32);
        assert_eq!(usize::from(id), 100usize);
        assert_eq!(StringId::from(100u32), id);
    }

    #[test]
    fn test_schema_id_conversions() {
        let id = SchemaId::new(7);
        assert_eq!(id.raw(), 7u32);
        assert_eq!(id.index(), 7usize);
        assert_eq!(u32::from(id), 7u32);
        assert_eq!(SchemaId::from(7u32), id);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", FunctionId::new(5)), "fn#5");
        assert_eq!(format!("{}", StringId::new(10)), "str#10");
        assert_eq!(format!("{}", SchemaId::new(3)), "schema#3");
        assert_eq!(format!("{}", StackSlotIdx::new(0)), "slot#0");
    }

    #[test]
    fn test_different_types_not_comparable() {
        // This is a compile-time check — these should NOT compile:
        // let _: FunctionId = StringId::new(1); // error: mismatched types
        // let _: StringId = FunctionId::new(1); // error: mismatched types
        // Just verify they're different types
        let fn_id = FunctionId::new(1);
        let str_id = StringId::new(1);
        assert_ne!(
            std::any::TypeId::of::<FunctionId>(),
            std::any::TypeId::of::<StringId>()
        );
        // Can't accidentally mix them, even though the raw values are the same
        let _ = (fn_id, str_id);
    }
}
