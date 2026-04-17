//! Exception handling operations for the VM executor
//!
//! Handles: SetupTry, PopHandler, Throw, TryUnwrap, UnwrapOption, ErrorContext,
//! Result helpers (IsOk/IsErr/UnwrapOk/UnwrapErr), TypeCheck

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{ExceptionHandler, VirtualMachine},
};
use shape_ast::TypeAnnotation;
use shape_runtime::type_schema::builtin_schemas::*;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueSlot, ValueWord, ValueWordExt};
use std::sync::Arc;

use crate::executor::objects::raw_helpers;

// =========================================================================
// Slot-based field access helpers for error/trace TypedObjects
// =========================================================================

/// Convert a ValueWord value to a (ValueSlot, is_heap) pair for TypedObject storage.
/// Stores raw ValueWord tag bits for inline types (lossless round-trip via
/// `ValueSlot::as_value_word`). Heap types are stored as heap-allocated HeapValue pointers.
fn nb_to_slot(nb: &ValueWord) -> (ValueSlot, bool) {
    ValueSlot::from_value_word(nb)
}

/// Read a string field from a TypedObject's slots directly.
fn typed_string_field_from_slots(
    slots: &[ValueSlot],
    heap_mask: u64,
    slot_index: usize,
) -> Option<String> {
    if slot_index >= slots.len() {
        return None;
    }
    if heap_mask & (1u64 << slot_index) != 0 {
        let nb = slots[slot_index].as_heap_nb();
        nb.as_str().map(|s| s.to_string())
    } else {
        None
    }
}

/// Read an i64 from a TypedObject slot.
///
/// Handles both heap slots (HeapValue with BigInt/number) and non-heap slots.
/// Non-heap slots may contain either:
/// - Raw f64 bits (from ValueSlot::from_number)
/// - Raw ValueWord bits (from ValueSlot::from_raw via nb_to_slot)
/// Both cases are handled by checking if the raw bits are ValueWord-tagged.
fn slot_as_i64(slots: &[ValueSlot], heap_mask: u64, slot_index: usize) -> Option<i64> {
    if slot_index >= slots.len() {
        return None;
    }
    if heap_mask & (1u64 << slot_index) != 0 {
        let nb = slots[slot_index].as_heap_nb();
        nb.as_i64().or_else(|| nb.as_f64().map(|n| n as i64))
    } else {
        let raw = slots[slot_index].raw();
        // Check if this is a ValueWord-tagged value (I48 integer stored via nb_to_slot)
        if shape_value::tags::is_tagged(raw) {
            let tag = shape_value::tags::get_tag(raw);
            if tag == shape_value::tags::TAG_INT {
                // I48: sign-extend the 48-bit payload
                return Some(shape_value::tags::sign_extend_i48(
                    shape_value::tags::get_payload(raw),
                ));
            }
            // Other tagged types (Bool, None, etc.) are not integers
            return None;
        }
        // Raw f64 bits (from ValueSlot::from_number)
        let n = slots[slot_index].as_f64();
        if n.is_finite() && n.fract() == 0.0 {
            Some(n as i64)
        } else {
            None
        }
    }
}

impl VirtualMachine {
    // ===== Helper Methods =====

    /// Handle an exception by unwinding to the nearest handler
    pub(in crate::executor) fn handle_exception_nb(
        &mut self,
        error: ValueWord,
    ) -> Result<(), VMError> {
        if let Some(handler) = self.exception_handlers.pop() {
            self.clear_last_uncaught_exception();
            // Unwind stack to handler's saved state (sp-based)
            for i in handler.stack_size..self.sp {
                drop(ValueWord::from_raw_bits(self.stack[i]));
                self.stack[i] = Self::NONE_BITS;
            }
            self.sp = handler.stack_size;
            self.call_stack.truncate(handler.call_depth);

            // Push error value for catch block
            self.push_raw_u64(error)?;

            // Jump to catch handler
            self.ip = handler.catch_ip;
            Ok(())
        } else {
            let host_error = if Self::is_any_error_nb(&error) {
                error.clone()
            } else {
                // Preserve current user-facing message formatting, but normalize host payload
                // to AnyError so adapters (ANSI/HTML/plain) can render structured diagnostics.
                let trace = self.trace_info_full_nb();
                self.build_any_error_nb(error.clone(), None, trace, None)
            };
            self.set_last_uncaught_exception(host_error);
            // No handler, propagate as runtime error — ValueWord-native path
            Err(VMError::RuntimeError(
                self.format_uncaught_exception_nb(&error),
            ))
        }
    }

    // ===== Opcode Implementations =====

    #[inline(always)]
    pub(in crate::executor) fn exec_exceptions(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            TypeCheck => self.op_type_check(instruction)?,
            SetupTry => self.op_setup_try(instruction)?,
            PopHandler => self.op_pop_handler()?,
            Throw => self.op_throw()?,
            TryUnwrap => self.op_try_unwrap()?,
            UnwrapOption => self.op_unwrap_option()?,
            ErrorContext => self.op_error_context()?,
            IsOk => self.op_is_ok()?,
            IsErr => self.op_is_err()?,
            UnwrapOk => self.op_unwrap_ok()?,
            UnwrapErr => self.op_unwrap_err()?,
            _ => unreachable!(
                "exec_exceptions called with non-exception opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    pub(in crate::executor) fn op_type_check(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let value_nb = self.pop_raw_u64()?;
        let type_annotation = match instruction.operand {
            Some(Operand::Const(idx)) => match self.program.constants.get(idx as usize) {
                Some(crate::bytecode::Constant::TypeAnnotation(annotation)) => annotation.clone(),
                _ => {
                    return Err(VMError::RuntimeError(
                        "TypeCheck expects type annotation constant".to_string(),
                    ));
                }
            },
            _ => return Err(VMError::InvalidOperand),
        };

        let result = self.check_instanceof_nb(&value_nb, &type_annotation);
        self.push_raw_u64(ValueWord::from_bool(result))?;
        Ok(())
    }

    /// ValueWord-native instanceof check — no ValueWord materialization needed.
    pub(in crate::executor) fn check_instanceof_nb(
        &self,
        value: &ValueWord,
        type_annotation: &TypeAnnotation,
    ) -> bool {
        use shape_value::heap_value::HeapKind;

        // Unwrap TypeAnnotatedValue wrappers (e.g. from `as int | string`)
        // so type checks see the underlying value, not the wrapper.
        if let Some(HeapKind::TypeAnnotatedValue) = value.heap_kind() {
            let inner_bits = raw_helpers::unwrap_annotated_bits(*value);
            return self.check_instanceof_nb(&inner_bits, type_annotation);
        }

        let as_int = |nb: &ValueWord| -> Option<i64> {
            if let Some(i) = nb.as_i64() {
                return Some(i);
            }
            let n = nb.as_number_coerce()?;
            if n.fract().abs() > f64::EPSILON || n < i64::MIN as f64 || n > i64::MAX as f64 {
                return None;
            }
            Some(n as i64)
        };

        match type_annotation {
            TypeAnnotation::Basic(type_name) => match type_name.as_str() {
                "number" => {
                    value.is_f64()
                        || matches!(value.heap_kind(), Some(HeapKind::Decimal))
                }
                "f32" | "f64" | "float" => value.is_f64(),
                "int" => {
                    value.is_i64()
                        || matches!(value.heap_kind(), Some(HeapKind::BigInt))
                }
                "i64" | "i32" | "i16" | "isize" | "u32" | "u64" | "usize" | "integer" => {
                    as_int(value).is_some()
                }
                "i8" => {
                    as_int(value).is_some_and(|v| (i8::MIN as i64..=i8::MAX as i64).contains(&v))
                }
                "char" => value.as_char().is_some(),
                "u8" | "byte" => as_int(value).is_some_and(|v| (0..=u8::MAX as i64).contains(&v)),
                "u16" => as_int(value).is_some_and(|v| (0..=u16::MAX as i64).contains(&v)),
                "string" => value.as_str().is_some(),
                "boolean" | "bool" => value.is_bool(),
                "null" => value.is_none(),
                "array" => value.as_any_array().is_some(),
                "object" => matches!(value.heap_kind(), Some(HeapKind::TypedObject)),
                "function" => {
                    (value.is_function() || value.is_module_function())
                        || matches!(
                            value.heap_kind(),
                            Some(HeapKind::Closure | HeapKind::HostClosure)
                        )
                }
                "row" => false,
                "series" => false,
                "time" => matches!(value.heap_kind(), Some(HeapKind::Time)),
                "duration" => matches!(value.heap_kind(), Some(HeapKind::Duration)),
                "timeframe" => matches!(value.heap_kind(), Some(HeapKind::Timeframe)),
                "range" => matches!(value.heap_kind(), Some(HeapKind::Range)),
                _ => false,
            },
            TypeAnnotation::Array(inner_type) => {
                if let Some(view) = value.as_any_array() {
                    let arr = view.to_generic();
                    arr.iter()
                        .all(|item| self.check_instanceof_nb(item, inner_type))
                } else {
                    false
                }
            }
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                value.is_none() || self.check_instanceof_nb(value, &args[0])
            }
            TypeAnnotation::Union(types) => types
                .iter()
                .any(|inner| self.check_instanceof_nb(value, inner)),
            TypeAnnotation::Intersection(types) => types
                .iter()
                .all(|inner| self.check_instanceof_nb(value, inner)),
            TypeAnnotation::Void => value.is_unit(),
            TypeAnnotation::Never => false,
            TypeAnnotation::Tuple(types) => {
                if let Some(view) = value.as_any_array() {
                    let arr = view.to_generic();
                    arr.len() == types.len()
                        && arr
                            .iter()
                            .zip(types.iter())
                            .all(|(item, ty)| self.check_instanceof_nb(item, ty))
                } else {
                    false
                }
            }
            TypeAnnotation::Object(_) => matches!(value.heap_kind(), Some(HeapKind::TypedObject)),
            TypeAnnotation::Function { .. } => {
                (value.is_function() || value.is_module_function())
                    || matches!(
                        value.heap_kind(),
                        Some(HeapKind::Closure | HeapKind::HostClosure)
                    )
            }
            TypeAnnotation::Generic { name, args: _ } => match name.as_str() {
                "Vec" => value.as_any_array().is_some(),
                _ => false,
            },
            TypeAnnotation::Reference(name) => match name.as_str() {
                "BacktestResult" => false,
                _ => false,
            },
            TypeAnnotation::Null => value.is_none(),
            TypeAnnotation::Undefined => value.is_none() || value.is_unit(),
            TypeAnnotation::Dyn(_) => true,
        }
    }

    pub(in crate::executor) fn op_setup_try(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let catch_ip = (self.ip as i32 + offset) as usize;
            self.exception_handlers.push(ExceptionHandler {
                catch_ip,
                stack_size: self.sp,
                call_depth: self.call_stack.len(),
            });
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    pub(in crate::executor) fn op_pop_handler(&mut self) -> Result<(), VMError> {
        self.exception_handlers.pop();
        Ok(())
    }

    pub(in crate::executor) fn op_throw(&mut self) -> Result<(), VMError> {
        let error_nb = self.pop_raw_u64()?;
        self.handle_exception_nb(error_nb)
    }

    fn current_instruction_ip(&self) -> usize {
        self.ip.saturating_sub(1)
    }

    fn function_name_for_id(&self, function_id: Option<u16>) -> Option<String> {
        function_id
            .and_then(|id| self.program.functions.get(id as usize))
            .map(|func| func.name.clone())
    }

    fn current_function_name(&self) -> Option<String> {
        self.function_name_for_id(self.call_stack.last().and_then(|frame| frame.function_id))
    }

    // ===== Construction: use builtin schema IDs + direct slot construction =====

    fn build_trace_frame_nb(&mut self, ip: usize, function_name: Option<String>) -> ValueWord {
        let schema_id = self.builtin_schemas.trace_frame;

        let mut slots = Vec::with_capacity(4);
        let mut heap_mask: u64 = 0;

        // Slot 0: ip (number, inline)
        slots.push(ValueSlot::from_number(ip as f64));

        if let Some((file_id, line)) = self.program.debug_info.get_location_for_instruction(ip) {
            // Slot 1: line (number, inline)
            slots.push(ValueSlot::from_number(line as f64));

            // Slot 2: file (string or none)
            if let Some(f) = self.program.debug_info.source_map.get_file(file_id) {
                slots.push(ValueSlot::from_heap(HeapValue::String(Arc::new(
                    f.to_string(),
                ))));
                heap_mask |= 1u64 << 2;
            } else {
                slots.push(ValueSlot::none());
            }
        } else {
            // Slot 1: line (none)
            slots.push(ValueSlot::none());
            // Slot 2: file (none)
            slots.push(ValueSlot::none());
        }

        // Slot 3: function name (string or none)
        if let Some(n) = function_name {
            slots.push(ValueSlot::from_heap(HeapValue::String(Arc::new(n))));
            heap_mask |= 1u64 << 3;
        } else {
            slots.push(ValueSlot::none());
        }

        ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        })
    }

    pub(in crate::executor) fn trace_info_full_nb(&mut self) -> ValueWord {
        let schema_id = self.builtin_schemas.trace_info_full;
        let ip = self.current_instruction_ip();
        let fn_name = self.current_function_name();
        let mut frames: Vec<ValueWord> = Vec::new();
        frames.push(self.build_trace_frame_nb(ip, fn_name));

        let call_stack_data: Vec<_> = self
            .call_stack
            .iter()
            .rev()
            .filter(|f| f.return_ip != 0)
            .map(|f| (f.return_ip.saturating_sub(1), f.function_id))
            .collect();

        for (call_site_ip, function_id) in call_stack_data {
            let name = self.function_name_for_id(function_id);
            frames.push(self.build_trace_frame_nb(call_site_ip, name));
        }

        // Slots: [kind, frames] — both heap
        let slots = vec![
            ValueSlot::from_heap(HeapValue::String(Arc::new("full".to_string()))),
            ValueSlot::from_heap(HeapValue::Array(shape_value::vmarray_from_vec(frames))),
        ];
        let heap_mask: u64 = 0b11;

        ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        })
    }

    pub(in crate::executor) fn trace_info_single_nb(&mut self) -> ValueWord {
        let schema_id = self.builtin_schemas.trace_info_single;
        let ip = self.current_instruction_ip();
        let fn_name = self.current_function_name();
        let frame_nb = self.build_trace_frame_nb(ip, fn_name);

        // Slots: [kind, frame]
        let mut heap_mask: u64 = 0b01; // slot 0 (kind) is always heap
        let frame_slot = if let Some(hv) = unsafe { raw_helpers::extract_heap_ref(frame_nb) } {
            heap_mask |= 1u64 << 1;
            ValueSlot::from_heap(hv.clone())
        } else {
            ValueSlot::none()
        };
        let slots = vec![
            ValueSlot::from_heap(HeapValue::String(Arc::new("single".to_string()))),
            frame_slot,
        ];

        ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        })
    }

    fn error_message_from_nb(payload: &ValueWord) -> String {
        if let Some(s) = payload.as_str() {
            return s.to_string();
        }
        if let Some(i) = payload.as_i64() {
            return i.to_string();
        }
        if let Some(n) = payload.as_f64() {
            return n.to_string();
        }
        if let Some(b) = payload.as_bool() {
            return b.to_string();
        }
        if payload.is_none() {
            return "Value was None".to_string();
        }
        if let Some(d) = payload.as_decimal() {
            return d.to_string();
        }
        format!("<{}>", payload.type_name())
    }

    fn is_any_error_nb(value: &ValueWord) -> bool {
        if let Some((_schema_id, slots, heap_mask)) = raw_helpers::extract_typed_object(*value) {
            if slots.is_empty() || heap_mask & 1 == 0 {
                return false;
            }
            let nb = slots[ANYERROR_CATEGORY].as_heap_nb();
            nb.as_str() == Some("AnyError")
        } else {
            false
        }
    }

    pub(in crate::executor) fn build_any_error_nb(
        &mut self,
        payload: ValueWord,
        cause: Option<ValueWord>,
        trace_info: ValueWord,
        code: Option<&str>,
    ) -> ValueWord {
        let schema_id = self.builtin_schemas.any_error;
        let message = Self::error_message_from_nb(&payload);

        let mut slots = Vec::with_capacity(6);
        let mut heap_mask: u64 = 0;

        // Slot 0: category (always heap string)
        slots.push(ValueSlot::from_heap(HeapValue::String(Arc::new(
            "AnyError".to_string(),
        ))));
        heap_mask |= 1u64 << 0;

        // Slot 1: payload
        let (s, h) = nb_to_slot(&payload);
        slots.push(s);
        if h {
            heap_mask |= 1u64 << 1;
        }

        // Slot 2: cause
        if let Some(cause_nb) = cause {
            let (s, h) = nb_to_slot(&cause_nb);
            slots.push(s);
            if h {
                heap_mask |= 1u64 << 2;
            }
        } else {
            slots.push(ValueSlot::none());
        }

        // Slot 3: trace_info
        let (s, h) = nb_to_slot(&trace_info);
        slots.push(s);
        if h {
            heap_mask |= 1u64 << 3;
        }

        // Slot 4: message (always heap string)
        slots.push(ValueSlot::from_heap(HeapValue::String(Arc::new(message))));
        heap_mask |= 1u64 << 4;

        // Slot 5: code (heap string or none)
        if let Some(c) = code {
            slots.push(ValueSlot::from_heap(HeapValue::String(Arc::new(
                c.to_string(),
            ))));
            heap_mask |= 1u64 << 5;
        } else {
            slots.push(ValueSlot::none());
        }

        ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        })
    }

    // ===== ValueWord-native error formatting =====

    /// Format a trace frame directly from a ValueWord TypedObject.
    fn format_trace_frame_nb(frame: &ValueWord) -> Option<String> {
        let (_schema_id, slots, heap_mask) = raw_helpers::extract_typed_object(*frame)?;

        let function = typed_string_field_from_slots(slots, heap_mask, TRACEFRAME_FUNCTION)
            .unwrap_or_else(|| "<anonymous>".to_string());

        let file = typed_string_field_from_slots(slots, heap_mask, TRACEFRAME_FILE);
        let line = slot_as_i64(slots, heap_mask, TRACEFRAME_LINE);
        let ip = slot_as_i64(slots, heap_mask, TRACEFRAME_IP);

        let mut rendered = format!("  at {}", function);
        match (file, line) {
            (Some(file), Some(line)) => rendered.push_str(&format!(" ({}:{})", file, line)),
            (Some(file), None) => rendered.push_str(&format!(" ({})", file)),
            (None, Some(line)) => rendered.push_str(&format!(" (line {})", line)),
            (None, None) => {}
        }
        if let Some(ip) = ip {
            rendered.push_str(&format!(" [ip {}]", ip));
        }
        Some(rendered)
    }

    /// Format trace info from a ValueWord TypedObject.
    fn format_trace_info_nb(trace_info: &ValueWord) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some((_schema_id, slots, heap_mask)) =
            raw_helpers::extract_typed_object(*trace_info)
        {
            let kind = typed_string_field_from_slots(slots, heap_mask, TRACEINFO_SINGLE_KIND);
            if kind.as_deref() == Some("single") {
                // Single frame in slot 1 — read as heap value, wrap in ValueWord
                if TRACEINFO_SINGLE_FRAME < slots.len()
                    && heap_mask & (1u64 << TRACEINFO_SINGLE_FRAME) != 0
                {
                    let frame_nb = slots[TRACEINFO_SINGLE_FRAME].as_heap_nb();
                    if let Some(line) = Self::format_trace_frame_nb(&frame_nb) {
                        lines.push(line);
                    }
                }
            } else {
                // "full" — slot 1 is frames array
                if TRACEINFO_FULL_FRAMES < slots.len()
                    && heap_mask & (1u64 << TRACEINFO_FULL_FRAMES) != 0
                {
                    let frames_nb = slots[TRACEINFO_FULL_FRAMES].as_heap_nb();
                    if let Some(view) = frames_nb.as_any_array() {
                        let arr = view.to_generic();
                        for frame_nb in arr.iter() {
                            if let Some(line) = Self::format_trace_frame_nb(frame_nb) {
                                lines.push(line);
                            }
                        }
                    }
                }
            }
        }

        lines
    }

    /// Format an error chain starting from a ValueWord error value.
    fn format_any_error_chain_nb(&self, error: &ValueWord) -> String {
        let mut output = String::from("Uncaught exception:");

        if !Self::is_any_error_nb(error) {
            output.push_str(&format!(
                "\nCaused by: {}",
                self.format_value_default_nb(error)
            ));
            return output;
        }

        if let Some((_schema_id, slots, heap_mask)) = raw_helpers::extract_typed_object(*error) {
            let message = typed_string_field_from_slots(slots, heap_mask, ANYERROR_MESSAGE)
                .unwrap_or_else(|| {
                    if ANYERROR_PAYLOAD < slots.len() {
                        let is_heap = heap_mask & (1u64 << ANYERROR_PAYLOAD) != 0;
                        let payload_nb = slots[ANYERROR_PAYLOAD].as_value_word(is_heap);
                        Self::error_message_from_nb(&payload_nb)
                    } else {
                        "Unknown error".to_string()
                    }
                });
            let code = typed_string_field_from_slots(slots, heap_mask, ANYERROR_CODE);

            if let Some(code) = code {
                output.push_str(&format!("\nError [{}]: {}", code, message));
            } else {
                output.push_str(&format!("\nError: {}", message));
            }

            // Format trace info
            if ANYERROR_TRACE_INFO < slots.len() && heap_mask & (1u64 << ANYERROR_TRACE_INFO) != 0
            {
                let trace_nb = slots[ANYERROR_TRACE_INFO].as_heap_nb();
                if !trace_nb.is_none() {
                    for line in Self::format_trace_info_nb(&trace_nb) {
                        output.push('\n');
                        output.push_str(&line);
                    }
                }
            }

            // Follow the cause chain
            if ANYERROR_CAUSE < slots.len() && heap_mask & (1u64 << ANYERROR_CAUSE) != 0 {
                let cause_nb = slots[ANYERROR_CAUSE].as_heap_nb();
                if !cause_nb.is_none() {
                    output.push_str(&self.format_error_chain_tail_nb(&cause_nb, 1));
                }
            }
        }

        output
    }

    /// Continue formatting error chain from a ValueWord cause value.
    fn format_error_chain_tail_nb(&self, cause: &ValueWord, mut depth: usize) -> String {
        let mut output = String::new();
        let mut current = cause.clone();

        loop {
            if !Self::is_any_error_nb(&current) {
                output.push_str(&format!(
                    "\nCaused by: {}",
                    self.format_value_default_nb(&current)
                ));
                break;
            }

            if let Some((_schema_id, slots, heap_mask)) =
                raw_helpers::extract_typed_object(current)
            {
                let message = typed_string_field_from_slots(slots, heap_mask, ANYERROR_MESSAGE)
                    .unwrap_or_else(|| {
                        if ANYERROR_PAYLOAD < slots.len()
                            && heap_mask & (1u64 << ANYERROR_PAYLOAD) != 0
                        {
                            let payload_nb = slots[ANYERROR_PAYLOAD].as_heap_nb();
                            Self::error_message_from_nb(&payload_nb)
                        } else {
                            "Unknown error".to_string()
                        }
                    });
                let code = typed_string_field_from_slots(slots, heap_mask, ANYERROR_CODE);

                if let Some(code) = code {
                    output.push_str(&format!("\nCaused by [{}]: {}", code, message));
                } else {
                    output.push_str(&format!("\nCaused by: {}", message));
                }

                // Format trace info
                if ANYERROR_TRACE_INFO < slots.len()
                    && heap_mask & (1u64 << ANYERROR_TRACE_INFO) != 0
                {
                    let trace_nb = slots[ANYERROR_TRACE_INFO].as_heap_nb();
                    if !trace_nb.is_none() {
                        for line in Self::format_trace_info_nb(&trace_nb) {
                            output.push('\n');
                            output.push_str(&line);
                        }
                    }
                }

                let has_cause =
                    ANYERROR_CAUSE < slots.len() && heap_mask & (1u64 << ANYERROR_CAUSE) != 0;
                let next_cause_nb = if has_cause {
                    slots[ANYERROR_CAUSE].as_heap_nb()
                } else {
                    ValueWord::none()
                };
                if next_cause_nb.is_none() {
                    break;
                }

                depth += 1;
                if depth >= 32 {
                    output.push_str("\nCaused by: [error chain truncated]");
                    break;
                }
                current = next_cause_nb;
            } else {
                output.push_str(&format!(
                    "\nCaused by: {}",
                    self.format_value_default_nb(&current)
                ));
                break;
            }
        }

        output
    }

    /// Format an uncaught exception from a ValueWord error value.
    fn format_uncaught_exception_nb(&self, error: &ValueWord) -> String {
        if Self::is_any_error_nb(error) {
            self.format_any_error_chain_nb(error)
        } else {
            format!(
                "Uncaught exception: {}",
                self.format_value_default_nb(error)
            )
        }
    }

    pub(in crate::executor) fn normalize_err_payload_nb(
        &mut self,
        payload: ValueWord,
    ) -> ValueWord {
        if Self::is_any_error_nb(&payload) {
            payload
        } else {
            let trace = self.trace_info_full_nb();
            self.build_any_error_nb(payload, None, trace, None)
        }
    }

    fn build_try_none_error_nb(&mut self) -> ValueWord {
        let trace = self.trace_info_single_nb();
        self.build_any_error_nb(
            ValueWord::from_string(Arc::new("Value was None".to_string())),
            None,
            trace,
            Some("OPTION_NONE"),
        )
    }

    pub(in crate::executor) fn op_error_context(&mut self) -> Result<(), VMError> {
        let context_nb = self.pop_raw_u64()?;
        let value_nb = self.pop_raw_u64()?;

        // Fast path: inline None
        if value_nb.is_none() {
            let none_cause = self.build_try_none_error_nb();
            let trace = self.trace_info_single_nb();
            let wrapped = self.build_any_error_nb(context_nb, Some(none_cause), trace, None);
            return self.push_raw_u64(ValueWord::from_err(wrapped));
        }

        let result = if let Some(inner) = raw_helpers::extract_ok_inner(value_nb) {
            ValueWord::from_ok(inner.clone())
        } else if let Some(inner) = raw_helpers::extract_some_inner(value_nb) {
            ValueWord::from_ok(inner.clone())
        } else if let Some(inner) = raw_helpers::extract_err_inner(value_nb) {
            let cause = self.normalize_err_payload_nb(inner.clone());
            let trace = self.trace_info_single_nb();
            let wrapped = self.build_any_error_nb(context_nb, Some(cause), trace, None);
            ValueWord::from_err(wrapped)
        } else {
            ValueWord::from_ok(value_nb)
        };

        self.push_raw_u64(result)
    }

    /// Try operator for unified Result/Option propagation.
    ///
    /// Behavior:
    /// - `Ok(value)` => unwraps to `value`
    /// - `Err(error)` => returns early with `Err(error)`
    /// - `None` => returns early with `Err(AnyError-like object)`
    /// - `Some(value)` => unwraps to `value`
    /// - bare non-`None` values => pass-through for nullable `Option<T>` runtime encoding
    pub(in crate::executor) fn op_try_unwrap(&mut self) -> Result<(), VMError> {
        fn return_early_with_err(
            vm: &mut VirtualMachine,
            payload: ValueWord,
        ) -> Result<(), VMError> {
            if vm.call_stack.is_empty() {
                vm.handle_exception_nb(payload)
            } else {
                vm.push_raw_u64(ValueWord::from_err(payload))?;
                vm.op_return_value()
            }
        }

        let nb = self.pop_raw_u64()?;
        // Fast path: inline None
        if nb.is_none() {
            let err = self.build_try_none_error_nb();
            return return_early_with_err(self, err);
        }
        // Heap types: Ok/Err/Some
        if let Some(inner) = raw_helpers::extract_ok_inner(nb) {
            self.push_raw_u64(inner.clone())?;
            Ok(())
        } else if let Some(inner) = raw_helpers::extract_err_inner(nb) {
            return_early_with_err(self, inner.clone())
        } else if let Some(inner) = raw_helpers::extract_some_inner(nb) {
            self.push_raw_u64(inner.clone())?;
            Ok(())
        } else {
            // All non-None, non-Err values are successful payloads
            self.push_raw_u64(nb)?;
            Ok(())
        }
    }

    pub(in crate::executor) fn op_unwrap_option(&mut self) -> Result<(), VMError> {
        let nb = self.pop_raw_u64()?;
        if nb.is_none() {
            return Err(VMError::RuntimeError(
                "Cannot unwrap None value".to_string(),
            ));
        }
        if let Some(inner) = raw_helpers::extract_some_inner(nb) {
            self.push_raw_u64(inner.clone())?;
            Ok(())
        } else {
            // Some() constructor returns the value unwrapped (not wrapped in
            // HeapValue::Some), so non-None values are already the inner value.
            self.push_raw_u64(nb)?;
            Ok(())
        }
    }

    #[inline(always)]
    pub(in crate::executor) fn op_is_ok(&mut self) -> Result<(), VMError> {
        let nb = self.pop_raw_u64()?;
        let is_ok = nb.as_ok_inner().is_some();
        self.push_raw_u64(ValueWord::from_bool(is_ok))
    }

    #[inline(always)]
    pub(in crate::executor) fn op_is_err(&mut self) -> Result<(), VMError> {
        let nb = self.pop_raw_u64()?;
        let is_err = nb.as_err_inner().is_some();
        self.push_raw_u64(ValueWord::from_bool(is_err))
    }

    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_ok(&mut self) -> Result<(), VMError> {
        let nb = self.pop_raw_u64()?;
        if let Some(inner) = raw_helpers::extract_ok_inner(nb) {
            self.push_raw_u64(inner.clone())
        } else {
            Err(VMError::RuntimeError(format!(
                "UnwrapOk can only be applied to Ok(value), got {}",
                nb.type_name()
            )))
        }
    }

    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_err(&mut self) -> Result<(), VMError> {
        let nb = self.pop_raw_u64()?;
        if let Some(inner) = raw_helpers::extract_err_inner(nb) {
            let inner_val = inner.clone();
            // If the inner value is an AnyError TypedObject (created by
            // normalize_err_payload_nb), extract and return the original
            // payload rather than exposing the full AnyError struct.
            // This ensures `match Err("fail") { Err(msg) => msg }` returns
            // "fail" instead of the AnyError wrapper.
            if Self::is_any_error_nb(&inner_val) {
                if let Some((_schema_id, slots, heap_mask)) =
                    raw_helpers::extract_typed_object(inner_val)
                {
                    if ANYERROR_PAYLOAD < slots.len() {
                        let is_heap = heap_mask & (1u64 << ANYERROR_PAYLOAD) != 0;
                        let payload = slots[ANYERROR_PAYLOAD].as_value_word(is_heap);
                        return self.push_raw_u64(payload);
                    }
                }
            }
            self.push_raw_u64(inner_val)
        } else {
            Err(VMError::RuntimeError(format!(
                "UnwrapErr can only be applied to Err(value), got {}",
                nb.type_name()
            )))
        }
    }
}
