//! Bytecode compiler - translates AST to bytecode

use shape_ast::error::{Result, ShapeError, SourceLocation};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::blob_cache_v2::BlobCache;
/// Borrow mode for reference parameters - Shared (&) or Exclusive (&mut).
/// Kept for codegen even though the lexical borrow checker has been removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowMode {
    Shared,
    Exclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprResultMode {
    Value,
    PreserveRef,
}

/// Wave 1a PART A: per-binding call-site argument-type hint for a let-bound
/// closure, produced by the whole-program pre-pass and consumed by
/// `compile_expr_closure`. See `closure_callsite_param_hints`.
#[derive(Debug, Clone)]
pub(crate) enum ClosureCallsiteHint {
    /// All observed call sites agree (per arg slot). `types[i]` is the inferred
    /// `TypeAnnotation` for argument slot `i`, or `None` when that slot's type
    /// could not be inferred at any site. Soundness: only applied to params
    /// that have no explicit annotation and no HOF hint.
    Types(Vec<Option<shape_ast::ast::TypeAnnotation>>),
    /// Two call sites disagreed on an argument's type (e.g. `f(1)` and
    /// `f(2.0)`), or the binding name was bound to a closure literal in more
    /// than one place (shadowing). The hint is NOT applied — the closure keeps
    /// its existing rejection. Strict-typing: do NOT silently pick one type.
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ExprReferenceResult {
    pub raw_mode: Option<BorrowMode>,
    pub auto_deref_mode: Option<BorrowMode>,
}

/// A borrow place key used for encoding borrow targets in codegen.
pub type BorrowPlace = u32;
use crate::bytecode::{
    BuiltinFunction, BytecodeProgram, Constant, FunctionBlob, FunctionHash, Instruction, OpCode,
    Operand, Program as ContentAddressedProgram,
};
use crate::type_tracking::{NativeKind, TypeTracker, VariableTypeInfo};
use shape_ast::ast::{FunctionDef, Item, Program, Span, TypeAnnotation};
use shape_runtime::type_schema::SchemaId;
use shape_runtime::type_system::{
    InferenceFacts, Type, TypeAnalysisMode, TypeErrorWithLocation, analyze_program_full,
    checking::MethodTable,
};

// Sub-modules
pub(crate) mod comptime;
pub(crate) mod comptime_builtins;
// ADR-009 D1 (S4): the generated-symbol query surface — the ONE query API
// (spec §4.1) tooling uses to resolve generated declarations to identity +
// provenance ({SymbolId, checked-decl, application, generator locations})
// and to list them for workspace symbols. Consumed via
// `BytecodeCompiler::generated_symbol_query()`.
pub use comptime_builtins::capture_plan::{
    CaptureSiteRole, GENERATED_CAPTURE_ARTIFACT_CONFLICT_CODE,
    GeneratedCaptureBindingIdentity,
    GeneratedCaptureDescriptorView, GeneratedCaptureOccurrenceIdentity, GeneratedCapturePosition,
    GeneratedCaptureQuery, GeneratedCaptureQueryIssue, GeneratedCaptureSemanticType,
    GeneratedCaptureSite, GeneratedCaptureSlot, GeneratedCaptureSourceMap,
    GeneratedCaptureSpecialization, GeneratedCaptureSpecializationIdentity, GeneratedCaptureStage,
};
pub use comptime_builtins::expansion_provenance::{
    GeneratedNodePath, GeneratedSymbolProvenance, GeneratedSymbolTable, HygienicRole,
    HygienicSymbol, SourceAnchor, SymbolId,
};
pub use generation_reachability::program_may_generate;
pub(crate) mod comptime_concrete;
pub(crate) mod comptime_diagnostics;
pub(crate) mod comptime_target;
mod control_flow;
mod body_analysis_authority;
mod checked_body;
mod comptime_fragments;
mod template_specialization;
mod expressions;
mod functions;
mod functions_annotations;
mod functions_foreign;
mod generation_reachability;
mod helpers;
mod helpers_binding;
mod helpers_reference;
mod import_permissions;
pub(crate) mod literal_widen;
mod literals;
mod loops;
pub(crate) mod mir_schema_threading;
mod module_local_calls;
mod module_local_expr_calls;
mod module_local_expr_helpers;
mod module_local_expr_scopes;
pub(crate) mod monomorphization;
mod original_body_rewrite;
pub(crate) mod patterns;
pub(crate) mod post_inference_verify;
mod reference_flow;
mod statements;
pub mod string_interpolation;
mod trait_object_emission;

/// Loop compilation context
pub(crate) struct LoopContext {
    /// Break jump targets
    pub(crate) break_jumps: Vec<usize>,
    /// Continue jump target (usize::MAX = deferred, use continue_jumps)
    pub(crate) continue_target: usize,
    /// Optional local to store break values for expression loops
    pub(crate) break_value_local: Option<u16>,
    /// Whether a for-in iterator is on the stack (break must pop it)
    pub(crate) iterator_on_stack: bool,
    /// Drop scope depth when the loop was entered (for break/continue early exit drops)
    pub(crate) drop_scope_depth: usize,
    /// Forward-patched continue jumps for range counter loops where the
    /// increment block is after the body (so continue must forward-jump).
    pub(crate) continue_jumps: Vec<usize>,
}

/// Information about an imported symbol (fields used for diagnostics/LSP)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ImportedSymbol {
    /// Original name in the source module
    pub original_name: String,
    /// Module path the symbol was imported from
    pub module_path: String,
    /// High-level kind of the imported symbol (function, type, etc.)
    /// `None` for legacy inlining path where kind is not tracked.
    pub kind: Option<shape_ast::module_utils::ModuleExportKind>,
}

/// Imported annotation binding routed through a hidden synthetic module.
#[derive(Debug, Clone)]
pub(crate) struct ImportedAnnotationSymbol {
    /// Original annotation name in the source module.
    pub original_name: String,
    /// Source module path the annotation was imported from.
    pub _module_path: String,
    /// Hidden synthetic module name that owns the compiled annotation scope.
    pub hidden_module_name: String,
}

/// Module-scoped builtin function declaration with a runtime source module.
#[derive(Debug, Clone)]
pub(crate) struct ModuleBuiltinFunction {
    /// The callable name as exported by the runtime/native module.
    pub export_name: String,
    /// Original source module path that provides the runtime implementation.
    pub source_module_path: String,
}

/// Compiler-internal scope taxonomy for name resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ResolutionScope {
    Local,
    ModuleBinding,
    NamedImport,
    NamespaceImport,
    TypeAssociated,
    Prelude,
    SyntaxReserved,
    InternalIntrinsic,
}

impl ResolutionScope {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Local => "local scope",
            Self::ModuleBinding => "module scope",
            Self::NamedImport => "named import scope",
            Self::NamespaceImport => "namespace import scope",
            Self::TypeAssociated => "type-associated scope",
            Self::Prelude => "implicit prelude scope",
            Self::SyntaxReserved => "syntax-reserved scope",
            Self::InternalIntrinsic => "internal intrinsic scope",
        }
    }
}

/// Builtin lookup result annotated with the scope class it currently belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuiltinNameResolution {
    Surface {
        builtin: BuiltinFunction,
        scope: ResolutionScope,
    },
    InternalOnly {
        builtin: BuiltinFunction,
        scope: ResolutionScope,
    },
}

impl BuiltinNameResolution {
    pub(crate) const fn scope(self) -> ResolutionScope {
        match self {
            Self::Surface { scope, .. } | Self::InternalOnly { scope, .. } => scope,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunctionReturnReferenceSummary {
    pub param_index: usize,
    pub mode: BorrowMode,
    pub projection: Option<Vec<crate::mir::types::ProjectionStep>>,
}

impl From<crate::mir::analysis::ReturnReferenceSummary> for FunctionReturnReferenceSummary {
    fn from(value: crate::mir::analysis::ReturnReferenceSummary) -> Self {
        Self {
            param_index: value.param_index,
            mode: match value.kind {
                crate::mir::types::BorrowKind::Shared => BorrowMode::Shared,
                crate::mir::types::BorrowKind::Exclusive => BorrowMode::Exclusive,
            },
            projection: value.projection,
        }
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
        capture_kinds: Vec<NativeKind>,
        capture_names: Vec<String>,
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
                    // Closure spec H5: `MakeClosure` now carries the function id
                    // (plus the escape flag) in a `ClosureAlloc` operand when the
                    // closure escapes. Treat it exactly like `Operand::Function`
                    // for dependency tracking — the content-addressed blob must
                    // record the closure's compiled body as a dependency.
                    Operand::ClosureAlloc { fid, .. } => {
                        let global_idx = fid.0 as usize;
                        if !func_remap.contains_key(&fid.0) {
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
        let mut local_instructions: Vec<Instruction> = global_instructions
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
                        // Closure spec H5: parallel remap for `ClosureAlloc`.
                        Operand::ClosureAlloc { fid, .. } => {
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

        // integration A6 (§4.2.0): `foreign_dependencies` is ordered,
        // first-use-deduped (was sorted+deduped), and every `CallForeign`
        // operand in the blob's instruction stream is rewritten from the
        // program-level foreign index to the blob-local ordinal — the position
        // of that entry's content hash in this blob's `foreign_dependencies`.
        // This mirrors how `Call` / `Constant::Function` references are
        // hash-normalized at blob build: after the rewrite the hashed
        // instruction stream contains only blob-local ordinals that are a
        // deterministic function of the blob's own instruction sequence, so a
        // blob's content hash no longer depends on the origin program's
        // foreign-table ordering (cross-program blob dedup, C10 fix). The
        // linker's `ForeignFunction` remap (`linker.rs`) inverts this ordinal →
        // hash → assembled-table index at every consuming node.
        //
        // Build invariant (§4.2.0-1): an entry whose `content_hash` is `None`
        // was silently skipped here pre-WF-2A. Under the ordinal rewrite a skip
        // would shift every subsequent ordinal, so a missing hash is now an
        // internal compile error, never a skip — the compiler always calls
        // `compute_content_hash()` before blob assembly
        // (`functions_foreign.rs`).
        let mut foreign_deps: Vec<[u8; 32]> = Vec::new();
        let mut foreign_hash_to_ordinal: HashMap<[u8; 32], u16> = HashMap::new();
        for instr in &mut local_instructions {
            if instr.opcode == crate::bytecode::OpCode::CallForeign {
                if let Some(Operand::ForeignFunction(prog_idx)) = instr.operand {
                    let entry = program
                        .foreign_functions
                        .get(prog_idx as usize)
                        .unwrap_or_else(|| {
                            panic!(
                                "CallForeign operand {prog_idx} in blob '{}' does not index a \
                                 valid foreign function (table len {})",
                                self.name,
                                program.foreign_functions.len(),
                            )
                        });
                    let hash = entry.content_hash.unwrap_or_else(|| {
                        panic!(
                            "foreign function '{}' referenced by CallForeign in blob '{}' has no \
                             content_hash at blob assembly — hash presence is a build invariant \
                             (integration A6 §4.2.0-1); compute_content_hash() must run first",
                            entry.name, self.name,
                        )
                    });
                    let ordinal = *foreign_hash_to_ordinal.entry(hash).or_insert_with(|| {
                        let ord = foreign_deps.len() as u16;
                        foreign_deps.push(hash);
                        ord
                    });
                    instr.operand = Some(Operand::ForeignFunction(ordinal));
                }
            }
        }

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
            capture_kinds,
            capture_names,
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
    Strict,
    RecoverAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileDiagnosticMode {
    FailFast,
    RecoverAll,
}

/// cluster-2-cw-IB-class-b (2026-05-16, supervisor R3 binding-ratified):
/// retained closure-literal peek used to re-run body return-type
/// inference at the value-call site with caller-context arg type hints.
/// Stored per-local-slot in `BytecodeCompiler.local_callable_closure_bodies`
/// at let-binding time (`update_callable_binding_from_expr` /
/// `FunctionExpr` arm). The body is the AST `Vec<Statement>` clone — no
/// bytecode-lowering happens at lookup time; the inference walker only
/// inspects AST shape.
#[derive(Debug, Clone)]
pub struct ClosureBodyPeek {
    /// Formal parameters of the closure literal (`|inner|` →
    /// `[FunctionParameter { pattern: Identifier("inner"), .. }]`).
    pub params: Vec<shape_ast::ast::FunctionParameter>,
    /// Closure literal body statements.
    pub body: Vec<shape_ast::ast::Statement>,
    /// Explicit `-> T` return annotation, if any.
    pub return_type: Option<shape_ast::ast::TypeAnnotation>,
    /// Compiled-function index assigned to the closure body by
    /// `compile_expr_closure`. `None` until the closure literal is
    /// actually lowered (the peek is built from the AST; the function
    /// index is assigned at compile-emission time). Used by the value-
    /// call propagation path to retroactively patch
    /// `mir.local_typed_array_element_types` for the closure body's
    /// MIR-side typed-array param seed.
    pub function_index: Option<usize>,
}

/// Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21) — identifies the
/// binding slot that holds a bare empty-array accumulator awaiting a
/// downstream-`.push()`-resolved element kind.
///
/// A `let mut out = []` either lands in a function-body local slot or a
/// top-level module binding; both are keyed by their `u16` index. The push
/// site (`compile_expr_method_call`) resolves the receiver to the same key,
/// patches the placeholder allocator, and promotes the binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EmptyArrayAccumulatorKey {
    /// Function-body local slot index.
    Local(u16),
    /// Top-level module-binding index.
    ModuleBinding(u16),
}

/// Phase 4b Round 6 WS-1b W16.2-C residual — a pending bare empty-array
/// accumulator. Carries the placeholder `NewArray(0)` instruction index so
/// the first `.push()` (or the end-of-compilation finalizer) can patch it.
#[derive(Debug, Clone)]
pub(crate) struct EmptyArrayAccumulator {
    /// Instruction index of the placeholder `OpCode::NewArray` emitted at the
    /// empty-array literal site. Patched in-place to `kind.new_opcode()` once
    /// the element kind is proven.
    pub alloc_instr_idx: usize,
    /// Source location of the empty-array literal — used for the clean
    /// "element type un-resolvable" diagnostic if no push ever resolves it.
    pub literal_loc: Option<SourceLocation>,
    /// The accumulator variable's source name (for diagnostics).
    pub var_name: String,
}

/// Source of a slot-scoped concrete binding fact consumed by monomorphization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingConcreteFactSource {
    DeclaredAnnotation,
    FunctionSignature,
    StructuralInitializer,
    MonomorphizedCallReturn,
    EmptyArrayAccumulator,
    ArrayPushElement,
    IteratorElement,
    MatchPayload,
}

/// Explicit slot-scoped concrete binding fact.
///
/// These facts are derived from a single proof point and are the transition
/// carrier for non-span VM projections that runtime `InferenceFacts` cannot
/// own directly, such as post-monomorphized method-call returns.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BindingConcreteFact {
    pub concrete_type: shape_value::v2::ConcreteType,
    pub source: BindingConcreteFactSource,
}

/// Compiler state
pub struct BytecodeCompiler {
    /// The program being built
    pub(crate) program: BytecodeProgram,

    /// Current function being compiled
    pub(crate) current_function: Option<usize>,

    /// Scoped authority for compiling a byte-identical generated function body
    /// under a distinct emission identity. The owned semantic-owner key may be
    /// consulted only while `current_function` is the exact emission id.
    active_body_analysis_authority: Option<body_analysis_authority::ActiveBodyAnalysisAuthority>,

    /// Local variable mappings (name -> index)
    pub(crate) locals: Vec<HashMap<String, u16>>,

    /// ADR-009 E3 (S1, U10): monotonic nonce feeding the hygienic-symbol mint
    /// for generated locals (`declare_hygienic_local`). Each mint is
    /// globally distinct within a compilation unit, so two applications of one
    /// annotation mint distinct non-forgeable tokens. Never a schema identity
    /// — it only disambiguates a `HygienicSymbol`, which is what keys the
    /// scope name table (via its unspellable descriptor), not the nonce.
    pub(crate) hygienic_local_nonce: u64,

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

    /// Closure function indices discovered during bytecode compilation of the
    /// current function. Each entry is (closure_function_name, function_index).
    /// Used to back-patch MIR ClosurePlaceholder/ClosureCapture after compilation.
    pub(crate) closure_function_ids: Vec<(String, u16)>,

    /// Registry of closure capture layouts (v2 closure specialization Phase A).
    ///
    /// Keyed on capture signature (`Vec<ConcreteType>`): two closures with
    /// identical capture signatures share a `ClosureTypeId`. Phase C will
    /// consume this to extend the monomorphization cache key.
    pub(crate) closure_registry: shape_value::v2::closure_layout::ClosureRegistry,

    /// Mapping from closure function index to its `ClosureTypeId`.
    /// Populated alongside `closure_function_ids` as each closure literal is
    /// lowered. Phase C reads this to key monomorphization on the closure type.
    pub(crate) closure_type_ids: Vec<(u16, shape_value::v2::concrete_type::ClosureTypeId)>,

    /// ADR-009 C1 — per-closure [`CapturePack`], one entry per closure literal,
    /// keyed by the closure's `func_idx` (`CapturePack::closure`). Produced by
    /// the ONE selector (`comptime_builtins::capture_plan`) and consumed by
    /// `compiler_impl_reference_model` to build the per-function
    /// `ClosureLayout`. Replaces the former `closure_capture_kinds` vector:
    /// the pack carries the emitted capture kind AND the body's access
    /// discipline together, so the two can no longer disagree. A closure with
    /// no pack (registered via a legacy/test path) falls back to the
    /// registry's all-immutable layout, matching the pre-fusion behaviour.
    ///
    /// [`CapturePack`]: crate::compiler::comptime_builtins::capture_plan::CapturePack
    pub(crate) closure_capture_packs:
        Vec<crate::compiler::comptime_builtins::capture_plan::CapturePack>,

    /// Distributed §4.4 — per-closure captured *variable names*, in declaration
    /// order, keyed by closure function index. Populated alongside
    /// `closure_capture_packs` (same `captured_vars` source). Stamped into the
    /// non-hash `FunctionBlob.capture_names` so the remote-capture-refusal path
    /// can name the offending variable. A missing entry yields empty names (the
    /// refusal message falls back to `capture #i`).
    pub(crate) closure_capture_names: Vec<(u16, Vec<String>)>,

    /// Registry of `Function<A, R>` signatures (v2 closure specialization
    /// Phase F). `FunctionTypeId`s assigned here are written into
    /// `TypedClosureHeader.type_id` in the JIT path and consumed by
    /// `CallFunctionIndirect` to pick a Cranelift `call_indirect` signature.
    pub(crate) function_type_registry:
        shape_value::v2::function_type_registry::FunctionTypeRegistry,

    /// Mapping from closure function index to the `FunctionTypeId` of its
    /// callable signature (params + return). Populated alongside
    /// `closure_type_ids` when a closure literal is lowered. The JIT reads
    /// this to emit the per-`ClosureTypeId` direct-call signature and the
    /// polymorphic `CallFunctionIndirect` signature.
    pub(crate) function_type_ids: Vec<(u16, shape_value::v2::concrete_type::FunctionTypeId)>,

    /// Phase F one-shot: if set, the next closure-literal emission uses
    /// `MakeClosureHeap` instead of the legacy `MakeClosure`. The flag is
    /// consumed (reset to false) as soon as `compile_expr_closure`
    /// finishes emitting the opcode. Callers that want to force the heap
    /// ABI (e.g. the return-statement compiler when the returned value is
    /// a closure literal) set this right before lowering the closure.
    pub(crate) emit_make_closure_heap_next: bool,

    /// Wave E+5-cleanup task #108 side-channel: native-kind hint for
    /// `GetProp` emit sites where the producer-flip in `op_get_prop`
    /// pushes raw native bits (I64 / Timestamp / F64 / Bool field tags
    /// against non-heap slots). Keyed by emitted-instruction index;
    /// consumed by `last_emitted_native_kind`'s `GetProp` arm so the
    /// host-boundary synthesizer re-tags raw bits per the recorded
    /// kind. `GetProp` is `Instruction::simple` (no operand) so neither
    /// operand-decode nor walk-back recovers the field tag — the
    /// compiler must record the resolved kind at the emit site
    /// (`compile_expr_property_access`).
    pub(crate) get_prop_native_kinds: HashMap<usize, crate::type_tracking::StorageHint>,

    /// When compiling a DataTable closure method (e.g. dt.filter(row => ...)),
    /// this holds the (schema_id, type_name) to tag the closure's row parameter as RowView.
    pub(crate) closure_row_schema: Option<(u32, String)>,

    /// Strict-typing-sweep (Cluster 3): bidirectional closure inference for
    /// HOF method calls. When the next compiled closure literal is the
    /// argument to `arr.map(|x| …)` / `.filter` / `.reduce` / etc., the
    /// outer `compile_expr_method_call` resolves the receiver's element
    /// type and stashes per-user-param `TypeAnnotation` hints here. The
    /// closure compile reads these hints and attaches them to params with
    /// no explicit annotation, then clears the field. The vector indexes
    /// USER params only (excludes synthesized capture-params).
    pub(crate) pending_closure_param_types: Option<Vec<Option<shape_ast::ast::TypeAnnotation>>>,

    /// Wave 1a PART A (bidirectional let-bound-closure param inference).
    ///
    /// A `let f = |a, b| a + b` binding compiles the closure body EAGERLY at
    /// the let-site, before any `f(2, 3)` call site is seen. The body's
    /// unannotated params `a`/`b` then surface "Cannot infer types for binary
    /// operation Add: operand types are unknown and unknown" because neither
    /// the HOF receiver-element hint nor the body literal-pairing heuristic
    /// can resolve them.
    ///
    /// This map is populated by a whole-program pre-pass
    /// (`collect_closure_callsite_param_hints`) that, for every binding whose
    /// initializer is a closure literal, scans the program for DIRECT calls
    /// `name(args)` and infers each argument's `TypeAnnotation`. The hint is
    /// keyed on the binding name; `compile_expr_closure` consults it via
    /// `pending_variable_name` and seeds the still-unannotated user params.
    ///
    /// Soundness (this is the strict-typing core — inference must be CORRECT):
    /// * A name called with CONFLICTING argument types at different sites, or
    ///   a name bound to a closure literal in more than one place (shadowing),
    ///   maps to `ClosureCallsiteHint::Conflict` — the hint is then NOT
    ///   applied and the closure keeps its existing rejection (do NOT silently
    ///   pick one type).
    /// * Only literal / structurally-obvious argument types are inferred; an
    ///   un-inferable argument contributes `None` for that slot (the param
    ///   stays unannotated and the body's own heuristics / clean error apply).
    /// * `int` and `number` do not unify — distinct annotations at the same
    ///   slot are a conflict.
    pub(crate) closure_callsite_param_hints: std::collections::HashMap<String, ClosureCallsiteHint>,

    /// Producer-function hint key while compiling a returned callable value.
    ///
    /// `pending_variable_name` covers `let f = |...|` and now
    /// `let f = match/if { |...| }` initializers. Returned callable producers
    /// (`fn chooser(){ if c { |...| } else { |...| } }`) have no binding name at
    /// the closure literal site, so explicit/implicit return compilation scopes
    /// this to the producer function name only while compiling that returned
    /// callable expression.
    pub(crate) pending_callable_hint_name: Option<String>,

    /// W21 HOF inference: bidirectional hints for closures returned by a
    /// function and then invoked through a result binding.
    ///
    /// Example: `let add = make_op("add"); add(10, 5)` proves that the
    /// closure literals returned by `make_op` have `(int, int)` user params.
    /// The pre-pass records the hint by producer function name, not by the
    /// result binding, so `compile_expr_closure` can consume it while compiling
    /// the producer function body. Conflicts are represented exactly like
    /// `closure_callsite_param_hints`.
    pub(crate) returned_closure_callsite_param_hints:
        std::collections::HashMap<String, ClosureCallsiteHint>,

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

    // U4-4 (T2): the standalone `last_expr_numeric_type` per-expression
    // register is DELETED. It was a SECOND source of truth for "is this
    // operand int / number / decimal / width" that competed with the engine
    // span-keyed `resolved_expr_types: HashMap<Span, Type>` table (SB-7
    // drift). NumericType is now derived from that one resolved Type at the
    // opcode-selection / storage-hint point via `numeric_type_of`
    // (`binary_ops.rs`) → `inferred_type_to_numeric` (`numeric_ops.rs`), the
    // SOLE Type→NumericType derivation. The `NumericType` enum itself survives
    // as the emit-time opcode index.
    /// E+5.5 Unit C step 2: captured top-level program return-kind, snapshotted
    /// right after the last item compiles (before drop-scope emission and
    /// Halt overwrite `last_expr_*`). Consumed by
    /// `populate_program_storage_hints` to populate
    /// `top_level_frame.return_kind` so the host boundary reads the kind
    /// off the parallel-kind track (per ADR-006 §2.7.7 — the deleted
    /// ValueWord-tagged synthesis is gone).
    ///
    /// Per ADR-006 §2.7.5.1 (compiler-tier intermediate state policy),
    /// "kind not yet stamped" is carried as `Option<StorageHint>` —
    /// `None` is the post-bulldozer replacement for the deleted
    /// `StorageHint::Unknown` sentinel.
    pub(crate) top_level_program_return_kind: Option<crate::type_tracking::StorageHint>,

    /// Result mode for the expression currently being compiled.
    pub(crate) current_expr_result_mode: ExprResultMode,

    /// Whether the last compiled expression left a raw reference on the stack.
    ///
    /// `auto_deref_mode` is only set for propagated ref results (identifier loads,
    /// ref-returning calls) that should implicitly dereference in value contexts.
    /// Explicit `&expr` results keep `raw_mode` without enabling auto-deref.
    pub(crate) last_expr_reference_result: ExprReferenceResult,

    /// Known pass modes for local callable bindings (closures / function aliases).
    pub(crate) local_callable_pass_modes: HashMap<u16, Vec<ParamPassMode>>,

    /// Known safe return-reference summaries for local callable bindings.
    pub(crate) local_callable_return_reference_summaries:
        HashMap<u16, FunctionReturnReferenceSummary>,

    /// Known pass modes for module-binding callable values.
    pub(crate) module_binding_callable_pass_modes: HashMap<u16, Vec<ParamPassMode>>,

    /// Known safe return-reference summaries for module-binding callable values.
    pub(crate) module_binding_callable_return_reference_summaries:
        HashMap<u16, FunctionReturnReferenceSummary>,

    /// cluster-2-cw-IB-class-b (2026-05-16, supervisor R3 binding-
    /// ratified): retained closure-literal body for local `let f = |..|
    /// ..` bindings. Populated at let-binding time by
    /// `update_callable_binding_from_expr`'s `FunctionExpr` arm;
    /// consumed at `compile_expr_function_call`'s value-call branch when
    /// re-running closure-body return-type inference with caller-context
    /// arg types.
    ///
    /// The retained body is the AST `Vec<Statement>` clone (no lowering
    /// occurs at lookup time — the inference walker only inspects AST
    /// shape, never emits bytecode for the body). Released on
    /// `clear_callable_binding` and on per-function compilation
    /// snapshot/restore alongside the other local callable maps.
    ///
    /// Memory cost: bounded by the number of local closure bindings in
    /// the active function frame; the body is the same AST already held
    /// by the parent `Statement::VariableDecl` initializer, just held
    /// for the lifetime of the enclosing function-compile pass to avoid
    /// re-walking the AST at every value-call site. Released when the
    /// enclosing function compile completes.
    pub(crate) local_callable_closure_bodies: HashMap<u16, ClosureBodyPeek>,

    /// cluster-2-cw-IB-class-b: module-binding variant of the closure
    /// body peek. Covers top-level / REPL `let f = |..|` bindings whose
    /// slots live in the module-binding space (not the local-slot
    /// space). Populated/cleared by `update_callable_binding_from_expr`.
    pub(crate) module_binding_callable_closure_bodies: HashMap<u16, ClosureBodyPeek>,

    /// ADR-006 §2.7.24 Q25.C trait-object emission (Wave 2.6 round-2):
    /// per-local-slot trait name for `let a: dyn Animal = ...` bindings.
    /// Consumed by `compile_expr_method_call` (`expressions/function_calls.rs`)
    /// to route method dispatch through `OpCode::DynMethodCall` instead
    /// of the standard `OpCode::CallMethod` path. Empty for non-dyn
    /// locals.
    pub(crate) dyn_locals: HashMap<u16, String>,

    /// ADR-006 §2.7.24 Q25.C trait-object emission: per-module-binding
    /// trait name for top-level `let a: dyn Animal = ...` declarations.
    /// Same role as `dyn_locals` but for the module-binding slot space.
    pub(crate) dyn_module_bindings: HashMap<u16, String>,

    /// Named functions that safely return one reference parameter unchanged.
    pub(crate) function_return_reference_summaries: HashMap<String, FunctionReturnReferenceSummary>,

    /// The return-reference summary of the function currently being compiled, if any.
    pub(crate) current_function_return_reference_summary: Option<FunctionReturnReferenceSummary>,

    /// ADR-006 §2.7.30 (FlipLive): true iff the function currently being
    /// compiled declares a `&T` / `&mut T` (Borrow) RETURN type. The
    /// reference-escape→RC promotion floor (`return &local`) is admitted ONLY
    /// when the function expresses this reference-return contract — the
    /// `-> &T` annotation is what drives the sound PromotedCell carrier on the
    /// return path. An UNANNOTATED `return &local` does NOT promote soundly
    /// (the raw ref bits escape without an owning carrier → dangling ref /
    /// UAF), so it keeps its B0003 reject via the compiler guard at the
    /// `Statement::Return` + implicit-return sites.
    pub(crate) current_function_returns_borrow: bool,

    /// Numeric-conversion §4 literal adoption (return-context widening, THE RULE
    /// user 2026-06-01): the declared return-type annotation of the function
    /// currently being compiled, when it is present. Drives the int-literal →
    /// `number` re-lowering at the explicit `Statement::Return(expr)` site so a
    /// `fn g() -> number { return 5 }` lowers `5` to the `Number` literal `5.0`
    /// (Float64-kinded), NOT an Int64 constant laid into a Float64 return slot
    /// (the bit-reinterpret hole). Saved/restored around each function-body
    /// compile alongside the other `current_function_*` state. The implicit
    /// tail-return site reads `func_def.return_type` directly.
    pub(crate) current_function_return_type: Option<shape_ast::ast::TypeAnnotation>,

    /// Expected result annotation for the expression currently being compiled
    /// in an annotated assignment/return context. Used narrowly by generic
    /// zero-arg calls whose type parameter appears only in the return type
    /// (for example `set::new<T>() -> Set<T>`). Argument-bearing calls still
    /// bind from their arguments; this is not a runtime fallback.
    pub(crate) pending_expected_call_return_type: Option<shape_ast::ast::TypeAnnotation>,

    /// ADR-006 §2.7.30 (escape-Drop-deferral): the local slot index of a
    /// Drop-bearing value that is being RETURNED by-value from the current
    /// function (`fn make() -> R { let r = R{..}; return r }`). When set,
    /// `emit_drops_for_early_exit` SKIPS the `DropCall` for this slot — the
    /// value's ownership (and its `Drop`) moves to the caller, so dropping
    /// it at the callee's scope exit would run the user `Drop::drop` body a
    /// SECOND time (the caller drops it again when its binding leaves
    /// scope) — the bind-then-return double-drop. The `LoadLocal` clone +
    /// `truncate_stack` slot-release already balance the refcount; only the
    /// spurious user-`Drop` invocation needs suppressing. Scoped to a
    /// single `Statement::Return` lowering (set immediately before
    /// `emit_drops_for_early_exit`, cleared immediately after).
    pub(crate) return_escape_drop_skip_local: Option<u16>,

    /// ADR-006 §2.7.30.4 (escape-Drop-deferral, closure-capture arm): local
    /// slot indices of Drop-bearing values captured by an ESCAPING closure
    /// (`let r = R{..}; let f = || r.id; f`). The closure outlives the
    /// current frame, so the capture must stay ALIVE for the closure's
    /// lifetime — running the user `Drop::drop` at the capturing scope's exit
    /// is a use-after-finalize (the returned closure reads a value whose
    /// `drop()` body already ran). When a slot is in this set,
    /// `emit_drops_for_early_exit` / `pop_drop_scope` SKIP its user-`Drop`
    /// `DropCall`. The slot's refcount share is still released — by the
    /// function-teardown `truncate_stack(bp)` at `op_return_value`
    /// (control_flow/mod.rs:984), a plain `drop_with_kind` that never
    /// dispatches user `Drop::drop` — so the escaping closure's own capture
    /// share keeps the referent alive until the closure itself is released.
    /// This is the closure-capture analogue of `return_escape_drop_skip_local`.
    /// Function-scoped: saved/restored in lockstep with `drop_locals` in
    /// `compile_function` so a capture skip in a nested/outer function does
    /// not leak its (frame-local) slot index into a sibling frame.
    pub(crate) closure_escape_drop_skip_locals: HashSet<u16>,

    /// ADR-006 §2.7.30.4 (escape-Drop-deferral, closure-capture arm): maps a
    /// closure-binding local slot (`let f = || r.id` → slot of `f`) to the
    /// frame-local slots of its Drop-bearing captures (slot of `r`). The
    /// literal escape signal (`emit_make_closure_heap_next`) only fires for a
    /// closure LITERAL in return position (`return || ...`); the far more
    /// common bind-then-return form (`let f = || ...; f`) is invisible to it.
    /// This table lets the return site (`return f` / implicit tail `f`)
    /// recognise that returning `f` escapes its captures, and mark them in
    /// `closure_escape_drop_skip_locals`. Populated at the `let f =
    /// <closure-literal>` binding from `pending_closure_capture_drop_locals`.
    /// Function-scoped (saved/restored with `drop_locals`).
    pub(crate) closure_binding_capture_drop_locals: HashMap<u16, Vec<u16>>,

    /// Transient carrier: the Drop-bearing captured local slots of the closure
    /// literal most recently lowered by `compile_expr_closure`. Consumed by
    /// the enclosing `let f = <closure-literal>` binding to populate
    /// `closure_binding_capture_drop_locals`. Overwritten on every closure
    /// lowering and taken on consume, so a stale value from a closure used as
    /// a call argument (`foo(|| r.id)`) is never mis-associated — consumption
    /// is gated on the initializer being a direct `Expr::FunctionExpr`.
    pub(crate) pending_closure_capture_drop_locals: Option<Vec<u16>>,

    /// ADR-006 §2.7.30 (escape-Drop-deferral, closure-capture arm, WF-1C
    /// lane b — CONSUMER side). Per-scope stack (lockstep with `drop_locals`)
    /// of local slots that received an ESCAPING closure from a call return
    /// (`let read = make_reader()`). At the slot's scope-exit a
    /// `LoadLocal; DropClosureCaptures` pair is emitted so the closure's
    /// Drop-bearing captures run their user `Drop::drop` exactly once —
    /// deferred from the *capturing* scope (where the producer suppressed
    /// them via `closure_escape_drop_skip_locals`) to the escaping closure's
    /// lifetime. The runtime opcode is a no-op for a non-closure value or a
    /// closure with no Drop-bearing captures, so registration is
    /// conservative: a `let x = <call>` whose callee's declared return type
    /// cannot be statically ruled out as a closure. Gated at bind time on the
    /// program having at least one `impl Drop` (`drop_type_info` non-empty) so
    /// non-Drop programs emit nothing. Pushed/popped in `push_drop_scope` /
    /// `pop_drop_scope`; saved/restored per-function in `compile_function`.
    pub(crate) closure_capture_drop_locals: Vec<Vec<u16>>,

    /// Type inference engine for match exhaustiveness and type checking
    pub(crate) type_inference: shape_runtime::type_system::inference::TypeInferenceEngine,

    /// Canonical facts from the same best-effort inference pass that populated
    /// `resolved_expr_types`. Later compiler phases derive function signature
    /// projections from this carrier directly instead of maintaining parallel
    /// per-param side tables.
    pub(crate) inference_facts: InferenceFacts,

    /// T1 KEYSTONE (strict-flip, 2026-06-22): POST-SOLVE per-expression type
    /// table keyed by source span, harvested from the reference-model inference
    /// pass (which walks the FULL program, including function bodies). This is
    /// the ROOT fix for the recurring static-type-erasure class:
    /// `BytecodeCompiler::infer_expr_type` consults this table FIRST (before the
    /// per-context patch ladder), so the resolved type of a
    /// collection-dispatch / match-arm / method-result local reaches the use
    /// site directly instead of erasing to `unknown`. Holds ONLY fully-resolved
    /// types (the engine drops any entry that stayed a free variable
    /// post-solve), so a hit is a genuine proof — never an Unknown-default.
    ///
    /// U4-3 (2026-06-23): this table + the per-context proof patches are now the
    /// SOLE L3 inference authority. The fallback `type_inference.infer_expr`
    /// re-derivation (module-scope, blind to function-body locals) is DELETED:
    /// a span-table MISS that no patch proves is a surface-and-stop compile
    /// error, never a re-derivation.
    pub(crate) resolved_expr_types: HashMap<shape_ast::ast::Span, shape_runtime::type_system::Type>,

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
    /// Imported annotations: local_name -> ImportedAnnotationSymbol
    pub(crate) imported_annotations: HashMap<String, ImportedAnnotationSymbol>,
    /// Opaque evidence that annotation declaration installation completed.
    annotation_declarations:
        statements::annotation_declarations::AnnotationDeclarationState,
    /// R8 W8 Cluster A (2026-05-24): imported `pub const NAME = expr`
    /// initializers, keyed by the local binding name (alias-respecting).
    /// At identifier-load time, references to these names compile to an
    /// inlined `PushConst(<comptime-value>)` rather than a
    /// `LoadModuleBinding` — the dispatch's "use the existing
    /// comptime-evaluated-constant mechanism (the `Constant` pool); NOT a
    /// new opcode, NOT a deferred init runtime computation" binding.
    /// ADR-006 §2.7.5 stamp-at-compile-time invariant preserved: the
    /// constant's kind is stamped from the literal's shape at compile time.
    pub(crate) imported_consts: HashMap<String, shape_ast::ast::Expr>,
    /// Qualified builtin function declarations available as module-scoped callables.
    pub(crate) module_builtin_functions: HashMap<String, ModuleBuiltinFunction>,
    /// Module namespace bindings introduced by `use module.path`.
    /// Used to avoid UFCS rewrites for module calls like `duckdb.connect(...)`.
    pub(crate) module_namespace_bindings: HashSet<String>,
    /// Imported synthetic/local module path -> original source module path.
    /// Used when code inside a wrapper module needs to dispatch to native exports
    /// from the underlying source module.
    pub(crate) module_scope_sources: HashMap<String, String>,
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
    /// Sweep phase 3c.x: per-(enum, variant) struct-variant field
    /// annotations. The schema-level `EnumVariantInfo` only carries field
    /// counts (`payload_fields`) and uses `__payload_N` field names with
    /// `FieldType::Any`, so the named-field types of `enum E { V { x: int,
    /// y: int } }` are otherwise lost at pattern compile time. Populated
    /// by `register_enum`; consumed by `compile_typed_enum_binding` (struct
    /// arm) so `match m::E::V { x, y } => x + y` propagates int onto x and
    /// y.
    pub(crate) enum_struct_variant_fields:
        HashMap<(String, String), Vec<(String, shape_ast::ast::TypeAnnotation)>>,
    /// R8 W7: per-(enum, variant) tuple-payload positional type
    /// annotations. Symmetric to `enum_struct_variant_fields` for
    /// tuple variants. The runtime schema collapses tuple payloads
    /// into `__payload_N: Any`, so per-position types of
    /// `enum E { V(string) }` are otherwise lost at pattern-compile
    /// time. Populated by `register_enum`; consumed by
    /// `compile_typed_enum_binding` (tuple arm) so `match E::V(id) =>
    /// id + "!"` propagates `string` onto `id`.
    pub(crate) enum_tuple_variant_fields:
        HashMap<(String, String), Vec<shape_ast::ast::TypeAnnotation>>,
    /// Cached const specializations keyed by `(base_name + const-arg fingerprint)`.
    pub(crate) const_specializations: HashMap<String, usize>,
    /// Monotonic counter for unique specialization symbol names.
    pub(crate) next_const_specialization_id: u64,
    /// Const-parameter bindings for specialized function symbols.
    /// These bindings are exposed to comptime handlers as typed module_bindings.
    /// Kinded carrier per ADR-006 §2.7 / Q7 (`KindedSlot`); the prior
    /// `shape_value::ValueWord` shape was deleted by the strict-typing
    /// bulldozer and the comptime ABI in
    /// `compiler/comptime.rs:execute_comptime_with_annotation_handler`
    /// already migrated to `(String, KindedSlot)` pairs.
    pub(crate) specialization_const_bindings:
        HashMap<String, Vec<(String, shape_value::KindedSlot)>>,

    /// Struct type definitions: type_name -> (field_names in order, definition span)
    pub(crate) struct_types: HashMap<String, (Vec<String>, shape_ast::ast::Span)>,
    /// Generic metadata for struct types used to instantiate runtime type names
    /// (e.g. `MyType<number>`) at struct-literal construction sites.
    pub(crate) struct_generic_info: HashMap<String, StructGenericInfo>,

    /// ADR-009 §4.1 (ticket A1, S1): the single per-compilation-unit semantic
    /// freeze, installed exactly once at the registration-complete barrier
    /// (`install_semantic_freeze`) — before Phase 1 in
    /// `compile_with_graph_and_prelude` when compiling with a module graph
    /// (the unit is root + dependencies, and imported-module comptime sites
    /// execute in Phase 1), else in `compile()`. `None` only before the
    /// barrier; comptime sites are rewired onto this handle in S2/S3 — a
    /// comptime site that cannot obtain it is a compile error, never an empty
    /// snapshot.
    pub(crate) semantic_freeze:
        Option<std::sync::Arc<comptime_builtins::semantic_freeze::SemanticFreeze>>,
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

    /// Phase 3e: inferred FieldType for hoisted fields, when the assigned
    /// RHS is a simple literal whose primitive type is statically known.
    /// Maps variable name → property name → inferred FieldType.
    ///
    /// Used by `compile_typed_object_literal` to register the schema with
    /// concrete primitive types (I64, F64, Bool, String) instead of falling
    /// back to FieldType::Any. Without this, `let mut a = { x: 10 }; a.y =
    /// 20; a.x + a.y` types `a.y` as Any in the schema, so the binary-op
    /// numeric path declines and trait dispatch fires (which has no runtime
    /// handler for `int.add`).
    pub(crate) hoisted_field_types:
        HashMap<String, HashMap<String, shape_runtime::type_schema::FieldType>>,

    /// When compiling a variable initializer, the name of the variable being assigned to.
    /// Used by compile_typed_object_literal to include hoisted fields in the schema.
    pub(crate) pending_variable_name: Option<String>,

    /// Binder span for the same initializer tracked by `pending_variable_name`.
    /// Closure compilation uses this to read finalized inference facts for
    /// stored function values without relying on name-only lookup.
    pub(crate) pending_variable_span: Option<shape_ast::ast::Span>,

    /// v2 Phase 3.1: when the enclosing `let arr: Array<T> = [...]` declares
    /// an explicit `Array<T>` annotation whose element type maps to a
    /// [`v2_typed_emission::TypedArrayKind`], stash the kind here so
    /// `compile_expr_array` can lower the literal to a v2 typed-array
    /// allocation. The statement-binding code path resets this to `None`
    /// before each new initializer.
    pub(crate) pending_variable_typed_array_kind:
        Option<crate::compiler::v2_typed_emission::TypedArrayKind>,

    /// Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
    /// When `pending_variable_typed_array_kind == Some(TraitObject)`, this
    /// carries the trait name extracted from the `Array<dyn Trait>` annotation
    /// so `compile_expr_array`'s element loop can emit `BoxTraitObject` (with
    /// the trait-name `Operand::Name`) after each concrete struct element is
    /// compiled — converting the `Ptr(HeapKind::TypedObject)` struct value into
    /// the `Ptr(HeapKind::TraitObject)` fat-pointer the
    /// `TypedArrayPushTraitObject` opcode requires. Reset alongside
    /// `pending_variable_typed_array_kind`. Per ADR-006 §2.7.5 the trait name
    /// is the producer-side proof (explicit annotation), never runtime-derived.
    pub(crate) pending_trait_object_array_trait: Option<String>,

    /// R5.4B: nested-array-literal depth.
    ///
    /// Incremented while `compile_expr_array` compiles the elements of an
    /// array literal, decremented after. When `> 0`, the compiler is inside
    /// a nested array-literal context (e.g. the inner `[1.0, 2.0]` of
    /// `[[1.0, 2.0], [3.0, 4.0]]`) and MUST refuse the typed-array fast
    /// path (`NewTypedArrayF64`/etc.) regardless of annotation or
    /// inference.
    ///
    /// Why: the v2 typed-array opcodes store the allocation as a raw
    /// native pointer on the kinded VM stack with
    /// `NativeKind::Ptr(HeapKind::TypedArray)` declared on the parallel-
    /// kind track (ADR-006 §2.7.7), not as a generic heap-tagged value.
    /// Downstream consumers that expect a generic `Array` via
    /// `slot.as_heap_value()` cannot decode a typed-array native pointer
    /// back into a generic Array (the deleted `as_heap_ref()` /
    /// `as_any_array()` carrier accessors are gone), so e.g.
    /// `intrinsic_matmul_mat` fails with "row 0 must be an array of
    /// numeric values". Refusing typed emission for inner rows forces
    /// them onto the legacy `NewArray` path, which produces a generic
    /// `HeapValue::Array` that round-trips correctly through a generic
    /// outer `Array`.
    #[allow(dead_code)]
    pub(crate) nested_array_literal_depth: u32,

    /// Depth counter: > 0 while compiling an interpolated-string inner
    /// expression (`f"...{expr}..."`).
    ///
    /// The inner `expr` is re-parsed at bytecode-compile time via
    /// `parse_expression_str`, which assigns it PARSER-LOCAL spans
    /// (offsets within the `{...}` fragment, e.g. `Span { 0..7 }`) that
    /// bear no relation to the original source offsets. The MIR borrow
    /// analysis keys its per-statement ownership decisions
    /// (`OwnershipDecision::Move`/`Clone`/`Copy`) by ORIGINAL-SOURCE span;
    /// a fragment-local span can COLLIDE with an unrelated real statement's
    /// span and make `query_ownership_decision` return that statement's
    /// `Move` for the f-string's identifier read. Emitting `LoadLocalMove`
    /// for such a read moves the value OUT of the slot — fatal when the
    /// identifier is a live loop counter (the slot zeroes and the loop
    /// never advances → non-termination). While this counter is > 0,
    /// `emit_load_local_owned` skips the span-keyed ownership query and
    /// emits the safe non-consuming load (plain / typed `LoadLocal`).
    /// An f-string read is value-producing for the format call and never
    /// the semantic last-use of the binding, so suppressing Move here is
    /// always correct.
    pub(crate) in_interpolation_expr_depth: u32,

    /// v2 Phase 3.1: per-local-slot record of which locals hold a v2
    /// typed array (allocated via `NewTypedArrayF64/I64/I32/Bool` rather
    /// than the legacy v1 `NewTypedArray`/`NewArray`). Populated by the
    /// statement-binding code path when an `Array<T>` annotation
    /// successfully picks a typed kind. Consumed by
    /// `resolve_receiver_typed_array_kind` so the typed Get/Set/Push/Len
    /// opcodes are only emitted for receivers that were ALSO allocated
    /// as v2 typed arrays — never for legacy NaN-boxed arrays.
    pub(crate) v2_typed_array_locals:
        HashMap<u16, crate::compiler::v2_typed_emission::TypedArrayKind>,
    /// v2 Phase 3.1: per-module-binding record of v2 typed arrays.
    /// Mirrors [`v2_typed_array_locals`] for top-level bindings.
    pub(crate) v2_typed_array_module_bindings:
        HashMap<u16, crate::compiler::v2_typed_emission::TypedArrayKind>,

    /// Phase 4b Round 6 WS-1 W16.2-C (2026-05-21) — list-comprehension
    /// element-kind capture.
    ///
    /// `compile_list_comprehension` emits the result accumulator's
    /// `NewTypedArray*` allocator BEFORE the comprehension body is compiled,
    /// but the element-expression's proven scalar kind is only known AFTER
    /// the body compiles. `compile_comprehension_clauses` writes the proven
    /// [`TypedArrayKind`] here at the innermost (clause-empty) base case,
    /// reading the bytecode compiler's `last_expr_numeric_type` /
    /// `last_expr_type_info` right after the element expression compiles —
    /// per ADR-006 §2.7.5 the kind is proven at the producer site, never
    /// fabricated. `compile_list_comprehension` then patches the recorded
    /// allocator instruction with the matching typed opcode. `None` means
    /// the element kind was not statically provable — a clean compile error.
    pub(crate) comprehension_element_kind:
        Option<crate::compiler::v2_typed_emission::TypedArrayKind>,

    /// Phase 4b Round 6 WS-1 W16.2-C (2026-05-21) — instruction indices of
    /// the placeholder `ArrayPush` opcodes emitted at list-comprehension
    /// element-push sites. `compile_comprehension_clauses` records each
    /// base-case push here; `compile_list_comprehension` patches them all
    /// to the resolved `TypedArrayPush*` opcode once the element kind is
    /// known — so the typed accumulator receives a typed push that the JIT
    /// and VM both dispatch unambiguously (no generic-carrier path).
    pub(crate) comprehension_push_sites: Vec<usize>,

    /// Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21) — bare
    /// empty-array accumulator deferred-kind capture.
    ///
    /// A bare `let mut out = []` (empty array literal, no `Array<T>`
    /// annotation, no elements) cannot resolve its `TypedArrayKind` at the
    /// literal site — the element type is determined only by downstream
    /// `out.push(x)` calls. `compile_expr_array` emits a placeholder
    /// `NewArray(0)` and records its instruction index here against the
    /// binding (local slot or module binding). The FIRST `arr.push(v)` on
    /// that binding resolves the element kind from the compiled argument's
    /// proven type (ADR-006 §2.7.5 producer-side stamp — read from the
    /// type-tracker at the push site, never decoded from runtime bits, never
    /// Bool-defaulted), patches the placeholder to the matching typed
    /// `NewTypedArray*` allocator, and promotes the binding into
    /// `v2_typed_array_locals` / `v2_typed_array_module_bindings` so every
    /// subsequent push is a typed `TypedArrayPush*`. A bare empty array that
    /// is never pushed to and never annotated has a genuinely un-resolvable
    /// element type — `finalize_unresolved_empty_array_accumulators`
    /// surface-and-stops with a clean structured compile error.
    pub(crate) empty_array_accumulators: HashMap<EmptyArrayAccumulatorKey, EmptyArrayAccumulator>,

    /// Phase 4b Round 6 WS-1b — instruction index of the placeholder
    /// `NewArray(0)` emitted by the most recent `compile_expr_array` call for
    /// a bare empty array literal. The enclosing `Statement::VariableDecl` /
    /// `Item::VariableDecl` reads this immediately after the initializer
    /// compiles and re-keys it into [`empty_array_accumulators`] against the
    /// resolved binding. `None` when the last array literal was non-empty,
    /// annotated, or otherwise resolved a kind directly.
    pub(crate) pending_empty_array_alloc_idx: Option<usize>,

    /// Empty-in-context element-type inference (issue #14, user-ratified
    /// 2026-07-07 CANONICAL-INSTANTIATE). When `true`, a context-free empty
    /// array literal `[]` — one that resolves NO element-type kind from any
    /// annotation / sibling / push — that is compiled at an UNCONSTRAINED
    /// monomorphic sink (a polymorphic `_`/`PolymorphicArg` param or the
    /// object-graph marshal boundary) lowers to the canonical monomorphic
    /// `TypedArray<int>` empty array (`NewTypedArrayI64(0)`) instead of the
    /// untyped `NewArray(0)` placeholder that SURFACEs at runtime. Sound
    /// because such an empty, never-pushed array's element type is provably
    /// UNOBSERVED at the sink (HM instantiation of `∀T. Array<T>` at a
    /// canonical unit `T`). This is NOT an untyped/any/Bool-default carrier —
    /// it is a concrete monomorphic `TypedArray<int>`. The flag is scoped
    /// (save/restore) to exactly the marshal/polymorphic-sink argument
    /// subtree; a context-bound empty array (binding/param/struct-field/return
    /// annotation) still resolves + gets its real element type, and a
    /// context-free empty array OUTSIDE such a sink (`let xs = []`) still
    /// surface-and-stops with the clean un-resolvable-element compile error.
    pub(crate) pending_empty_array_canonical_instantiate: bool,

    /// strict-flip S1 (array-destructure element-kind, 2026-06-22): the proven
    /// element type NAME (`"int"` / `"number"` / `"string"` / …) of the array
    /// being destructured by the enclosing `let [a, b] = <Array<T>>`. Set from
    /// `concrete_type_for_expr(init).Array(elem)` at the VariableDecl
    /// ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback, 2026-05-12):
    /// per-local-slot record of locals known to hold a Copy-on-Write
    /// collection (HashSet / HashMap / Deque / PriorityQueue / Array of
    /// generic carrier). Populated at let-binding time when the initializer
    /// is one of `Set()` / `HashMap()` / `Deque()` / `PriorityQueue()` /
    /// `[…]`, or when the binding has an explicit type annotation that
    /// names the container kind.
    ///
    /// Consumed by `compile_expr_method_call`'s `&mut self` writeback
    /// emission gate: for an identifier-receiver method call where
    /// `(container_kind, method_name)` matches a `MUT_SELF_*` entry, the
    /// compiler emits `Dup; StoreLocal recv` after `CallMethod` so the
    /// new (possibly Arc-cloned) receiver Arc updates the binding slot.
    pub(crate) mut_self_container_locals:
        HashMap<u16, crate::compiler::mutation_writeback::ContainerKind>,

    /// ADR-006 §2.7.27 / Item 4 ruling: per-module-binding record of
    /// mutating-container module bindings. Mirrors
    /// [`mut_self_container_locals`] for top-level bindings.
    pub(crate) mut_self_container_bindings:
        HashMap<u16, crate::compiler::mutation_writeback::ContainerKind>,

    /// ADR-006 §2.7.27 / Item 4 ruling: signal raised by container-ctor
    /// builtin call emitters (`SetCtor`, `DequeCtor`,
    /// `PriorityQueueCtor`, `HashMapCtor`) so the surrounding statement-
    /// binding code path can transfer the kind onto the target local
    /// / module binding. Mirrors the existing
    /// [`pending_variable_typed_array_kind`] convention.
    pub(crate) pending_variable_container_kind:
        Option<crate::compiler::mutation_writeback::ContainerKind>,

    /// Lexical names that will later need their binding value to remain a raw reference.
    /// This is only used to choose `Value` vs `PreserveRef` lowering for bindings; MIR
    /// remains the sole authority for borrow legality.
    pub(crate) future_reference_use_name_scopes: Vec<HashSet<String>>,

    /// Known trait names (populated in the first pass so meta definitions can reference traits)
    pub(crate) known_traits: std::collections::HashSet<String>,

    /// Full trait definitions keyed by trait name.
    /// Used to install default method implementations for impl blocks that omit them.
    pub(crate) trait_defs: HashMap<String, shape_ast::ast::types::TraitDef>,

    /// J-CT.2 (2026-05-23) — comptime impl blocks deferred for in-mini-VM
    /// registration. The outer compiler does NOT desugar/register/compile
    /// methods of `comptime impl Trait for Type { ... }` blocks into the
    /// runtime program; they are skipped from runtime processing and stored
    /// here so the comptime-evaluator mini-VM (`execute_comptime` in
    /// `compiler/comptime.rs`) can prepend them as `Item::Impl` items, where
    /// the in-comptime-mode compiler then desugars + compiles them normally.
    /// Method dispatch from `instance.method()` inside a `comptime { }` block
    /// then routes through the standard UFCS / `Type::method` resolution
    /// path without a new dispatch shape (audit §2.D carve-out).
    pub(crate) comptime_impl_blocks: Vec<shape_ast::ast::types::ImplBlock>,

    /// J-CT.2 (2026-05-23) — original struct definitions captured during
    /// the first pass. The outer compiler only retains field NAMES in
    /// `struct_types`; the comptime-evaluator mini-VM needs full AST
    /// (typed annotations, generic info, annotations) to compile
    /// struct-literal constructions and field accesses inside `comptime { }`
    /// blocks that interact with `comptime_impl_blocks`. Populated in the
    /// first-pass `Item::StructType` arm.
    pub(crate) comptime_context_struct_defs: HashMap<String, shape_ast::ast::types::StructTypeDef>,

    /// Extension registry for comptime execution
    pub(crate) extension_registry: Option<Arc<Vec<shape_runtime::module_exports::ModuleExports>>>,

    /// Comptime field values per type: type_name -> (field_name -> bake-time
    /// constant). These are type-level constants baked at compile time
    /// with zero runtime cost. Inner map releases heap-backed comptime
    /// values (strings via `Arc<String>`, etc.) when a type's entry is
    /// removed or the compiler is dropped.
    ///
    /// Kinded carrier per ADR-006 §2.7 / Q7 (`KindedSlot`); the prior
    /// `shape_value::ValueMap` shape was deleted by the strict-typing
    /// bulldozer along with `ValueWord`. Mirrors the
    /// `comptime_builtins::ComptimeDirective::SetParamValue { value:
    /// KindedSlot }` migration already landed in
    /// `compiler/comptime_builtins.rs`.
    pub(crate) comptime_fields: HashMap<String, HashMap<String, shape_value::KindedSlot>>,
    /// Type diagnostic mode for shared analyzer diagnostics.
    pub(crate) type_diagnostic_mode: TypeDiagnosticMode,
    /// Expression compilation diagnostic mode.
    pub(crate) compile_diagnostic_mode: CompileDiagnosticMode,
    /// Whether this compiler instance is compiling code for comptime execution.
    /// Enables comptime-only builtins and comptime-specific statement semantics.
    pub(crate) comptime_mode: bool,
    /// Functions removed by comptime annotation handlers (`remove target`).
    /// These are still present in `program.functions` (registered in the first pass)
    /// but must produce a clear compile-time error when called instead of jumping
    /// to an invalid entry point.
    pub(crate) removed_functions: HashSet<String>,
    /// Snapshot of the whole-program AST exactly as it was handed to the
    /// pre-body `analyze_program` pass (compiler_impl_reference_model.rs). It
    /// is the source-of-truth program for directive re-analysis: when a
    /// comptime directive (`set return` / `set param`) mutates a function's
    /// signature during body compilation, the mutated signature is patched
    /// into a clone of this program and re-checked through the SAME
    /// `analyze_program` path the explicit-annotation path uses, so a
    /// directive-set signature that disagrees with the body becomes an
    /// ordinary compile error instead of a segfault (design §4.5, S3).
    pub(crate) directive_reanalysis_program: Option<shape_ast::ast::Program>,
    /// The `known_bindings` slice handed to the pre-body `analyze_program`
    /// pass, captured so directive re-analysis reproduces the same binding
    /// environment (module bindings, imported consts, etc.).
    pub(crate) directive_reanalysis_known_bindings: Vec<String>,
    /// Accumulated per-function signature overrides applied by comptime
    /// directives, keyed by function name. Each re-analysis patches ALL known
    /// overrides into the snapshot so cross-function references observe the
    /// post-directive signatures (declaration-order semantics, design §4.5.1).
    pub(crate) directive_signature_overrides: HashMap<
        String,
        (
            Vec<shape_ast::ast::FunctionParameter>,
            Option<shape_ast::ast::TypeAnnotation>,
        ),
    >,
    /// Internal guard for compiler-synthesized `__comptime__` helper calls.
    /// User source must never access `__comptime__` directly.
    pub(crate) allow_internal_comptime_namespace: bool,
    /// Method table for data-driven method signature queries.
    /// Used to replace hardcoded heuristics (e.g., is_type_preserving_table_method)
    /// with MethodTable lookups (is_self_returning, takes_closure_with_receiver_param).
    pub(crate) method_table: MethodTable,
    /// Locals that are reference-typed in the current function.
    pub(crate) ref_locals: HashSet<u16>,
    /// Subset of ref_locals that hold exclusive (`&mut`) borrows.
    /// Used to enforce the three concurrency rules at task boundaries.
    pub(crate) exclusive_ref_locals: HashSet<u16>,
    /// Subset of ref_locals that were INFERRED as by-reference (not explicitly declared `&`).
    /// Inferred-ref params are owned values passed by reference for performance;
    /// closures may capture them (the value is dereferenced at capture time).
    pub(crate) inferred_ref_locals: HashSet<u16>,
    /// Locals whose binding value is itself a first-class reference (`let r = &x`).
    /// Reads auto-deref; writes still rebind the local.
    pub(crate) reference_value_locals: HashSet<u16>,
    /// Subset of reference_value_locals that hold exclusive (`&mut`) references.
    pub(crate) exclusive_reference_value_locals: HashSet<u16>,
    /// U4-5b: the single structural referent `ConcreteType` carrier for a
    /// first-class reference binding (`let r = &n` / `let r = &a`). Serves BOTH
    /// `r[i]` (array element via `ConcreteType::Array`, in
    /// `tracked_array_element_type`) AND the value-position scalar auto-deref
    /// `r + 1` / `-r` (scalar projected by `reference_referent_scalar_type_name`
    /// in `infer_expr_type`), mirroring the `r.len()` method-dispatch auto-deref.
    /// Keyed by local slot. Collapses the deleted parallel referent
    /// display-string carrier — the referent's ConcreteType IS the proof.
    pub(crate) reference_value_local_referent_concrete_type:
        HashMap<u16, shape_value::v2::ConcreteType>,
    /// As `reference_value_local_referent_concrete_type`, keyed by
    /// module-binding index.
    pub(crate) reference_value_module_binding_referent_concrete_type:
        HashMap<u16, shape_value::v2::ConcreteType>,
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
    /// Module bindings whose value is itself a first-class reference.
    pub(crate) reference_value_module_bindings: HashSet<u16>,
    /// Subset of reference_value_module_bindings that hold exclusive (`&mut`) references.
    pub(crate) exclusive_reference_value_module_bindings: HashSet<u16>,
    /// ModuleBinding-ref writebacks collected while compiling current call args.
    pub(crate) call_arg_module_binding_ref_writebacks: Vec<Vec<(u16, u16)>>,
    /// Inferred reference parameters for untyped params: function -> per-param flag.
    pub(crate) inferred_ref_params: HashMap<String, Vec<bool>>,
    /// Inferred mutating-reference params: function -> per-param flag.
    pub(crate) inferred_ref_mutates: HashMap<String, Vec<bool>>,
    /// Effective per-parameter pass mode (explicit + inferred), by function name.
    pub(crate) inferred_param_pass_modes: HashMap<String, Vec<ParamPassMode>>,
    /// Stack of scopes, each containing locals that need Drop calls at scope exit.
    /// Each entry is (local_index, is_async).
    pub(crate) drop_locals: Vec<Vec<(u16, bool)>>,
    /// Phase V1.1C: parallel scope stack tracking locals whose storage class
    /// is `UniqueHeap` and therefore (when the ownership-moves flag is on)
    /// need an ownership-aware `DropLocal` opcode emitted at scope exit to
    /// release the owned heap allocation. Orthogonal to `drop_locals` (which
    /// drives user-facing `Drop` trait calls via `DropCall`). Populated and
    /// consumed only when `ownership_moves_enabled()` is true; otherwise the
    /// scope stack is pushed/popped in lockstep with `drop_locals` but kept
    /// empty, so the compiler's emission path is byte-identical to
    /// pre-V1.1C.
    pub(crate) ownership_drop_locals: Vec<Vec<u16>>,
    /// Per-type drop kind: tracks whether each type has sync, async, or both drop impls.
    /// Populated during the first-pass registration of impl blocks.
    pub(crate) drop_type_info: HashMap<String, DropKind>,
    /// ADR-009 C2 #13 (slice 2, D6): set true during `compile_function` whenever
    /// the RAII drop-plan resolves a drop-obligated LOCAL (`local_drop_kind` /
    /// annotation / initializer-call-return — the emission authority, so an
    /// INFERRED drop type is caught, not under-detected). `compile_function`
    /// save/restores it around the body compile (re-entrancy under
    /// monomorphization) and reads THIS function's value for the async-drop-context
    /// gate. See `checked_body::async_drop_context`.
    pub(crate) current_function_saw_drop_obligated_local: bool,
    // ADR-009 C2 #13 (slice 4): the former `pending_generated_body_origin`
    // shared field was DELETED. Generated-body provenance for the D6
    // async-drop-context gate is now threaded as a PARAMETER through
    // `compile_function_with_generated_origin` → `compile_function_inner`, so a
    // nested monomorphization compile can never steal it and the gate evaluates
    // over the EFFECTIVE definition (covering `replace body` edits, whose swap
    // never reaches the outer `func_def`).
    /// Module bindings that need Drop calls at program exit.
    /// Each entry is (binding_index, is_async).
    pub(crate) drop_module_bindings: Vec<(u16, bool)>,
    /// Mutable closure captures in the current function being compiled.
    /// Maps captured variable name -> upvalue index (for LoadClosure/StoreClosure).
    /// Only populated while compiling a closure body that has mutable captures.
    pub(crate) mutable_closure_captures: HashMap<String, u16>,

    /// Track A.1C.2: subset of `mutable_closure_captures` whose source
    /// binding classifies as `CaptureAccess::SharedCell` (a `var` binding
    /// mutably captured through a closure). Maps captured variable name
    /// → capture index. When the identifier lookup finds a name in this
    /// map, the closure body emits `LoadSharedCapture` /
    /// `StoreSharedCapture` (A.1B) instead of the legacy `LoadClosure`
    /// / `StoreClosure`. Populated while compiling a closure body.
    pub(crate) shared_closure_captures: HashMap<String, u16>,

    /// Track A.1C.2b: subset of `mutable_closure_captures` whose source
    /// binding classifies as `CaptureAccess::OwnedMutableCell` (a `let mut`
    /// binding captured by move into a single closure). Maps captured
    /// variable name → capture index. When the identifier lookup finds
    /// a name in this map, the closure body emits
    /// `LoadOwnedMutableCapture` / `StoreOwnedMutableCapture` (A.1B)
    /// instead of the legacy `LoadClosure` / `StoreClosure`. Populated
    /// while compiling a closure body.
    pub(crate) owned_mutable_closure_captures: HashMap<String, u16>,

    /// Wave E: parallel to `owned_mutable_closure_captures`, records
    /// the interior `FieldKind` of each OwnedMutable capture cell. The
    /// inner kind is derived from the captured binding's resolved
    /// `ConcreteType` (`concrete_type_for_expr → ConcreteType::to_field_kind`)
    /// at the point of closure construction. The closure body uses
    /// this map to dispatch to the typed Wave D.1 opcodes
    /// (`LoadOwnedMutableCapture<Kind>` / `StoreOwnedMutableCapture<Kind>`,
    /// codes 0x140-0x155) instead of the legacy untyped `0x132`/`0x133`
    /// opcodes (kind-erased pre-strict-typing — ADR-006 §2.7.7).
    /// Populated alongside `owned_mutable_closure_captures`,
    /// saved/restored across nested closure-body compilations.
    pub(crate) owned_mutable_capture_inner_kinds:
        HashMap<String, shape_value::v2::struct_layout::FieldKind>,

    /// A2-refined / task #17: parallel to `shared_closure_captures`,
    /// records the interior `FieldKind` of each Shared (`var`) capture
    /// cell. Derived from the captured binding's resolved `ConcreteType`
    /// at closure-construction time, mirroring
    /// `owned_mutable_capture_inner_kinds`. The closure body uses this
    /// map to dispatch to the typed Wave D.2 opcodes
    /// (`LoadSharedCapture<Kind>` / `StoreSharedCapture<Kind>`, codes
    /// 0x156-0x16B) instead of the legacy untyped `0x134`/`0x135`
    /// opcodes (kind-erased pre-strict-typing — ADR-006 §2.7.7).
    /// Saved/restored across nested closure-body compilations.
    pub(crate) shared_capture_inner_kinds:
        HashMap<String, shape_value::v2::struct_layout::FieldKind>,

    /// ADR-009 C1 slice 4: one-shot structural capture evidence for the next
    /// recursively compiled closure body. The vector is in capture-parameter
    /// order and comes directly from that closure's [`CapturePack`].
    /// `compile_function` consumes it before compiling the body; it is never
    /// reconstructed from a parameter name or runtime bits.
    ///
    /// [`CapturePack`]: crate::compiler::comptime_builtins::capture_plan::CapturePack
    pub(crate) pending_closure_capture_parameter_evidence:
        Option<Vec<crate::compiler::comptime_builtins::capture_plan::CaptureParameterEvidence>>,

    /// Structural evidence for every synthetic capture parameter in the
    /// current closure body, keyed by compiler-issued local slot. Every entry
    /// preserves the original binding lineage and exact semantic type through
    /// nested forwarding; `CaptureAccess::SharedCell` additionally proves that
    /// the slot carries the canonical raw `*const SharedCell` carrier.
    pub(crate) inherited_capture_parameter_evidence:
        HashMap<u16, crate::compiler::comptime_builtins::capture_plan::CaptureParameterEvidence>,

    /// Variables in the current scope that have been boxed into SharedCells
    /// by a mutable closure capture. When a subsequent closure captures one
    /// of these variables (even immutably), it must use the SharedCell path
    /// so it shares the same mutable cell.
    pub(crate) boxed_locals: HashSet<String>,

    /// Track A.1C.2: local slots that have been promoted to
    /// `Arc<parking_lot::Mutex<u64>>` via the `AllocSharedLocal`
    /// opcode (with the matching `NativeKind` declared on the cell's
    /// parallel-kind track per ADR-006 §2.7.8). After promotion, every
    /// outer-scope read/write of the slot
    /// must go through `LoadSharedLocal` / `StoreSharedLocal` (never plain
    /// `LoadLocal` / `StoreLocal`), and scope exit must emit
    /// `DropSharedLocal` so the Arc strong count is released exactly once
    /// per owning frame.
    ///
    /// Keyed by the binding *name* (mirroring `boxed_locals`). Populated
    /// by `compile_expr_closure` when a `var` capture escapes into a
    /// closure and gets classified as `CaptureAccess::SharedCell`.
    pub(crate) shared_locals: HashSet<String>,

    /// Track A.1C.3: local names that have been classified as
    /// `CaptureAccess::OwnedMutableCell` by at least one closure in the
    /// current function scope. Needed because `binding_semantics_for_name`
    /// can return `None` after an inner closure's `compile_function`
    /// wipes the type-tracker local semantics, and we need the
    /// classification to be stable across sibling closures (otherwise
    /// a second closure capturing a different `let mut` local could
    /// fall back to `Immutable`, nulling the layout's OwnedMutable mask
    /// bit and triggering a layout mismatch in `op_make_closure`).
    pub(crate) owned_mutable_locals: HashSet<String>,

    /// Session 1 (Rust-move semantics for `let mut`): names of `let mut`
    /// local bindings that have been **moved by value into a closure
    /// capture** (i.e. classified as `CaptureAccess::OwnedMutableCell` and
    /// emitted at `op_make_closure` time as `Box::into_raw(Box::new(bits))`).
    /// After the move, the outer slot holds only a stale snapshot of the
    /// initial value — every subsequent outer-scope read or write of the
    /// same binding is a compile error ("use-after-move").
    ///
    /// Stored as `name → span-of-the-capturing-closure` so the diagnostic
    /// can point the user at the exact capture site that consumed the
    /// binding. Populated in `compile_expr_closure`, consulted in
    /// `compile_expr_identifier` and `compile_expr_assign`. Saved /
    /// restored across nested `compile_function` calls, mirroring the
    /// `shared_locals` / `owned_mutable_locals` discipline.
    pub(crate) captured_let_mut_moved: HashMap<String, Span>,

    /// Track A.1C.3: module-binding slots that have been promoted to
    /// `Arc<parking_lot::Mutex<u64>>` via `AllocSharedModuleBinding`
    /// (with the matching `NativeKind` declared on the cell's
    /// parallel-kind track per ADR-006 §2.7.8).
    /// After promotion, every outer-scope read/write of the binding must
    /// go through `LoadSharedModuleBinding` / `StoreSharedModuleBinding`
    /// (never plain `LoadModuleBinding` / `StoreModuleBinding`). Keyed
    /// by the binding's **scoped name** — the same key used by
    /// `resolve_scoped_module_binding_name` and the `module_bindings`
    /// index map. Populated by `compile_expr_closure` when a
    /// module-scope `var` is captured mutably.
    pub(crate) shared_module_bindings: HashSet<String>,

    /// Track A.1C.2: per-scope stack of slot indices that need a
    /// `DropSharedLocal` opcode at scope exit, mirroring the
    /// `ownership_drop_locals` discipline. Push in lockstep with
    /// `push_drop_scope`; pop and emit in lockstep with `pop_drop_scope`.
    pub(crate) shared_drop_locals: Vec<Vec<u16>>,

    /// Active permission set for capability checking.
    ///
    /// When set, imported stdlib functions are checked against capability_tags.
    /// If a function requires a permission not in this set, a compile error is
    /// emitted and the function never enters bytecode.
    ///
    /// `None` means no checking (backwards-compatible default).
    pub(crate) permission_set: Option<shape_abi_v1::PermissionSet>,
    /// Typed ownership for graph-module import authorization and carriers.
    graph_permission_state: import_permissions::GraphPermissionState,

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

    /// A-final ROOT-C: set while compiling the body of an *uninstantiated
    /// implicit-generic* function (an unannotated, never-called `fn add(a, b)
    /// { a + b }` whose value params stay unresolved type variables). Such a
    /// body is a deferred template — its bytecode is DEAD (re-emitted with
    /// proven kinds per concrete call site), so the polymorphic-numeric
    /// proof-gap (and the sibling "cannot infer types for binary operation"
    /// strict-typing error) on its unproven operands must NOT abort
    /// compilation. When set, the numeric binop emitter defers that specific
    /// typed-OPCODE emission (emits a stack-balancing `Pop` placeholder into
    /// the dead blob — never a fabricated typed numeric opcode), while every
    /// STRUCTURAL/schema body check (e.g. object-spread-without-known-schema)
    /// still surfaces its `Err`. This narrows the prior whole-body skip so a
    /// genuine structural error is no longer suppressed alongside the benign
    /// numeric proof-gap.
    pub(crate) deferring_uninstantiated_template_body: bool,

    /// wave7 finance-field-arith-gap (repair): nesting depth of
    /// call-/method-argument compilation. Non-zero while an argument
    /// expression is being lowered for a function or method call.
    ///
    /// The implicit-generic function-as-value guard in
    /// `compile_expr_identifier` (which refuses capturing a
    /// `fn f(row) { row.high - row.low }`-shaped template as a first-class
    /// value, since the un-monomorphized template blob would silently drop an
    /// operand / dynamic-dispatch) must NOT fire when the function identifier
    /// is passed DIRECTLY as a call/HOF argument (`[1,2,3].map(double)`). A
    /// bare `let f = double` capture (depth 0) still errors — that is the
    /// genuinely-unsound reachability the guard keeps closed — but a direct
    /// HOF-consumer argument (depth > 0) is exempt so it compiles and runs.
    pub(crate) call_argument_depth: u32,

    /// Package-scoped native library resolutions for the current host.
    pub(crate) native_resolution_context:
        Option<shape_runtime::native_resolution::NativeResolutionSet>,

    /// Active synthetic MIR context while compiling non-function code.
    pub(crate) non_function_mir_context_stack: Vec<String>,

    /// MIR lowered for compiled functions and synthetic non-function contexts.
    pub(crate) mir_functions: HashMap<String, crate::mir::types::MirFunction>,

    /// Borrow analyses produced from lowered MIR for compiled functions and
    /// synthetic non-function contexts.
    pub(crate) mir_borrow_analyses: HashMap<String, crate::mir::BorrowAnalysis>,

    /// Storage plans produced by the storage planning pass for each function.
    /// Maps function name to the plan mapping each MIR slot to a `BindingStorageClass`.
    pub(crate) mir_storage_plans: HashMap<String, crate::mir::StoragePlan>,

    /// Per-function borrow summaries for interprocedural alias checking.
    /// Describes which parameters conflict and must not alias at call sites.
    pub(crate) function_borrow_summaries: HashMap<String, crate::mir::FunctionBorrowSummary>,

    /// Per-function mapping from AST spans to MIR program points.
    /// Used to bridge the bytecode compiler (which knows AST spans) to
    /// MIR ownership decisions (which are keyed by `Point`).
    pub(crate) mir_span_to_point:
        HashMap<String, HashMap<shape_ast::ast::Span, crate::mir::types::Point>>,

    /// Field-level definite-initialization and liveness analyses for compiled functions.
    pub(crate) mir_field_analyses: HashMap<String, crate::mir::FieldAnalysis>,

    /// Graph-compiled namespace map: local namespace name -> canonical module path.
    /// Populated during graph-driven compilation to resolve qualified names.
    pub(crate) graph_namespace_map: HashMap<String, String>,

    /// Module dependency graph (set during graph-driven compilation).
    pub(crate) module_graph: Option<std::sync::Arc<crate::module_graph::ModuleGraph>>,

    /// Explicit concrete facts for local bindings in the current function.
    /// This is the fact carrier consumed by monomorphization identifier lookup.
    pub(crate) current_function_local_concrete_facts: HashMap<u16, BindingConcreteFact>,

    /// Per-local-slot AST binder span for the function currently being compiled.
    /// This bridges VM slot identity to runtime `InferenceFacts` without
    /// re-parsing initializer expressions or keeping collection-specific
    /// carrier tables.
    pub(crate) local_binding_spans: HashMap<u16, shape_ast::ast::Span>,

    /// Capture names invoked as a callee (`g(...)`) inside the closure
    /// literal currently being compiled. Populated by `compile_expr_closure`
    /// / `mint_closure_type_id_peek` immediately before capture-type
    /// resolution and consulted by `resolve_capture_concrete_type` so an
    /// unannotated callable capture stamps `ConcreteType::Function`
    /// (→ `Ptr(HeapKind::Closure)`) instead of falling through to the
    /// `Pointer(Void)` → `NativeView` "unknown" sentinel. Without this an
    /// `fn f(g) { |x| g(x) }`-style returned closure carries a
    /// wrong-carrier `NativeView` label and is rejected by the
    /// `call_value_immediate_nb` callee match. Cleared after each closure.
    pub(crate) current_closure_callee_captures: std::collections::BTreeSet<String>,

    /// Explicit concrete facts for module bindings, consumed by
    /// monomorphization identifier lookup.
    pub(crate) module_binding_concrete_facts: HashMap<u16, BindingConcreteFact>,

    /// Per-module-binding AST binder span. Closure capture kind resolution
    /// consults `InferenceFacts` through this span when the regular concrete
    /// slot tables do not carry a fact.
    pub(crate) module_binding_spans: HashMap<u16, shape_ast::ast::Span>,

    /// Monomorphization cache for generic function specialization.
    pub(crate) monomorphization_cache: monomorphization::cache::MonomorphizationCache,

    /// BUG3 — cycle detector for generic method / free-function monomorphization.
    ///
    /// Holds typed exact-or-legacy keys whose specialization is currently
    /// being compiled. The domain tag prevents an ABI-only attempt from
    /// blocking or borrowing a semantically exact attempt with the same
    /// physical ABI key.
    ///
    /// Note: direct self-recursion in the body is already handled by the
    /// cache-insert-before-compile behaviour; this guard only fires on the
    /// pathological transitive-resolution cycle.
    pub(crate) monomorphization_in_progress: std::collections::HashSet<
        monomorphization::semantic_specialization::SpecializationProgressKey,
    >,

    /// Drop-restored semantic overlays for nested specialized-body compiles.
    /// Declaration-only frames preserve existing generic reflection while
    /// refusing instantiated capture evidence; exact frames additionally map
    /// declared TypeVar capabilities to closed semantic candidates.
    pub(crate) specialization_type_overlays:
        monomorphization::semantic_specialization::SpecializationTypeOverlayStack,

    /// Structural generated-function nesting used to address inference-owned
    /// semantic call-site facts. Source offsets alone are not unique across
    /// generated nodes.
    pub(crate) active_generated_node_stack: Vec<shape_runtime::type_system::GeneratedNodeKey>,

    /// ADR-009 C1: compilation-instance capability for generated AST nodes.
    /// A provenance carrier is trusted only when this exact issuer recognizes
    /// its non-serialized token. Foreign construction and serde round-trips
    /// therefore cannot turn ordinary source into generated code.
    pub(crate) generated_node_issuer: shape_ast::ast::GeneratedNodeIssuer,

    /// ADR-009 D1 (Decision 68) — compiler-owned registry of
    /// generated-symbol identities: the single source of truth for which
    /// declarations were generated, by which expansion, from which source
    /// anchor. Issued identities are content-derived
    /// (`expansion_provenance::SymbolId`), never counter-allocated. Both
    /// phases of the existing extend/materialization path reserve into this
    /// table (comptime-excellence §4.5.1 pre-pass +
    /// `apply_comptime_extend`/`apply_comptime_extend_items` pass-2); the
    /// former name-keyed `materialized_comptime_fns` set is deleted — name
    /// membership is the table's derived `contains_name` view.
    pub(crate) generated_symbols: comptime_builtins::expansion_provenance::GeneratedSymbolTable,

    /// ADR-009 C2 #13 (slice 1) — when set, a rolled-back generated-body
    /// install (see [`checked_body`]) retains the generated-query reservation
    /// tables (`generated_symbols`, `closure_capture_packs`) after a recoverable
    /// compile `Err`, so the LSP generated-symbol/capture query entries can keep
    /// answering from them. Off for ordinary (batch/install) compilation, which
    /// rolls back every publication. A named query-session mode, not a
    /// soft-fail: executable publications still roll back in both modes.
    pub(crate) retain_generated_reservations_for_query_session: bool,

    /// ADR-009 C2 #13 (slice 1) — the install transaction's displaced-entry undo
    /// journal (see [`checked_body::journal`]). `Some` only while a generated-body
    /// install transaction is live (opened in `begin_checked_body_install`,
    /// cleared on commit or consumed on rollback); every keyed install write
    /// records its displaced prior here so a rollback restores it rather than
    /// deleting a shared prelude/dependency key.
    pub(in crate::compiler) install_journal: Option<checked_body::InstallJournal>,

    /// ADR-009 C3 #14 (slice 2, S2b) — the hook-template INSTALL registry:
    /// one row per applied `install(...)` directive (annotation name, target
    /// name, hook kind, template-Sig rendering, specialized symbol/index,
    /// capture renderings, `@application` span). Compiler-owned query state
    /// (the C1 slice-4 `generated_symbol_query` precedent — the S8 hover
    /// surface reads THIS, never a text scan). Rows are written at the pass-2
    /// apply seam and journaled through the open [`checked_body`] install
    /// transaction (`journal_record_hook_install_row`), so a rolled-back
    /// compile leaves no row.
    pub(in crate::compiler) hook_install_registry:
        Vec<template_specialization::install_registry::HookInstallRecord>,

    /// ADR-009 E3 (slice S1) — the generated analysis items materialized by
    /// the executed declaration-discovery pre-pass
    /// (`materialize_computed_comptime_extends`) for this compilation unit.
    /// This is the SINGLE authority for generated `extend`/free-function
    /// items: the former non-evaluating static AST scan (deleted
    /// `shape_ast::transform::comptime_extends`) is gone. Populated once by
    /// the reference-model driver immediately after the executed pre-pass and
    /// read back by `generated_analysis_items()` so static consumers (LSP
    /// inference helpers, the `expand-comptime` CLI report) augment from the
    /// executed authority instead of a parallel scan.
    pub(crate) generated_analysis_items: Vec<shape_ast::ast::Item>,

    /// ADR-009 E2 #18 (slice 3) — const-free function-target `replace body` edits
    /// materialized by the executed declaration-discovery pre-pass
    /// (`materialize_computed_comptime_extends`), for the reference-model driver
    /// to apply to the analysis-program clone BEFORE `analyze_program_full` (swap
    /// the target's body + prepend the hygienic `ctx.original` shadow). This
    /// makes the analyzer see the replacement's closures and publish their
    /// structural facts, flipping the C0911 quarantine. Populated per pre-pass
    /// run (cleared at its start) and drained by the driver; pass-2 still
    /// performs the authoritative install byte-unchanged. Empty on the LSP
    /// generation-reachability / row-3 pre-pass entry points, which never edit an
    /// analysis program.
    pub(in crate::compiler) pending_replace_body_analysis:
        Vec<comptime_fragments::CheckedReplaceBody>,

    /// ADR-009 A3 (review round 1) — names of call-site specializations
    /// (`__w24_method_*`, `__w27_implicit_*`) whose body compile FAILED after
    /// registration. `register_function` runs before `compile_function` (the
    /// body may reference itself), and a failed compile cannot roll the
    /// registration back (later registrations may already have shifted
    /// indices). Without this set, a second call site's `find_function`
    /// reuse short-circuit would return the registered-but-never-compiled
    /// function index — dispatching a ZERO-instruction body (silent wrong
    /// output / linker `remap_fid` self-recursion). The short-circuits
    /// consult this set and re-raise a hard error instead
    /// (surface-and-stop).
    pub(crate) failed_call_site_specializations: std::collections::HashSet<String>,

    /// Monotonic counter for monomorphization specialization IDs.
    pub(crate) next_monomorphization_id: u64,

    /// Phase C — running count of closure-aware specializations emitted in
    /// the current module. When this exceeds
    /// [`monomorphization::cache::DEFAULT_CLOSURE_SPECIALIZATION_BUDGET`],
    /// further closure-aware specializations bail back to the generic
    /// (non-inlined) dispatch path.
    pub(crate) closure_specialization_count: u32,
}

impl Default for BytecodeCompiler {
    fn default() -> Self {
        Self::new()
    }
}

mod compiler_impl_initialization;
mod compiler_impl_reference_model;

/// True when `program`'s top-level (module-scope) code contains a
/// `comptime { ... }` block / `comptime for` in value or statement position.
/// Functions are NOT descended into — a comptime block inside a `fn` body
/// lowers to that function's own MIR, not the `top_level_mir` the JIT
/// top-level strategy consumes.
///
/// The JIT executor calls this on the raw `Program` AST (before any bytecode
/// compilation) so it can deopt a top-level-comptime program straight to the
/// bytecode interpreter. That avoids compiling the program twice — once on the
/// JIT path's `compile_program_for_inspection`, once on the `[jit-fallback]`
/// re-compile — which would re-run a side-effecting comptime body's
/// observable effects (`comptime { print(...) }`) a second time, diverging
/// from `--mode vm` (exactly-once). See ADR-006 §2.7.14.
pub fn program_has_top_level_comptime(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(compiler_impl_reference_model::top_level_item_contains_comptime)
}

/// Infer effective reference parameters and mutation behavior without compiling bytecode.
///
/// Returns `(inferred_ref_params, inferred_ref_mutates)` keyed by function name.
/// - `inferred_ref_params[f][i] == true` means parameter `i` of `f` is inferred/treated as ref.
/// - `inferred_ref_mutates[f][i] == true` means that reference parameter is mutating (`&mut`).
pub fn infer_reference_model(
    program: &Program,
) -> (HashMap<String, Vec<bool>>, HashMap<String, Vec<bool>>) {
    let (inferred_ref_params, inferred_ref_mutates, _, _) =
        BytecodeCompiler::infer_reference_model(program);
    (inferred_ref_params, inferred_ref_mutates)
}

/// ADR-009 E3 (slice S1): materialize the generated analysis items
/// (`Item::Extend` / generated free `Item::Function`) for `program` through
/// the SINGLE executed declaration-discovery authority
/// (`materialize_computed_comptime_extends`, reached via `compile_in_place`).
/// This replaces the deleted non-evaluating static AST scan
/// (`shape_ast::transform::comptime_extends`): static consumers that need
/// generated declarations visible to inference/reporting (the LSP inference
/// helpers, the `expand-comptime` CLI) obtain them here, from the executed
/// result, never from a parallel scan.
///
/// A structural fast path returns no items only when the complete inline item
/// tree proves that no semantic compilation stage can generate, avoiding a
/// compile for the common case. Module annotations and raw module `comptime`
/// blocks conservatively force compilation through their separate pass-2
/// topology-mutating APIs, although their output is not represented by this
/// fixed-point query. The compile runs in RecoverAll modes and tolerates errors
/// — the executed authority still records every declaration reserved before a
/// failure.
pub fn executed_generated_items(program: &Program) -> Vec<Item> {
    if !program_may_generate(program) {
        return Vec::new();
    }
    let mut compiler = BytecodeCompiler::new();
    compiler.set_type_diagnostic_mode(TypeDiagnosticMode::RecoverAll);
    compiler.set_compile_diagnostic_mode(CompileDiagnosticMode::RecoverAll);
    let _ = compiler.compile_in_place(program);
    compiler.generated_analysis_items().to_vec()
}

/// ADR-009 E3 (slice S1): return a clone of `program` with the executed
/// authority's generated items appended — the direct replacement for the
/// deleted `shape_ast::transform::augment_program_with_generated_extends`,
/// but sourced from the executed declaration-discovery pre-pass instead of a
/// parallel non-evaluating scan.
pub fn augment_program_with_executed_extends(program: &Program) -> Program {
    let mut augmented = program.clone();
    augmented.items.extend(executed_generated_items(program));
    augmented
}

/// Infer effective parameter pass modes (`ByValue` / `ByRefShared` / `ByRefExclusive`)
/// keyed by function name.
pub fn infer_param_pass_modes(program: &Program) -> HashMap<String, Vec<ParamPassMode>> {
    let (inferred_ref_params, inferred_ref_mutates, _, _) =
        BytecodeCompiler::infer_reference_model(program);
    BytecodeCompiler::build_param_pass_mode_map(
        program,
        &inferred_ref_params,
        &inferred_ref_mutates,
    )
}

// ADR-006 §2.7.4 / §2.7.7 — Phase 2c deferral.
//
// `compiler_tests.rs` is a deep test harness that uses `eval()`-style
// helpers returning the deleted `shape_value::ValueWord`. Per playbook
// §7 REVISED #4, the correct surface for a non-migratable test site is
// `cfg(any())`-gating rather than reintroducing the §2.7.7 forbidden
// carrier. Re-enabling is Phase 2c work tracked in playbook §10's
// Wave-β B12 deferral pattern.
#[cfg(any())]
#[path = "compiler_tests.rs"]
mod compiler_deep;
pub(crate) mod v2_array_emission;
pub(crate) mod v2_map_emission;
pub(crate) mod v2_typed_emission;

// ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback, 2026-05-12):
// compile-time write-back emission for `&mut self` opt-in methods on
// COW container receivers (HashSet / HashMap / Array / Deque /
// PriorityQueue / TypedArray).
pub(crate) mod mutation_writeback;
