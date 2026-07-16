//! Source target collection for the one declaration-discovery fixed point.

use shape_ast::ast::{ExportItem, FunctionDef, Item, ModuleDecl, Program};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::comptime_reflection::NominalShape;

use super::BytecodeCompiler;

/// Whether pass 2 will compile the module's authored item list without first
/// letting module-level comptime replace/remove/add to that topology.
pub(super) fn source_module_topology_is_stable(module: &ModuleDecl) -> bool {
    module.annotations.is_empty()
        && !module
            .items
            .iter()
            .any(|item| matches!(item, Item::Comptime(..)))
}

/// One source-executable annotation target, in the exact lexical form pass 2
/// compiles. The fixed-point evaluator remains in the parent module; this type
/// only makes its source frontier complete.
#[derive(Clone, Debug)]
pub(super) enum DeclarationDiscoveryTarget {
    Struct {
        definition: shape_ast::ast::types::StructTypeDef,
        lexical_module_path: Option<String>,
    },
    Function {
        definition: FunctionDef,
        lexical_module_path: Option<String>,
    },
}

impl DeclarationDiscoveryTarget {
    pub(super) fn name(&self) -> &str {
        match self {
            Self::Struct { definition, .. } => &definition.name,
            Self::Function { definition, .. } => &definition.name,
        }
    }

    pub(super) fn annotations(&self) -> &[shape_ast::ast::functions::Annotation] {
        match self {
            Self::Struct { definition, .. } => &definition.annotations,
            Self::Function { definition, .. } => &definition.annotations,
        }
    }

    pub(super) fn lexical_module_path(&self) -> Option<&str> {
        match self {
            Self::Struct {
                lexical_module_path,
                ..
            }
            | Self::Function {
                lexical_module_path,
                ..
            } => lexical_module_path.as_deref(),
        }
    }

    pub(super) fn nominal_shape(&self) -> NominalShape {
        match self {
            Self::Struct { .. } => NominalShape::Struct,
            Self::Function { .. } => NominalShape::Opaque,
        }
    }

    pub(super) fn comptime_target(
        &self,
    ) -> (super::super::comptime_target::ComptimeTarget, Option<String>) {
        match self {
            Self::Struct { definition, .. } => {
                let fields = definition
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            Some(field.type_annotation.clone()),
                            field.annotations.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                (
                    super::super::comptime_target::ComptimeTarget::from_type(
                        &definition.name,
                        &fields,
                    ),
                    Some(definition.name.clone()),
                )
            }
            Self::Function { definition, .. } => (
                super::super::comptime_target::ComptimeTarget::from_function(definition),
                None,
            ),
        }
    }
}

impl BytecodeCompiler {
    /// Collect the complete v1 annotation-handler discovery frontier.
    ///
    /// Inline-module children are qualified with the same function that pass 2
    /// uses in `compile_module_decl`, including module-local call binding. Module
    /// handlers and raw module `comptime` blocks are intentionally absent: they
    /// mutate module topology through separate pass-2 APIs and cannot join this
    /// fixed point without moving that topology under the same worklist.
    /// Annotated trait defaults are also pass-2-only because their concrete
    /// callable target does not exist until a particular impl installs them.
    pub(super) fn collect_declaration_discovery_targets(
        &mut self,
        program: &Program,
    ) -> Result<Vec<DeclarationDiscoveryTarget>> {
        let mut targets = Vec::new();
        let lexical_module_path = self.module_scope_stack.last().cloned();
        self.collect_declaration_discovery_items(
            &program.items,
            lexical_module_path.as_deref(),
            false,
            &mut targets,
        )?;
        Ok(targets)
    }

    fn collect_declaration_discovery_items(
        &mut self,
        items: &[Item],
        lexical_module_path: Option<&str>,
        qualify_items: bool,
        targets: &mut Vec<DeclarationDiscoveryTarget>,
    ) -> Result<()> {
        let local_functions = Self::module_local_function_names(items);
        for source_item in items {
            let item = if qualify_items {
                let module_path = lexical_module_path.ok_or_else(|| ShapeError::RuntimeError {
                    message: "Internal error: nested declaration discovery lost its lexical module path"
                        .to_string(),
                    location: None,
                })?;
                self.qualify_module_item_with_local_function_calls(
                    source_item,
                    module_path,
                    &local_functions,
                )?
            } else {
                source_item.clone()
            };

            match item {
                Item::StructType(definition, _) => {
                    targets.push(DeclarationDiscoveryTarget::Struct {
                        definition,
                        lexical_module_path: lexical_module_path.map(str::to_string),
                    });
                }
                Item::Function(definition, _)
                    if !definition.is_comptime && !definition.annotations.is_empty() =>
                {
                    targets.push(DeclarationDiscoveryTarget::Function {
                        definition,
                        lexical_module_path: lexical_module_path.map(str::to_string),
                    });
                }
                Item::Export(export, _) => match export.item {
                    ExportItem::Struct(definition) => {
                        targets.push(DeclarationDiscoveryTarget::Struct {
                            definition,
                            lexical_module_path: lexical_module_path.map(str::to_string),
                        });
                    }
                    ExportItem::Function(definition)
                        if !definition.is_comptime && !definition.annotations.is_empty() =>
                    {
                        targets.push(DeclarationDiscoveryTarget::Function {
                            definition,
                            lexical_module_path: lexical_module_path.map(str::to_string),
                        });
                    }
                    _ => {}
                },
                Item::Extend(extend, _) => {
                    for method in extend
                        .methods
                        .iter()
                        .filter(|method| !method.annotations.is_empty())
                    {
                        targets.push(DeclarationDiscoveryTarget::Function {
                            definition: self.desugar_extend_method(method, &extend.type_name)?,
                            lexical_module_path: lexical_module_path.map(str::to_string),
                        });
                    }
                }
                Item::Impl(impl_block, _) if !impl_block.is_comptime => {
                    self.collect_impl_declaration_discovery_targets(
                        &impl_block,
                        lexical_module_path,
                        targets,
                    )?;
                }
                Item::Module(module, _) => {
                    if !source_module_topology_is_stable(&module) {
                        continue;
                    }
                    let module_path = match lexical_module_path {
                        Some(parent) => Self::qualify_module_symbol(parent, &module.name),
                        None => module.name.clone(),
                    };
                    self.module_scope_stack.push(module_path.clone());
                    let collect_result = self.collect_declaration_discovery_items(
                        &module.items,
                        Some(&module_path),
                        true,
                        targets,
                    );
                    self.module_scope_stack.pop();
                    collect_result?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_impl_declaration_discovery_targets(
        &self,
        impl_block: &shape_ast::ast::types::ImplBlock,
        lexical_module_path: Option<&str>,
        targets: &mut Vec<DeclarationDiscoveryTarget>,
    ) -> Result<()> {
        let raw_trait_name = match &impl_block.trait_name {
            shape_ast::ast::TypeName::Simple(name) => name.as_str(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
        };
        let (_, trait_basename) = self.resolve_trait_name(raw_trait_name);

        // The authoritative From/TryFrom path uses `desugar_from_method`, which
        // deliberately emits an unannotated constructor function. Claiming those
        // methods here would invent an application pass 2 never executes.
        if matches!(trait_basename.as_str(), "From" | "TryFrom") {
            return Ok(());
        }

        let type_name = match &impl_block.target_type {
            shape_ast::ast::TypeName::Simple(name) => name.as_str(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
        };
        for method in impl_block
            .methods
            .iter()
            .filter(|method| !method.annotations.is_empty())
        {
            let definition = self.desugar_impl_method(
                method,
                &trait_basename,
                type_name,
                impl_block.impl_name.as_deref(),
                &impl_block.target_type,
            )?;
            targets.push(DeclarationDiscoveryTarget::Function {
                definition,
                lexical_module_path: lexical_module_path.map(str::to_string),
            });
        }
        Ok(())
    }
}
