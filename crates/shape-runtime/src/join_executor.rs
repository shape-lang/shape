//! JOIN execution engine
//!
//! Executes SQL-style JOINs between data sources:
//! - INNER JOIN: Only matching rows
//! - LEFT JOIN: All left rows, matching right rows (or nulls)
//! - RIGHT JOIN: Matching left rows (or nulls), all right rows
//! - FULL JOIN: All rows from both sides
//! - CROSS JOIN: Cartesian product
//! - TEMPORAL JOIN: Time-based matching within a tolerance window

use crate::context::ExecutionContext;
use shape_ast::ast::{JoinClause, JoinCondition, JoinType};
use shape_ast::error::Result;
use shape_value::ValueWord;
use std::collections::HashMap;

/// Execute JOINs between data sources
pub struct JoinExecutor;

impl JoinExecutor {
    /// Execute a join between left and right datasets
    pub fn execute(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
        join: &JoinClause,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        Self::execute_with_evaluator(left, right, join, None, ctx)
    }

    /// Execute a join with an optional expression evaluator for ON clause evaluation
    pub fn execute_with_evaluator(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
        join: &JoinClause,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        match join.join_type {
            JoinType::Inner => Self::inner_join(left, right, &join.condition, evaluator, ctx),
            JoinType::Left => Self::left_join(left, right, &join.condition, evaluator, ctx),
            JoinType::Right => Self::right_join(left, right, &join.condition, evaluator, ctx),
            JoinType::Full => Self::full_join(left, right, &join.condition, evaluator, ctx),
            JoinType::Cross => Self::cross_join(left, right),
        }
    }

    /// Execute INNER JOIN: only rows that match the condition
    fn inner_join(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
        condition: &JoinCondition,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        let mut results = Vec::new();

        for l_row in &left {
            for r_row in &right {
                if Self::matches_condition(l_row, r_row, condition, evaluator, ctx)? {
                    let merged = Self::merge_rows(l_row, r_row, "right");
                    results.push(merged);
                }
            }
        }

        Ok(results)
    }

    /// Execute LEFT JOIN: all left rows, with matching right rows or nulls
    fn left_join(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
        condition: &JoinCondition,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        let mut results = Vec::new();

        for l_row in &left {
            let mut matched = false;

            for r_row in &right {
                if Self::matches_condition(l_row, r_row, condition, evaluator, ctx)? {
                    let merged = Self::merge_rows(l_row, r_row, "right");
                    results.push(merged);
                    matched = true;
                }
            }

            // No match - include left row with nulls for right columns
            if !matched {
                let merged = Self::merge_with_nulls(l_row, &right, "right");
                results.push(merged);
            }
        }

        Ok(results)
    }

    /// Execute RIGHT JOIN: matching left rows or nulls, with all right rows
    fn right_join(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
        condition: &JoinCondition,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        let mut results = Vec::new();

        for r_row in &right {
            let mut matched = false;

            for l_row in &left {
                if Self::matches_condition(l_row, r_row, condition, evaluator, ctx)? {
                    let merged = Self::merge_rows(l_row, r_row, "right");
                    results.push(merged);
                    matched = true;
                }
            }

            // No match - include right row with nulls for left columns
            if !matched {
                let merged = Self::merge_with_nulls_left(&left, r_row, "right");
                results.push(merged);
            }
        }

        Ok(results)
    }

    /// Execute FULL JOIN: all rows from both sides
    fn full_join(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
        condition: &JoinCondition,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        let mut results = Vec::new();
        let mut right_matched = vec![false; right.len()];

        // Left outer join part
        for l_row in &left {
            let mut matched = false;

            for (r_idx, r_row) in right.iter().enumerate() {
                if Self::matches_condition(l_row, r_row, condition, evaluator, ctx)? {
                    let merged = Self::merge_rows(l_row, r_row, "right");
                    results.push(merged);
                    matched = true;
                    right_matched[r_idx] = true;
                }
            }

            if !matched {
                let merged = Self::merge_with_nulls(l_row, &right, "right");
                results.push(merged);
            }
        }

        // Add unmatched right rows
        for (r_idx, r_row) in right.iter().enumerate() {
            if !right_matched[r_idx] {
                let merged = Self::merge_with_nulls_left(&left, r_row, "right");
                results.push(merged);
            }
        }

        Ok(results)
    }

    /// Execute CROSS JOIN: Cartesian product
    fn cross_join(
        left: Vec<HashMap<String, ValueWord>>,
        right: Vec<HashMap<String, ValueWord>>,
    ) -> Result<Vec<HashMap<String, ValueWord>>> {
        let mut results = Vec::new();

        for l_row in &left {
            for r_row in &right {
                let merged = Self::merge_rows(l_row, r_row, "right");
                results.push(merged);
            }
        }

        Ok(results)
    }

    /// Check if two rows match the join condition
    fn matches_condition(
        left: &HashMap<String, ValueWord>,
        right: &HashMap<String, ValueWord>,
        condition: &JoinCondition,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<bool> {
        match condition {
            JoinCondition::On(expr) => {
                // Set up context with both row values
                ctx.push_scope();

                // Add left row values
                for (k, v) in left {
                    let _ = ctx.set_variable_nb(k, v.clone());
                }

                // Add right row values with prefix
                for (k, v) in right {
                    let _ = ctx.set_variable_nb(&format!("right.{}", k), v.clone());
                }

                let result = if let Some(eval) = evaluator {
                    // ExpressionEvaluator returns ValueWord, convert to ValueWord for inspection
                    let vm_result = eval
                        .eval_expr(expr, ctx)
                        .unwrap_or(ValueWord::from_bool(false));
                    vm_result
                } else {
                    ValueWord::from_bool(true) // Fallback: match all if no evaluator
                };
                ctx.pop_scope();

                if let Some(b) = result.as_bool() {
                    Ok(b)
                } else if let Some(n) = result.as_f64() {
                    Ok(n != 0.0 && !n.is_nan())
                } else {
                    Ok(false)
                }
            }

            JoinCondition::Using(columns) => {
                // Match on specified columns
                for col in columns {
                    let l_val = left.get(col);
                    let r_val = right.get(col);

                    match (l_val, r_val) {
                        (Some(a), Some(b)) if !nb_values_equal(a, b) => return Ok(false),
                        (None, None) => {} // Both null matches
                        (None, Some(_)) | (Some(_), None) => return Ok(false),
                        _ => {}
                    }
                }
                Ok(true)
            }

            JoinCondition::Temporal {
                left_time,
                right_time,
                within,
            } => {
                let l_ts = left.get(left_time).and_then(extract_timestamp_nb);
                let r_ts = right.get(right_time).and_then(extract_timestamp_nb);

                if let (Some(l), Some(r)) = (l_ts, r_ts) {
                    let diff_ms = (l - r).abs();
                    let threshold_ms = within.to_seconds() as f64 * 1000.0;
                    Ok(diff_ms <= threshold_ms)
                } else {
                    Ok(false)
                }
            }

            JoinCondition::Natural => {
                // Match on all common column names
                for (k, l_val) in left {
                    if let Some(r_val) = right.get(k) {
                        if !nb_values_equal(l_val, r_val) {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
        }
    }

    /// Merge two rows, prefixing right columns
    fn merge_rows(
        left: &HashMap<String, ValueWord>,
        right: &HashMap<String, ValueWord>,
        right_prefix: &str,
    ) -> HashMap<String, ValueWord> {
        let mut merged = left.clone();

        for (k, v) in right {
            merged.insert(format!("{}.{}", right_prefix, k), v.clone());
        }

        merged
    }

    /// Merge left row with null values for right columns
    fn merge_with_nulls(
        left: &HashMap<String, ValueWord>,
        right_sample: &[HashMap<String, ValueWord>],
        right_prefix: &str,
    ) -> HashMap<String, ValueWord> {
        let mut merged = left.clone();

        // Get column names from first right row (if any)
        if let Some(first_right) = right_sample.first() {
            for k in first_right.keys() {
                merged.insert(format!("{}.{}", right_prefix, k), ValueWord::none());
            }
        }

        merged
    }

    /// Merge null values for left columns with right row
    fn merge_with_nulls_left(
        left_sample: &[HashMap<String, ValueWord>],
        right: &HashMap<String, ValueWord>,
        right_prefix: &str,
    ) -> HashMap<String, ValueWord> {
        let mut merged = HashMap::new();

        // Get column names from first left row (if any)
        if let Some(first_left) = left_sample.first() {
            for k in first_left.keys() {
                merged.insert(k.clone(), ValueWord::none());
            }
        }

        // Add right columns
        for (k, v) in right {
            merged.insert(format!("{}.{}", right_prefix, k), v.clone());
        }

        merged
    }
}

/// Check if two ValueWord values are equal
fn nb_values_equal(a: &ValueWord, b: &ValueWord) -> bool {
    use shape_value::NanTag;
    match (a.tag(), b.tag()) {
        (NanTag::F64, NanTag::F64)
        | (NanTag::I48, NanTag::I48)
        | (NanTag::F64, NanTag::I48)
        | (NanTag::I48, NanTag::F64) => {
            if let (Some(an), Some(bn)) = (a.as_f64(), b.as_f64()) {
                if an.is_nan() && bn.is_nan() {
                    true
                } else {
                    (an - bn).abs() < f64::EPSILON
                }
            } else {
                false
            }
        }
        (NanTag::Heap, NanTag::Heap) => {
            if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
                sa == sb
            } else {
                false
            }
        }
        (NanTag::Bool, NanTag::Bool) => a.as_bool() == b.as_bool(),
        (NanTag::None, NanTag::None) => true,
        _ => {
            // For Time values, fall back
            if let (Some(ta), Some(tb)) = (a.as_time(), b.as_time()) {
                ta == tb
            } else {
                false
            }
        }
    }
}

/// Extract timestamp as milliseconds from a ValueWord value
fn extract_timestamp_nb(v: &ValueWord) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        Some(n)
    } else if let Some(t) = v.as_time() {
        Some(t.timestamp_millis() as f64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionContext;
    use shape_ast::ast::JoinSource;

    fn make_rows(data: Vec<Vec<(&str, ValueWord)>>) -> Vec<HashMap<String, ValueWord>> {
        data.into_iter()
            .map(|row| row.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
            .collect()
    }

    #[test]
    fn test_inner_join_using() {
        let mut ctx = ExecutionContext::new_empty();

        let left = make_rows(vec![
            vec![
                ("id", ValueWord::from_f64(1.0)),
                (
                    "name",
                    ValueWord::from_string(std::sync::Arc::new("A".to_string())),
                ),
            ],
            vec![
                ("id", ValueWord::from_f64(2.0)),
                (
                    "name",
                    ValueWord::from_string(std::sync::Arc::new("B".to_string())),
                ),
            ],
            vec![
                ("id", ValueWord::from_f64(3.0)),
                (
                    "name",
                    ValueWord::from_string(std::sync::Arc::new("C".to_string())),
                ),
            ],
        ]);

        let right = make_rows(vec![
            vec![
                ("id", ValueWord::from_f64(1.0)),
                ("value", ValueWord::from_f64(100.0)),
            ],
            vec![
                ("id", ValueWord::from_f64(3.0)),
                ("value", ValueWord::from_f64(300.0)),
            ],
        ]);

        let join = JoinClause {
            join_type: JoinType::Inner,
            right: JoinSource::Named("test".to_string()),
            condition: JoinCondition::Using(vec!["id".to_string()]),
        };

        let result = JoinExecutor::execute(left, right, &join, &mut ctx).unwrap();

        // Should have 2 matching rows (id 1 and 3)
        assert_eq!(result.len(), 2);

        // Check first match
        assert_eq!(result[0].get("id").map(|v| v.as_f64()), Some(Some(1.0)));
        assert_eq!(result[0].get("name").and_then(|v| v.as_str()), Some("A"));
        assert_eq!(
            result[0].get("right.value").map(|v| v.as_f64()),
            Some(Some(100.0))
        );
    }

    #[test]
    fn test_left_join() {
        let mut ctx = ExecutionContext::new_empty();

        let left = make_rows(vec![
            vec![("id", ValueWord::from_f64(1.0))],
            vec![("id", ValueWord::from_f64(2.0))],
        ]);

        let right = make_rows(vec![vec![
            ("id", ValueWord::from_f64(1.0)),
            ("val", ValueWord::from_f64(10.0)),
        ]]);

        let join = JoinClause {
            join_type: JoinType::Left,
            right: JoinSource::Named("test".to_string()),
            condition: JoinCondition::Using(vec!["id".to_string()]),
        };

        let result = JoinExecutor::execute(left, right, &join, &mut ctx).unwrap();

        // Should have 2 rows (all left rows)
        assert_eq!(result.len(), 2);

        // First row has match
        assert_eq!(
            result[0].get("right.val").map(|v| v.as_f64()),
            Some(Some(10.0))
        );

        // Second row has null
        assert!(
            result[1]
                .get("right.val")
                .map(|v| v.is_none())
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_cross_join() {
        let left = make_rows(vec![
            vec![("a", ValueWord::from_f64(1.0))],
            vec![("a", ValueWord::from_f64(2.0))],
        ]);

        let right = make_rows(vec![
            vec![("b", ValueWord::from_f64(10.0))],
            vec![("b", ValueWord::from_f64(20.0))],
        ]);

        let result = JoinExecutor::cross_join(left, right).unwrap();

        // Should have 2 * 2 = 4 rows
        assert_eq!(result.len(), 4);
    }
}
