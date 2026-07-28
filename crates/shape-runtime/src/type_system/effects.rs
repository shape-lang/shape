//! Closed effect rows as components of function types (ADR-014 §8, R21).
//!
//! An effect row describes what a callable may *do*. It is not authority: a
//! row never grants filesystem, network, FFI, or provider access, and it is
//! never interchangeable with `PermissionSet` (ADR-014 §2). The per-blob
//! `required_permissions` union remains the host-authority mechanism; this
//! module is the typed, compile-time fact.
//!
//! ## What is closed and what is a binder
//!
//! ADR-014 §1 bans open string effects, wildcard effects, row variables, and
//! row polymorphism from *checked or persisted row facts*. §8.3 permits a
//! generic signature to bind explicit [`EffectParamRef`] binders which
//! substitute to closed rows at instantiation — a schema, not an open row.
//! Those are the only two inhabited, meaningful states.
//!
//! ## The third state is a proof gap, not a default
//!
//! [`EffectRow::Unproven`] marks a function type whose row this slice has not
//! derived. It is deliberately **not** the empty row: ADR-014 §8.4 names
//! "absent from the table is not proven pure" as one of the two soundness
//! caveats this work closes rather than inherits, and spelling an underived
//! row as `{}` would re-open exactly that hole one layer up. `Unproven`
//! yields no subset evidence, satisfies no boundary, and is rejected from
//! every persisted fact. The only way to obtain a [`ClosedEffectRow`] from an
//! [`EffectRow`] is [`EffectRow::prove_closed`], whose failure type cannot be
//! constructed outside this module — the same mechanical discipline as
//! `prove_native_kind() -> Result<NativeKind, ProofGap>`.
//!
//! ## Determinism
//!
//! Rows are `BTreeSet`-backed and every rendering, hash, and diagnostic
//! payload sorts atom names explicitly before emitting. No unordered
//! container reaches a diagnostic, a rendered row, or a persisted fact.

use std::collections::BTreeSet;
use std::fmt;

/// The stage a row belongs to. ADR-014 §1: there is no implicit conversion
/// between stages even when both rows carry the same operational identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectStage {
    Runtime,
    Comptime,
}

impl EffectStage {
    pub fn as_str(self) -> &'static str {
        match self {
            EffectStage::Runtime => "runtime",
            EffectStage::Comptime => "comptime",
        }
    }
}

/// The closed operational alphabet of ADR-014 §1. This list is the catalog;
/// adding a member is a catalog-version change, not an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationalEffectId {
    FsRead,
    FsWrite,
    NetConnect,
    NetListen,
    Process,
    Env,
    Time,
    Random,
    Ffi,
}

impl OperationalEffectId {
    pub const ALL: [OperationalEffectId; 9] = [
        OperationalEffectId::FsRead,
        OperationalEffectId::FsWrite,
        OperationalEffectId::NetConnect,
        OperationalEffectId::NetListen,
        OperationalEffectId::Process,
        OperationalEffectId::Env,
        OperationalEffectId::Time,
        OperationalEffectId::Random,
        OperationalEffectId::Ffi,
    ];

    pub fn canonical_name(self) -> &'static str {
        match self {
            OperationalEffectId::FsRead => "FsRead",
            OperationalEffectId::FsWrite => "FsWrite",
            OperationalEffectId::NetConnect => "NetConnect",
            OperationalEffectId::NetListen => "NetListen",
            OperationalEffectId::Process => "Process",
            OperationalEffectId::Env => "Env",
            OperationalEffectId::Time => "Time",
            OperationalEffectId::Random => "Random",
            OperationalEffectId::Ffi => "Ffi",
        }
    }

    /// Parse a surface spelling. Unknown atoms reject — ADR-014 §1 has no
    /// open string effects, so an unrecognized name is a diagnostic, never a
    /// silently-carried opaque atom.
    pub fn from_source_name(name: &str) -> Option<OperationalEffectId> {
        OperationalEffectId::ALL
            .into_iter()
            .find(|id| id.canonical_name() == name)
    }
}

/// A single atom of a row.
///
/// `Remote(ResolvedProviderIdentity)` from ADR-014 §1 is deliberately absent:
/// no `ResolvedProviderIdentity` exists in the workspace yet, and minting a
/// placeholder identity would make exact-equality provider matching (§3)
/// unimplementable-as-specified. The variant lands with provider resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectAtom {
    Operation(OperationalEffectId),
    /// Runtime stage only. `ClosedEffectRow::insert` rejects it at comptime
    /// stage per ADR-014 §1.
    Suspend,
}

impl EffectAtom {
    pub fn canonical_name(self) -> &'static str {
        match self {
            EffectAtom::Operation(id) => id.canonical_name(),
            EffectAtom::Suspend => "Suspend",
        }
    }

    pub fn from_source_name(name: &str) -> Option<EffectAtom> {
        if name == "Suspend" {
            return Some(EffectAtom::Suspend);
        }
        OperationalEffectId::from_source_name(name).map(EffectAtom::Operation)
    }

    fn legal_at(self, stage: EffectStage) -> bool {
        match self {
            EffectAtom::Operation(_) => true,
            EffectAtom::Suspend => stage == EffectStage::Runtime,
        }
    }
}

/// Catalog version. Rows of differing versions never compare (ADR-014 §1:
/// unsupported catalog versions reject at load or admission).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectCatalogVersion(pub u32);

impl EffectCatalogVersion {
    pub const CURRENT: EffectCatalogVersion = EffectCatalogVersion(1);
}

/// Why two rows could not be compared or joined. Stage and catalog mismatches
/// are errors, never coerced away — ADR-014 §1 forbids implicit conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectRowError {
    StageMismatch {
        left: EffectStage,
        right: EffectStage,
    },
    CatalogMismatch {
        left: EffectCatalogVersion,
        right: EffectCatalogVersion,
    },
    AtomIllegalAtStage {
        atom: &'static str,
        stage: EffectStage,
    },
    UnknownAtom {
        name: String,
    },
}

impl fmt::Display for EffectRowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EffectRowError::StageMismatch { left, right } => write!(
                f,
                "effect rows belong to different stages ({} vs {}); ADR-014 §1 permits no implicit conversion",
                left.as_str(),
                right.as_str()
            ),
            EffectRowError::CatalogMismatch { left, right } => write!(
                f,
                "effect rows use different catalog versions ({} vs {})",
                left.0, right.0
            ),
            EffectRowError::AtomIllegalAtStage { atom, stage } => write!(
                f,
                "effect atom `{}` is not part of the {} effect algebra",
                atom,
                stage.as_str()
            ),
            EffectRowError::UnknownAtom { name } => {
                write!(f, "unknown effect atom `{name}`")
            }
        }
    }
}

/// A canonical, sorted, deduplicated, stage-branded, catalog-versioned set of
/// atoms. This is the only shape that may appear in a checked or persisted
/// row fact (ADR-014 §1, §8.3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClosedEffectRow {
    stage: EffectStage,
    catalog_version: EffectCatalogVersion,
    atoms: BTreeSet<EffectAtom>,
}

impl ClosedEffectRow {
    /// The explicitly-pure row — what `! {}` spells. Reached only by a
    /// deliberate purity claim, never by absence of information.
    pub fn pure(stage: EffectStage) -> Self {
        ClosedEffectRow {
            stage,
            catalog_version: EffectCatalogVersion::CURRENT,
            atoms: BTreeSet::new(),
        }
    }

    /// The conservative top row: every atom legal at `stage`. This is what a
    /// derivation returns when it cannot prove a narrower row but must not
    /// claim purity (ADR-014 §8.4 caveat one).
    pub fn conservative_top(stage: EffectStage) -> Self {
        let mut row = ClosedEffectRow::pure(stage);
        for id in OperationalEffectId::ALL {
            row.atoms.insert(EffectAtom::Operation(id));
        }
        if EffectAtom::Suspend.legal_at(stage) {
            row.atoms.insert(EffectAtom::Suspend);
        }
        row
    }

    pub fn from_atoms(
        stage: EffectStage,
        atoms: impl IntoIterator<Item = EffectAtom>,
    ) -> Result<Self, EffectRowError> {
        let mut row = ClosedEffectRow::pure(stage);
        for atom in atoms {
            row.insert(atom)?;
        }
        Ok(row)
    }

    pub fn insert(&mut self, atom: EffectAtom) -> Result<(), EffectRowError> {
        if !atom.legal_at(self.stage) {
            return Err(EffectRowError::AtomIllegalAtStage {
                atom: atom.canonical_name(),
                stage: self.stage,
            });
        }
        self.atoms.insert(atom);
        Ok(())
    }

    pub fn stage(&self) -> EffectStage {
        self.stage
    }

    pub fn catalog_version(&self) -> EffectCatalogVersion {
        self.catalog_version
    }

    pub fn is_pure(&self) -> bool {
        self.atoms.is_empty()
    }

    pub fn atoms(&self) -> impl Iterator<Item = EffectAtom> + '_ {
        self.atoms.iter().copied()
    }

    fn comparable(&self, other: &ClosedEffectRow) -> Result<(), EffectRowError> {
        if self.stage != other.stage {
            return Err(EffectRowError::StageMismatch {
                left: self.stage,
                right: other.stage,
            });
        }
        if self.catalog_version != other.catalog_version {
            return Err(EffectRowError::CatalogMismatch {
                left: self.catalog_version,
                right: other.catalog_version,
            });
        }
        Ok(())
    }

    /// Subsumption is subset (ADR-014 §8.1): a value of row `self` is usable
    /// where row `other` is expected iff `self ⊆ other`.
    pub fn is_subset_of(&self, other: &ClosedEffectRow) -> Result<bool, EffectRowError> {
        self.comparable(other)?;
        Ok(self.atoms.is_subset(&other.atoms))
    }

    /// The join of ADR-014 §3: canonical set union, the least upper bound.
    pub fn union(&self, other: &ClosedEffectRow) -> Result<ClosedEffectRow, EffectRowError> {
        self.comparable(other)?;
        Ok(ClosedEffectRow {
            stage: self.stage,
            catalog_version: self.catalog_version,
            atoms: self.atoms.union(&other.atoms).copied().collect(),
        })
    }

    /// Atoms in `self` that `other` does not permit — the payload a boundary
    /// diagnostic reports. Sorted, like every other rendering here.
    pub fn excess_over(
        &self,
        other: &ClosedEffectRow,
    ) -> Result<Vec<&'static str>, EffectRowError> {
        self.comparable(other)?;
        let mut names: Vec<&'static str> = self
            .atoms
            .difference(&other.atoms)
            .map(|atom| atom.canonical_name())
            .collect();
        names.sort_unstable();
        Ok(names)
    }

    /// Canonical atom names, explicitly sorted. Every render, hash, and
    /// diagnostic payload goes through this one function so there is a single
    /// place where row order is decided.
    pub fn canonical_atom_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.atoms.iter().map(|a| a.canonical_name()).collect();
        names.sort_unstable();
        names
    }

    /// The surface rendering: `{}`, `{FsRead}`, `{FsRead, NetConnect}`.
    pub fn render(&self) -> String {
        format!("{{{}}}", self.canonical_atom_names().join(", "))
    }

    /// Stage- and catalog-covered canonical form. This is the string a row
    /// hash covers; it is stable across runs because the atom list is sorted.
    pub fn canonical_form(&self) -> String {
        format!(
            "{}@{}{}",
            self.stage.as_str(),
            self.catalog_version.0,
            self.render()
        )
    }
}

impl fmt::Display for ClosedEffectRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// An explicit effect binder on a generic signature — `effect F` (ADR-014
/// §8.3). It is a schema binder mirroring `TypeParamRef`, not an open row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectParamRef {
    name: String,
}

impl EffectParamRef {
    pub fn new(name: impl Into<String>) -> Self {
        EffectParamRef { name: name.into() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for EffectParamRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// Returned when a row that is not closed is asked for closed-row evidence.
///
/// The constructor is private to this module: checking code cannot fabricate
/// "I proved the row". Same mechanism as `ProofGap` in
/// `crates/shape-vm/src/compiler/type_tracking.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRowProofGap {
    reason: EffectRowProofGapReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectRowProofGapReason {
    /// A generic schema binder that has not been substituted yet.
    UnsubstitutedParameter(EffectParamRef),
    /// No row was derived for this function type.
    NotDerived,
}

impl EffectRowProofGap {
    pub fn reason(&self) -> &EffectRowProofGapReason {
        &self.reason
    }
}

impl fmt::Display for EffectRowProofGap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            EffectRowProofGapReason::UnsubstitutedParameter(param) => write!(
                f,
                "effect parameter `{param}` is unbound; it must substitute to a closed row before checking, freezing, or persistence (ADR-010 §13)"
            ),
            EffectRowProofGapReason::NotDerived => {
                f.write_str("no effect row has been derived for this function type")
            }
        }
    }
}

/// The row component of a function type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectRow {
    /// A checked, persistable row fact.
    Closed(ClosedEffectRow),
    /// A generic-schema binder awaiting instantiation (ADR-014 §8.3).
    Param(EffectParamRef),
    /// No row derived. Not a purity claim; see the module docs.
    Unproven,
}

impl EffectRow {
    pub fn pure(stage: EffectStage) -> Self {
        EffectRow::Closed(ClosedEffectRow::pure(stage))
    }

    pub fn param(name: impl Into<String>) -> Self {
        EffectRow::Param(EffectParamRef::new(name))
    }

    /// The only route from a row to closed-row evidence.
    pub fn prove_closed(&self) -> Result<&ClosedEffectRow, EffectRowProofGap> {
        match self {
            EffectRow::Closed(row) => Ok(row),
            EffectRow::Param(param) => Err(EffectRowProofGap {
                reason: EffectRowProofGapReason::UnsubstitutedParameter(param.clone()),
            }),
            EffectRow::Unproven => Err(EffectRowProofGap {
                reason: EffectRowProofGapReason::NotDerived,
            }),
        }
    }

    pub fn as_param(&self) -> Option<&EffectParamRef> {
        match self {
            EffectRow::Param(param) => Some(param),
            _ => None,
        }
    }

    pub fn is_unproven(&self) -> bool {
        matches!(self, EffectRow::Unproven)
    }

    /// True iff this row may appear in a persisted *fact* (ADR-014 §8.3). A
    /// declared generic *schema* persists with its binders and is checked by
    /// [`EffectRow::is_persistable_in_schema`] instead.
    pub fn is_persistable_as_fact(&self) -> bool {
        matches!(self, EffectRow::Closed(_))
    }

    /// True iff this row may appear in a persisted declared contract schema:
    /// closed rows and explicit binders, never an underived row.
    pub fn is_persistable_in_schema(&self) -> bool {
        matches!(self, EffectRow::Closed(_) | EffectRow::Param(_))
    }

    /// Substitute effect binders per an instantiation map. A binder with no
    /// entry survives — callers that require closure must then fail
    /// `prove_closed`, which is exactly how ADR-010 §13 is enforced.
    pub fn substitute(&self, bindings: &EffectSubstitution) -> EffectRow {
        match self {
            EffectRow::Param(param) => match bindings.get(param) {
                Some(row) => EffectRow::Closed(row.clone()),
                None => self.clone(),
            },
            other => other.clone(),
        }
    }

    pub fn render(&self) -> String {
        match self {
            EffectRow::Closed(row) => row.render(),
            EffectRow::Param(param) => param.name().to_string(),
            EffectRow::Unproven => "<underived>".to_string(),
        }
    }

    /// Project back to a surface annotation, for round-tripping a type
    /// through `TypeAnnotation`. A proof gap has no honest spelling, so it
    /// projects to `None` — the same thing the source would have said by
    /// omitting the clause — rather than to `! {}`.
    pub fn to_annotation(&self) -> Option<shape_ast::ast::EffectRowAnnotation> {
        use shape_ast::ast::EffectRowAnnotation as Ann;
        match self {
            EffectRow::Closed(row) => Some(Ann::Atoms {
                names: row
                    .canonical_atom_names()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                span: Default::default(),
            }),
            EffectRow::Param(param) => Some(Ann::Param {
                name: param.name().to_string(),
                span: Default::default(),
            }),
            EffectRow::Unproven => None,
        }
    }
}

impl fmt::Display for EffectRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// Instantiation-time bindings from effect binders to closed rows. A binder
/// can only ever bind to a *closed* row (ADR-014 §8.3), so there is no
/// parameter-to-parameter substitution and no chain to iterate to a fixpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectSubstitution {
    bindings: std::collections::BTreeMap<EffectParamRef, ClosedEffectRow>,
}

impl EffectSubstitution {
    pub fn new() -> Self {
        EffectSubstitution::default()
    }

    /// Bind a parameter. Rebinding joins by §3 canonical union rather than
    /// overwriting: a generic used at two rows within one instantiation must
    /// satisfy both, and the least row satisfying both is their union.
    pub fn bind(
        &mut self,
        param: EffectParamRef,
        row: ClosedEffectRow,
    ) -> Result<(), EffectRowError> {
        match self.bindings.get(&param) {
            Some(existing) => {
                let joined = existing.union(&row)?;
                self.bindings.insert(param, joined);
            }
            None => {
                self.bindings.insert(param, row);
            }
        }
        Ok(())
    }

    pub fn get(&self, param: &EffectParamRef) -> Option<&ClosedEffectRow> {
        self.bindings.get(param)
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Deterministic iteration: `BTreeMap` ordered by binder name.
    pub fn iter(&self) -> impl Iterator<Item = (&EffectParamRef, &ClosedEffectRow)> {
        self.bindings.iter()
    }
}

/// Resolve a surface `!` clause into a row (ADR-014 §8.3).
///
/// Atom sets resolve against the closed catalog and an unrecognized name is
/// an error, never an opaque carried atom — §1 has no open string effects.
/// A binder reference resolves to [`EffectRow::Param`]; whether that binder is
/// actually in scope is a separate check made where the scope is known
/// ([`validate_binders_in_scope`]), because this function is called from
/// conversions that do not carry a generic environment.
pub fn resolve_row_annotation(
    annotation: &shape_ast::ast::EffectRowAnnotation,
    stage: EffectStage,
) -> Result<EffectRow, EffectRowError> {
    use shape_ast::ast::EffectRowAnnotation as Ann;
    match annotation {
        Ann::Atoms { names, .. } => {
            let mut row = ClosedEffectRow::pure(stage);
            for name in names {
                let atom = EffectAtom::from_source_name(name)
                    .ok_or_else(|| EffectRowError::UnknownAtom { name: name.clone() })?;
                row.insert(atom)?;
            }
            Ok(EffectRow::Closed(row))
        }
        Ann::Param { name, .. } => Ok(EffectRow::param(name.clone())),
    }
}

/// Resolve an optional clause. `None` means the source declared no row, which
/// is a proof gap and NOT a purity claim — the distinction ADR-014 §8.2 draws
/// between an omitted row and `! {}`.
pub fn resolve_optional_row_annotation(
    annotation: Option<&shape_ast::ast::EffectRowAnnotation>,
    stage: EffectStage,
) -> Result<EffectRow, EffectRowError> {
    match annotation {
        Some(ann) => resolve_row_annotation(ann, stage),
        None => Ok(EffectRow::Unproven),
    }
}

/// Check that every binder a row references is declared by an `effect` binder
/// in scope. Returns the offending name if not.
pub fn validate_binders_in_scope(row: &EffectRow, in_scope: &[String]) -> Result<(), String> {
    match row {
        EffectRow::Param(param) => {
            if in_scope.iter().any(|name| name == param.name()) {
                Ok(())
            } else {
                Err(param.name().to_string())
            }
        }
        _ => Ok(()),
    }
}

/// The outcome of checking an actual row against an expected row at a
/// checking boundary (ADR-014 §8.1: subsumption is subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSubsumption {
    /// `actual ⊆ expected` proved, or both sides are the same binder.
    Holds,
    /// The expected side is a binder and closes to the actual row. This is
    /// the instantiation step of ADR-014 §8.3.
    Binds {
        param: EffectParamRef,
        row: ClosedEffectRow,
    },
    /// One side carries no row fact. Nothing is proved and nothing is
    /// violated — an [`EffectRow::Unproven`] side must not manufacture either
    /// a pass or a failure.
    NoFact,
    /// The actual row exceeds the declared boundary row.
    Exceeds {
        actual: ClosedEffectRow,
        expected: ClosedEffectRow,
        /// Sorted canonical names of the atoms the boundary does not permit.
        excess: Vec<&'static str>,
    },
    /// An unsubstituted binder on the actual side cannot be shown to fit a
    /// closed expected row. ADR-010 §13: it had to close first.
    UnboundActual(EffectParamRef),
    /// Stage or catalog-version mismatch — never coerced away.
    Incomparable(EffectRowError),
}

/// Check `actual` against `expected` under §8.1 subsumption.
///
/// This is the one place row subsumption is decided; unification, subtyping,
/// and boundary checking all route through it so they cannot drift apart.
pub fn subsume(actual: &EffectRow, expected: &EffectRow) -> RowSubsumption {
    match (actual, expected) {
        // No fact on either side: a proof gap proves nothing in either
        // direction. It must not silently satisfy a declared boundary, and it
        // must not fabricate a violation.
        (EffectRow::Unproven, _) | (_, EffectRow::Unproven) => RowSubsumption::NoFact,

        (EffectRow::Closed(a), EffectRow::Closed(e)) => match a.is_subset_of(e) {
            Err(err) => RowSubsumption::Incomparable(err),
            Ok(true) => RowSubsumption::Holds,
            Ok(false) => match a.excess_over(e) {
                Ok(excess) => RowSubsumption::Exceeds {
                    actual: a.clone(),
                    expected: e.clone(),
                    excess,
                },
                Err(err) => RowSubsumption::Incomparable(err),
            },
        },

        // A generic boundary accepting a concrete argument row: the binder
        // closes to that row.
        (EffectRow::Closed(a), EffectRow::Param(param)) => RowSubsumption::Binds {
            param: param.clone(),
            row: a.clone(),
        },

        // Same binder on both sides — the identity case inside a generic body.
        (EffectRow::Param(a), EffectRow::Param(e)) if a == e => RowSubsumption::Holds,

        // Different binders, or a binder checked against a closed row: no
        // closed-row evidence exists, so nothing may be proved.
        (EffectRow::Param(a), _) => RowSubsumption::UnboundActual(a.clone()),
    }
}

/// Structural row equality for type identity.
///
/// `Unproven` compares equal to everything: it asserts no row, so it must not
/// manufacture a type distinction that ADR-014 §8.1 would only draw between
/// two *known* rows.
pub fn rows_structurally_equal(left: &EffectRow, right: &EffectRow) -> bool {
    match (left, right) {
        (EffectRow::Unproven, _) | (_, EffectRow::Unproven) => true,
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_row(atoms: &[EffectAtom]) -> ClosedEffectRow {
        ClosedEffectRow::from_atoms(EffectStage::Runtime, atoms.iter().copied()).unwrap()
    }

    const FS_READ: EffectAtom = EffectAtom::Operation(OperationalEffectId::FsRead);
    const NET_CONNECT: EffectAtom = EffectAtom::Operation(OperationalEffectId::NetConnect);

    #[test]
    fn pure_row_is_subset_of_every_row() {
        let pure = ClosedEffectRow::pure(EffectStage::Runtime);
        let fs = runtime_row(&[FS_READ]);
        assert!(pure.is_subset_of(&fs).unwrap());
        assert!(!fs.is_subset_of(&pure).unwrap());
    }

    #[test]
    fn union_is_the_least_upper_bound() {
        let fs = runtime_row(&[FS_READ]);
        let net = runtime_row(&[NET_CONNECT]);
        let joined = fs.union(&net).unwrap();
        assert_eq!(joined.render(), "{FsRead, NetConnect}");
        assert!(fs.is_subset_of(&joined).unwrap());
        assert!(net.is_subset_of(&joined).unwrap());
    }

    #[test]
    fn rendering_is_deterministic_regardless_of_insertion_order() {
        let forward = runtime_row(&[FS_READ, NET_CONNECT]);
        let backward = runtime_row(&[NET_CONNECT, FS_READ]);
        assert_eq!(forward.render(), backward.render());
        assert_eq!(forward.canonical_form(), backward.canonical_form());
        assert_eq!(forward.render(), "{FsRead, NetConnect}");
    }

    #[test]
    fn stages_do_not_compare_or_join() {
        let runtime = runtime_row(&[FS_READ]);
        let comptime = ClosedEffectRow::from_atoms(EffectStage::Comptime, [FS_READ]).unwrap();
        assert!(matches!(
            runtime.is_subset_of(&comptime),
            Err(EffectRowError::StageMismatch { .. })
        ));
        assert!(matches!(
            runtime.union(&comptime),
            Err(EffectRowError::StageMismatch { .. })
        ));
    }

    #[test]
    fn suspend_is_runtime_only() {
        assert!(ClosedEffectRow::from_atoms(EffectStage::Runtime, [EffectAtom::Suspend]).is_ok());
        assert!(matches!(
            ClosedEffectRow::from_atoms(EffectStage::Comptime, [EffectAtom::Suspend]),
            Err(EffectRowError::AtomIllegalAtStage { .. })
        ));
    }

    #[test]
    fn unproven_is_not_pure_and_proves_nothing() {
        let unproven = EffectRow::Unproven;
        assert!(unproven.prove_closed().is_err());
        assert_ne!(unproven, EffectRow::pure(EffectStage::Runtime));
        assert!(!unproven.is_persistable_as_fact());
        assert!(!unproven.is_persistable_in_schema());
    }

    #[test]
    fn conservative_top_contains_every_operational_atom() {
        let top = ClosedEffectRow::conservative_top(EffectStage::Runtime);
        for id in OperationalEffectId::ALL {
            let single = runtime_row(&[EffectAtom::Operation(id)]);
            assert!(
                single.is_subset_of(&top).unwrap(),
                "{id:?} missing from top"
            );
        }
        assert!(!top.is_pure());
    }

    #[test]
    fn unbound_parameter_never_yields_closed_evidence() {
        let param = EffectRow::param("F");
        let gap = param.prove_closed().unwrap_err();
        assert!(matches!(
            gap.reason(),
            EffectRowProofGapReason::UnsubstitutedParameter(_)
        ));
        // A schema may persist the binder; a *fact* may not.
        assert!(param.is_persistable_in_schema());
        assert!(!param.is_persistable_as_fact());
    }

    #[test]
    fn substitution_closes_a_parameter_to_a_closed_row() {
        let mut subst = EffectSubstitution::new();
        subst
            .bind(EffectParamRef::new("F"), runtime_row(&[FS_READ]))
            .unwrap();
        let closed = EffectRow::param("F").substitute(&subst);
        assert_eq!(closed.prove_closed().unwrap().render(), "{FsRead}");
        assert!(closed.is_persistable_as_fact());
    }

    #[test]
    fn rebinding_a_parameter_joins_rather_than_overwrites() {
        let mut subst = EffectSubstitution::new();
        let param = EffectParamRef::new("F");
        subst.bind(param.clone(), runtime_row(&[FS_READ])).unwrap();
        subst
            .bind(param.clone(), runtime_row(&[NET_CONNECT]))
            .unwrap();
        assert_eq!(subst.get(&param).unwrap().render(), "{FsRead, NetConnect}");
    }

    #[test]
    fn excess_names_are_sorted() {
        let wide = runtime_row(&[
            NET_CONNECT,
            FS_READ,
            EffectAtom::Operation(OperationalEffectId::Env),
        ]);
        let narrow = runtime_row(&[FS_READ]);
        assert_eq!(
            wide.excess_over(&narrow).unwrap(),
            vec!["Env", "NetConnect"]
        );
    }

    #[test]
    fn unknown_atom_names_do_not_parse() {
        assert!(EffectAtom::from_source_name("FsRead").is_some());
        assert!(EffectAtom::from_source_name("Suspend").is_some());
        assert!(EffectAtom::from_source_name("Whatever").is_none());
        assert!(EffectAtom::from_source_name("fsread").is_none());
    }
}
