use super::super::*;
// `VMError` is intentionally left out of the executor/mod.rs star-import
// (see executor/mod.rs:126 comment); name it locally for the
// `invoke_module_fn_id_stub` surface.
use shape_value::VMError;

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

    /// Invoke a module-function entry by ID — Phase-2c stub.
    ///
    /// The legacy signature took `args: &[ValueWord]` and returned
    /// `Result<ValueWord, VMError>`. `ValueWord` was deleted; the kinded
    /// rebuild (ADR-006 §2.7.4 / §2.7.5) takes `&[KindedSlot]` and
    /// returns `Result<KindedSlot, VMError>`. Migration of every caller
    /// (`call_convention.rs`, `control_flow/mod.rs`, …) is out of this
    /// cluster's territory.
    pub(crate) fn invoke_module_fn_id_stub(&mut self, _fn_id: usize) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "invoke_module_fn_id: ValueWord-shaped args/result carrier deleted; \
             kinded `&[KindedSlot] -> Result<KindedSlot, _>` host-API revival is \
             Phase-2c — see ADR-006 §2.7.4 and the §2.7.5 cross-crate ABI policy."
                .to_string(),
        ))
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
