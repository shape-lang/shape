//! ADR-009 D2 / C1 (slice 2) — node-borne generated-code provenance.
//!
//! # Why the AST carries this at all
//!
//! The Wave-46 generated-closure gate (`compile_expr_closure`, shape-vm) has
//! to answer ONE question: *was this closure node produced by a comptime
//! expansion?* Until slice 2 it answered by a NAME predicate —
//! `generated_symbols.contains_name(current_function.name)` — which is holed
//! four ways: a monomorphized specialization compiles under a mangled name; a
//! `replace body` expansion compiles under the USER's function name; a closure
//! nested inside a generated closure compiles under `__closure_N`; and a
//! hygienic body name is not the decl name. A name predicate cannot see any of
//! those.
//!
//! [`GeneratedNodeOrigin`] moves the answer ONTO THE NODE. It is stamped by the
//! compiler at the four points where comptime-produced AST enters the program
//! (`extend`, `extend (items)`, the declaration-discovery pre-pass, and
//! `replace body`), and from there it travels with the node through every
//! transform: monomorphization substitution, `original_body_rewrite`, and the
//! compiler-owned AST transforms. Substitution and the rewrite rebuild
//! `Expr::FunctionExpr` field-by-field with no `..`, so dropping the stamp is a
//! COMPILE ERROR, not a silent loss. Serde preserves the diagnostic fields but
//! deliberately erases compiler-instance authority; an ingestion path must
//! re-stamp a generated payload before it becomes trusted generated code.
//!
//! # What it is NOT
//!
//! It is not an identity of its own. The identity of a generated declaration
//! lives in the compiler's `GeneratedSymbolTable` (`SymbolId`, content-derived
//! from the `ExpansionIdentity`, private constructor). This struct is the
//! stamped VIEW of that identity: [`GeneratedNodeOrigin::expansion_high`] /
//! `expansion_low` are the 128 bits of the owning `ExpansionIdentity`
//! fingerprint, and [`GeneratedNodeOrigin::node_path`] is the structured
//! `GeneratedNodePath` rendering. shape-ast cannot depend on shape-vm, so the
//! carrier lives here and the MINTING stays in
//! `compiler/comptime_builtins/expansion_provenance.rs` — the one file allowed
//! to construct it (`scripts/check-no-dynamic.sh`).
//!
//! [`GeneratedNodeOrigin::owner_display`] is PROSE — the owning declaration's
//! name, used only to render the "in generated function 'f'" tail of a
//! diagnostic. It is never compared, never a key, and never an identity
//! (R1: no name identity on the live path).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::span::Span;

/// Provenance stamped on a generated AST node (today: `Expr::FunctionExpr`).
///
/// Constructed ONLY by the compiler's expansion-provenance module from a
/// registered `GeneratedOrigin`. A `None` field on a node means "this node
/// came from ordinary user source" — the absence is meaningful and total;
/// there is no unknown/partial state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedNodeOrigin {
    /// High/low 64 bits of the owning `ExpansionIdentity` fingerprint
    /// (content-derived SHA-256 — never a counter, never name text).
    expansion_high: i64,
    expansion_low: i64,
    /// Structured path from the generated declaration root to this node,
    /// e.g. `["extend:Job", "method:read", "closure:0"]`. Structural — the
    /// closure segment is the deterministic traversal index, NOT a span.
    node_path: Vec<String>,
    /// The real source anchor (SourceMap file id + span) of the OWNING
    /// declaration's application site. Never `Span::DUMMY`: the compiler
    /// validates it into a `SourceAnchor` before minting.
    anchor_file_id: u16,
    anchor_span: Span,
    /// Diagnostic prose ONLY: the owning declaration's name. Never compared,
    /// never a key, never an identity.
    owner_display: String,
    /// Compilation-instance authority. Deliberately omitted from serde: a
    /// serialized provenance record is useful diagnostic data, but it is not
    /// evidence that the current compiler issued the node.
    #[serde(skip)]
    authority: Option<Arc<()>>,
}

/// Compilation-scoped capability that issues and verifies generated-node
/// provenance.
///
/// The constructor is public because shape-ast cannot know which consumer is
/// the compiler. Trust comes from compiler-instance equality, not constructor
/// visibility: an origin issued by another token (or reconstructed by serde)
/// is rejected by [`GeneratedNodeIssuer::recognizes`].
#[derive(Debug, Clone)]
pub struct GeneratedNodeIssuer {
    authority: Arc<()>,
}

impl Default for GeneratedNodeIssuer {
    fn default() -> Self {
        Self::new()
    }
}

impl GeneratedNodeIssuer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            authority: Arc::new(()),
        }
    }

    /// Issue a node stamp under this exact compilation capability.
    #[must_use]
    pub fn issue(
        &self,
        expansion_fingerprint: (i64, i64),
        node_path: Vec<String>,
        anchor_file_id: u16,
        anchor_span: Span,
        owner_display: String,
    ) -> GeneratedNodeOrigin {
        GeneratedNodeOrigin {
            expansion_high: expansion_fingerprint.0,
            expansion_low: expansion_fingerprint.1,
            node_path,
            anchor_file_id,
            anchor_span,
            owner_display,
            authority: Some(Arc::clone(&self.authority)),
        }
    }

    /// True only for a stamp issued by this exact compiler instance.
    #[must_use]
    pub fn recognizes(&self, origin: &GeneratedNodeOrigin) -> bool {
        origin
            .authority
            .as_ref()
            .is_some_and(|authority| Arc::ptr_eq(authority, &self.authority))
    }
}

impl PartialEq for GeneratedNodeOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.expansion_high == other.expansion_high
            && self.expansion_low == other.expansion_low
            && self.node_path == other.node_path
            && self.anchor_file_id == other.anchor_file_id
            && self.anchor_span == other.anchor_span
            && self.owner_display == other.owner_display
    }
}

impl Eq for GeneratedNodeOrigin {}

impl Hash for GeneratedNodeOrigin {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.expansion_high.hash(state);
        self.expansion_low.hash(state);
        self.node_path.hash(state);
        self.anchor_file_id.hash(state);
        self.anchor_span.hash(state);
        self.owner_display.hash(state);
    }
}

impl GeneratedNodeOrigin {
    /// Extend the node path by one structural segment, keeping the same
    /// expansion identity, anchor and owner. Used when descending into a
    /// nested closure (`closure:0/closure:1`).
    #[must_use]
    pub fn child(&self, segment: impl Into<String>) -> Self {
        let mut node_path = self.node_path.clone();
        node_path.push(segment.into());
        Self {
            node_path,
            ..self.clone()
        }
    }

    /// The 128-bit expansion fingerprint this node was generated under.
    #[must_use]
    pub fn expansion_fingerprint(&self) -> (i64, i64) {
        (self.expansion_high, self.expansion_low)
    }

    #[must_use]
    pub fn node_path(&self) -> &[String] {
        &self.node_path
    }

    #[must_use]
    pub fn anchor(&self) -> (u16, Span) {
        (self.anchor_file_id, self.anchor_span)
    }

    /// Diagnostic prose only (see the struct docs).
    #[must_use]
    pub fn owner_display(&self) -> &str {
        &self.owner_display
    }

    /// Canonical `root/child/…` rendering of the node path, for diagnostics.
    #[must_use]
    pub fn render_path(&self) -> String {
        self.node_path.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(issuer: &GeneratedNodeIssuer) -> GeneratedNodeOrigin {
        issuer.issue(
            (1, 2),
            vec!["extend:Job".to_string(), "closure:0".to_string()],
            3,
            Span { start: 5, end: 9 },
            "Job.read".to_string(),
        )
    }

    #[test]
    fn authority_is_compiler_instance_scoped() {
        let compiler = GeneratedNodeIssuer::new();
        let foreign = GeneratedNodeIssuer::new();
        let origin = issue(&foreign);
        assert!(foreign.recognizes(&origin));
        assert!(!compiler.recognizes(&origin));
    }

    #[test]
    fn serde_round_trip_preserves_data_but_erases_authority() {
        let compiler = GeneratedNodeIssuer::new();
        let issued = issue(&compiler);
        assert!(compiler.recognizes(&issued));

        let json = serde_json::to_string(&issued).unwrap();
        let decoded: GeneratedNodeOrigin = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, issued, "diagnostic provenance must round-trip");
        assert!(
            !compiler.recognizes(&decoded),
            "serde must not confer compiler-issued authority"
        );
    }
}
