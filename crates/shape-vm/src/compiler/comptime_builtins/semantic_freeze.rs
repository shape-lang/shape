//! ADR-009 §4.1 (ticket A1, slice S1): the canonical per-compilation-unit
//! semantic freeze. // ADR-009
//!
//! `SemanticFreeze` is the single frozen semantic type table built ONCE per
//! compilation unit at the registration-complete barrier
//! (`BytecodeCompiler::install_semantic_freeze`). On the graph-driven
//! pipeline the compilation unit is root + dependency modules and the
//! barrier runs at the graph entry point BEFORE Phase 1 dependency
//! compilation (`compile_with_graph_and_prelude`), so imported-module
//! comptime sites hold the handle too; `compile()` runs the barrier only
//! when it IS the entry point (single-module unit).
//! Slice S2 deleted the per-comptime-site `build_type_reflection_snapshot`
//! rebuilds (the S1 transitional scaffolding): every comptime site now
//! obtains this handle through [`BytecodeCompiler::comptime_freeze_overlay`]
//! — a site that cannot obtain one is a compile error
//! ([`NO_FREEZE_HANDLE_DIAGNOSTIC`], rejection-matrix row 3), never an
//! empty snapshot. Scoped generic parameters enter through a
//! [`FreezeOverlay`], never through a rebuild of the base index.
//!
//! ## Named freeze inputs (spec §4.1 no-two-derivations rule)
//!
//! Every fact enters the freeze from exactly ONE source; no fact is
//! re-derived from a second table:
//!
//! 1. **Struct shapes** — `BytecodeCompiler::struct_types` (field order) plus
//!    `BytecodeCompiler::struct_generic_info.runtime_field_types` (field
//!    annotations). These compiler tables are today the only source of
//!    ordered runtime struct-field annotations; per spec §4.1 ("promotes that
//!    data into the shared surface or documents it as freeze input") they are
//!    documented here as named freeze inputs. The freeze does not re-derive
//!    struct shapes from any other surface.
//! 2. **Type aliases** — `BytecodeCompiler::type_aliases` (named freeze
//!    input, same rationale).
//! 3. **Enums** — `BytecodeCompiler::type_tracker.schema_registry()`, the
//!    canonical schema registry.
//! 4. **Unresolved-inference-variable detection** — the analyzer's canonical
//!    `\u{1}tyvar:` annotation encoding (`annotation_as_tyvar`), i.e. the
//!    exact vocabulary the `TypeInferenceEngine` substitution store uses.
//!    No parallel encoding is introduced.
//!
//! ## Freeze-boundary rejection (Dec 52, rejection-matrix row 4)
//!
//! Freezing partial semantic state is a named-diagnostic error, never a
//! populated freeze: a struct field or alias whose annotation still carries
//! an unresolved inference variable rejects the whole freeze before any
//! comptime body could observe it. `SemanticFreeze` deliberately has NO
//! `Default` impl and no empty constructor — an "empty freeze" cannot exist.
//! The old `TypeReflectionSnapshot::default()` annotation-handler defect was
//! exactly the shape this forbids; S2 deleted it. S3 resolved the pre-pass
//! ordering by moving the barrier ahead of the speculative annotation
//! pre-passes in `compile()`: EVERY annotation-handler execution
//! (speculative or authoritative) consumes the real registration-complete
//! handle. The S2-era pre-pass reflection-rejection module is deleted;
//! exemption-by-suppression is a forbidden shape (S3 pre-pass freeze rule,
//! `functions_annotations.rs::s3_freeze_gate_tests`).
//!
//! ## Identity scheme
//!
//! The identity scheme (SHA-256 canonical descriptors, alias-fixpoint
//! transparency, primitive-synonym coalescing, collision assertion) is reused
//! byte-for-byte from `type_reflection`: the freeze calls the same interning
//! code (`rebuild_frozen_type_index`), it does not reimplement it. The S1
//! byte-identical parity oracle retired with the old builder's deletion;
//! `freeze_pins_identity_scheme_through_the_query_api` below and the 9-test
//! identity matrix in `type_reflection/tests.rs` are the ongoing tripwires.

use super::type_reflection::{FrozenTypeCategory, FrozenTypeIdentity, FrozenTypeIndex};
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::TypeAnnotation;
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::annotation_as_tyvar;
use std::collections::HashMap;
use std::sync::Arc;

/// Rejection-matrix row 3 (ticket A1): a comptime site that cannot obtain
/// the per-compilation-unit freeze handle is a compile error — never an
/// empty snapshot, never a per-site rebuild.
pub(crate) const NO_FREEZE_HANDLE_DIAGNOSTIC: &str = "comptime site has no semantic freeze handle: the per-compilation-unit \
     semantic-freeze barrier (ADR-009 §4.1) must run before any comptime \
     site executes";

/// Named-diagnostic freeze failure (Dec 52 vocabulary). A freeze either
/// completes over fully resolved semantic state or rejects with one of these
/// — there is no partially populated freeze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SemanticFreezeError {
    /// Rejection-matrix row 4: the named subject cannot be frozen because its
    /// type still contains an unresolved inference variable.
    UnresolvedInferenceVariable { subject: String },
}

impl SemanticFreezeError {
    pub(crate) fn diagnostic(&self) -> String {
        match self {
            Self::UnresolvedInferenceVariable { subject } => format!(
                "semantic freeze rejected: {subject} cannot be frozen because \
                 its type contains an unresolved inference variable"
            ),
        }
    }
}

/// The per-compilation-unit frozen semantic type table (ADR-009 §4.1).
///
/// Internal storage is the [`FrozenTypeIndex`] (the reduced remainder of the
/// deleted per-site snapshot carrier) so the interning/identity code stays
/// single-sourced; the index lives only inside this freeze, never as a
/// reachable parallel carrier.
#[derive(Debug)]
pub(crate) struct SemanticFreeze {
    index: FrozenTypeIndex,
}

impl SemanticFreeze {
    /// Build the freeze from the named freeze inputs (module doc). Called
    /// exactly once per compilation unit at the registration-complete
    /// barrier. Errors are freeze-boundary rejections (Dec 52): they fire
    /// before any user comptime body executes.
    pub(crate) fn freeze(
        compiler: &BytecodeCompiler,
    ) -> std::result::Result<Arc<Self>, SemanticFreezeError> {
        // The freeze barrier is the single construction point of the index:
        // `FrozenTypeIndex` deliberately has no `Default`/empty constructor
        // reachable elsewhere.
        let mut index = FrozenTypeIndex {
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            alias_defs: HashMap::new(),
            frozen_type_ids: HashMap::new(),
            frozen_type_categories: HashMap::new(),
        };

        // Named freeze input 1: struct shapes (`struct_types` field order +
        // `struct_generic_info.runtime_field_types` field annotations).
        let mut struct_names: Vec<&String> = compiler.struct_types.keys().collect();
        struct_names.sort();
        for name in struct_names {
            let (field_names, _span) = &compiler.struct_types[name];
            let field_types = compiler
                .struct_generic_info
                .get(name)
                .map(|info| &info.runtime_field_types);
            let mut ordered = Vec::with_capacity(field_names.len());
            for field_name in field_names {
                let Some(annotation) = field_types.and_then(|types| types.get(field_name)).cloned()
                else {
                    continue;
                };
                if annotation_has_unresolved_inference_variable(&annotation) {
                    return Err(SemanticFreezeError::UnresolvedInferenceVariable {
                        subject: format!("struct field {name}.{field_name}"),
                    });
                }
                ordered.push((field_name.clone(), annotation));
            }
            index.struct_defs.insert(name.clone(), ordered);
        }

        // Named freeze input 2: type aliases (`type_aliases`).
        for (alias, target) in &compiler.type_aliases {
            let annotation = TypeAnnotation::Basic(target.clone());
            if annotation_has_unresolved_inference_variable(&annotation) {
                return Err(SemanticFreezeError::UnresolvedInferenceVariable {
                    subject: format!("type alias {alias}"),
                });
            }
            index.alias_defs.insert(alias.clone(), annotation);
        }

        // Named freeze input 3: enums from the canonical schema registry.
        let registry = compiler.type_tracker.schema_registry();
        for type_name in registry
            .type_names()
            .map(str::to_string)
            .collect::<Vec<_>>()
        {
            let Some(schema) = registry.get(&type_name) else {
                continue;
            };
            let Some(enum_info) = schema.get_enum_info() else {
                continue;
            };
            index.enum_defs.insert(
                type_name,
                enum_info
                    .variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect(),
            );
        }

        // The base freeze is module-scoped: function type parameters enter
        // ONLY through a `FreezeOverlay`, never through the base index.
        index.rebuild_frozen_type_index();
        Ok(Arc::new(Self { index }))
    }

    /// Shared query API: canonical semantic identity for a resolvable name.
    pub(crate) fn identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.index.frozen_type_id(name)
    }

    /// Shared query API: semantic category for a frozen identity.
    pub(crate) fn category_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<FrozenTypeCategory, String> {
        self.index.category_for_identity(identity)
    }

    /// The freeze's internal type index. Visible only inside
    /// `comptime_builtins` (the legacy `type_info` path reads nominal/alias/
    /// enum membership and struct field rows from the SAME frozen index —
    /// no second derivation; E5 deletes that consumer).
    pub(super) fn index(&self) -> &FrozenTypeIndex {
        &self.index
    }
}

/// Scoped generic-parameter overlay over an [`Arc<SemanticFreeze>`].
///
/// Carries `parameter:{owner}:{name}` identities for the enclosing function's
/// declared type parameters WITHOUT rebuilding the base index. Lookup order
/// reproduces the S1 builder's interning order byte-for-byte: a name already
/// frozen in the base (primitive, builtin nominal, user nominal, alias) wins
/// over a same-named parameter, exactly as `intern_identity`'s early return
/// did.
///
/// Known semantic edge (inherited, unchanged): a module-level alias whose
/// target is a function type parameter resolved through the old per-site
/// rebuild's fixpoint; that shape is not valid Shape at module scope and is
/// not reproduced by the overlay.
#[derive(Debug)]
pub(crate) struct FreezeOverlay {
    base: Arc<SemanticFreeze>,
    parameters: HashMap<String, FrozenTypeIdentity>,
}

impl FreezeOverlay {
    /// Overlay `type_params` scoped to `parameter_owner` (the enclosing
    /// function name; module scope uses `"<module>"`, matching the S1
    /// builder's default owner).
    pub(crate) fn new(
        base: Arc<SemanticFreeze>,
        parameter_owner: &str,
        type_params: &[String],
    ) -> Self {
        let mut parameters = HashMap::new();
        for name in type_params {
            // Interning-order parity with `intern_identity`'s name-keyed early
            // return: a name already frozen in the base (primitive, builtin
            // nominal, user nominal, alias) wins over a same-named parameter.
            if base.identity_of(name).is_some() {
                continue;
            }
            let identity = FrozenTypeIdentity::from_canonical_descriptor(&format!(
                "parameter:{parameter_owner}:{name}"
            ));
            parameters.insert(name.clone(), identity);
        }
        Self { base, parameters }
    }

    /// Shared query API: base identities first (interning-order parity), then
    /// this overlay's scoped parameters.
    pub(crate) fn identity_of(&self, name: &str) -> Option<FrozenTypeIdentity> {
        self.base
            .identity_of(name)
            .or_else(|| self.parameters.get(name).copied())
    }

    /// Shared query API: overlay parameters classify as
    /// [`FrozenTypeCategory::Parameter`]; everything else defers to the base.
    pub(crate) fn category_of(
        &self,
        identity: FrozenTypeIdentity,
    ) -> std::result::Result<FrozenTypeCategory, String> {
        if self
            .parameters
            .values()
            .any(|&parameter| parameter == identity)
        {
            return Ok(FrozenTypeCategory::Parameter);
        }
        self.base.category_of(identity)
    }

    /// The shared base freeze this overlay scopes (no rebuild happened).
    pub(crate) fn base(&self) -> &Arc<SemanticFreeze> {
        &self.base
    }

    /// True when `name` is one of this overlay's scoped generic parameters
    /// (i.e. it resolves to [`FrozenTypeCategory::Parameter`] here and is
    /// not shadowed by a base identity).
    pub(crate) fn is_scoped_parameter(&self, name: &str) -> bool {
        self.parameters.contains_key(name)
    }
}

/// Freeze-boundary predicate for rejection-matrix row 4: true when the
/// annotation structurally contains the analyzer's canonical unresolved
/// inference-variable marker. Exhaustive over `TypeAnnotation` so a new
/// variant forces this walk to be revisited.
fn annotation_has_unresolved_inference_variable(annotation: &TypeAnnotation) -> bool {
    if annotation_as_tyvar(annotation).is_some() {
        return true;
    }
    match annotation {
        TypeAnnotation::Basic(_) => false,
        TypeAnnotation::Array(inner) => annotation_has_unresolved_inference_variable(inner),
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => items
            .iter()
            .any(annotation_has_unresolved_inference_variable),
        TypeAnnotation::Object(fields) => fields
            .iter()
            .any(|field| annotation_has_unresolved_inference_variable(&field.type_annotation)),
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|param| annotation_has_unresolved_inference_variable(&param.type_annotation))
                || annotation_has_unresolved_inference_variable(returns)
        }
        TypeAnnotation::Generic { args, .. } => args
            .iter()
            .any(annotation_has_unresolved_inference_variable),
        TypeAnnotation::Borrow { inner, .. } => annotation_has_unresolved_inference_variable(inner),
        TypeAnnotation::Reference(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined => false,
        TypeAnnotation::Dyn(_) => false,
    }
}

impl BytecodeCompiler {
    /// ADR-009 §4.1 registration-complete semantic-freeze barrier: install
    /// the single per-compilation-unit freeze. Runs exactly once per
    /// compilation unit — from the graph entry point before Phase 1 when
    /// compiling with a module graph, else from `compile()` (which consumes
    /// the compiler); a second install is an internal-invariant compile
    /// error, and a freeze-boundary rejection surfaces as a compile error
    /// BEFORE any comptime site could execute (Dec 52).
    pub(crate) fn install_semantic_freeze(&mut self) -> Result<()> {
        if self.semantic_freeze.is_some() {
            return Err(ShapeError::TypeError(
                "semantic freeze already installed: the freeze barrier runs \
                 exactly once per compilation unit"
                    .to_string(),
            ));
        }
        let freeze = SemanticFreeze::freeze(self)
            .map_err(|error| ShapeError::TypeError(error.diagnostic()))?;
        self.semantic_freeze = Some(freeze);
        Ok(())
    }

    /// ADR-009 §4.1 (slice S2): the freeze handle a comptime site consumes.
    ///
    /// Returns the installed per-compilation-unit freeze, scoped by a
    /// [`FreezeOverlay`] over the enclosing function's declared type
    /// parameters when `current_function` is generic (module scope uses the
    /// `"<module>"` owner with no parameters). This replaces the deleted
    /// per-site `build_type_reflection_snapshot` rebuild: the base index is
    /// shared (`Arc`), never rebuilt.
    ///
    /// A comptime site reached without an installed freeze is a compile
    /// error (rejection-matrix row 3, [`NO_FREEZE_HANDLE_DIAGNOSTIC`]) —
    /// never an empty snapshot.
    pub(crate) fn comptime_freeze_overlay(&self) -> Result<Arc<FreezeOverlay>> {
        let Some(freeze) = self.semantic_freeze.as_ref() else {
            return Err(ShapeError::TypeError(
                NO_FREEZE_HANDLE_DIAGNOSTIC.to_string(),
            ));
        };
        let mut parameter_owner = "<module>".to_string();
        let mut type_params: Vec<String> = Vec::new();
        if let Some(function) = self
            .current_function
            .and_then(|index| self.program.functions.get(index))
            && let Some(definition) = self.function_defs.get(&function.name)
        {
            parameter_owner = function.name.clone();
            if let Some(parameters) = &definition.type_params {
                type_params = parameters
                    .iter()
                    .map(|parameter| parameter.name().to_string())
                    .collect();
            }
        }
        // ADR-009 A3 — specialization overlay: while a monomorphized body
        // compiles, the registered def carries `type_params = None`
        // (substitution strips them), so the discovery above finds nothing.
        // The overlay set around `compile_function` in
        // `monomorphization/cache.rs` re-supplies the BASE generic function's
        // declared type-param names, with the owner scoped to the BASE name
        // (never the mono key) so Parameter identities are declaration-stable
        // across instantiations (ADR-009 §Semantic Freeze, Decision 52
        // pre-substitution identities).
        if let Some((base_name, parameters)) = &self.specialization_type_param_overlay {
            parameter_owner = base_name.clone();
            type_params.extend(parameters.iter().cloned());
        }
        Ok(Arc::new(FreezeOverlay::new(
            Arc::clone(freeze),
            &parameter_owner,
            &type_params,
        )))
    }
}

/// Test-only fabricator (S2): a REAL freeze over a fabricated compiler,
/// wrapped in a module-scope overlay. This replaces the deleted
/// `TypeReflectionSnapshot::default()` + field-poking test constructions —
/// tests go through the same single freeze barrier as production code.
#[cfg(test)]
pub(crate) fn overlay_for_tests(compiler: &BytecodeCompiler) -> Arc<FreezeOverlay> {
    let freeze = SemanticFreeze::freeze(compiler).expect("test compiler state must freeze");
    Arc::new(FreezeOverlay::new(freeze, "<module>", &[]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::EnumVariantInfo;
    use shape_runtime::type_system::{TypeVar, tyvar_to_annotation};

    fn compiler_with_module_scope_types() -> BytecodeCompiler {
        let mut compiler = BytecodeCompiler::new();
        // Structs (named freeze input 1): field order + field annotations.
        compiler.struct_types.insert(
            "Point".to_string(),
            (
                vec!["x".to_string(), "y".to_string()],
                shape_ast::ast::Span::DUMMY,
            ),
        );
        compiler.struct_generic_info.insert(
            "Point".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [
                    ("x".to_string(), TypeAnnotation::Basic("int".to_string())),
                    ("y".to_string(), TypeAnnotation::Basic("number".to_string())),
                ]
                .into_iter()
                .collect(),
            },
        );
        compiler.struct_types.insert(
            "Alpha".to_string(),
            (Vec::new(), shape_ast::ast::Span::DUMMY),
        );
        // Aliases (named freeze input 2), including a two-hop chain.
        compiler
            .type_aliases
            .insert("UserId".to_string(), "int".to_string());
        compiler
            .type_aliases
            .insert("First".to_string(), "Second".to_string());
        compiler
            .type_aliases
            .insert("Second".to_string(), "int".to_string());
        // Enum (named freeze input 3): canonical schema registry.
        compiler
            .type_tracker
            .schema_registry_mut()
            .register_enum_scoped(
                "Color",
                vec![
                    EnumVariantInfo::new("Red", 0, 0),
                    EnumVariantInfo::new("Green", 1, 0),
                    EnumVariantInfo::new("Blue", 2, 1),
                ],
            );
        compiler
    }

    /// The S1 parity oracle (byte-identical tables vs the old per-site
    /// builder) retired with the builder's S2 deletion; the identity scheme
    /// stays pinned through the shared query API — structs, enums, aliases,
    /// primitive synonyms, builtin nominals — plus the ongoing 9-test
    /// identity matrix in `type_reflection/tests.rs`.
    #[test]
    fn freeze_pins_identity_scheme_through_the_query_api() {
        let compiler = compiler_with_module_scope_types();
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        for name in ["Point", "Alpha", "Color", "UserId", "First", "int", "Array"] {
            assert!(freeze.identity_of(name).is_some(), "{name} must be frozen");
        }
        // Identity is the canonical-descriptor hash, not an interning-order
        // ordinal: nominal identity is reproducible from the descriptor.
        assert_eq!(
            freeze.identity_of("Point"),
            Some(FrozenTypeIdentity::from_canonical_descriptor(
                "nominal:Point"
            ))
        );
        // Primitive synonyms coalesce; alias chains reach the terminal
        // identity; distinct types stay distinct.
        assert_eq!(freeze.identity_of("int"), freeze.identity_of("i64"));
        assert_eq!(freeze.identity_of("UserId"), freeze.identity_of("int"));
        assert_eq!(freeze.identity_of("First"), freeze.identity_of("int"));
        assert_ne!(freeze.identity_of("int"), freeze.identity_of("bool"));
        let color = freeze.identity_of("Color").expect("Color identity");
        assert_eq!(freeze.category_of(color), Ok(FrozenTypeCategory::Nominal));
        let never = freeze.identity_of("never").expect("never identity");
        assert_eq!(freeze.category_of(never), Ok(FrozenTypeCategory::Never));
    }

    /// S2: `comptime_freeze_overlay` performs the site-side discovery the
    /// deleted builder did — the enclosing generic function's declared type
    /// parameters enter as a scoped overlay (`parameter:{owner}:{name}`)
    /// over the shared base, without a rebuild.
    #[test]
    fn comptime_freeze_overlay_discovers_current_function_type_parameters() {
        let program = shape_ast::parse_program("fn identity<T>(value: T) -> T { value }")
            .expect("generic function parses");
        let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
            panic!("expected function item");
        };
        let mut compiler = BytecodeCompiler::new();
        compiler
            .register_function(definition)
            .expect("generic signature registers");
        compiler.current_function = compiler
            .program
            .functions
            .iter()
            .position(|function| function.name == "identity");
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");

        let overlay = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        let t = overlay.identity_of("T").expect("active T is discovered");
        assert_eq!(
            t,
            FrozenTypeIdentity::from_canonical_descriptor("parameter:identity:T"),
            "parameter identity must be owner-scoped"
        );
        assert_eq!(overlay.category_of(t), Ok(FrozenTypeCategory::Parameter));
        assert!(overlay.is_scoped_parameter("T"));
        // No rebuild: the overlay shares the installed freeze.
        assert!(Arc::ptr_eq(
            overlay.base(),
            compiler.semantic_freeze.as_ref().expect("freeze installed")
        ));
    }

    /// S2 rejection-matrix row 3: a comptime site reached without an
    /// installed freeze is a compile error with the named diagnostic —
    /// never an empty snapshot, never a per-site rebuild.
    #[test]
    fn comptime_site_without_freeze_handle_is_a_named_compile_error() {
        let compiler = BytecodeCompiler::new();
        let error = compiler
            .comptime_freeze_overlay()
            .expect_err("pre-barrier site must not obtain a handle");
        assert!(
            matches!(
                &error,
                ShapeError::TypeError(message)
                    if message.contains("no semantic freeze handle")
            ),
            "row-3 diagnostic missing: {error:?}"
        );
    }

    /// Overlay identities are scoped by owner (`parameter:{owner}:{name}`):
    /// same owner ⇒ same identity, different owner ⇒ different identity.
    /// Mirrors `parameter_identity_is_scoped_by_owning_function`.
    #[test]
    fn overlay_scopes_parameter_identities_by_owner() {
        let freeze = SemanticFreeze::freeze(&BytecodeCompiler::new()).expect("empty unit freezes");
        let params = vec!["T".to_string()];

        let map_a = FreezeOverlay::new(Arc::clone(&freeze), "map", &params);
        let map_b = FreezeOverlay::new(Arc::clone(&freeze), "map", &params);
        let filter = FreezeOverlay::new(Arc::clone(&freeze), "filter", &params);

        let map_t = map_a.identity_of("T").expect("map T identity");
        assert_eq!(map_b.identity_of("T"), Some(map_t));
        assert_ne!(filter.identity_of("T"), Some(map_t));
        assert_eq!(map_a.category_of(map_t), Ok(FrozenTypeCategory::Parameter));
    }

    /// The overlay shares the base index (no rebuild): base lookups pass
    /// through unchanged and a name already frozen in the base wins over a
    /// same-named parameter, matching the builder's interning order.
    #[test]
    fn overlay_reads_base_without_rebuilding() {
        let mut compiler = compiler_with_module_scope_types();
        compiler
            .struct_types
            .insert("T".to_string(), (Vec::new(), shape_ast::ast::Span::DUMMY));
        let freeze = SemanticFreeze::freeze(&compiler).expect("resolved state freezes");

        let overlay = FreezeOverlay::new(
            Arc::clone(&freeze),
            "map",
            &["T".to_string(), "U".to_string()],
        );

        // Same Arc — the base index was not rebuilt.
        assert!(Arc::ptr_eq(overlay.base(), &freeze));
        // Base names resolve identically through the overlay.
        assert_eq!(overlay.identity_of("int"), freeze.identity_of("int"));
        assert_eq!(overlay.identity_of("Point"), freeze.identity_of("Point"));
        // Base nominal `T` shadows the parameter `T` (interning-order parity).
        let t = overlay.identity_of("T").expect("T identity");
        assert_eq!(freeze.identity_of("T"), Some(t));
        assert_eq!(overlay.category_of(t), Ok(FrozenTypeCategory::Nominal));
        // `U` exists only in the overlay.
        let u = overlay.identity_of("U").expect("U identity");
        assert_eq!(freeze.identity_of("U"), None);
        assert_eq!(overlay.category_of(u), Ok(FrozenTypeCategory::Parameter));
    }

    /// Rejection-matrix row 4 (Dec 52): freezing partial semantic state is a
    /// named-diagnostic error, never a populated freeze.
    #[test]
    fn unresolved_inference_variable_cannot_be_frozen() {
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "Aabb".to_string(),
            (vec!["min".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Aabb".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [(
                    "min".to_string(),
                    tyvar_to_annotation(&TypeVar("T7".to_string())),
                )]
                .into_iter()
                .collect(),
            },
        );

        let error =
            SemanticFreeze::freeze(&compiler).expect_err("partial semantic state must not freeze");
        let diagnostic = error.diagnostic();
        assert!(
            diagnostic.contains("cannot be frozen"),
            "named Dec 52 class missing from: {diagnostic}"
        );
        assert!(
            diagnostic.contains("unresolved inference variable"),
            "named Dec 52 cause missing from: {diagnostic}"
        );
        assert!(
            diagnostic.contains("Aabb") && diagnostic.contains("min"),
            "diagnostic must name the frozen subject: {diagnostic}"
        );
    }

    /// Row 4 recursion: the marker is found structurally, not only at the
    /// top level of an annotation.
    #[test]
    fn nested_unresolved_inference_variable_cannot_be_frozen() {
        let mut compiler = BytecodeCompiler::new();
        compiler.struct_types.insert(
            "Holder".to_string(),
            (vec!["items".to_string()], shape_ast::ast::Span::DUMMY),
        );
        compiler.struct_generic_info.insert(
            "Holder".to_string(),
            crate::compiler::StructGenericInfo {
                type_params: Vec::new(),
                runtime_field_types: [(
                    "items".to_string(),
                    TypeAnnotation::Array(Box::new(tyvar_to_annotation(&TypeVar(
                        "T9".to_string(),
                    )))),
                )]
                .into_iter()
                .collect(),
            },
        );

        let error = SemanticFreeze::freeze(&compiler)
            .expect_err("nested partial semantic state must not freeze");
        assert!(error.diagnostic().contains("unresolved inference variable"));
    }

    /// The barrier installs the freeze exactly once per compilation unit; a
    /// second install is an internal-invariant compile error.
    #[test]
    fn semantic_freeze_installs_exactly_once_at_registration_barrier() {
        let mut compiler = BytecodeCompiler::new();
        compiler
            .install_semantic_freeze()
            .expect("first install succeeds");
        assert!(
            compiler.semantic_freeze.is_some(),
            "freeze handle installed"
        );

        let error = compiler
            .install_semantic_freeze()
            .expect_err("second install is rejected");
        assert!(
            matches!(&error, ShapeError::TypeError(message) if message.contains("exactly once")),
            "unexpected error: {error:?}"
        );
    }
}
