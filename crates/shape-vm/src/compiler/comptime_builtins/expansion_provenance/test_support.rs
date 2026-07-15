//! Test-only generated-node origins issued through the production authority.

use shape_ast::ast::{GeneratedExpansionFingerprint, GeneratedNodeOrigin, GeneratedNodePath, Span};

use super::{GeneratedOrigin, new_test_node_issuer};

impl GeneratedOrigin {
    /// Mint a carrier-test origin without standing up an entire expansion.
    /// Keeping this on `GeneratedOrigin` preserves the single-mint sentinel.
    pub(crate) fn node_origin_for_tests(
        node_path: &[&str],
        owner_display: &str,
    ) -> GeneratedNodeOrigin {
        new_test_node_issuer().issue(
            GeneratedExpansionFingerprint::from_components(0x0BAD_F00D, 0x0DEF_ACED),
            GeneratedNodePath::try_from_rendered_segments(node_path.iter().copied())
                .expect("test origin path must contain valid structural segments"),
            3,
            Span { start: 5, end: 9 },
            owner_display.to_string(),
        )
    }
}
