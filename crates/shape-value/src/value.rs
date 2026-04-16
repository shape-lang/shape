//! Core value types for Shape
//!
//! This module defines supporting types used throughout the Shape runtime and VM.
//! The canonical runtime value representation is `ValueWord` (8-byte NaN-boxed).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::value_word::ValueWord;

/// Type alias for array storage — ValueWord elements for 9x memory reduction.
pub type VMArray = Arc<Vec<ValueWord>>;

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
