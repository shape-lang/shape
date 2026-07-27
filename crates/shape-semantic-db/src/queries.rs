//! The Salsa query layer.
//!
//! Salsa owns exactly what ADR-013 §2 assigns it: revisions, dependency
//! recording, red-green validation, early cutoff, interning and concurrent
//! reads. It owns none of Shape's identities — every identity published here is
//! computed by [`crate::identity`] from canonical data and would be byte-equal
//! in a different process with no database at all.
//!
//! The stop line (R16) is structural, not a convention: the only inputs are
//! source text and the unit map. No `BytecodeCompiler`, program, emission
//! state, journal or backend cache is reachable from any query in this module.

use std::collections::BTreeMap;
use std::sync::Arc;

use shape_ast::ast::program::Program;

use crate::diagnostics::{SemanticDiagnostic, codes};
use crate::facts::{
    CallSiteFacts, CallableFacts, CallableResolution, ContractFacts, ResolutionOutcome,
    ResolvedDefinition, SourceProvenance, Visibility,
};
use crate::identity::{DefinitionIdentity, DefinitionPath, UnitIdentity};
use crate::index::{DeclarationIndex, UnitProvenance, build_index, build_provenance};
use crate::types::NormalizedType;

#[salsa::db]
pub trait SemanticDb: salsa::Database {}

/// One source unit: its module path and its text. The only mutable state in
/// the seam.
#[salsa::input]
pub struct SourceUnit {
    #[returns(ref)]
    pub path: String,
    #[returns(ref)]
    pub text: String,
}

/// The set of units that make up one program, keyed by module path.
#[salsa::input]
pub struct ProgramInputs {
    #[returns(ref)]
    pub units: BTreeMap<String, SourceUnit>,
}

/// A database-local handle for one resolved definition.
///
/// Interning is a Salsa-owned acceleration (ADR-013 §2): the handle is how a
/// query names a definition cheaply. `identity` is the portable fact; the rest
/// is the local site used to find its declaration.
#[salsa::interned]
pub struct DefinitionRef<'db> {
    #[returns(copy)]
    pub identity: DefinitionIdentity,
    #[returns(copy)]
    pub unit: SourceUnit,
    #[returns(ref)]
    pub name: String,
    #[returns(copy)]
    pub ordinal: u32,
}

/// The site of a declaration inside the program. Local accelerator only.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct DeclarationSite {
    pub unit_path: String,
    pub name: String,
    pub ordinal: u32,
}

#[derive(Clone, Debug)]
pub struct ParsedUnit {
    pub program: Arc<Program>,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

/// Parses one unit.
///
/// `no_eq`: the AST has no structural equality, so this query cannot backdate.
/// That is deliberate — equality is established one layer down, by the
/// span-free [`declaration_index`], which is where early cutoff belongs. Adding
/// a comment therefore re-parses and re-indexes, and stops there.
#[salsa::tracked(returns(ref), no_eq)]
pub fn parsed_unit(db: &dyn SemanticDb, unit: SourceUnit) -> ParsedUnit {
    match shape_ast::parse_program(unit.text(db)) {
        Ok(program) => ParsedUnit {
            program: Arc::new(program),
            diagnostics: Vec::new(),
        },
        Err(error) => ParsedUnit {
            program: Arc::new(Program {
                items: Vec::new(),
                docs: Default::default(),
            }),
            diagnostics: vec![SemanticDiagnostic::error(
                codes::PARSE_FAILED,
                [
                    ("unit", unit.path(db).clone()),
                    ("error", error.to_string()),
                ],
            )],
        },
    }
}

/// The span-free declaration index for one unit.
#[salsa::tracked(returns(ref))]
pub fn declaration_index(db: &dyn SemanticDb, unit: SourceUnit) -> DeclarationIndex {
    let parsed = parsed_unit(db, unit);
    let mut index = build_index(unit.path(db), &parsed.program);
    index.diagnostics.extend(parsed.diagnostics.iter().cloned());
    index.diagnostics.sort();
    index
}

/// Spans for one unit. Separate from the index so a span shift cannot
/// invalidate a contract.
#[salsa::tracked(returns(ref))]
pub fn unit_provenance(db: &dyn SemanticDb, unit: SourceUnit) -> UnitProvenance {
    build_provenance(&parsed_unit(db, unit).program)
}

/// Looks up a unit by module path.
///
/// A tracked query rather than a direct map read so that adding an unrelated
/// unit backdates here instead of invalidating every resolution in the program.
#[salsa::tracked]
pub fn unit_for_path(
    db: &dyn SemanticDb,
    program: ProgramInputs,
    path: String,
) -> Option<SourceUnit> {
    program.units(db).get(&path).copied()
}

/// Resolves one callable name as written in one unit.
///
/// Order: a declaration in the unit itself, then an import binding. A local
/// declaration therefore shadows an import of the same name and gets its own
/// identity — an alias resolves to the imported definition's identity, a
/// homonym never does.
#[salsa::tracked(returns(ref))]
pub fn resolve_callable(
    db: &dyn SemanticDb,
    program: ProgramInputs,
    unit: SourceUnit,
    name: String,
) -> CallableResolution {
    let index = declaration_index(db, unit);
    let mut diagnostics = Vec::new();

    if let Some(local) = index.local_binding(&name) {
        if let Some(import) = index.import(&name) {
            diagnostics.push(SemanticDiagnostic::warning(
                codes::IMPORT_SHADOWED_BY_LOCAL_DECLARATION,
                [("name", name.clone()), ("from", import.from_unit.clone())],
            ));
        }
        let path =
            DefinitionPath::top_level_callable(unit.path(db), &local.name, local.same_name_ordinal);
        diagnostics.sort();
        return CallableResolution {
            outcome: ResolutionOutcome::Resolved(ResolvedDefinition {
                identity: path.identity(),
                declaring_unit: unit.path(db).clone(),
                name: local.name.clone(),
                same_name_ordinal: local.same_name_ordinal,
            }),
            written_name: name,
            diagnostics,
        };
    }

    let Some(import) = index.import(&name) else {
        diagnostics.push(SemanticDiagnostic::error(
            codes::UNRESOLVED_CALLABLE,
            [("name", name.clone())],
        ));
        return CallableResolution {
            outcome: ResolutionOutcome::Unresolved,
            written_name: name,
            diagnostics,
        };
    };

    let Some(target_unit) = unit_for_path(db, program, import.from_unit.clone()) else {
        diagnostics.push(SemanticDiagnostic::error(
            codes::UNRESOLVED_IMPORT_UNIT,
            [
                ("from", import.from_unit.clone()),
                ("name", import.exported_name.clone()),
            ],
        ));
        return CallableResolution {
            outcome: ResolutionOutcome::Unresolved,
            written_name: name,
            diagnostics,
        };
    };

    let target_index = declaration_index(db, *target_unit);
    let Some(declaration) = target_index.local_binding(&import.exported_name) else {
        diagnostics.push(SemanticDiagnostic::error(
            codes::IMPORTED_DEFINITION_NOT_FOUND,
            [
                ("from", import.from_unit.clone()),
                ("name", import.exported_name.clone()),
            ],
        ));
        return CallableResolution {
            outcome: ResolutionOutcome::Unresolved,
            written_name: name,
            diagnostics,
        };
    };

    if declaration.visibility != Visibility::Public {
        // The identity is known; the access is not permitted. Publishing both
        // lets tooling navigate while the compiler still reports the error.
        diagnostics.push(SemanticDiagnostic::error(
            codes::IMPORTED_DEFINITION_NOT_PUBLIC,
            [
                ("from", import.from_unit.clone()),
                ("name", import.exported_name.clone()),
            ],
        ));
    }

    let path = DefinitionPath::top_level_callable(
        target_unit.path(db),
        &declaration.name,
        declaration.same_name_ordinal,
    );
    diagnostics.sort();
    CallableResolution {
        outcome: ResolutionOutcome::Resolved(ResolvedDefinition {
            identity: path.identity(),
            declaring_unit: target_unit.path(db).clone(),
            name: declaration.name.clone(),
            same_name_ordinal: declaration.same_name_ordinal,
        }),
        written_name: name,
        diagnostics,
    }
}

/// The span-free semantic core of a callable fact.
#[salsa::tracked(returns(ref))]
pub fn callable_contract<'db>(db: &'db dyn SemanticDb, def: DefinitionRef<'db>) -> ContractFacts {
    let unit = def.unit(db);
    let index = declaration_index(db, unit);
    let name = def.name(db);
    let ordinal = def.ordinal(db);

    match index.callable(name, ordinal) {
        Some(declaration) => ContractFacts {
            identity: def.identity(db),
            path: DefinitionPath::top_level_callable(unit.path(db), name, ordinal),
            visibility: declaration.visibility,
            contract: declaration.contract.clone(),
            diagnostics: declaration.diagnostics.clone(),
        },
        None => ContractFacts {
            identity: def.identity(db),
            path: DefinitionPath::top_level_callable(unit.path(db), name, ordinal),
            visibility: Visibility::Private,
            contract: crate::facts::CallableContract {
                type_params: Vec::new(),
                params: Vec::new(),
                result: NormalizedType::NotDeclared,
                is_async: false,
                is_comptime: false,
            },
            diagnostics: vec![SemanticDiagnostic::error(
                codes::UNRESOLVED_CALLABLE,
                [("name", name.clone())],
            )],
        },
    }
}

/// The published callable fact: contract facts plus source provenance.
#[salsa::tracked(returns(ref))]
pub fn callable_facts<'db>(db: &'db dyn SemanticDb, def: DefinitionRef<'db>) -> CallableFacts {
    let unit = def.unit(db);
    let contract_facts = callable_contract(db, def).clone();
    let provenance_index = unit_provenance(db, unit);
    let (declaration_span, name_span) = provenance_index
        .declaration(def.name(db), def.ordinal(db))
        .unwrap_or_default();

    let diagnostics = contract_facts
        .diagnostics
        .iter()
        .cloned()
        .map(|diagnostic| diagnostic.with_span(name_span))
        .collect();

    CallableFacts {
        contract_facts,
        provenance: SourceProvenance {
            unit_identity: UnitIdentity::for_path(unit.path(db)),
            unit_path: unit.path(db).clone(),
            declaration_span,
            name_span,
        },
        diagnostics,
    }
}

/// The published fact for one call-site occurrence.
///
/// This query is the seam's own downstream consumer of `callable_contract`:
/// the diagnostics below are produced by checking the written arguments against
/// the callee's *published* contract, so a signature edit is observable here.
#[salsa::tracked(returns(ref))]
pub fn call_site_facts(
    db: &dyn SemanticDb,
    program: ProgramInputs,
    unit: SourceUnit,
    occurrence: u32,
) -> Option<CallSiteFacts> {
    let index = declaration_index(db, unit);
    let site = index.call_sites.get(occurrence as usize)?.clone();
    let resolution = resolve_callable(db, program, unit, site.written_name.clone());
    let mut diagnostics = resolution.diagnostics.clone();

    let Some(callee) = resolution.resolved() else {
        diagnostics.sort();
        return Some(CallSiteFacts {
            unit_path: unit.path(db).clone(),
            occurrence,
            written_name: site.written_name,
            callee: None,
            callee_contract_identity: None,
            argument_types: site.argument_types,
            diagnostics,
        });
    };

    let declaring_unit = unit_for_path(db, program, callee.declaring_unit.clone())
        .expect("resolution only names units that are in the program");
    let def = DefinitionRef::new(
        db,
        callee.identity,
        declaring_unit,
        callee.name.clone(),
        callee.same_name_ordinal,
    );
    let contract_facts = callable_contract(db, def);
    let contract = &contract_facts.contract;

    let required = contract
        .params
        .iter()
        .filter(|param| !param.has_default)
        .count();
    let supplied = site.argument_types.len();
    if supplied < required || supplied > contract.params.len() {
        diagnostics.push(SemanticDiagnostic::error(
            codes::CALL_ARGUMENT_COUNT_MISMATCH,
            [
                ("callee", callee.name.clone()),
                ("expected", contract.params.len().to_string()),
                ("actual", supplied.to_string()),
            ],
        ));
    }

    for (position, argument) in site.argument_types.iter().enumerate() {
        let Some(param) = contract.params.get(position) else {
            break;
        };
        match argument {
            Some(actual) if *actual != param.ty => {
                diagnostics.push(SemanticDiagnostic::error(
                    codes::CALL_ARGUMENT_TYPE_MISMATCH,
                    [
                        ("callee", callee.name.clone()),
                        ("index", position.to_string()),
                        ("expected", param.ty.render()),
                        ("actual", actual.render()),
                    ],
                ));
            }
            Some(_) => {}
            None => diagnostics.push(SemanticDiagnostic::note(
                codes::CALL_ARGUMENT_TYPE_NOT_STATIC,
                [
                    ("callee", callee.name.clone()),
                    ("index", position.to_string()),
                ],
            )),
        }
    }

    diagnostics.sort();
    Some(CallSiteFacts {
        unit_path: unit.path(db).clone(),
        occurrence,
        written_name: site.written_name,
        callee: Some(callee.clone()),
        callee_contract_identity: Some(contract_facts.content_identity()),
        argument_types: site.argument_types,
        diagnostics,
    })
}

/// Program-wide map from portable identity to declaration site.
///
/// Only the identity-keyed public entry point uses this; the ordinary path
/// (resolve, then read facts) never does, so its program-wide dependency does
/// not coarsen invalidation for compilation or tooling.
#[salsa::tracked(returns(ref))]
pub fn definition_sites(
    db: &dyn SemanticDb,
    program: ProgramInputs,
) -> BTreeMap<DefinitionIdentity, DeclarationSite> {
    let mut sites = BTreeMap::new();
    for (unit_path, unit) in program.units(db) {
        for declaration in &declaration_index(db, *unit).callables {
            let path = DefinitionPath::top_level_callable(
                unit_path,
                &declaration.name,
                declaration.same_name_ordinal,
            );
            sites.insert(
                path.identity(),
                DeclarationSite {
                    unit_path: unit_path.clone(),
                    name: declaration.name.clone(),
                    ordinal: declaration.same_name_ordinal,
                },
            );
        }
    }
    sites
}
