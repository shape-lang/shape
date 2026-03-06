//! Window function execution engine
//!
//! Executes SQL-style window functions over datasets:
//! - Ranking functions: ROW_NUMBER, RANK, DENSE_RANK, NTILE
//! - Navigation functions: LAG, LEAD, FIRST_VALUE, LAST_VALUE, NTH_VALUE
//! - Aggregate functions: SUM, AVG, MIN, MAX, COUNT over window frames

use crate::context::ExecutionContext;
use shape_ast::ast::{Expr, SortDirection, WindowBound, WindowExpr, WindowFrame, WindowFunction};
use shape_ast::error::Result;
use shape_value::ValueWord;
use std::collections::HashMap;

/// Execute window functions over a dataset
pub struct WindowExecutor {
    /// Partitioned row data
    partitions: HashMap<Vec<OrderedValue>, Vec<RowData>>,
}

/// A row of data with its original index for result placement
struct RowData {
    /// Original row index in the input dataset
    original_index: usize,
    /// Row values by field name
    values: HashMap<String, ValueWord>,
}

/// Wrapper for ValueWord that implements Eq + Hash for partition keys
#[derive(Clone, Debug)]
struct OrderedValue(ValueWord);

impl PartialEq for OrderedValue {
    fn eq(&self, other: &Self) -> bool {
        use shape_value::NanTag;
        match (self.0.tag(), other.0.tag()) {
            (NanTag::F64, NanTag::F64)
            | (NanTag::I48, NanTag::I48)
            | (NanTag::F64, NanTag::I48)
            | (NanTag::I48, NanTag::F64) => match (self.0.as_f64(), other.0.as_f64()) {
                (Some(a), Some(b)) => {
                    if a.is_nan() && b.is_nan() {
                        true
                    } else {
                        a == b
                    }
                }
                _ => false,
            },
            (NanTag::Heap, NanTag::Heap) => {
                if let (Some(a), Some(b)) = (self.0.as_str(), other.0.as_str()) {
                    a == b
                } else {
                    false
                }
            }
            (NanTag::Bool, NanTag::Bool) => self.0.as_bool() == other.0.as_bool(),
            (NanTag::None, NanTag::None) => true,
            _ => {
                if let (Some(a), Some(b)) = (self.0.as_time(), other.0.as_time()) {
                    a == b
                } else {
                    false
                }
            }
        }
    }
}

impl Eq for OrderedValue {}

impl std::hash::Hash for OrderedValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use shape_value::NanTag;
        match self.0.tag() {
            NanTag::F64 | NanTag::I48 => {
                state.write_u8(0);
                if let Some(n) = self.0.as_f64() {
                    state.write_u64(n.to_bits());
                }
            }
            NanTag::Heap => {
                if let Some(s) = self.0.as_str() {
                    state.write_u8(1);
                    s.hash(state);
                } else {
                    state.write_u8(255);
                }
            }
            NanTag::Bool => {
                state.write_u8(2);
                if let Some(b) = self.0.as_bool() {
                    b.hash(state);
                }
            }
            NanTag::None => {
                state.write_u8(4);
            }
            _ => {
                if let Some(t) = self.0.as_time() {
                    state.write_u8(3);
                    t.timestamp_nanos_opt().unwrap_or(0).hash(state);
                } else {
                    state.write_u8(255);
                }
            }
        }
    }
}

impl WindowExecutor {
    /// Create a new window executor
    pub fn new() -> Self {
        Self {
            partitions: HashMap::new(),
        }
    }

    /// Execute a window function over rows
    pub fn execute(
        &mut self,
        rows: &[HashMap<String, ValueWord>],
        window_expr: &WindowExpr,
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<ValueWord>> {
        self.partitions.clear();

        // 1. Partition rows
        self.partition_rows(rows, &window_expr.over.partition_by, evaluator, ctx)?;

        // 2. Sort each partition
        if let Some(ref order_by) = window_expr.over.order_by {
            self.sort_partitions(order_by)?;
        }

        // 3. Apply window function
        let mut results = vec![ValueWord::none(); rows.len()];

        for partition in self.partitions.values() {
            for (pos, row) in partition.iter().enumerate() {
                let value = evaluate_window_function(
                    &window_expr.function,
                    partition,
                    pos,
                    &window_expr.over.frame,
                    evaluator,
                    ctx,
                )?;
                results[row.original_index] = value;
            }
        }

        Ok(results)
    }

    fn partition_rows(
        &mut self,
        rows: &[HashMap<String, ValueWord>],
        partition_by: &[Expr],
        evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
        ctx: &mut ExecutionContext,
    ) -> Result<()> {
        if partition_by.is_empty() {
            // Single partition with all rows
            let all_rows: Vec<_> = rows
                .iter()
                .enumerate()
                .map(|(idx, row)| RowData {
                    original_index: idx,
                    values: row.clone(),
                })
                .collect();
            self.partitions.insert(vec![], all_rows);
            return Ok(());
        }

        for (idx, row) in rows.iter().enumerate() {
            ctx.push_scope();
            for (key, value) in row {
                let _ = ctx.set_variable_nb(key, value.clone());
            }

            let mut key = Vec::with_capacity(partition_by.len());
            for expr in partition_by {
                let value = if let Some(eval) = evaluator {
                    eval.eval_expr(expr, ctx).unwrap_or(ValueWord::none())
                } else {
                    ValueWord::none()
                };
                key.push(OrderedValue(value));
            }

            ctx.pop_scope();

            self.partitions.entry(key).or_default().push(RowData {
                original_index: idx,
                values: row.clone(),
            });
        }

        Ok(())
    }

    fn sort_partitions(&mut self, order_by: &shape_ast::ast::OrderByClause) -> Result<()> {
        for partition in self.partitions.values_mut() {
            partition.sort_by(|a, b| {
                for (expr, direction) in &order_by.columns {
                    let a_val = extract_sort_value(&a.values, expr);
                    let b_val = extract_sort_value(&b.values, expr);

                    let cmp = compare_nb_values(&a_val, &b_val);
                    let cmp = match direction {
                        SortDirection::Ascending => cmp,
                        SortDirection::Descending => cmp.reverse(),
                    };

                    if cmp != std::cmp::Ordering::Equal {
                        return cmp;
                    }
                }
                std::cmp::Ordering::Equal
            });
        }
        Ok(())
    }
}

impl Default for WindowExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate window function for a specific row
fn evaluate_window_function(
    func: &WindowFunction,
    partition: &[RowData],
    current_idx: usize,
    frame: &Option<WindowFrame>,
    evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
    ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    match func {
        WindowFunction::RowNumber => Ok(ValueWord::from_f64((current_idx + 1) as f64)),

        WindowFunction::Rank => {
            let rank = calculate_rank(partition, current_idx, false);
            Ok(ValueWord::from_f64(rank as f64))
        }

        WindowFunction::DenseRank => {
            let rank = calculate_rank(partition, current_idx, true);
            Ok(ValueWord::from_f64(rank as f64))
        }

        WindowFunction::Ntile(n) => {
            let bucket = if partition.is_empty() {
                1
            } else {
                (current_idx * *n / partition.len()) + 1
            };
            Ok(ValueWord::from_f64(bucket as f64))
        }

        WindowFunction::Lag {
            expr,
            offset,
            default,
        } => {
            if let Some(target_idx) = current_idx.checked_sub(*offset) {
                if target_idx < partition.len() {
                    return eval_expr_at(expr, &partition[target_idx], evaluator, ctx);
                }
            }
            if let Some(def) = default {
                if let Some(eval) = evaluator {
                    Ok(eval.eval_expr(def, ctx)?)
                } else {
                    Ok(ValueWord::none())
                }
            } else {
                Ok(ValueWord::none())
            }
        }

        WindowFunction::Lead {
            expr,
            offset,
            default,
        } => {
            let target_idx = current_idx + *offset;
            if target_idx < partition.len() {
                return eval_expr_at(expr, &partition[target_idx], evaluator, ctx);
            }
            if let Some(def) = default {
                if let Some(eval) = evaluator {
                    Ok(eval.eval_expr(def, ctx)?)
                } else {
                    Ok(ValueWord::none())
                }
            } else {
                Ok(ValueWord::none())
            }
        }

        WindowFunction::FirstValue(expr) => {
            let (start, _) = get_frame_bounds(frame, partition.len(), current_idx);
            eval_expr_at(expr, &partition[start], evaluator, ctx)
        }

        WindowFunction::LastValue(expr) => {
            let (_, end) = get_frame_bounds(frame, partition.len(), current_idx);
            eval_expr_at(expr, &partition[end], evaluator, ctx)
        }

        WindowFunction::NthValue(expr, n) => {
            let (start, end) = get_frame_bounds(frame, partition.len(), current_idx);
            let target_idx = start + n - 1;
            if target_idx <= end && target_idx < partition.len() {
                eval_expr_at(expr, &partition[target_idx], evaluator, ctx)
            } else {
                Ok(ValueWord::none())
            }
        }

        WindowFunction::Sum(expr)
        | WindowFunction::Avg(expr)
        | WindowFunction::Min(expr)
        | WindowFunction::Max(expr) => {
            let (start, end) = get_frame_bounds(frame, partition.len(), current_idx);
            let mut values = Vec::new();

            for i in start..=end.min(partition.len().saturating_sub(1)) {
                let nb = eval_expr_at(expr, &partition[i], evaluator, ctx)?;
                if let Some(n) = nb.as_f64() {
                    values.push(n);
                }
            }

            if values.is_empty() {
                return Ok(ValueWord::none());
            }

            let result = match func {
                WindowFunction::Sum(_) => values.iter().sum::<f64>(),
                WindowFunction::Avg(_) => values.iter().sum::<f64>() / values.len() as f64,
                WindowFunction::Min(_) => values
                    .iter()
                    .cloned()
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or(f64::NAN),
                WindowFunction::Max(_) => values
                    .iter()
                    .cloned()
                    .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or(f64::NAN),
                _ => unreachable!(),
            };

            Ok(ValueWord::from_f64(result))
        }

        WindowFunction::Count(expr_opt) => {
            let (start, end) = get_frame_bounds(frame, partition.len(), current_idx);

            let count = if let Some(expr) = expr_opt {
                (start..=end.min(partition.len().saturating_sub(1)))
                    .filter(|&i| {
                        eval_expr_at(expr, &partition[i], evaluator, ctx)
                            .map(|v| !v.is_none())
                            .unwrap_or(false)
                    })
                    .count()
            } else {
                end.min(partition.len().saturating_sub(1))
                    .saturating_sub(start)
                    + 1
            };

            Ok(ValueWord::from_f64(count as f64))
        }
    }
}

/// Evaluate expression with row context
fn eval_expr_at(
    expr: &Expr,
    row: &RowData,
    evaluator: Option<&dyn crate::engine::ExpressionEvaluator>,
    ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    ctx.push_scope();
    for (key, value) in &row.values {
        let _ = ctx.set_variable_nb(key, value.clone());
    }
    let result = if let Some(eval) = evaluator {
        Ok(eval.eval_expr(expr, ctx)?)
    } else {
        // Fallback: try simple identifier lookup
        if let Expr::Identifier(name, _) = expr {
            Ok(row.values.get(name).cloned().unwrap_or(ValueWord::none()))
        } else {
            Ok(ValueWord::none())
        }
    };
    ctx.pop_scope();
    result
}

/// Calculate rank within partition
fn calculate_rank(_partition: &[RowData], current_idx: usize, dense: bool) -> usize {
    if current_idx == 0 {
        return 1;
    }
    // Simplified: each row gets sequential rank
    // Full implementation would compare ORDER BY values
    if dense {
        current_idx + 1
    } else {
        current_idx + 1
    }
}

/// Get frame bounds for aggregate functions
fn get_frame_bounds(
    frame: &Option<WindowFrame>,
    partition_len: usize,
    current_idx: usize,
) -> (usize, usize) {
    match frame {
        Some(f) => {
            let start = match &f.start {
                WindowBound::UnboundedPreceding => 0,
                WindowBound::CurrentRow => current_idx,
                WindowBound::Preceding(n) => current_idx.saturating_sub(*n),
                WindowBound::Following(n) => (current_idx + n).min(partition_len.saturating_sub(1)),
                WindowBound::UnboundedFollowing => partition_len.saturating_sub(1),
            };
            let end = match &f.end {
                WindowBound::UnboundedPreceding => 0,
                WindowBound::CurrentRow => current_idx,
                WindowBound::Preceding(n) => current_idx.saturating_sub(*n),
                WindowBound::Following(n) => (current_idx + n).min(partition_len.saturating_sub(1)),
                WindowBound::UnboundedFollowing => partition_len.saturating_sub(1),
            };
            (start, end)
        }
        None => (0, current_idx),
    }
}

/// Extract sort value from expression
fn extract_sort_value(row: &HashMap<String, ValueWord>, expr: &Expr) -> ValueWord {
    if let Expr::Identifier(name, _) = expr {
        return row.get(name).cloned().unwrap_or(ValueWord::none());
    }
    ValueWord::none()
}

/// Compare two ValueWord values for sorting
fn compare_nb_values(a: &ValueWord, b: &ValueWord) -> std::cmp::Ordering {
    use shape_value::NanTag;
    match (a.tag(), b.tag()) {
        (NanTag::F64, NanTag::F64)
        | (NanTag::I48, NanTag::I48)
        | (NanTag::F64, NanTag::I48)
        | (NanTag::I48, NanTag::F64) => match (a.as_f64(), b.as_f64()) {
            (Some(an), Some(bn)) => an.partial_cmp(&bn).unwrap_or(std::cmp::Ordering::Equal),
            _ => std::cmp::Ordering::Equal,
        },
        (NanTag::Heap, NanTag::Heap) => match (a.as_str(), b.as_str()) {
            (Some(sa), Some(sb)) => sa.cmp(sb),
            _ => std::cmp::Ordering::Equal,
        },
        (NanTag::Bool, NanTag::Bool) => match (a.as_bool(), b.as_bool()) {
            (Some(ba), Some(bb)) => ba.cmp(&bb),
            _ => std::cmp::Ordering::Equal,
        },
        (NanTag::None, NanTag::None) => std::cmp::Ordering::Equal,
        (NanTag::None, _) => std::cmp::Ordering::Less,
        (_, NanTag::None) => std::cmp::Ordering::Greater,
        _ => match (a.as_time(), b.as_time()) {
            (Some(ta), Some(tb)) => ta.cmp(&tb),
            _ => std::cmp::Ordering::Equal,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rows(data: Vec<Vec<(&str, ValueWord)>>) -> Vec<HashMap<String, ValueWord>> {
        data.into_iter()
            .map(|row| row.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
            .collect()
    }

    #[test]
    fn test_row_number_simple() {
        let mut ctx = ExecutionContext::new_empty();
        let mut executor = WindowExecutor::new();

        let rows = make_rows(vec![
            vec![("x", ValueWord::from_f64(1.0))],
            vec![("x", ValueWord::from_f64(2.0))],
            vec![("x", ValueWord::from_f64(3.0))],
        ]);

        let window_expr = WindowExpr {
            function: WindowFunction::RowNumber,
            over: shape_ast::ast::WindowSpec {
                partition_by: vec![],
                order_by: None,
                frame: None,
            },
        };

        let results = executor
            .execute(&rows, &window_expr, None, &mut ctx)
            .unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].as_f64(), Some(1.0));
        assert_eq!(results[1].as_f64(), Some(2.0));
        assert_eq!(results[2].as_f64(), Some(3.0));
    }
}
