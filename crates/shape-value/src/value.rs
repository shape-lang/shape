//! Surviving typed value types after the strict-typing bulldozer.
//!
//! Most of this module's content (VMArray, Upvalue, HostCallable, PrintResult,
//! PrintSpan) was deleted along with the v1 ValueWord representation. What
//! remains are the pure-data filter / vtable types that don't reference any
//! dynamic-word machinery.

use std::collections::HashMap;

/// Comparison operator for filter expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A literal value in a filter expression (for SQL generation).
#[derive(Debug, Clone, PartialEq)]
pub enum FilterLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

/// Filter expression tree for SQL pushdown.
///
/// Built from comparisons and logical operations. Represents typed
/// column-vs-literal predicates suitable for pushdown to a SQL backend.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterNode {
    /// Column compared to a literal value.
    Compare {
        column: String,
        op: FilterOp,
        value: FilterLiteral,
    },
    /// Logical AND of two filter nodes.
    And(Box<FilterNode>, Box<FilterNode>),
    /// Logical OR of two filter nodes.
    Or(Box<FilterNode>, Box<FilterNode>),
    /// Logical NOT of a filter node.
    Not(Box<FilterNode>),
}

/// Virtual method table for trait objects.
///
/// Maps method names to function IDs for dynamic dispatch. Created when a
/// concrete value is boxed into a `dyn Trait`.
#[derive(Debug, Clone)]
pub struct VTable {
    /// Trait names this vtable implements.
    pub trait_names: Vec<String>,
    /// Map from method name to function ID (bytecode offset or closure).
    pub methods: HashMap<String, VTableEntry>,
}

/// An entry in a vtable — how to dispatch a method call.
#[derive(Debug, Clone)]
pub enum VTableEntry {
    /// A compiled function by ID.
    FunctionId(u16),
    /// A closure implementation of a trait method.
    ///
    /// Track A.3: VTable closure entries carry `(function_id, type_id)`;
    /// dispatch allocates a fresh `OwnedClosureBlock` per call via the
    /// program's `closure_function_layouts` registry so the call convention
    /// sees the same raw `TypedClosureHeader` shape that `op_make_closure`
    /// emits.
    Closure { function_id: u32, type_id: u32 },
}
