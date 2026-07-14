//! Compile-stage finalization for selectively compiled programs.
//!
//! A Phase-4 failure leaves its pre-declared body undefined. An unreferenced
//! failure is harmless and must not deopt the rest of a prelude-heavy program.
//! A native relocation to that body is different: defining a runtime `-1`
//! stub would let the caller run side effects before requesting an interpreter
//! rerun, duplicating those effects. Finalization therefore converts only a
//! reachable unresolved symbol into a compile-stage refusal and preserves the
//! original reason for diagnostics.

use cranelift_jit::JITModule;

/// Finalize definitions without allowing Cranelift's unresolved-symbol panic
/// to escape, preserving the originating function refusal when its symbol is
/// the unresolved relocation.
pub(super) fn finalize_program_definitions(
    module: &mut JITModule,
    compile_failures: &[(String, String)],
) -> Result<(), String> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        module.finalize_definitions()
    }));
    std::panic::set_hook(previous_hook);

    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => unresolved_definition_error(format!("{error:?}"), compile_failures),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_else(|| "unknown finalize panic".to_string());
            unresolved_definition_error(message, compile_failures)
        }
    }
}

fn unresolved_definition_error(
    message: String,
    compile_failures: &[(String, String)],
) -> Result<(), String> {
    if let Some((_, reason)) = compile_failures
        .iter()
        .find(|(symbol, _)| message.contains(symbol))
    {
        return Err(reason.clone());
    }

    Err(format!(
        "WF-1A signal-reexec (audit 2026-07-04 §4(a)): JIT finalize could \
         not resolve a native reference to a function that failed Phase-4 \
         JIT compile ({message}). Whole-program deopt to the bytecode \
         interpreter at COMPILE stage (before any native side effect); the \
         demoted function has no runtime `-1` stub, so the executor's \
         outer-Err interpreter re-run cannot double already-executed side \
         effects."
    ))
}
