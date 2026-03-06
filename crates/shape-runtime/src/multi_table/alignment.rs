//! Multi-table alignment and joining

use crate::data::OwnedDataRow as RowValue;
use shape_ast::ast::Timeframe;
use shape_ast::error::Result;
use std::collections::HashMap;

/// Aligns right rows to match the timestamps of left rows
pub fn align_tables(
    left_rows: &[RowValue],
    right_rows: &[RowValue],
    _timeframe: Timeframe,
) -> Result<Vec<RowValue>> {
    let mut right_by_ts: HashMap<i64, &RowValue> = HashMap::new();
    for row in right_rows {
        right_by_ts.insert(row.timestamp, row);
    }

    let mut result = Vec::with_capacity(left_rows.len());
    let mut last_valid: Option<&RowValue> = None;

    for left_row in left_rows {
        if let Some(&right_row) = right_by_ts.get(&left_row.timestamp) {
            result.push(right_row.clone());
            last_valid = Some(right_row);
        } else if let Some(prev) = last_valid {
            // Forward fill
            let mut filled = prev.clone();
            filled.timestamp = left_row.timestamp;
            result.push(filled);
        } else {
            // No previous data - use NaN placeholder
            result.push(RowValue::new_generic(left_row.timestamp, HashMap::new()));
        }
    }

    Ok(result)
}

/// Aligns multiple row sets to their intersection of timestamps
pub fn align_intersection(series_data: &[Vec<RowValue>]) -> Result<Vec<Vec<RowValue>>> {
    if series_data.is_empty() {
        return Ok(Vec::new());
    }

    // Find intersection of all timestamps
    let mut common_ts: std::collections::HashSet<i64> =
        series_data[0].iter().map(|r| r.timestamp).collect();
    for series in &series_data[1..] {
        let ts: std::collections::HashSet<i64> = series.iter().map(|r| r.timestamp).collect();
        common_ts = common_ts.intersection(&ts).cloned().collect();
    }

    let mut sorted_ts: Vec<i64> = common_ts.into_iter().collect();
    sorted_ts.sort_unstable();

    let mut result = Vec::with_capacity(series_data.len());
    for series in series_data {
        let mut series_by_ts: HashMap<i64, &RowValue> =
            series.iter().map(|r| (r.timestamp, r)).collect();
        let mut aligned = Vec::with_capacity(sorted_ts.len());
        for ts in &sorted_ts {
            aligned.push(series_by_ts.remove(ts).unwrap().clone());
        }
        result.push(aligned);
    }

    Ok(result)
}

/// Aligns multiple row sets to the union of all timestamps
pub fn align_union(series_data: &[Vec<RowValue>]) -> Result<Vec<Vec<RowValue>>> {
    if series_data.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_ts = std::collections::HashSet::new();
    for series in series_data {
        for row in series {
            all_ts.insert(row.timestamp);
        }
    }

    let mut sorted_ts: Vec<i64> = all_ts.into_iter().collect();
    sorted_ts.sort_unstable();

    let mut result = Vec::with_capacity(series_data.len());
    for series in series_data {
        let series_by_ts: HashMap<i64, &RowValue> =
            series.iter().map(|r| (r.timestamp, r)).collect();
        let mut aligned = Vec::with_capacity(sorted_ts.len());
        let mut last_valid: Option<RowValue> = None;

        for ts in &sorted_ts {
            if let Some(row) = series_by_ts.get(ts) {
                let r = (*row).clone();
                aligned.push(r.clone());
                last_valid = Some(r);
            } else if let Some(ref prev) = last_valid {
                let mut filled = prev.clone();
                filled.timestamp = *ts;
                aligned.push(filled);
            } else {
                aligned.push(RowValue::new_generic(*ts, HashMap::new()));
            }
        }
        result.push(aligned);
    }

    Ok(result)
}

/// Aligns one row set to a reference row set (left join)
pub fn align_left(left: &[RowValue], right: &[RowValue]) -> Result<Vec<RowValue>> {
    align_tables(left, right, Timeframe::default())
}

/// Joins two row sets by timestamp (inner join)
pub fn join_tables(left_rows: &[RowValue], right_rows: &[RowValue]) -> Result<Vec<RowValue>> {
    let mut right_by_ts: HashMap<i64, &RowValue> = HashMap::new();
    for row in right_rows {
        right_by_ts.insert(row.timestamp, row);
    }

    let mut result = Vec::new();

    for left_row in left_rows {
        if let Some(&right_row) = right_by_ts.get(&left_row.timestamp) {
            // Merge fields
            let mut fields = left_row.fields.clone();
            for (k, v) in &right_row.fields {
                fields.insert(k.clone(), *v);
            }
            result.push(RowValue::new_generic(left_row.timestamp, fields));
        }
    }

    Ok(result)
}
