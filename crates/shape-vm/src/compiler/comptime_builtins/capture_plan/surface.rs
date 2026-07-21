use shape_ast::ast::{CaptureClause, GeneratedNodeOrigin, Span};
use shape_ast::error::{Result, ShapeError};

use super::implicit_capture_message;
use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    /// Authorize the generated-only capture surface before either a
    /// specialization peek or real closure emission can plan/intern it.
    ///
    /// This is the sole lexical/provenance gate. Both callers pass the same
    /// canonical environment-analysis capture set; neither may infer a
    /// replacement plan after this rejects.
    pub(crate) fn validate_capture_surface<'a>(
        &self,
        declared: Option<&CaptureClause>,
        generated_origin: Option<&'a GeneratedNodeOrigin>,
        captured_vars: &[String],
        closure_span: Span,
    ) -> Result<Option<&'a GeneratedNodeOrigin>> {
        // A public AST carrier is data, not authority. Trust only a stamp
        // issued by THIS BytecodeCompiler instance. A foreign issuer or serde
        // round-trip cannot reproduce the non-serialized token.
        let generated_origin = match generated_origin {
            None => None,
            Some(origin) if self.generated_node_issuer.recognizes(origin) => Some(origin),
            Some(_) => {
                return Err(ShapeError::SemanticError {
                    message: "[C0909] generated-node provenance was not issued by this compiler \
                              instance; serialized or externally fabricated provenance is \
                              non-authoritative"
                        .to_string(),
                    location: Some(self.span_to_source_location(closure_span)),
                });
            }
        };

        // Cross-check the legacy generated-declaration name view only where it
        // was sound. Node provenance remains the authority for nested,
        // monomorphized, and replace-body generated closures. ADR-009 C3-S6:
        // the hook-template weave's IMPL SHADOW is excluded — it is a
        // generated declaration by reservation whose body is the USER's own
        // source carried verbatim, so its closures are legitimately unstamped
        // (they keep ordinary capture inference; the name-view heuristic is
        // unsound for that class).
        debug_assert!(
            {
                let enclosing_is_generated_decl = self
                    .current_function
                    .and_then(|idx| self.program.functions.get(idx))
                    .is_some_and(|function| {
                        self.generated_symbols.contains_name(function.name.as_str())
                            && !self
                                .template_weave_shadow_names
                                .contains(function.name.as_str())
                    });
                !enclosing_is_generated_decl || generated_origin.is_some()
            },
            "generated declaration reached capture planning with an unstamped closure node"
        );

        // Capture clauses are generated-code-only. Check this before the
        // capture set so an empty or otherwise valid ordinary clause cannot
        // mint a specialization artifact.
        if declared.is_some() && generated_origin.is_none() {
            return Err(ShapeError::SemanticError {
                message: "[C0903] a capture clause is only valid in comptime-generated code; \
                          ordinary source closures infer their captures — remove the `;` clause"
                    .to_string(),
                location: Some(self.span_to_source_location(closure_span)),
            });
        }

        // Generated closures may not acquire undeclared free captures. The
        // diagnostic text is shared with declared-plan set validation.
        if let Some(origin) = generated_origin
            && declared.is_none()
            && !captured_vars.is_empty()
        {
            let names: Vec<&str> = captured_vars.iter().map(String::as_str).collect();
            return Err(ShapeError::SemanticError {
                message: implicit_capture_message(&names, Some(origin)),
                location: Some(self.span_to_source_location(closure_span)),
            });
        }

        Ok(generated_origin)
    }
}
