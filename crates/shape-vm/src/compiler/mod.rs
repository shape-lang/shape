//! Bytecode compiler - translates AST to bytecode

use shape_ast::error::{Result, ShapeError, SourceLocation};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::blob_cache_v2::BlobCache;
use crate::borrow_checker::BorrowMode;
use crate::bytecode::{
    BytecodeProgram, Constant, FunctionBlob, FunctionHash, Instruction, OpCode,
    Program as ContentAddressedProgram,
};
use crate::type_tracking::{TypeTracker, VariableTypeInfo};
use shape_ast::ast::{FunctionDef, Program, TypeAnnotation};
use shape_runtime::type_schema::SchemaId;
use shape_runtime::type_system::{
    Type, TypeAnalysisMode, TypeError, TypeErrorWithLocation, analyze_program_with_mode,
    checking::MethodTable,
};

// Sub-modules
pub(crate) mod comptime;
pub(crate) mod comptime_builtins;
pub(crate) mod comptime_target;
mod control_flow;
mod expressions;
mod functions;
mod helpers;
mod literals;
mod loops;
mod patterns;
mod statements;
pub mod string_interpolation;

/// Loop compilation context
pub(crate) struct LoopContext {
    /// Break jump targets
    pub(crate) break_jumps: Vec<usize>,
    /// Continue jump target
    pub(crate) continue_target: usize,
    /// Optional local to store break values for expression loops
    pub(crate) break_value_local: Option<u16>,
    /// Whether a for-in iterator is on the stack (break must pop it)
    pub(crate) iterator_on_stack: bool,
    /// Drop scope depth when the loop was entered (for break/continue early exit drops)
    pub(crate) drop_scope_depth: usize,
}

/// Information about an imported symbol (fields used for diagnostics/LSP)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ImportedSymbol {
    /// Original name in the source module
    pub original_name: String,
    /// Module path the symbol was imported from
    pub module_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StructGenericInfo {
    pub type_params: Vec<shape_ast::ast::TypeParam>,
    pub runtime_field_types: HashMap<String, shape_ast::ast::TypeAnnotation>,
}

/// Whether a type's Drop impl is sync-only, async-only, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropKind {
    SyncOnly,
    AsyncOnly,
    Both,
}

/// Canonical compile-time parameter passing contract.
///
/// This is the single source of truth used by compiler lowering and LSP rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamPassMode {
    ByValue,
    ByRefShared,
    ByRefExclusive,
}

impl ParamPassMode {
    pub const fn is_reference(self) -> bool {
        !matches!(self, Self::ByValue)
    }

    pub const fn is_exclusive(self) -> bool {
        matches!(self, Self::ByRefExclusive)
    }
}

/// Per-function blob builder for content-addressed compilation.
///
/// Uses a **snapshot** strategy: records the global instruction/constant/string
/// pool sizes at the start of function compilation, then at finalization
/// extracts the delta and remaps global indices to blob-local indices.
pub(crate) struct FunctionBlobBuilder {
    /// Function name.
    pub name: String,
    /// Global instruction index where this function's code starts.
    pub instr_start: usize,
    /// Global constant pool size when this function started compiling.
    #[allow(dead_code)]
    pub const_start: usize,
    /// Global string pool size when this function started compiling.
    #[allow(dead_code)]
    pub string_start: usize,
    /// Names of functions called by this function (for dependency tracking).
    pub called_functions: Vec<String>,
    /// Type schema names this function constructs.
    pub type_schemas: Vec<String>,
    /// Accumulated permissions required by this function's direct calls.
    pub required_permissions: shape_abi_v1::PermissionSet,
}

impl FunctionBlobBuilder {
    pub fn new(name: String, instr_start: usize, const_start: usize, string_start: usize) -> Self {
        Self {
            name,
            instr_start,
            const_start,
            string_start,
            called_functions: Vec::new(),
            type_schemas: Vec::new(),
            required_permissions: shape_abi_v1::PermissionSet::pure(),
        }
    }

    /// Record that this function calls another function by name.
    pub fn record_call(&mut self, callee_name: &str) {
        if !self.called_functions.iter().any(|n| n == callee_name) {
            self.called_functions.push(callee_name.to_owned());
        }
    }

    /// Record that this function requires the given permissions
    /// (e.g., from a stdlib module call identified by capability_tags).
    pub fn record_permissions(&mut self, perms: &shape_abi_v1::PermissionSet) {
        self.required_permissions = self.required_permissions.union(perms);
    }

    /// Finalize this builder into a FunctionBlob by extracting the delta from
    /// the global program pools and remapping indices to blob-local ones.
    pub fn finalize(
        &self,
        program: &crate::bytecode::BytecodeProgram,
        func: &crate::bytecode::Function,
        blob_name_to_hash: &HashMap<String, FunctionHash>,
        instr_end: usize,
    ) -> FunctionBlob {
        use crate::bytecode::Operand;

        // Extract global-indexed instructions for this function.
        let global_instructions = &program.instructions[self.instr_start..instr_end];

        // Build constant remap: global index -> local index.
        let mut const_remap: HashMap<u16, u16> = HashMap::new();
        let mut local_constants: Vec<Constant> = Vec::new();
        // Build string remap similarly.
        let mut string_remap: HashMap<u16, u16> = HashMap::new();
        let mut local_strings: Vec<String> = Vec::new();
        // Build function operand remap: global function index -> dependency-local index.
        let mut func_remap: HashMap<u16, u16> = HashMap::new();
        // Start from explicitly recorded call dependencies, then augment with
        // function-value references found in constants/operands.
        let mut called_functions = self.called_functions.clone();

        let mut ensure_called = |callee_name: &str| -> u16 {
            if let Some(dep_idx) = called_functions.iter().position(|n| n == callee_name) {
                dep_idx as u16
            } else {
                called_functions.push(callee_name.to_owned());
                (called_functions.len() - 1) as u16
            }
        };

        // Scan instructions for all constant/string references and build
        // blob-local pools with remapped indices.
        for instr in global_instructions {
            if let Some(ref operand) = instr.operand {
                match operand {
                    Operand::Const(idx) => {
                        if !const_remap.contains_key(idx) {
                            let local_idx = local_constants.len() as u16;
                            const_remap.insert(*idx, local_idx);
                            let mut constant = program.constants[*idx as usize].clone();
                            if let Constant::Function(fid) = constant {
                                let global_idx = fid as usize;
                                if let Some(callee) = program.functions.get(global_idx) {
                                    let dep_idx = ensure_called(&callee.name);
                                    constant = Constant::Function(dep_idx);
                                }
                            }
                            local_constants.push(constant);
                        }
                    }
                    Operand::Property(idx) => {
                        if !string_remap.contains_key(idx) {
                            let local_idx = local_strings.len() as u16;
                            string_remap.insert(*idx, local_idx);
                            local_strings.push(program.strings[*idx as usize].clone());
                        }
                    }
                    Operand::Name(sid) => {
                        let gidx = sid.0 as u16;
                        if !string_remap.contains_key(&gidx) {
                            let local_idx = local_strings.len() as u16;
                            string_remap.insert(gidx, local_idx);
                            local_strings.push(program.strings[gidx as usize].clone());
                        }
                    }
                    Operand::MethodCall { name, .. } => {
                        let gidx = name.0 as u16;
                        if !string_remap.contains_key(&gidx) {
                            let local_idx = local_strings.len() as u16;
                            string_remap.insert(gidx, local_idx);
                            local_strings.push(program.strings[gidx as usize].clone());
                        }
                    }
                    Operand::TypedMethodCall { string_id, .. } => {
                        let gidx = *string_id;
                        if !string_remap.contains_key(&gidx) {
                            let local_idx = local_strings.len() as u16;
                            string_remap.insert(gidx, local_idx);
                            local_strings.push(program.strings[gidx as usize].clone());
                        }
                    }
                    Operand::Function(fid) => {
                        let global_idx = fid.0 as usize;
                        if !func_remap.contains_key(&fid.0) {
                            // Map global function index -> dependency-local index.
                            // If this call target was not explicitly recorded (e.g. emitted via
                            // function-valued constants), add it so content-addressed linking can
                            // remap stable function IDs correctly.
                            if let Some(callee) = program.functions.get(global_idx) {
                                let dep_idx = ensure_called(&callee.name);
                                func_remap.insert(fid.0, dep_idx);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Remap instructions to use local indices.
        let local_instructions: Vec<Instruction> = global_instructions
            .iter()
            .map(|instr| {
                let mut remapped = instr.clone();
                if let Some(operand) = &mut remapped.operand {
                    match operand {
                        Operand::Const(idx) => {
                            if let Some(&local) = const_remap.get(idx) {
                                *idx = local;
                            }
                        }
                        Operand::Property(idx) => {
                            if let Some(&local) = string_remap.get(idx) {
                                *idx = local;
                            }
                        }
                        Operand::Name(sid) => {
                            if let Some(&local) = string_remap.get(&(sid.0 as u16)) {
                                sid.0 = local as u32;
                            }
                        }
                        Operand::MethodCall { name, arg_count: _ } => {
                            if let Some(&local) = string_remap.get(&(name.0 as u16)) {
                                name.0 = local as u32;
                            }
                        }
                        Operand::TypedMethodCall { string_id, .. } => {
                            if let Some(&local) = string_remap.get(string_id) {
                                *string_id = local;
                            }
                        }
                        Operand::Function(fid) => {
                            if let Some(&local) = func_remap.get(&fid.0) {
                                fid.0 = local;
                            }
                        }
                        _ => {}
                    }
                }
                remapped
            })
            .collect();

        // Build dependency list from called function names.
        // Use FunctionHash::ZERO as sentinel for forward references (not yet compiled).
        let dependencies: Vec<FunctionHash> = called_functions
            .iter()
            .map(|callee| {
                blob_name_to_hash
                    .get(callee)
                    .copied()
                    .unwrap_or(FunctionHash::ZERO)
            })
            .collect();

        // Build source map from global debug info.
        let source_map: Vec<(usize, u32, u32)> = program
            .debug_info
            .line_numbers
            .iter()
            .filter(|(idx, _, _)| *idx >= self.instr_start && *idx < instr_end)
            .map(|(idx, fid, line)| (idx - self.instr_start, *fid as u32, *line))
            .collect();

        // Scan instructions for CallForeign operands and collect content hashes
        // from the program's foreign_functions table.
        let mut foreign_deps: Vec<[u8; 32]> = Vec::new();
        for instr in &local_instructions {
            if instr.opcode == crate::bytecode::OpCode::CallForeign {
                if let Some(Operand::ForeignFunction(idx)) = instr.operand {
                    if let Some(entry) = program.foreign_functions.get(idx as usize) {
                        if let Some(hash) = entry.content_hash {
                            foreign_deps.push(hash);
                        }
                    }
                }
            }
        }
        foreign_deps.sort();
        foreign_deps.dedup();

        let mut blob = FunctionBlob {
            content_hash: FunctionHash::ZERO,
            name: self.name.clone(),
            arity: func.arity,
            param_names: func.param_names.clone(),
            locals_count: func.locals_count,
            is_closure: func.is_closure,
            captures_count: func.captures_count,
            is_async: func.is_async,
            ref_params: func.ref_params.clone(),
            ref_mutates: func.ref_mutates.clone(),
            mutable_captures: func.mutable_captures.clone(),
            frame_descriptor: func.frame_descriptor.clone(),
            required_permissions: self.required_permissions.clone(),
            instructions: local_instructions,
            constants: local_constants,
            strings: local_strings,
            dependencies,
            callee_names: called_functions,
            type_schemas: self.type_schemas.clone(),
            foreign_dependencies: foreign_deps,
            source_map,
        };
        blob.finalize();
        blob
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeDiagnosticMode {
    ReliableOnly,
    Strict,
    RecoverAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileDiagnosticMode {
    FailFast,
    RecoverAll,
}

/// Compiler state
pub struct BytecodeCompiler {
    /// The program being built
    pub(crate) program: BytecodeProgram,

    /// Current function being compiled
    pub(crate) current_function: Option<usize>,

    /// Local variable mappings (name -> index)
    pub(crate) locals: Vec<HashMap<String, u16>>,

    /// ModuleBinding variable mappings (name -> index)
    pub(crate) module_bindings: HashMap<String, u16>,

    /// Next local variable index
    pub(crate) next_local: u16,

    /// Next module_binding variable index
    pub(crate) next_global: u16,

    /// Loop context stack for break/continue
    pub(crate) loop_stack: Vec<LoopContext>,

    /// Counter for synthetic closure function names
    pub(crate) closure_counter: u64,

    /// When compiling a DataTable closure method (e.g. dt.filter(row => ...)),
    /// this holds the (schema_id, type_name) to tag the closure's row parameter as RowView.
    pub(crate) closure_row_schema: Option<(u32, String)>,

    /// Unified type metadata for the last compiled expression.
    ///
    /// This is the single source for relational/value kind propagation
    /// (Table<T>, Indexed<T>, known object schema, etc.).
    pub(crate) last_expr_type_info: Option<VariableTypeInfo>,

    /// Type tracker for optimized field access
    pub(crate) type_tracker: TypeTracker,

    /// Schema ID of the last compiled expression (if it's a TypedObject).
    /// Used for compile-time typed merge optimization.
    pub(crate) last_expr_schema: Option<SchemaId>,

    /// Numeric type of the last compiled expression (for typed opcode emission).
    /// Set by literal compilation, variable loads, and other expression compilers.
    /// Read by binary op compilation to emit typed opcodes (e.g., MulInt).
    pub(crate) last_expr_numeric_type: Option<crate::type_tracking::NumericType>,

    /// Type inference engine for match exhaustiveness and type checking
    pub(crate) type_inference: shape_runtime::type_system::inference::TypeInferenceEngine,

    /// Track type aliases defined in the program
    /// Maps alias name -> target type (for type validation)
    pub(crate) type_aliases: HashMap<String, String>,

    /// Current source line being compiled (for debug info)
    pub(crate) current_line: u32,

    /// Current source file ID (for multi-file debug info)
    pub(crate) current_file_id: u16,

    /// Source text (for error messages)
    pub(crate) source_text: Option<String>,

    /// Source lines (split from source_text for quick access)
    pub(crate) source_lines: Vec<String>,

    /// Imported symbols: local_name -> ImportedSymbol
    pub(crate) imported_names: HashMap<String, ImportedSymbol>,
    /// Module namespace bindings introduced by `use module.path`.
    /// Used to avoid UFCS rewrites for module calls like `duckdb.connect(...)`.
    pub(crate) module_namespace_bindings: HashSet<String>,
    /// Active lexical module scope stack while compiling `mod Name { ... }`.
    pub(crate) module_scope_stack: Vec<String>,

    /// Known exports for import suggestions: function_name -> module_path
    /// Used to provide helpful error messages like "Did you mean to import from...?"
    pub(crate) known_exports: HashMap<String, String>,
    /// Function arity bounds keyed by function name: (required_params, total_params).
    /// Required params are non-default parameters. Defaults are only allowed
    /// in trailing positions.
    pub(crate) function_arity_bounds: HashMap<String, (usize, usize)>,
    /// Function const parameter indices keyed by function name.
    /// Const parameters must receive compile-time constant arguments at call sites.
    pub(crate) function_const_params: HashMap<String, Vec<usize>>,
    /// Original function definitions keyed by function name.
    /// Used for const-template specialization at call sites.
    pub(crate) function_defs: HashMap<String, FunctionDef>,
    /// Foreign function definitions keyed by function name.
    /// Used to resolve the effective (Result-wrapped) return type at call sites.
    pub(crate) foreign_function_defs: HashMap<String, shape_ast::ast::ForeignFunctionDef>,
    /// Cached const specializations keyed by `(base_name + const-arg fingerprint)`.
    pub(crate) const_specializations: HashMap<String, usize>,
    /// Monotonic counter for unique specialization symbol names.
    pub(crate) next_const_specialization_id: u64,
    /// Const-parameter bindings for specialized function symbols.
    /// These bindings are exposed to comptime handlers as typed module_bindings.
    pub(crate) specialization_const_bindings:
        HashMap<String, Vec<(String, shape_value::ValueWord)>>,

    /// Struct type definitions: type_name -> (field_names in order, definition span)
    pub(crate) struct_types: HashMap<String, (Vec<String>, shape_ast::ast::Span)>,
    /// Generic metadata for struct types used to instantiate runtime type names
    /// (e.g. `MyType<number>`) at struct-literal construction sites.
    pub(crate) struct_generic_info: HashMap<String, StructGenericInfo>,
    /// Names of `type C` declarations with native layout metadata.
    pub(crate) native_layout_types: HashSet<String>,
    /// Generated conversion pair cache keys: `c_type::object_type`.
    pub(crate) generated_native_conversion_pairs: HashSet<String>,

    /// Whether the current function being compiled is async
    pub(crate) current_function_is_async: bool,

    /// Directory of the source file being compiled (for resolving relative source paths)
    pub(crate) source_dir: Option<std::path::PathBuf>,

    /// Collected compilation errors (for multi-error reporting)
    pub(crate) errors: Vec<shape_ast::error::ShapeError>,

    /// Hoisted fields from optimistic hoisting pre-pass.
    /// Maps variable name → list of property names assigned later (e.g., a.y = 2 → "a" → ["y"]).
    /// Used to include future property assignments in inline object schemas at compile time.
    pub(crate) hoisted_fields: HashMap<String, Vec<String>>,

    /// When compiling a variable initializer, the name of the variable being assigned to.
    /// Used by compile_typed_object_literal to include hoisted fields in the schema.
    pub(crate) pending_variable_name: Option<String>,

    /// Known trait names (populated in the first pass so meta definitions can reference traits)
    pub(crate) known_traits: std::collections::HashSet<String>,

    /// Full trait definitions keyed by trait name.
    /// Used to install default method implementations for impl blocks that omit them.
    pub(crate) trait_defs: HashMap<String, shape_ast::ast::types::TraitDef>,

    /// Extension registry for comptime execution
    pub(crate) extension_registry: Option<Arc<Vec<shape_runtime::module_exports::ModuleExports>>>,

    /// Comptime field values per type: type_name -> (field_name -> ValueWord)
    /// These are type-level constants baked at compile time with zero runtime cost.
    pub(crate) comptime_fields: HashMap<String, HashMap<String, shape_value::ValueWord>>,
    /// Type diagnostic mode for shared analyzer diagnostics.
    pub(crate) type_diagnostic_mode: TypeDiagnosticMode,
    /// Expression compilation diagnostic mode.
    pub(crate) compile_diagnostic_mode: CompileDiagnosticMode,
    /// Whether this compiler instance is compiling code for comptime execution.
    /// Enables comptime-only builtins and comptime-specific statement semantics.
    pub(crate) comptime_mode: bool,
    /// Internal guard for compiler-synthesized `__comptime__` helper calls.
    /// User source must never access `__comptime__` directly.
    pub(crate) allow_internal_comptime_namespace: bool,
    /// Method table for data-driven method signature queries.
    /// Used to replace hardcoded heuristics (e.g., is_type_preserving_table_method)
    /// with MethodTable lookups (is_self_returning, takes_closure_with_receiver_param).
    pub(crate) method_table: MethodTable,
    /// Borrow checker for reference lifetime tracking.
    pub(crate) borrow_checker: crate::borrow_checker::BorrowChecker,
    /// Locals that are reference-typed in the current function.
    pub(crate) ref_locals: HashSet<u16>,
    /// Subset of ref_locals that hold exclusive (`&mut`) borrows.
    /// Used to enforce the three concurrency rules at task boundaries.
    pub(crate) exclusive_ref_locals: HashSet<u16>,
    /// Subset of ref_locals that were INFERRED as by-reference (not explicitly declared `&`).
    /// Inferred-ref params are owned values passed by reference for performance;
    /// closures may capture them (the value is dereferenced at capture time).
    pub(crate) inferred_ref_locals: HashSet<u16>,
    /// Local variable indices declared as `const` (immutable binding).
    pub(crate) const_locals: HashSet<u16>,
    /// Module binding indices declared as `const` (immutable binding).
    pub(crate) const_module_bindings: HashSet<u16>,
    /// Local variable indices declared as immutable `let` (not `let mut` or `var`).
    pub(crate) immutable_locals: HashSet<u16>,
    /// Local variable indices that are function parameters (first N locals in a function).
    /// Used to avoid trusting inferred type hints for params with no explicit annotation.
    pub(crate) param_locals: HashSet<u16>,
    /// Module binding indices declared as immutable `let`.
    pub(crate) immutable_module_bindings: HashSet<u16>,
    /// True while compiling function call arguments (allows `&` references).
    pub(crate) in_call_args: bool,
    /// Borrow mode for the argument currently being compiled.
    pub(crate) current_call_arg_borrow_mode: Option<BorrowMode>,
    /// ModuleBinding-ref writebacks collected while compiling current call args.
    pub(crate) call_arg_module_binding_ref_writebacks: Vec<Vec<(u16, u16)>>,
    /// Inferred reference parameters for untyped params: function -> per-param flag.
    pub(crate) inferred_ref_params: HashMap<String, Vec<bool>>,
    /// Inferred mutating-reference params: function -> per-param flag.
    pub(crate) inferred_ref_mutates: HashMap<String, Vec<bool>>,
    /// Effective per-parameter pass mode (explicit + inferred), by function name.
    pub(crate) inferred_param_pass_modes: HashMap<String, Vec<ParamPassMode>>,
    /// Inferred parameter type hints for unannotated params.
    /// Keyed by function name; each entry is a per-param optional type string.
    pub(crate) inferred_param_type_hints: HashMap<String, Vec<Option<String>>>,
    /// Stack of scopes, each containing locals that need Drop calls at scope exit.
    /// Each entry is (local_index, is_async).
    pub(crate) drop_locals: Vec<Vec<(u16, bool)>>,
    /// Per-type drop kind: tracks whether each type has sync, async, or both drop impls.
    /// Populated during the first-pass registration of impl blocks.
    pub(crate) drop_type_info: HashMap<String, DropKind>,
    /// Module bindings that need Drop calls at program exit.
    /// Each entry is (binding_index, is_async).
    pub(crate) drop_module_bindings: Vec<(u16, bool)>,
    /// Mutable closure captures in the current function being compiled.
    /// Maps captured variable name -> upvalue index (for LoadClosure/StoreClosure).
    /// Only populated while compiling a closure body that has mutable captures.
    pub(crate) mutable_closure_captures: HashMap<String, u16>,

    /// Variables in the current scope that have been boxed into SharedCells
    /// by a mutable closure capture. When a subsequent closure captures one
    /// of these variables (even immutably), it must use the SharedCell path
    /// so it shares the same mutable cell.
    pub(crate) boxed_locals: HashSet<String>,

    /// Active permission set for capability checking.
    ///
    /// When set, imported stdlib functions are checked against capability_tags.
    /// If a function requires a permission not in this set, a compile error is
    /// emitted and the function never enters bytecode.
    ///
    /// `None` means no checking (backwards-compatible default).
    pub(crate) permission_set: Option<shape_abi_v1::PermissionSet>,

    // -- Content-addressed blob tracking --
    /// Active blob builder (set while compiling a function body).
    pub(crate) current_blob_builder: Option<FunctionBlobBuilder>,
    /// Completed function blobs (finalized with content hash).
    pub(crate) completed_blobs: Vec<FunctionBlob>,
    /// Map from function name to content hash (populated after finalization).
    pub(crate) blob_name_to_hash: HashMap<String, FunctionHash>,
    /// The content-addressed program produced alongside BytecodeProgram.
    pub(crate) content_addressed_program: Option<ContentAddressedProgram>,
    /// Content hash per compiled function index (function_id -> blob hash).
    /// This is the stable identity bridge for the flat runtime format.
    pub(crate) function_hashes_by_id: Vec<Option<FunctionHash>>,

    /// Optional blob-level cache for incremental compilation.
    /// When set, compiled blobs are stored after finalization and looked up
    /// by content hash to avoid redundant work across compilations.
    pub(crate) blob_cache: Option<BlobCache>,

    /// Temporary function name aliases for comptime replace body.
    /// Maps alias (e.g., `__original__`) to actual function name (e.g., `__original__myFunc`).
    /// Set before compiling a replacement body and cleared after.
    pub(crate) function_aliases: HashMap<String, String>,

    /// Parameters of the function currently being compiled.
    /// Used by match exhaustiveness checking to fall back to type annotations
    /// when the type inference engine cannot resolve a parameter's type.
    pub(crate) current_function_params: Vec<shape_ast::ast::FunctionParameter>,

    /// Legacy cache of function names collected from stdlib-loaded modules.
    ///
    /// Internal builtin access is now gated by per-definition declaring-module
    /// provenance, not by membership in this set.
    pub stdlib_function_names: HashSet<String>,

    /// Per-function flag: when true, `get_builtin_function` resolves `__*` names.
    /// Toggled during compilation for definitions originating from `std::*`.
    pub(crate) allow_internal_builtins: bool,

    /// Package-scoped native library resolutions for the current host.
    pub(crate) native_resolution_context:
        Option<shape_runtime::native_resolution::NativeResolutionSet>,
}

impl Default for BytecodeCompiler {
    fn default() -> Self {
        Self::new()
    }
}

mod compiler_impl_part1;
mod compiler_impl_part2;
mod compiler_impl_part3;
mod compiler_impl_part4;

/// Infer effective reference parameters and mutation behavior without compiling bytecode.
///
/// Returns `(inferred_ref_params, inferred_ref_mutates)` keyed by function name.
/// - `inferred_ref_params[f][i] == true` means parameter `i` of `f` is inferred/treated as ref.
/// - `inferred_ref_mutates[f][i] == true` means that reference parameter is mutating (`&mut`).
pub fn infer_reference_model(
    program: &Program,
) -> (HashMap<String, Vec<bool>>, HashMap<String, Vec<bool>>) {
    let (inferred_ref_params, inferred_ref_mutates, _) =
        BytecodeCompiler::infer_reference_model(program);
    (inferred_ref_params, inferred_ref_mutates)
}

/// Infer effective parameter pass modes (`ByValue` / `ByRefShared` / `ByRefExclusive`)
/// keyed by function name.
pub fn infer_param_pass_modes(program: &Program) -> HashMap<String, Vec<ParamPassMode>> {
    let (inferred_ref_params, inferred_ref_mutates, _) =
        BytecodeCompiler::infer_reference_model(program);
    BytecodeCompiler::build_param_pass_mode_map(
        program,
        &inferred_ref_params,
        &inferred_ref_mutates,
    )
}

#[cfg(all(test, feature = "deep-tests"))]
#[path = "compiler_tests.rs"]
mod compiler_deep;

#[cfg(all(test, feature = "deep-tests"))]
#[path = "borrow_deep_tests.rs"]
mod borrow_deep_tests;
