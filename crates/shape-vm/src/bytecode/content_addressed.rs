use super::*;
use crate::type_tracking::{FrameDescriptor, NativeKind, StorageHint};

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

    /// Per-capture proven `NativeKind` for this function's upvalues, in
    /// declaration order (distributed §4.4 / §4.8). Stamped at blob
    /// construction from the compiler's proven capture kinds — the same proof
    /// source as `frame_descriptor.slots` (the closure registry's
    /// `capture_native_kinds`). Empty for non-closure functions.
    ///
    /// Included in `FunctionBlobHashInput`: a closure body that reads capture 0
    /// as `Float64` is a different function from one that reads it as
    /// `Ptr(TypedObject)` — capture layout is call-ABI identity exactly like
    /// param kinds. Typed `NativeKind` only (ADR-006 §Forbidden Patterns — no
    /// raw-bits / Bool-default).
    #[serde(default)]
    pub capture_kinds: Vec<NativeKind>,

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
    /// Typed frame layout (per-slot `NativeKind` + ABI return kind). Defines
    /// the call ABI the remote-marshal path trusts — hash-covered so a
    /// tampered/divergent descriptor cannot hide behind a matching hash
    /// (distributed §4.8 / OQ-6). Added to the hash input in the WF-2A stage-0
    /// window.
    frame_descriptor: &'a Option<FrameDescriptor>,
    /// Per-capture proven `NativeKind` — capture layout is call-ABI identity
    /// exactly like param kinds (distributed §4.4 / §4.8). Added to the hash
    /// input in the WF-2A stage-0 window.
    capture_kinds: &'a [NativeKind],
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
            frame_descriptor: &self.frame_descriptor,
            capture_kinds: &self.capture_kinds,
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

    /// Per-slot fully-resolved `ConcreteType` for top-level locals.
    ///
    /// ADR-006 §2.7.5 conduit (content-addressed mirror of
    /// `BytecodeProgram.top_level_local_concrete_types`). Survives the
    /// `Program` → `link()` → `LinkedProgram` → `BytecodeProgram`
    /// round-trip so JIT compilation of in-memory-compiled programs can
    /// use the typed-array / TypedObject fast paths. Not serialised —
    /// `ConcreteType` carries opaque registry IDs that aren't a stable
    /// wire shape; cached-program loads fall through to the legacy
    /// NaN-boxed path.
    #[serde(skip, default)]
    pub top_level_local_concrete_types: Vec<shape_value::v2::ConcreteType>,

    /// Per-user-function per-MIR-slot `ConcreteType` side-table.
    ///
    /// ADR-006 §2.7.5 conduit (W12-jit-aggregate-non-array, 2026-05-12):
    /// content-addressed mirror of
    /// `BytecodeProgram.function_local_concrete_types`. Survives the
    /// `Program` → `link()` → `LinkedProgram` → `BytecodeProgram` round-
    /// trip so JIT compilation of in-memory-compiled programs can use
    /// the TypedObject Aggregate short-circuit inside user-function
    /// bodies (Smoke 1.5 `divide`, Smoke 2 `first_positive`, 28 stdlib
    /// helpers). Not serialised — same rationale as
    /// `top_level_local_concrete_types`.
    #[serde(skip, default)]
    pub function_local_concrete_types: Vec<Vec<shape_value::v2::ConcreteType>>,

    /// Per-user-function declared `ConcreteType` for the return value.
    ///
    /// ADR-006 §2.7.5 conduit (W12-jit-call-return-kind close, 2026-05-12):
    /// content-addressed mirror of
    /// `BytecodeProgram.function_return_concrete_types`. Survives the
    /// `Program` → `link()` → `LinkedProgram` → `BytecodeProgram`
    /// round-trip so the conduit can stamp Call-terminator destination
    /// slots from the callee's declared return type. Not serialised —
    /// same rationale as the sibling `*_concrete_types` side-tables.
    #[serde(skip, default)]
    pub function_return_concrete_types: Vec<shape_value::v2::ConcreteType>,

    /// ADR-006 §2.7.5 conduit (V3-S6b-jit-method-monomorph-conduit
    /// close, 2026-05-15): content-addressed mirror of
    /// `BytecodeProgram.monomorphized_method_call_sites`. Survives the
    /// `Program` → `link()` → `LinkedProgram` → `BytecodeProgram`
    /// round-trip so the conduit producer can lift
    /// `function_return_concrete_types[specialized_idx]` into the
    /// destination slot's ConcreteType at `MirConstant::Method` Call-
    /// terminator sites. Not serialised — opaque per-program FunctionId
    /// indices aren't a stable wire shape.
    #[serde(skip, default)]
    pub monomorphized_method_call_sites:
        HashMap<(shape_ast::ast::span::Span, Option<usize>), usize>,

    /// ADR-006 §2.7.5 conduit (cluster-2-cw-IB-class-b close, 2026-05-16):
    /// content-addressed mirror of
    /// `BytecodeProgram.value_call_return_concrete_types`. Survives the
    /// `Program` → `link()` → `LinkedProgram` → `BytecodeProgram`
    /// round-trip so the conduit producer can stamp value-call
    /// `TerminatorKind::Call` destination slots from the closure-bound
    /// callee's inferred return `ConcreteType`. Not serialised —
    /// `ConcreteType` carries opaque registry IDs that aren't a stable
    /// wire shape.
    #[serde(skip, default)]
    pub value_call_return_concrete_types:
        HashMap<(shape_ast::ast::span::Span, Option<usize>), shape_value::v2::ConcreteType>,

    /// ADR-006 §2.7.5 conduit (W10 jit-call-method-user-trait-fix close,
    /// 2026-05-17): content-addressed mirror of
    /// `BytecodeProgram.operator_trait_dispatch_sites`. Survives the
    /// `Program` → `link()` → `LinkedProgram` → `BytecodeProgram`
    /// round-trip so the JIT consumer can re-emit user-type binary/unary
    /// trait-dispatch as method-call IR. Not serialised — Spans carry
    /// source-position offsets that aren't a stable wire shape.
    #[serde(skip, default)]
    pub operator_trait_dispatch_sites: HashMap<shape_ast::ast::span::Span, (String, u16)>,

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

    /// Closure spec §14.6 (H6.5): per-function-name `ClosureLayout`
    /// side-table threaded through the content-addressed `Program` so it
    /// survives the `link()` → `LinkedProgram` → `BytecodeProgram` round-
    /// trip. Keyed by function name because the content-addressed store
    /// reorders blobs topologically; the linker remaps to post-link
    /// function-id positions in `LinkedProgram.closure_function_layouts`.
    /// Not serialised — programs loaded from disk fall back to the
    /// legacy `HeapValue::Closure` variant.
    #[serde(skip, default)]
    pub closure_function_layouts_by_name: std::collections::HashMap<
        String,
        std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>,
    >,

    /// ADR-006 §2.7.24 Q25.C trait-object vtable registry. Keyed by
    /// `"Trait::ConcreteType"` strings (matching the existing
    /// `trait_method_symbols` key prefix). Built at impl-block
    /// compilation; consumed at `op_box_trait_object` runtime to build
    /// `Arc<TraitObjectStorage>`. Not serialised because `Arc<VTable>`
    /// is not a stable wire shape; in cached-program-load mode the
    /// vtables are rebuilt at link time from `trait_method_symbols`.
    #[serde(skip, default)]
    pub trait_vtables:
        std::collections::HashMap<String, std::sync::Arc<shape_value::value::VTable>>,

    /// R8 W8 Cluster A surface-and-stop flag (2026-05-25). Mirror of
    /// `BytecodeProgram::has_imported_const_inline` for the content-
    /// addressed Program shape; propagated through the linker so the JIT
    /// executor can deopt the whole program to the bytecode interpreter
    /// when imported `pub const` identifiers were inlined-at-use by
    /// `compile_expr_identifier`. v0.4 root-cause fix per close-summary
    /// §5.16. NOT serialised — compile-time state.
    #[serde(skip, default)]
    pub has_imported_const_inline: bool,

    /// R8 W9 B1 W17-marshal-return surface-and-stop flag (2026-05-25).
    /// Mirror of `BytecodeProgram::has_w17_marshal_residual` for the
    /// content-addressed Program shape; propagated through the linker so
    /// the JIT executor can deopt the whole program to the bytecode
    /// interpreter when the program contains direct calls to imported
    /// stdlib functions (which route through `Ptr(HeapKind::ModuleFn)`
    /// callees that the JIT's `jit_call_value` silently returns
    /// TAG_NULL for). v0.4 root-cause fix per close-summary §5.16.
    /// NOT serialised — compile-time state.
    #[serde(skip, default)]
    pub has_w17_marshal_residual: bool,

    /// c4-4B `?`-operator (TryUnwrap) JIT surface-and-stop flag (2026-05-28).
    /// Mirror of `BytecodeProgram::has_try_unwrap_residual` for the
    /// content-addressed Program shape; propagated through the linker so
    /// the JIT executor can deopt the whole program to the bytecode
    /// interpreter when the program contains an `OpCode::TryUnwrap` (the
    /// `?` operator) — whose unwrap-or-early-return semantics MIR drops
    /// (`Expr::TryOperator => copy` at `mir/lowering/expr.rs:2594`), so
    /// the JIT-emitted code stores the trampoline's heap-Result `u64`
    /// into a slot whose stamped kind is the success type's `NativeKind`
    /// (`Int64` for `Result<int,_>`), and `terminators.rs:1801-1813`'s
    /// I64-wide arm stamps `RETURN_TAG_I64` on pointer bits — silent-
    /// wrong-output per `regression::jit::jit_trampoline_result_callvalue`.
    /// SURFACE-and-deopt per supervisor 2026-05-28 c4-4B ratification (audit
    /// doc `docs/cluster-audits/v0.3.3/04-pointer-as-float-leak.md` §4B
    /// Sub-cluster — FN-REG-CORRECTNESS / RELEASE-BLOCKING).
    /// NOT serialised — compile-time state.
    #[serde(skip, default)]
    pub has_try_unwrap_residual: bool,

    /// ADR-006 §2.7.30 (GapA): mirror of
    /// `BytecodeProgram::has_reference_escape_promotion` — forces JIT deopt for
    /// programs whose functions return a reference via the `PromotedCell`
    /// escape carrier (no JIT lowering; VM is correct). NOT serialised.
    #[serde(skip, default)]
    pub has_reference_escape_promotion: bool,

    /// v0.3.3 book-gate `??` fix: mirror of
    /// `BytecodeProgram::has_null_coalesce_residual` — forces JIT deopt for
    /// programs that compile a null-coalescing `??` (the VM unwraps an
    /// `Option<T>` `Some(v) -> v` via `CoalesceProbe`; the JIT has no
    /// Option-unwrap lowering). NOT serialised.
    #[serde(skip, default)]
    pub has_null_coalesce_residual: bool,
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

    /// Per-slot fully-resolved `ConcreteType` for top-level locals.
    ///
    /// ADR-006 §2.7.5 conduit (top-level concrete-types side-table):
    /// propagated from `BytecodeProgram.top_level_local_concrete_types`
    /// through the linker so JIT compilation of linked programs can use
    /// the typed-array / TypedObject fast paths. Not serialised — the
    /// embedded `StructLayoutId` / `EnumLayoutId` are compile-time-local
    /// registry indices; cached-program loads fall through to the legacy
    /// NaN-boxed path.
    #[serde(skip, default)]
    pub top_level_local_concrete_types: Vec<shape_value::v2::ConcreteType>,

    /// Per-user-function per-MIR-slot `ConcreteType` side-table.
    ///
    /// ADR-006 §2.7.5 conduit (W12-jit-aggregate-non-array, 2026-05-12):
    /// LinkedProgram mirror of `Program.function_local_concrete_types`
    /// — propagated through the linker so JIT compilation of linked
    /// programs can use the TypedObject Aggregate short-circuit inside
    /// user-function bodies. Not serialised — same rationale as
    /// `top_level_local_concrete_types`.
    #[serde(skip, default)]
    pub function_local_concrete_types: Vec<Vec<shape_value::v2::ConcreteType>>,

    /// Per-user-function declared `ConcreteType` for the return value.
    ///
    /// ADR-006 §2.7.5 conduit (W12-jit-call-return-kind, 2026-05-12):
    /// LinkedProgram mirror of
    /// `Program.function_return_concrete_types` — propagated through
    /// the linker so the conduit can stamp Call-terminator destination
    /// slots from the callee's declared return type. Not serialised —
    /// same rationale.
    #[serde(skip, default)]
    pub function_return_concrete_types: Vec<shape_value::v2::ConcreteType>,

    /// ADR-006 §2.7.5 conduit (V3-S6b-jit-method-monomorph-conduit
    /// close, 2026-05-15): LinkedProgram mirror of
    /// `Program.monomorphized_method_call_sites` — propagated through
    /// the linker so the conduit producer can lift
    /// `function_return_concrete_types[specialized_idx]` into the
    /// destination slot's ConcreteType at `MirConstant::Method` Call-
    /// terminator sites. Not serialised — same rationale.
    #[serde(skip, default)]
    pub monomorphized_method_call_sites:
        HashMap<(shape_ast::ast::span::Span, Option<usize>), usize>,

    /// ADR-006 §2.7.5 conduit (cluster-2-cw-IB-class-b close, 2026-05-16):
    /// LinkedProgram mirror of `Program.value_call_return_concrete_types`
    /// — propagated through the linker so the conduit producer can stamp
    /// value-call `TerminatorKind::Call` destination slots from the
    /// closure-bound callee's inferred return `ConcreteType`. Not
    /// serialised — same rationale.
    #[serde(skip, default)]
    pub value_call_return_concrete_types:
        HashMap<(shape_ast::ast::span::Span, Option<usize>), shape_value::v2::ConcreteType>,

    /// ADR-006 §2.7.5 conduit (W10 jit-call-method-user-trait-fix close,
    /// 2026-05-17): LinkedProgram mirror of
    /// `Program.operator_trait_dispatch_sites` — propagated through the
    /// linker so the JIT consumer can re-emit user-type binary/unary
    /// trait-dispatch as method-call IR. Not serialised — same rationale.
    #[serde(skip, default)]
    pub operator_trait_dispatch_sites: HashMap<shape_ast::ast::span::Span, (String, u16)>,

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

    /// Closure spec §14.6 (H6.5): per-function `ClosureLayout` propagated
    /// from `BytecodeProgram.closure_function_layouts` so the linked
    /// program carries enough metadata for `op_make_closure`'s raw
    /// producer path. `None` entries indicate the function is not a
    /// closure body or the layout wasn't computed. Not serialised —
    /// programs loaded from disk fall back to the legacy `HeapValue::
    /// Closure` variant in the VM producer.
    #[serde(skip, default)]
    pub closure_function_layouts:
        Vec<Option<std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>>>,

    /// ADR-006 §2.7.24 Q25.C trait-object vtable registry. Keyed by
    /// `"Trait::ConcreteType"`. Threaded through from the
    /// content-addressed Program at link time so the VM `op_box_trait_object`
    /// handler can look up the vtable to build `Arc<TraitObjectStorage>`.
    /// Not serialised (Arc<VTable> is not a stable wire shape).
    #[serde(skip, default)]
    pub trait_vtables:
        std::collections::HashMap<String, std::sync::Arc<shape_value::value::VTable>>,

    /// R8 W8 Cluster A surface-and-stop flag (2026-05-25).
    ///
    /// Mirrors `BytecodeProgram::has_imported_const_inline`; propagated
    /// through the linker so the JIT executor (which receives the
    /// post-link `BytecodeProgram`) can refuse to JIT-compile the program
    /// when imported `pub const` identifiers were inlined-at-use as
    /// `PushConst(<value>)` by `compile_expr_identifier`. Triggers W12
    /// `[jit-fallback]` deopt to bytecode interpreter; root-cause fix in
    /// JIT identifier-eval lowering is v0.4 per
    /// `docs/v0.3-close-summary.md` §5.16. NOT serialised — compile-time
    /// state.
    #[serde(skip, default)]
    pub has_imported_const_inline: bool,

    /// R8 W9 B1 W17-marshal-return surface-and-stop flag (2026-05-25).
    /// Mirror of `Program::has_w17_marshal_residual` propagated through
    /// the linker so the JIT executor can refuse to JIT-compile programs
    /// containing direct stdlib-imported function calls. Triggers W12
    /// `[jit-fallback]` deopt to bytecode interpreter; root-cause fix in
    /// JIT ModuleFn dispatch (`jit_call_value` arm + `dispatch_module_fn_call`
    /// `todo!()`) is v0.4 per `docs/v0.3-close-summary.md` §5.16. NOT
    /// serialised — compile-time state.
    #[serde(skip, default)]
    pub has_w17_marshal_residual: bool,

    /// c4-4B `?`-operator (TryUnwrap) JIT surface-and-stop flag (2026-05-28).
    /// Mirror of `Program::has_try_unwrap_residual` propagated through the
    /// linker so the JIT executor can refuse to JIT-compile programs
    /// containing the `?` operator (whose `OpCode::TryUnwrap` semantics
    /// MIR collapses to a transparent copy at `mir/lowering/expr.rs:2594`,
    /// leaving the JIT-emitted Return arm to stamp `RETURN_TAG_I64` on
    /// heap-Result pointer bits). Triggers W12 `[jit-fallback]` deopt to
    /// bytecode interpreter. SURFACE-and-deopt per supervisor 2026-05-28
    /// c4-4B ratification (audit doc
    /// `docs/cluster-audits/v0.3.3/04-pointer-as-float-leak.md` §4B
    /// Sub-cluster — FN-REG-CORRECTNESS / RELEASE-BLOCKING).
    /// NOT serialised — compile-time state.
    #[serde(skip, default)]
    pub has_try_unwrap_residual: bool,

    /// ADR-006 §2.7.30 (GapA): mirror of
    /// `BytecodeProgram::has_reference_escape_promotion` — forces JIT deopt for
    /// programs whose functions return a reference via the `PromotedCell`
    /// escape carrier (no JIT lowering; VM is correct). NOT serialised.
    #[serde(skip, default)]
    pub has_reference_escape_promotion: bool,

    /// v0.3.3 book-gate `??` fix: mirror of
    /// `BytecodeProgram::has_null_coalesce_residual` — forces JIT deopt for
    /// programs that compile a null-coalescing `??` (the VM unwraps an
    /// `Option<T>` `Some(v) -> v` via `CoalesceProbe`; the JIT has no
    /// Option-unwrap lowering). NOT serialised.
    #[serde(skip, default)]
    pub has_null_coalesce_residual: bool,
}
