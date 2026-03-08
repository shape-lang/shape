use super::super::*;

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

        // VM-native stdlib modules are always available, independent of
        // user-installed extension plugins.
        vm.register_stdlib_module(state_builtins::create_state_module());
        vm.register_stdlib_module(create_transport_module_exports());
        vm.register_stdlib_module(shape_runtime::stdlib::regex::create_regex_module());
        vm.register_stdlib_module(shape_runtime::stdlib::http::create_http_module());
        vm.register_stdlib_module(shape_runtime::stdlib::crypto::create_crypto_module());
        vm.register_stdlib_module(shape_runtime::stdlib::env::create_env_module());
        vm.register_stdlib_module(shape_runtime::stdlib::json::create_json_module());
        vm.register_stdlib_module(shape_runtime::stdlib::toml_module::create_toml_module());
        vm.register_stdlib_module(shape_runtime::stdlib::yaml::create_yaml_module());
        vm.register_stdlib_module(shape_runtime::stdlib::xml::create_xml_module());
        vm.register_stdlib_module(shape_runtime::stdlib::compress::create_compress_module());
        vm.register_stdlib_module(shape_runtime::stdlib::archive::create_archive_module());
        vm.register_stdlib_module(shape_runtime::stdlib::parallel::create_parallel_module());
        vm.register_stdlib_module(shape_runtime::stdlib::unicode::create_unicode_module());
        vm.register_stdlib_module(shape_runtime::stdlib::csv_module::create_csv_module());
        vm.register_stdlib_module(shape_runtime::stdlib::msgpack_module::create_msgpack_module());

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

    /// Generate a unique future ID for spawned async tasks
    pub(crate) fn next_future_id(&mut self) -> u64 {
        self.future_id_counter += 1;
        self.future_id_counter
    }

    /// Get function ID for fast repeated calls (avoids name lookup in hot loops)
    pub fn get_function_id(&self, name: &str) -> Option<u16> {
        self.program
            .functions
            .iter()
            .position(|f| f.name == name)
            .map(|id| id as u16)
    }
}
