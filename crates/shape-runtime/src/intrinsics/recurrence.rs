//! Recurrence intrinsics — partial migration to typed marshal layer
//! (cross-crate-dual-consumer pattern; see `docs/defections.md` 2026-05-07
//! intrinsics-typed-CC entry's N1 sub-decision).
//!
//! `__intrinsic_linear_recurrence` has two distinct consumers:
//!
//! 1. **Compiler-typed-call-sites consumer** — typed entry via
//!    [`create_recurrence_intrinsics_module`] (`register_typed_fn_3_full`
//!    using the N1 `FromSlot<Option<f64>>` impl from `marshal.rs`).
//!
//! 2. **Shape-vm `BuiltinFunction` dispatcher consumer** — references the
//!    legacy [`intrinsic_linear_recurrence`] body directly from
//!    `crates/shape-vm/src/executor/builtins/runtime_delegated.rs:138`.
//!    No `vm_intrinsic_linear_recurrence` shape-vm-side parallel copy exists
//!    (unlike rolling/random/distributions which have `vm_intrinsic_*`
//!    copies in `crates/shape-vm/src/executor/builtins/intrinsics/`).
//!
//! Both consumers are real and structurally distinct. This is two-consumer-
//! two-paths, NOT dual-registration soft-fail. Consolidation (drop legacy
//! body + edit shape-vm dispatcher) is deferred to the shape-vm cleanup
//! workstream per M-A scope binding.
//!
//! Computes `y[t] = y[t-1] * decay + input[t]`. Used for recursive indicators
//! (EMA, etc.). Optional initial value: when omitted/null, `y[0] = input[0]`.

use crate::context::ExecutionContext;
use crate::marshal::register_typed_fn_3_full;
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;
use std::sync::Arc;

// ───────────────────── Module factory (1 typed entry) ─────────────────────

/// Create the recurrence intrinsics module with the typed-marshal entry
/// for `__intrinsic_linear_recurrence`.
///
/// The legacy body [`intrinsic_linear_recurrence`] below is retained for
/// the shape-vm `BuiltinFunction::IntrinsicLinearRecurrence` dispatcher
/// arm at `runtime_delegated.rs:138` (cross-crate-dual-consumer pattern).
pub fn create_recurrence_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::recurrence");
    module.description =
        "Recurrence intrinsics (typed entry; legacy body retained for shape-vm dispatcher consumer per cross-crate-dual-consumer pattern)"
            .to_string();

    register_typed_fn_3_full::<_, Arc<Vec<f64>>, f64, Option<f64>>(
        &mut module,
        "__intrinsic_linear_recurrence",
        "Linear recurrence: y[t] = y[t-1] * decay + input[t]",
        [
            ModuleParam {
                name: "input".to_string(),
                type_name: "Array<number>".to_string(),
                required: true,
                description: "Input series".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "decay".to_string(),
                type_name: "number".to_string(),
                required: true,
                description: "Decay factor applied to previous output".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "initial_value".to_string(),
                type_name: "number?".to_string(),
                required: false,
                description: "Optional seed for y[0]; when omitted, y[0] = input[0]"
                    .to_string(),
                default_snippet: Some("null".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::ArrayNumber,
        |input, decay, initial_value, _ctx| {
            let data = input.as_slice();
            if data.is_empty() {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![])));
            }

            let mut result = Vec::with_capacity(data.len());
            if let Some(init) = initial_value {
                let mut prev = init;
                for &val in data {
                    let curr = prev * decay + val;
                    result.push(curr);
                    prev = curr;
                }
            } else {
                let first = data[0];
                result.push(first);
                let mut prev = first;
                for &val in &data[1..] {
                    let curr = prev * decay + val;
                    result.push(curr);
                    prev = curr;
                }
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(result)))
        },
    );

    module
}

// ───────────────────── Legacy body (shape-vm dispatcher consumer) ──────────

/// Intrinsic: Linear Recurrence
///
/// Computes y[t] = y[t-1] * decay + input[t]
///
/// **Retained alongside the typed factory entry** for the shape-vm
/// `BuiltinFunction::IntrinsicLinearRecurrence` dispatcher arm at
/// `runtime_delegated.rs:138`. Removal blocked on shape-vm cleanup
/// workstream (M-A scope binding) — see module docstring.
pub fn intrinsic_linear_recurrence(
    _args: &[KindedSlot],
    _ctx: &mut ExecutionContext,
) -> Result<KindedSlot> {
    Err(ShapeError::RuntimeError {
        message: "intrinsic_linear_recurrence: pending Phase 2c intrinsic kind threading — see ADR-006 §2.7.4".to_string(),
        location: None,
    })
}
