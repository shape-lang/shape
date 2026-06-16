use super::super::*;
// `VMError` is intentionally left out of the executor/mod.rs star-import
// (see executor/mod.rs:126 comment); name it locally for the
// `invoke_module_fn_id_stub` surface.
use shape_value::VMError;

/// Project a single `ConcreteReturn` leaf into a `KindedSlot`.
///
/// **STAGE K1 (2026-06-02).** The leaf projector shared by both
/// `project_typed_return`'s `Concrete` arm and the wrapper arms
/// (`Ok`/`Err`/`Some`) — those build their payload through this function
/// and then wrap the resulting `KindedSlot` in `ResultData` / `OptionData`.
/// Every arm builds the typed-Arc carrier directly per ADR-005 §1 /
/// ADR-006 §2.7 — no `Box<HeapValue>` wrapping, no value synthesis, no
/// `ValueWord`. Arms that genuinely cannot project typed-Arc-direct at
/// this boundary (`JsonValue` needs the runtime `Json` enum-construction
/// subsystem; `HashMapStringHeapValue` needs the K3 `HashMapData`
/// kinded-value-track amendment) surface clean per §2.7.4.
fn project_concrete_return(
    c: shape_runtime::typed_module_exports::ConcreteReturn,
) -> Result<shape_value::KindedSlot, VMError> {
    use shape_runtime::typed_module_exports::ConcreteReturn;
    use shape_value::heap_value::{HashMapData, HashMapKindedRef};
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{
        ELEM_TYPE_F64, ELEM_TYPE_I64, ELEM_TYPE_STRING, TypedArray, stamp_elem_type,
    };
    use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;
    match c {
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
        ConcreteReturn::Unit => Ok(KindedSlot::new(ValueSlot::from_raw(0), NativeKind::Bool)),
        ConcreteReturn::String(s) => Ok(KindedSlot::from_string_arc(Arc::new(s))),
        ConcreteReturn::OpaqueTypedObject(hv) => {
            // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): hv is
            // `Arc<HeapValue::TypedObject(TypedObjectPtr)>`. Clone the
            // wrapper (bumps v2-raw refcount); into_raw moves the share
            // to the slot via `from_typed_object_raw`.
            match &*hv {
                shape_value::heap_value::HeapValue::TypedObject(s) => {
                    Ok(KindedSlot::from_typed_object_raw(s.clone().into_raw()))
                }
                other => Err(VMError::RuntimeError(format!(
                    "project_concrete_return: OpaqueTypedObject expected \
                     HeapValue::TypedObject payload, got {:?}",
                    other.kind()
                ))),
            }
        }
        // R8 W6 G.1 W17-marshal-return-arms close (2026-05-24): explicit
        // arms for ConcreteReturn::IoHandle (disc 16) +
        // ConcreteReturn::DataTable (disc 15). Mirrors the 14 existing
        // typed-Arc constructor precedents in `kinded_slot.rs` per
        // ADR-005 §1 single-discriminator + ADR-006 §2.7.6 / Q8
        // bounded carrier-API.
        ConcreteReturn::IoHandle(h) => Ok(KindedSlot::from_io_handle(h)),
        ConcreteReturn::DataTable(d) => Ok(KindedSlot::from_data_table(d)),

        // ── STAGE K1 (2026-06-02): typed-array leaves ──────────────────
        //
        // `Array<int>` / `Array<number>` carriers are the monomorphic
        // flat-struct `*mut TypedArray<T>` per `docs/runtime-v2-spec.md`;
        // slot bits are the raw pointer, kind `Ptr(HeapKind::TypedArray)`,
        // and the element-type discriminant is stamped at HeapHeader
        // offset 7 so the release path (`release_v2_typed_array`) picks
        // the matching `drop_array::<T>` monomorphization. `Bytes`
        // surfaces to user code as `Array<int>`, so each byte widens to
        // i64 into a `TypedArray<i64>` (ELEM_TYPE_I64). Empty arrays still
        // allocate a stamped zero-length carrier (the round-trip reader
        // maps a null pointer → empty; an owned non-null carrier is the
        // canonical shape for a value pushed onto the stack).
        ConcreteReturn::ArrayI64(v) => {
            let arr = TypedArray::<i64>::from_slice(&v);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayF64(v) => {
            let arr = TypedArray::<f64>::from_slice(&v);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::Bytes(bytes) => {
            let widened: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
            let arr = TypedArray::<i64>::from_slice(&widened);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayString(v) => {
            // Each element is a fresh `StringObj` (refcount = 1); the
            // array owns each allocation outright. Mirror of the marshal
            // `ToSlot<Vec<Arc<String>>>` producer.
            let arr = TypedArray::<*const StringObj>::with_capacity(v.len() as u32);
            unsafe {
                stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
                for s in &v {
                    let p = StringObj::new(s.as_str()) as *const StringObj;
                    TypedArray::<*const StringObj>::push(arr, p);
                }
            }
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayHeapValue(elems) => {
            // STAGE K2 dispatcher: the per-element-T marshal producer
            // (`ToSlot<Vec<Arc<HeapValue>>>` in shape-runtime/marshal.rs)
            // inspects the first element's HeapValue variant, allocates
            // the matching `TypedArray<T>`, stamps the element
            // discriminant, and takes its own share per element. The
            // returned bits are the raw `*mut TypedArray<T>` carrier; the
            // kind is `Ptr(HeapKind::TypedArray)` (the K2 producer's
            // `NATIVE_KIND`). Empty Vec → null carrier (bits = 0).
            use shape_runtime::marshal::ToSlot;
            let bits = elems.to_slot();
            Ok(KindedSlot::new(
                ValueSlot::from_raw(bits),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }

        // ── STAGE K1 (2026-06-02): string→string HashMap leaf ──────────
        //
        // `HashMap<string, string>` builds a `HashMapData<*const
        // StringObj>` (string-value monomorphization), wrapped in the
        // per-V carrier enum `HashMapKindedRef::String`. Each value is a
        // fresh `StringObj` (refcount = 1) whose owned share transfers
        // into the map via `HashMapData::insert`; keys are allocated as
        // fresh `StringObj`s inside `insert`. The `from_hashmap`
        // constructor moves one outer `Arc<HashMapKindedRef>` share into
        // the slot with kind `Ptr(HeapKind::HashMap)`. No heap-value
        // track is needed — the value monomorphization is string, not
        // the polymorphic K3 `Arc<HeapValue>` shape.
        ConcreteReturn::HashMapStringString(pairs) => {
            let mut data: HashMapData<*const StringObj> = HashMapData::new();
            for (k, v) in &pairs {
                let value_ptr = StringObj::new(v.as_str()) as *const StringObj;
                // SAFETY: `value_ptr` is a freshly-allocated StringObj
                // (refcount = 1); `insert` takes ownership of that single
                // share per the HashMapData::insert contract.
                unsafe {
                    data.insert(k.as_str(), value_ptr);
                }
            }
            let kref = Arc::new(HashMapKindedRef::String(Arc::new(data)));
            Ok(KindedSlot::from_hashmap(kref))
        }

        // ── Genuinely-cannot-go-typed-Arc-direct: SURFACE, do not shim ──
        //
        // `JsonValue` projects to the runtime `Json` ADT
        // (`stdlib-src/core/json_value.shape`), whose Array/Object
        // variants are enum-tagged TypedObjects requiring the VM's
        // schema-registry-backed enum-construction machinery + recursive
        // descent — neither is reachable from a typed-Arc builder at this
        // boundary. `HashMapStringHeapValue` is K3 territory: the
        // polymorphic-value HashMap needs the ADR-006 `HashMapData`
        // kinded-value-track amendment (a parallel `Vec<NativeKind>` over
        // the values) before it can carry `Arc<HeapValue>` payloads
        // without a Bool-default kind. Both surface clean rather than
        // shim — a fabricated carrier here is exactly the deleted-pattern
        // class CLAUDE.md §Forbidden refuses.
        other => Err(VMError::NotImplemented(format!(
            "project_concrete_return: ConcreteReturn::{:?} has no \
             typed-Arc-direct projection at the module-return boundary. \
             JsonValue needs the runtime Json enum-construction subsystem; \
             HashMapStringHeapValue is K3 (pending the ADR-006 HashMapData \
             kinded-value-track amendment). SURFACED per ADR-006 §2.7.4 — \
             no shim. ",
            std::mem::discriminant(&other)
        ))),
    }
}

/// Project a `TypedReturn` value into a `KindedSlot` ready for stack
/// placement.
///
/// **STAGE K1 (2026-06-02).** Lands the container / wrapper arms over the
/// scalar/leaf base established by the W17-snapshot-roundtrip work
/// (Phase 2d Wave 2.6, 2026-05-11). Wrapper arms build their inner
/// payload through [`project_concrete_return`] and wrap the resulting
/// `KindedSlot` in the typed-Arc `ResultData` / `OptionData` carriers
/// (kinds `Ptr(HeapKind::Result)` / `Ptr(HeapKind::Option)`); typed-object
/// arms build a `TypedObjectStorage` via
/// [`shape_runtime::type_schema::typed_object_from_pairs`]. Each arm picks
/// its `NativeKind` from the discriminator without value synthesis per
/// ADR-006 §2.7.4. `ArrayObjectPairs` (array of typed objects) needs the
/// typed-object-array element-construction path that pairs with the K2
/// array-of-heap-value producer and surfaces clean pending that landing.
fn project_typed_return(
    tr: shape_runtime::typed_module_exports::TypedReturn,
) -> Result<shape_value::KindedSlot, VMError> {
    use shape_runtime::type_schema::typed_object_from_pairs;
    use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
    use shape_value::KindedSlot;
    use shape_value::heap_value::{OptionData, ResultData};
    use std::sync::Arc;

    // Build a `TypedObjectStorage`-backed KindedSlot from string→leaf
    // pairs. Each leaf projects through `project_concrete_return`; the
    // share is moved into `typed_object_from_pairs`' slot list (which
    // clones-then-forgets per its construction contract).
    fn typed_object_from_concrete_pairs(
        pairs: Vec<(String, ConcreteReturn)>,
    ) -> Result<KindedSlot, VMError> {
        // Project each leaf, holding the owned (name, KindedSlot) pairs so
        // the `&str` borrows the builder needs live across the call.
        let mut owned: Vec<(String, KindedSlot)> = Vec::with_capacity(pairs.len());
        for (name, c) in pairs {
            owned.push((name, project_concrete_return(c)?));
        }
        // `typed_object_from_pairs` borrows each `KindedSlot`, clones it
        // (one refcount bump moved into the slot list), then forgets the
        // clone. Our `owned` originals Drop normally at scope exit, each
        // releasing its single share — net one share owned by the new
        // typed object's slot list.
        let borrowed: Vec<(&str, KindedSlot)> = owned
            .iter()
            .map(|(name, slot)| (name.as_str(), slot.clone()))
            .collect();
        let out = typed_object_from_pairs(&borrowed);
        // `borrowed`'s clones Drop here (−1 each); `owned`'s originals Drop
        // at function return (−1 each). The builder's internal forget keeps
        // exactly one share per field inside the typed object.
        Ok(out)
    }

    match tr {
        TypedReturn::Concrete(c) => project_concrete_return(c),

        // ── Result / Option wrappers (ADR-006 §2.7.17 / Q18) ───────────
        TypedReturn::Ok(c) => {
            let payload = project_concrete_return(c)?;
            let res = Arc::new(ResultData::ok(payload));
            Ok(KindedSlot::from_result(res))
        }
        TypedReturn::Err(c) => {
            let payload = project_concrete_return(c)?;
            let res = Arc::new(ResultData::err(payload));
            Ok(KindedSlot::from_result(res))
        }
        TypedReturn::Some(c) => {
            let payload = project_concrete_return(c)?;
            let opt = Arc::new(OptionData::some(payload));
            Ok(KindedSlot::from_option(opt))
        }
        TypedReturn::None => {
            let opt = Arc::new(OptionData::none());
            Ok(KindedSlot::from_option(opt))
        }

        // ── Typed-object wrappers (TypedObjectStorage builder) ─────────
        TypedReturn::ObjectPairs(pairs) | TypedReturn::TypedObject(pairs) => {
            typed_object_from_concrete_pairs(pairs)
        }
        TypedReturn::SomeObjectPairs(pairs) => {
            let payload = typed_object_from_concrete_pairs(pairs)?;
            let opt = Arc::new(OptionData::some(payload));
            Ok(KindedSlot::from_option(opt))
        }
        TypedReturn::OkObjectPairs(pairs) => {
            let payload = typed_object_from_concrete_pairs(pairs)?;
            let res = Arc::new(ResultData::ok(payload));
            Ok(KindedSlot::from_result(res))
        }
        TypedReturn::ErrObjectPairs(pairs) => {
            let payload = typed_object_from_concrete_pairs(pairs)?;
            let res = Arc::new(ResultData::err(payload));
            Ok(KindedSlot::from_result(res))
        }

        // ── ArrayObjectPairs: SURFACE pending typed-object-array prod ──
        //
        // An array whose elements are themselves typed objects needs a
        // `TypedArray<*const TypedObjectStorage>` built from per-row
        // `TypedObjectStorage` allocations — the array-element-construction
        // path that pairs with the K2 `Vec<Arc<HeapValue>>` producer but
        // is not in K1 scope. Surface clean rather than shim.
        other_tr @ TypedReturn::ArrayObjectPairs(_) => Err(VMError::NotImplemented(format!(
            "project_typed_return: TypedReturn::{:?} (array of typed \
                 objects) needs the typed-object-array element-construction \
                 path. SURFACED per ADR-006 §2.7.4 — no shim.",
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
                let typed_return =
                    (typed.invoke)(&raw_bits, &ctx).map_err(VMError::RuntimeError)?;
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
                    entries.push((export_name.clone(), ModuleFnEntry::Typed(typed_fn.clone())));
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

            // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new` per D1
            // API surface. The raw pointer is directly the carrier bits;
            // `module_binding_write_kinded` consumes a u64 + NativeKind
            // pair so this site cleanly migrates without depending on
            // the `HeapValue::TypedObject` variant signature flip.
            let ptr = TypedObjectStorage::_new(
                schema.id as u64,
                slots.into_boxed_slice(),
                heap_mask,
                Arc::from(field_kinds.into_boxed_slice()),
            );

            // Hand off one share to the binding slot. The v2-raw pointer
            // bits are the carrier directly (per ADR-006 §2.4 / D1's
            // `from_typed_object_raw` constructor contract). One strong
            // count owned by us is transferred into the binding via
            // `module_binding_write_kinded`.
            let bits = ptr as u64;
            self.module_binding_write_kinded(
                binding_idx,
                bits,
                NativeKind::Ptr(HeapKind::TypedObject),
            );
        }
    }
}

#[cfg(test)]
mod stage_k1_tests {
    //! STAGE K1 (2026-06-02) round-trip verification for the
    //! `project_typed_return` container/wrapper arms + the
    //! `project_concrete_return` leaf arms. Each test drives a representative
    //! value through a **real module-fn return**: it registers a
    //! `ModuleFnEntry::Typed` whose body returns the `TypedReturn` under test,
    //! invokes it through [`VirtualMachine::invoke_module_fn_id_stub`] (the
    //! same dispatch path `LoadModuleBinding + GetFieldTyped + CallValue`
    //! routes through), and recovers the projected `KindedSlot` to assert the
    //! value survived the projection.
    //!
    //! Recovery reads the typed-Arc / v2-raw carriers directly (ADR-005 §1):
    //! `Arc::from_raw` + `into_raw` for `ResultData` / `OptionData` (restores
    //! the share), and a borrow through the raw `*const TypedArray<T>` /
    //! `*const TypedObjectStorage` / `*const HashMapKindedRef` for the heap
    //! carriers. The recovered `KindedSlot`'s own `Drop` retires the carrier.

    use super::*;
    use shape_runtime::typed_module_exports::{
        ConcreteReturn, ConcreteType, TypedModuleFunction, TypedReturn,
    };
    use shape_value::heap_value::{HashMapKindedRef, OptionData, ResultData, TypedObjectStorage};
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::TypedArray;
    use shape_value::{HeapKind, KindedSlot, NativeKind};
    use std::sync::Arc;

    /// Register a typed module-fn whose 0-arg body returns `tr`, then invoke
    /// it through the real `invoke_module_fn_id_stub` dispatch path. Returns
    /// the projected `KindedSlot`.
    fn roundtrip(tr: TypedReturn) -> KindedSlot {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let tr_cell = std::sync::Mutex::new(Some(tr));
        let tmf = TypedModuleFunction {
            invoke: Arc::new(move |_slots, _ctx| {
                Ok(tr_cell
                    .lock()
                    .unwrap()
                    .take()
                    .expect("module-fn body invoked once"))
            }),
            return_type: ConcreteType::Any,
            arg_types: vec![],
            arg_kinds: vec![],
        };
        let entry = shape_runtime::module_exports::ModuleFnEntry::Typed(tmf);
        let fn_id = vm.register_module_fn_entry(entry);
        vm.invoke_module_fn_id_stub(fn_id, &[])
            .expect("module-fn invocation projects cleanly")
    }

    #[test]
    fn array_i64_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayI64(vec![
            1, 2, 3, -7,
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<i64>;
        let got = unsafe { TypedArray::<i64>::as_slice(arr) };
        assert_eq!(got, &[1, 2, 3, -7]);
    }

    #[test]
    fn array_f64_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![
            1.5, 2.5, -0.25,
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<f64>;
        let got = unsafe { TypedArray::<f64>::as_slice(arr) };
        assert_eq!(got, &[1.5, 2.5, -0.25]);
    }

    #[test]
    fn bytes_roundtrips_as_array_int() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::Bytes(vec![
            0u8, 127, 255,
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        // Bytes surface as Array<int>: each byte widened to i64.
        let arr = slot.raw() as *const TypedArray<i64>;
        let got = unsafe { TypedArray::<i64>::as_slice(arr) };
        assert_eq!(got, &[0i64, 127, 255]);
    }

    #[test]
    fn array_string_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayString(vec![
            "alpha".to_string(),
            "beta".to_string(),
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<*const StringObj>;
        let ptrs = unsafe { TypedArray::<*const StringObj>::as_slice(arr) };
        let got: Vec<&str> = ptrs
            .iter()
            .map(|&p| unsafe { StringObj::as_str(p) })
            .collect();
        assert_eq!(got, vec!["alpha", "beta"]);
    }

    #[test]
    fn array_heap_value_typed_object_roundtrips_via_k2() {
        // Build a Vec<Arc<HeapValue::TypedObject>> via the same typed-object
        // builder the wrapper arms use, then project through the K2
        // dispatcher's ArrayHeapValue arm.
        let obj = super::project_concrete_return_for_test_typed_object();
        let elems = vec![obj];
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(elems)));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<*const TypedObjectStorage>;
        let n = unsafe { TypedArray::<*const TypedObjectStorage>::len(arr) };
        assert_eq!(n, 1);
    }

    #[test]
    fn hashmap_string_string_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::HashMapStringString(
            vec![
                ("k1".to_string(), "v1".to_string()),
                ("k2".to_string(), "v2".to_string()),
            ],
        )));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::HashMap));
        let kref = slot.raw() as *const HashMapKindedRef;
        match unsafe { &*kref } {
            HashMapKindedRef::String(data) => {
                assert_eq!(data.len(), 2);
                // keys + values are parallel string arrays.
                let keys = unsafe { TypedArray::<*const StringObj>::as_slice(data.keys) };
                let vals = unsafe { TypedArray::<*const StringObj>::as_slice(data.values) };
                let pairs: Vec<(&str, &str)> = keys
                    .iter()
                    .zip(vals.iter())
                    .map(|(&k, &v)| unsafe { (StringObj::as_str(k), StringObj::as_str(v)) })
                    .collect();
                assert!(pairs.contains(&("k1", "v1")));
                assert!(pairs.contains(&("k2", "v2")));
            }
            other => panic!(
                "expected HashMapKindedRef::String, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn ok_wrapper_roundtrips() {
        let slot = roundtrip(TypedReturn::Ok(ConcreteReturn::I64(99)));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Result));
        unsafe {
            let arc = Arc::<ResultData>::from_raw(slot.raw() as *const ResultData);
            assert!(arc.is_ok);
            assert_eq!(arc.payload.kind(), NativeKind::Int64);
            assert_eq!(arc.payload.slot().raw() as i64, 99);
            let _ = Arc::into_raw(arc);
        }
    }

    #[test]
    fn err_wrapper_roundtrips() {
        let slot = roundtrip(TypedReturn::Err(ConcreteReturn::String("boom".to_string())));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Result));
        unsafe {
            let arc = Arc::<ResultData>::from_raw(slot.raw() as *const ResultData);
            assert!(!arc.is_ok);
            assert_eq!(arc.payload.kind(), NativeKind::String);
            let s = arc.payload.slot().raw() as *const String;
            assert_eq!(&**(&*s), "boom");
            let _ = Arc::into_raw(arc);
        }
    }

    #[test]
    fn some_wrapper_roundtrips() {
        let slot = roundtrip(TypedReturn::Some(ConcreteReturn::Bool(true)));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Option));
        unsafe {
            let arc = Arc::<OptionData>::from_raw(slot.raw() as *const OptionData);
            assert!(arc.is_some);
            assert_eq!(arc.payload.kind(), NativeKind::Bool);
            assert_eq!(arc.payload.slot().raw(), 1);
            let _ = Arc::into_raw(arc);
        }
    }

    #[test]
    fn none_wrapper_roundtrips() {
        let slot = roundtrip(TypedReturn::None);
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Option));
        unsafe {
            let arc = Arc::<OptionData>::from_raw(slot.raw() as *const OptionData);
            assert!(!arc.is_some);
            let _ = Arc::into_raw(arc);
        }
    }

    #[test]
    fn object_pairs_roundtrips_as_typed_object() {
        let slot = roundtrip(TypedReturn::ObjectPairs(vec![
            ("count".to_string(), ConcreteReturn::I64(5)),
            ("ratio".to_string(), ConcreteReturn::F64(0.5)),
        ]));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let obj = slot.raw() as *const TypedObjectStorage;
        let storage = unsafe { &*obj };
        assert_eq!(storage.slots.len(), 2);
    }

    #[test]
    fn ok_object_pairs_roundtrips() {
        let slot = roundtrip(TypedReturn::OkObjectPairs(vec![(
            "x".to_string(),
            ConcreteReturn::I64(1),
        )]));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Result));
        unsafe {
            let arc = Arc::<ResultData>::from_raw(slot.raw() as *const ResultData);
            assert!(arc.is_ok);
            assert_eq!(arc.payload.kind(), NativeKind::Ptr(HeapKind::TypedObject));
            let _ = Arc::into_raw(arc);
        }
    }

    #[test]
    fn json_value_surfaces_clean() {
        // SURFACED arm (genuine — needs the runtime Json enum-construction
        // subsystem). Must NOT shim.
        let mut vm = VirtualMachine::new(VMConfig::default());
        let tr_cell = std::sync::Mutex::new(Some(TypedReturn::Ok(ConcreteReturn::JsonValue(
            shape_runtime::json_value::JsonValue::Int(1),
        ))));
        let tmf = TypedModuleFunction {
            invoke: Arc::new(move |_slots, _ctx| Ok(tr_cell.lock().unwrap().take().unwrap())),
            return_type: ConcreteType::Any,
            arg_types: vec![],
            arg_kinds: vec![],
        };
        let fn_id =
            vm.register_module_fn_entry(shape_runtime::module_exports::ModuleFnEntry::Typed(tmf));
        let err = vm.invoke_module_fn_id_stub(fn_id, &[]).unwrap_err();
        assert!(
            matches!(err, shape_value::VMError::NotImplemented(_)),
            "JsonValue must surface NotImplemented, got {err:?}"
        );
    }

    #[test]
    fn hashmap_string_heap_value_surfaces_clean_k3() {
        // K3 territory — must stay SURFACED (no shim) pending the ADR-006
        // HashMapData kinded-value-track amendment.
        let mut vm = VirtualMachine::new(VMConfig::default());
        let obj = super::project_concrete_return_for_test_typed_object();
        let tr_cell = std::sync::Mutex::new(Some(TypedReturn::Concrete(
            ConcreteReturn::HashMapStringHeapValue(vec![("k".to_string(), obj)]),
        )));
        let tmf = TypedModuleFunction {
            invoke: Arc::new(move |_slots, _ctx| Ok(tr_cell.lock().unwrap().take().unwrap())),
            return_type: ConcreteType::Any,
            arg_types: vec![],
            arg_kinds: vec![],
        };
        let fn_id =
            vm.register_module_fn_entry(shape_runtime::module_exports::ModuleFnEntry::Typed(tmf));
        let err = vm.invoke_module_fn_id_stub(fn_id, &[]).unwrap_err();
        assert!(
            matches!(err, shape_value::VMError::NotImplemented(_)),
            "HashMapStringHeapValue (K3) must surface NotImplemented, got {err:?}"
        );
    }
}

/// Test-only helper: build a representative `Arc<HeapValue::TypedObject>`
/// for the K2 / K3 array-and-map-of-heap-value tests above. Lives outside
/// the `#[cfg(test)]` module so it can be a `super::` reference from the
/// nested test module while still being compiled only under `cfg(test)`.
#[cfg(test)]
fn project_concrete_return_for_test_typed_object()
-> std::sync::Arc<shape_value::heap_value::HeapValue> {
    use shape_runtime::type_schema::typed_object_from_pairs;
    use shape_value::KindedSlot;
    use shape_value::heap_value::HeapValue;
    // Build a 1-field typed object via the shared builder, then recover its
    // raw TypedObject pointer into an Arc<HeapValue::TypedObject> carrier
    // (the ArrayHeapValue / HashMapStringHeapValue element shape).
    let slot: KindedSlot = typed_object_from_pairs(&[("id", KindedSlot::from_int(7))]);
    let ptr = slot.raw() as *const shape_value::heap_value::TypedObjectStorage;
    // The slot owns one share; transfer it into the Arc<HeapValue> wrapper
    // via `TypedObjectPtr::new` (which takes over that single share).
    std::mem::forget(slot);
    let tp = shape_value::heap_value::TypedObjectPtr::new(ptr);
    std::sync::Arc::new(HeapValue::TypedObject(tp))
}
