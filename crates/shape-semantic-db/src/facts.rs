//! The immutable semantic facts this seam publishes.
//!
//! Two layers, deliberately:
//!
//! * [`ContractFacts`] is the span-free semantic core — identity, visibility,
//!   normalized base contract, contract diagnostics. A body edit or a comment
//!   that shifts spans cannot change it, so everything that depends on meaning
//!   (call-site checking) depends on this layer and cuts off early.
//! * [`CallableFacts`] is the published fact of ADR-013 §1 — contract facts
//!   plus source provenance and located diagnostics. Consumers that need to
//!   point at source (LSP, CLI rendering) use this layer.
//!
//! Both layers publish a content identity: a domain-separated digest of the
//! canonical encoding. Two sessions that computed the same facts from the same
//! input agree on it byte for byte, which is the property ADR-013 §7.1 asks
//! compiler and LSP to demonstrate.

use shape_ast::ast::span::Span;

use crate::diagnostics::SemanticDiagnostic;
use crate::identity::{
    CanonicalDigest, ContentDigest, DefinitionIdentity, DefinitionPath, DigestWriter, UnitIdentity,
};
use crate::types::NormalizedType;

const DOMAIN_CONTRACT_FACTS: &str = "shape.semantic.contract-facts";
const DOMAIN_CALLABLE_FACTS: &str = "shape.semantic.callable-facts";
const DOMAIN_CALL_SITE_FACTS: &str = "shape.semantic.call-site-facts";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Visibility {
    Private,
    Public,
}

/// One parameter of a normalized base contract.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ParamContract {
    pub name: String,
    pub ty: NormalizedType,
    pub by_reference: bool,
    pub mutable_reference: bool,
    pub is_const: bool,
    pub has_default: bool,
}

impl CanonicalDigest for ParamContract {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.str(&self.name);
        writer.nested(&self.ty);
        writer.bool(self.by_reference);
        writer.bool(self.mutable_reference);
        writer.bool(self.is_const);
        writer.bool(self.has_default);
    }
}

/// The normalized base callable contract (ADR-011 §2): the facts a dependent
/// check needs before contract elaboration runs. Annotation contributions are
/// explicitly outside this slice's stop line, so this is the *base* contract
/// and is labelled as such wherever it is published.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CallableContract {
    pub type_params: Vec<String>,
    pub params: Vec<ParamContract>,
    pub result: NormalizedType,
    pub is_async: bool,
    pub is_comptime: bool,
}

impl CallableContract {
    /// Renders the contract in Shape's declaration syntax. Presentation only.
    pub fn render(&self, name: &str) -> String {
        let mut rendered = String::new();
        if self.is_comptime {
            rendered.push_str("comptime ");
        }
        if self.is_async {
            rendered.push_str("async ");
        }
        rendered.push_str("fn ");
        rendered.push_str(name);
        if !self.type_params.is_empty() {
            rendered.push('<');
            rendered.push_str(&self.type_params.join(", "));
            rendered.push('>');
        }
        rendered.push('(');
        let params: Vec<String> = self
            .params
            .iter()
            .map(|param| {
                let mut text = String::new();
                if param.is_const {
                    text.push_str("const ");
                }
                if param.by_reference {
                    text.push('&');
                    if param.mutable_reference {
                        text.push_str("mut ");
                    }
                }
                text.push_str(&param.name);
                text.push_str(": ");
                text.push_str(&param.ty.render());
                text
            })
            .collect();
        rendered.push_str(&params.join(", "));
        rendered.push(')');
        if self.result != NormalizedType::NotDeclared {
            rendered.push_str(" -> ");
            rendered.push_str(&self.result.render());
        }
        rendered
    }
}

impl CanonicalDigest for CallableContract {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.seq(&self.type_params);
        writer.seq(&self.params);
        writer.nested(&self.result);
        writer.bool(self.is_async);
        writer.bool(self.is_comptime);
    }
}

/// Span-free semantic core of a callable fact.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ContractFacts {
    pub identity: DefinitionIdentity,
    pub path: DefinitionPath,
    pub visibility: Visibility,
    pub contract: CallableContract,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

impl ContractFacts {
    pub fn content_identity(&self) -> ContentDigest {
        self.canonical_digest(DOMAIN_CONTRACT_FACTS)
    }
}

impl CanonicalDigest for ContractFacts {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.nested(&self.identity);
        writer.str(&self.path.unit_path);
        writer.u8(match self.path.kind {
            crate::identity::DefinitionKind::Callable => 1,
        });
        writer.seq(&self.path.scope);
        writer.str(&self.path.name);
        writer.u32(self.path.same_name_ordinal);
        writer.u8(match self.visibility {
            Visibility::Private => 1,
            Visibility::Public => 2,
        });
        writer.nested(&self.contract);
        writer.seq(&self.diagnostics);
    }
}

/// Where a published definition came from.
///
/// Spans are provenance, not meaning: they live here and in the facts layer's
/// diagnostics, never in [`ContractFacts`].
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SourceProvenance {
    pub unit_identity: UnitIdentity,
    pub unit_path: String,
    pub declaration_span: Span,
    pub name_span: Span,
}

impl CanonicalDigest for SourceProvenance {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.nested(&self.unit_identity);
        writer.str(&self.unit_path);
        writer.usize(self.declaration_span.start);
        writer.usize(self.declaration_span.end);
        writer.usize(self.name_span.start);
        writer.usize(self.name_span.end);
    }
}

/// The fact published by `callable_facts(DefinitionIdentity)` (ADR-013 §1).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CallableFacts {
    pub contract_facts: ContractFacts,
    pub provenance: SourceProvenance,
    /// Contract diagnostics with provenance spans attached.
    pub diagnostics: Vec<SemanticDiagnostic>,
}

impl CallableFacts {
    pub fn identity(&self) -> DefinitionIdentity {
        self.contract_facts.identity
    }

    pub fn name(&self) -> &str {
        &self.contract_facts.path.name
    }

    pub fn contract(&self) -> &CallableContract {
        &self.contract_facts.contract
    }

    /// The shared content identity. Compiler and LSP compare *this*, not
    /// rendered text.
    pub fn content_identity(&self) -> ContentDigest {
        self.canonical_digest(DOMAIN_CALLABLE_FACTS)
    }
}

impl CanonicalDigest for CallableFacts {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.nested(&self.contract_facts);
        writer.nested(&self.provenance);
        writer.seq(&self.diagnostics);
    }
}

/// How a name at a use site resolved.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResolutionOutcome {
    /// Resolved to a declaration in this program.
    Resolved(ResolvedDefinition),
    /// No declaration is in scope under that name.
    Unresolved,
}

/// A resolved definition: the portable identity plus the database-local site
/// used to reach its facts. The site is an accelerator; only `identity` is
/// portable (ADR-013 §2).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ResolvedDefinition {
    pub identity: DefinitionIdentity,
    pub declaring_unit: String,
    pub name: String,
    pub same_name_ordinal: u32,
}

/// The result of resolving one name in one unit.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CallableResolution {
    pub outcome: ResolutionOutcome,
    /// The spelling written at the use site. Presentation: it does not select
    /// anything, and an alias resolves to the same identity as the original.
    pub written_name: String,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

impl CallableResolution {
    pub fn identity(&self) -> Option<DefinitionIdentity> {
        match &self.outcome {
            ResolutionOutcome::Resolved(definition) => Some(definition.identity),
            ResolutionOutcome::Unresolved => None,
        }
    }

    pub fn resolved(&self) -> Option<&ResolvedDefinition> {
        match &self.outcome {
            ResolutionOutcome::Resolved(definition) => Some(definition),
            ResolutionOutcome::Unresolved => None,
        }
    }
}

/// The published fact for one call site: which definition it names, and how it
/// checks against that definition's published contract.
///
/// This is the seam-internal downstream consumer required by R17: the fact is
/// load-bearing because changing the callee's declared signature changes the
/// diagnostics published here.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CallSiteFacts {
    pub unit_path: String,
    pub occurrence: u32,
    pub written_name: String,
    pub callee: Option<ResolvedDefinition>,
    /// Content identity of the contract facts this call site was checked
    /// against. Proves the check consumed the published fact.
    pub callee_contract_identity: Option<ContentDigest>,
    pub argument_types: Vec<Option<NormalizedType>>,
    pub diagnostics: Vec<SemanticDiagnostic>,
}

impl CallSiteFacts {
    pub fn content_identity(&self) -> ContentDigest {
        self.canonical_digest(DOMAIN_CALL_SITE_FACTS)
    }

    pub fn callee_identity(&self) -> Option<DefinitionIdentity> {
        self.callee.as_ref().map(|callee| callee.identity)
    }
}

impl CanonicalDigest for CallSiteFacts {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        writer.str(&self.unit_path);
        writer.u32(self.occurrence);
        writer.str(&self.written_name);
        match &self.callee {
            None => writer.u8(0),
            Some(callee) => {
                writer.u8(1);
                writer.nested(&callee.identity);
                writer.str(&callee.declaring_unit);
                writer.str(&callee.name);
                writer.u32(callee.same_name_ordinal);
            }
        }
        match self.callee_contract_identity {
            None => writer.u8(0),
            Some(digest) => {
                writer.u8(1);
                writer.digest(digest);
            }
        }
        writer.usize(self.argument_types.len());
        for argument in &self.argument_types {
            match argument {
                None => writer.u8(0),
                Some(ty) => {
                    writer.u8(1);
                    writer.nested(ty);
                }
            }
        }
        writer.seq(&self.diagnostics);
    }
}
