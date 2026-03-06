//! Query plan and SQL generation for data source pushdown.
//!
//! Provides a standalone query plan data structure and SQL generator.
//! Used by DbTable and other Queryable implementations to build queries
//! that can be pushed down to the data source.

use shape_value::{FilterLiteral, FilterNode, FilterOp};

/// A query plan that accumulates operations for SQL generation.
#[derive(Debug, Clone, Default)]
pub struct QueryPlan {
    /// Table name to query
    pub table: String,
    /// Pushdown-able filter predicates
    pub filters: Vec<FilterNode>,
    /// Column projections (None = SELECT *)
    pub projections: Option<Vec<String>>,
    /// ORDER BY clauses: (column, descending)
    pub order_by: Vec<(String, bool)>,
    /// GROUP BY columns
    pub group_by: Vec<String>,
    /// LIMIT
    pub limit: Option<usize>,
    /// OFFSET
    pub offset: Option<usize>,
}

impl QueryPlan {
    /// Create a new query plan for a table
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            ..Default::default()
        }
    }

    /// Generate a SQL query string from this plan
    pub fn to_sql(&self) -> String {
        let mut sql = String::new();

        // SELECT
        sql.push_str("SELECT ");
        match &self.projections {
            Some(cols) if !cols.is_empty() => {
                sql.push_str(
                    &cols
                        .iter()
                        .map(|c| quote_ident(c))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            _ => sql.push('*'),
        }

        // FROM
        sql.push_str(" FROM ");
        sql.push_str(&quote_ident(&self.table));

        // WHERE
        if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            let filter_clauses: Vec<String> =
                self.filters.iter().map(|f| filter_to_sql(f)).collect();
            sql.push_str(&filter_clauses.join(" AND "));
        }

        // GROUP BY
        if !self.group_by.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(
                &self
                    .group_by
                    .iter()
                    .map(|c| quote_ident(c))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }

        // ORDER BY
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let order_clauses: Vec<String> = self
                .order_by
                .iter()
                .map(|(col, desc)| {
                    if *desc {
                        format!("{} DESC", quote_ident(col))
                    } else {
                        quote_ident(col)
                    }
                })
                .collect();
            sql.push_str(&order_clauses.join(", "));
        }

        // LIMIT
        if let Some(limit) = self.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        // OFFSET
        if let Some(offset) = self.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        sql
    }
}

/// Generate SQL for a FilterNode
pub fn filter_to_sql(node: &FilterNode) -> String {
    match node {
        FilterNode::Compare { column, op, value } => {
            let op_str = match op {
                FilterOp::Eq => "=",
                FilterOp::Neq => "!=",
                FilterOp::Gt => ">",
                FilterOp::Gte => ">=",
                FilterOp::Lt => "<",
                FilterOp::Lte => "<=",
            };
            format!(
                "{} {} {}",
                quote_ident(column),
                op_str,
                literal_to_sql(value)
            )
        }
        FilterNode::And(left, right) => {
            format!("({} AND {})", filter_to_sql(left), filter_to_sql(right))
        }
        FilterNode::Or(left, right) => {
            format!("({} OR {})", filter_to_sql(left), filter_to_sql(right))
        }
        FilterNode::Not(inner) => {
            format!("NOT ({})", filter_to_sql(inner))
        }
    }
}

/// Generate SQL for a FilterLiteral
pub fn literal_to_sql(lit: &FilterLiteral) -> String {
    match lit {
        FilterLiteral::Int(i) => i.to_string(),
        FilterLiteral::Float(f) => format!("{}", f),
        FilterLiteral::String(s) => format!("'{}'", s.replace('\'', "''")),
        FilterLiteral::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        FilterLiteral::Null => "NULL".to_string(),
    }
}

/// Quote a SQL identifier (simple quoting for safety)
fn quote_ident(name: &str) -> String {
    // Don't quote simple identifiers
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.is_empty() {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_select() {
        let plan = QueryPlan::new("users");
        assert_eq!(plan.to_sql(), "SELECT * FROM users");
    }

    #[test]
    fn test_select_with_projections() {
        let mut plan = QueryPlan::new("users");
        plan.projections = Some(vec!["name".to_string(), "age".to_string()]);
        assert_eq!(plan.to_sql(), "SELECT name, age FROM users");
    }

    #[test]
    fn test_where_clause() {
        let mut plan = QueryPlan::new("users");
        plan.filters.push(FilterNode::Compare {
            column: "age".to_string(),
            op: FilterOp::Gte,
            value: FilterLiteral::Int(18),
        });
        assert_eq!(plan.to_sql(), "SELECT * FROM users WHERE age >= 18");
    }

    #[test]
    fn test_compound_filters() {
        let mut plan = QueryPlan::new("users");
        plan.filters.push(FilterNode::And(
            Box::new(FilterNode::Compare {
                column: "age".to_string(),
                op: FilterOp::Gte,
                value: FilterLiteral::Int(18),
            }),
            Box::new(FilterNode::Compare {
                column: "active".to_string(),
                op: FilterOp::Eq,
                value: FilterLiteral::Bool(true),
            }),
        ));
        assert_eq!(
            plan.to_sql(),
            "SELECT * FROM users WHERE (age >= 18 AND active = TRUE)"
        );
    }

    #[test]
    fn test_or_filter() {
        let filter = FilterNode::Or(
            Box::new(FilterNode::Compare {
                column: "role".to_string(),
                op: FilterOp::Eq,
                value: FilterLiteral::String("admin".to_string()),
            }),
            Box::new(FilterNode::Compare {
                column: "role".to_string(),
                op: FilterOp::Eq,
                value: FilterLiteral::String("moderator".to_string()),
            }),
        );
        assert_eq!(
            filter_to_sql(&filter),
            "(role = 'admin' OR role = 'moderator')"
        );
    }

    #[test]
    fn test_not_filter() {
        let filter = FilterNode::Not(Box::new(FilterNode::Compare {
            column: "deleted".to_string(),
            op: FilterOp::Eq,
            value: FilterLiteral::Bool(true),
        }));
        assert_eq!(filter_to_sql(&filter), "NOT (deleted = TRUE)");
    }

    #[test]
    fn test_order_by() {
        let mut plan = QueryPlan::new("users");
        plan.order_by.push(("age".to_string(), true));
        assert_eq!(plan.to_sql(), "SELECT * FROM users ORDER BY age DESC");
    }

    #[test]
    fn test_order_by_asc() {
        let mut plan = QueryPlan::new("users");
        plan.order_by.push(("name".to_string(), false));
        assert_eq!(plan.to_sql(), "SELECT * FROM users ORDER BY name");
    }

    #[test]
    fn test_limit_offset() {
        let mut plan = QueryPlan::new("users");
        plan.limit = Some(100);
        plan.offset = Some(50);
        assert_eq!(plan.to_sql(), "SELECT * FROM users LIMIT 100 OFFSET 50");
    }

    #[test]
    fn test_full_query() {
        let mut plan = QueryPlan::new("users");
        plan.projections = Some(vec!["name".to_string(), "age".to_string()]);
        plan.filters.push(FilterNode::Compare {
            column: "age".to_string(),
            op: FilterOp::Gte,
            value: FilterLiteral::Int(18),
        });
        plan.filters.push(FilterNode::Compare {
            column: "active".to_string(),
            op: FilterOp::Eq,
            value: FilterLiteral::Bool(true),
        });
        plan.order_by.push(("age".to_string(), true));
        plan.limit = Some(100);
        assert_eq!(
            plan.to_sql(),
            "SELECT name, age FROM users WHERE age >= 18 AND active = TRUE ORDER BY age DESC LIMIT 100"
        );
    }

    #[test]
    fn test_string_literal_escaping() {
        let lit = FilterLiteral::String("O'Brien".to_string());
        assert_eq!(literal_to_sql(&lit), "'O''Brien'");
    }

    #[test]
    fn test_null_comparison() {
        let filter = FilterNode::Compare {
            column: "email".to_string(),
            op: FilterOp::Eq,
            value: FilterLiteral::Null,
        };
        assert_eq!(filter_to_sql(&filter), "email = NULL");
    }

    #[test]
    fn test_group_by() {
        let mut plan = QueryPlan::new("orders");
        plan.group_by.push("status".to_string());
        assert_eq!(plan.to_sql(), "SELECT * FROM orders GROUP BY status");
    }
}
