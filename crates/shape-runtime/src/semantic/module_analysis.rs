//! Module system analysis
//!
//! This module handles analysis of imports, exports, modules, type aliases,
//! interfaces, enums, and extend statements.

use shape_ast::ast::{
    EnumDef, ExportItem, ExportStmt, ExtendStatement, ImportItems, ImportStmt, InterfaceDef,
    InterfaceMember, Span, Spanned, TypeAliasDef, TypeName,
};
use shape_ast::error::Result;

use super::type_annotation_to_type;
use super::types;

/// Implementation of module system analysis methods for SemanticAnalyzer
impl super::SemanticAnalyzer {
    /// Analyze an import statement
    pub(super) fn analyze_import(&mut self, import: &ImportStmt) -> Result<()> {
        // Module resolution is done at runtime; here we register imported names
        // with Unknown type since the actual types come from the module

        match &import.items {
            ImportItems::Named(specs) => {
                for spec in specs {
                    let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                    self.symbol_table.define_variable(
                        local_name,
                        types::Type::Unknown,
                        shape_ast::ast::VarKind::Const,
                        true,
                    )?;
                }
            }
            ImportItems::Namespace { name, alias } => {
                let local_name = alias.as_ref().unwrap_or(name);
                self.symbol_table.define_variable(
                    local_name,
                    types::Type::Unknown,
                    shape_ast::ast::VarKind::Const,
                    true,
                )?;
            }
        }

        Ok(())
    }

    /// Analyze an export statement
    pub(super) fn analyze_export(&mut self, export: &ExportStmt) -> Result<()> {
        match &export.item {
            ExportItem::Function(func) => {
                // Export function - analyze it first
                self.analyze_function(func)?;
                // Mark it as exported
            }
            ExportItem::TypeAlias(alias) => {
                // Register exported type alias in module scope
                // Note: Cannot evaluate overrides during static analysis, store placeholder or ignore
                // For static analysis we mostly care about symbol availability

                // Add to exported symbols
                self.exported_symbols.insert(alias.name.clone());
            }
            ExportItem::Named(specs) => {
                // pub { a, b as c }
                for spec in specs {
                    // Verify that spec.name exists in current scope
                    if self.symbol_table.lookup_variable(&spec.name).is_none()
                        && self.symbol_table.lookup_function(&spec.name).is_none()
                    {
                        return Err(self.error_at(
                            Span::DUMMY,
                            format!("Cannot export '{}': not defined", spec.name),
                        ));
                    }
                }
            }
            ExportItem::Enum(enum_def) => {
                self.analyze_enum(enum_def)?;
                self.exported_symbols.insert(enum_def.name.clone());
            }
            ExportItem::Struct(_struct_def) => {
                // Struct types are registered at compile time
            }
            ExportItem::Interface(iface_def) => {
                self.analyze_interface(iface_def)?;
                self.exported_symbols.insert(iface_def.name.clone());
            }
            ExportItem::Trait(trait_def) => {
                // Traits use the same member structure as interfaces
                self.exported_symbols.insert(trait_def.name.clone());
            }
            ExportItem::ForeignFunction(func) => {
                // Foreign functions are treated like regular functions for export purposes
                self.exported_symbols.insert(func.name.clone());
            }
        }

        Ok(())
    }

    /// Analyze a type alias definition
    pub(super) fn analyze_type_alias(&mut self, alias: &TypeAliasDef) -> Result<()> {
        // Validate that the type annotation is well-formed
        let resolved_type = type_annotation_to_type(&alias.type_annotation);

        // Check for type parameters if present
        if let Some(type_params) = &alias.type_params {
            for param in type_params {
                // Validate any default types on type parameters
                if let Some(default_type) = &param.default_type {
                    let _constraint_type = type_annotation_to_type(default_type);
                    // Default type validation is done at instantiation time
                }
            }
        }

        if resolved_type == types::Type::Error {
            return Err(self.error_at(
                Span::DUMMY,
                format!("Invalid type in type alias '{}'", alias.name),
            ));
        }

        // Register the type alias in the symbol table for later resolution
        self.symbol_table.define_type_alias_at(
            &alias.name,
            alias.type_annotation.clone(),
            alias.meta_param_overrides.clone(),
            shape_ast::ast::Span::DUMMY,
        )?;

        Ok(())
    }

    /// Analyze an interface definition
    pub(super) fn analyze_interface(&mut self, interface: &InterfaceDef) -> Result<()> {
        // Validate each member
        for member in &interface.members {
            match member {
                InterfaceMember::Property {
                    type_annotation, ..
                } => {
                    let _prop_type = type_annotation_to_type(type_annotation);
                }
                InterfaceMember::Method {
                    params,
                    return_type,
                    ..
                } => {
                    // Validate parameter types
                    for param in params {
                        let _param_type = type_annotation_to_type(&param.type_annotation);
                    }
                    // Validate return type
                    let _ret_type = type_annotation_to_type(return_type);
                }
                InterfaceMember::IndexSignature { return_type, .. } => {
                    let _ret_type = type_annotation_to_type(return_type);
                }
            }
        }

        Ok(())
    }

    /// Analyze an enum definition
    pub(super) fn analyze_enum(&mut self, enum_def: &EnumDef) -> Result<()> {
        // Track enum member values to check for duplicates
        let mut seen_values: std::collections::HashSet<String> = std::collections::HashSet::new();

        for member in &enum_def.members {
            // Check for duplicate member names
            if !seen_values.insert(member.name.clone()) {
                return Err(self.error_at(
                    Span::DUMMY,
                    format!(
                        "Duplicate enum member '{}' in enum '{}'",
                        member.name, enum_def.name
                    ),
                ));
            }

            // Basic validation of variant payloads
            match &member.kind {
                shape_ast::ast::EnumMemberKind::Struct(fields) => {
                    let mut field_names = std::collections::HashSet::new();
                    for field in fields {
                        if !field_names.insert(field.name.clone()) {
                            return Err(self.error_at(
                                Span::DUMMY,
                                format!(
                                    "Duplicate field '{}' in enum variant '{}::{}'",
                                    field.name, enum_def.name, member.name
                                ),
                            ));
                        }
                    }
                }
                shape_ast::ast::EnumMemberKind::Tuple(_)
                | shape_ast::ast::EnumMemberKind::Unit { .. } => {}
            }
        }

        // Register the enum as a type (accessible as a namespace for its members)
        self.symbol_table.define_variable(
            &enum_def.name,
            types::Type::Unknown, // Enum types need special handling
            shape_ast::ast::VarKind::Const,
            true,
        )?;

        // Store enum definition for later validation (constructors, patterns)
        self.symbol_table.define_enum(enum_def.clone())?;

        Ok(())
    }

    /// Analyze an extend statement
    pub(super) fn analyze_extend(&mut self, extend: &ExtendStatement) -> Result<()> {
        // Validate each method being added
        for method in &extend.methods {
            // Create a scope for method analysis
            self.symbol_table.push_scope();

            // Register 'self' as a variable of the extended type
            let self_type = match &extend.type_name {
                TypeName::Simple(name) => match name.as_str() {
                    "Column" => types::Type::Column(Box::new(types::Type::Unknown)),
                    "Vec" => types::Type::Array(Box::new(types::Type::Unknown)),
                    _ => types::Type::Unknown,
                },
                TypeName::Generic { name, type_args } => {
                    if name == "Column" && !type_args.is_empty() {
                        types::Type::Column(Box::new(type_annotation_to_type(&type_args[0])))
                    } else if name == "Vec" && !type_args.is_empty() {
                        types::Type::Array(Box::new(type_annotation_to_type(&type_args[0])))
                    } else {
                        types::Type::Unknown
                    }
                }
            };

            self.symbol_table.define_variable(
                "self",
                self_type,
                shape_ast::ast::VarKind::Const,
                true,
            )?;

            // Register method parameters
            for param in &method.params {
                let param_type = param
                    .type_annotation
                    .as_ref()
                    .map(type_annotation_to_type)
                    .unwrap_or(types::Type::Unknown);
                let var_kind = if param.is_reference {
                    shape_ast::ast::VarKind::Var
                } else {
                    shape_ast::ast::VarKind::Const
                };
                // Define all variables from the pattern
                for name in param.get_identifiers() {
                    self.symbol_table
                        .define_variable(&name, param_type.clone(), var_kind, true)?;
                }
            }

            // Analyze the when clause if present
            if let Some(when_expr) = &method.when_clause {
                let when_type = self.check_expr_type(when_expr)?;
                if when_type != types::Type::Bool && when_type != types::Type::Unknown {
                    return Err(self.error_at(
                        when_expr.span(),
                        format!(
                            "When clause must be boolean, got {} in method '{}'",
                            when_type, method.name
                        ),
                    ));
                }
            }

            // Analyze the method body
            for stmt in &method.body {
                self.analyze_statement(stmt)?;
            }

            self.symbol_table.pop_scope();
        }

        Ok(())
    }
}
