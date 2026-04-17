//! Function-signature registry for Phase F `Function<A, R>` dispatch.
//!
//! Every `Function<A, R>` value and every heap-allocated `TypedClosureHeader`
//! carries a `FunctionTypeId`. Two closures with the same callable signature
//! `(A) -> R` but different capture layouts (`ClosureTypeId`s) share a
//! `FunctionTypeId` — that's how `Array<Function<(int) -> int>>` works: the
//! element type constrains the signature, not the capture shape.
//!
//! This registry interns `FunctionSignature`s (parameter types + return type)
//! and hands out sequential `FunctionTypeId`s. The JIT uses the id to look up
//! a Cranelift call signature for `call_indirect`; the VM interpreter uses it
//! as a sanity tag (mismatches signal a compiler bug, not a user error).
//!
//! See `docs/v2-closure-specialization.md` §1.3 and §5.4.

use super::concrete_type::{ConcreteType, FunctionTypeId};
use std::collections::HashMap;

/// Signature of a callable value: ordered parameter types + a single return.
///
/// Keyed on `Vec<ConcreteType>` + `ConcreteType`; two callables with the same
/// signature share a `FunctionTypeId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionSignature {
    /// Parameter types in declaration order. Excludes captures — captures are
    /// implementation detail and belong in `ClosureLayout`, not in the
    /// cross-value `Function<A, R>` type.
    pub params: Vec<ConcreteType>,
    /// Return type. `ConcreteType::Void` for procedures.
    pub ret: ConcreteType,
}

impl FunctionSignature {
    /// Build a signature from its parts.
    pub fn new(params: Vec<ConcreteType>, ret: ConcreteType) -> Self {
        Self { params, ret }
    }

    /// Monomorphization key — used when the signature appears in a
    /// `mono_key()` (e.g. `Function<(int) -> int>`).
    pub fn mono_key(&self) -> String {
        let parts: Vec<_> = self.params.iter().map(|p| p.mono_key()).collect();
        format!("fnsig_{}__ret_{}", parts.join("_"), self.ret.mono_key())
    }
}

/// Registry of `FunctionSignature`s, keyed for O(1) intern + reverse lookup.
///
/// `FunctionTypeId`s are sequential `u32`s from 0. The registry is populated
/// lazily as the compiler encounters new `Function<A, R>` types during
/// monomorphization or closure emission.
#[derive(Debug, Default, Clone)]
pub struct FunctionTypeRegistry {
    signatures: Vec<FunctionSignature>,
    sig_to_id: HashMap<FunctionSignature, FunctionTypeId>,
}

impl FunctionTypeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a signature and return its `FunctionTypeId`. Idempotent —
    /// repeat calls with the same signature return the same id.
    pub fn intern(&mut self, sig: FunctionSignature) -> FunctionTypeId {
        if let Some(&id) = self.sig_to_id.get(&sig) {
            return id;
        }
        let id = FunctionTypeId(self.signatures.len() as u32);
        self.signatures.push(sig.clone());
        self.sig_to_id.insert(sig, id);
        id
    }

    /// Get the signature for a previously interned id.
    pub fn get(&self, id: FunctionTypeId) -> Option<&FunctionSignature> {
        self.signatures.get(id.0 as usize)
    }

    /// Look up an id by signature without interning.
    pub fn lookup(&self, sig: &FunctionSignature) -> Option<FunctionTypeId> {
        self.sig_to_id.get(sig).copied()
    }

    /// Number of distinct signatures interned.
    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    /// Iterate over `(id, signature)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (FunctionTypeId, &FunctionSignature)> {
        self.signatures
            .iter()
            .enumerate()
            .map(|(i, s)| (FunctionTypeId(i as u32), s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_registry() {
        let r = FunctionTypeRegistry::new();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn test_intern_same_signature_returns_same_id() {
        let mut r = FunctionTypeRegistry::new();
        let sig = FunctionSignature::new(vec![ConcreteType::I64], ConcreteType::I64);
        let a = r.intern(sig.clone());
        let b = r.intern(sig.clone());
        assert_eq!(a, b);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn test_intern_different_signatures_distinct_ids() {
        let mut r = FunctionTypeRegistry::new();
        let a = r.intern(FunctionSignature::new(vec![ConcreteType::I64], ConcreteType::I64));
        let b = r.intern(FunctionSignature::new(vec![ConcreteType::F64], ConcreteType::F64));
        let c = r.intern(FunctionSignature::new(vec![ConcreteType::I64], ConcreteType::F64));
        let d = r.intern(FunctionSignature::new(
            vec![ConcreteType::I64, ConcreteType::I64],
            ConcreteType::I64,
        ));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn test_get_roundtrip() {
        let mut r = FunctionTypeRegistry::new();
        let sig = FunctionSignature::new(
            vec![ConcreteType::I64, ConcreteType::String],
            ConcreteType::Bool,
        );
        let id = r.intern(sig.clone());
        let got = r.get(id).expect("signature should exist");
        assert_eq!(got, &sig);
    }

    #[test]
    fn test_lookup_without_intern() {
        let mut r = FunctionTypeRegistry::new();
        let sig = FunctionSignature::new(vec![ConcreteType::I64], ConcreteType::I64);
        assert!(r.lookup(&sig).is_none());
        let id = r.intern(sig.clone());
        assert_eq!(r.lookup(&sig), Some(id));
    }

    #[test]
    fn test_mono_key_format() {
        let sig = FunctionSignature::new(
            vec![ConcreteType::I64, ConcreteType::F64],
            ConcreteType::Bool,
        );
        assert_eq!(sig.mono_key(), "fnsig_i64_f64__ret_bool");
    }

    #[test]
    fn test_ids_sequential_from_zero() {
        let mut r = FunctionTypeRegistry::new();
        let a = r.intern(FunctionSignature::new(vec![], ConcreteType::Void));
        let b = r.intern(FunctionSignature::new(vec![ConcreteType::I64], ConcreteType::I64));
        let c = r.intern(FunctionSignature::new(vec![ConcreteType::F64], ConcreteType::F64));
        assert_eq!(a, FunctionTypeId(0));
        assert_eq!(b, FunctionTypeId(1));
        assert_eq!(c, FunctionTypeId(2));
    }

    #[test]
    fn test_iter_order() {
        let mut r = FunctionTypeRegistry::new();
        r.intern(FunctionSignature::new(vec![], ConcreteType::Void));
        r.intern(FunctionSignature::new(vec![ConcreteType::I64], ConcreteType::I64));
        let collected: Vec<_> = r.iter().collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0].0, FunctionTypeId(0));
        assert_eq!(collected[1].0, FunctionTypeId(1));
    }
}
