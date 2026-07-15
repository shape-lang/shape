//! Read-only authored-source projection of the canonical generated-node walk.

use crate::ast::{CaptureClause, FunctionParameter, Statement};

/// One closure located by the same structural traversal that stamps generated
/// node provenance. This clone carries no compiler provenance authority.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedClosureSourcePath {
    pub node_path: Vec<String>,
    pub params: Vec<FunctionParameter>,
    pub body: Vec<Statement>,
    pub captures: Option<CaptureClause>,
}

/// Enumerate authored closure paths without minting or attaching provenance.
/// The implementation is the live stamper's traversal, not a second walker.
pub fn generated_closure_source_paths(
    body: &[Statement],
    root_path: &[String],
) -> Vec<GeneratedClosureSourcePath> {
    let mut cloned_body = body.to_vec();
    let mut source_paths = Vec::new();
    let mut walker = super::Stamper {
        origin: None,
        node_path: root_path.to_vec(),
        source_paths: Some(&mut source_paths),
        next_index: 0,
    };
    walker.statements(&mut cloned_body);
    source_paths
}
