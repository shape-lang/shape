//! Type checking and conversion builtin implementations
//!
//! Wave 6.5 substep-2 Wave-α (sub-cluster D-type-ops, ADR-006 §2.7.7 / Q9):
//! the pre-Wave-6 bodies of `builtin_is_*`, `builtin_to_*`,
//! `dispatch_native_interop_builtin`, `builtin_type_of`,
//! `builtin_some_ctor`, `builtin_ok_ctor`, `builtin_err_ctor`, and the
//! `op_convert*` / `op_try_convert*` machinery depend on a constellation
//! of deleted symbols (the substep-1 shim deletion list plus the W-series
//! deleted dynamic-dispatch carrier and its NB-suffixed legacy formatters
//! and error constructors). None of those have a kinded-API equivalent in
//! this wave: the
//! `builtin_*`/`dispatch_*` arms are routed to `todo!("phase-1b-vm wave 5c
//! …")` placeholders in `executor/vm_impl/builtins.rs` (waves 5c–5e own
//! the body migrations), and the conversion opcodes need a kind-aware
//! domain table that the playbook does not specify in §2 (the `Convert*`
//! row is not in the kind-sourcing table — these dispatch on a
//! `TypeAnnotation` operand and read source kind off the popped slot).
//!
//! Rather than reintroduce the deleted shims under any name (forbidden #1
//! per playbook §4) or paper over with `NativeKind::Bool`-default
//! placeholders (forbidden #9, the W-series rationalization), this file
//! is reduced to its minimum live surface: the `op_convert*` /
//! `op_try_convert*` opcode handlers (wired live in `dispatch.rs:711-722`)
//! become `todo!("phase-2c — see ADR-006 §2.7.4")` placeholders that
//! surface the gap as a runtime panic rather than silent miscompile, and
//! the `builtin_*` / `dispatch_*` methods are deleted (their dispatch
//! arms in `executor/vm_impl/builtins.rs` already terminate with
//! `todo!()` for waves 5c–5e). The single out-of-territory caller of
//! `builtin_type_of` (`executor/objects/mod.rs:304`) is owned by the
//! `D-objects-mod` sub-cluster; deleting `builtin_type_of` here surfaces
//! that call site for `D-objects-mod` to rewrite.
//!
//! See:
//! - `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §7 REVISED,
//!   §10 D-type-ops row.
//! - `docs/adr/006-value-and-memory-model.md` §2.7.4 (Phase-2c deferral
//!   pattern), §2.7.7 (parallel-kind stack ABI), §2.7.8 (cell-storage
//!   parallel-kind invariant).

use crate::bytecode::Instruction;
use crate::executor::VirtualMachine;
use shape_value::VMError;

impl VirtualMachine {
    pub(in crate::executor) fn op_convert(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        todo!(
            "phase-2c — Convert opcode (TryInto/Into trait dispatch) needs \
             kind-aware source-domain reading + per-target-selector \
             converters. ADR-006 §2.7.4: deferred until the kinded \
             conversion machinery lands."
        )
    }

    /// ConvertToInt: pop any value, convert to i64, push native i64.
    #[inline]
    pub(in crate::executor) fn op_convert_to_int(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — ConvertToInt needs kind-aware source-domain reading. \
             ADR-006 §2.7.4: deferred until the kinded conversion machinery \
             lands."
        )
    }

    /// ConvertToNumber: pop any value, convert to f64, push raw f64.
    #[inline]
    pub(in crate::executor) fn op_convert_to_number(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — ConvertToNumber needs kind-aware source-domain \
             reading. ADR-006 §2.7.4: deferred until the kinded conversion \
             machinery lands."
        )
    }

    /// ConvertToString: pop any value, convert to string, push result.
    #[inline]
    pub(in crate::executor) fn op_convert_to_string(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — ConvertToString needs the kinded formatter \
             (executor/printing.rs Wave 5e scope). ADR-006 §2.7.4: \
             deferred until the kinded conversion machinery lands."
        )
    }

    /// ConvertToBool: pop any value, convert to bool, push native bool.
    #[inline]
    pub(in crate::executor) fn op_convert_to_bool(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — ConvertToBool needs kind-aware truthiness check. \
             ADR-006 §2.7.4: deferred until the kinded conversion machinery \
             lands."
        )
    }

    /// ConvertToDecimal: pop value, convert to decimal, push result.
    #[inline]
    pub(in crate::executor) fn op_convert_to_decimal(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — ConvertToDecimal needs kind-aware source-domain \
             reading. ADR-006 §2.7.4: deferred until the kinded conversion \
             machinery lands."
        )
    }

    /// ConvertToChar: pop value, convert to char, push result.
    #[inline]
    pub(in crate::executor) fn op_convert_to_char(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — ConvertToChar needs kind-aware source-domain \
             reading. ADR-006 §2.7.4: deferred until the kinded conversion \
             machinery lands."
        )
    }

    /// TryConvertToInt: pop value, try convert to int, push Result<int, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_int(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — TryConvertToInt needs kind-aware source-domain \
             reading + AnyError construction (depends on the kinded \
             AnyError builder migration in executor/exceptions/mod.rs). \
             ADR-006 §2.7.4: deferred."
        )
    }

    /// TryConvertToNumber: pop value, try convert to number, push Result<number, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_number(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — TryConvertToNumber needs kind-aware source-domain \
             reading + AnyError construction. ADR-006 §2.7.4: deferred."
        )
    }

    /// TryConvertToString: pop value, try convert to string, push Result<string, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_string(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — TryConvertToString needs the kinded formatter \
             (executor/printing.rs Wave 5e scope). ADR-006 §2.7.4: deferred."
        )
    }

    /// TryConvertToBool: pop value, try convert to bool, push Result<bool, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_bool(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — TryConvertToBool needs kind-aware truthiness check \
             + AnyError construction. ADR-006 §2.7.4: deferred."
        )
    }

    /// TryConvertToDecimal: pop value, try convert to decimal, push Result<decimal, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_decimal(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — TryConvertToDecimal needs kind-aware source-domain \
             reading + AnyError construction. ADR-006 §2.7.4: deferred."
        )
    }

    /// TryConvertToChar: pop value, try convert to char, push Result<char, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_char(&mut self) -> Result<(), VMError> {
        todo!(
            "phase-2c — TryConvertToChar needs kind-aware source-domain \
             reading + AnyError construction. ADR-006 §2.7.4: deferred."
        )
    }
}
