use super::*;
use crate::type_tracking::{FrameDescriptor, SlotKind, StorageHint};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataFrameSchema {
    /// Column names in order (index = column index)
    pub column_names: Vec<String>,
    /// Column name to index mapping for O(1) lookup
    pub name_to_index: std::collections::HashMap<String, u32>,
}

impl DataFrameSchema {
    /// Create schema from column names (indices assigned in order)
    pub fn from_columns(columns: Vec<String>) -> Self {
        let name_to_index = columns
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i as u32))
            .collect();
        Self {
            column_names: columns,
            name_to_index,
        }
    }

    /// Get column index by name
    pub fn get_index(&self, name: &str) -> Option<u32> {
        self.name_to_index.get(name).copied()
    }

    /// Get column name by index
    pub fn get_name(&self, index: u32) -> Option<&str> {
        self.column_names.get(index as usize).map(|s| s.as_str())
    }

    /// Number of columns
    pub fn len(&self) -> usize {
        self.column_names.len()
    }

    /// Check if schema is empty
    pub fn is_empty(&self) -> bool {
        self.column_names.is_empty()
    }
}

/// Metadata for a foreign function stored in the program.
/// The compiler creates these; the engine links them to language runtimes before execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignFunctionEntry {
    pub name: String,
    pub language: String,
    pub body_text: String,
    pub param_names: Vec<String>,
    pub param_types: Vec<String>,
    pub return_type: Option<String>,
    pub arg_count: u16,
    pub is_async: bool,
    /// Whether this foreign function's runtime has a dynamic error model.
    /// When `true`, the executor wraps results in `Result<T>` (Ok/Err).
    /// Defaults to `true` at compile time (safe default); overridden at link
    /// time from the actual runtime's `ErrorModel`.
    #[serde(default = "default_dynamic_errors")]
    pub dynamic_errors: bool,
    /// Schema ID for the return type when it contains an inline object type.
    /// Set at compile time by `compile_foreign_function()`. Used by the
    /// executor to construct `HeapValue::TypedObject` from msgpack Maps.
    #[serde(default)]
    pub return_type_schema_id: Option<u32>,
    /// Content hash for caching and deduplication.
    /// Computed from (language, body_text, param_types, return_type).
    #[serde(default)]
    pub content_hash: Option<[u8; 32]>,
    /// Native C ABI metadata for `extern "C"` declarations.
    ///
    /// When present, the VM links this function through the internal C ABI
    /// path instead of a language-runtime extension.
    #[serde(default)]
    pub native_abi: Option<NativeAbiSpec>,
}

/// Native C ABI metadata stored alongside a foreign function entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeAbiSpec {
    /// ABI name (currently `"C"`).
    pub abi: String,
    /// Shared library path/name to load.
    pub library: String,
    /// Symbol name to resolve in the library.
    pub symbol: String,
    /// Canonical C signature string, e.g. `fn(f64) -> f64`.
    pub signature: String,
}

/// Native `type C` layout entry emitted by the compiler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeStructLayoutEntry {
    pub name: String,
    pub abi: String,
    pub size: u32,
    pub align: u32,
    pub fields: Vec<NativeStructFieldLayout>,
}

/// Native field layout metadata for one struct field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeStructFieldLayout {
    pub name: String,
    pub c_type: String,
    pub offset: u32,
    pub size: u32,
    pub align: u32,
}

// ---------------------------------------------------------------------------
// OSR (On-Stack Replacement) and Deoptimization Metadata
// ---------------------------------------------------------------------------

/// Metadata for an OSR entry point (typically a loop header).
///
/// When the VM detects a hot loop via back-edge counting, it can request
/// compilation of just the loop body.  This struct describes where the loop
/// starts and ends in bytecode, which locals are live at the header, and the
/// types of those locals for marshaling between interpreter and JIT frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsrEntryPoint {
    /// Bytecode IP of the loop header (LoopStart instruction).
    pub bytecode_ip: usize,
    /// Which local slots are live at the entry point.
    pub live_locals: Vec<u16>,
    /// SlotKind for each live local (parallel to `live_locals`, used for marshaling).
    pub local_kinds: Vec<SlotKind>,
    /// The bytecode IP of the loop exit (LoopEnd + 1).
    pub exit_ip: usize,
}

/// Metadata for deoptimization -- maps JIT state back to interpreter state.
///
/// When a type guard or other speculative assumption fails inside JIT-compiled
/// code, the JIT returns the deopt sentinel (`u64::MAX`).  The VM uses this
/// struct to reconstruct interpreter state from the JITContext locals and
/// resume execution in the bytecode interpreter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeoptInfo {
    /// Bytecode IP to resume at in the interpreter.
    pub resume_ip: usize,
    /// Map from JIT local index to bytecode local index.
    /// Each pair is `(jit_local_idx, bytecode_local_idx)`.
    pub local_mapping: Vec<(u16, u16)>,
    /// SlotKind for each mapped local (parallel to `local_mapping`, used for
    /// unmarshaling JIT values back to interpreter `ValueWord` representation).
    pub local_kinds: Vec<SlotKind>,
    /// Stack depth at this deopt point (number of values on the operand stack).
    pub stack_depth: u16,
    /// Function ID of the innermost frame (where the guard fired).
    /// For single-frame deopt this equals the physical JIT function.
    /// For multi-frame deopt this is the inlined callee.
    #[serde(default)]
    pub innermost_function_id: Option<u16>,
    /// Caller frames for multi-frame deopt (outermost-first: [0] = outermost physical function).
    /// Empty for single-frame deopt (the common case).
    #[serde(default)]
    pub inline_frames: Vec<InlineFrameInfo>,
}

/// Metadata for a single caller frame in a multi-frame inline deopt.
///
/// When a guard fails inside inlined code, the VM needs to reconstruct
/// the full call stack. Each `InlineFrameInfo` describes one caller frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineFrameInfo {
    /// Function ID of this caller frame.
    pub function_id: u16,
    /// Bytecode IP to resume at in this caller (the Call/CallValue instruction).
    pub resume_ip: usize,
    /// Map from ctx_buf position to bytecode local index for this frame.
    pub local_mapping: Vec<(u16, u16)>,
    /// SlotKind for each mapped local.
    pub local_kinds: Vec<SlotKind>,
    /// Stack depth for this frame.
    pub stack_depth: u16,
}

impl ForeignFunctionEntry {
    /// Compute a content hash from (language, body_text, param_types, return_type).
    pub fn compute_content_hash(&mut self) {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.language.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.body_text.as_bytes());
        hasher.update(b"\0");
        for pt in &self.param_types {
            hasher.update(pt.as_bytes());
            hasher.update(b"\0");
        }
        if let Some(ref rt) = self.return_type {
            hasher.update(rt.as_bytes());
        }
        if let Some(native) = &self.native_abi {
            hasher.update(b"\0native\0");
            hasher.update(native.abi.as_bytes());
            hasher.update(b"\0");
            hasher.update(native.library.as_bytes());
            hasher.update(b"\0");
            hasher.update(native.symbol.as_bytes());
            hasher.update(b"\0");
            hasher.update(native.signature.as_bytes());
        }
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        self.content_hash = Some(hash);
    }
}

fn default_dynamic_errors() -> bool {
    true
}

pub(crate) fn default_permission_set() -> PermissionSet {
    PermissionSet::pure()
}

/// A compiled bytecode program
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BytecodeProgram {
    /// The bytecode instructions
    pub instructions: Vec<Instruction>,

    /// Constant pool for literals
    pub constants: Vec<Constant>,

    /// String pool for identifiers and properties
    pub strings: Vec<String>,

    /// Function table
    pub functions: Vec<Function>,

    /// Debug information (always present, used for error messages)
    pub debug_info: DebugInfo,

    /// DataFrame schema for column name resolution (required for data access)
    pub data_schema: Option<DataFrameSchema>,

    /// ModuleBinding variable names (index -> name mapping for REPL persistence)
    pub module_binding_names: Vec<String>,

    /// Number of locals used by top-level code.
    /// The executor advances `sp` past this many slots before execution
    /// so that expression evaluation doesn't overlap with local storage.
    pub top_level_locals_count: u16,

    /// Storage hints for top-level locals (index -> hint).
    /// Used by JIT lowering to preserve native width integer codegen.
    #[serde(default)]
    pub top_level_local_storage_hints: Vec<StorageHint>,

    /// Storage hints for module bindings (index -> hint).
    #[serde(default)]
    pub module_binding_storage_hints: Vec<StorageHint>,

    /// Per-function local storage hints.
    /// `function_local_storage_hints[f][local]` is the hint for local slot in function `f`.
    #[serde(default)]
    pub function_local_storage_hints: Vec<Vec<StorageHint>>,

    /// Typed frame layout for top-level locals.
    ///
    /// When present, supersedes `top_level_local_storage_hints` for the
    /// JIT and VM.  `None` means fall back to the legacy hints vec.
    #[serde(default)]
    pub top_level_frame: Option<FrameDescriptor>,

    /// Type schema registry for TypedObject field resolution
    /// Used to convert TypedObject back to Object when needed
    #[serde(default)]
    pub type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry,

    /// Compiled annotation definitions
    /// Maps annotation name to its compiled handlers
    #[serde(skip, default)]
    pub compiled_annotations: std::collections::HashMap<String, CompiledAnnotation>,

    /// Trait method dispatch registry:
    /// (trait, type, impl selector, method) -> compiled function symbol name.
    ///
    /// This is populated by impl-block compilation and used by runtime dispatch
    /// (e.g. print() -> Display::display) without relying on symbol naming heuristics.
    pub trait_method_symbols: std::collections::HashMap<String, String>,

    /// Final function definitions after comptime mutation/specialization.
    ///
    /// This is a compile-time inspection artifact for tooling (e.g. `shape --expand`)
    /// and is not serialized into cached bytecode.
    #[serde(skip, default)]
    pub expanded_function_defs: std::collections::HashMap<String, shape_ast::ast::FunctionDef>,

    /// Reverse index for O(1) string dedup during compilation.
    /// Maps string content → index in `self.strings`.
    /// Not serialized — rebuilt lazily on first `intern_string` call after deserialization.
    #[serde(skip, default)]
    pub string_index: std::collections::HashMap<String, u32>,

    /// Foreign function metadata table.
    /// Populated by the compiler when `fn python ...` blocks are compiled.
    /// Linked to language runtimes before execution.
    #[serde(default)]
    pub foreign_functions: Vec<ForeignFunctionEntry>,

    /// Native `type C` layout metadata table.
    #[serde(default)]
    pub native_struct_layouts: Vec<NativeStructLayoutEntry>,

    /// Content-addressed program built alongside the flat bytecode.
    ///
    /// When present, this contains per-function `FunctionBlob`s with content
    /// hashes. It is produced by the compiler as a dual-output alongside the
    /// traditional flat instruction array.
    #[serde(default)]
    pub content_addressed: Option<Program>,

    /// Content hash for each function in `functions`, indexed by function ID.
    ///
    /// This provides stable function identity without relying on function names.
    /// `None` entries indicate missing content-addressed metadata.
    #[serde(default)]
    pub function_blob_hashes: Vec<Option<FunctionHash>>,
}

/// Constants in the constant pool
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    Int(i64),
    /// Unsigned 64-bit integer (for u64 values > i64::MAX)
    UInt(u64),
    Number(f64),
    /// Decimal type for exact arithmetic (finance, currency)
    Decimal(rust_decimal::Decimal),
    String(String),
    Bool(bool),
    Null,
    Unit,
    Function(u16),
    Timeframe(Timeframe),
    Duration(Duration),
    TimeReference(TimeReference),
    DateTimeExpr(DateTimeExpr),
    DataDateTimeRef(DataDateTimeRef),
    TypeAnnotation(TypeAnnotation),
    /// Opaque runtime value (not serializable — used for host-injected constants like RowView, DataTable, etc.)
    #[serde(skip)]
    Value(ValueWord),
}

/// Function definition in bytecode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub arity: u16,
    pub param_names: Vec<String>, // Parameter names for named argument support
    pub locals_count: u16,
    pub entry_point: usize,
    pub body_length: usize,
    pub is_closure: bool,
    pub captures_count: u16,
    pub is_async: bool,
    /// Which parameters are passed by reference (`&`).
    /// `ref_params[i] == true` means param `i` is a reference.
    #[serde(default)]
    pub ref_params: Vec<bool>,
    /// Which reference parameters mutate their target.
    /// `ref_mutates[i] == true` means ref param `i` performs writes (exclusive borrow).
    /// `false` means read-only (shared borrow).
    #[serde(default)]
    pub ref_mutates: Vec<bool>,
    /// Which upvalues (captures) are mutable.
    /// `mutable_captures[i] == true` means capture `i` is assigned inside the closure
    /// and should use `Mutable(Arc<RwLock<>>)` upvalues for shared state.
    #[serde(default)]
    pub mutable_captures: Vec<bool>,
    /// Typed frame layout for this function's locals (params + locals).
    ///
    /// When present, the JIT / VM can use per-slot type information to
    /// skip NaN-boxing and allocate native-width registers.  `None` means
    /// all slots are treated as generic `Boxed` values.
    #[serde(default)]
    pub frame_descriptor: Option<FrameDescriptor>,
    /// OSR (On-Stack Replacement) entry points for hot loops in this function.
    ///
    /// Populated by loop analysis when the JIT compiles a loop body separately.
    /// Each entry describes a loop header where the VM can transfer execution
    /// from the interpreter into JIT-compiled code mid-function.
    #[serde(default)]
    pub osr_entry_points: Vec<OsrEntryPoint>,
}

/// A compiled annotation definition.
///
/// Stores the annotation's parameter names and the function IDs
/// for each lifecycle handler (before, after, on_define, metadata).
/// The comptime handler is stored as AST (not compiled to bytecode)
/// since it executes at compile time when the annotation is applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledAnnotation {
    pub name: String,
    pub param_names: Vec<String>,
    /// Function ID for `before(args, ctx)` handler (if defined)
    pub before_handler: Option<u16>,
    /// Function ID for `after(args, result, ctx)` handler (if defined)
    pub after_handler: Option<u16>,
    /// Function ID for `on_define(target, ctx)` handler (if defined)
    pub on_define_handler: Option<u16>,
    /// Function ID for `metadata(target, ctx)` handler (if defined)
    pub metadata_handler: Option<u16>,
    /// AST for `comptime pre(target, ctx) { ... }` (executed before function inference/compilation)
    #[serde(skip, default)]
    pub comptime_pre_handler: Option<shape_ast::ast::AnnotationHandler>,
    /// AST for `comptime post(target, ctx) { ... }` (executed after function inference/compilation)
    #[serde(skip, default)]
    pub comptime_post_handler: Option<shape_ast::ast::AnnotationHandler>,
    /// Allowed target kinds for this annotation.
    /// Inferred from handler types: before/after → Function only;
    /// metadata/comptime only → any target.
    /// Empty means no restriction (any target allowed).
    #[serde(skip, default)]
    pub allowed_targets: Vec<shape_ast::ast::functions::AnnotationTargetKind>,
}

/// Source file table for multi-file programs
///
/// When modules are merged, each file gets a unique ID. This allows
/// error messages to report the correct source file after merging.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceMap {
    /// File paths indexed by file_id
    pub files: Vec<String>,
    /// Source text for each file (indexed by file_id)
    /// Used for displaying source context in error messages
    #[serde(skip, default)]
    pub source_texts: Vec<String>,
}

impl SourceMap {
    /// Create a new source map with a single file
    pub fn new(file_path: String) -> Self {
        Self {
            files: vec![file_path],
            source_texts: Vec::new(),
        }
    }

    /// Add a file to the source map and return its file_id
    pub fn add_file(&mut self, file_path: String) -> u16 {
        // Check if file already exists
        if let Some(idx) = self.files.iter().position(|f| f == &file_path) {
            return idx as u16;
        }
        let id = self.files.len() as u16;
        self.files.push(file_path);
        id
    }

    /// Add source text for a file
    pub fn set_source_text(&mut self, file_id: u16, source: String) {
        let idx = file_id as usize;
        // Extend source_texts if needed
        while self.source_texts.len() <= idx {
            self.source_texts.push(String::new());
        }
        self.source_texts[idx] = source;
    }

    /// Get file path by file_id
    pub fn get_file(&self, file_id: u16) -> Option<&str> {
        self.files.get(file_id as usize).map(|s| s.as_str())
    }

    /// Get source text for a file
    pub fn get_source_text(&self, file_id: u16) -> Option<&str> {
        self.source_texts.get(file_id as usize).map(|s| s.as_str())
    }

    /// Get number of files
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Debug information for better error messages
///
/// Supports multi-file programs by tracking which file each instruction
/// came from. This is essential for correct error reporting after modules
/// are merged together.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DebugInfo {
    /// Source file table for multi-file support
    pub source_map: SourceMap,

    /// Line number mappings: (instruction_index, file_id, line_number)
    /// The file_id indexes into source_map.files
    pub line_numbers: Vec<(usize, u16, u32)>,

    /// Variable name mappings
    pub variable_names: Vec<(u16, String)>,

    /// Legacy: Source text for showing context in error messages
    /// Deprecated: Use source_map.source_texts instead
    #[serde(skip, default)]
    pub source_text: String,
}

impl DebugInfo {
    /// Create a new DebugInfo with a single source file
    pub fn new(source_file: String) -> Self {
        Self {
            source_map: SourceMap::new(source_file),
            line_numbers: Vec::new(),
            variable_names: Vec::new(),
            source_text: String::new(),
        }
    }

    /// Add a line number mapping for an instruction
    pub fn add_line(&mut self, instruction_idx: usize, file_id: u16, line: u32) {
        self.line_numbers.push((instruction_idx, file_id, line));
    }

    /// Get line number and file for an instruction index
    pub fn get_location_for_instruction(&self, ip: usize) -> Option<(u16, u32)> {
        // Find the most recent line number mapping that's <= ip
        self.line_numbers
            .iter()
            .rev()
            .find(|(idx, _, _)| *idx <= ip)
            .map(|(_, file_id, line)| (*file_id, *line))
    }

    /// Get line number for an instruction index (legacy, file_id=0)
    pub fn get_line_for_instruction(&self, ip: usize) -> Option<u32> {
        self.get_location_for_instruction(ip).map(|(_, line)| line)
    }

    /// Get a source line by line number (1-indexed) from the primary file
    pub fn get_source_line(&self, line: usize) -> Option<&str> {
        // First try the source_map
        if let Some(source) = self.source_map.get_source_text(0) {
            return source.lines().nth(line.saturating_sub(1));
        }
        // Fall back to legacy source_text
        self.source_text.lines().nth(line.saturating_sub(1))
    }

    /// Get a source line from a specific file
    pub fn get_source_line_from_file(&self, file_id: u16, line: usize) -> Option<&str> {
        if let Some(source) = self.source_map.get_source_text(file_id) {
            return source.lines().nth(line.saturating_sub(1));
        }
        // Fall back to legacy if file_id is 0
        if file_id == 0 {
            return self.source_text.lines().nth(line.saturating_sub(1));
        }
        None
    }

    /// Get the file name for an instruction
    pub fn get_file_for_instruction(&self, ip: usize) -> Option<&str> {
        self.get_location_for_instruction(ip)
            .and_then(|(file_id, _)| self.source_map.get_file(file_id))
    }
}

impl Instruction {
    /// Create a new instruction
    pub fn new(opcode: OpCode, operand: Option<Operand>) -> Self {
        Self { opcode, operand }
    }

    /// Create a simple instruction without operand
    pub fn simple(opcode: OpCode) -> Self {
        Self {
            opcode,
            operand: None,
        }
    }

    /// Get the size of this instruction in bytes
    pub fn size(&self) -> usize {
        1 + match &self.operand {
            None => 0,
            Some(op) => match op {
                Operand::Const(_)
                | Operand::Local(_)
                | Operand::ModuleBinding(_)
                | Operand::Function(_)
                | Operand::Count(_)
                | Operand::Property(_) => 2,
                Operand::Offset(_) | Operand::ColumnIndex(_) => 4,
                Operand::Builtin(_) => 1,
                // TypedField: type_id (2) + field_idx (2) + field_type_tag (2) = 6 bytes
                Operand::TypedField { .. } => 6,
                // TypedObjectAlloc: schema_id (2) + field_count (2) = 4 bytes
                Operand::TypedObjectAlloc { .. } => 4,
                // TypedMerge: target_schema_id (2) + left_size (2) + right_size (2) = 6 bytes
                Operand::TypedMerge { .. } => 6,
                // ColumnAccess: col_id (4) = 4 bytes
                Operand::ColumnAccess { .. } => 4,
                // Name: StringId (4 bytes)
                Operand::Name(_) => 4,
                // MethodCall: StringId (4) + arg_count (2) = 6 bytes
                Operand::MethodCall { .. } => 6,
                // TypedMethodCall: method_id (2) + arg_count (2) + string_id (2) = 6 bytes
                Operand::TypedMethodCall { .. } => 6,
                // ForeignFunction: u16 index = 2 bytes
                Operand::ForeignFunction(_) => 2,
                // MatrixDims: rows (2) + cols (2) = 4 bytes
                Operand::MatrixDims { .. } => 4,
                // Width: NumericWidth (1 byte)
                Operand::Width(_) => 1,
                // TypedLocal: local_idx (2) + width (1) = 3 bytes
                Operand::TypedLocal(_, _) => 3,
            },
        }
    }
}
