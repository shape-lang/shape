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
pub struct JITExecutor {
    /// Bytecode executor used for extension loading, module resolution,
    /// and other pre-compilation setup that the CLI wires through.
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
        use shape_vm::BytecodeCompiler;
        let emit_phase_metrics = std::env::var_os("SHAPE_JIT_PHASE_METRICS").is_some();

        // Capture source text before getting runtime reference (for error messages)
        let source_for_compilation = engine.current_source().map(|s| s.to_string());

        // Compile to bytecode first to check JIT compatibility
        let runtime = engine.get_runtime_mut();

        // Get known module bindings — prefer persistent context, fallback to precompiled names
        let known_bindings: Vec<String> = if let Some(ctx) = runtime.persistent_context() {
            let names = ctx.root_scope_binding_names();
            if names.is_empty() {
                shape_vm::stdlib::core_binding_names(runtime)
            } else {
                names
            }
        } else {
            shape_vm::stdlib::core_binding_names(runtime)
        };

        // Build module graph and compile via graph pipeline
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        let (graph, stdlib_names, prelude_imports) =
            shape_vm::module_resolution::build_graph_and_stdlib_names(
                program,
                &mut loader,
                &[],
            )
            .map_err(|e| shape_runtime::error::ShapeError::RuntimeError {
                message: format!("Module graph construction failed: {}", e),
                location: None,
            })?;

        let bytecode_compile_start = Instant::now();
        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        compiler.register_known_bindings(&known_bindings);
        if let Some(source) = &source_for_compilation {
            compiler.set_source(source);
        }
        let bytecode = compiler
            .compile_with_graph_and_prelude(program, graph, &prelude_imports)
            .map_err(|e| shape_runtime::error::ShapeError::RuntimeError {
                message: format!("Bytecode compilation failed: {}", e),
                location: None,
            })?;
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

        // Create JIT context and execute
        let mut jit_ctx = JITContext::default();
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

        // Link the JIT function table into the context. jit_call_value uses
        // this table to resolve callees — unlinked, every closure / function
        // value dispatch BAILs at the "fn_id out of bounds" check.
        //
        // SAFETY: `jit.get_function_table()` borrows from the JITCompiler
        // which lives for the duration of this block. The ctx does not
        // outlive `jit` — we execute below and drop `jit` at end of scope.
        {
            let table: &[*const u8] = jit.get_function_table();
            jit_ctx.function_table = table.as_ptr() as *const crate::context::JittedStrategyFn;
            jit_ctx.function_table_len = table.len();
        }

        // ADR-006 §2.7.10 / Q11 (Phase 3 cluster-0 Round 20 sub-cluster γ —
        // W12-jit-trait-impl-method-registry, 2026-05-14): link the JIT
        // function-name table into the context so `jit_call_method`'s
        // user-method UFCS dispatch (`try_call_user_method` →
        // `find_function_by_name("TypeName::method")`) can resolve user-
        // defined trait/impl methods at runtime.
        //
        // The R15 W17-narrow sub-cluster fixed the upstream classification
        // (`receiver_type_name`) to correctly return the schema's type
        // name for `Ptr(HeapKind::TypedObject)` receivers. Without the
        // function-name table linkage here, every UFCS lookup at
        // `find_function_by_name` returned None (the early-return guard at
        // `call_method/mod.rs:230` triggered because `function_names_ptr`
        // was always the `JITContext::default()` null sentinel). That
        // returned TAG_NULL from `try_call_user_method`, which surfaced as
        // `None` at the print path (post-R19 C β filter; pre-β SIGSEGV).
        //
        // Cluster-0 close criterion for Smoke 3: `t.name()` on
        // `let t = X{}` returns `"x"` under `--mode jit` matching VM.
        //
        // The names slice is built from `bytecode.functions` 1:1 by index
        // so `function_table[idx]` and `function_names[idx]` describe the
        // same function — the same invariant `compile_program_selective`
        // upholds for the function-table itself (see
        // `compiler/program.rs:800-823`). The `Vec<String>` lives in the
        // local `function_names_storage` and is dropped after `jit_fn`
        // executes — same lifetime discipline as the function-table
        // borrow above and the trampoline VM below.
        let function_names_storage: Vec<String> =
            bytecode.functions.iter().map(|f| f.name.clone()).collect();
        jit_ctx.function_names_ptr = function_names_storage.as_ptr();
        jit_ctx.function_names_len = function_names_storage.len();

        // Set up the trampoline VM that JIT's `jit_call_value` falls back
        // to when a callee's function_table slot is null (i.e. the
        // function was not JIT-compiled, typically because its MIR
        // lowering bailed). Without this, `dispatch_call_via_trampoline_vm`
        // short-circuits to TAG_NULL, losing the callee's real result.
        //
        // The VM is populated with the **unlinked** bytecode (the exact
        // same input the JIT compiled from) so function_id lookups agree
        // between JIT and interpreter. Going through `load_program` with
        // a `content_addressed` field set would route through the linker,
        // which topologically sorts function blobs and renumbers them —
        // breaking JIT↔interpreter function-ID parity. Clear the
        // content-addressed payload first so `load_program` takes the
        // direct path.
        //
        // The trampoline VM lives for the duration of `jit_fn` execution
        // and is unset afterwards so a stale pointer does not leak across
        // threads / subsequent executions.
        let mut trampoline_bytecode = bytecode.clone();
        trampoline_bytecode.content_addressed = None;
        let mut trampoline_vm = shape_vm::VirtualMachine::new(shape_vm::VMConfig::default());
        trampoline_vm.load_program(trampoline_bytecode);
        unsafe {
            crate::ffi::control::set_trampoline_vm(
                &mut trampoline_vm as *mut shape_vm::VirtualMachine,
            );
        }

        // Drop guard: even if `jit_fn` panics, the thread-local
        // TRAMPOLINE_VM must not keep pointing at a VM that is about to
        // be freed when the stack unwinds.
        struct TrampolineGuard;
        impl Drop for TrampolineGuard {
            fn drop(&mut self) {
                crate::ffi::control::unset_trampoline_vm();
            }
        }
        let _trampoline_guard = TrampolineGuard;

        // Execute the JIT-compiled function
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            eprintln!("[jit-debug] compilation OK, about to execute...");
        }
        // W11-jit-new-array (supervisor reopen Step 4): snapshot arc
        // retain/release counters before/after the JIT-emitted code runs
        // so the supervisor can verify refcount balance — silent leaks
        // here are the W-series defection-attractor shape we're refusing.
        let retain_before =
            crate::ffi::arc::JIT_ARC_RETAIN_CALLS.load(std::sync::atomic::Ordering::Relaxed);
        let release_before =
            crate::ffi::arc::JIT_ARC_RELEASE_CALLS.load(std::sync::atomic::Ordering::Relaxed);
        let frees_before =
            crate::ffi::arc::JIT_ARC_RELEASE_FREES.load(std::sync::atomic::Ordering::Relaxed);

        let jit_exec_start = Instant::now();
        let signal = unsafe { jit_fn(&mut jit_ctx) };
        let jit_exec_ms = jit_exec_start.elapsed().as_millis();

        if std::env::var_os("SHAPE_JIT_ARC_COUNTERS").is_some() {
            let retain_after =
                crate::ffi::arc::JIT_ARC_RETAIN_CALLS.load(std::sync::atomic::Ordering::Relaxed);
            let release_after =
                crate::ffi::arc::JIT_ARC_RELEASE_CALLS.load(std::sync::atomic::Ordering::Relaxed);
            let frees_after =
                crate::ffi::arc::JIT_ARC_RELEASE_FREES.load(std::sync::atomic::Ordering::Relaxed);
            eprintln!(
                "[shape-jit-arc] retain_calls={} release_calls={} release_frees={}",
                retain_after - retain_before,
                release_after - release_before,
                frees_after - frees_before
            );
        }

        // Get result from JIT context stack via TypedScalar boundary
        let raw_result = if jit_ctx.stack_ptr > 0 {
            jit_ctx.stack[0]
        } else {
            crate::ffi::value_ffi::TAG_NULL
        };

        // Check for errors
        if signal < 0 {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: format!("JIT execution error (code: {})", signal),
                location: None,
            });
        }

        // v2: check return_type_tag for native-typed return values.
        // Non-zero tags bypass NaN-box decoding entirely.
        let wire_value = match jit_ctx.return_type_tag {
            crate::context::RETURN_TAG_F64 => {
                WireValue::Number(f64::from_bits(raw_result))
            }
            crate::context::RETURN_TAG_I64 => {
                WireValue::Integer(raw_result as i64)
            }
            crate::context::RETURN_TAG_I32 => {
                WireValue::Integer((raw_result as i32) as i64)
            }
            crate::context::RETURN_TAG_BOOL => {
                WireValue::Bool(raw_result != 0)
            }
            crate::context::RETURN_TAG_UNIT => {
                // W11-jit-new-array: `()`-typed return — the program's
                // terminal expression produced no value. Map to Null
                // (matches the VM's `wire_value` for `print(x)` at the
                // top level).
                WireValue::Null
            }
            _ => {
                // tag=0 (RETURN_TAG_NANBOXED) or unknown: per ADR-006
                // §2.7.5 / §2.7.5.1, the JIT-FFI return path must be
                // kind-stamped at compile time from the call signature
                // (`FrameDescriptor::return_kind: Option<NativeKind>`).
                // The pre-strict-typing fallback decoded `tag_bits` from
                // `raw_result` to recover a kind at runtime — that path
                // is the W-series defection-attractor (deleted-runtime
                // tag-bit dispatch + kind-blind classifier) and is
                // forbidden per CLAUDE.md "Forbidden Patterns".
                //
                // The correct §2.7.5 surface stamps `return_kind` from
                // the JIT-emitted call signature so the typed return
                // path (RETURN_TAG_F64 / I64 / I32 / BOOL) handles every
                // case statically. A `RETURN_TAG_NANBOXED` arrival here
                // is a kind-source gap — surface-and-stop per W10
                // jit-playbook §5.
                //
                // PHASE_2C / SURFACE: stamp `return_type_tag` to a
                // typed variant from the FrameDescriptor at JIT-emit
                // time (rvalue path — W10-mir-compiler territory) so
                // this arm is unreachable in production bytecode.
                let return_hint = bytecode
                    .top_level_frame
                    .as_ref()
                    .and_then(|fd| fd.return_kind.or_else(|| fd.slots.last().copied()));
                let _ = return_hint;
                return Err(shape_runtime::error::ShapeError::RuntimeError {
                    message: format!(
                        "JIT-FFI return path: RETURN_TAG_NANBOXED reached the \
                         host boundary without a stamped NativeKind (raw_bits={:#x}). \
                         Per ADR-006 §2.7.5 / §2.7.5.1 the return tag must be a \
                         typed variant; this is a kind-source gap (W10 jit-playbook \
                         §5 surface-and-stop). See executor.rs:267 comment.",
                        raw_result
                    ),
                    location: None,
                });
            }
        };

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

    // typed_scalar_to_wire and value_word_to_wire removed — both were
    // kind-blind dispatch paths. The former dispatched on
    // `ScalarKind::None` to `value_word_to_wire`; the latter decoded
    // `tag_bits` from a raw u64 to recover a kind. Per ADR-006 §2.7.5
    // / §2.7.5.1 the JIT-FFI return path stamps a typed `RETURN_TAG_*`
    // from the JIT-emitted call signature, so the kind-blind fallback
    // is unreachable in production bytecode (and the surface-and-stop
    // path on the `_ =>` arm of the `return_type_tag` match documents
    // any kind-source gap that does land here).
    //
    // CLAUDE.md "Forbidden Patterns" forbids `tag_bits` decode in JIT
    // codegen; the W-series defection-attractor list forbids the
    // "decode/tag/dispatch helper/bridge/probe" framing these helpers
    // would need to come back under.
}
