//! DataTable simulation method: simulate.

use crate::executor::VirtualMachine;
use crate::executor::objects::object_creation::nb_to_slot_with_field_type;
use shape_runtime::type_schema::FieldType;
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::sync::Arc;

use super::common::{
    extract_dt_nb, extract_schema_id_nb, typed_object_entries_nb_vm, typed_object_to_hashmap_nb_vm,
};

/// Check if a ValueWord value is callable (Function, ModuleFunction, Closure, HostClosure).
fn is_callable_nb(nb: &ValueWord) -> bool {
    match nb.tag() {
        NanTag::Function | NanTag::ModuleFunction => true,
        NanTag::Heap => matches!(
            nb.heap_kind(),
            Some(HeapKind::Closure | HeapKind::HostClosure)
        ),
        _ => false,
    }
}

/// `dt.simulate(handler, config?)` — unified simulation method on DataTable.
///
/// Single mode: handler(row, state, idx) where row is RowView.
/// Correlated mode: if config.tables present, handler(ctx, state, idx) where ctx has named RowViews.
///
/// Handler result: if Object with "state" key -> extract state + optional "result"; else treat as new state.
/// Returns: { final_state, results, elements_processed, completed }
pub(crate) fn handle_simulate(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let schema_id = extract_schema_id_nb(&args[0]);

    // Require handler function
    let handler_nb = args.get(1).filter(|nb| is_callable_nb(nb)).ok_or_else(|| {
        VMError::RuntimeError(
            "simulate() requires a handler function as first argument".to_string(),
        )
    })?;

    // Parse optional config as ValueWord object.
    let config = args
        .get(2)
        .map(|nb| typed_object_to_hashmap_nb_vm(vm, nb))
        .transpose()?;

    let initial_state = config
        .as_ref()
        .and_then(|c| c.get("initial_state").cloned())
        .unwrap_or_else(ValueWord::none);

    let collect_results = config
        .as_ref()
        .and_then(|c| c.get("collect_results"))
        .map(|v| v.is_truthy())
        .unwrap_or(true);

    let collect_event_log = config
        .as_ref()
        .and_then(|c| c.get("collect_event_log"))
        .map(|v| v.is_truthy())
        .unwrap_or(false);

    let seed = config.as_ref().and_then(|c| c.get("seed").cloned());

    // Check for correlated tables
    let correlated_tables: Option<Vec<(String, ValueWord)>> = config
        .as_ref()
        .and_then(|c| c.get("tables"))
        .and_then(|v| typed_object_entries_nb_vm(vm, v).ok());

    // Correlated context schema must be predeclared at compile time.
    let ctx_schema_id = if let Some(ref tables) = correlated_tables {
        let mut ctx_field_names: Vec<&str> = vec!["row"];
        for (name, _) in tables.iter() {
            ctx_field_names.push(name.as_str());
        }
        let cache_name = format!("__sim_ctx_{}", ctx_field_names[1..].join(","));
        if let Some(schema) = vm.lookup_schema_by_name(&cache_name) {
            Some(schema.id)
        } else {
            return Err(VMError::RuntimeError(format!(
                "Missing predeclared correlated simulation schema '{}' (runtime schema generation is disabled).",
                cache_name
            )));
        }
    } else {
        None
    };

    let dt_arc = Arc::new(dt.as_ref().clone());
    let row_count = dt_arc.row_count();
    let mut state = initial_state;
    let mut results: Vec<ValueWord> = if collect_results {
        Vec::with_capacity(row_count)
    } else {
        Vec::new()
    };
    let mut event_log: Vec<ValueWord> = if collect_event_log {
        Vec::new()
    } else {
        Vec::new()
    };
    let mut handler_args: Vec<ValueWord> = Vec::with_capacity(3);

    for row_idx in 0..row_count {
        handler_args.clear();
        if let Some(ref tables) = correlated_tables {
            // Correlated mode: build ctx TypedObject with row + named RowViews
            let mut ctx_values: Vec<ValueWord> = Vec::with_capacity(1 + tables.len());
            ctx_values.push(ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx));

            // Add correlated tables -- use min(row_idx, len-1) to avoid out-of-bounds
            for (_, table_val) in tables.iter() {
                if let Ok(other_dt) = extract_dt_nb(table_val) {
                    let other_schema_id = extract_schema_id_nb(table_val);
                    let other_idx = row_idx.min(other_dt.row_count().saturating_sub(1));
                    ctx_values.push(ValueWord::from_row_view(
                        other_schema_id,
                        other_dt.clone(),
                        other_idx,
                    ));
                } else {
                    ctx_values.push(table_val.clone());
                }
            }

            // Build context TypedObject directly using pre-registered schema
            let sid = ctx_schema_id.unwrap();
            let mut slots = Vec::with_capacity(ctx_values.len());
            let mut heap_mask: u64 = 0;
            for (i, val) in ctx_values.into_iter().enumerate() {
                let (slot, is_heap) = nb_to_slot_with_field_type(&val, Some(&FieldType::Any));
                slots.push(slot);
                if is_heap {
                    heap_mask |= 1u64 << i;
                }
            }
            handler_args.push(ValueWord::from_heap_value(
                shape_value::heap_value::HeapValue::TypedObject {
                    schema_id: sid as u64,
                    slots: slots.into_boxed_slice(),
                    heap_mask,
                },
            ));
            handler_args.push(state.clone());
            handler_args.push(ValueWord::from_i64(row_idx as i64));
        } else {
            // Single mode: handler(row, state, idx)
            let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
            handler_args.push(row_view);
            handler_args.push(state.clone());
            handler_args.push(ValueWord::from_i64(row_idx as i64));
        };

        let result = vm.call_value_immediate_nb(handler_nb, &handler_args, ctx.as_deref_mut())?;

        // Interpret result: if object with "state" key -> extract state + optional "result"/"event_type"
        let result_map = typed_object_to_hashmap_nb_vm(vm, &result)
            .ok()
            .filter(|obj| obj.contains_key("state"));
        if let Some(obj) = result_map {
            state = obj.get("state").cloned().unwrap_or_else(ValueWord::none);
            if collect_results {
                if let Some(r) = obj.get("result") {
                    results.push(r.clone());
                }
            }
            if collect_event_log {
                if let Some(event_type) = obj.get("event_type") {
                    let entry_schema = vm.builtin_schemas.event_log_entry;
                    let result_val = obj.get("result").cloned().unwrap_or_else(ValueWord::none);
                    let row_idx_nb = ValueWord::from_i64(row_idx as i64);
                    let (slot0, heap0) =
                        nb_to_slot_with_field_type(&row_idx_nb, Some(&FieldType::I64));
                    let (slot1, heap1) =
                        nb_to_slot_with_field_type(event_type, Some(&FieldType::String));
                    let (slot2, heap2) =
                        nb_to_slot_with_field_type(&result_val, Some(&FieldType::Any));
                    let mut heap_mask = 0u64;
                    if heap0 {
                        heap_mask |= 1 << 0;
                    }
                    if heap1 {
                        heap_mask |= 1 << 1;
                    }
                    if heap2 {
                        heap_mask |= 1 << 2;
                    }
                    event_log.push(ValueWord::from_heap_value(
                        shape_value::heap_value::HeapValue::TypedObject {
                            schema_id: entry_schema as u64,
                            slots: vec![slot0, slot1, slot2].into_boxed_slice(),
                            heap_mask,
                        },
                    ));
                }
            }
            continue;
        }
        // Otherwise treat entire result as new state
        state = result;
    }

    // Build return object using builtin SimulateReturn schema (6 slots)
    let sim_schema = vm.builtin_schemas.simulate_return;
    let event_log_val = if collect_event_log {
        ValueWord::from_array(Arc::new(event_log))
    } else {
        ValueWord::none()
    };
    let seed_val = seed.unwrap_or_else(ValueWord::none);
    let results_val = ValueWord::from_array(Arc::new(results));
    let processed_val = ValueWord::from_i64(row_count as i64);
    let completed_val = ValueWord::from_bool(true);

    let values = vec![
        state,
        results_val,
        processed_val,
        completed_val,
        event_log_val,
        seed_val,
    ];
    let field_types = [
        FieldType::Any,
        FieldType::Any,
        FieldType::I64,
        FieldType::Bool,
        FieldType::Any,
        FieldType::Any,
    ];
    let mut slots = Vec::with_capacity(values.len());
    let mut heap_mask = 0u64;
    for (i, value) in values.iter().enumerate() {
        let (slot, is_heap) = nb_to_slot_with_field_type(value, Some(&field_types[i]));
        slots.push(slot);
        if is_heap {
            heap_mask |= 1u64 << i;
        }
    }

    vm.push_vw(ValueWord::from_heap_value(
        shape_value::heap_value::HeapValue::TypedObject {
            schema_id: sim_schema as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        },
    ))
}
