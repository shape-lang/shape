use super::*;

impl VirtualMachine {
    pub fn new(config: VMConfig) -> Self {
        let debugger = if config.debug_mode {
            Some(VMDebugger::new())
        } else {
            None
        };

        let gc = GarbageCollector::new(config.gc_config.clone());

        // Initialize builtin schema IDs (overwritten from loaded bytecode registry
        // in `load_program`).
        let (registry, builtin_schemas) =
            shape_runtime::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();

        let mut program = BytecodeProgram::new();
        program.type_schema_registry = registry;

        let mut vm = Self {
            config,
            program,
            ip: 0,
            stack: (0..crate::constants::DEFAULT_STACK_CAPACITY)
                .map(|_| ValueWord::none())
                .collect(),
            sp: 0,
            module_bindings: Vec::new(),
            call_stack: Vec::with_capacity(crate::constants::DEFAULT_CALL_STACK_CAPACITY),
            loop_stack: Vec::new(),
            timeframe_stack: Vec::new(),
            debugger,
            gc,
            instruction_count: 0,
            exception_handlers: Vec::new(),
            builtin_schemas,
            last_error_line: None,
            last_error_file: None,
            last_uncaught_exception: None,
            module_init_done: false,
            output_buffer: None,
            module_registry: shape_runtime::module_exports::ModuleExportRegistry::new(),
            module_fn_table: Vec::new(),
            function_name_index: HashMap::new(),
            extension_methods: HashMap::new(),
            merged_schema_cache: HashMap::new(),
            interrupt: Arc::new(AtomicU8::new(0)),
            future_id_counter: 0,
            async_scope_stack: Vec::new(),
            task_scheduler: task_scheduler::TaskScheduler::new(),
            foreign_fn_handles: Vec::new(),
            function_hashes: Vec::new(),
            function_hash_raw: Vec::new(),
            function_id_by_hash: HashMap::new(),
            function_entry_points: Vec::new(),
            program_entry_ip: 0,
            resource_usage: None,
            time_travel: None,
            #[cfg(feature = "gc")]
            gc_heap: None,
            #[cfg(feature = "jit")]
            jit_compiled: false,
            #[cfg(feature = "jit")]
            jit_dispatch_table: std::collections::HashMap::new(),
            tier_manager: None,
            pending_resume: None,
            pending_frame_resume: None,
            metrics: None,
            feedback_vectors: Vec::new(),
            megamorphic_cache: crate::megamorphic_cache::MegamorphicCache::new(),
        };

        // VM-native std modules are always available, independent of
        // user-registered extension modules.
        vm.register_extension(state_builtins::create_state_module());
        vm.register_extension(create_transport_module_exports());
        vm.register_extension(shape_runtime::stdlib::regex::create_regex_module());
        vm.register_extension(shape_runtime::stdlib::http::create_http_module());
        vm.register_extension(shape_runtime::stdlib::crypto::create_crypto_module());
        vm.register_extension(shape_runtime::stdlib::env::create_env_module());
        vm.register_extension(shape_runtime::stdlib::log::create_log_module());
        vm.register_extension(shape_runtime::stdlib::json::create_json_module());
        vm.register_extension(shape_runtime::stdlib::toml_module::create_toml_module());
        vm.register_extension(shape_runtime::stdlib::yaml::create_yaml_module());
        vm.register_extension(shape_runtime::stdlib::xml::create_xml_module());
        vm.register_extension(shape_runtime::stdlib::compress::create_compress_module());
        vm.register_extension(shape_runtime::stdlib::archive::create_archive_module());
        vm.register_extension(shape_runtime::stdlib::parallel::create_parallel_module());
        vm.register_extension(shape_runtime::stdlib::unicode::create_unicode_module());

        // Initialise metrics collector when requested.
        if vm.config.metrics_enabled {
            vm.metrics = Some(crate::metrics::VmMetrics::new());
        }

        // Auto-initialise the tracing GC heap when requested.
        #[cfg(feature = "gc")]
        if vm.config.use_tracing_gc {
            vm.init_gc_heap();
        }

        vm
    }

    /// Attach resource limits to this VM. The dispatch loop will enforce them.
    pub fn with_resource_limits(mut self, limits: crate::resource_limits::ResourceLimits) -> Self {
        let mut usage = crate::resource_limits::ResourceUsage::new(limits);
        usage.start();
        self.resource_usage = Some(usage);
        self
    }

    /// Initialize the GC heap for this VM instance (gc feature only).
    ///
    /// Sets up the GcHeap and registers it as the thread-local heap so
    /// ValueWord::heap_box() and ValueSlot::from_heap() can allocate through it.
    /// Also configures the GC threshold from the VM's GCConfig.
    #[cfg(feature = "gc")]
    pub fn init_gc_heap(&mut self) {
        let heap = shape_gc::GcHeap::new();
        self.gc_heap = Some(heap);
        // Set thread-local GC heap pointer AFTER the move into self.gc_heap
        // so the pointer remains valid for the VM's lifetime.
        if let Some(ref mut heap) = self.gc_heap {
            unsafe { shape_gc::set_thread_gc_heap(heap as *mut _) };
        }
    }

    /// Set the interrupt flag (shared with Ctrl+C handler).
    pub fn set_interrupt(&mut self, flag: Arc<AtomicU8>) {
        self.interrupt = flag;
    }

    /// Enable time-travel debugging with the given capture mode and history limit.
    pub fn enable_time_travel(&mut self, mode: time_travel::CaptureMode, max_entries: usize) {
        self.time_travel = Some(time_travel::TimeTravel::new(mode, max_entries));
    }

    /// Disable time-travel debugging and discard history.
    pub fn disable_time_travel(&mut self) {
        self.time_travel = None;
    }

    /// Mark this VM as having been JIT-compiled selectively.
    ///
    /// Call this after using `shape_jit::JITCompiler::compile_program_selective`
    /// externally to JIT-compile functions that benefit from native execution.
    /// The caller is responsible for performing the compilation via `shape-jit`
    /// (which depends on `shape-vm`, so the dependency flows one way).
    ///
    /// # Example (in a crate that depends on both `shape-vm` and `shape-jit`):
    ///
    /// ```ignore
    /// let mut compiler = shape_jit::JITCompiler::new()?;
    /// let (_jitted_fn, _table) = compiler.compile_program_selective("main", vm.program())?;
    /// vm.set_jit_compiled();
    /// ```
    #[cfg(feature = "jit")]
    pub fn set_jit_compiled(&mut self) {
        self.jit_compiled = true;
    }

    /// Returns whether selective JIT compilation has been applied to this VM.
    #[cfg(feature = "jit")]
    pub fn is_jit_compiled(&self) -> bool {
        self.jit_compiled
    }

    /// Register a JIT-compiled function in the dispatch table.
    ///
    /// After registration, calls to this function_id will attempt JIT dispatch
    /// before falling back to bytecode interpretation.
    #[cfg(feature = "jit")]
    pub fn register_jit_function(&mut self, function_id: u16, ptr: JitFnPtr) {
        self.jit_dispatch_table.insert(function_id, ptr);
        self.jit_compiled = true;
    }

    /// Get the JIT dispatch table for inspection or external use.
    #[cfg(feature = "jit")]
    pub fn jit_dispatch_table(&self) -> &std::collections::HashMap<u16, JitFnPtr> {
        &self.jit_dispatch_table
    }

    /// Enable tiered compilation for this VM.
    ///
    /// Must be called after `load_program()` so the function count is known.
    /// The caller is responsible for spawning a background compilation thread
    /// that reads from the request channel and sends results back.
    ///
    /// Returns `(request_rx, result_tx)` that the background thread should use.
    pub fn enable_tiered_compilation(
        &mut self,
    ) -> (
        std::sync::mpsc::Receiver<crate::tier::CompilationRequest>,
        std::sync::mpsc::Sender<crate::tier::CompilationResult>,
    ) {
        let function_count = self.program.functions.len();
        let mut mgr = crate::tier::TierManager::new(function_count, true);

        let (req_tx, req_rx) = std::sync::mpsc::channel();
        let (res_tx, res_rx) = std::sync::mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        self.tier_manager = Some(mgr);
        (req_rx, res_tx)
    }

    /// Get a reference to the tier manager, if tiered compilation is enabled.
    pub fn tier_manager(&self) -> Option<&crate::tier::TierManager> {
        self.tier_manager.as_ref()
    }

    /// Poll the tier manager for completed background JIT compilations.
    ///
    /// Completed compilations are applied by `TierManager::poll_completions()`,
    /// which updates its internal `native_code_table`. The JIT dispatch fast
    /// path in `op_call` reads from `tier_mgr.get_native_code()`.
    ///
    /// Called every 1024 instructions from the dispatch loop (same cadence as
    /// interrupt and GC safepoint checks).
    pub(crate) fn poll_tier_completions(&mut self) {
        if let Some(ref mut tier_mgr) = self.tier_manager {
            // poll_completions() reads from the compilation_rx channel and
            // updates native_code_table internally.
            let completions = tier_mgr.poll_completions();

            // Record tier transition events in metrics if enabled.
            if let Some(ref mut metrics) = self.metrics {
                for result in &completions {
                    if result.native_code.is_some() {
                        let from_tier = match result.compiled_tier {
                            crate::tier::Tier::BaselineJit => 0,   // was Interpreted
                            crate::tier::Tier::OptimizingJit => 1, // was BaselineJit
                            crate::tier::Tier::Interpreted => continue,
                        };
                        let to_tier = match result.compiled_tier {
                            crate::tier::Tier::BaselineJit => 1,
                            crate::tier::Tier::OptimizingJit => 2,
                            crate::tier::Tier::Interpreted => continue,
                        };
                        metrics.record_tier_event(crate::metrics::TierEvent {
                            function_id: result.function_id,
                            from_tier,
                            to_tier,
                            call_count: tier_mgr.get_call_count(result.function_id),
                            timestamp_us: metrics.elapsed_us(),
                        });
                    }
                }
            }
        }
    }

    /// Get or create a feedback vector for the current function.
    /// Returns None if tiered compilation is disabled.
    #[inline]
    pub(crate) fn current_feedback_vector(
        &mut self,
    ) -> Option<&mut crate::feedback::FeedbackVector> {
        let func_id = self.call_stack.last()?.function_id? as usize;
        if func_id >= self.feedback_vectors.len() {
            return None;
        }
        if self.feedback_vectors[func_id].is_none() {
            if self.tier_manager.is_none() {
                return None;
            }
            self.feedback_vectors[func_id] =
                Some(crate::feedback::FeedbackVector::new(func_id as u16));
        }
        self.feedback_vectors[func_id].as_mut()
    }

    /// Access the feedback vectors (for JIT compilation).
    pub fn feedback_vectors(&self) -> &[Option<crate::feedback::FeedbackVector>] {
        &self.feedback_vectors
    }

    /// Get a reference to the loaded program (for external JIT compilation).
    pub fn program(&self) -> &BytecodeProgram {
        &self.program
    }

    /// Get a reference to the time-travel debugger, if enabled.
    pub fn time_travel(&self) -> Option<&time_travel::TimeTravel> {
        self.time_travel.as_ref()
    }

    /// Get a mutable reference to the time-travel debugger, if enabled.
    pub fn time_travel_mut(&mut self) -> Option<&mut time_travel::TimeTravel> {
        self.time_travel.as_mut()
    }

    /// Get a reference to the extension module registry.
    pub fn module_registry(&self) -> &shape_runtime::module_exports::ModuleExportRegistry {
        &self.module_registry
    }

    /// Register an extension module into the VM's module registry.
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

    /// Generate a unique future ID for spawned async tasks
    pub(crate) fn next_future_id(&mut self) -> u64 {
        self.future_id_counter += 1;
        self.future_id_counter
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
    // ========================================================================
    // Conversion and Helper Methods

    /// Create a TypedObject from field name-value pairs.
    ///
    /// Create a TypedObject from predeclared compile-time schemas.
    pub(crate) fn create_typed_object_from_pairs(
        &mut self,
        fields: &[(&str, ValueWord)],
    ) -> Result<ValueWord, VMError> {
        // Build field names for schema lookup
        let field_names: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
        let key: String = field_names.join(",");
        let schema_name = format!("__native_{}", key);

        // Runtime schema synthesis is retired: these object layouts must be
        // predeclared in compile-time registries.
        let schema_id = self
            .lookup_schema_by_name(&schema_name)
            .map(|s| s.id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Missing predeclared schema '{}'. Runtime schema generation is disabled.",
                    schema_name
                ))
            })?;

        let field_types = self.lookup_schema(schema_id).map(|schema| {
            schema
                .fields
                .iter()
                .map(|f| f.field_type.clone())
                .collect::<Vec<_>>()
        });

        // Build slots and heap_mask.
        let mut slots = Vec::with_capacity(fields.len());
        let mut heap_mask: u64 = 0;
        for (i, (_name, nb)) in fields.iter().enumerate() {
            let field_type = field_types.as_ref().and_then(|types| types.get(i));
            let (slot, is_heap) =
                crate::executor::objects::object_creation::nb_to_slot_with_field_type(
                    nb, field_type,
                );
            slots.push(slot);
            if is_heap {
                heap_mask |= 1u64 << i;
            }
        }

        Ok(ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        }))
    }

    /// Look up a schema by ID in the compiled program registry.
    pub(crate) fn lookup_schema(
        &self,
        schema_id: u32,
    ) -> Option<&shape_runtime::type_schema::TypeSchema> {
        self.program.type_schema_registry.get_by_id(schema_id)
    }

    pub(crate) fn lookup_schema_by_name(
        &self,
        name: &str,
    ) -> Option<&shape_runtime::type_schema::TypeSchema> {
        self.program.type_schema_registry.get(name)
    }

    /// Derive a merged schema from two existing schemas.
    /// Right fields overwrite left fields with the same name.
    /// Caches result by (left_schema_id, right_schema_id).
    pub(crate) fn derive_merged_schema(
        &mut self,
        left_id: u32,
        right_id: u32,
    ) -> Result<u32, VMError> {
        if let Some(&cached) = self.merged_schema_cache.get(&(left_id, right_id)) {
            return Ok(cached);
        }

        // Runtime schema synthesis is disabled; merged schemas must exist.
        let merged_name = format!("__merged_{}_{}", left_id, right_id);
        let intersection_name = format!("__intersection_{}_{}", left_id, right_id);
        let merged_id = self
            .lookup_schema_by_name(&merged_name)
            .or_else(|| self.lookup_schema_by_name(&intersection_name))
            .map(|s| s.id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Missing predeclared merged schema for {} + {} (expected '{}' or '{}').",
                    left_id, right_id, merged_name, intersection_name
                ))
            })?;
        self.merged_schema_cache
            .insert((left_id, right_id), merged_id);

        Ok(merged_id)
    }

    /// Derive a subset schema: base schema minus excluded fields.
    /// Uses registry name-based lookup for caching.
    pub(crate) fn derive_subset_schema(
        &mut self,
        base_id: u32,
        exclude: &std::collections::HashSet<String>,
    ) -> Result<u32, VMError> {
        // Build deterministic cache name
        let mut excluded_sorted: Vec<&String> = exclude.iter().collect();
        excluded_sorted.sort();
        let cache_name = format!(
            "__sub_{}_exc_{}",
            base_id,
            excluded_sorted
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        // Runtime schema synthesis is disabled; subset schemas must be predeclared.
        if let Some(schema) = self.lookup_schema_by_name(&cache_name) {
            return Ok(schema.id);
        }
        Err(VMError::RuntimeError(format!(
            "Missing predeclared subset schema '{}' (runtime schema derivation is disabled).",
            cache_name
        )))
    }
}
