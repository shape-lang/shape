//! `SemanticDb` — the shared incremental semantic seam (ADR-011, ADR-013, R16).
//!
//! This crate holds the first production slice of Shape's semantic query
//! graph. It publishes, for a callable and a call site, exactly four things:
//! the resolved [`DefinitionIdentity`], the normalized base contract,
//! deterministic diagnostics, and source provenance. The compiler and the LSP
//! read those facts from this one query — there is no parallel tooling copy.
//!
//! # The stop line (R16)
//!
//! This slice does not migrate annotations, generated symbols, method tables,
//! discovery, comptime, typed Core/MIR, or backend state. `BytecodeCompiler`,
//! bytecode programs, mutable expression and function stacks, journals, backend
//! caches, and VM/JIT state are not inputs, tracked values, or query-owned
//! mutable state here — the crate does not depend on `shape-vm` at all, so the
//! line is enforced by the dependency graph rather than by review.
//!
//! # What Salsa owns, and what it does not
//!
//! Salsa owns revisions, dependency recording, red-green validation, early
//! cutoff, interning and concurrent reads. Shape owns identity: every published
//! identity is a domain-separated digest computed in [`identity`], derived from
//! canonical data alone. No Salsa id enters a published fact, and none could —
//! the pre-image functions take no database.
//!
//! # Example
//!
//! ```
//! use shape_semantic_db::SemanticSession;
//!
//! let mut session = SemanticSession::new();
//! session.insert_unit("app::math", "pub fn add(a: int, b: int) -> int { a + b }\n");
//! session.insert_unit(
//!     "app::main",
//!     "from app::math use { add }\nlet total = add(1, 2)\n",
//! );
//!
//! let facts = session.callable_facts_of("app::main", "add").unwrap();
//! assert_eq!(facts.contract().render("add"), "fn add(a: int, b: int) -> int");
//!
//! // The call site names the same definition the declaration published.
//! let call = session.call_site_facts("app::main", 0).unwrap();
//! assert_eq!(call.callee_identity(), Some(facts.identity()));
//! ```

pub mod diagnostics;
pub mod facts;
pub mod identity;
pub mod index;
pub mod queries;
pub mod trace;
pub mod types;

#[cfg(test)]
mod acceptance;

use std::collections::BTreeMap;

use shape_ast::ast::span::Span;

pub use diagnostics::{DiagnosticSeverity, SemanticDiagnostic};
pub use facts::{
    CallSiteFacts, CallableContract, CallableFacts, CallableResolution, ContractFacts,
    ParamContract, ResolutionOutcome, ResolvedDefinition, SourceProvenance, Visibility,
};
pub use identity::{
    ContentDigest, DefinitionIdentity, DefinitionKind, DefinitionPath, IDENTITY_SCHEME_VERSION,
    UnitIdentity,
};
pub use queries::{ProgramInputs, SemanticDb, SourceUnit};
pub use trace::{QueryEvent, QueryEventKind, QueryTrace, QueryTraceRecorder};
pub use types::NormalizedType;

use queries::DefinitionRef;
use salsa::Setter;

/// The Salsa database backing one semantic session.
#[salsa::db]
pub struct SemanticDatabase {
    storage: salsa::Storage<Self>,
    recorder: Option<QueryTraceRecorder>,
}

impl SemanticDatabase {
    fn new(recorder: Option<QueryTraceRecorder>) -> Self {
        let callback_recorder = recorder.clone();
        let storage = salsa::Storage::new(callback_recorder.map(|recorder| {
            Box::new(move |event: salsa::Event| recorder.record(&event))
                as Box<dyn Fn(salsa::Event) + Send + Sync + 'static>
        }));
        SemanticDatabase { storage, recorder }
    }
}

#[salsa::db]
impl salsa::Database for SemanticDatabase {}

#[salsa::db]
impl SemanticDb for SemanticDatabase {}

/// Memory held by one query's memos.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QueryMemoryEntry {
    pub query: String,
    pub bytes: usize,
}

/// A measured query-memory report for one session.
///
/// R16 requires an initial query-memory budget to be recorded rather than
/// assumed; this is how the budget is measured. See
/// `docs/program/adr011-012/salsa-seam.md`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct QueryMemoryReport {
    pub struct_bytes: usize,
    pub memo_bytes: usize,
    pub queries: Vec<QueryMemoryEntry>,
}

impl QueryMemoryReport {
    pub fn total_bytes(&self) -> usize {
        self.struct_bytes + self.memo_bytes
    }
}

/// One semantic session: a database plus the program's units.
///
/// # Ownership, cancellation and snapshots
///
/// A session is single-owner. Mutations go through `&mut self`
/// ([`SemanticSession::insert_unit`], [`SemanticSession::set_unit_text`]),
/// reads through `&self`; Rust's borrow checker therefore enforces that no read
/// is outstanding when a revision starts, which is exactly the invariant
/// Salsa's cancellation machinery exists to protect. This slice does not hand
/// out cross-thread snapshots (`salsa::StorageHandle` clones) and does not run
/// parallel readers, so no cancellation policy of Shape's own is needed yet;
/// the LSP consumes an ephemeral session per request. A later slice that keeps
/// a long-lived LSP session must adopt handle cloning and decide the
/// cancellation policy explicitly.
pub struct SemanticSession {
    db: SemanticDatabase,
    program: ProgramInputs,
    units: BTreeMap<String, SourceUnit>,
}

impl Default for SemanticSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticSession {
    /// Creates an empty session.
    pub fn new() -> Self {
        Self::build(None)
    }

    /// Creates a session that records Salsa's query events for [`Self::take_trace`].
    pub fn with_query_trace() -> Self {
        Self::build(Some(QueryTraceRecorder::new()))
    }

    fn build(recorder: Option<QueryTraceRecorder>) -> Self {
        let db = SemanticDatabase::new(recorder);
        let program = ProgramInputs::new(&db, BTreeMap::new());
        SemanticSession {
            db,
            program,
            units: BTreeMap::new(),
        }
    }

    /// Adds a unit, or replaces the text of one that is already present.
    pub fn insert_unit(&mut self, unit_path: &str, text: &str) {
        match self.units.get(unit_path) {
            Some(_) => self.set_unit_text(unit_path, text),
            None => {
                let unit = SourceUnit::new(&self.db, unit_path.to_string(), text.to_string());
                self.units.insert(unit_path.to_string(), unit);
                let units = self.units.clone();
                self.program.set_units(&mut self.db).to(units);
            }
        }
    }

    /// Replaces the text of an existing unit, starting a new revision.
    pub fn set_unit_text(&mut self, unit_path: &str, text: &str) {
        let Some(unit) = self.units.get(unit_path).copied() else {
            return;
        };
        unit.set_text(&mut self.db).to(text.to_string());
    }

    pub fn unit_paths(&self) -> Vec<String> {
        self.units.keys().cloned().collect()
    }

    fn unit(&self, unit_path: &str) -> Option<SourceUnit> {
        self.units.get(unit_path).copied()
    }

    /// `callable_facts(DefinitionIdentity) -> CallableFacts` (ADR-013 §1).
    ///
    /// The portable identity is the key: a consumer holding an identity from an
    /// artifact, a diagnostic, or another session can ask this session for the
    /// same facts.
    pub fn callable_facts(&self, identity: DefinitionIdentity) -> Option<CallableFacts> {
        let sites = queries::definition_sites(&self.db, self.program);
        let site = sites.get(&identity)?;
        let unit = self.unit(&site.unit_path)?;
        let def = DefinitionRef::new(&self.db, identity, unit, site.name.clone(), site.ordinal);
        Some(queries::callable_facts(&self.db, def).clone())
    }

    /// Facts for the definition a bare name refers to in one unit, following
    /// imports and aliases exactly as resolution does.
    pub fn callable_facts_of(&self, unit_path: &str, name: &str) -> Option<CallableFacts> {
        let resolution = self.resolve_callable(unit_path, name)?;
        let resolved = resolution.resolved()?;
        let unit = self.unit(&resolved.declaring_unit)?;
        let def = DefinitionRef::new(
            &self.db,
            resolved.identity,
            unit,
            resolved.name.clone(),
            resolved.same_name_ordinal,
        );
        Some(queries::callable_facts(&self.db, def).clone())
    }

    /// Facts for one declaration by its structural position, without going
    /// through resolution.
    pub fn declared_callable_facts(
        &self,
        unit_path: &str,
        name: &str,
        ordinal: u32,
    ) -> Option<CallableFacts> {
        let unit = self.unit(unit_path)?;
        let identity = DefinitionPath::top_level_callable(unit_path, name, ordinal).identity();
        let def = DefinitionRef::new(&self.db, identity, unit, name.to_string(), ordinal);
        Some(queries::callable_facts(&self.db, def).clone())
    }

    /// Resolves a callable name as written in one unit.
    pub fn resolve_callable(&self, unit_path: &str, name: &str) -> Option<CallableResolution> {
        let unit = self.unit(unit_path)?;
        Some(queries::resolve_callable(&self.db, self.program, unit, name.to_string()).clone())
    }

    /// The normalized base contract layer, without provenance.
    pub fn contract_facts_of(&self, unit_path: &str, name: &str) -> Option<ContractFacts> {
        let resolution = self.resolve_callable(unit_path, name)?;
        let resolved = resolution.resolved()?;
        let unit = self.unit(&resolved.declaring_unit)?;
        let def = DefinitionRef::new(
            &self.db,
            resolved.identity,
            unit,
            resolved.name.clone(),
            resolved.same_name_ordinal,
        );
        Some(queries::callable_contract(&self.db, def).clone())
    }

    pub fn call_site_count(&self, unit_path: &str) -> usize {
        self.unit(unit_path)
            .map(|unit| queries::declaration_index(&self.db, unit).call_sites.len())
            .unwrap_or(0)
    }

    /// Facts for one call-site occurrence in a unit.
    pub fn call_site_facts(&self, unit_path: &str, occurrence: u32) -> Option<CallSiteFacts> {
        let unit = self.unit(unit_path)?;
        queries::call_site_facts(&self.db, self.program, unit, occurrence).clone()
    }

    /// The source span of one call-site occurrence (provenance layer).
    pub fn call_site_span(&self, unit_path: &str, occurrence: u32) -> Option<Span> {
        let unit = self.unit(unit_path)?;
        queries::unit_provenance(&self.db, unit).call_site(occurrence)
    }

    /// Every callable declared in a unit, in source order.
    pub fn declared_callables(&self, unit_path: &str) -> Vec<(String, u32)> {
        self.unit(unit_path)
            .map(|unit| {
                queries::declaration_index(&self.db, unit)
                    .callables
                    .iter()
                    .map(|declaration| (declaration.name.clone(), declaration.same_name_ordinal))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Unit-level diagnostics (parse failures, duplicate declarations).
    pub fn unit_diagnostics(&self, unit_path: &str) -> Vec<SemanticDiagnostic> {
        self.unit(unit_path)
            .map(|unit| {
                queries::declaration_index(&self.db, unit)
                    .diagnostics
                    .clone()
            })
            .unwrap_or_default()
    }

    /// Takes the recorded query events since the last call. Empty unless the
    /// session was created with [`Self::with_query_trace`].
    ///
    /// Salsa reports keys as `query(Id(n))`. Ids of this session's units are
    /// rewritten to their module path, so a trace reads `declaration_index(app::math)`
    /// and a test can say which unit re-executed.
    pub fn take_trace(&self) -> QueryTrace {
        use salsa::plumbing::AsId;

        let mut trace = self
            .db
            .recorder
            .as_ref()
            .map(QueryTraceRecorder::take)
            .unwrap_or_default();
        let labels: Vec<(String, String)> = self
            .units
            .iter()
            .map(|(path, unit)| (format!("({:?})", unit.as_id()), format!("({path})")))
            .collect();
        for event in &mut trace.events {
            for (id, path) in &labels {
                if event.key.ends_with(id) {
                    event.key = event.key.replace(id, path);
                    break;
                }
            }
        }
        trace
    }

    /// Measures memo memory currently held by this session.
    pub fn query_memory(&self) -> QueryMemoryReport {
        let db: &dyn salsa::Database = &self.db;
        let usage = db.memory_usage();
        let struct_bytes: usize = usage
            .structs
            .iter()
            .map(|info| {
                info.size_of_fields()
                    + info.size_of_metadata()
                    + info.heap_size_of_fields().unwrap_or(0)
            })
            .sum();
        let mut queries: Vec<QueryMemoryEntry> = usage
            .queries
            .iter()
            .map(|(name, info)| QueryMemoryEntry {
                query: (*name).to_string(),
                bytes: info.size_of_fields()
                    + info.size_of_metadata()
                    + info.heap_size_of_fields().unwrap_or(0),
            })
            .collect();
        queries.sort_by(|a, b| a.query.cmp(&b.query));
        let memo_bytes = queries.iter().map(|entry| entry.bytes).sum();
        QueryMemoryReport {
            struct_bytes,
            memo_bytes,
            queries,
        }
    }
}
