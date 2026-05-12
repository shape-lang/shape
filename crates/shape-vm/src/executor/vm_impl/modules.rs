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

    /// Populate extension module objects as module_bindings — W17-comptime-vm-dispatch rebuild.
    ///
    /// **W17-comptime-vm-dispatch (Phase 2d Wave 3, 2026-05-12).**
    /// Per ADR-006 §2.7.26 amendment. Builds a kinded `TypedObject`
    /// per registered extension module, with field slots that store
    /// **module-function-id field references** as `Ptr(HeapKind::ModuleFn)`
    /// inline-scalar payloads. The dispatch chain
    /// `LoadModuleBinding(idx) + GetFieldTyped(...) + CallValue` routes
    /// through:
    ///
    /// 1. `LoadModuleBinding(idx)` reads the kinded module-binding
    ///    slot (TypedObject + `Ptr(HeapKind::TypedObject)` kind) and
    ///    pushes it via `clone_with_kind` retain-on-read (§2.7.7).
    /// 2. `GetFieldTyped { type_id, field_idx, field_type_tag }` pops
    ///    the receiver, recovers the `Arc<TypedObjectStorage>` per
    ///    ADR-005 §1, and reads the field. The compiler emits
    ///    `field_type_tag = FIELD_TAG_ANY` for schema fields of
    ///    `FieldType::Any` (the comptime predeclared schema shape).
    ///    The `op_get_field_typed` body falls through to
    ///    `push_field_value_with_kind`, which sources the kind from
    ///    `storage.field_kinds[field_idx]` (the §2.7.7 parallel-kind
    ///    track) — resolving hardening item (f) — and pushes the
    ///    `module_fn_id as u64` bits with kind
    ///    `Ptr(HeapKind::ModuleFn)`.
    /// 3. `CallValue` pops args + callee, dispatches via
    ///    `call_value_immediate_nb` whose `Ptr(HeapKind::ModuleFn)`
    ///    arm routes to `invoke_module_fn_id_stub(bits as usize, args)` —
    ///    the same path used by `W17-snapshot-roundtrip` for direct
    ///    module-fn invocation.
    ///
    /// Per-module construction:
    ///
    /// - Look up the predeclared `__mod_<name>` schema (registered by
    ///   `compiler/comptime.rs::ensure_module_object_schema` before
    ///   bytecode compilation). The schema field names define the
    ///   storage's field order; missing schemas are skipped (the
    ///   module's exports remain unreachable through this binding).
    /// - For each typed export (sync and async), register a
    ///   `ModuleFnEntry::Typed` / `TypedAsync` into `module_fn_table`
    ///   to obtain a `module_fn_id`.
    /// - For each schema field, look up the matching `module_fn_id`,
    ///   write a `ValueSlot::from_raw(module_fn_id as u64)` with
    ///   `field_kinds[i] = Ptr(HeapKind::ModuleFn)` and `heap_mask`
    ///   bit set. Unmatched fields (a schema field with no
    ///   corresponding export) get the `(0u64, NativeKind::Bool)`
    ///   sentinel pair — same shape as the
    ///   `module_binding_pad_to_kinded` uninitialised-slot convention.
    /// - Construct `Arc<TypedObjectStorage>` via the typed constructor
    ///   per ADR-006 §2.4 and write the `Ptr(HeapKind::TypedObject)`
    ///   slot to the module-binding via
    ///   `module_binding_write_kinded` (§2.7.8 / Q10 lockstep).
    ///
    /// Resolves the upstream `populate_module_objects` no-op blocker
    /// flagged by `W17-snapshot-roundtrip` (commit `fbfbfb6`). The
    /// 4 comptime introspection forms wired by C2-comptime-rebuild
    /// (`a5df165`) — `build_config` / `implements` / `warning` /
    /// `error` — now dispatch end-to-end via VM mode.
    pub fn populate_module_objects(&mut self) {
        use shape_runtime::module_exports::ModuleFnEntry;
        use shape_value::heap_value::TypedObjectStorage;
        use shape_value::{HeapKind, NativeKind, ValueSlot};
        use std::sync::Arc;

        // Phase 1 — collect: gather per-module data without taking a
        // mutable borrow on `self` while iterating the registry. The
        // `register_module_fn_entry` call mutates `self.module_fn_table`,
        // which conflicts with an active borrow on `self.module_registry`.
        let module_names: Vec<String> = self
            .module_registry
            .module_names()
            .iter()
            .map(|s| s.to_string())
            .collect();

        for module_name in module_names {
            // Resolve the module's typed exports. The `module_registry.get`
            // borrow is local to this iteration and dropped before we
            // mutate `module_fn_table` below.
            let typed_entries: Vec<(String, ModuleFnEntry)> = {
                let module = match self.module_registry.get(&module_name) {
                    Some(m) => m,
                    None => continue,
                };
                let typed = module.typed_exports();
                let mut entries: Vec<(String, ModuleFnEntry)> =
                    Vec::with_capacity(typed.functions.len() + typed.async_functions.len());
                for (export_name, typed_fn) in &typed.functions {
                    entries.push((
                        export_name.clone(),
                        ModuleFnEntry::Typed(typed_fn.clone()),
                    ));
                }
                for (export_name, typed_async) in &typed.async_functions {
                    entries.push((
                        export_name.clone(),
                        ModuleFnEntry::TypedAsync(typed_async.clone()),
                    ));
                }
                entries
            };

            // Locate the binding index for this module — prefer the
            // hidden native binding (`__imported_module__::<name>`,
            // injected by the compiler's
            // `ensure_hidden_native_module_binding`), fall back to the
            // plain binding name. The hidden form is used when a Shape
            // artifact module with the same name would otherwise
            // shadow the native object.
            let hidden_name = format!("__imported_module__::{}", module_name);
            let binding_idx = self
                .program
                .module_binding_names
                .iter()
                .position(|n| n == &hidden_name)
                .or_else(|| {
                    self.program
                        .module_binding_names
                        .iter()
                        .position(|n| n == &module_name)
                });
            let binding_idx = match binding_idx {
                Some(i) => i,
                None => continue, // No binding name — nothing to populate.
            };

            // Resolve the predeclared module-object schema. The schema
            // is registered before compilation by
            // `compiler/comptime.rs::ensure_module_object_schema`
            // (under canonical name `__mod_<module_name>`).
            // Without it we can't define a stable field order for
            // the typed-object layout, so skip — the binding stays at
            // the no-op-on-drop sentinel and any reference to a field
            // through this binding will surface clean at GetFieldTyped.
            let schema_name = format!("__mod_{}", module_name);
            let schema = match self.lookup_schema_by_name(&schema_name) {
                Some(s) => s.clone(),
                None => continue,
            };

            // Register each typed entry into `module_fn_table` and
            // build a name → module_fn_id lookup.
            let mut fn_id_by_name: std::collections::HashMap<String, u64> =
                std::collections::HashMap::with_capacity(typed_entries.len());
            for (export_name, entry) in typed_entries {
                let fn_id = self.register_module_fn_entry(entry);
                fn_id_by_name.insert(export_name, fn_id as u64);
            }

            // Build the typed-object slot list in schema field order.
            // Each field maps to either:
            //   - a known module-fn-id → ValueSlot(fn_id) with kind
            //     Ptr(HeapKind::ModuleFn) and heap_mask bit set, or
            //   - no matching export → (0, NativeKind::Bool) sentinel
            //     (same shape as the module-binding-pad uninitialised
            //     slot convention — no Bool-default fallback for a
            //     known-callable-but-missing field; the compiler
            //     should have surfaced 'module has no export' earlier).
            let field_count = schema.fields.len();
            let mut slots: Vec<ValueSlot> = Vec::with_capacity(field_count);
            let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(field_count);
            let mut heap_mask: u64 = 0;
            for (i, field) in schema.fields.iter().enumerate() {
                match fn_id_by_name.get(&field.name) {
                    Some(&fn_id) => {
                        // ModuleFn inline-scalar slot: bits = fn_id,
                        // kind = Ptr(HeapKind::ModuleFn). Mark the
                        // heap_mask bit so the read path sees the slot
                        // as "kind-bearing" and dispatches through the
                        // FIELD_TAG_ANY / field_kinds resolver in
                        // op_get_field_typed.
                        slots.push(ValueSlot::from_raw(fn_id));
                        field_kinds.push(NativeKind::Ptr(HeapKind::ModuleFn));
                        heap_mask |= 1u64 << i;
                    }
                    None => {
                        // Schema field present but no typed export:
                        // sentinel slot. `clone_with_kind` /
                        // `drop_with_kind` are no-op on (0, Bool).
                        slots.push(ValueSlot::from_raw(0));
                        field_kinds.push(NativeKind::Bool);
                    }
                }
            }

            let storage = Arc::new(TypedObjectStorage::new(
                schema.id as u64,
                slots.into_boxed_slice(),
                heap_mask,
                Arc::from(field_kinds.into_boxed_slice()),
            ));

            // Hand off one share to the binding slot. `Arc::into_raw`
            // converts the Arc<TypedObjectStorage> to its raw pointer
            // bits; `module_binding_write_kinded` retires the prior
            // occupant (sentinel pair → no-op) and transfers our
            // share into the binding.
            let bits = Arc::into_raw(storage) as u64;
            self.module_binding_write_kinded(
                binding_idx,
                bits,
                NativeKind::Ptr(HeapKind::TypedObject),
            );
        }
    }
}
