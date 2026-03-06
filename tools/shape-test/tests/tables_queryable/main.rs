//! Integration tests for table operations and LINQ-style queryable syntax.
//!
//! Covers:
//! - Array-based from..in..select queries (LINQ desugaring)
//! - Query clauses: where, order by, group by, join, let
//! - Queryable trait registration and dispatch

mod queryable;
mod table_methods;
