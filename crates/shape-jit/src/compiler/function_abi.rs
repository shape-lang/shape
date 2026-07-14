//! Single authority for user-function native signatures.

use cranelift::prelude::{AbiParam, Signature, types};
use shape_vm::bytecode::Function;

pub(super) struct ProvenUserFunctionAbi {
    pub(super) signature: Signature,
    pub(super) native_arity: u16,
}

/// Prove the bytecode/MIR arity invariant and build the one native ABI used
/// for both function pre-declaration and definition.
///
/// `Function::arity` already counts the leading closure captures because the
/// producer's parameter list is `captures ++ user_params`. Adding
/// `captures_count` again would declare a different ABI from the call site.
pub(super) fn prove_user_function_abi(
    mut signature: Signature,
    function: &Function,
) -> Result<ProvenUserFunctionAbi, String> {
    if function.captures_count > function.arity {
        return Err(arity_error(
            function,
            format!(
                "captures_count={} exceeds Function.arity={}",
                function.captures_count, function.arity
            ),
        ));
    }

    if let Some(mir_data) = function.mir_data.as_ref() {
        let param_slots = mir_data.mir.param_slots.len();
        if param_slots != function.arity as usize {
            return Err(arity_error(
                function,
                format!(
                    "mir.param_slots.len()={param_slots} but Function.arity={}",
                    function.arity
                ),
            ));
        }
    }

    signature.params.push(AbiParam::new(types::I64)); // ctx_ptr
    signature
        .params
        .extend((0..function.arity).map(|_| AbiParam::new(types::I64)));
    signature.returns.push(AbiParam::new(types::I32));

    Ok(ProvenUserFunctionAbi {
        signature,
        native_arity: function.arity,
    })
}

fn arity_error(function: &Function, mismatch: String) -> String {
    format!(
        "SURFACE (ADR-006 §2.7.5): closure/function calling-convention \
         invariant broken for '{}': {mismatch} (captures_count={}). The native \
         ABI is `fn(ctx, param_0..param_{{arity-1}})` and params \
         [0..captures_count) are already the closure's leading captures. \
         Pre-declaration and definition refuse the same invalid signature; \
         padding the call site would fabricate a capture value.",
        function.name, function.captures_count,
    )
}
