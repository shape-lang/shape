//! Program compilation and execution logic.
//!
//! Contains the VM execution loop, module_binding variable synchronization,
//! snapshot resume, compilation pipeline, and trait implementations
//! for `ProgramExecutor` and `ExpressionEvaluator`.
//!
//! W12-host-boundary (ADR-006 §2.7.4 / §2.7.5): the program-completion
//! host boundary now flows the VM's `KindedSlot` completion value
//! through the kind-threaded `wire_conversion::slot_to_envelope` /
//! `slot_to_wire` / `slot_extract_content` helpers. The deleted
//! `nb_to_wire` / `nb_to_envelope` / `nb_extract_content` /
//! `synthesize_value_word_from_raw` ValueWord-shape host-API surface
//! does not return; the kinded helpers take `(bits, kind)` directly per
//! ADR-006 §2.7.5 and the slot's kind is sourced from `KindedSlot::kind`
//! (compiler-proven via `BytecodeProgram::top_level_frame.return_kind`).
//!
//! Snapshot resume / `eval_statements` remain Phase-2c stubs — the
//! suspend/resume marker rebuild (kinded `Snapshot::Resumed`
//! constructor + push) and the REPL-binding round-trip
//! (`save_module_bindings_to_context` / `load_module_bindings_from_context`)
//! are independent host-boundary workstreams.

use std::sync::Arc;

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;
use crate::configuration::BytecodeExecutor;
use crate::executor::{VMConfig, VirtualMachine};

use shape_ast::Program;
use shape_runtime::context::ExecutionContext;
use shape_runtime::engine::{ExecutionType, ProgramExecutor, ShapeEngine};
use shape_runtime::error::Result;
use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_runtime::type_schema::builtin_schemas::{
    OPTION_PAYLOAD, OPTION_VARIANT, OPTION_VARIANT_NONE,
};
use shape_runtime::wire_conversion;
use shape_value::{HeapKind, KindedSlot, NativeKind};

impl BytecodeExecutor {
    /// Compile a program to bytecode without executing it.
    ///
    /// This performs the same compilation pipeline as `execute_program`
    /// (merging core stdlib, extensions, virtual modules) but stops
    /// before creating a VM or executing. Compilation does not depend on
    /// the deleted `ValueWord` carrier — it returns `BytecodeProgram`
    /// directly.
    pub(crate) fn compile_program_impl(
        &mut self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<BytecodeProgram> {
        let source_for_compilation = engine.current_source().map(|s| s.to_string());

        // Check bytecode cache before expensive compilation
        if let (Some(cache), Some(source)) = (&self.bytecode_cache, &source_for_compilation) {
            if let Some(cached) = cache.get(source) {
                return Ok(cached);
            }
        }

        // Install this engine's runtime-scoped TypeSchemaRegistry as the
        // ambient handle for the duration of compilation.
        let _schema_scope = engine.runtime.enter_schema_scope();

        // REPL cross-cell persistence (WS-11): the persisted user type
        // schemas (struct / enum) carrying their session-stable
        // `SchemaId`s, and a `schema_id -> name` reverse map for
        // resolving the type name of a persisted `TypedObject` binding.
        let persistent_schemas: Vec<shape_runtime::type_schema::TypeSchema> =
            engine.repl_user_schemas().values().cloned().collect();
        let schema_id_to_name: std::collections::HashMap<u32, String> = persistent_schemas
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();

        let runtime = engine.get_runtime_mut();

        let known_bindings: Vec<String> = if let Some(ctx) = runtime.persistent_context() {
            ctx.root_scope_binding_names()
        } else {
            Vec::new()
        };

        // REPL cross-cell persistence (WS-11): derive a compiler-facing
        // type name for each persisted binding from the value stored in
        // the context. Without this the next cell's `a + b` (where `a`,
        // `b` were `let`-bound in earlier cells) falls into the
        // strict-typing `unknown + unknown` reject path. The kind is read
        // off the persisted `KindedSlot` — no fabrication, the producer
        // stamped it (ADR-006 §2.7.5).
        let known_binding_types: Vec<(String, String)> = if let Some(ctx) =
            runtime.persistent_context()
        {
            known_bindings
                .iter()
                .filter_map(|name| {
                    let value = ctx.get_variable(name).ok().flatten()?;
                    let type_name =
                        binding_type_name_for_kind(value.kind(), value.raw(), &schema_id_to_name)?;
                    Some((name.clone(), type_name))
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut root_program = program.clone();
        crate::module_resolution::annotate_program_native_abi_package_key(
            &mut root_program,
            self.root_package_key.as_deref(),
        );

        let mut loader = self
            .module_loader
            .take()
            .unwrap_or_else(shape_runtime::module_loader::ModuleLoader::new);
        let (graph, stdlib_names, prelude_imports) =
            crate::module_resolution::build_graph_and_stdlib_names(
                &root_program,
                &mut loader,
                &self.extensions,
            )?;
        self.module_loader = Some(loader);

        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;

        // REPL cross-cell persistence (WS-11): seed the compiler's
        // schema registry with user `type` / `enum` schemas from prior
        // cells so each keeps a stable `SchemaId` for the whole session
        // — a `TypedObject` persisted across cells carries the id
        // stamped at construction, and `GetFieldTyped` resolves it
        // against this cell's program registry. Seeding before
        // `register_known_bindings` (which can itself touch schemas via
        // `register_extension_module_schema`) keeps the user ids stable.
        if !persistent_schemas.is_empty() {
            compiler.seed_persistent_schemas(&persistent_schemas);
        }

        compiler.register_known_bindings(&known_bindings);
        for (name, type_name) in &known_binding_types {
            compiler.register_known_binding_type(name, type_name);
        }

        if !self.extensions.is_empty() {
            compiler.extension_registry = Some(Arc::new(self.extensions.clone()));
        }

        if let Ok(cwd) = std::env::current_dir() {
            compiler.set_source_dir(cwd);
        }

        compiler.native_resolution_context = self.native_resolution_context.clone();

        if let Some(source) = &source_for_compilation {
            compiler.set_source(source);
        }

        let bytecode =
            compiler.compile_with_graph_and_prelude(&root_program, graph, &prelude_imports)?;

        // Store in bytecode cache (best-effort, ignore errors)
        if let (Some(cache), Some(source)) = (&self.bytecode_cache, &source_for_compilation) {
            let _ = cache.put(source, &bytecode);
        }

        Ok(bytecode)
    }

    /// Compile a program with the same pipeline as execution, but do not run it.
    pub fn compile_program_for_inspection(
        &mut self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<BytecodeProgram> {
        self.compile_program_impl(engine, program)
    }

    /// Resume execution from a snapshot with the SAME code (design §4.5.1).
    ///
    /// Orchestration over the existing STAGE-R5 `from_snapshot` restore: rebuild
    /// the VM from `VmSnapshot`, push `Ok(Snapshot::Resumed)` as the
    /// `snapshot()` call site's return value (§4.1.3), and run the normal
    /// dispatch loop from `VmSnapshot.ip`. Permissions/limits are the RESUMING
    /// host's and are re-verified before any bytecode executes (zero-trust,
    /// §4.7.3). The kinded suspend/resume marker rebuild the stub cited is done
    /// via `build_snapshot_resumed_marker` — ordinary kinded enum construction,
    /// no ValueWord (Constraint 1 / ADR-006 §2.7.4).
    pub fn resume_snapshot(
        &self,
        engine: &mut ShapeEngine,
        vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        bytecode: BytecodeProgram,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        self.resume_from_snapshot_impl(engine, vm_snapshot, bytecode)
    }

    /// Recompile edited source and resume from a snapshot (design §4.5.2).
    ///
    /// **Sound-relocation is gated on the §4.2.2 frame-relocation producer**
    /// (per-frame `local_ip` + top-level `ip_blob_hash`/`ip_local_offset`),
    /// which is not yet populated at capture — so an ip captured against the
    /// snapshot's bytecode cannot be soundly re-mapped into freshly-compiled
    /// bytecode (recompilation is not byte-stable, and heuristic line-mapping
    /// into changed bytecode is rejected, §5.11). Rather than a check that is
    /// both unreliable (identical source does not recompile identically) and
    /// unsound if it passed (no frame-safety proof), this v1 recompiles the
    /// source to validate it and then **cleanly refuses** with the §4.5.2 /
    /// §4.11 mismatch message, directing the user to plain `--resume <hash>`
    /// (which restores the snapshot's own authoritative bytecode and is fully
    /// supported). Recompile-and-resume of edited source lands with the
    /// relocation producer (design §6 Stage 4).
    pub fn recompile_and_resume(
        &mut self,
        engine: &mut ShapeEngine,
        vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        _old_bytecode: BytecodeProgram,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        use shape_runtime::error::ShapeError;

        // Recompile so a broken edit still fails at compile (not silently).
        let _new_bytecode = self.compile_program_impl(engine, program)?;
        let _ = vm_snapshot;

        // §4.5.2 mismatch table / §4.11 catalog (ResumeFunctionChanged shape):
        // edited-source resume is a clean refuse until the relocation producer
        // lands; plain resume runs the snapshot's original code.
        Err(ShapeError::RuntimeError {
            message: "cannot resume with an edited source file in this build: sound \
                      ip relocation into recompiled code requires the frame-relocation \
                      metadata that this snapshot does not yet carry. Resume the \
                      original code with `shape --resume <hash>` (no source file)."
                .to_string(),
            location: None,
        })
    }

    /// Shared restore → re-prime → resume → project spine for both resume entry
    /// points (design §4.5.1). Zero-trust: the resuming host's permission
    /// envelope (installed on `self` by the CLI) is re-verified against the
    /// program's required-permission union before any bytecode runs (§4.7.3).
    fn resume_from_snapshot_impl(
        &self,
        engine: &mut ShapeEngine,
        vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        bytecode: BytecodeProgram,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        use shape_runtime::error::ShapeError;

        // Snapshot store (needed by `from_snapshot` to fetch chunked sidecars)
        // + the envelope seed for chained snapshots on the resumed VM.
        let (store, seed) = match engine.snapshot_install_context()? {
            Some((store, seed)) => (store, seed),
            None => {
                return Err(ShapeError::RuntimeError {
                    message: "resume: no snapshot store is configured on the engine"
                        .to_string(),
                    location: None,
                });
            }
        };

        // §4.7.3 permission re-verification (fail closed BEFORE execution).
        // Zero trust in the snapshot's self-declaration: recompute the required
        // union from the program's content-addressed blobs and require it a
        // subset of the resuming host's grant.
        if let (Some(granted), Some(ca)) = (
            self.granted_permissions.as_ref(),
            bytecode.content_addressed.as_ref(),
        ) {
            let linked = crate::linker::link(ca).map_err(|e| ShapeError::SemanticError {
                message: format!("resume: failed to link snapshot program: {e}"),
                location: None,
            })?;
            if !linked.total_required_permissions.is_subset(granted) {
                let missing = linked.total_required_permissions.difference(granted);
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot resume: this snapshot needs permission(s) not granted \
                         here: {missing:?}. Re-run with those permissions granted."
                    ),
                    location: None,
                });
            }
        }

        // Restore the VM (STAGE-R5 two-pass identity restore + call-stack
        // rebuild). `from_snapshot` sets `vm.ip = snapshot.ip` — the
        // post-`snapshot()`-call instruction.
        let mut vm = VirtualMachine::from_snapshot(bytecode, &vm_snapshot, &store).map_err(|e| {
            ShapeError::RuntimeError {
                message: format!("resume: failed to restore VM state: {e}"),
                location: None,
            }
        })?;

        // Re-prime the restored VM with the resuming host's execution envelope.
        if let Some(limits) = self.resource_limits.clone() {
            vm = vm.with_resource_limits(limits);
        }
        vm.set_interrupt(self.interrupt.clone());
        vm.set_permissions(
            self.granted_permissions.clone(),
            self.scope_constraints.clone(),
        );
        for ext in &self.extensions {
            vm.register_extension(ext.clone());
        }
        vm.populate_module_objects();

        // WF-2F axis B (polyglot-distributed §4.5 resume table): a resumed VM
        // whose remaining code calls a `fn python` / `fn typescript` must be
        // able to re-link the runtime on the first post-resume foreign call.
        // The restored `from_snapshot` VM starts with an empty runtime registry
        // (STATEFUL_OPAQUE runtimes are never serialized — §4.5 non-goal), so
        // register the resuming host's own runtimes here, exactly as the fresh
        // execute path (`execution.rs` load) and the remote path (`remote.rs`)
        // do. `extern C` needs no registry (it dlopens on call). Same typed
        // threading shape; no value crosses here, only the runtime handles.
        vm.set_language_runtimes(engine.language_runtimes());
        // Re-establish the foreign stub module bindings (the `(func_id,
        // UInt64)` slots that top-level module-init writes on a fresh run) so a
        // post-resume `LoadModuleBinding` + `CallValue` to a foreign stub
        // resolves a real callee, not the uninitialised sentinel. Idempotent
        // with any bindings the snapshot already restored (writes the same
        // typed value; ADR-006 §2.7.8 kinded write, no Bool-default).
        vm.initialize_foreign_stub_bindings().map_err(|e| {
            ShapeError::RuntimeError {
                message: format!("resume: foreign stub binding init failed: {e:?}"),
                location: None,
            }
        })?;

        // Chained snapshots: a resumed VM is indistinguishable from a running
        // one and may snapshot again (§4.5.1 step 5).
        vm.set_snapshot_context(store, seed);

        // WF-3F conditional resume-marker push. Push `Ok(Snapshot::Resumed)`
        // as the value the instruction at the resume ip expects for
        // `snapshot()` (§4.1.3 / §4.5.1 step 4) ONLY for a snapshot()-call
        // origin. An interrupt-saved snapshot's ip is a rewound un-executed
        // instruction (dispatch.rs `self.ip -= 1`) that expects a PRISTINE
        // operand stack — pushing the marker there shifts the stack by one
        // slot and makes the pending call pop the marker as an argument and
        // the slot below the real callee AS the callee (the release-blocking
        // silent-corruption bug). See VmSnapshot::interrupt_saved.
        if !vm_snapshot.interrupt_saved {
            let resumed = vm
                .build_snapshot_resumed_marker()
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("resume: {e}"),
                    location: None,
                })?;
            vm.push_kinded_slot(resumed)
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("resume: failed to push resume marker: {e}"),
                    location: None,
                })?;
        }

        // Run the dispatch loop from `resume_ip` with a live context for
        // stdlib I/O + wire-conversion lookups (§4.5.1 step 5).
        let mut owned_ctx_fallback;
        let ctx_borrow: &mut ExecutionContext = match engine.runtime.persistent_context_mut() {
            Some(ctx) => ctx,
            None => {
                owned_ctx_fallback = ExecutionContext::new_empty();
                &mut owned_ctx_fallback
            }
        };

        let completion: KindedSlot = match vm.execute(Some(ctx_borrow)) {
            Ok(completion) => completion,
            Err(shape_value::VMError::Interrupted) => {
                // A resumed run can itself be interrupted (§4.5.1 step 5).
                use crate::executor::snapshot::SnapshotOutcome;
                let snapshot_hash = match vm.capture_interrupt_snapshot(Some(ctx_borrow)) {
                    SnapshotOutcome::Saved(hash) => Some(hash),
                    SnapshotOutcome::Barrier(_) | SnapshotOutcome::PersistFailed(_) => None,
                };
                return Err(ShapeError::Interrupted { snapshot_hash });
            }
            Err(e) => {
                let message = match &e {
                    shape_value::VMError::Suspended { future_id, .. }
                        if *future_id == crate::executor::SNAPSHOT_FUTURE_ID =>
                    {
                        "internal error: snapshot() reached the host boundary — \
                         this is a shape bug, please report it"
                            .to_string()
                    }
                    _ => e.to_string(),
                };
                return Err(ShapeError::RuntimeError {
                    message,
                    location: None,
                });
            }
        };

        // Host-boundary projection (§4.5.1 step 6) — identical kinded path as a
        // fresh run (`wire_conversion::slot_*`).
        let bits = completion.raw();
        let kind = completion.kind();
        let envelope = wire_conversion::slot_to_envelope(bits, kind, "", ctx_borrow);
        let (content_json, content_html, content_terminal) =
            wire_conversion::slot_extract_content(bits, kind);

        Ok(shape_runtime::engine::ProgramExecutorResult {
            wire_value: envelope.value,
            type_info: Some(envelope.type_info),
            execution_type: ExecutionType::Script,
            content_json,
            content_html,
            content_terminal,
        })
    }
}

impl shape_runtime::engine::ExpressionEvaluator for BytecodeExecutor {
    fn eval_statements(
        &self,
        _stmts: &[shape_ast::Statement],
        _ctx: &mut ExecutionContext,
    ) -> Result<KindedSlot> {
        // Phase-2c surface (ADR-006 §2.7.4): the legacy implementation
        // round-tripped the result through `vm.execute()` (which returned
        // `ValueWord`) and persisted module bindings via
        // `save_module_bindings_to_context` (which called the deleted
        // `synthesize_value_word_from_raw`). The kinded rebuild returns
        // `KindedSlot` directly from a `vm.execute_kinded()` shape and
        // persists bindings via per-slot `(bits, NativeKind)` writes —
        // both Phase-2c.
        Err(shape_runtime::error::ShapeError::RuntimeError {
            message: "eval_statements: depends on `vm.execute() -> ValueWord` \
                      and the deleted `synthesize_value_word_from_raw` \
                      host-boundary path; the kinded `vm.execute_kinded() \
                      -> KindedSlot` rebuild is Phase-2c (ADR-006 §2.7.4)."
                .to_string(),
            location: None,
        })
    }

    fn eval_expr(&self, expr: &shape_ast::Expr, ctx: &mut ExecutionContext) -> Result<KindedSlot> {
        let stmt = shape_ast::Statement::Expression(expr.clone(), shape_ast::Span::DUMMY);
        self.eval_statements(&[stmt], ctx)
    }
}

/// Derive the compiler-facing type-name string for a persisted REPL
/// binding from its runtime `NativeKind` (WS-11).
///
/// The bytecode compiler's type tracker recognises canonical type names
/// (`"int"`, `"number"`, `"bool"`, `"string"`, and registered struct /
/// enum names) via `register_known_binding_type`. Mapping the persisted
/// `KindedSlot`'s kind to one of those names lets the next cell's
/// expressions referencing the binding (`a + b`, `p.x`) compile through
/// the strict-typing path instead of the `unknown` reject.
///
/// The kind was stamped by the original producer at compile time
/// (ADR-006 §2.7.5) — this reads it, it does not fabricate it. Kinds
/// without a stable surface type name (the no-op `Bool`/`Null` sentinel
/// for never-written slots, container kinds the compiler resolves
/// structurally) return `None`; the binding then carries no type info
/// and falls back to the polymorphic path, which is correct — it is not
/// a wrong-result, just a missed specialization.
fn binding_type_name_for_kind(
    kind: shape_value::NativeKind,
    bits: u64,
    schema_id_to_name: &std::collections::HashMap<u32, String>,
) -> Option<String> {
    use shape_value::{HeapKind, NativeKind};
    match kind {
        NativeKind::Int64 | NativeKind::UInt64 | NativeKind::IntSize | NativeKind::UIntSize => {
            Some("int".to_string())
        }
        NativeKind::Int8
        | NativeKind::UInt8
        | NativeKind::Int16
        | NativeKind::UInt16
        | NativeKind::Int32
        | NativeKind::UInt32 => Some("int".to_string()),
        NativeKind::Float64 | NativeKind::Float32 => Some("number".to_string()),
        NativeKind::Bool => Some("bool".to_string()),
        NativeKind::String | NativeKind::StringV2 => Some("string".to_string()),
        NativeKind::Ptr(HeapKind::TypedObject) => {
            // The slot bits are a live `*const TypedObjectStorage`
            // (ADR-006 §2.4 v2-raw carrier). Read its `schema_id` and
            // resolve the registered struct name via the persisted
            // user-schema id map so the next cell's `binding.field`
            // access compiles through the typed-field path.
            if bits == 0 {
                return None;
            }
            let ptr = bits as *const shape_value::heap_value::TypedObjectStorage;
            // SAFETY: a `Ptr(HeapKind::TypedObject)`-kinded slot whose
            // bits are non-zero points at a live `TypedObjectStorage`
            // (the context's `KindedSlot` holds an owning share for the
            // duration of this read). `schema_id` is a POD `u64` field.
            let schema_id = unsafe { (*ptr).schema_id };
            schema_id_to_name.get(&(schema_id as u32)).cloned()
        }
        // Other heap kinds (arrays, maps, options, results, …) are
        // resolved structurally by the compiler; no flat type name to
        // register. Bool/Null sentinel and remaining scalar kinds carry
        // no useful binding-type info.
        _ => None,
    }
}

/// Collect the names introduced by top-level `let` / `var` / `const`
/// declarations in a program (REPL cross-cell persistence, WS-11).
///
/// A top-level binding can appear either as `Item::VariableDecl` or as
/// `Item::Statement(Statement::VariableDecl(..))` depending on how the
/// parser bucketed the line; both shapes are walked. Destructuring
/// patterns contribute every bound identifier.
fn collect_top_level_binding_names(program: &Program) -> std::collections::HashSet<String> {
    use shape_ast::ast::{Item, Statement};
    let mut names = std::collections::HashSet::new();
    let mut absorb = |decl: &shape_ast::ast::VariableDecl| {
        for ident in decl.pattern.get_identifiers() {
            names.insert(ident);
        }
    };
    for item in &program.items {
        match item {
            Item::VariableDecl(decl, _) => absorb(decl),
            Item::Statement(Statement::VariableDecl(decl, _), _) => absorb(decl),
            _ => {}
        }
    }
    names
}

impl BytecodeExecutor {
    /// REPL load-side binding round-trip (WS-11).
    ///
    /// For every VM module-binding slot whose name matches a value
    /// binding live in the persistent `ExecutionContext`'s root scope,
    /// copy the context's `KindedSlot` into the slot. This is what makes
    /// a variable defined in a prior cell resolvable from a later one:
    /// the compiler reserved the slot via `register_known_bindings`, and
    /// this fills it with the persisted value before execution.
    ///
    /// The copy retains an independent strong-count share — the
    /// context's variable keeps its own ownership; the VM slot owns the
    /// clone, and `module_binding_write_kinded` releases whatever
    /// occupied the slot before. No tag synthesis: `KindedSlot` already
    /// carries the `NativeKind`, so the slot's bits and kind transfer
    /// directly per ADR-006 §2.7.8 / Q10.
    fn load_module_bindings_from_context(vm: &mut VirtualMachine, ctx: &ExecutionContext) {
        let names = vm.program.module_binding_names.clone();
        for (idx, name) in names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            // Only root-scope context variables are user value bindings;
            // `get_variable` searches inner-to-outer but the REPL context
            // has a single root scope between cells.
            let Ok(Some(value)) = ctx.get_variable(name) else {
                continue;
            };
            // `value` is a fresh clone with its own share (KindedSlot's
            // Clone retains). Hand that share to the binding slot.
            let bits = value.raw();
            let kind = value.kind();
            std::mem::forget(value);
            vm.module_binding_write_kinded(idx, bits, kind);
        }
    }

    /// REPL save-side binding round-trip (WS-11).
    ///
    /// After a cell executes, copy every VM module-binding slot whose
    /// name is a user value binding (`user_binding_names`) back into the
    /// persistent `ExecutionContext` so the next cell sees it. The name
    /// filter excludes module-namespace objects and stdlib/prelude
    /// function bindings — those are not user values and must not leak
    /// into the context's variable scope.
    ///
    /// `module_binding_read_owned_kinded` bumps the strong-count so the
    /// VM's own slot (dropped with the VM at end of cell) and the
    /// context's stored copy each hold an independent share.
    fn save_module_bindings_to_context(
        vm: &VirtualMachine,
        ctx: &mut ExecutionContext,
        user_binding_names: &std::collections::HashSet<String>,
    ) {
        let names = vm.program.module_binding_names.clone();
        for (idx, name) in names.iter().enumerate() {
            if name.is_empty() || !user_binding_names.contains(name) {
                continue;
            }
            if idx >= vm.module_bindings_len() {
                continue;
            }
            let value = vm.module_binding_read_owned_kinded(idx);
            // `set_variable` updates an existing variable or creates a
            // fresh `var`; either way the persisted slot now holds this
            // cell's final value for `name`.
            let _ = ctx.set_variable(name, value);
        }
    }
}

impl ProgramExecutor for BytecodeExecutor {
    fn execute_program(
        &mut self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        // Phase 1 — compile (does not depend on the deleted ValueWord).
        // `compile_program_impl` installs the runtime's schema scope for the
        // duration of the compile; `execute_compiled` re-installs it for
        // execution. No ambient schema-registry scope spans the two phases,
        // so a caller that already holds a compiled `BytecodeProgram` can run
        // it via `execute_compiled` without a second compile advancing the
        // shared `schema_registry.next_id` counter.

        // REPL cross-cell persistence (WS-11): re-prepend `fn` / `type` /
        // `enum` / `trait` / `impl` / type-alias / annotation definitions
        // from prior cells so the bytecode compiler resolves names
        // declared in earlier lines. `execute_program` owns the only
        // cross-cell-stable handle (`ShapeEngine`) and is the single
        // path every executor caller routes through (real `execute_repl`,
        // notebook, test helpers), so the injection lives here rather
        // than in any one entry point.
        let augmented_program: Program;
        let compile_target: &Program = if engine.has_repl_definitions() {
            let priors = engine.repl_definitions();
            let mut items = Vec::with_capacity(priors.len() + program.items.len());
            items.extend(priors.iter().cloned());
            items.extend(program.items.iter().cloned());
            augmented_program = Program {
                items,
                docs: program.docs.clone(),
            };
            &augmented_program
        } else {
            program
        };

        let bytecode = self.compile_program_impl(engine, compile_target)?;

        // Phase 2 — execute the just-built bytecode on the interpreter.
        // Extracted into `execute_compiled` so the JIT `[jit-fallback]` path
        // (`crates/shape-jit/src/executor.rs`) can run the ALREADY-COMPILED
        // inspection bytecode WITHOUT a second `compile_program_impl`. The
        // double-compile was the definitional cause of the object-merge
        // schema-id collision (WF-1A-followup / fix-plan §6quinquies): the
        // inspection compile advances `engine.runtime.schema_registry.next_id`
        // (ambient-domain merged/named-type id allocation), and a second
        // compile on the same counter-advanced registry shifts those ambient
        // ids UP while the compiler-LOCAL inline-object ids reset to their
        // per-compile base — colliding in the shared `SchemaId` namespace and
        // resolving a wide merged object against a narrow inline schema
        // (`MakeFieldRef field_idx N out of bounds`). Running the pre-built
        // bytecode removes the second compile, so the fallback observes no
        // schema-registry state mutated by the inspection compile.
        self.execute_compiled(engine, bytecode, program)
    }
}

impl BytecodeExecutor {
    /// Execute an ALREADY-COMPILED `bytecode` under the bytecode interpreter
    /// and project its completion value across the host boundary.
    ///
    /// This is the post-compile half of [`ProgramExecutor::execute_program`],
    /// extracted so a caller that already holds a compiled `BytecodeProgram`
    /// can run it WITHOUT recompiling. `program` is the ORIGINAL
    /// (un-augmented) source program; it is consumed only for REPL cross-cell
    /// bookkeeping and completion-shape terminal rendering — it is never
    /// recompiled here.
    ///
    /// The JIT `[jit-fallback]` path calls this with the bytecode built by
    /// `compile_program_for_inspection`, so the fallback interpreter run
    /// never observes schema-registry state (the `next_id` counter on
    /// `engine.runtime.schema_registry`) mutated by a second compile
    /// (WF-1A-followup / fix-plan §6quinquies). Because it runs the exact
    /// same bytecode `--mode vm` would compile, it is provably
    /// semantics-identical to the VM oracle.
    pub fn execute_compiled(
        &mut self,
        engine: &mut ShapeEngine,
        bytecode: BytecodeProgram,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        // Install this engine's runtime-scoped schema registry as the ambient
        // handle for the duration of execution — wire-conversion / completion
        // projection consult `current_registry()`.
        let _schema_scope = engine.runtime.enter_schema_scope();

        // Build a VM and prime extensions / foreign-function links.
        // These steps don't reach into the deleted ValueWord carrier
        // themselves; the host-boundary persistence + completion-value
        // synthesis is what's deferred to Phase-2c.
        let mut vm = VirtualMachine::new(VMConfig::default());
        // Install resource limits (if configured) so a runaway program fails
        // in-process via the dispatch-loop tick_instruction / record_allocation
        // caps rather than exhausting the host. `None` (default) = unlimited,
        // preserving trusted CLI semantics.
        if let Some(limits) = self.resource_limits.clone() {
            vm = vm.with_resource_limits(limits);
        }
        vm.set_interrupt(self.interrupt.clone());

        // WF-1D security wiring: install the runtime permission envelope so
        // gated stdlib dispatch (file / net / process / env) is checked at
        // call time. `None` = allow-all, preserved ONLY for genuinely-trusted
        // local `unlimited()` runs; serve / remote / wire install a concrete
        // set so they fail closed.
        vm.set_permissions(
            self.granted_permissions.clone(),
            self.scope_constraints.clone(),
        );

        // Load-time capability gate — "permissions baked into content hash,
        // checked at load time". When a granted set is installed AND the
        // program is content-addressed, verify its transitive required
        // permissions are a subset of the grant and fail closed BEFORE
        // executing a single instruction. Non-content-addressed programs fall
        // through to the runtime `check_permission` gate installed above.
        match self.granted_permissions.clone() {
            Some(granted) => match bytecode.content_addressed.clone() {
                Some(ca) => {
                    vm.load_program_with_permissions(ca, &granted).map_err(|e| {
                        shape_runtime::error::ShapeError::SemanticError {
                            message: format!("Permission denied at load: {e}"),
                            location: None,
                        }
                    })?;
                }
                None => vm.load_program(bytecode),
            },
            None => vm.load_program(bytecode),
        }
        // ffi-rebuild §4.11 / WF-2A: thread the resolved package-scoped
        // `[native-dependencies]` map (set by the CLI's
        // `wire_vm_executor_module_loading` → `set_native_resolution_context`)
        // into the VM so the link-now path resolves an `extern C` declaration's
        // `[native-dependencies]` alias to its real vendored/path `dlopen`
        // target instead of `dlopen`-ing the bare alias string.
        vm.set_native_resolutions(
            self.native_resolution_context
                .clone()
                .map(std::sync::Arc::new),
            self.root_package_key.clone(),
        );
        for ext in &self.extensions {
            vm.register_extension(ext.clone());
        }
        // populate_module_objects is itself a Phase-2c stub (see
        // vm_impl/modules.rs) — calling it is a no-op until the kinded
        // module-binding cell-storage rebuild lands per ADR-006 §2.7.8 / Q10.
        vm.populate_module_objects();

        // WF-2A stage 1 — LAZY LINKING (ffi-rebuild §4.2). The eager
        // link-at-load loop is DELETED: declaring a foreign function
        // (`extern C fn` / `fn python` / `fn typescript`) is NEVER fatal.
        // Every handle starts `None`; `op_call_foreign` / the shared
        // `invoke_foreign_kinded` core performs link-now on first call and
        // surfaces link/compile failures there as structured errors. This
        // also drops the deleted loop's `dynamic_errors = false` runtime flip
        // — the compile side already stamps `dynamic_errors: dynamic_language`
        // on the entry, so no consumer loses the flag.
        vm.foreign_fn_handles =
            std::iter::repeat_with(|| None)
                .take(vm.program.foreign_functions.len())
                .collect();
        // Install the VM-level language-runtime registry (ffi-rebuild §4.2) so
        // the dynamic foreign-call link-now path can resolve its runtime —
        // same threading shape `remote.rs` uses.
        vm.set_language_runtimes(engine.language_runtimes());

        // Opt-in eager foreign linking (WF-2A stage 1, `shape run --eager-link`
        // / CI validation): link/compile EVERY foreign function up front,
        // reporting ALL failures, BEFORE executing a single instruction. The
        // default stays lazy.
        if engine.eager_link_foreign() {
            if let Err(errors) = vm.eager_link_all() {
                return Err(shape_runtime::error::ShapeError::RuntimeError {
                    message: format!(
                        "eager foreign linking failed ({} error(s)):\n  - {}",
                        errors.len(),
                        errors.join("\n  - ")
                    ),
                    location: None,
                });
            }
        }

        // Install the snapshot persistence context (design §4.1 / §4.3.4) so
        // an in-flight `snapshot()` captures → persists → continues. Must run
        // after `load_program` (which populates `function_hashes`) so the
        // cached `CodeManifest` sees the program's blob hashes. `None` = the
        // engine has no store configured; `snapshot()` then refuses cleanly
        // with the `NoStore` barrier rather than trapping.
        if let Some((snap_store, snap_seed)) = engine.snapshot_install_context()? {
            vm.set_snapshot_context(snap_store, snap_seed);
        }

        // Phase 2 — execute. `vm.execute(ctx)` returns
        // `Result<KindedSlot, VMError>` (dispatch.rs:25). The slot's
        // kind is sourced from `BytecodeProgram::top_level_frame.
        // return_kind` for typed-producer programs and from the
        // §2.7.7 stack parallel-kind track when the producer pushed a
        // post-resolution kind directly. No tag-bit decode, no
        // ValueWord round-trip.
        // REPL cross-cell persistence (WS-11): the set of names the user
        // can reference as value bindings across cells — every name
        // already live in the persistent context's root scope plus every
        // top-level `let`/`var` declared in this cell. Module-namespace
        // objects, stdlib functions, and prelude builtins are never in
        // this set (they are not `VariableDecl`s and the prior cell never
        // `set_variable`d them), so the round-trip touches user variables
        // only.
        let repl_persistence = engine.repl_persistence();
        let user_binding_names: std::collections::HashSet<String> = if repl_persistence {
            let mut names = collect_top_level_binding_names(program);
            if let Some(ctx) = engine.runtime.persistent_context() {
                names.extend(ctx.root_scope_binding_names());
            }
            names
        } else {
            std::collections::HashSet::new()
        };

        let runtime = engine.get_runtime_mut();
        runtime.clear_last_runtime_error();
        let mut owned_ctx_fallback;
        let ctx_borrow: &mut ExecutionContext = match runtime.persistent_context_mut() {
            Some(ctx) => ctx,
            None => {
                // Programs without a persistent ExecutionContext (the
                // non-REPL `shape run` path) still need a live context
                // for stdlib I/O dispatch + wire-conversion lookups.
                // An empty context exposes no host data but satisfies
                // the borrow.
                owned_ctx_fallback = ExecutionContext::new_empty();
                &mut owned_ctx_fallback
            }
        };

        // REPL load-side: copy every persisted value binding from the
        // context into its VM module-binding slot before execution, so a
        // reference to a variable defined in a prior cell resolves.
        if repl_persistence {
            Self::load_module_bindings_from_context(&mut vm, ctx_borrow);
        }

        // Install the per-execution heap-growth budget (if a memory cap is
        // configured) for the duration of this VM run. The doubling-realloc
        // growth paths (TypedArray, etc.) charge against it, so an unbounded
        // allocating loop fails in-process at the cap rather than climbing RSS
        // until the host OOM-killer reaps the process. The guard restores the
        // prior budget on drop, so nested/sequential executions are isolated.
        // No memory cap configured (CLI default) => unlimited, no-op.
        let _alloc_budget_guard = shape_value::v2::alloc_budget::BudgetGuard::new(
            self.resource_limits
                .as_ref()
                .and_then(|l| l.max_memory_bytes),
        );

        // WF-2E (2026-07-05): install the program's own schema registry
        // (the superset containing inline object-literal schemas, merged
        // stdlib, and user types — the same registry the VM hands module
        // bodies as `ctx.schemas`) as the ambient thread-local scope for
        // the duration of this run. Without this, marshal-boundary readers
        // that resolve a `TypedObject`'s field names through the ambient
        // `lookup_schema_by_id_public` (e.g. `FromSlot<JsonValue>` in a
        // native module arg that has no per-arg `ModuleContext`) fall back
        // to `runtime.schema_registry_arc()`, which never received the
        // inline schemas, and fail with "unknown TypedObject schema id N".
        // The guard restores the prior ambient value on drop.
        let _program_schema_scope = shape_runtime::type_schema::SyncRegistryScope::enter(
            std::sync::Arc::new(vm.program.type_schema_registry.clone()),
        );

        let completion: KindedSlot = match vm.execute(Some(ctx_borrow)) {
            Ok(completion) => completion,
            Err(shape_value::VMError::Interrupted) => {
                // Design §4.4: the Ctrl+C host-boundary consumer. Capture →
                // persist → terminate. The VM already carries the snapshot
                // context (installed above) and `self.ip` is the un-executed
                // instruction (§4.4 no-skip rule), so this persists a valid
                // resume point. The CLI prints the resume command and exits
                // 130 on `ShapeError::Interrupted`.
                use crate::executor::snapshot::SnapshotOutcome;
                let snapshot_hash = match vm.capture_interrupt_snapshot(Some(ctx_borrow)) {
                    SnapshotOutcome::Saved(hash) => Some(hash),
                    // No-save (a persistent barrier / store failure): terminate
                    // now with nothing written (design §4.4 terminate-immediate).
                    SnapshotOutcome::Barrier(_) | SnapshotOutcome::PersistFailed(_) => None,
                };
                return Err(shape_runtime::error::ShapeError::Interrupted { snapshot_hash });
            }
            Err(e) => {
                // Design §4.4: the in-loop consumer (§4.1) handles every
                // `snapshot()` suspension, so a `SNAPSHOT_FUTURE_ID`
                // suspension is unreachable-by-construction here. If one ever
                // does reach the host boundary, render a named-bug message —
                // NEVER the leaked internal
                // `Suspended on future 18446744073709551615` sentinel string
                // (design §4.11 rendering rule; this is the leak class the
                // rule kills).
                let message = match &e {
                    shape_value::VMError::Suspended { future_id, .. }
                        if *future_id == crate::executor::SNAPSHOT_FUTURE_ID =>
                    {
                        "internal error: snapshot() reached the host boundary — \
                         this is a shape bug, please report it"
                            .to_string()
                    }
                    _ => e.to_string(),
                };
                let payload = vm.take_last_uncaught_exception().map(|payload| {
                    uncaught_exception_payload_to_wire(
                        payload,
                        vm.builtin_schemas.any_error as u64,
                        ctx_borrow,
                    )
                });
                runtime.set_last_runtime_error(payload);
                return Err(shape_runtime::error::ShapeError::RuntimeError {
                    message,
                    location: None,
                });
            }
        };

        // Post-run memory-ceiling backstop. A breach recorded on an
        // infallible allocation path is normally surfaced at the next
        // dispatch-loop safepoint, but a JIT-native run carries no
        // per-instruction safepoint — so if one was recorded during this
        // execution and the run still returned `Ok`, surface it here as a
        // clean error rather than returning a truncated result. The offending
        // buffer was already bounded at the ceiling (grow refused), so this is
        // purely the surfacing hop — never a panic.
        if let Some(breach) = shape_value::v2::alloc_budget::take_breach() {
            return Err(shape_runtime::error::ShapeError::RuntimeError {
                message: breach.to_string(),
                location: None,
            });
        }

        // REPL save-side: copy this cell's value bindings back into the
        // context so the next cell can reference them.
        if repl_persistence {
            Self::save_module_bindings_to_context(&vm, ctx_borrow, &user_binding_names);
        }

        // Phase 3 — host-boundary projection. Pull `(bits, kind)` off
        // the `KindedSlot` once and feed the kinded
        // `wire_conversion::slot_*` helpers (ADR-006 §2.7.5). The
        // KindedSlot owns the strong-count share for the duration of
        // this scope; the helpers read by-pointer and do not consume
        // the share.
        let bits = completion.raw();
        let kind = completion.kind();

        let envelope = wire_conversion::slot_to_envelope(bits, kind, "", ctx_borrow);
        let (content_json, content_html, mut content_terminal) =
            wire_conversion::slot_extract_content(bits, kind);
        if content_terminal.is_none() {
            content_terminal = completion_shape_terminal_rendering(
                &completion,
                &vm.program.type_schema_registry,
                program,
                &vm.program,
            );
        }

        // The `ctx_borrow` reborrow of `engine.runtime` ends at its last
        // use above (NLL), freeing `engine` for the persistence
        // bookkeeping below.
        //
        // REPL cross-cell persistence (WS-11): the cell executed cleanly,
        // so fold its definition items into the accumulator for the next
        // cell. Harvest from the ORIGINAL `program` (not the augmented
        // one) — prior definitions are already in the accumulator and
        // re-absorbing them would be redundant (the identity-dedup makes
        // it harmless either way, but harvesting the cell's own items is
        // the precise intent).
        if repl_persistence {
            // Record each user `type` / `enum` schema under its
            // first-assigned id. `remember_repl_user_schema` is
            // first-write-wins, so a type compiled in an earlier cell
            // keeps that cell's id — exactly the id every already
            // persisted instance of the type carries. Harvest from the
            // cell's own definition items; types declared in this cell
            // for the first time are captured here, and re-injected
            // prior types resolve to their seeded (already-recorded)
            // schema so the `or_insert` is a no-op.
            for type_name in ShapeEngine::repl_user_type_names(program) {
                if let Some(schema) = vm.program.type_schema_registry.get(&type_name) {
                    engine.remember_repl_user_schema(schema.clone());
                }
            }
            engine.absorb_repl_cell_definitions(program);
        }

        Ok(shape_runtime::engine::ProgramExecutorResult {
            wire_value: envelope.value,
            type_info: Some(envelope.type_info),
            execution_type: ExecutionType::Script,
            content_json,
            content_html,
            content_terminal,
        })
    }
}

fn completion_shape_terminal_rendering(
    completion: &KindedSlot,
    schema_registry: &TypeSchemaRegistry,
    source_program: &Program,
    bytecode: &BytecodeProgram,
) -> Option<String> {
    if completion_is_option_none_carrier(completion, schema_registry) {
        let formatter = crate::executor::printing::ValueFormatter::new(schema_registry);
        return Some(formatter.format_kinded(completion));
    }

    if completion.kind() == NativeKind::Null
        && top_level_null_completion_is_shape_none(source_program, bytecode)
    {
        return Some("None".to_string());
    }

    None
}

fn completion_is_option_none_carrier(
    completion: &KindedSlot,
    schema_registry: &TypeSchemaRegistry,
) -> bool {
    match completion.kind() {
        NativeKind::Ptr(HeapKind::TypedObject) => {
            let Some(storage) = completion.as_typed_object_storage() else {
                return false;
            };
            let Some(schema) = schema_registry.get_by_id(storage.schema_id as u32) else {
                return false;
            };
            if schema.name != "__Option"
                || storage.slots().len() <= OPTION_PAYLOAD
                || storage.field_kinds.len() <= OPTION_PAYLOAD
                || storage.field_kinds[OPTION_VARIANT] != NativeKind::Int64
            {
                return false;
            }
            storage.slots()[OPTION_VARIANT].as_i64() == OPTION_VARIANT_NONE
        }
        _ => false,
    }
}

fn top_level_null_completion_is_shape_none(
    source_program: &Program,
    bytecode: &BytecodeProgram,
) -> bool {
    let Some(expr) = top_level_tail_expr(source_program) else {
        return false;
    };
    match expr {
        shape_ast::ast::Expr::Literal(shape_ast::ast::Literal::None, _) => true,
        shape_ast::ast::Expr::FunctionCall { name, .. } => {
            bytecode_function_returns_option(bytecode, name)
        }
        shape_ast::ast::Expr::QualifiedFunctionCall {
            namespace,
            function,
            ..
        } => {
            let name = format!("{}::{}", namespace, function);
            bytecode_function_returns_option(bytecode, &name)
        }
        _ => false,
    }
}

fn top_level_tail_expr(source_program: &Program) -> Option<&shape_ast::ast::Expr> {
    use shape_ast::ast::{Item, Statement};
    source_program
        .items
        .iter()
        .rev()
        .find_map(|item| match item {
            Item::Expression(expr, _) => Some(expr),
            Item::Statement(Statement::Expression(expr, _), _) => Some(expr),
            _ => None,
        })
}

fn bytecode_function_returns_option(bytecode: &BytecodeProgram, name: &str) -> bool {
    bytecode
        .functions
        .iter()
        .find(|func| func.name == name)
        .and_then(|func| func.frame_descriptor.as_ref())
        .is_some_and(|frame| {
            frame.effective_return_wrapper() == crate::type_tracking::FrameReturnWrapper::Option
        })
}

fn uncaught_exception_payload_to_wire(
    payload: KindedSlot,
    any_error_schema_id: u64,
    ctx: &ExecutionContext,
) -> shape_wire::WireValue {
    let is_any_error = payload
        .as_typed_object_storage()
        .is_some_and(|obj| obj.schema_id == any_error_schema_id);

    let mut wire = wire_conversion::slot_to_wire(payload.raw(), payload.kind(), ctx);
    if is_any_error {
        if let shape_wire::WireValue::Object(ref mut obj) = wire {
            obj.insert(
                "category".to_string(),
                shape_wire::WireValue::String("AnyError".to_string()),
            );
        }
    }
    wire
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Function;
    use crate::type_tracking::{FrameDescriptor, FrameReturnWrapper};
    use shape_wire::WireValue;

    // The snapshot-resume integration tests (snapshot_resume_keeps_…,
    // snapshot_resumed_variant_matches_without_resume_flow,
    // stdlib_json_value_methods_can_use_internal_json_builtins,
    // snapshot_resume_direct_vm_from_snapshot_with_marker) all asserted
    // on `WireValue::as_number()` / `as_str()` / `as_bool()` round-trips
    // through the deleted ValueWord host boundary, plus called the
    // deleted `vm.create_typed_enum_nb` / `synthesize_value_word_from_raw`
    // helpers directly. They land in the Phase-2c snapshot rebuild
    // session along with their host-API counterparts (ADR-006 §2.7.4).
    //
    // No tests are kept in this module for the duration of the surface;
    // the integration coverage lives in
    // `crates/shape-vm/src/lib_tests_parts/` once the kinded host-API
    // returns.

    fn parse_program(source: &str) -> Program {
        shape_ast::parser::parse_program(source).expect("test source parses")
    }

    fn empty_bytecode() -> BytecodeProgram {
        BytecodeProgram::default()
    }

    fn option_returning_function(name: &str) -> Function {
        let mut frame = FrameDescriptor::new();
        frame.return_kind = Some(NativeKind::Ptr(HeapKind::TypedObject));
        frame.return_wrapper = FrameReturnWrapper::Option;
        Function {
            name: name.to_string(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: Some(frame),
            osr_entry_points: Vec::new(),
            mir_data: None,
        }
    }

    #[test]
    fn schema_backed_none_completion_renders_none_but_wire_stays_null() {
        let (registry, ids) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let completion = crate::executor::result_option_carrier::build_none(&ids);
        let source_program = parse_program("1");
        let bytecode = empty_bytecode();

        assert_eq!(
            completion_shape_terminal_rendering(&completion, &registry, &source_program, &bytecode),
            Some("None".to_string())
        );

        let ctx = shape_runtime::Context::new_empty();
        assert_eq!(
            wire_conversion::slot_to_wire(completion.raw(), completion.kind(), &ctx),
            WireValue::Null
        );
    }

    #[test]
    fn plain_null_completion_without_option_context_stays_silent() {
        let registry = TypeSchemaRegistry::new();
        let completion = KindedSlot::none();
        let source_program = parse_program("print(1)");
        let bytecode = empty_bytecode();

        assert_eq!(
            completion_shape_terminal_rendering(&completion, &registry, &source_program, &bytecode),
            None
        );
    }

    #[test]
    fn direct_top_level_none_null_completion_renders_none() {
        let registry = TypeSchemaRegistry::new();
        let completion = KindedSlot::none();
        let source_program = parse_program("None");
        let bytecode = empty_bytecode();

        assert_eq!(
            completion_shape_terminal_rendering(&completion, &registry, &source_program, &bytecode),
            Some("None".to_string())
        );
    }

    #[test]
    fn null_completion_from_static_option_returning_call_renders_none() {
        let registry = TypeSchemaRegistry::new();
        let completion = KindedSlot::none();
        let source_program = parse_program("maybe()");
        let bytecode = BytecodeProgram {
            functions: vec![option_returning_function("maybe")],
            ..BytecodeProgram::default()
        };

        assert_eq!(
            completion_shape_terminal_rendering(&completion, &registry, &source_program, &bytecode),
            Some("None".to_string())
        );
    }
}
