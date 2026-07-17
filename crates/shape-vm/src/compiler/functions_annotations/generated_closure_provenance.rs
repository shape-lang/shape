//! Node-borne provenance for closures emitted by comptime annotations.
//!
//! Every generated-body copy must be stamped before registration or
//! compilation. This includes pass-2 generated methods and functions, their
//! whole-program analysis copies, the registry copy later rebuilt by generic
//! monomorphization, and replacement bodies that compile under a user function
//! name. The capture gate therefore follows the generated node instead of a
//! function-name predicate, including nested closures and mangled
//! specializations. The hygienic shadow that retains a user's original body is
//! deliberately never passed through this helper.

use super::super::BytecodeCompiler;
use super::super::comptime_builtins::expansion_provenance::{
    ExpansionSite, GeneratedNodePath, GeneratedOrigin, SourceAnchor,
};
use shape_ast::ast::{FunctionDef, GeneratedNodeOrigin, Span, Statement};
use shape_ast::error::Result;

/// Re-base a generated declaration's declaration-level spans to its real
/// application anchor. Handler-emitted declarations originate in synthetic
/// snippets or fragments, so those spans cannot index the compiling file.
/// Body-node spans remain handler-emitted until virtual expansion documents
/// provide per-node mappings.
///
/// Call this only after the raw generated content has been fingerprinted, so
/// speculative and authoritative expansion phases hash identical output.
pub(super) fn anchor_generated_function_decl(func_def: &mut FunctionDef, anchor: Span) {
    func_def.name_span = anchor;
    if let Some(type_params) = func_def.type_params.as_mut() {
        for type_param in type_params {
            match type_param {
                shape_ast::ast::TypeParam::Type { span, .. }
                | shape_ast::ast::TypeParam::Const { span, .. } => *span = anchor,
            }
        }
    }
}

impl BytecodeCompiler {
    pub(super) fn stamp_generated_closure_provenance(
        &self,
        body: &mut [Statement],
        origin: &GeneratedOrigin,
        owner: &str,
    ) {
        shape_ast::transform::stamp_generated_closures(
            body,
            &origin.to_node_origin(&self.generated_node_issuer, owner),
        );
    }

    pub(super) fn stamp_generated_analysis_method(
        &self,
        method: &mut shape_ast::ast::types::MethodDef,
        site: &ExpansionSite,
        source_anchor: SourceAnchor,
        extend_type: &str,
    ) {
        let node_path = GeneratedNodePath::decl_root(format!("extend:{extend_type}"))
            .child(format!("method:{}", method.name));
        let origin = GeneratedOrigin {
            expansion: site.identity().clone(),
            node_path,
            source_anchor,
        };
        let owner = format!("{extend_type}.{}", method.name);
        self.stamp_generated_closure_provenance(&mut method.body, &origin, &owner);
    }

    /// Stamp a `replace body` replacement's closures with generated provenance
    /// AND return the node-borne declaration origin so the caller can arm the D6
    /// async-drop-context gate over the REPLACEMENT (the swapped body compiles
    /// under the user function name, so the gate cannot recover its provenance
    /// from a name — ADR-009 C2 #13 slice 4). The returned origin carries this
    /// compiler instance's issuer capability, so `recognizes` authenticates it.
    pub(super) fn stamp_generated_replacement_body(
        &self,
        function: &mut FunctionDef,
        site: &ExpansionSite,
    ) -> Result<GeneratedNodeOrigin> {
        let source_anchor = site
            .source_anchor()
            .map_err(|message| self.expansion_rejection(message, site))?;
        let origin = GeneratedOrigin {
            expansion: site.identity().clone(),
            node_path: GeneratedNodePath::decl_root(format!("fn:{}", function.name))
                .child("replace_body"),
            source_anchor,
        };
        let owner = function.name.clone();
        self.stamp_generated_closure_provenance(&mut function.body, &origin, &owner);
        Ok(origin.to_node_origin(&self.generated_node_issuer, &owner))
    }
}
