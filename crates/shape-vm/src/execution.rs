//! Program compilation and execution logic.
//!
//! Contains the VM execution loop, module_binding variable synchronization,
//! snapshot resume, compilation pipeline, and trait implementations
//! for `ProgramExecutor` and `ExpressionEvaluator`.
//!
//! Phase-2c surface (ADR-006 §2.7.4): every host-boundary path in this file
//! depends on the deleted `ValueWord` runtime representation and the
//! deleted `synthesize_value_word_from_raw` / `nb_to_wire` /
//! `nb_extract_content` / `nb_to_envelope` helpers (forbidden under
//! CLAUDE.md "Forbidden Patterns"). The kinded rebuild — `KindedSlot`-
//! backed module-binding persistence, kinded host-boundary completion
//! values, kinded suspend/resume markers — lands in the Phase-2c snapshot
//! / host-API revival session per ADR-006 §2.7.4. Until then every
//! function body surfaces `NotImplemented` / `todo!()` so the deleted
//! carriers cannot be silently re-stitched.

use std::sync::Arc;

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;
use crate::configuration::BytecodeExecutor;
use crate::executor::{ForeignFunctionHandle, VMConfig, VirtualMachine};

use shape_ast::Program;
use shape_runtime::context::ExecutionContext;
use shape_runtime::engine::{ExecutionType, ProgramExecutor, ShapeEngine};
use shape_runtime::error::Result;
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

        let runtime = engine.get_runtime_mut();

        let known_bindings: Vec<String> = if let Some(ctx) = runtime.persistent_context() {
            ctx.root_scope_binding_names()
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
        compiler.register_known_bindings(&known_bindings);

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

impl ProgramExecutor for BytecodeExecutor {
    fn execute_program(
        &mut self,
        engine: &mut ShapeEngine,
        program: &Program,
    ) -> Result<shape_runtime::engine::ProgramExecutorResult> {
        // Phase 1 — compile (does not depend on the deleted ValueWord).
        let _schema_scope = engine.runtime.enter_schema_scope();
        let bytecode = self.compile_program_impl(engine, program)?;

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

        // Phase 2 — execute. The legacy run-loop returned a tagged
        // `ValueWord` and routed it through `nb_to_wire` /
        // `nb_extract_content` / `nb_to_envelope` to build the
        // `ProgramExecutorResult`. Every one of those helpers was deleted
        // alongside `ValueWord`. The kinded rebuild flows
        // `vm.execute_kinded() -> Result<KindedSlot, _>` through a
        // kinded `wire_conversion::kinded_to_wire` (Phase-2c).
        Err(shape_runtime::error::ShapeError::RuntimeError {
            message: "execute_program: host-boundary completion-value path \
                      depends on the deleted ValueWord carrier + deleted \
                      `nb_to_wire` / `nb_extract_content` / `nb_to_envelope` \
                      host-API helpers; the kinded `KindedSlot -> WireValue` \
                      rebuild is Phase-2c (ADR-006 §2.7.4 / §2.7.5)."
                .to_string(),
            location: None,
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
