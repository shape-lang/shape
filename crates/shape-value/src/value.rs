//! Core value types for Shape
//!
//! This module defines supporting types used throughout the Shape runtime and VM.
//! The canonical runtime value representation is `ValueWord` (8-byte NaN-boxed).

use std::collections::HashMap;
use std::sync::Arc;

use crate::value_word::{ValueWord, ValueWordExt};
use crate::value_word_drop::{vw_clone, vw_drop};

/// Inline capacity for small VMArrays (≤ 8 elements stored inline, no heap buffer).
pub const VMARRAY_INLINE_CAP: usize = 8;

/// Backing buffer for VMArray — SmallVec with 8-element inline capacity.
/// Small arrays skip the separate Vec heap allocation entirely.
pub type VMArrayBuf = smallvec::SmallVec<[ValueWord; VMARRAY_INLINE_CAP]>;

/// Type alias for array storage — ValueWord elements for 9x memory reduction.
/// Small arrays (≤ 8 elements) are stored inline in the SmallVec; larger arrays spill to heap.
pub type VMArray = Arc<VMArrayBuf>;

/// Upvalue — a captured value held by a closure.
///
/// Track A.1C.3: `Upvalue` holds a single `ValueWord` per capture with
/// no auto-deref. Mutable captures (both `CaptureKind::OwnedMutable`
/// and `CaptureKind::Shared`) store raw pointer bits
/// (`*mut ValueWord` from `Box::into_raw` for OwnedMutable,
/// `*const SharedCell` from `Arc::into_raw` for Shared) in the
/// upvalue slot; the A.1B capture opcodes
/// (`Load/StoreOwnedMutableCapture`, `Load/StoreSharedCapture`) read
/// those bits directly via `clone_inner_bits_for_raw_pointer_access`
/// and dereference the pointer. Immutable captures carry the
/// ValueWord payload verbatim and are read via `get()` / written via
/// `set()` unchanged.
///
/// The retired-in-A.1C.3 legacy path (SharedCell-wrapped upvalue
/// populated by `BoxLocal` / `BoxModuleBinding`) is gone. `get()` and
/// `set()` no longer run any auto-deref.
///
/// Wave 4 WC.2: the inner `ValueWord` bit pattern may carry a heap
/// tag for `CaptureKind::Immutable` captures, in which case the
/// refcount must be paired — the manual `Clone` runs `vw_clone` to
/// bump the ref and the manual `Drop` runs `vw_drop` to release it.
/// For `OwnedMutable` / `Shared` captures the inner bits are raw
/// pointer values (`Box::into_raw` / `Arc::into_raw`) and are not
/// NaN-box tagged, so `vw_clone` / `vw_drop` both short-circuit to
/// no-ops; ownership of those Box/Arc allocations is tracked
/// separately through the capture opcodes.
#[derive(Debug)]
pub struct Upvalue(ValueWord);

impl Clone for Upvalue {
    fn clone(&self) -> Self {
        Upvalue(vw_clone(self.0))
    }
}

impl Drop for Upvalue {
    fn drop(&mut self) {
        vw_drop(self.0);
    }
}

impl Upvalue {
    /// Create a new upvalue carrying `value`.
    ///
    /// Track A.1C.3: the caller is responsible for installing the
    /// right capture form — raw pointer bits for
    /// `CaptureKind::OwnedMutable` / `CaptureKind::Shared`, a plain
    /// ValueWord for `CaptureKind::Immutable`. There is no longer a
    /// `HeapValue::SharedCell` fallback; unrecognised forms read back
    /// whatever bits were installed.
    #[inline]
    pub fn new(value: ValueWord) -> Self {
        Upvalue(value)
    }

    /// Read the captured value. Returns the stored `ValueWord`
    /// unchanged. For `OwnedMutable` / `Shared` captures this is the
    /// raw pointer bits and callers must dereference via the A.1B
    /// capture opcodes — plain `get()` is appropriate only for
    /// `Immutable` captures.
    #[inline]
    pub fn get(&self) -> ValueWord {
        self.0.clone()
    }

    /// Write the captured ValueWord. Replaces the stored bits. For
    /// `OwnedMutable` / `Shared` captures the A.1B capture opcodes
    /// write through the pointer directly; they do not call `set()`.
    pub fn set(&mut self, value: ValueWord) {
        self.0 = value;
    }

    /// Track A.1B: return the raw inner bits of the upvalue's
    /// `ValueWord`. Used by the A.1B capture opcodes to recover the
    /// raw pointer bits for `OwnedMutable` / `Shared` captures.
    #[inline]
    pub fn clone_inner_bits_for_raw_pointer_access(&self) -> u64 {
        self.0
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
    /// A closure implementation of a trait method.
    ///
    /// Track A.3: VTable closure entries now carry `(function_id,
    /// type_id)` instead of a legacy `Vec<Upvalue>`. Dispatch allocates
    /// a fresh `OwnedClosureBlock` per call via the program's
    /// `closure_function_layouts` registry so the call convention sees
    /// the same raw `TypedClosureHeader` shape that
    /// `op_make_closure` emits today.
    ///
    /// No producer for this variant ships today — vtable closure
    /// methods have never been constructed by the compiler. The
    /// representation is kept (a) so `VTableEntry` remains expressive
    /// enough to describe closure-as-method dispatch if the compiler
    /// lights up the feature, and (b) so Track A.5 can retire
    /// `HeapValue::Closure` without a data-shape edit to the vtable
    /// carrier.
    Closure { function_id: u32, type_id: u32 },
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
