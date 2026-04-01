//! JIT executor implementing the ProgramExecutor trait

use shape_ast::Program;
use shape_runtime::engine::{ExecutionType, ProgramExecutor, ShapeEngine};
use shape_runtime::error::Result;
use shape_wire::WireValue;
use std::time::Instant;

/// JIT executor with selective per-function compilation.
///
/// JIT-compatible functions are compiled to native code; incompatible functions
/// (e.g. those using async, pattern matching, or unsupported builtins) are left
/// as `Interpreted` entries in the mixed function table for VM fallback.
///
/// Wraps a `BytecodeExecutor` so that compilation (including module resolution,
/// extension capability modules, and module loading) is delegated to the same
/// pipeline used by the VM path.
pub struct JITExecutor {
    pub bytecode_executor: shape_vm::BytecodeExecutor,
}

impl JITExecutor {
    pub fn new() -> Self {
        Self {
            bytecode_executor: shape_vm::BytecodeExecutor::new(),
        }
    }
}

impl ProgramExecutor for JITExecutor {
    fn execute_program(
        &mut self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        let emit_phase_metrics = std::env::var_os("SHAPE_JIT_PHASE_METRICS").is_some();

        let bytecode_compile_start = Instant::now();
        let bytecode = self
            .bytecode_executor
            .compile_program_for_inspection(engine, program)?;
        let bytecode_compile_ms = bytecode_compile_start.elapsed().as_millis();

        self.execute_with_jit(engine, &bytecode, bytecode_compile_ms, emit_phase_metrics)
    }
}

impl JITExecutor {
    fn execute_with_jit(
        &self,
        engine: &mut ShapeEngine,
        bytecode: &shape_vm::bytecode::BytecodeProgram,
        bytecode_compile_ms: u128,
        emit_phase_metrics: bool,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        use crate::JITConfig;
        use crate::JITContext;
        use crate::compiler::JITCompiler;

        // JIT compile the bytecode
        let jit_config = JITConfig::default();
        let mut jit = JITCompiler::new(jit_config).map_err(|e| {
            shape_runtime::error::ShapeError::RuntimeError {
                message: format!("JIT compiler initialization failed: {}", e),
                location: None,
            }
        })?;

        // Use selective compilation: JIT-compatible functions get native code,
        // incompatible ones get Interpreted entries for VM fallback.
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            eprintln!(
                "[jit-debug] starting compile_program_selective with {} instructions, {} functions",
                bytecode.instructions.len(),
                bytecode.functions.len()
            );
            for (i, instr) in bytecode.instructions.iter().enumerate() {
                eprintln!(
                    "[jit-debug] instr[{}]: {:?} {:?}",
                    i, instr.opcode, instr.operand
                );
            }
        }
        let jit_compile_start = Instant::now();
        let compile_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            jit.compile_program_selective("main", bytecode)
        }));
        let jit_compile_ms = jit_compile_start.elapsed().as_millis();
        let (jit_fn, _mixed_table) = match compile_result {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                return Err(shape_runtime::error::ShapeError::RuntimeError {
                    message: format!("JIT compilation failed: {}", e),
                    location: None,
                });
            }
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                return Err(shape_runtime::error::ShapeError::RuntimeError {
                    message: format!("JIT compilation panicked: {}", msg),
                    location: None,
                });
            }
        };

        let foreign_bridge = {
            let runtime = engine.get_runtime_mut();
            crate::foreign_bridge::link_foreign_functions_for_jit(
                bytecode,
                runtime.persistent_context(),
            )
            .map_err(|e| shape_runtime::error::ShapeError::RuntimeError {
                message: format!("JIT foreign-function linking failed: {}", e),
                location: None,
            })?
        };

        // Create JIT context and set function table for CallValue dispatch
        let mut jit_ctx = JITContext::default();
        let func_table = jit.get_function_table();
        if !func_table.is_empty() {
            jit_ctx.function_table =
                func_table.as_ptr() as *const crate::context::JittedStrategyFn;
            jit_ctx.function_table_len = func_table.len();
        }
        if let Some(state) = foreign_bridge.as_ref() {
            jit_ctx.foreign_bridge_ptr = state.as_ref() as *const _ as *const std::ffi::c_void;
        }

        // Set exec_context_ptr so JIT FFI can access cached data
        {
            let runtime = engine.get_runtime_mut();
            if let Some(ctx) = runtime.persistent_context_mut() {
                jit_ctx.exec_context_ptr = ctx as *mut _ as *mut std::ffi::c_void;
            }
        }

        // Register the generic builtin dispatch trampoline so builtins
        // without dedicated JIT lowering can fall back to the VM interpreter.
        unsafe {
            crate::ffi::generic_builtin::register_generic_builtin_fn(
                crate::ffi::generic_builtin::builtin_dispatch_trampoline,
            );
        }

        // Create a trampoline VM for executing functions that weren't
        // JIT-compiled (demoted due to block depth errors, unsupported opcodes,
        // etc.). The VM is loaded with the full bytecode program and extensions
        // so it can execute any function by ID.
        let mut trampoline_vm = shape_vm::VirtualMachine::new(shape_vm::VMConfig::default());
        trampoline_vm.load_program(bytecode.clone());
        for ext in self.bytecode_executor.extensions() {
            trampoline_vm.register_extension(ext.clone());
        }
        trampoline_vm.populate_module_objects();

        // Pre-run the program on the VM to populate module bindings (stdlib
        // prelude creates module namespace objects, registers extension functions,
        // etc.). The JIT can't reliably do this because many stdlib functions
        // are demoted. After VM execution, copy the populated module bindings
        // to the JIT context so the JIT-compiled user code sees them.
        // Output is captured (not printed) to avoid double output.
        trampoline_vm.enable_output_capture();
        let vm_exec_ok = trampoline_vm.execute(None).is_ok();
        // Copy VM module bindings to JIT context locals regardless of
        // execution success — partial initialization is better than none.
        let vm_bindings = trampoline_vm.module_bindings_snapshot();
        let debug = std::env::var_os("SHAPE_JIT_DEBUG").is_some();
        if debug {
            let non_null = vm_bindings.iter().filter(|vw| !vw.is_none()).count();
            eprintln!(
                "[jit-init] VM pre-run {}: {} module bindings ({} non-null)",
                if vm_exec_ok { "OK" } else { "FAILED" },
                vm_bindings.len(),
                non_null
            );
        }
        let mut owned_locals = crate::context::OwnedLocals::new();
        for (idx, vw) in vm_bindings.into_iter().enumerate() {
            if idx < jit_ctx.locals.len() && !vw.is_none() {
                owned_locals.transfer_binding(&mut jit_ctx, idx, vw);
            }
        }

        // Reset VM state for trampoline use (clear stack/IP but keep program + bindings).
        // Disable output capture so print() works normally during JIT execution.
        trampoline_vm.reset_for_trampoline();
        trampoline_vm.disable_output_capture();

        unsafe {
            crate::ffi::control::set_trampoline_vm(
                &mut trampoline_vm as *mut shape_vm::VirtualMachine,
            );
        }

        // Execute the JIT-compiled function
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            eprintln!("[jit-debug] compilation OK, about to execute...");
        }
        let jit_exec_start = Instant::now();
        let signal = unsafe { jit_fn(&mut jit_ctx) };
        let jit_exec_ms = jit_exec_start.elapsed().as_millis();

        // Drain unified heap refs whose refcounts were bumped during VM→JIT
        // conversion (nanboxed_to_jit_bits). This balances the refcounts so
        // unified heap values aren't permanently leaked.
        crate::ffi::object::conversion::drain_unified_heap_refs();

        // Clear the trampoline VM pointer (VM is about to be dropped)
        crate::ffi::control::unset_trampoline_vm();

        // Release ownership of pre-populated module binding slots.
        owned_locals.cleanup(&mut jit_ctx);

        // Get result from JIT context stack via TypedScalar boundary
        let raw_result = if jit_ctx.stack_ptr > 0 {
            jit_ctx.stack[0]
        } else {
            crate::nan_boxing::TAG_NULL
        };

        // Check for errors
        if signal < 0 {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: format!("JIT execution error (code: {})", signal),
                location: None,
            });
        }

        // Check if the result is an Err value from `?` propagation at the top level.
        // In the interpreter, uncaught `?` on Err raises a runtime error.
        // Match that behavior: extract the Err inner value and return a RuntimeError.
        if crate::nan_boxing::is_err_tag(raw_result) {
            let inner = unsafe { crate::nan_boxing::unbox_result_inner(raw_result) };
            let err_msg = if crate::nan_boxing::is_heap_kind(inner, crate::nan_boxing::HK_STRING) {
                let s = unsafe { crate::nan_boxing::unbox_string(inner) };
                s.to_string()
            } else {
                format!("Error (uncaught)")
            };
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: err_msg,
                location: None,
            });
        }

        // Use FrameDescriptor hint to preserve integer type identity.
        // Prefer return_kind when populated; fall back to last slot.
        let return_hint = bytecode.top_level_frame.as_ref().and_then(|fd| {
            if fd.return_kind != shape_vm::type_tracking::SlotKind::Unknown {
                Some(fd.return_kind)
            } else {
                fd.slots.last().copied()
            }
        });
        let result_scalar =
            crate::ffi::object::conversion::jit_bits_to_typed_scalar(raw_result, return_hint);

        // Convert TypedScalar to WireValue
        let wire_value = self.typed_scalar_to_wire(&result_scalar, raw_result);

        if emit_phase_metrics {
            let total_ms = bytecode_compile_ms + jit_compile_ms + jit_exec_ms;
            eprintln!(
                "[shape-jit-phases] bytecode_compile_ms={} jit_compile_ms={} jit_exec_ms={} total_ms={}",
                bytecode_compile_ms, jit_compile_ms, jit_exec_ms, total_ms
            );
        }

        Ok(shape_runtime::engine::ProgramExecutorResult {
            wire_value,
            type_info: None,
            execution_type: ExecutionType::Script,
            content_json: None,
            content_html: None,
            content_terminal: None,
        })
    }

    /// Convert a TypedScalar result to WireValue.
    ///
    /// For scalar types, the TypedScalar carries enough information. For heap types
    /// (strings, arrays) that TypedScalar can't represent, we fall back to raw bits.
    fn typed_scalar_to_wire(&self, ts: &shape_value::TypedScalar, raw_bits: u64) -> WireValue {
        use shape_value::ScalarKind;

        match ts.kind {
            ScalarKind::I8
            | ScalarKind::I16
            | ScalarKind::I32
            | ScalarKind::I64
            | ScalarKind::U8
            | ScalarKind::U16
            | ScalarKind::U32
            | ScalarKind::U64
            | ScalarKind::I128
            | ScalarKind::U128 => {
                // Integer result — preserve as exact integer in WireValue::Number
                WireValue::Number(ts.payload_lo as i64 as f64)
            }
            ScalarKind::F64 | ScalarKind::F32 => WireValue::Number(f64::from_bits(ts.payload_lo)),
            ScalarKind::Bool => WireValue::Bool(ts.payload_lo != 0),
            ScalarKind::Unit => WireValue::Null,
            ScalarKind::None => {
                // None could also be a fallback for non-scalar heap types.
                // Check if raw_bits is actually a heap value.
                self.nan_boxed_to_wire(raw_bits)
            }
        }
    }

    fn nan_boxed_to_wire(&self, bits: u64) -> WireValue {
        use crate::nan_boxing::{
            HK_STRING, TAG_BOOL_FALSE, TAG_BOOL_TRUE, TAG_NULL, is_heap_kind, is_number,
            unbox_number, unbox_string, is_ok_tag, is_err_tag,
        };
        use shape_value::tags::{TAG_INT, get_payload, get_tag, is_tagged, sign_extend_i48};

        if is_number(bits) {
            WireValue::Number(unbox_number(bits))
        } else if bits == TAG_NULL {
            WireValue::Null
        } else if bits == TAG_BOOL_TRUE {
            WireValue::Bool(true)
        } else if bits == TAG_BOOL_FALSE {
            WireValue::Bool(false)
        } else if is_tagged(bits) && get_tag(bits) == TAG_INT {
            // NaN-boxed i48 integer — sign-extend to i64 and return as integer
            let int_val = sign_extend_i48(get_payload(bits));
            WireValue::Integer(int_val)
        } else if is_heap_kind(bits, HK_STRING) {
            let s = unsafe { unbox_string(bits) };
            WireValue::String(s.to_string())
        } else if is_ok_tag(bits) {
            let inner = unsafe { crate::nan_boxing::unbox_result_inner(bits) };
            let inner_wire = self.nan_boxed_to_wire(inner);
            WireValue::String(format!("Ok({})", wire_value_display(&inner_wire)))
        } else if is_err_tag(bits) {
            let inner = unsafe { crate::nan_boxing::unbox_result_inner(bits) };
            let inner_wire = self.nan_boxed_to_wire(inner);
            WireValue::String(format!("Err({})", wire_value_display(&inner_wire)))
        } else {
            // Default to interpreting as a number for unknown tags
            WireValue::Number(f64::from_bits(bits))
        }
    }
}

/// Simple display for WireValue (used in Ok/Err formatting).
fn wire_value_display(v: &WireValue) -> String {
    match v {
        WireValue::String(s) => s.clone(),
        WireValue::Number(n) => format!("{}", n),
        WireValue::Integer(n) => format!("{}", n),
        WireValue::Bool(b) => format!("{}", b),
        WireValue::Null => "null".to_string(),
        _ => format!("{:?}", v),
    }
}
