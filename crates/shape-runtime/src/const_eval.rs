//! Const evaluator for metadata() handlers.
//!
//! Phase 1.B (ADR-006 §2.7.4 audit-accuracy ruling): the pre-bulldozer
//! evaluator decoded `&ValueWord`s via tag-bit dispatch (`as_f64()`,
//! `as_bool()`, `is_heap()`, `vw_equals`, `is_truthy`, ...) and
//! constructed values via `ValueWord::from_*`. After `ValueWord`'s
//! deletion the entire body is a deferred Phase 2c rebuild — the
//! kind-threaded const evaluator constructs `KindedSlot`s directly
//! from `Literal::*` arms (the kind is statically known per arm) and
//! dispatches operators on the slot bits + the carried `NativeKind`.
//!
//! There are no external callers of `ConstEvaluator` outside this
//! crate (verified via cross-crate grep) so this stub does not block
//! shape-vm or shape-jit. The file is preserved (rather than deleted)
//! because `lib.rs:29` re-exports the module path; deletion would
//! require coordination with the `lib.rs` module list.

use shape_ast::ast::Expr;
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;
use std::collections::HashMap;

/// Const evaluator for metadata() handlers.
#[derive(Debug, Clone, Default)]
pub struct ConstEvaluator {
    /// Annotation parameters available during evaluation.
    params: HashMap<String, KindedSlot>,
}

impl ConstEvaluator {
    /// Create a new const evaluator with no parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a const evaluator with annotation parameters.
    pub fn with_params(params: HashMap<String, KindedSlot>) -> Self {
        Self { params }
    }

    /// Add an annotation parameter to the scope.
    pub fn add_param(&mut self, name: String, value: KindedSlot) {
        self.params.insert(name, value);
    }

    /// Add an annotation parameter to the scope (alias kept for API
    /// compatibility — both forms now take a `KindedSlot`).
    pub fn add_param_nb(&mut self, name: String, value: KindedSlot) {
        self.params.insert(name, value);
    }

    /// Evaluate an expression as a const value.
    ///
    /// Phase 1.B: the kind-threaded const evaluator is deferred to
    /// Phase 2c. Until then, eval returns a deferred error rather than
    /// silently produce wrong values.
    pub fn eval(&self, _expr: &Expr) -> Result<KindedSlot> {
        Err(ShapeError::RuntimeError {
            message: "ConstEvaluator: pending Phase 2c kind-threaded rebuild — see ADR-006 §2.7.4".to_string(),
            location: None,
        })
    }

    /// Evaluate an expression as a const `KindedSlot` (alias kept for
    /// API compatibility with the deleted `eval_nb` shape).
    pub fn eval_as_nb(&self, expr: &Expr) -> Result<KindedSlot> {
        self.eval(expr)
    }
}

#[cfg(test)]
mod tests {
    // Pre-bulldozer tests of the evaluator covered the literal /
    // arithmetic / object / array / identifier paths. The kind-threaded
    // rebuild reintroduces them in Phase 2c.
}
