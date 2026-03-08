use super::super::*;

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
        let module_type_name = format!("__mod_{}", module.name);
        let module_entry = self.extension_methods.entry(module_type_name).or_default();
        for (export_name, func) in &module.exports {
            module_entry.insert(export_name.clone(), func.clone());
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
            module_entry.insert(export_name.clone(), wrapped);
        }
        self.module_registry.register(module);
    }

    /// Register a ModuleFn in the table and return its ID (for ValueWord::ModuleFunction).
    pub fn register_module_fn(&mut self, f: shape_runtime::module_exports::ModuleFn) -> usize {
        let id = self.module_fn_table.len();
        self.module_fn_table.push(f);
        id
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

            let result = module_fn(args, &ctx).map_err(VMError::RuntimeError);

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
        // Collect module data first to avoid borrow conflicts
        let module_data: Vec<(
            String,
            Vec<(String, shape_runtime::module_exports::ModuleFn)>,
            Vec<(String, shape_runtime::module_exports::AsyncModuleFn)>,
            Vec<String>,
        )> = self
            .module_registry
            .module_names()
            .iter()
            .filter_map(|name| {
                let module = self.module_registry.get(name)?;
                let sync_exports: Vec<_> = module
                    .exports
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let async_exports: Vec<_> = module
                    .async_exports
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
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
                Some((
                    name.to_string(),
                    sync_exports,
                    async_exports,
                    source_exports,
                ))
            })
            .collect();

        for (module_name, sync_exports, async_exports, source_exports) in module_data {
            // Find the module_binding index for this module name
            let binding_idx = self
                .program
                .module_binding_names
                .iter()
                .position(|n| n == &module_name);

            if let Some(idx) = binding_idx {
                let mut obj = HashMap::new();

                // Register sync exports directly
                for (export_name, module_fn) in sync_exports {
                    let fn_id = self.register_module_fn(module_fn);
                    obj.insert(export_name, ValueWord::from_module_function(fn_id as u32));
                }

                // Wrap async exports: block_in_place + block_on at call time
                for (export_name, async_fn) in async_exports {
                    let wrapped: shape_runtime::module_exports::ModuleFn =
                        Arc::new(move |args: &[ValueWord], _ctx: &shape_runtime::module_exports::ModuleContext| {
                            let future = async_fn(args);
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(future)
                            })
                        });
                    let fn_id = self.register_module_fn(wrapped);
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
                let cache_name = format!("__mod_{}", module_name);
                let schema_id = if let Some(schema) = self.lookup_schema_by_name(&cache_name) {
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
                    self.module_bindings.resize_with(idx + 1, ValueWord::none);
                }
                // BARRIER: heap write site — overwrites module binding during typed object initialization
                self.module_bindings[idx] = typed_nb;
            }
        }
    }
}
