//! Built-in functions for multi-table analysis

use super::alignment::{align_intersection, align_union};
use super::config::AlignmentMode;
use crate::context::ExecutionContext;
use crate::data::OwnedDataRow as RowValue;
use crate::data::Timeframe;
use crate::timeframe_utils::parse_timeframe_string;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;
use std::sync::Arc;

fn parse_dataset_id(id: &str, default_timeframe: Timeframe) -> Result<(String, Timeframe)> {
    if let Some((base_id, tf_str)) = id.rsplit_once('_') {
        if let Ok(tf) = parse_timeframe_string(tf_str) {
            return Ok((base_id.to_string(), tf));
        }
    }
    Ok((id.to_string(), default_timeframe))
}

fn load_rows(_ctx: &ExecutionContext, _id: &str, _timeframe: Timeframe) -> Result<Vec<RowValue>> {
    Err(ShapeError::RuntimeError {
        message: "Data access requires prefetching data first via execute_async()".to_string(),
        location: None,
    })
}

/// Align multiple datasets
pub fn align_tables(ctx: &mut ExecutionContext, args: &[ValueWord]) -> Result<ValueWord> {
    if args.is_empty() || args.len() > 2 {
        return Err(ShapeError::RuntimeError {
            message: "align_tables() requires 1-2 arguments: ids, [mode]".into(),
            location: None,
        });
    }

    let dataset_ids = match args[0].as_any_array() {
        Some(view) => {
            let arr = view.to_generic();
            arr.iter()
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        Ok(Arc::new(s.to_string()))
                    } else {
                        Err(ShapeError::RuntimeError {
                            message: "IDs must be strings".into(),
                            location: None,
                        })
                    }
                })
                .collect::<Result<Vec<_>>>()?
        }
        None => {
            return Err(ShapeError::RuntimeError {
                message: "First argument must be an array of IDs".into(),
                location: None,
            });
        }
    };

    let mode = if args.len() > 1 {
        match args[1].as_str() {
            Some("intersection") => AlignmentMode::Intersection,
            Some("union") => AlignmentMode::Union,
            Some(s) => {
                return Err(ShapeError::RuntimeError {
                    message: format!("Unknown alignment mode: {}", s),
                    location: None,
                });
            }
            None => AlignmentMode::Intersection,
        }
    } else {
        AlignmentMode::Intersection
    };

    let default_tf = ctx.get_current_timeframe().unwrap_or_default();
    let mut datasets = Vec::with_capacity(dataset_ids.len());
    for id in &dataset_ids {
        let (base_id, timeframe) = parse_dataset_id(id, default_tf)?;
        let rows = load_rows(ctx, &base_id, timeframe)?;
        datasets.push(rows);
    }

    let aligned = match mode {
        AlignmentMode::Intersection => align_intersection(&datasets)?,
        AlignmentMode::Union => align_union(&datasets)?,
        _ => {
            return Err(ShapeError::RuntimeError {
                message: "align_tables supports only intersection or union modes".to_string(),
                location: None,
            });
        }
    };

    let ids_val = ValueWord::from_array(Arc::new(
        dataset_ids
            .iter()
            .map(|s| ValueWord::from_string(s.clone()))
            .collect(),
    ));

    // Convert aligned data to ValueWord
    let mut aligned_data_val: Vec<ValueWord> = Vec::new();
    for rows in aligned {
        let rows_val: Vec<ValueWord> = rows
            .into_iter()
            .map(|r| {
                let pairs: Vec<(&str, ValueWord)> = r
                    .fields
                    .iter()
                    .map(|(k, v)| (k.as_str(), ValueWord::from_f64(*v)))
                    .collect();
                crate::type_schema::typed_object_from_nb_pairs(&pairs)
            })
            .collect();
        aligned_data_val.push(ValueWord::from_array(Arc::new(rows_val)));
    }

    Ok(crate::type_schema::typed_object_from_nb_pairs(&[
        ("ids", ids_val),
        ("data", ValueWord::from_array(Arc::new(aligned_data_val))),
    ]))
}

pub fn correlation(_ctx: &mut ExecutionContext, args: &[ValueWord]) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "correlation() requires 2 series arguments".into(),
            location: None,
        });
    }

    // Placeholder
    Ok(ValueWord::from_f64(0.0))
}

pub fn find_divergences(_ctx: &mut ExecutionContext, _args: &[ValueWord]) -> Result<ValueWord> {
    Err(ShapeError::RuntimeError {
        message: "find_divergences() not implemented".into(),
        location: None,
    })
}

pub fn spread(_ctx: &mut ExecutionContext, _args: &[ValueWord]) -> Result<ValueWord> {
    Err(ShapeError::RuntimeError {
        message: "spread() not implemented".into(),
        location: None,
    })
}

pub fn temporal_join(_ctx: &mut ExecutionContext, _args: &[ValueWord]) -> Result<ValueWord> {
    Err(ShapeError::RuntimeError {
        message: "temporal_join() not implemented".into(),
        location: None,
    })
}
