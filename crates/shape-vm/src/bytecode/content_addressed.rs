use super::*;
use crate::type_tracking::{FrameDescriptor, StorageHint};

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FunctionHash(pub [u8; 32]);

impl std::fmt::Debug for FunctionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FunctionHash({})", self)
    }
}

impl std::fmt::Display for FunctionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl FunctionHash {
    /// The zero hash, used as a sentinel/placeholder.
    pub const ZERO: Self = Self([0u8; 32]);
}

/// A self-contained, content-addressed function blob.
///
/// Each blob carries its own instructions, constants, and strings (no shared
/// pools). The `content_hash` is the SHA-256 of the serialized content fields,
/// making deduplication and caching trivial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionBlob {
    /// SHA-256 hash of the serialized content (everything below).
    pub content_hash: FunctionHash,

    // -- metadata --
    pub name: String,
    pub arity: u16,
    pub param_names: Vec<String>,
    pub locals_count: u16,
    pub is_closure: bool,
    pub captures_count: u16,
    pub is_async: bool,
    #[serde(default)]
    pub ref_params: Vec<bool>,
    #[serde(default)]
    pub ref_mutates: Vec<bool>,
    #[serde(default)]
    pub mutable_captures: Vec<bool>,
    /// Typed frame layout for this function's locals (propagated from compiler).
    #[serde(default)]
    pub frame_descriptor: Option<FrameDescriptor>,

    // -- code --
    /// This function's bytecode instructions.
    pub instructions: Vec<Instruction>,
    /// This function's constant pool.
    pub constants: Vec<Constant>,
    /// This function's string pool.
    pub strings: Vec<String>,

    // -- permissions --
    /// Permissions required by this function (from capability_tags analysis).
    #[serde(default = "default_permission_set")]
    pub required_permissions: PermissionSet,

    // -- dependency graph --
    /// Content hashes of functions this blob references
    /// (`Operand::Function(idx)` indexes into this vector).
    pub dependencies: Vec<FunctionHash>,

    /// Callee names corresponding to each dependency entry.
    /// Used during compilation to resolve forward references; not serialized.
    #[serde(skip, default)]
    pub callee_names: Vec<String>,

    // -- type info --
    /// Type names this function constructs (schema references).
    pub type_schemas: Vec<String>,

    // -- foreign function dependencies --
    /// Content hashes of foreign functions referenced by `CallForeign` opcodes.
    /// Sorted and deduplicated for deterministic hashing.
    #[serde(default)]
    pub foreign_dependencies: Vec<[u8; 32]>,

    // -- debug --
    /// Source mapping entries local to this blob:
    /// `(local_instruction_offset, file_id, line)`.
    pub source_map: Vec<(usize, u32, u32)>,
}

/// Helper struct for deterministic content hashing.
/// We serialize exactly the fields that define the function's identity.
#[derive(Serialize)]
struct FunctionBlobHashInput<'a> {
    name: &'a str,
    arity: u16,
    param_names: &'a [String],
    locals_count: u16,
    is_closure: bool,
    captures_count: u16,
    is_async: bool,
    ref_params: &'a [bool],
    ref_mutates: &'a [bool],
    mutable_captures: &'a [bool],
    instructions: &'a [Instruction],
    constants: &'a [Constant],
    strings: &'a [String],
    dependencies: &'a [FunctionHash],
    type_schemas: &'a [String],
    /// Permission names sorted deterministically for stable hashing.
    required_permission_names: Vec<&'a str>,
    /// Content hashes of foreign functions referenced by this blob.
    foreign_dependencies: &'a [[u8; 32]],
}

impl FunctionBlob {
    /// Compute the content hash from the blob's fields.
    /// Call this after populating all fields, then assign the result to `content_hash`.
    pub fn compute_hash(&self) -> FunctionHash {
        // Convert PermissionSet to sorted permission names for deterministic hashing.
        let perm_names: Vec<&str> = self.required_permissions.iter().map(|p| p.name()).collect();
        let input = FunctionBlobHashInput {
            name: &self.name,
            arity: self.arity,
            param_names: &self.param_names,
            locals_count: self.locals_count,
            is_closure: self.is_closure,
            captures_count: self.captures_count,
            is_async: self.is_async,
            ref_params: &self.ref_params,
            ref_mutates: &self.ref_mutates,
            mutable_captures: &self.mutable_captures,
            instructions: &self.instructions,
            constants: &self.constants,
            strings: &self.strings,
            dependencies: &self.dependencies,
            type_schemas: &self.type_schemas,
            required_permission_names: perm_names,
            foreign_dependencies: &self.foreign_dependencies,
        };
        // Use bincode-compatible MessagePack for deterministic serialization.
        // rmp_serde::encode::to_vec uses the struct-as-array format which is
        // order-preserving and deterministic for the types we use here.
        let bytes = rmp_serde::encode::to_vec(&input)
            .expect("FunctionBlob content serialization should not fail");
        let digest = Sha256::digest(&bytes);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&digest);
        FunctionHash(hash)
    }

    /// Build a blob with all fields set, then compute and assign its content hash.
    pub fn finalize(&mut self) {
        self.content_hash = self.compute_hash();
    }
}

/// A content-addressed program: a set of `FunctionBlob`s plus program-level metadata.
///
/// This is the **storage / cache** representation. Before execution the linker
/// flattens it into a `LinkedProgram`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    /// Hash of the entry-point function.
    pub entry: FunctionHash,

    /// All function blobs keyed by content hash.
    pub function_store: HashMap<FunctionHash, FunctionBlob>,

    /// Number of locals used by top-level code.
    pub top_level_locals_count: u16,

    /// Storage hints for top-level locals.
    #[serde(default)]
    pub top_level_local_storage_hints: Vec<StorageHint>,

    /// Module-binding variable names (index -> name).
    pub module_binding_names: Vec<String>,

    /// Storage hints for module bindings.
    #[serde(default)]
    pub module_binding_storage_hints: Vec<StorageHint>,

    /// Per-function local storage hints.
    #[serde(default)]
    pub function_local_storage_hints: Vec<Vec<StorageHint>>,

    /// Typed frame layout for top-level locals.
    #[serde(default)]
    pub top_level_frame: Option<FrameDescriptor>,

    /// DataFrame schema for column name resolution.
    pub data_schema: Option<DataFrameSchema>,

    /// Type schema registry for TypedObject field resolution.
    #[serde(default)]
    pub type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry,

    /// Trait method dispatch registry.
    pub trait_method_symbols: HashMap<String, String>,

    /// Foreign function metadata table.
    #[serde(default)]
    pub foreign_functions: Vec<ForeignFunctionEntry>,

    /// Native `type C` layout metadata table.
    #[serde(default)]
    pub native_struct_layouts: Vec<NativeStructLayoutEntry>,

    /// Debug information (source files, variable names).
    pub debug_info: DebugInfo,
}

/// A linked function ready for execution in a flat instruction array.
///
/// Mirrors `Function` but adds `blob_hash` so the runtime can trace back
/// to the original content-addressed blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedFunction {
    /// Content hash of the `FunctionBlob` this was linked from.
    pub blob_hash: FunctionHash,

    /// Offset into the flat `LinkedProgram::instructions` array.
    pub entry_point: usize,
    /// Number of instructions in this function's body.
    pub body_length: usize,

    // -- metadata (same as Function) --
    pub name: String,
    pub arity: u16,
    pub param_names: Vec<String>,
    pub locals_count: u16,
    pub is_closure: bool,
    pub captures_count: u16,
    pub is_async: bool,
    #[serde(default)]
    pub ref_params: Vec<bool>,
    #[serde(default)]
    pub ref_mutates: Vec<bool>,
    #[serde(default)]
    pub mutable_captures: Vec<bool>,
    /// Typed frame layout for this function's locals.
    #[serde(default)]
    pub frame_descriptor: Option<FrameDescriptor>,
}

/// A linked, execution-ready program with flat instruction/constant/string arrays.
///
/// This mirrors today's `BytecodeProgram` layout so the executor can run it
/// with minimal changes. Produced by the linker from a `Program`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkedProgram {
    /// Hash of the entry-point function for execution.
    #[serde(default)]
    pub entry: FunctionHash,

    /// Flat instruction array (all functions concatenated).
    pub instructions: Vec<Instruction>,

    /// Merged constant pool.
    pub constants: Vec<Constant>,

    /// Merged string pool.
    pub strings: Vec<String>,

    /// Linked function table (replaces `Vec<Function>`).
    pub functions: Vec<LinkedFunction>,

    /// Reverse lookup: content hash -> function index in `functions`.
    pub hash_to_id: HashMap<FunctionHash, usize>,

    /// Debug information.
    pub debug_info: DebugInfo,

    /// DataFrame schema for column name resolution.
    pub data_schema: Option<DataFrameSchema>,

    /// Module-binding variable names.
    pub module_binding_names: Vec<String>,

    /// Number of locals used by top-level code.
    pub top_level_locals_count: u16,

    /// Storage hints for top-level locals.
    #[serde(default)]
    pub top_level_local_storage_hints: Vec<StorageHint>,

    /// Type schema registry for TypedObject field resolution.
    #[serde(default)]
    pub type_schema_registry: shape_runtime::type_schema::TypeSchemaRegistry,

    /// Storage hints for module bindings.
    #[serde(default)]
    pub module_binding_storage_hints: Vec<StorageHint>,

    /// Per-function local storage hints.
    #[serde(default)]
    pub function_local_storage_hints: Vec<Vec<StorageHint>>,

    /// Typed frame layout for top-level locals.
    #[serde(default)]
    pub top_level_frame: Option<FrameDescriptor>,

    /// Trait method dispatch registry.
    pub trait_method_symbols: HashMap<String, String>,

    /// Foreign function metadata table.
    #[serde(default)]
    pub foreign_functions: Vec<ForeignFunctionEntry>,

    /// Native `type C` layout metadata table.
    #[serde(default)]
    pub native_struct_layouts: Vec<NativeStructLayoutEntry>,

    /// Transitive union of all required permissions across all blobs.
    /// Computed by the linker during `link()`.
    #[serde(default = "default_permission_set")]
    pub total_required_permissions: PermissionSet,
}
