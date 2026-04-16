use shape_runtime::module_exports::ModuleContext;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

// ===========================================================================
// Capture / introspection implementations (live VM access via ctx.vm_state)
// ===========================================================================

/// Convert an optional `[u8; 32]` blob hash to a hex string ValueWord, or `ValueWord::none()`.
fn blob_hash_to_nb(hash: Option<[u8; 32]>) -> ValueWord {
    match hash {
        Some(bytes) => {
            let hex = bytes.iter().fold(String::with_capacity(64), |mut acc, b| {
                use std::fmt::Write;
                let _ = write!(acc, "{:02x}", b);
                acc
            });
            ValueWord::from_string(Arc::new(hex))
        }
        None => ValueWord::none(),
    }
}

/// Build a FrameState-like typed object from a `FrameInfo`.
fn frame_info_to_nb(frame: &shape_runtime::module_exports::FrameInfo) -> ValueWord {
    let function_name = ValueWord::from_string(Arc::new(frame.function_name.clone()));
    let blob_hash = blob_hash_to_nb(frame.blob_hash);
    let ip = ValueWord::from_i64(frame.local_ip as i64);
    let locals = ValueWord::from_array(Arc::new(frame.locals.clone()));
    let args = ValueWord::from_array(Arc::new(frame.args.clone()));
    let upvalues = match &frame.upvalues {
        Some(vals) => ValueWord::from_array(Arc::new(vals.clone())),
        None => ValueWord::none(),
    };

    shape_runtime::type_schema::typed_object_from_pairs(&[
        ("function_name", function_name),
        ("blob_hash", blob_hash),
        ("ip", ip),
        ("locals", locals),
        ("args", args),
        ("upvalues", upvalues),
    ])
}

/// `state.capture() -> FrameState`
///
/// Capture the current function's frame state (name, hash, ip, locals, args).
pub(crate) fn state_capture_stub(
    _args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let vm_state = ctx
        .vm_state
        .ok_or("state.capture: VM state not available")?;
    let frame = vm_state
        .current_frame()
        .ok_or("state.capture: no current frame")?;
    Ok(frame_info_to_nb(&frame))
}

/// `state.capture_all() -> VmState`
///
/// Capture the full VM call stack, module bindings, and instruction count.
pub(crate) fn state_capture_all_stub(
    _args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let vm_state = ctx
        .vm_state
        .ok_or("state.capture_all: VM state not available")?;
    let frames = vm_state.all_frames();
    let frame_nbs: Vec<ValueWord> = frames.iter().map(frame_info_to_nb).collect();
    let frames_arr = ValueWord::from_array(Arc::new(frame_nbs));

    let bindings = vm_state.module_bindings();
    let pairs: Vec<ValueWord> = bindings
        .into_iter()
        .map(|(name, value)| {
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new(name)),
                value,
            ]))
        })
        .collect();
    let bindings_arr = ValueWord::from_array(Arc::new(pairs));
    let ic = ValueWord::from_i64(vm_state.instruction_count() as i64);

    Ok(shape_runtime::type_schema::typed_object_from_pairs(&[
        ("frames", frames_arr),
        ("module_bindings", bindings_arr),
        ("instruction_count", ic),
    ]))
}

/// `state.capture_module() -> ModuleState`
///
/// Capture module-level bindings as an array of `[name, value]` pairs.
pub(crate) fn state_capture_module_stub(
    _args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let vm_state = ctx
        .vm_state
        .ok_or("state.capture_module: VM state not available")?;
    let bindings = vm_state.module_bindings();
    let pairs: Vec<ValueWord> = bindings
        .into_iter()
        .map(|(name, value)| {
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new(name)),
                value,
            ]))
        })
        .collect();
    let bindings_arr = ValueWord::from_array(Arc::new(pairs));

    Ok(shape_runtime::type_schema::typed_object_from_pairs(&[(
        "bindings",
        bindings_arr,
    )]))
}

/// `state.capture_call(f, args) -> CallPayload`
///
/// Build a ready-to-call payload: extract the function's content hash and
/// package the arguments for later replay.
pub(crate) fn state_capture_call_stub(
    args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let f = args
        .first()
        .ok_or("state.capture_call: requires function argument")?;
    let call_args = args
        .get(1)
        .and_then(|nb| nb.as_any_array().map(|v| v.to_generic()))
        .ok_or("state.capture_call: second argument must be an array of args")?;

    // Resolve function hash via fn_hash logic
    let func_id = if let Some(fid) = f.as_function_id() {
        Some(fid as usize)
    } else if let Some((fid, _)) = crate::executor::objects::raw_helpers::extract_closure_info(f.raw_bits()) {
        Some(fid as usize)
    } else {
        None
    };

    let hash_nb = if let Some(fid) = func_id {
        if let Some(hashes) = ctx.function_hashes {
            if let Some(Some(hash_bytes)) = hashes.get(fid) {
                let hex = hash_bytes
                    .iter()
                    .fold(String::with_capacity(64), |mut acc, b| {
                        use std::fmt::Write;
                        let _ = write!(acc, "{:02x}", b);
                        acc
                    });
                ValueWord::from_string(Arc::new(hex))
            } else {
                ValueWord::from_string(Arc::new(format!("fn:{}", fid)))
            }
        } else {
            ValueWord::from_string(Arc::new(format!("fn:{}", fid)))
        }
    } else {
        return Err("state.capture_call: first argument is not a function".to_string());
    };

    let args_arr = ValueWord::from_array(Arc::new(call_args.to_vec()));

    Ok(shape_runtime::type_schema::typed_object_from_pairs(&[
        ("hash", hash_nb),
        ("args", args_arr),
    ]))
}

/// `state.resume(snapshot) -> !`
///
/// Resume full VM state from a snapshot captured by `state.capture_all()`.
///
/// Stores the snapshot via the `set_pending_resume` callback on `ModuleContext`.
/// The dispatch loop intercepts the `ResumeRequested` signal and applies the
/// state restoration (replacing call_stack, ip, sp, locals, module_bindings).
pub(crate) fn state_resume_stub(
    args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let snapshot = args
        .first()
        .ok_or("state.resume: requires a VmState snapshot argument")?;

    let set_resume = ctx
        .set_pending_resume
        .ok_or("state.resume: resume callback not available in this context")?;

    // Store the snapshot — the dispatch loop will apply it after this returns.
    set_resume(snapshot.clone());

    // Return value is irrelevant; the dispatch loop will intercept ResumeRequested
    // before this result is used.
    Ok(ValueWord::none())
}

/// `state.resume_frame(frame_state) -> any`
///
/// Re-enter a captured function frame by looking up the function (first by
/// content hash, then by name) and invoking it with the captured arguments.
///
/// When the FrameState contains a non-zero `ip` and captured `locals`, the
/// function is resumed mid-execution: invoke sets up the call frame, then
/// `pending_frame_resume` overrides IP and locals so execution continues
/// from the captured point rather than restarting from the beginning.
pub(crate) fn state_resume_frame_stub(
    args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let frame_nb = args
        .first()
        .ok_or("state.resume_frame: requires a FrameState argument")?;

    // Extract args from the FrameState typed object
    // FrameState fields (v1): function_name, blob_hash, ip, locals, args
    // FrameState fields (v2): function_name, blob_hash, ip, locals, args, upvalues
    let invoke = ctx
        .invoke_callable
        .ok_or("state.resume_frame: callable invoker not available in this context")?;

    if let Some((_schema_id, slots, heap_mask)) = frame_nb.as_typed_object() {
        // We need at least 5 slots (function_name, blob_hash, ip, locals, args)
        if slots.len() < 5 {
            return Err("state.resume_frame: invalid FrameState (not enough fields)".to_string());
        }

        // slot 4 = args (array of captured arguments)
        let captured_args = if heap_mask & (1 << 4) != 0 {
            let args_nb = slots[4].as_heap_nb();
            args_nb
                .as_any_array()
                .map(|v| v.to_generic().to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // slot 2 = ip (captured instruction pointer offset within the function)
        let captured_ip = slots[2].as_i64() as usize;

        // slot 3 = locals (array of captured local variables)
        let captured_locals: Vec<ValueWord> = if heap_mask & (1 << 3) != 0 {
            let locals_nb = slots[3].as_heap_nb();
            locals_nb
                .as_any_array()
                .map(|v| v.to_generic().to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // slot 0 = function_name (string)
        let func_name_nb = if heap_mask & 1 != 0 {
            slots[0].as_heap_nb()
        } else {
            return Err(
                "state.resume_frame: cannot extract function_name from FrameState".to_string(),
            );
        };

        // Try hash-based lookup first via ctx.function_hashes
        let blob_hash_nb = if heap_mask & (1 << 1) != 0 {
            Some(slots[1].as_heap_nb())
        } else {
            None
        };

        // Helper: schedule mid-function resume BEFORE invoke so the dispatch loop
        // applies it to the new frame (not the caller's frame which would already
        // be popped after invoke returns).
        let invoke_and_resume =
            |fn_nb: &ValueWord, call_args: &[ValueWord]| -> Result<ValueWord, String> {
                if captured_ip > 0 {
                    if let Some(set_frame_resume) = ctx.set_pending_frame_resume {
                        set_frame_resume(captured_ip, captured_locals.clone());
                    }
                }

                let result = invoke(fn_nb, call_args)?;
                Ok(result)
            };

        if let Some(hash_nb) = blob_hash_nb {
            if let Some(hash_str) = hash_nb.as_str() {
                if let Some(hashes) = ctx.function_hashes {
                    if let Ok(bytes) = hex::decode(hash_str.as_bytes()) {
                        if bytes.len() == 32 {
                            let mut target = [0u8; 32];
                            target.copy_from_slice(&bytes);
                            // Find function index matching this hash
                            for (idx, maybe_hash) in hashes.iter().enumerate() {
                                if *maybe_hash == Some(target) {
                                    // Found matching function by hash — invoke by index
                                    let fn_nb = ValueWord::from_function(idx as u16);
                                    return invoke_and_resume(&fn_nb, &captured_args).map_err(|e| {
                                        format!(
                                            "state.resume_frame: hash-based invocation failed: {}",
                                            e
                                        )
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fall back to name-based invocation
        invoke_and_resume(&func_name_nb, &captured_args).map_err(|e| {
            format!(
                "state.resume_frame: invocation failed (function '{}' not found by hash or name): {}",
                func_name_nb
                    .as_str()
                    .unwrap_or(&Arc::new("<unknown>".to_string())),
                e
            )
        })
    } else {
        Err("state.resume_frame: argument must be a FrameState TypedObject".to_string())
    }
}

/// `state.caller() -> FunctionRef?`
///
/// Get a reference to the calling function (one frame up the call stack).
/// Returns none if there is no caller.
pub(crate) fn state_caller_stub(
    _args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let vm_state = ctx.vm_state.ok_or("state.caller: VM state not available")?;
    match vm_state.caller_frame() {
        Some(frame) => {
            let name = ValueWord::from_string(Arc::new(frame.function_name.clone()));
            let hash = blob_hash_to_nb(frame.blob_hash);

            Ok(shape_runtime::type_schema::typed_object_from_pairs(&[
                ("name", name),
                ("hash", hash),
            ]))
        }
        None => Ok(ValueWord::none()),
    }
}

/// `state.args() -> Array<any>`
///
/// Get the current function's arguments as an array.
pub(crate) fn state_args_stub(
    _args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let vm_state = ctx.vm_state.ok_or("state.args: VM state not available")?;
    let current_args = vm_state.current_args();
    Ok(ValueWord::from_array(Arc::new(current_args)))
}

/// `state.locals() -> Map<string, any>`
///
/// Get the current scope's local variables as an array of `[name, value]` pairs.
pub(crate) fn state_locals_stub(
    _args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let vm_state = ctx.vm_state.ok_or("state.locals: VM state not available")?;
    let locals = vm_state.current_locals();
    let pairs: Vec<ValueWord> = locals
        .into_iter()
        .map(|(name, value)| {
            ValueWord::from_array(Arc::new(vec![
                ValueWord::from_string(Arc::new(name)),
                value,
            ]))
        })
        .collect();
    Ok(ValueWord::from_array(Arc::new(pairs)))
}

// ===========================================================================
// Tests
// ===========================================================================
