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
/// # `--mode jit` semantics (W12-jit-mode-semantics-and-fallthrough, 2026-05-18)
///
/// `--mode jit` attempts to JIT-compile the toplevel script and every function
/// it reaches. If JIT compilation or JIT execution fails for any reason (a
/// preflight rejection at `compile_program_selective`, a Cranelift codegen
/// error, a panic in the JIT pipeline, a `RETURN_TAG_NANBOXED` kind-source
/// gap surface, etc.), the executor falls through to the bytecode interpreter
/// via `BytecodeExecutor::execute_program` instead of surfacing a hard error
/// to the CLI. A one-line `[jit-fallback]` diagnostic is emitted on stderr at
/// `tracing::info` level (default) so the fall-through is observable but does
/// not silently mask broken programs (the interpreter still re-runs the same
/// `Program` and surfaces any genuine runtime error from there).
///
/// Verbose JIT tracing remains under the `--trace-jit=...` CLI flag (replaces
/// the legacy `SHAPE_JIT_DEBUG` env-var per closure-wave-F migration). Tier-up
/// thresholds at T1@100 / T2@10k on hot functions are preserved by the
/// underlying `compile_program_selective` pipeline — fall-through only fires
/// when JIT can not handle the program at all.
pub struct JITExecutor {
    /// Bytecode executor used for extension loading, module resolution,
    /// and other pre-compilation setup that the CLI wires through.
    /// Also the fall-through target when JIT compile/execute fails.
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

        // REPL cross-cell persistence (WS-11): when the engine is a REPL
        // (`init_repl` enabled persistence), execute the cell on the
        // bytecode interpreter. Cross-cell `let`/`var` bindings and
        // `fn`/`type` definitions are round-tripped through the
        // persistent `ExecutionContext` by `BytecodeExecutor::
        // execute_program`; the JIT's ahead-of-time `compile_strategy`
        // path stores top-level module bindings in its own `jit_ctx`
        // locals which never reach that context, so a JIT-executed cell
        // would silently drop every binding the next cell needs.
        //
        // This is not a fallback or a degradation hatch: a REPL cell is
        // a one-shot interactive line for which ahead-of-time native
        // codegen yields no measurable benefit, and the interpreter's
        // own tiered JIT (T1@100 / T2@10k) still promotes any function
        // that genuinely runs hot across cells. The `--mode jit` flag
        // continues to drive AOT compilation for `shape run` scripts;
        // only the interactive REPL routes through the interpreter, so
        // cross-cell correctness is identical to `--mode vm`.
        if engine.repl_persistence() {
            return self.bytecode_executor.execute_program(engine, program);
        }

        // Cluster-2 closure-wave-F tracing-crate migration (2026-05-16):
        // `tracing::enabled!` compiles away under `release_max_level_off`
        // (the default when the `jit-trace` Cargo feature is OFF), so this
        // collapses to `false` and the phase-timing accounting below is
        // dead-code-eliminated by the optimizer. Replaces the legacy
        // `SHAPE_JIT_PHASE_METRICS` env-var; CLI selector is
        // `--trace-jit=shape_jit::metrics=info`.
        let emit_phase_metrics = tracing::enabled!(
            target: "shape_jit::metrics",
            tracing::Level::INFO,
        );

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

        // Build module graph and compile via graph pipeline.
        //
        // W9: pass `self.bytecode_executor.extensions()` so the graph build
        // can hybridize native extension modules with their Shape overlay
        // (e.g. `std::core::remote`'s `pub annotation remote(addr)`). Without
        // the extensions list, the graph would skip the hybridization probe
        // and the namespace import path would lose annotation visibility.
        let extensions = self.bytecode_executor.extensions().to_vec();
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        let (graph, stdlib_names, prelude_imports) =
            shape_vm::module_resolution::build_graph_and_stdlib_names(
                program,
                &mut loader,
                &extensions,
            )
            .map_err(|e| shape_runtime::error::ShapeError::RuntimeError {
                message: format!("Module graph construction failed: {}", e),
                location: None,
            })?;

        let bytecode_compile_start = Instant::now();
        let mut compiler = if extensions.is_empty() {
            BytecodeCompiler::new()
        } else {
            BytecodeCompiler::new().with_extensions(extensions.clone())
        };
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

        // W12-jit-mode-semantics-and-fallthrough (Phase 3d, 2026-05-18):
        // attempt JIT compile+execute; if either step fails for any reason,
        // fall through to the bytecode interpreter so `--mode jit` never
        // silently no-ops on a broken JIT path. The interpreter executes
        // the same `Program` directly (not the JIT-side bytecode), so
        // bytecode-graph differences between the two paths do not matter
        // here. The fall-through is observable via a one-line stderr
        // `[jit-fallback]` diagnostic at `tracing::info` level (default
        // visibility); `--trace-jit=shape_jit=debug` promotes the JIT
        // pipeline's own tracing for root-cause investigation.
        //
        // Per supervisor path (3) binding 2026-05-18: "On JIT-compile
        // failure: fall through to interpreter (NOT silent-no-output);
        // Diagnostic emitted at info level: `[jit-fallback] function X
        // failed JIT compile: <reason>; running under interpreter`".
        // r5c-2-gz-cp2-jit-div: `execute_with_jit` returns a nested result so
        // a JIT-COMPILE-stage failure (fall through to interpreter) is kept
        // distinct from a genuine PROGRAM runtime error the JIT executed
        // soundly (e.g. division by zero). The latter must propagate
        // directly — re-running it under the interpreter would execute the
        // program a second time, doubling any side effects (a `print`
        // before the failing divide would fire twice). Outer `Err` =
        // compile-stage failure; `Ok(Err(_))` = JIT-executed runtime error.
        match self.execute_with_jit(engine, &bytecode, bytecode_compile_ms, emit_phase_metrics) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(runtime_err)) => Err(runtime_err),
            Err(jit_err) => {
                // Emit the structured fall-through diagnostic. Use `eprintln!`
                // so the diagnostic is visible even when the user has not
                // wired up a tracing subscriber (the default `shape run`
                // CLI invocation has no subscriber installed). Mirror the
                // event to `tracing::info!` so JSON-tracing consumers and
                // the `--trace-jit` filter see it too.
                let reason = jit_err.to_string();
                let function_name = "main".to_string();
                eprintln!(
                    "[jit-fallback] function {function_name} failed JIT compile: \
                     {reason}; running under interpreter"
                );
                tracing::info!(
                    target: "shape_jit::fallback",
                    function = %function_name,
                    reason = %reason,
                    "jit-fallback: function failed JIT compile, running under interpreter",
                );
                self.bytecode_executor.execute_program(engine, program)
            }
        }
    }
}

impl JITExecutor {
    /// Run the JIT pipeline for `bytecode`.
    ///
    /// r5c-2-gz-cp2-jit-div: the nested result separates two error classes
    /// the W12 fall-through must treat differently:
    ///
    /// - Outer `Err` — a JIT-COMPILE-stage failure (compiler init, selective
    ///   compile, foreign-fn link, or a `RETURN_TAG_NANBOXED` kind-source
    ///   gap). The interpreter can run the program; `execute_program` falls
    ///   through with a `[jit-fallback]` diagnostic.
    /// - `Ok(Err(_))` — the JIT compiled and EXECUTED the program soundly,
    ///   but the program itself hit a runtime error the bytecode VM also
    ///   reports (division by zero). This must propagate directly: a
    ///   fall-through would re-execute the program under the interpreter and
    ///   double any side effects already performed by the JIT run.
    /// - `Ok(Ok(_))` — success.
    fn execute_with_jit(
        &self,
        engine: &mut ShapeEngine,
        bytecode: &shape_vm::bytecode::BytecodeProgram,
        bytecode_compile_ms: u128,
        emit_phase_metrics: bool,
    ) -> Result<Result<shape_runtime::engine::ProgramExecutorResult>> {
        use crate::JITConfig;
        use crate::JITContext;
        use crate::compiler::JITCompiler;

        // R8 W7 G.5 (v0.3 divergence-elimination, ADR-006 §2.7.14 SURFACE):
        // Refuse to JIT-compile programs whose V2 typed opcodes lack matching
        // FrameDescriptor entries. The bytecode interpreter handles such
        // opcodes by reading their kind from the runtime parallel-kind
        // track per §2.7.7 — the JIT path consumes FrameDescriptors and
        // previously emitted native code that silently bypassed the runtime
        // string-key check in `as_string_key`, returning garbage where the
        // VM cleanly errored (audit
        // `docs/cluster-audits/v0.3-r8w6-hashmap-key-kind-audit.md` §4 —
        // `set::from_array([1,2,3])` ec=0 with `{"Integer": -1407...}` vs
        // VM ec=1 "HashMap key must be a string"). Returning the outer
        // `Err` triggers the existing `[jit-fallback]` path in
        // `JITExecutor::execute_program` (line 173): the program runs
        // under the bytecode interpreter and reports the same surface as
        // `--mode vm`. Full V2 type soundness for every JIT-emitted opcode
        // is v0.4 follow-up (per audit §5 Option B; Option A was infeasible
        // because smoke s2 currently emits the same `Vec.map::*` unverified
        // shape and depends on the interpreter handling it cleanly).
        if let Err(errors) = shape_vm::bytecode::verifier::verify_v2_typed_opcodes(bytecode) {
            let total = errors.len();
            let first = errors
                .first()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "<none>".to_string());
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: format!(
                    "V2 bytecode verification failed: {} violation(s); first: {}. \
                     R8 W7 G.5 SURFACE (ADR-006 §2.7.14) — JIT refuses unverified \
                     V2 typed opcodes; falling through to bytecode interpreter so \
                     the runtime error surface agrees with `--mode vm`. Tracked via \
                     docs/cluster-audits/v0.3-r8w6-hashmap-key-kind-audit.md (v0.4 \
                     / planned: full V2 type soundness for every JIT-emitted opcode)",
                    total, first,
                ),
                location: None,
            });
        }

        // R8 W8 Cluster A imported-const ident-eval SURFACE
        // (v0.3 divergence-elimination per supervisor 2026-05-25 path (i),
        // ADR-006 §2.7.14): Refuse to JIT-compile programs whose bytecode
        // was emitted via the Cluster A `compile_expr_identifier`
        // inlined-at-use intercept for imported `pub const` bindings.
        // The inlined `PushConst(<value>)` bytecode is correct, but the
        // JIT direct-identifier-eval lowering of this shape fires
        // `jit_print_*` FFI with zero-init bits — silent-wrong-output
        // VM=2 / JIT=0 on `print(IMPORTED_CONST)` bare. Whole-program
        // deopt to the bytecode interpreter is the binding-compliant
        // surface-and-stop (the interpreter evaluates the inlined
        // PushConst correctly). Mirrors R8 W7 G.5 V2-verifier deopt
        // immediately above + R8 W8 aliased-CoW
        // `mir_has_prior_move_of_slot` precedent.
        // Root-cause fix in JIT identifier-eval lowering is v0.4 per
        // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup
        // workstream.
        if bytecode.has_imported_const_inline {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: "R8 W8 Cluster A imported-const ident-eval SURFACE \
                          (ADR-006 §2.7.14): the program uses imported `pub const` \
                          identifiers whose values were inlined-at-use as \
                          `PushConst(<value>)` bytecode by `compile_expr_identifier`. \
                          The JIT direct-identifier-eval lowering of this shape \
                          produces silent-wrong-output (zero-init bits at the print \
                          FFI dispatch); whole-program deopting to the bytecode \
                          interpreter via this `[jit-fallback]` path preserves \
                          VM == JIT semantics. Tracked via \
                          `docs/v0.3-close-summary.md` §5.16 (v0.4 / planned: JIT \
                          identifier-eval lowering root-cause fix)".to_string(),
                location: None,
            });
        }

        // R8 W9 B1 W17-marshal-return JIT surface-and-stop
        // (v0.3 divergence-elimination per supervisor 2026-05-25 ruling,
        // ADR-006 §2.7.14): Refuse to JIT-compile programs whose
        // bytecode contains direct calls to imported stdlib functions
        // (callee resolved via `resolve_scoped_module_binding_name` at
        // `compile_expr_function_call` — see
        // `crates/shape-vm/src/compiler/expressions/function_calls.rs`).
        // Such calls flow through `op_call_value` whose VM-side
        // ModuleFn dispatch arm in
        // `crates/shape-vm/src/executor/call_convention.rs:999` cleanly
        // routes through `invoke_module_fn_id_stub` +
        // `project_typed_return` and surfaces the W17-marshal-return-arms
        // catch-all at
        // `crates/shape-vm/src/executor/vm_impl/modules.rs:74` when the
        // stdlib body returns a `ConcreteReturn` arm without a typed-slot
        // projection (`Bytes` / `ArrayHeapValue` /
        // `HashMapStringHeapValue` / etc.).
        //
        // The JIT-side `jit_call_value` ModuleFn arm at
        // `crates/shape-jit/src/ffi/control/mod.rs:704-715` instead
        // returns `TAG_NULL` silently (`-1407374883553280` NaN-box null
        // pattern) with only a `tracing::debug!` line — swallowing the
        // surface and producing silent-wrong-output VM=ec1 SURFACE /
        // JIT=ec0 garbage on `print(serialize([1.0,2.0,3.0]).len())`.
        //
        // Whole-program deopt to the bytecode interpreter is the
        // binding-compliant surface-and-stop (mirrors R8 W7 G.5
        // V2-verifier deopt + R8 W8 imported-const-inline deopt
        // immediately above + R8 W8 aliased-CoW
        // `mir_has_prior_move_of_slot` precedent). Root-cause fix in
        // JIT ModuleFn dispatch (`dispatch_module_fn_call` `todo!()` +
        // §2.7.10/Q11 kinded handler ABI rebuild) is v0.4 per
        // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup
        // workstream — third member of the bundle alongside Cluster A
        // imported-const-inline + aliased-CoW.
        if bytecode.has_w17_marshal_residual {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: "R8 W9 B1 W17-marshal-return-arms SURFACE (ADR-006 \
                          §2.7.14): the program contains direct calls to imported \
                          stdlib functions (callee resolved via \
                          `resolve_scoped_module_binding_name`). The JIT-side \
                          `jit_call_value` ModuleFn dispatch arm at \
                          `ffi/control/mod.rs:704-715` returns TAG_NULL silently, \
                          swallowing the W17-marshal-return-arms surface that \
                          VM-side `invoke_module_fn_id_stub` + \
                          `project_typed_return` would clean-surface on (e.g. \
                          `state.serialize` returning Array<int>/Bytes hits the \
                          catch-all at `vm_impl/modules.rs:74`). Whole-program \
                          deopting to the bytecode interpreter via this \
                          `[jit-fallback]` path preserves VM == JIT semantics. \
                          Tracked via `docs/v0.3-close-summary.md` §5.16 (v0.4 / \
                          planned: JIT ModuleFn dispatch root-cause fix at \
                          `dispatch_module_fn_call` todo!() + §2.7.10/Q11 kinded \
                          handler ABI rebuild)".to_string(),
                location: None,
            });
        }

        // R8 W9 B3 Drop-bearing-scope-exit SURFACE (v0.3 divergence-elimination
        // per supervisor 2026-05-25 G.2 Step 2 ruling, ADR-006 §2.7.14):
        // Refuse to JIT-compile programs that register a user `impl Drop for T`
        // impl. The JIT `emit_drop` codegen (`mir_compiler/ownership.rs`)
        // currently only releases the slot's refcount and nulls the slot —
        // there is NO user-Drop trait-method dispatch on the JIT path, so
        // `drop_locals_at_scope_exit` silently elides the user's `Drop::drop`
        // body, breaking the resource-management.mdx documented RAII
        // contract (VM prints the user's drop output; JIT prints nothing).
        // Whole-program deopt to the bytecode interpreter is the binding-
        // compliant surface-and-stop: the interpreter has a working Drop
        // dispatch at `executor/trait_object_ops.rs::op_drop_call_impl`
        // (per audit `docs/cluster-audits/v0.3-r8w9-drop-runtime-audit.md`
        // §4 — `op_drop_call_impl` looks up `Drop::TypeName::__default__::drop`
        // in `trait_method_symbols` and dispatches via
        // `call_function_with_nb_args`).
        //
        // Detection: presence of any `Drop::*::*::drop` entry in
        // `trait_method_symbols` (registered at compile time per
        // `compiler/statements.rs::register_trait_method_symbol` for every
        // `impl Drop for T` block — see drop_type_info handling at
        // `compiler/statements.rs:418`).
        //
        // Root-cause fix in JIT Drop codegen (Drop-trait-method dispatch
        // at `emit_drop` time) is v0.4 per `docs/v0.3-close-summary.md`
        // §5.16 JIT-lowering followup workstream (joining the
        // aliased-CoW + imported-const + W17-marshal bundle).
        let has_user_drop_impl = bytecode
            .trait_method_symbols
            .keys()
            .any(|k| k.starts_with("Drop::"));
        if has_user_drop_impl {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: "R8 W9 B3 Drop-bearing-scope-exit SURFACE \
                          (ADR-006 §2.7.14): the program registers one or more \
                          `impl Drop for T` impls. The JIT `emit_drop` codegen \
                          (mir_compiler/ownership.rs::emit_drop) lacks user-Drop \
                          trait-method dispatch — it only releases refcounts + \
                          nulls slots, silently eliding the user's `Drop::drop` \
                          body and breaking the resource-management.mdx \
                          documented RAII contract. Whole-program deopting to \
                          the bytecode interpreter via this `[jit-fallback]` \
                          path preserves VM == JIT semantics (the interpreter \
                          dispatches Drop methods through \
                          `op_drop_call_impl` at trait_object_ops.rs:687). \
                          Tracked via `docs/v0.3-close-summary.md` §5.16 \
                          (v0.4 / planned: JIT Drop codegen root-cause fix)"
                    .to_string(),
                location: None,
            });
        }

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
        //
        // Cluster-2 closure-wave-F tracing-crate migration (2026-05-16):
        // `tracing::enabled!` collapses to `false` under feature-OFF builds
        // so the per-instruction enumeration loop is dead-code-eliminated.
        // Replaces SHAPE_JIT_DEBUG env-var gating.
        if tracing::enabled!(target: "shape_jit", tracing::Level::DEBUG) {
            tracing::debug!(
                target: "shape_jit",
                instruction_count = bytecode.instructions.len(),
                function_count = bytecode.functions.len(),
                "starting compile_program_selective",
            );
            for (i, instr) in bytecode.instructions.iter().enumerate() {
                tracing::debug!(
                    target: "shape_jit",
                    idx = i,
                    opcode = ?instr.opcode,
                    operand = ?instr.operand,
                    "instruction",
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
        tracing::debug!(
            target: "shape_jit",
            "compilation OK, about to execute",
        );
        // W11-jit-new-array (supervisor reopen Step 4): snapshot arc
        // retain/release counters before/after the JIT-emitted code runs
        // so the supervisor can verify refcount balance — silent leaks
        // here are the W-series defection-attractor shape we're refusing.
        //
        // Cluster-2 closure-wave-F tracing-crate migration (2026-05-16):
        // gate the snapshot reads on `tracing::enabled!` so the atomic
        // loads themselves are dead-code-eliminated under feature-OFF
        // builds (`release_max_level_off` collapses the macro to `false`).
        // Replaces SHAPE_JIT_ARC_COUNTERS env-var; CLI selector is
        // `--trace-jit=shape_jit::arc_counters=info`.
        //
        // Cluster-2 closure-wave-E §F string-constant leak measurement
        // (2026-05-16): STRING_* counters share the same arc_counters
        // gate (Arc<UnifiedValue> + Arc<String> are both Arc-tier;
        // single tracing target keeps CLI filter narrow). Take-both
        // ceremony at Round 1 merge.
        let arc_counters_enabled = tracing::enabled!(
            target: "shape_jit::arc_counters",
            tracing::Level::INFO,
        );
        let (retain_before, release_before, frees_before,
             str_allocs_before, str_retain_before,
             str_release_before, str_frees_before) = if arc_counters_enabled {
            (
                crate::ffi::arc::JIT_ARC_RETAIN_CALLS.load(std::sync::atomic::Ordering::Relaxed),
                crate::ffi::arc::JIT_ARC_RELEASE_CALLS.load(std::sync::atomic::Ordering::Relaxed),
                crate::ffi::arc::JIT_ARC_RELEASE_FREES.load(std::sync::atomic::Ordering::Relaxed),
                crate::ffi::arc::STRING_CONSTANT_ALLOCS.load(std::sync::atomic::Ordering::Relaxed),
                crate::ffi::arc::STRING_RETAIN_CALLS.load(std::sync::atomic::Ordering::Relaxed),
                crate::ffi::arc::STRING_RELEASE_CALLS.load(std::sync::atomic::Ordering::Relaxed),
                crate::ffi::arc::STRING_RELEASE_FREES.load(std::sync::atomic::Ordering::Relaxed),
            )
        } else {
            (0, 0, 0, 0, 0, 0, 0)
        };

        let jit_exec_start = Instant::now();
        let signal = unsafe { jit_fn(&mut jit_ctx) };
        let jit_exec_ms = jit_exec_start.elapsed().as_millis();

        if arc_counters_enabled {
            let retain_after =
                crate::ffi::arc::JIT_ARC_RETAIN_CALLS.load(std::sync::atomic::Ordering::Relaxed);
            let release_after =
                crate::ffi::arc::JIT_ARC_RELEASE_CALLS.load(std::sync::atomic::Ordering::Relaxed);
            let frees_after =
                crate::ffi::arc::JIT_ARC_RELEASE_FREES.load(std::sync::atomic::Ordering::Relaxed);
            let str_allocs_after =
                crate::ffi::arc::STRING_CONSTANT_ALLOCS
                    .load(std::sync::atomic::Ordering::Relaxed);
            let str_retain_after =
                crate::ffi::arc::STRING_RETAIN_CALLS
                    .load(std::sync::atomic::Ordering::Relaxed);
            let str_release_after =
                crate::ffi::arc::STRING_RELEASE_CALLS
                    .load(std::sync::atomic::Ordering::Relaxed);
            let str_frees_after =
                crate::ffi::arc::STRING_RELEASE_FREES
                    .load(std::sync::atomic::Ordering::Relaxed);
            tracing::info!(
                target: "shape_jit::arc_counters",
                retain_calls = retain_after - retain_before,
                release_calls = release_after - release_before,
                release_frees = frees_after - frees_before,
                "shape-jit-arc counter delta",
            );
            // cluster-2-cw-E §F measurement output: per-call-site
            // §2.7.5 String carrier metrics. Leak quantification
            // shape is `str_allocs - str_frees` = number of
            // permanently-leaked Arc<String> allocations for this
            // execution. The "_cum" event is process-wide running
            // total — surfaces compile-time allocations that happen
            // before any jit_fn invocation (the dominant source for
            // `MirConstant::Str` materialization). Migrated to
            // tracing::info! per cw-F mechanism at Round 1 merge
            // take-both ceremony (2026-05-16).
            tracing::info!(
                target: "shape_jit::arc_counters",
                str_allocs = str_allocs_after - str_allocs_before,
                str_retain = str_retain_after - str_retain_before,
                str_release = str_release_after - str_release_before,
                str_frees = str_frees_after - str_frees_before,
                leaked = (str_allocs_after - str_allocs_before)
                    .saturating_sub(str_frees_after - str_frees_before),
                "shape-jit-arc-str counter delta",
            );
            tracing::info!(
                target: "shape_jit::arc_counters",
                str_allocs_total = str_allocs_after,
                str_retain_total = str_retain_after,
                str_release_total = str_release_after,
                str_frees_total = str_frees_after,
                leaked_total = str_allocs_after.saturating_sub(str_frees_after),
                "shape-jit-arc-str cumulative",
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
            // Recoverable Shape-level runtime errors the bytecode VM handles
            // cleanly are carved out of the negative-signal space. The JIT
            // executed soundly up to the error point, so they are returned as
            // `Ok(Err(_))` and propagated directly by `execute_program` — NOT
            // an outer `Err`, which would fall through to an interpreter
            // re-run and double any prior side effects (the double-execution
            // bug r5c-2-gz-cp2-jit-div found). An unknown negative signal is a
            // JIT-pipeline failure (outer `Err` -> interpreter fall-through,
            // preserving W12 behavior).
            match signal {
                crate::context::JIT_SIGNAL_DIVISION_BY_ZERO => {
                    // r5c-2-gz-cp2-jit-div: JIT codegen emits a guarded branch
                    // returning this signal instead of a `ud2`/`sdiv` trap.
                    return Ok(Err(shape_runtime::error::ShapeError::RuntimeError {
                        message: "Division by zero".to_string(),
                        location: None,
                    }));
                }
                crate::context::JIT_SIGNAL_INDEX_OUT_OF_BOUNDS => {
                    // WS-3 F1: JIT typed-array codegen emits a guarded branch
                    // returning this signal on an out-of-bounds element
                    // access instead of silently fabricating the element-type
                    // zero (read) / skipping the store (write). Maps to the
                    // same `Index out of bounds` diagnostic the bytecode VM
                    // emits for `VMError::IndexOutOfBounds`, so `--mode jit`
                    // reports the SAME error as `--mode vm`.
                    return Ok(Err(shape_runtime::error::ShapeError::RuntimeError {
                        message: "Index out of bounds".to_string(),
                        location: None,
                    }));
                }
                crate::context::SIGNAL_TRAMPOLINE_ERROR => {
                    // r5c-2-bz-b-jit-err-surface: a VM-trampoline FFI call
                    // (`jit_call_method`) surfaced a clean `Err` (e.g.
                    // `Set.add()` with a non-string key) and the JIT frame was
                    // abandoned before the placeholder result could reach a
                    // heap-kinded refcount-retain site. The VM-side message
                    // was stored in the `JIT_RUNTIME_ERROR` thread-local —
                    // surface it verbatim so `--mode jit` reports the SAME
                    // error the interpreter would.
                    let message =
                        match crate::ffi::control::take_jit_runtime_error() {
                            Some(vm_err) => vm_err,
                            None => format!("JIT execution error (code: {})", signal),
                        };
                    return Ok(Err(shape_runtime::error::ShapeError::RuntimeError {
                        message,
                        location: None,
                    }));
                }
                _ => {
                    return Err(shape_runtime::error::ShapeError::RuntimeError {
                        message: format!("JIT execution error (code: {})", signal),
                        location: None,
                    });
                }
            }
        }
        // Clear any stale trampoline error on the success path so it cannot
        // leak into a later, unrelated JIT execution on the same thread.
        let _ = crate::ffi::control::take_jit_runtime_error();

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
            tracing::info!(
                target: "shape_jit::metrics",
                bytecode_compile_ms = bytecode_compile_ms,
                jit_compile_ms = jit_compile_ms,
                jit_exec_ms = jit_exec_ms,
                total_ms = total_ms,
                "shape-jit-phases timing",
            );
        }

        // r5c-2-gz-cp2-jit-div: `Ok(Ok(_))` — JIT compiled and executed
        // successfully (see `execute_with_jit` nested-result contract).
        Ok(Ok(shape_runtime::engine::ProgramExecutorResult {
            wire_value,
            type_info: None,
            execution_type: ExecutionType::Script,
            content_json: None,
            content_html: None,
            content_terminal: None,
        }))
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
