//! Parser for data source and query declarations

use crate::ast::data_sources::{DataSourceDecl, QueryDecl};
use crate::error::{Result, ShapeError};
use pest::iterators::Pair;

use super::expressions;
use super::types::parse_type_annotation;
use super::{Rule, pair_location, pair_span};

/// Parse a datasource declaration: `datasource Name: Type = expr`
pub fn parse_datasource_def(pair: Pair<Rule>) -> Result<DataSourceDecl> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse name
    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected datasource name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let name_span = pair_span(&name_pair);
    let name = name_pair.as_str().to_string();

    // Parse type annotation (DataSource<T>)
    let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected type annotation after ':'".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let schema = parse_type_annotation(type_pair)?;

    // Parse provider expression
    let expr_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected provider expression after '='".to_string(),
        location: Some(pair_loc),
    })?;
    let provider_expr = expressions::parse_expression(expr_pair)?;

    Ok(DataSourceDecl {
        name,
        name_span,
        schema,
        provider_expr,
    })
}

/// Parse a query declaration: `query Name: Type = expr`
pub fn parse_query_decl(pair: Pair<Rule>) -> Result<QueryDecl> {
    let pair_loc = pair_location(&pair);
    let mut inner = pair.into_inner();

    // Parse name
    let name_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected query name".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let name_span = pair_span(&name_pair);
    let name = name_pair.as_str().to_string();

    // Parse type annotation (Query<T, Params>)
    let type_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected type annotation after ':'".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let type_ann = parse_type_annotation(type_pair)?;

    // Extract output_schema and params_schema from generic type
    // Expected: Query<OutputType, ParamsType>
    let (output_schema, params_schema) = match &type_ann {
        crate::ast::TypeAnnotation::Generic { args, .. } if args.len() == 2 => {
            (args[0].clone(), args[1].clone())
        }
        _ => {
            // Single type: treat as output schema with empty params
            (type_ann, crate::ast::TypeAnnotation::Object(vec![]))
        }
    };

    // Parse initializer expression (e.g., sql(DB, "SELECT ..."))
    let expr_pair = inner.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected expression after '='".to_string(),
        location: Some(pair_loc.clone()),
    })?;
    let init_expr = expressions::parse_expression(expr_pair)?;

    // Extract source_name and sql from the expression
    // For now, store them as defaults - the compiler will resolve these
    let (source_name, sql, sql_span) = extract_query_parts(&init_expr, &pair_loc)?;

    Ok(QueryDecl {
        name,
        name_span,
        output_schema,
        params_schema,
        source_name,
        sql,
        sql_span,
    })
}

/// Try to extract source name and SQL from a sql(Source, "...") expression.
/// Falls back to defaults if the expression doesn't match this pattern.
fn extract_query_parts(
    expr: &crate::ast::Expr,
    loc: &crate::error::SourceLocation,
) -> Result<(String, String, crate::ast::Span)> {
    // Try to match sql(Source, "SELECT ...")
    if let crate::ast::Expr::FunctionCall {
        name, args, span, ..
    } = expr
    {
        if name == "sql" && args.len() == 2 {
            let source = if let crate::ast::Expr::Identifier(src, _) = &args[0] {
                src.clone()
            } else {
                return Err(ShapeError::ParseError {
                    message: "expected data source identifier as first argument to sql()"
                        .to_string(),
                    location: Some(loc.clone()),
                });
            };
            let sql_str = if let crate::ast::Expr::Literal(crate::ast::Literal::String(s), _) =
                &args[1]
            {
                s.clone()
            } else {
                return Err(ShapeError::ParseError {
                    message: "expected SQL string literal as second argument to sql()".to_string(),
                    location: Some(loc.clone()),
                });
            };
            return Ok((source, sql_str, *span));
        }
    }

    // For non-sql expressions (e.g., provider functions), store empty defaults
    Ok((String::new(), String::new(), crate::ast::Span::new(0, 0)))
}
