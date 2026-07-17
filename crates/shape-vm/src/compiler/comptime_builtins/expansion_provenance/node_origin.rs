//! Projection from compiler expansion provenance to the AST carrier.

use shape_ast::ast::{GeneratedExpansionFingerprint, GeneratedNodeIssuer, GeneratedNodeOrigin};

use super::GeneratedOrigin;

impl GeneratedOrigin {
    /// Project a registered declaration origin into the node-borne AST stamp.
    ///
    /// The carrier is defined in `shape-ast`, but this compiler-owned mint
    /// binds it to the expansion fingerprint, structured node path, real
    /// source anchor, and current compiler-instance issuer capability.
    pub(crate) fn to_node_origin(
        &self,
        issuer: &GeneratedNodeIssuer,
        owner_display: &str,
    ) -> GeneratedNodeOrigin {
        let fingerprint = self.expansion.fingerprint();
        issuer.issue(
            GeneratedExpansionFingerprint::from_components(fingerprint.high, fingerprint.low),
            self.node_path.clone(),
            self.source_anchor.file_id(),
            self.source_anchor.span(),
            owner_display.to_string(),
        )
    }
}
