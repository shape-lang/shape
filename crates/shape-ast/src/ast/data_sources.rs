//! Data source and query declaration types for Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::span::Span;
use super::types::TypeAnnotation;

/// Data source declaration: `datasource MarketData: DataSource<CandleRow> = provider("market_data")`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceDecl {
    pub name: String,
    pub name_span: Span,
    /// Output schema type (e.g., CandleRow)
    pub schema: TypeAnnotation,
    /// Provider expression (e.g., provider("postgres"))
    pub provider_expr: Expr,
}

/// Query declaration: `query UserById: Query<UserRow, { id: i64 }> = sql(DB, "SELECT ...")`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryDecl {
    pub name: String,
    pub name_span: Span,
    /// Output schema type (e.g., UserRow)
    pub output_schema: TypeAnnotation,
    /// Runtime parameter schema (object type)
    pub params_schema: TypeAnnotation,
    /// Data source reference (identifier)
    pub source_name: String,
    /// SQL string literal
    pub sql: String,
    pub sql_span: Span,
}
