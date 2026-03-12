//! Module system parsing for Shape

use crate::error::{Result, ShapeError};
use crate::parser::pair_location;
use pest::iterators::Pair;

use crate::ast::{
    ExportItem, ExportSpec, ExportStmt, ImportItems, ImportSpec, ImportStmt, Item, ModuleDecl,
};
use crate::parser::{Rule, functions, items, pair_span};

/// Parse an import statement
///
/// Handles 3 grammar alternatives:
///   from std::core::math use { a, b }       → Named with path (`::`-separated)
///   use std::core::math as math             → Namespace with alias
///   use std::core::math                      → Namespace without alias (binds `math`)
pub fn parse_import_stmt(pair: Pair<Rule>) -> Result<ImportStmt> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "invalid import statement".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let first_str = first.as_str();
    let first_rule = first.as_rule();

    // Dispatch based on first token
    match first_rule {
        Rule::module_path => {
            let module_path = first_str.to_string();
            match inner.next() {
                Some(pair) if pair.as_rule() == Rule::import_item_list => {
                    // "from <module_path> use { ... }"
                    let specs = parse_import_item_list(pair)?;
                    Ok(ImportStmt {
                        items: ImportItems::Named(specs),
                        from: module_path,
                    })
                }
                Some(pair) if pair.as_rule() == Rule::ident => {
                    // "use <module_path> as <alias>"
                    let alias = pair.as_str().to_string();
                    let local_name = module_path
                        .rsplit("::")
                        .next()
                        .unwrap_or(module_path.as_str())
                        .to_string();
                    Ok(ImportStmt {
                        items: ImportItems::Namespace {
                            name: local_name,
                            alias: Some(alias),
                        },
                        from: module_path,
                    })
                }
                None => {
                    // "use <module_path>"
                    let local_name = module_path
                        .rsplit("::")
                        .next()
                        .unwrap_or(module_path.as_str())
                        .to_string();
                    Ok(ImportStmt {
                        items: ImportItems::Namespace {
                            name: local_name,
                            alias: None,
                        },
                        from: module_path,
                    })
                }
                _ => Err(ShapeError::ParseError {
                    message: "unexpected token in use statement".to_string(),
                    location: Some(pair_loc),
                }),
            }
        }
        _ => Err(ShapeError::ParseError {
            message: format!(
                "unexpected token in import statement: {:?} '{}'",
                first_rule, first_str
            ),
            location: Some(pair_loc.with_hint("use 'from path use { ... }' or 'use path'")),
        }),
    }
}

/// Parse import item list
fn parse_import_item_list(pair: Pair<Rule>) -> Result<Vec<ImportSpec>> {
    let mut imports = Vec::new();

    for item_pair in pair.into_inner() {
        if item_pair.as_rule() == Rule::import_item {
            imports.push(parse_import_item(item_pair)?);
        }
    }

    Ok(imports)
}

/// Parse a single import item
fn parse_import_item(pair: Pair<Rule>) -> Result<ImportSpec> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let item_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected import item name".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    match item_pair.as_rule() {
        Rule::annotation_import_item => {
            let mut annotation_inner = item_pair.into_inner();
            let name_pair = annotation_inner
                .next()
                .ok_or_else(|| ShapeError::ParseError {
                    message: "expected annotation import name".to_string(),
                    location: Some(pair_loc.clone()),
                })?;
            Ok(ImportSpec {
                name: name_pair.as_str().to_string(),
                alias: None,
                is_annotation: true,
            })
        }
        Rule::regular_import_item => {
            let mut regular_inner = item_pair.into_inner();
            let name_pair = regular_inner.next().ok_or_else(|| ShapeError::ParseError {
                message: "expected import item name".to_string(),
                location: Some(pair_loc.clone()),
            })?;
            Ok(ImportSpec {
                name: name_pair.as_str().to_string(),
                alias: regular_inner.next().map(|p| p.as_str().to_string()),
                is_annotation: false,
            })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected import item: {:?}", item_pair.as_rule()),
            location: Some(pair_location(&item_pair)),
        }),
    }
}

/// Parse a pub item (visibility modifier on definitions)
pub fn parse_export_item(pair: Pair<Rule>) -> Result<ExportStmt> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Get first token
    let next_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected pub item content".to_string(),
        location: Some(
            pair_loc
                .clone()
                .with_hint("use 'pub fn', 'pub enum', 'pub type', or 'pub { name }'"),
        ),
    })?;

    let item = match next_pair.as_rule() {
        Rule::foreign_function_def => {
            ExportItem::ForeignFunction(functions::parse_foreign_function_def(next_pair)?)
        }
        Rule::extern_native_function_def => {
            ExportItem::ForeignFunction(functions::parse_extern_native_function_def(next_pair)?)
        }
        Rule::function_def => ExportItem::Function(functions::parse_function_def(next_pair)?),
        Rule::builtin_function_decl => {
            ExportItem::BuiltinFunction(functions::parse_builtin_function_decl(next_pair)?)
        }
        Rule::builtin_type_decl => {
            ExportItem::BuiltinType(crate::parser::types::parse_builtin_type_decl(next_pair)?)
        }
        Rule::type_alias_def => {
            ExportItem::TypeAlias(crate::parser::types::parse_type_alias_def(next_pair)?)
        }
        Rule::enum_def => ExportItem::Enum(crate::parser::types::parse_enum_def(next_pair)?),
        Rule::struct_type_def => {
            ExportItem::Struct(crate::parser::types::parse_struct_type_def(next_pair)?)
        }
        Rule::native_struct_type_def => ExportItem::Struct(
            crate::parser::types::parse_native_struct_type_def(next_pair)?,
        ),
        Rule::interface_def => {
            ExportItem::Interface(crate::parser::types::parse_interface_def(next_pair)?)
        }
        Rule::trait_def => ExportItem::Trait(crate::parser::types::parse_trait_def(next_pair)?),
        Rule::annotation_def => {
            ExportItem::Annotation(crate::parser::extensions::parse_annotation_def(next_pair)?)
        }
        Rule::variable_decl => {
            let var_decl = items::parse_variable_decl(next_pair.clone())?;
            match var_decl.pattern.as_identifier() {
                Some(name) => {
                    let item = ExportItem::Named(vec![ExportSpec {
                        name: name.to_string(),
                        alias: None,
                    }]);
                    return Ok(ExportStmt {
                        item,
                        source_decl: Some(var_decl),
                    });
                }
                None => {
                    return Err(ShapeError::ParseError {
                        message: "destructuring patterns are not supported in pub declarations"
                            .to_string(),
                        location: Some(
                            pair_location(&next_pair)
                                .with_hint("use a simple name: 'pub let name = value'"),
                        ),
                    });
                }
            }
        }
        Rule::export_spec_list => {
            let specs = parse_export_spec_list(next_pair)?;
            ExportItem::Named(specs)
        }
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("unexpected pub item type: {:?}", next_pair.as_rule()),
                location: Some(pair_location(&next_pair)),
            });
        }
    };

    Ok(ExportStmt {
        item,
        source_decl: None,
    })
}

/// Parse export specification list
fn parse_export_spec_list(pair: Pair<Rule>) -> Result<Vec<ExportSpec>> {
    let mut specs = Vec::new();

    for spec_pair in pair.into_inner() {
        if spec_pair.as_rule() == Rule::export_spec {
            specs.push(parse_export_spec(spec_pair)?);
        }
    }

    Ok(specs)
}

/// Parse a single export specification
fn parse_export_spec(pair: Pair<Rule>) -> Result<ExportSpec> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected export specification name".to_string(),
        location: Some(pair_loc),
    })?;
    let name = name_pair.as_str().to_string();
    let alias = inner.next().map(|p| p.as_str().to_string());

    Ok(ExportSpec { name, alias })
}

/// Parse an inline module declaration: `mod Name { ... }`.
pub fn parse_module_decl(pair: Pair<Rule>) -> Result<ModuleDecl> {
    let pair_loc = pair_location(&pair);
    let mut annotations = Vec::new();
    let mut name: Option<String> = None;
    let mut name_span = crate::ast::Span::DUMMY;
    let mut items_out: Vec<Item> = Vec::new();

    for part in pair.into_inner() {
        match part.as_rule() {
            Rule::annotations => {
                annotations = functions::parse_annotations(part)?;
            }
            Rule::ident => {
                if name.is_none() {
                    name = Some(part.as_str().to_string());
                    name_span = pair_span(&part);
                }
            }
            Rule::item => {
                items_out.push(crate::parser::parse_item(part)?);
            }
            Rule::item_recovery => {
                let span = part.as_span();
                let text = part.as_str().trim();
                let preview = if text.len() > 40 {
                    format!("{}...", &text[..40])
                } else {
                    text.to_string()
                };
                return Err(ShapeError::ParseError {
                    message: format!("Syntax error in module body near: {}", preview),
                    location: Some(pair_location(&part).with_length(span.end() - span.start())),
                });
            }
            _ => {}
        }
    }

    let name = name.ok_or_else(|| ShapeError::ParseError {
        message: "missing module name".to_string(),
        location: Some(pair_loc),
    })?;

    Ok(ModuleDecl {
        name,
        name_span,
        doc_comment: None,
        annotations,
        items: items_out,
    })
}
