//! Core value types for Shape
//!
//! This module defines supporting types used throughout the Shape runtime and VM.
//! The canonical runtime value representation is `ValueWord` (8-byte NaN-boxed).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::value_word::ValueWord;

/// Inline capacity for small VMArrays (≤ 8 elements stored inline, no heap buffer).
pub const VMARRAY_INLINE_CAP: usize = 8;

/// Backing buffer for VMArray — SmallVec with 8-element inline capacity.
/// Small arrays skip the separate Vec heap allocation entirely.
pub type VMArrayBuf = smallvec::SmallVec<[ValueWord; VMARRAY_INLINE_CAP]>;

/// Type alias for array storage — ValueWord elements for 9x memory reduction.
/// Small arrays (≤ 8 elements) are stored inline in the SmallVec; larger arrays spill to heap.
pub type VMArray = Arc<VMArrayBuf>;

/// Upvalue - a captured variable that can be shared between closures and their enclosing scope.
///
/// Most captures are immutable (the compiler never emits `StoreClosure`), so the
/// `Immutable` variant stores the ValueWord directly — no Arc, no RwLock, just an
/// 8-byte clone.  The `Mutable` variant preserves the old `Arc<RwLock<ValueWord>>`
/// path for the rare case where a capture is written to.
#[derive(Debug, Clone)]
pub enum Upvalue {
    /// Direct value — no lock, no Arc overhead.  Clone is 8 bytes (or Arc bump for heap values).
    Immutable(ValueWord),
    /// Shared mutable capture — used if `StoreClosure` is ever emitted.
    Mutable(Arc<RwLock<ValueWord>>),
}

impl Upvalue {
    /// Create a new **immutable** upvalue (fast path — default for all captures).
    #[inline]
    pub fn new(value: ValueWord) -> Self {
        Upvalue::Immutable(value)
    }

    /// Create a new **mutable** upvalue (slow path — only when writes are needed).
    #[inline]
    pub fn new_mutable(value: ValueWord) -> Self {
        Upvalue::Mutable(Arc::new(RwLock::new(value)))
    }

    /// Get a clone of the contained ValueWord value.
    #[inline]
    pub fn get(&self) -> ValueWord {
        match self {
            Upvalue::Immutable(nb) => nb.clone(),
            Upvalue::Mutable(arc) => arc.read().unwrap().clone(),
        }
    }

    /// Set the contained value.
    ///
    /// If the upvalue is `Immutable`, it is upgraded to `Mutable` on the first write.
    /// This requires `&mut self`.
    pub fn set(&mut self, value: ValueWord) {
        match self {
            Upvalue::Mutable(arc) => {
                *arc.write().unwrap() = value;
            }
            Upvalue::Immutable(_) => {
                // Upgrade to mutable on first write
                *self = Upvalue::Mutable(Arc::new(RwLock::new(value)));
            }
        }
    }
}

/// Print result with structured spans for reformattable output
#[derive(Debug, Clone)]
pub struct PrintResult {
    /// Rendered output string (cached)
    pub rendered: String,

    /// Structured spans with metadata for reformatting
    pub spans: Vec<PrintSpan>,
}

/// A span in print output (literal text or formatted value)
#[derive(Debug, Clone)]
pub enum PrintSpan {
    /// Literal text span
    Literal {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
    },

    /// Value span with formatting metadata
    Value {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
        variable_name: Option<String>,
        raw_value: Box<ValueWord>,
        type_name: String,
        current_format: String,
        format_params: HashMap<String, ValueWord>,
    },
}

/// A callable closure backed by native Rust code with captured state.
/// Used by extension modules to return objects with callable methods (e.g., DbTable.filter).
#[derive(Clone)]
pub struct HostCallable {
    inner: Arc<dyn Fn(&[ValueWord]) -> Result<ValueWord, String> + Send + Sync>,
}

impl HostCallable {
    /// Create a new HostCallable from a closure
    pub fn new(
        f: impl Fn(&[ValueWord]) -> Result<ValueWord, String> + Send + Sync + 'static,
    ) -> Self {
        Self { inner: Arc::new(f) }
    }

    /// Call the host closure with the given arguments
    pub fn call(&self, args: &[ValueWord]) -> Result<ValueWord, String> {
        (self.inner)(args)
    }
}

impl std::fmt::Debug for HostCallable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<host_closure>")
    }
}

/// Comparison operator for filter expressions
#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A literal value in a filter expression (for SQL generation)
#[derive(Debug, Clone, PartialEq)]
pub enum FilterLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

/// Filter expression tree for SQL pushdown.
/// Built from ExprProxy comparisons and logical operations.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterNode {
    /// Column compared to a literal value
    Compare {
        column: String,
        op: FilterOp,
        value: FilterLiteral,
    },
    /// Logical AND of two filter nodes
    And(Box<FilterNode>, Box<FilterNode>),
    /// Logical OR of two filter nodes
    Or(Box<FilterNode>, Box<FilterNode>),
    /// Logical NOT of a filter node
    Not(Box<FilterNode>),
}

/// Virtual method table for trait objects.
///
/// Maps method names to function IDs for dynamic dispatch.
/// Created when a concrete value is boxed into a `dyn Trait`.
#[derive(Debug, Clone)]
pub struct VTable {
    /// Trait names this vtable implements
    pub trait_names: Vec<String>,
    /// Map from method name to function ID (bytecode offset or closure)
    pub methods: HashMap<String, VTableEntry>,
}

/// An entry in a vtable — how to dispatch a method call
#[derive(Debug, Clone)]
pub enum VTableEntry {
    /// A compiled function by ID
    FunctionId(u16),
    /// A closure with captured upvalues
    Closure {
        function_id: u16,
        upvalues: Vec<Upvalue>,
    },
}

/// Create a VMArray from an iterator of ValueWord values.
pub fn vmarray_from_value_words(iter: impl IntoIterator<Item = ValueWord>) -> VMArray {
    Arc::new(iter.into_iter().collect())
}

/// Backward-compatibility alias.
pub fn vmarray_from_nanboxed(iter: impl IntoIterator<Item = ValueWord>) -> VMArray {
    vmarray_from_value_words(iter)
}

/// Wrap a buffer of `ValueWord`s as a VMArray.
///
/// Accepts either a `Vec<ValueWord>` or an existing `VMArrayBuf`. When the
/// buffer has 8 or fewer elements the data is moved inline into the SmallVec,
/// skipping the separate heap buffer allocation that a plain `Arc<Vec<_>>`
/// would incur. Larger inputs reuse the existing heap buffer.
#[inline]
pub fn vmarray_from_vec<V: Into<VMArrayBuf>>(v: V) -> VMArray {
    Arc::new(v.into())
}

#[cfg(test)]
mod inline_vmarray_tests {
    //! Verify the small-array-inline behavior of `VMArray`.
    //!
    //! `VMArray = Arc<SmallVec<[ValueWord; 8]>>` so arrays with up to 8
    //! elements store their data inline in the SmallVec (no separate heap
    //! buffer), while arrays with 9+ elements spill transparently.
    use super::*;
    use crate::value_word::{ValueWord, ValueWordExt};

    #[test]
    fn empty_small_array_is_inline() {
        let arr: VMArray = vmarray_from_vec(Vec::<ValueWord>::new());
        assert_eq!(arr.len(), 0);
        assert!(!arr.spilled(), "empty SmallVec must stay inline");
    }

    #[test]
    fn small_array_up_to_inline_cap_stays_inline() {
        for n in 1..=VMARRAY_INLINE_CAP {
            let v: Vec<ValueWord> = (0..n as i64).map(ValueWord::from_i64).collect();
            let arr: VMArray = vmarray_from_vec(v);
            assert_eq!(arr.len(), n);
            assert!(
                !arr.spilled(),
                "len={n} (≤ {VMARRAY_INLINE_CAP}) should not spill to heap"
            );
            for i in 0..n {
                assert_eq!(arr[i].as_i64(), Some(i as i64));
            }
        }
    }

    #[test]
    fn large_array_spills_to_heap() {
        let big = VMARRAY_INLINE_CAP + 4;
        let v: Vec<ValueWord> = (0..big as i64).map(ValueWord::from_i64).collect();
        let arr: VMArray = vmarray_from_vec(v);
        assert_eq!(arr.len(), big);
        assert!(arr.spilled(), "len={big} must spill to heap");
        assert_eq!(arr[0].as_i64(), Some(0));
        assert_eq!(arr[big - 1].as_i64(), Some((big - 1) as i64));
    }

    #[test]
    fn push_on_unique_owner_transitions_inline_to_heap() {
        let mut buf = VMArrayBuf::new();
        for i in 0..VMARRAY_INLINE_CAP as i64 {
            buf.push(ValueWord::from_i64(i));
        }
        assert!(!buf.spilled(), "at inline capacity we must still be inline");
        // One past inline capacity triggers spill.
        buf.push(ValueWord::from_i64(99));
        assert!(buf.spilled(), "exceeding inline cap must spill");
        assert_eq!(buf.len(), VMARRAY_INLINE_CAP + 1);
        assert_eq!(buf[VMARRAY_INLINE_CAP].as_i64(), Some(99));
    }

    #[test]
    fn iter_and_index_work_inline_and_spilled() {
        // Inline case.
        let small: VMArray = vmarray_from_vec(vec![
            ValueWord::from_i64(10),
            ValueWord::from_i64(20),
            ValueWord::from_i64(30),
        ]);
        let sum_small: i64 = small.iter().map(|v| v.as_i64().unwrap()).sum();
        assert_eq!(sum_small, 60);
        assert_eq!(small[1].as_i64(), Some(20));

        // Spilled case.
        let v: Vec<ValueWord> = (0..32i64).map(ValueWord::from_i64).collect();
        let big: VMArray = vmarray_from_vec(v);
        assert!(big.spilled());
        let sum_big: i64 = big.iter().map(|v| v.as_i64().unwrap()).sum();
        assert_eq!(sum_big, (0..32i64).sum::<i64>());
        assert_eq!(big[31].as_i64(), Some(31));
    }

    #[test]
    fn vmarray_size_on_stack_is_reasonable() {
        // 8 inline ValueWords (8 bytes each) + len/cap metadata.
        // Upper bound: 8 * 8 + 24 = 88 bytes, allow slight slack.
        let sz = std::mem::size_of::<VMArrayBuf>();
        assert!(
            sz <= 96,
            "VMArrayBuf should be <=96 bytes on stack, got {sz}",
        );
    }
}
