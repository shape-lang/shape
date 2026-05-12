use super::super::*;
// `VMError` is intentionally left out of the executor/mod.rs star-import
// (see executor/mod.rs:126 comment); name it locally for the
// `invoke_module_fn_id_stub` surface.
use shape_value::VMError;

/// Project a `TypedReturn` value into a `KindedSlot` ready for stack
/// placement.
///
/// **W17-snapshot-roundtrip (Phase 2d Wave 2.6, 2026-05-11).** Implements
/// the scalar/leaf return arms (`Concrete::*`) verbatim per ADR-006
/// §2.7.4 — each arm picks its target `NativeKind` from the
/// `ConcreteReturn` discriminator without intermediate value synthesis.
/// Container / wrapper arms (`Ok`/`Err`/`Some`/`None`/typed objects)
/// surface clean per §2.7.4 — building the typed-Arc `ResultData` /
/// `OptionData` / `TypedObjectStorage` requires the per-arm KindedSlot
/// projection path that lands in follow-up.
fn project_typed_return(
    tr: shape_runtime::typed_module_exports::TypedReturn,
) -> Result<shape_value::KindedSlot, VMError> {
    use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
    use shape_value::{KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;
    match tr {
        TypedReturn::Concrete(c) => match c {
            ConcreteReturn::I64(i) => Ok(KindedSlot::new(
                ValueSlot::from_raw(i as u64),
                NativeKind::Int64,
            )),
            ConcreteReturn::F64(f) => Ok(KindedSlot::new(
                ValueSlot::from_raw(f.to_bits()),
                NativeKind::Float64,
            )),
            ConcreteReturn::Bool(b) => Ok(KindedSlot::new(
                ValueSlot::from_raw(if b { 1 } else { 0 }),
                NativeKind::Bool,
            )),
            ConcreteReturn::Unit => Ok(KindedSlot::new(
                ValueSlot::from_raw(0),
                NativeKind::Bool,
            )),
            ConcreteReturn::String(s) => {
                Ok(KindedSlot::from_string_arc(Arc::new(s)))
            }
            ConcreteReturn::OpaqueTypedObject(hv) => {
                // hv is `Arc<HeapValue::TypedObject(Arc<TypedObjectStorage>)>`.
                // Extract the inner Arc<TypedObjectStorage> and rewrap as a
                // typed slot via `KindedSlot::from_typed_object`.
                match &*hv {
                    shape_value::heap_value::HeapValue::TypedObject(s) => Ok(
                        KindedSlot::from_typed_object(Arc::clone(s)),
                    ),
                    other => Err(VMError::RuntimeError(format!(
                        "project_typed_return: OpaqueTypedObject expected \
                         HeapValue::TypedObject payload, got {:?}",
                        other.kind()
                    ))),
                }
            }
            other => Err(VMError::NotImplemented(format!(
                "project_typed_return: W17-snapshot-roundtrip surface — \
                 ConcreteReturn::{:?} arm has no in-session KindedSlot \
                 projection. Tracked as W17-marshal-return-arms follow-up. \
                 ADR-006 §2.7.4.",
                std::mem::discriminant(&other)
            ))),
        },
        other_tr => Err(VMError::NotImplemented(format!(
            "project_typed_return: W17-snapshot-roundtrip surface — \
             TypedReturn::{:?} container arm needs the per-arm KindedSlot \
             projection path (typed-Arc ResultData/OptionData/\
             TypedObjectStorage builders). Tracked as W17-marshal-return-arms \
             follow-up. ADR-006 §2.7.4.",
            std::mem::discriminant(&other_tr)
        ))),
    }
}

impl VirtualMachine {
    /// Register a built-in stdlib module into the VM's module registry.
    /// Delegates to `register_extension` — this is a semantic alias to
    /// distinguish VM-native stdlib modules from user-installed extension plugins.
    pub fn register_stdlib_module(&mut self, module: shape_runtime::module_exports::ModuleExports) {
        self.register_extension(module);
    }

    /// Register an external/user extension module (e.g. loaded from a .so plugin)
    /// into the VM's module registry.
    /// Also merges any method intrinsics for fast Object dispatch.
    ///
    /// Phase-2c surface (ADR-006 §2.7.4 / §2.7.5): the body wraps each
    /// `TypedModuleFunction` into a `ModuleFn` whose signature is
    /// `Fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>`.
    /// `ValueWord` was deleted by the strict-typing bulldozer (no type to
    /// import); the kinded rebuild per §2.7.5 makes `ModuleFn`'s argument
    /// slice `&[KindedSlot]` and its return `Result<KindedSlot, String>`.
    /// Extensions stay on the stable raw-bits ABI and convert at the
    /// `RawCallableInvoker` boundary inside shape-runtime.
    ///
    /// The cross-crate `ModuleFn` signature change is shape-runtime
    /// territory (R-shape-runtime sub-cluster) and the corresponding
    /// `TypedReturn::into_value_word()` helper is also deleted; this
    /// caller hand-off lands in the Phase-2c rebuild session.
    pub fn register_extension(&mut self, module: shape_runtime::module_exports::ModuleExports) {
        // Merge method intrinsics — these don't carry ValueWord shapes
        // through the registration path and are safe to keep here.
        for (type_name, methods) in &module.method_intrinsics {
            let entry = self.extension_methods.entry(type_name.clone()).or_default();
            for (method_name, func) in methods {
                entry.insert(method_name.clone(), func.clone());
            }
        }
        // The `module.typed_exports()` rewrap into `ModuleFn` (which
        // marshals `TypedReturn -> ValueWord` at the boundary) is the
        // Phase-2c host-API rebuild (ADR-006 §2.7.4 / §2.7.5):
        // `ModuleFn` becomes `Fn(&[KindedSlot], _) -> Result<KindedSlot, _>`
        // and the marshal step disappears (the typed body's
        // `TypedReturn` is converted directly to `KindedSlot` inside
        // shape-runtime).
        self.module_registry.register(module);
    }

    /// Register a module-function entry in the table and return its ID.
    ///
    /// Phase-2c surface (ADR-006 §2.7.4 / §2.7.5): the
    /// `ValueWord::ModuleFunction` carrier shape this function feeds
    /// depends on the deleted `ValueWord` runtime representation.
    /// Replaced with a kinded `NativeKind::ModuleFunction`-style ID
    /// carrier in the Phase-2c rebuild.
    pub fn register_module_fn_entry(
        &mut self,
        entry: shape_runtime::module_exports::ModuleFnEntry,
    ) -> usize {
        let id = self.module_fn_table.len();
        self.module_fn_table.push(entry);
        id
    }

    /// Invoke a module-function entry by ID.
    ///
    /// **W17-snapshot-roundtrip close (Phase 2d Wave 2.6, 2026-05-11).**
    /// Lands the kinded shape per ADR-006 §2.7.4 / §2.7.5: takes
    /// `&[KindedSlot]` and returns `Result<KindedSlot, VMError>`,
    /// dispatching through the existing [`module_fn_table`] entry
    /// (sum-typed `Typed` / `TypedAsync` per Phase 4c.3). The async
    /// arm runs the future to completion on the ambient tokio
    /// runtime; the sync arm calls the body directly with the slice's
    /// raw `u64` bits and the registered `arg_kinds` table on the
    /// receiver (the body is contract-bound to interpret each slot
    /// per its `arg_kinds[i]`).
    ///
    /// Per `module_exports::ModuleContext`, the body receives a borrow
    /// of the VM's type schema registry plus the optional invoker
    /// hooks needed for callbacks back into the VM. The body is
    /// `Send + Sync` so it can be invoked from worker tasks.
    ///
    /// Returns:
    /// - `Ok(KindedSlot)` — successful invocation; the slot carries
    ///   the projected `TypedReturn` value with the registered return
    ///   type's `NativeKind`.
    /// - `Err(VMError::InvalidCall)` — `fn_id` out of range for the
    ///   current `module_fn_table`.
    /// - `Err(VMError::RuntimeError(msg))` — body returned an error
    ///   string; the message propagates verbatim.
    /// - `Err(VMError::NotImplemented(msg))` — async body called with
    ///   no ambient tokio runtime, or `TypedReturn::*` arm that needs
    ///   the kind-threaded slot projection follow-up. Surface-and-stop
    ///   per ADR-006 §2.7.4 — no Bool-default fallback.
    pub(crate) fn invoke_module_fn_id_stub(
        &mut self,
        fn_id: usize,
        args: &[shape_value::KindedSlot],
    ) -> Result<shape_value::KindedSlot, VMError> {
        let entry = self
            .module_fn_table
            .get(fn_id)
            .ok_or(VMError::InvalidCall)?
            .clone();

        // **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).**
        // Build a `ModuleContext` borrow against the live schema
        // registry and capture a read-only `VmStateSnapshot` so state.*
        // bodies can introspect the VM via `ctx.vm_state` (per
        // ADR-006 §2.7.4 — state.* reads dispatched through the
        // VmStateAccessor trait). The snapshot owns its own KindedSlot
        // shares so the live VM is undisturbed.
        let vm_state_snap = self.capture_vm_state();
        let schema_registry: &shape_runtime::type_schema::TypeSchemaRegistry =
            &self.program.type_schema_registry;
        // SAFETY: extend the borrow lifetime to 'ctx via transmute is
        // not needed here because `ModuleContext` is invariant on its
        // lifetime parameter and the body call below holds the borrow
        // for the duration of the dispatch.
        let ctx = shape_runtime::module_exports::ModuleContext {
            schemas: schema_registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: Some(&vm_state_snap),
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        };

        match entry {
            shape_runtime::module_exports::ModuleFnEntry::Typed(typed) => {
                // The body takes `&[u64]` slot bits (per its kind table)
                // and returns `Result<TypedReturn, String>`. Translate
                // `&[KindedSlot]` to `Vec<u64>` at the boundary.
                let raw_bits: Vec<u64> = args.iter().map(|s| s.slot().raw()).collect();
                let typed_return = (typed.invoke)(&raw_bits, &ctx)
                    .map_err(VMError::RuntimeError)?;
                project_typed_return(typed_return)
            }
            shape_runtime::module_exports::ModuleFnEntry::TypedAsync(async_entry) => {
                let raw_bits: Vec<u64> = args.iter().map(|s| s.slot().raw()).collect();
                let fut = (async_entry.invoke)(raw_bits);
                // Drive the future on the ambient tokio runtime. If no
                // runtime is available we surface — async dispatch
                // requires an explicit host runtime per the §2.7.4 task-
                // scheduler boundary.
                let typed_return = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => tokio::task::block_in_place(|| {
                        handle.block_on(fut).map_err(VMError::RuntimeError)
                    })?,
                    Err(_) => {
                        return Err(VMError::NotImplemented(
                            "invoke_module_fn_id: async dispatch requires an \
                             ambient tokio runtime — wrap the call in \
                             tokio::runtime::Builder::new_current_thread().build() \
                             or use a worker thread. ADR-006 §2.7.4 \
                             task-scheduler boundary."
                                .to_string(),
                        ));
                    }
                };
                project_typed_return(typed_return)
            }
        }
    }

    /// Populate extension module objects as module_bindings — Phase-2c stub.
    ///
    /// The legacy body assembled a `TypedObject` module binding by
    /// inserting `ValueWord::from_module_function` / `ValueWord::from_function`
    /// into a `HashMap<String, ValueWord>` and writing it through the
    /// deleted module-binding raw-write shim. Both halves depend on the
    /// deleted `ValueWord` runtime representation. The kinded equivalent
    /// (Arc<TypedObjectStorage> + parallel-kind track + the §2.7.8 / Q10
    /// module-binding cell-storage rebuild) lands with the Phase-2c
    /// host-API revival per ADR-006 §2.7.4.
    pub fn populate_module_objects(&mut self) {
        // No-op: module-binding object population is deferred to the
        // Phase-2c rebuild. Tests / callers that depend on the populated
        // object hit the surface inside their downstream call sites,
        // not here, so the VM init path stays compile-clean. Surfacing
        // at the call-site rather than here also keeps the read-side
        // legacy ValueWord reach contained.
        let _ = &self.module_registry;
    }
}
