use super::super::*;
use shape_value::ValueWordExt;

impl VirtualMachine {
    /// Load a program into the VM
    pub fn load_program(&mut self, program: BytecodeProgram) {
        // Content-addressed bytecode is the canonical runtime format.
        // Do not silently fall back to the flat instruction stream if linking fails.
        if let Some(ref ca_program) = program.content_addressed {
            let linked = crate::linker::link(ca_program).unwrap_or_else(|e| {
                panic!(
                    "content-addressed linker failed ({} function blobs): {}",
                    ca_program.function_store.len(),
                    e
                )
            });
            self.load_linked_program(linked);
            return;
        }

        self.program = program;
        if shape_runtime::type_schema::builtin_schemas::resolve_builtin_schema_ids(
            &self.program.type_schema_registry,
        )
        .is_none()
        {
            // Programs built manually in tests may omit builtin schemas.
            // Merge the static stdlib registry (includes builtin fixed schemas)
            // without synthesizing any dynamic runtime schemas.
            let (stdlib_registry, _) =
                shape_runtime::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
            self.program.type_schema_registry.merge(stdlib_registry);
        }
        self.builtin_schemas =
            shape_runtime::type_schema::builtin_schemas::resolve_builtin_schema_ids(
                &self.program.type_schema_registry,
            )
            .expect(
                "compiled program is missing builtin schemas (__AnyError, __TraceFrame, ...); \
             schema registry must include static builtin schemas",
            );
        // Reserve schema IDs above the compiled program registry.
        let max_program_id = self
            .program
            .type_schema_registry
            .max_schema_id()
            .unwrap_or(0);
        shape_runtime::type_schema::ensure_next_schema_id_above(max_program_id);
        self.rebuild_function_name_index();
        self.populate_content_addressed_metadata();
        self.program_entry_ip = 0;
        self.module_init_done = false;
        self.feedback_vectors
            .resize_with(self.program.functions.len(), || None);
        self.reset();

        // Bytecode verification: ensure trusted opcodes have valid FrameDescriptors.
        #[cfg(debug_assertions)]
        {
            if let Err(errors) = crate::bytecode::verifier::verify_trusted_opcodes(&self.program) {
                eprintln!(
                    "Bytecode verification warning: {} violation(s) found",
                    errors.len()
                );
                for e in &errors {
                    eprintln!("  - {}", e);
                }
            }
            if let Err(errors) =
                crate::bytecode::verifier::verify_v2_typed_opcodes(&self.program)
            {
                eprintln!(
                    "V2 bytecode verification warning: {} violation(s) found",
                    errors.len()
                );
                for e in &errors {
                    eprintln!("  - {}", e);
                }
            }
        }

        #[cfg(not(debug_assertions))]
        {
            if let Err(errors) = crate::bytecode::verifier::verify_trusted_opcodes(&self.program) {
                eprintln!(
                    "Bytecode verification failed: {} violation(s)",
                    errors.len()
                );
                for e in &errors {
                    eprintln!("  - {}", e);
                }
            }
            if let Err(errors) =
                crate::bytecode::verifier::verify_v2_typed_opcodes(&self.program)
            {
                eprintln!(
                    "V2 bytecode verification failed: {} violation(s)",
                    errors.len()
                );
                for e in &errors {
                    eprintln!("  - {}", e);
                }
            }
        }
    }

    /// Load a `LinkedProgram` into the VM, extracting content-addressed metadata
    /// directly from the linked function table.
    ///
    /// This converts the `LinkedProgram` into the flat `BytecodeProgram` layout that
    /// the executor expects, then populates `function_hashes` and `function_entry_points`
    /// from the linked function metadata.
    pub fn load_linked_program(&mut self, linked: crate::bytecode::LinkedProgram) {
        let entry_function_id = linked
            .hash_to_id
            .get(&linked.entry)
            .copied()
            .or_else(|| linked.functions.iter().position(|f| f.name == "__main__"))
            .unwrap_or(0);
        let entry_ip = linked
            .functions
            .get(entry_function_id)
            .map(|f| f.entry_point)
            .unwrap_or(0);

        // Extract hash metadata before converting
        let hashes: Vec<Option<FunctionHash>> = linked
            .functions
            .iter()
            .map(|lf| {
                if lf.blob_hash == FunctionHash::ZERO {
                    None
                } else {
                    Some(lf.blob_hash)
                }
            })
            .collect();
        let entry_points: Vec<usize> = linked.functions.iter().map(|lf| lf.entry_point).collect();

        // Convert LinkedProgram functions to BytecodeProgram functions
        let functions: Vec<crate::bytecode::Function> = linked
            .functions
            .iter()
            .map(|lf| crate::bytecode::Function {
                name: lf.name.clone(),
                arity: lf.arity,
                param_names: lf.param_names.clone(),
                locals_count: lf.locals_count,
                entry_point: lf.entry_point,
                body_length: lf.body_length,
                is_closure: lf.is_closure,
                captures_count: lf.captures_count,
                is_async: lf.is_async,
                ref_params: lf.ref_params.clone(),
                ref_mutates: lf.ref_mutates.clone(),
                mutable_captures: lf.mutable_captures.clone(),
                frame_descriptor: lf.frame_descriptor.clone(),
                osr_entry_points: Vec::new(),
                mir_data: None,
            })
            .collect();

        let program = BytecodeProgram {
            instructions: linked.instructions,
            constants: linked.constants,
            strings: linked.strings,
            functions,
            debug_info: linked.debug_info,
            data_schema: linked.data_schema,
            module_binding_names: linked.module_binding_names,
            top_level_locals_count: linked.top_level_locals_count,
            top_level_local_storage_hints: linked.top_level_local_storage_hints,
            type_schema_registry: linked.type_schema_registry,
            module_binding_storage_hints: linked.module_binding_storage_hints,
            function_local_storage_hints: linked.function_local_storage_hints,
            trait_method_symbols: linked.trait_method_symbols,
            foreign_functions: linked.foreign_functions,
            native_struct_layouts: linked.native_struct_layouts,
            function_blob_hashes: entry_points
                .iter()
                .enumerate()
                .map(|(idx, _)| hashes.get(idx).copied().flatten())
                .collect(),
            ..BytecodeProgram::default()
        };

        // Load the program normally (handles schema resolution, function name index, etc.)
        self.load_program(program);

        // Override the content-addressed metadata with the linked data
        // (load_program calls populate_content_addressed_metadata which won't find
        // content_addressed since we didn't set it — override here)
        self.function_hashes = hashes;
        self.function_hash_raw = self
            .function_hashes
            .iter()
            .map(|opt| opt.map(|fh| fh.0))
            .collect();
        self.function_id_by_hash.clear();
        for (idx, maybe_hash) in self.function_hashes.iter().enumerate() {
            if let Some(hash) = maybe_hash {
                self.function_id_by_hash.entry(*hash).or_insert(idx as u16);
            }
        }
        self.function_entry_points = entry_points;
        self.program_entry_ip = entry_ip;
        self.reset();
    }

    /// Hot-patch a single function in the loaded program with a new blob.
    ///
    /// The new blob's instructions, constants, and strings replace the existing
    /// function's bytecode in-place. The function's metadata (arity, param names,
    /// locals count, etc.) is also updated. The content hash is recorded so that
    /// in-flight frames referencing the old hash remain valid (they execute from
    /// their saved IP which is now stale, but callers that resolve by function ID
    /// will pick up the new code on the next call).
    ///
    /// Returns `Ok(old_hash)` on success (the previous content hash, if any),
    /// or `Err(msg)` if the function ID is out of range.
    pub fn patch_function(
        &mut self,
        fn_id: u16,
        new_blob: FunctionBlob,
    ) -> Result<Option<FunctionHash>, String> {
        let idx = fn_id as usize;

        if idx >= self.program.functions.len() {
            return Err(format!(
                "patch_function: fn_id {} out of range (program has {} functions)",
                fn_id,
                self.program.functions.len()
            ));
        }

        // Capture the old hash before overwriting.
        let old_hash = self.function_hashes.get(idx).copied().flatten();

        let func = &mut self.program.functions[idx];
        let old_entry = func.entry_point;

        // Compute instruction splice range: from this function's entry point
        // to the next function's entry point (or end of instructions).
        let next_entry = self
            .program
            .functions
            .get(idx + 1)
            .map(|f| f.entry_point)
            .unwrap_or(self.program.instructions.len());

        let old_len = next_entry - old_entry;
        let new_len = new_blob.instructions.len();

        // Splice instructions.
        self.program.instructions.splice(
            old_entry..old_entry + old_len,
            new_blob.instructions.iter().cloned(),
        );

        // If the new function has a different instruction count, shift all
        // subsequent function entry points.
        if new_len != old_len {
            let delta = new_len as isize - old_len as isize;
            for subsequent in self.program.functions.iter_mut().skip(idx + 1) {
                subsequent.entry_point = (subsequent.entry_point as isize + delta) as usize;
            }
            // Also update function_entry_points mirror.
            for ep in self.function_entry_points.iter_mut().skip(idx + 1) {
                *ep = (*ep as isize + delta) as usize;
            }
        }

        // Append new constants and strings to the program pools.
        // The blob's Operand indices reference its local pools, so we need to
        // remap them to the global pool offsets.
        let const_offset = self.program.constants.len();
        let string_offset = self.program.strings.len();
        self.program
            .constants
            .extend(new_blob.constants.iter().cloned());
        self.program
            .strings
            .extend(new_blob.strings.iter().cloned());

        // Remap operands in the spliced instructions to use global pool offsets.
        let instr_slice = &mut self.program.instructions[old_entry..old_entry + new_len];
        for instr in instr_slice.iter_mut() {
            remap_operand(&mut instr.operand, const_offset, string_offset);
        }

        // Update function metadata.
        let func = &mut self.program.functions[idx];
        func.name = new_blob.name;
        func.arity = new_blob.arity;
        func.param_names = new_blob.param_names;
        func.locals_count = new_blob.locals_count;
        func.is_closure = new_blob.is_closure;
        func.captures_count = new_blob.captures_count;
        func.is_async = new_blob.is_async;
        func.ref_params = new_blob.ref_params;
        func.ref_mutates = new_blob.ref_mutates;
        func.mutable_captures = new_blob.mutable_captures;

        // Update content hash metadata.
        let new_hash = new_blob.content_hash;
        if idx < self.function_hashes.len() {
            self.function_hashes[idx] = Some(new_hash);
        }
        if idx < self.function_hash_raw.len() {
            self.function_hash_raw[idx] = Some(new_hash.0);
        }
        self.function_id_by_hash.entry(new_hash).or_insert(fn_id);

        // Update function_entry_points for this function.
        if idx < self.function_entry_points.len() {
            self.function_entry_points[idx] = old_entry;
        }

        // Rebuild function name index so UFCS dispatch picks up renames.
        self.rebuild_function_name_index();

        Ok(old_hash)
    }

    /// Load a content-addressed `Program` with permission checking.
    ///
    /// Links the program, checks that `total_required_permissions` is a subset of
    /// `granted`, and loads normally if the check passes. Returns an error listing
    /// the missing permissions if the check fails.
    pub fn load_program_with_permissions(
        &mut self,
        program: crate::bytecode::Program,
        granted: &shape_abi_v1::PermissionSet,
    ) -> Result<(), PermissionError> {
        let linked =
            crate::linker::link(&program).map_err(|e| PermissionError::LinkError(e.to_string()))?;
        if !linked.total_required_permissions.is_subset(granted) {
            let missing = linked.total_required_permissions.difference(granted);
            return Err(PermissionError::InsufficientPermissions {
                required: linked.total_required_permissions.clone(),
                granted: granted.clone(),
                missing,
            });
        }
        self.load_linked_program(linked);
        Ok(())
    }

    /// Load a `LinkedProgram` with permission checking.
    ///
    /// Checks that `total_required_permissions` is a subset of `granted`, then
    /// loads normally. Returns an error listing the missing permissions if the
    /// check fails.
    pub fn load_linked_program_with_permissions(
        &mut self,
        linked: crate::bytecode::LinkedProgram,
        granted: &shape_abi_v1::PermissionSet,
    ) -> Result<(), PermissionError> {
        if !linked.total_required_permissions.is_subset(granted) {
            let missing = linked.total_required_permissions.difference(granted);
            return Err(PermissionError::InsufficientPermissions {
                required: linked.total_required_permissions.clone(),
                granted: granted.clone(),
                missing,
            });
        }
        self.load_linked_program(linked);
        Ok(())
    }

    /// Populate `function_hashes` and `function_entry_points` from the loaded program.
    ///
    /// If the program was compiled with content-addressed metadata (`content_addressed`
    /// is `Some`), we extract blob hashes by matching function names/entry points.
    /// Otherwise both vectors remain empty and `CallFrame::blob_hash` will be `None`.
    pub(crate) fn populate_content_addressed_metadata(&mut self) {
        let func_count = self.program.functions.len();
        self.function_entry_points = self
            .program
            .functions
            .iter()
            .map(|f| f.entry_point)
            .collect();

        if self.program.function_blob_hashes.len() == func_count {
            self.function_hashes = self.program.function_blob_hashes.clone();
        } else if let Some(ref ca_program) = self.program.content_addressed {
            // Build a lookup from function name -> blob hash from the Program's function_store
            let mut name_to_hash: HashMap<String, FunctionHash> =
                HashMap::with_capacity(ca_program.function_store.len());
            for (hash, blob) in &ca_program.function_store {
                name_to_hash.insert(blob.name.clone(), *hash);
            }

            self.function_hashes = Vec::with_capacity(func_count);
            for func in &self.program.functions {
                self.function_hashes
                    .push(name_to_hash.get(&func.name).copied());
            }
        } else {
            self.function_hashes = vec![None; func_count];
        }

        // Build the raw byte mirror for ModuleContext.
        self.function_hash_raw = self
            .function_hashes
            .iter()
            .map(|opt| opt.map(|fh| fh.0))
            .collect();
        self.function_id_by_hash.clear();
        for (idx, maybe_hash) in self.function_hashes.iter().enumerate() {
            if let Some(hash) = maybe_hash {
                self.function_id_by_hash.entry(*hash).or_insert(idx as u16);
            }
        }
    }

    /// Build the function name → index map for runtime UFCS dispatch.
    /// Called after program load or merge to enable type-scoped method resolution
    /// (e.g., "DbTable::filter" looked up when calling .filter() on an Object with __type "DbTable").
    pub(crate) fn rebuild_function_name_index(&mut self) {
        self.function_name_index.clear();
        for (i, func) in self.program.functions.iter().enumerate() {
            self.function_name_index.insert(func.name.clone(), i as u16);
        }
    }

    /// Reset VM state
    /// Get a snapshot of all module binding values.
    pub fn module_bindings_snapshot(&self) -> Vec<ValueWord> {
        (0..self.module_bindings.len())
            .map(|i| self.binding_read_raw(i))
            .collect()
    }

    /// Reset VM execution state for trampoline use.
    /// Clears stack, call frames, error state, and exception handlers but
    /// preserves the loaded program, module bindings, module_fn_table,
    /// and registered extensions.
    pub fn reset_for_trampoline(&mut self) {
        for i in 0..self.sp {
            drop(ValueWord::from_raw_bits(self.stack[i]));
            self.stack[i] = Self::NONE_BITS;
        }
        self.sp = 0;
        self.ip = 0;
        self.call_stack.clear();
        self.loop_stack.clear();
        self.timeframe_stack.clear();
        self.exception_handlers.clear();
        self.instruction_count = 0;
        self.last_error_line = None;
        self.last_error_file = None;
        self.last_uncaught_exception = None;
        self.module_init_done = true; // Skip re-init on next call
    }

    pub fn reset(&mut self) {
        self.ip = self.program_entry_ip;
        for i in 0..self.sp {
            drop(ValueWord::from_raw_bits(self.stack[i]));
            self.stack[i] = Self::NONE_BITS;
        }
        // Advance sp past top-level locals so expression evaluation
        // doesn't overlap with local variable storage in register windows.
        let tl = self.program.top_level_locals_count as usize;
        self.sp = tl;
        self.call_stack.clear();
        self.loop_stack.clear();
        self.timeframe_stack.clear();
        self.exception_handlers.clear();
        self.instruction_count = 0;
        self.last_error_line = None;
        self.last_error_file = None;
        self.last_uncaught_exception = None;
    }

    /// Reset stack only (for reusing compiled program across iterations)
    /// Keeps program, module_bindings, and GC state intact - only clears execution state
    pub fn reset_stack(&mut self) {
        self.ip = self.program_entry_ip;
        for i in 0..self.sp {
            drop(ValueWord::from_raw_bits(self.stack[i]));
            self.stack[i] = Self::NONE_BITS;
        }
        let tl = self.program.top_level_locals_count as usize;
        self.sp = tl;
        self.call_stack.clear();
        self.loop_stack.clear();
        self.timeframe_stack.clear();
        self.exception_handlers.clear();
        self.last_error_line = None;
        self.last_error_file = None;
        self.last_uncaught_exception = None;
    }

    /// Minimal reset for hot loops - only clears essential state
    /// Use this when you know the function doesn't create GC objects or use exceptions
    #[inline]
    pub fn reset_minimal(&mut self) {
        self.ip = self.program_entry_ip;
        for i in 0..self.sp {
            drop(ValueWord::from_raw_bits(self.stack[i]));
            self.stack[i] = Self::NONE_BITS;
        }
        let tl = self.program.top_level_locals_count as usize;
        self.sp = tl;
        self.call_stack.clear();
        self.last_error_line = None;
        self.last_error_file = None;
        self.last_uncaught_exception = None;
    }

    /// Push a value onto the stack (public, for testing and host integration)
    pub fn push_value(&mut self, value: ValueWord) {
        if self.sp >= self.stack.len() {
            self.stack.resize_with(self.sp * 2 + 1, || Self::NONE_BITS);
        }
        self.stack_write_raw(self.sp, value);
        self.sp += 1;
    }
}
