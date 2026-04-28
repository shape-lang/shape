use super::super::*;
use shape_value::ValueWordExt;

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
    pub fn register_extension(&mut self, module: shape_runtime::module_exports::ModuleExports) {
        // Merge method intrinsics
        for (type_name, methods) in &module.method_intrinsics {
            let entry = self.extension_methods.entry(type_name.clone()).or_default();
            for (method_name, func) in methods {
                entry.insert(method_name.clone(), func.clone());
            }
        }
        // Expose module exports as methods on the module object type so
        // `module.fn(...)` dispatches via CallMethod without UFCS rewrites.
        // Register under the canonical type name only (`__mod_std::core::json`).
        let canonical_type_name = format!("__mod_{}", module.name);

        let mut sync_methods: Vec<(String, shape_runtime::module_exports::ModuleFn)> = Vec::new();
        for (export_name, func) in &module.exports {
            sync_methods.push((export_name.clone(), func.clone()));
        }
        for (export_name, async_fn) in &module.async_exports {
            let async_fn = async_fn.clone();
            let wrapped: shape_runtime::module_exports::ModuleFn = Arc::new(
                move |args: &[ValueWord], _ctx: &shape_runtime::module_exports::ModuleContext| {
                    let future = async_fn(args);
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(future)
                    })
                },
            );
            sync_methods.push((export_name.clone(), wrapped));
        }

        let canonical_entry = self
            .extension_methods
            .entry(canonical_type_name)
            .or_default();
        for (name, func) in &sync_methods {
            canonical_entry.insert(name.clone(), func.clone());
        }

        self.module_registry.register(module);
    }

    /// Register a module-function entry in the table and return its ID
    /// (for `ValueWord::ModuleFunction`).
    pub fn register_module_fn_entry(
        &mut self,
        entry: shape_runtime::module_exports::ModuleFnEntry,
    ) -> usize {
        let id = self.module_fn_table.len();
        self.module_fn_table.push(entry);
        id
    }

    /// Backwards-compatible legacy registration. Wraps the supplied
    /// `ModuleFn` in `ModuleFnEntry::Legacy` and registers it.
    pub fn register_module_fn(&mut self, f: shape_runtime::module_exports::ModuleFn) -> usize {
        self.register_module_fn_entry(shape_runtime::module_exports::ModuleFnEntry::Legacy(f))
    }

    /// Invoke a module-function entry by ID, marshalling args/result at
    /// the dispatch boundary.
    ///
    /// This is the typed dispatch entry point introduced in Phase 4c.3.
    /// For `Typed`/`TypedAsync` entries the function body returns a
    /// `TypedReturn` directly — marshalling to `ValueWord` happens here,
    /// not inside the body. For `Legacy` entries the body returns a
    /// `ValueWord` directly (legacy ABI).
    pub(crate) fn invoke_module_fn_id(
        &mut self,
        fn_id: usize,
        args: &[ValueWord],
    ) -> Result<ValueWord, VMError> {
        let entry = self.module_fn_table.get(fn_id).cloned().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Module function ID {} not found in registry",
                fn_id
            ))
        })?;
        self.invoke_module_fn_entry(&entry, args)
    }

    /// Invoke a registered module-function entry.
    ///
    /// Routes to one of three paths based on the entry kind:
    /// - `Typed`: build a synchronous `ModuleContext`, run the typed
    ///   body, marshal the resulting `TypedReturn` into a `ValueWord`.
    /// - `TypedAsync`: block on the future via tokio's
    ///   `block_in_place` + `block_on`, marshal the result.
    /// - `Legacy`: build a synchronous `ModuleContext`, run the body
    ///   (which already returns a `ValueWord`).
    pub(crate) fn invoke_module_fn_entry(
        &mut self,
        entry: &shape_runtime::module_exports::ModuleFnEntry,
        args: &[ValueWord],
    ) -> Result<ValueWord, VMError> {
        use shape_runtime::module_exports::ModuleFnEntry;
        match entry {
            ModuleFnEntry::Typed(typed) => {
                let typed_clone = typed.clone();
                self.invoke_typed_module_fn(&typed_clone, args)
            }
            ModuleFnEntry::TypedAsync(typed_async) => {
                let invoke = typed_async.invoke.clone();
                let owned_args = args.to_vec();
                let typed = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(invoke(owned_args))
                })
                .map_err(VMError::RuntimeError)?;
                Ok(typed.into_value_word())
            }
            ModuleFnEntry::Legacy(module_fn) => {
                self.invoke_module_fn(module_fn, args)
            }
        }
    }

    /// Invoke a typed (synchronous) module function, building the
    /// `ModuleContext` exactly the same way as the legacy invoke path.
    /// The function body returns `TypedReturn`; marshalling to
    /// `ValueWord` happens at this boundary.
    pub(crate) fn invoke_typed_module_fn(
        &mut self,
        typed_fn: &shape_runtime::typed_module_exports::TypedModuleFunction,
        args: &[ValueWord],
    ) -> Result<ValueWord, VMError> {
        unsafe {
            let vm_ptr = self as *mut VirtualMachine;

            let invoker =
                |callable: &ValueWord, call_args: &[ValueWord]| -> Result<ValueWord, String> {
                    (*vm_ptr)
                        .call_value_immediate_nb(callable, call_args, None)
                        .map_err(|e| e.to_string())
                };

            unsafe fn vm_callable_invoker(
                ctx: *mut std::ffi::c_void,
                callable: &ValueWord,
                args: &[ValueWord],
            ) -> Result<ValueWord, String> {
                let vm = unsafe { &mut *(ctx as *mut VirtualMachine) };
                vm.call_value_immediate_nb(callable, args, None)
                    .map_err(|err| err.to_string())
            }

            let vm_snapshot = (*vm_ptr).capture_vm_state();

            let ctx = shape_runtime::module_exports::ModuleContext {
                schemas: &(*vm_ptr).program.type_schema_registry,
                invoke_callable: Some(&invoker),
                raw_invoker: Some(shape_runtime::module_exports::RawCallableInvoker {
                    ctx: vm_ptr as *mut std::ffi::c_void,
                    invoke: vm_callable_invoker,
                }),
                function_hashes: if (*vm_ptr).function_hash_raw.is_empty() {
                    None
                } else {
                    Some(&(*vm_ptr).function_hash_raw)
                },
                vm_state: Some(&vm_snapshot),
                granted_permissions: None,
                scope_constraints: None,
                set_pending_resume: Some(&|snapshot| {
                    (*vm_ptr).pending_resume = Some(snapshot);
                }),
                set_pending_frame_resume: Some(&|ip_offset, locals| {
                    (*vm_ptr).pending_frame_resume = Some(FrameResumeData { ip_offset, locals });
                }),
            };

            // Set thread-local program reference so remote.__call() can access it.
            crate::executor::builtins::remote_builtins::set_current_program(&(*vm_ptr).program);
            let typed_result = (typed_fn.invoke)(args, &ctx).map_err(VMError::RuntimeError);
            crate::executor::builtins::remote_builtins::clear_current_program();

            // Check if the module function requested a VM state resume.
            if (*vm_ptr).pending_resume.is_some() {
                return Err(VMError::ResumeRequested);
            }

            // Marshal TypedReturn → ValueWord at the dispatch boundary.
            // The body did NOT round-trip through `into_value_word()`
            // itself; this is the only place that conversion happens
            // for typed exports.
            typed_result.map(|t| t.into_value_word())
        }
    }

    /// Invoke a registered module function with a scoped `ModuleContext`.
    ///
    /// The context provides access to the type schema registry, a callable
    /// invoker closure, and a raw invoker that extensions can capture in
    /// long-lived structs (e.g., CFFI callback userdata).
    pub(crate) fn invoke_module_fn(
        &mut self,
        module_fn: &shape_runtime::module_exports::ModuleFn,
        args: &[ValueWord],
    ) -> Result<ValueWord, VMError> {
        // SAFETY: The module function is called synchronously and the VM pointer
        // remains valid for the duration of the call.  We use a raw pointer so
        // that: (a) the callable invoker can re-enter the VM, and (b) we can
        // simultaneously borrow the schema registry.
        unsafe {
            let vm_ptr = self as *mut VirtualMachine;

            let invoker =
                |callable: &ValueWord, call_args: &[ValueWord]| -> Result<ValueWord, String> {
                    (*vm_ptr)
                        .call_value_immediate_nb(callable, call_args, None)
                        .map_err(|e| e.to_string())
                };

            unsafe fn vm_callable_invoker(
                ctx: *mut std::ffi::c_void,
                callable: &ValueWord,
                args: &[ValueWord],
            ) -> Result<ValueWord, String> {
                let vm = unsafe { &mut *(ctx as *mut VirtualMachine) };
                vm.call_value_immediate_nb(callable, args, None)
                    .map_err(|err| err.to_string())
            }

            // Capture a read-only snapshot of VM state before dispatching.
            // The snapshot lives on the stack and is referenced by ModuleContext
            // for the duration of this synchronous call.
            let vm_snapshot = (*vm_ptr).capture_vm_state();

            let ctx = shape_runtime::module_exports::ModuleContext {
                schemas: &(*vm_ptr).program.type_schema_registry,
                invoke_callable: Some(&invoker),
                raw_invoker: Some(shape_runtime::module_exports::RawCallableInvoker {
                    ctx: vm_ptr as *mut std::ffi::c_void,
                    invoke: vm_callable_invoker,
                }),
                function_hashes: if (*vm_ptr).function_hash_raw.is_empty() {
                    None
                } else {
                    Some(&(*vm_ptr).function_hash_raw)
                },
                vm_state: Some(&vm_snapshot),
                granted_permissions: None,
                scope_constraints: None,
                set_pending_resume: Some(&|snapshot| {
                    // vm_ptr is valid for the duration of the module function call
                    // (outer unsafe block covers this).
                    (*vm_ptr).pending_resume = Some(snapshot);
                }),
                set_pending_frame_resume: Some(&|ip_offset, locals| {
                    // vm_ptr is valid for the duration of the module function call
                    // (outer unsafe block covers this).
                    (*vm_ptr).pending_frame_resume = Some(FrameResumeData { ip_offset, locals });
                }),
            };

            // Set thread-local program reference so remote.__call() can access it.
            crate::executor::builtins::remote_builtins::set_current_program(&(*vm_ptr).program);
            let result = module_fn(args, &ctx).map_err(VMError::RuntimeError);
            crate::executor::builtins::remote_builtins::clear_current_program();

            // Check if the module function requested a VM state resume.
            // If so, return a special error that the dispatch loop intercepts.
            if (*vm_ptr).pending_resume.is_some() {
                return Err(VMError::ResumeRequested);
            }

            result
        }
    }

    /// Populate extension module objects as module_bindings (json, duckdb, etc.).
    /// These are used by extension Shape code (e.g., `duckdb.query(...)`).
    /// Call this after load_program().
    pub fn populate_module_objects(&mut self) {
        // Collect module data first to avoid borrow conflicts.
        //
        // Phase 4c.3: prefer typed exports — typed sync entries become
        // `ModuleFnEntry::Typed`, typed async become
        // `ModuleFnEntry::TypedAsync`, and any remaining legacy
        // sync/async entries (intrinsic-only modules / unmigrated
        // exports) become `ModuleFnEntry::Legacy` so the dispatch path
        // still works.
        let module_data: Vec<(
            String,
            Vec<(String, shape_runtime::module_exports::ModuleFnEntry)>,
            Vec<String>,
        )> = self
            .module_registry
            .module_names()
            .iter()
            .filter_map(|name| {
                let module = self.module_registry.get(name)?;
                let mut entries: Vec<(
                    String,
                    shape_runtime::module_exports::ModuleFnEntry,
                )> = Vec::new();
                let typed = module.typed_exports();

                // Typed sync exports first — gives the dispatch path
                // the typed body without the legacy ValueWord round-trip.
                for (export_name, typed_fn) in &typed.functions {
                    entries.push((
                        export_name.clone(),
                        shape_runtime::module_exports::ModuleFnEntry::Typed(typed_fn.clone()),
                    ));
                }
                // Typed async exports — block_in_place wrapping happens
                // inside `invoke_module_fn_entry`.
                for (export_name, typed_async) in &typed.async_functions {
                    entries.push((
                        export_name.clone(),
                        shape_runtime::module_exports::ModuleFnEntry::TypedAsync(
                            typed_async.clone(),
                        ),
                    ));
                }
                // Legacy fallback: any name NOT already covered by a
                // typed entry. After Phase 4c.2 the shipped stdlib has
                // no remaining legacy sync/async exports, but this
                // keeps user/extension modules working.
                for (export_name, module_fn) in &module.exports {
                    if typed.functions.contains_key(export_name)
                        || typed.async_functions.contains_key(export_name)
                    {
                        continue;
                    }
                    entries.push((
                        export_name.clone(),
                        shape_runtime::module_exports::ModuleFnEntry::Legacy(module_fn.clone()),
                    ));
                }
                for (export_name, async_fn) in &module.async_exports {
                    if typed.functions.contains_key(export_name)
                        || typed.async_functions.contains_key(export_name)
                    {
                        continue;
                    }
                    let async_fn = async_fn.clone();
                    let wrapped: shape_runtime::module_exports::ModuleFn = Arc::new(
                        move |args: &[ValueWord],
                              _ctx: &shape_runtime::module_exports::ModuleContext| {
                            let future = async_fn(args);
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(future)
                            })
                        },
                    );
                    entries.push((
                        export_name.clone(),
                        shape_runtime::module_exports::ModuleFnEntry::Legacy(wrapped),
                    ));
                }

                let mut source_exports = Vec::new();
                for artifact in &module.module_artifacts {
                    if artifact.module_path != *name {
                        continue;
                    }
                    let Some(source) = artifact.source.as_deref() else {
                        continue;
                    };
                    if let Ok(exports) =
                        shape_runtime::module_loader::collect_exported_function_names_from_source(
                            &artifact.module_path,
                            source,
                        )
                    {
                        source_exports.extend(exports);
                    }
                }
                source_exports.sort();
                source_exports.dedup();
                Some((name.to_string(), entries, source_exports))
            })
            .collect();

        for (module_name, entries, source_exports) in module_data {
            // Find the module_binding index for this module name.
            // Prefer the hidden native binding (`__imported_module__::X`) when it exists,
            // so that compiled artifact code referencing the hidden binding gets the
            // native module object. The plain binding is filled by the compiled module
            // declaration at runtime.
            let hidden_name =
                crate::compiler::BytecodeCompiler::hidden_native_module_binding_name(&module_name);
            let binding_idx = self
                .program
                .module_binding_names
                .iter()
                .position(|binding_name| binding_name == &hidden_name)
                .or_else(|| {
                    self.program
                        .module_binding_names
                        .iter()
                        .position(|binding_name| binding_name == &module_name)
                });

            if let Some(idx) = binding_idx {
                let mut obj = HashMap::new();

                // Register all entries (typed sync, typed async, legacy
                // fallback) into the module-fn table. Dispatch on
                // entry kind happens inside `invoke_module_fn_entry`.
                for (export_name, entry) in entries {
                    let fn_id = self.register_module_fn_entry(entry);
                    obj.insert(export_name, ValueWord::from_module_function(fn_id as u32));
                }

                // Add Shape-source exported functions (compiled into bytecode).
                // These are regular VM functions, not host module functions.
                for export_name in source_exports {
                    if obj.contains_key(&export_name) {
                        continue;
                    }
                    if let Some(&func_id) = self.function_name_index.get(&export_name) {
                        obj.insert(export_name, ValueWord::from_function(func_id));
                    }
                }

                // Module object schemas must be predeclared at compile time.
                // Use the canonical module name only.
                let cache_name = format!("__mod_{}", module_name);
                let schema_id =
                    if let Some(schema) = self.lookup_schema_by_name(&cache_name) {
                        schema.id
                    } else {
                        // Keep execution predictable: no runtime schema synthesis.
                        // Missing module schema means compiler/loader setup is incomplete.
                        continue;
                    };

                // Look up schema to get field ordering
                let Some(schema) = self.lookup_schema(schema_id) else {
                    continue;
                };
                let field_order: Vec<String> =
                    schema.fields.iter().map(|f| f.name.clone()).collect();

                let mut slots = Vec::with_capacity(field_order.len());
                let mut heap_mask: u64 = 0;
                for (i, field_name) in field_order.iter().enumerate() {
                    let nb_val = obj.get(field_name).cloned().unwrap_or_else(ValueWord::none);
                    let (slot, is_heap) =
                        crate::executor::objects::object_creation::nb_to_slot_with_field_type(
                            &nb_val, None,
                        );
                    slots.push(slot);
                    if is_heap {
                        heap_mask |= 1u64 << i;
                    }
                }

                let typed_nb = ValueWord::from_heap_value(HeapValue::TypedObject {
                    schema_id: schema_id as u64,
                    slots: slots.into_boxed_slice(),
                    heap_mask,
                });
                if idx >= self.module_bindings.len() {
                    self.module_bindings.resize_with(idx + 1, || Self::NONE_BITS);
                }
                // BARRIER: heap write site — overwrites module binding during typed object initialization
                self.binding_write_raw(idx, typed_nb);
            }
        }
    }
}
