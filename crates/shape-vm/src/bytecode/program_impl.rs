use super::*;

impl BytecodeProgram {
    fn trait_method_symbol_key(
        trait_name: &str,
        type_name: &str,
        impl_name: Option<&str>,
        method_name: &str,
    ) -> String {
        format!(
            "{}::{}::{}::{}",
            trait_name,
            type_name,
            impl_name.unwrap_or(DEFAULT_TRAIT_IMPL_SELECTOR),
            method_name
        )
    }

    /// Create a new empty program
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a constant to the pool and return its index
    pub fn add_constant(&mut self, constant: Constant) -> u16 {
        // Check if constant already exists
        if let Some(idx) = self.constants.iter().position(|c| c == &constant) {
            return idx as u16;
        }

        let idx = self.constants.len() as u16;
        self.constants.push(constant);
        idx
    }

    /// Add a string to the pool and return its index.
    /// Uses the HashMap index for O(1) dedup.
    pub fn add_string(&mut self, string: String) -> u16 {
        self.ensure_string_index();
        if let Some(&idx) = self.string_index.get(&string) {
            return idx as u16;
        }
        let idx = self.strings.len() as u32;
        self.string_index.insert(string.clone(), idx);
        self.strings.push(string);
        idx as u16
    }

    /// Intern a string and return a `StringId` for use in `Operand::Name` /
    /// `Operand::MethodCall`. Uses the HashMap index for O(1) dedup.
    pub fn intern_string(&mut self, s: &str) -> StringId {
        self.ensure_string_index();
        if let Some(&idx) = self.string_index.get(s) {
            return StringId(idx);
        }
        let idx = self.strings.len() as u32;
        self.string_index.insert(s.to_owned(), idx);
        self.strings.push(s.to_owned());
        StringId(idx)
    }

    /// Ensure the string_index HashMap is populated.
    /// After deserialization, string_index is empty — rebuild it from the strings Vec.
    pub fn ensure_string_index(&mut self) {
        if self.string_index.is_empty() && !self.strings.is_empty() {
            for (i, s) in self.strings.iter().enumerate() {
                self.string_index.insert(s.clone(), i as u32);
            }
        }
    }

    /// Resolve a `StringId` back to a `&str`.
    ///
    /// Panics if the id is out of bounds (indicates a compiler bug).
    pub fn resolve_string(&self, id: StringId) -> &str {
        &self.strings[id.0 as usize]
    }

    /// Add an instruction to the program
    pub fn emit(&mut self, instruction: Instruction) -> usize {
        let idx = self.instructions.len();
        self.instructions.push(instruction);
        idx
    }

    /// Get the current instruction pointer
    pub fn current_offset(&self) -> usize {
        self.instructions.len()
    }

    /// Register a trait-method dispatch symbol.
    pub fn register_trait_method_symbol(
        &mut self,
        trait_name: &str,
        type_name: &str,
        impl_name: Option<&str>,
        method_name: &str,
        function_name: &str,
    ) {
        let key = Self::trait_method_symbol_key(trait_name, type_name, impl_name, method_name);
        self.trait_method_symbols
            .insert(key, function_name.to_string());
    }

    /// Resolve a trait-method dispatch symbol name.
    pub fn lookup_trait_method_symbol(
        &self,
        trait_name: &str,
        type_name: &str,
        impl_name: Option<&str>,
        method_name: &str,
    ) -> Option<&str> {
        let key = Self::trait_method_symbol_key(trait_name, type_name, impl_name, method_name);
        self.trait_method_symbols.get(&key).map(|s| s.as_str())
    }

    /// List named impl selectors for a trait method on a specific type.
    ///
    /// Excludes the default selector (`__default__`) and returns stable sorted names.
    pub fn named_trait_impls_for_method(
        &self,
        trait_name: &str,
        type_name: &str,
        method_name: &str,
    ) -> Vec<String> {
        let prefix = format!("{}::{}::", trait_name, type_name);
        let suffix = format!("::{}", method_name);
        let mut names = std::collections::BTreeSet::new();

        for key in self.trait_method_symbols.keys() {
            if !key.starts_with(&prefix) || !key.ends_with(&suffix) {
                continue;
            }
            let middle = &key[prefix.len()..key.len() - suffix.len()];
            if middle != DEFAULT_TRAIT_IMPL_SELECTOR && !middle.is_empty() {
                names.insert(middle.to_string());
            }
        }

        names.into_iter().collect()
    }

    /// Find the default trait impl function for a given type and method (any trait).
    ///
    /// Searches `trait_method_symbols` for any entry whose type and method match,
    /// preferring the default selector (`__default__`). Returns the compiled function
    /// symbol name if found.
    ///
    /// This is used by method call dispatch to detect when a trait impl method exists
    /// for the receiver type, so that builtin functions with the same name don't shadow it.
    pub fn find_default_trait_impl_for_type_method(
        &self,
        type_name: &str,
        method_name: &str,
    ) -> Option<&str> {
        // Pattern: "Trait::Type::Selector::method"
        // We look for any key ending with "::Type::__default__::method"
        let default_suffix = format!(
            "::{}::{}::{}",
            type_name, DEFAULT_TRAIT_IMPL_SELECTOR, method_name
        );
        for (key, func_name) in &self.trait_method_symbols {
            if key.ends_with(&default_suffix) {
                return Some(func_name.as_str());
            }
        }
        // Fall back to any named impl (first match)
        let suffix = format!("::{}", method_name);
        let type_segment = format!("::{}::", type_name);
        for (key, func_name) in &self.trait_method_symbols {
            if key.contains(&type_segment) && key.ends_with(&suffix) {
                return Some(func_name.as_str());
            }
        }
        None
    }

    /// Append another program's bytecode to this one.
    ///
    /// Used internally for stdlib compilation where each module is compiled
    /// separately and then combined. Unlike the old `merge_prepend`, this
    /// appends `other` after `self`, so only `other`'s indices are adjusted.
    pub(crate) fn merge_append(&mut self, other: BytecodeProgram) {
        let mut other = other;

        // Remove trailing Halt from self (we'll have the appended one or add our own)
        if self.instructions.last().map(|i| i.opcode) == Some(OpCode::Halt) {
            self.instructions.pop();
        }

        // Offsets for adjusting other's references
        let const_offset = self.constants.len() as u16;
        let string_offset = self.strings.len() as u16;
        let func_offset = self.functions.len() as u16;
        let foreign_offset = self.foreign_functions.len() as u16;
        let instr_offset = self.instructions.len();

        // Adjust other's constants
        for constant in &mut other.constants {
            if let Constant::Function(id) = constant {
                *id += func_offset;
            }
        }

        // Adjust other's function entry points
        for func in &mut other.functions {
            func.entry_point += instr_offset;
        }

        // Adjust other's instruction operands
        for instr in &mut other.instructions {
            if let Some(ref mut operand) = instr.operand {
                match operand {
                    Operand::Const(idx) => *idx += const_offset,
                    Operand::Property(idx) => *idx += string_offset,
                    Operand::Function(idx) => idx.0 += func_offset,
                    Operand::Name(id) => id.0 += string_offset as u32,
                    Operand::MethodCall { name, .. } => name.0 += string_offset as u32,
                    Operand::TypedMethodCall { string_id, .. } => {
                        *string_id += string_offset;
                    }
                    Operand::ForeignFunction(idx) => *idx += foreign_offset,
                    Operand::Offset(_)
                    | Operand::Local(_)
                    | Operand::ModuleBinding(_)
                    | Operand::Count(_)
                    | Operand::Builtin(_)
                    | Operand::ColumnIndex(_)
                    | Operand::TypedField { .. }
                    | Operand::TypedObjectAlloc { .. }
                    | Operand::TypedMerge { .. }
                    | Operand::ColumnAccess { .. }
                    | Operand::MatrixDims { .. }
                    | Operand::Width(_)
                    | Operand::TypedLocal(_, _) => {}
                }
            }
        }

        // Append constants, strings, functions, instructions
        self.constants.append(&mut other.constants);
        self.strings.append(&mut other.strings);
        // Rebuild string index after merge
        self.string_index.clear();
        self.functions.append(&mut other.functions);
        self.instructions.append(&mut other.instructions);

        // Merge function local storage hints
        self.function_local_storage_hints
            .append(&mut other.function_local_storage_hints);

        // Merge module binding names (dedup by name)
        for name in other.module_binding_names {
            if !self.module_binding_names.contains(&name) {
                self.module_binding_names.push(name);
            }
        }

        // Merge type schema registry
        self.type_schema_registry.merge(other.type_schema_registry);

        // Merge trait method symbols (self wins on collision)
        for (key, value) in other.trait_method_symbols {
            self.trait_method_symbols.entry(key).or_insert(value);
        }

        // Merge expanded function definitions (self wins on collision)
        for (key, value) in other.expanded_function_defs {
            self.expanded_function_defs.entry(key).or_insert(value);
        }

        // Merge foreign functions
        self.foreign_functions.append(&mut other.foreign_functions);

        // Merge native struct layouts (dedup by name, self wins)
        for layout in other.native_struct_layouts {
            if !self.native_struct_layouts.iter().any(|l| l.name == layout.name) {
                self.native_struct_layouts.push(layout);
            }
        }
    }
}
