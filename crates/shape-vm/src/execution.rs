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
use crate::executor::{ForeignFunctionHandle, VMConfig, VirtualMachine};

use shape_ast::Program;
use shape_runtime::context::ExecutionContext;
use shape_runtime::engine::{ExecutionType, ProgramExecutor, ShapeEngine};
use shape_runtime::error::Result;
use shape_runtime::wire_conversion;
use shape_value::KindedSlot;

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
        let known_binding_types: Vec<(String, String)> =
            if let Some(ctx) = runtime.persistent_context() {
                known_bindings
                    .iter()
                    .filter_map(|name| {
                        let value = ctx.get_variable(name).ok().flatten()?;
                        let type_name = binding_type_name_for_kind(
                            value.kind(),
                            value.raw(),
                            &schema_id_to_name,
                        )?;
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

        let mut loader = self.module_loader.take().unwrap_or_else(
            shape_runtime::module_loader::ModuleLoader::new,
        );
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

    /// Resume execution from a snapshot — Phase-2c stub.
    ///
    /// The legacy body built a `Snapshot::Resumed` marker via the deleted
    /// `create_typed_enum_nb` returning a `ValueWord`, pushed it via the
    /// deleted raw-bits stack push, then ran the suspend/resume loop —
    /// every step of which depended on `ValueWord` / `EnumValue` /
    /// `nb_to_wire`. Phase-2c (ADR-006 §2.7.4) rebuilds the marker as a
    /// kinded `Arc<TypedObjectStorage>` payload + parallel-kind track,
    /// pushed via `push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))`.
    pub fn resume_snapshot(
        &self,
        _engine: &mut ShapeEngine,
        _vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        _bytecode: BytecodeProgram,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        Err(shape_runtime::error::ShapeError::RuntimeError {
            message: "resume_snapshot: snapshot rebuild depends on the deleted \
                      ValueWord carrier and the deleted `create_typed_enum_nb` / \
                      `nb_to_wire` host-API surface — Phase-2c, see ADR-006 §2.7.4."
                .to_string(),
            location: None,
        })
    }

    /// Recompile source and resume from a snapshot — Phase-2c stub.
    ///
    /// Same surface as `resume_snapshot`: the snapshot-to-host marker
    /// hop depends on the deleted `ValueWord` carrier (ADR-006 §2.7.4).
    pub fn recompile_and_resume(
        &mut self,
        _engine: &mut ShapeEngine,
        _vm_snapshot: shape_runtime::snapshot::VmSnapshot,
        _old_bytecode: BytecodeProgram,
        _program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        Err(shape_runtime::error::ShapeError::RuntimeError {
            message: "recompile_and_resume: snapshot resume depends on the \
                      deleted ValueWord carrier and the kinded suspend/resume \
                      marker rebuild is Phase-2c (ADR-006 §2.7.4)."
                .to_string(),
            location: None,
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
    fn load_module_bindings_from_context(
        vm: &mut VirtualMachine,
        ctx: &ExecutionContext,
    ) {
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
        let _schema_scope = engine.runtime.enter_schema_scope();

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

        // Build a VM and prime extensions / foreign-function links.
        // These steps don't reach into the deleted ValueWord carrier
        // themselves; the host-boundary persistence + completion-value
        // synthesis is what's deferred to Phase-2c.
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.set_interrupt(self.interrupt.clone());
        vm.load_program(bytecode);
        for ext in &self.extensions {
            vm.register_extension(ext.clone());
        }
        // populate_module_objects is itself a Phase-2c stub (see
        // vm_impl/modules.rs) — calling it is a no-op until the kinded
        // module-binding cell-storage rebuild lands per ADR-006 §2.7.8 / Q10.
        vm.populate_module_objects();
        vm.foreign_fn_handles.clear();
        if !vm.program.foreign_functions.is_empty() {
            let entries = vm.program.foreign_functions.clone();
            let mut handles: Vec<Option<ForeignFunctionHandle>> = Vec::with_capacity(entries.len());
            let mut native_library_cache: std::collections::HashMap<
                String,
                std::sync::Arc<libloading::Library>,
            > = std::collections::HashMap::new();
            for (idx, entry) in entries.iter().enumerate() {
                if let Some(native_spec) = &entry.native_abi {
                    let linked = crate::executor::native_abi::link_native_function(
                        native_spec,
                        &vm.program.native_struct_layouts,
                        &mut native_library_cache,
                    )
                    .map_err(|e| {
                        shape_runtime::error::ShapeError::RuntimeError {
                            message: format!(
                                "Failed to link native function '{}': {}",
                                entry.name, e
                            ),
                            location: None,
                        }
                    })?;
                    vm.program.foreign_functions[idx].dynamic_errors = false;
                    handles.push(Some(ForeignFunctionHandle::Native(std::sync::Arc::new(
                        linked,
                    ))));
                    continue;
                }
                handles.push(None);
            }
            vm.foreign_fn_handles = handles;
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

        let completion: KindedSlot = vm.execute(Some(ctx_borrow)).map_err(|e| {
            shape_runtime::error::ShapeError::RuntimeError {
                message: e.to_string(),
                location: None,
            }
        })?;

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
        let (content_json, content_html, content_terminal) =
            wire_conversion::slot_extract_content(bits, kind);

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

#[cfg(test)]
mod tests {
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
}
