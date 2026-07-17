//! Lexical qualification for imported annotation handler bodies.

use std::collections::HashSet;

use shape_ast::ast::AnnotationDef;

use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    pub(super) fn qualify_local_calls_in_annotation_definition(
        definition: &mut AnnotationDef,
        module_path: &str,
        local_functions: &HashSet<String>,
    ) {
        let mut annotation_bindings = HashSet::new();
        for parameter in &mut definition.params {
            if let Some(default) = parameter.default_value.as_mut() {
                Self::qualify_local_calls_in_expr(
                    default,
                    module_path,
                    local_functions,
                    &mut annotation_bindings,
                );
            }
            Self::bind_pattern_names(&parameter.pattern, &mut annotation_bindings);
        }

        for handler in &mut definition.handlers {
            let mut shadowed = annotation_bindings.clone();
            shadowed.extend(
                handler
                    .params
                    .iter()
                    .map(|parameter| parameter.name.clone()),
            );
            Self::qualify_local_calls_in_expr(
                &mut handler.body,
                module_path,
                local_functions,
                &mut shadowed,
            );
        }
    }
}
