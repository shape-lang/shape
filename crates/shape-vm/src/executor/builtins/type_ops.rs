//! Type checking and conversion builtin implementations
//!
//! Handles: isNumber, isString, isBool, isArray, isObject, isDataRow, toString, toNumber, toBool, typeOf

use crate::bytecode::{BuiltinFunction, Constant, Instruction, Operand};
use crate::executor::VirtualMachine;
use rust_decimal::prelude::ToPrimitive;
use shape_ast::ast::TypeAnnotation;
use shape_value::{HeapKind, NanTag, VMError, ValueWord, heap_value::HeapValue};
use std::sync::Arc;

const INTO_DISPATCH_TAG: &str = "__IntoDispatch";
const TRY_INTO_DISPATCH_TAG: &str = "__TryIntoDispatch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvertDispatchKind {
    Into,
    TryInto,
}

/// Helper to check that exactly `expected` args were passed.
#[inline]
fn check_arity(function: &str, args: &[ValueWord], expected: usize) -> Result<(), VMError> {
    if args.len() != expected {
        return Err(VMError::ArityMismatch {
            function: function.to_string(),
            expected,
            got: args.len(),
        });
    }
    Ok(())
}

#[inline]
fn ptr_arg_as_usize(arg: &ValueWord, function: &str, arg_name: &str) -> Result<usize, VMError> {
    arg.as_usize().ok_or_else(|| VMError::InvalidArgument {
        function: function.to_string(),
        message: format!("{arg_name} must be a pointer-compatible value"),
    })
}

impl VirtualMachine {
    fn convert_to_int_no_checks(value: &ValueWord) -> Result<ValueWord, String> {
        if let Some(i) = value.as_i64() {
            return Ok(ValueWord::from_i64(i));
        }
        if let Some(n) = value.as_f64() {
            if !n.is_finite() {
                return Err("cannot convert non-finite number to int".to_string());
            }
            let i = n as i64;
            if (i as f64 - n).abs() > f64::EPSILON {
                return Err(format!("cannot convert non-integer number '{n}' to int"));
            }
            return Ok(ValueWord::from_i64(i));
        }
        if let Some(s) = value.as_str() {
            let parsed = s
                .parse::<i64>()
                .map_err(|_| format!("cannot convert string '{s}' to int"))?;
            return Ok(ValueWord::from_i64(parsed));
        }
        if let Some(b) = value.as_bool() {
            return Ok(ValueWord::from_i64(if b { 1 } else { 0 }));
        }
        if let Some(d) = value.as_decimal() {
            if let Some(i) = d.to_i64() {
                return Ok(ValueWord::from_i64(i));
            }
            return Err(format!("cannot convert decimal '{d}' to int"));
        }
        if let Some(c) = value.as_char() {
            return Ok(ValueWord::from_i64(c as i64));
        }
        Err(format!("cannot convert {} to int", value.type_name()))
    }

    fn convert_to_number_no_checks(value: &ValueWord) -> Result<ValueWord, String> {
        if let Some(n) = value.as_number_coerce() {
            return Ok(ValueWord::from_f64(n));
        }
        if let Some(s) = value.as_str() {
            let parsed = s
                .parse::<f64>()
                .map_err(|_| format!("cannot convert string '{s}' to number"))?;
            return Ok(ValueWord::from_f64(parsed));
        }
        if let Some(b) = value.as_bool() {
            return Ok(ValueWord::from_f64(if b { 1.0 } else { 0.0 }));
        }
        Err(format!("cannot convert {} to number", value.type_name()))
    }

    fn convert_to_decimal_no_checks(value: &ValueWord) -> Result<ValueWord, String> {
        if let Some(d) = value.as_decimal() {
            return Ok(ValueWord::from_decimal(d));
        }
        if let Some(i) = value.as_i64() {
            return Ok(ValueWord::from_decimal(rust_decimal::Decimal::from(i)));
        }
        if let Some(n) = value.as_f64() {
            let d = rust_decimal::Decimal::from_f64_retain(n)
                .ok_or_else(|| format!("cannot convert number '{n}' to decimal"))?;
            return Ok(ValueWord::from_decimal(d));
        }
        if let Some(s) = value.as_str() {
            let d = s
                .parse::<rust_decimal::Decimal>()
                .map_err(|_| format!("cannot convert string '{s}' to decimal"))?;
            return Ok(ValueWord::from_decimal(d));
        }
        if let Some(b) = value.as_bool() {
            return Ok(ValueWord::from_decimal(rust_decimal::Decimal::from(if b {
                1
            } else {
                0
            })));
        }
        Err(format!("cannot convert {} to decimal", value.type_name()))
    }

    fn convert_to_bool_no_checks(value: &ValueWord) -> Result<ValueWord, String> {
        if let Some(b) = value.as_bool() {
            return Ok(ValueWord::from_bool(b));
        }
        if let Some(i) = value.as_i64() {
            return Ok(ValueWord::from_bool(i != 0));
        }
        if let Some(n) = value.as_f64() {
            return Ok(ValueWord::from_bool(n != 0.0));
        }
        if let Some(s) = value.as_str() {
            let parsed = match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => return Err(format!("cannot convert string '{s}' to bool")),
            };
            return Ok(ValueWord::from_bool(parsed));
        }
        Err(format!("cannot convert {} to bool", value.type_name()))
    }

    fn convert_to_char_no_checks(value: &ValueWord) -> Result<ValueWord, String> {
        if let Some(c) = value.as_char() {
            return Ok(ValueWord::from_char(c));
        }
        if let Some(i) = value.as_i64() {
            let code = i as u32;
            return char::from_u32(code)
                .map(ValueWord::from_char)
                .ok_or_else(|| format!("invalid Unicode code point: {}", code));
        }
        if let Some(n) = value.as_f64() {
            let code = n as u32;
            return char::from_u32(code)
                .map(ValueWord::from_char)
                .ok_or_else(|| format!("invalid Unicode code point: {}", code));
        }
        if let Some(s) = value.as_str() {
            let mut chars = s.chars();
            if let Some(c) = chars.next() {
                if chars.next().is_none() {
                    return Ok(ValueWord::from_char(c));
                }
            }
            return Err(format!(
                "cannot convert string '{}' to char (must be single character)",
                s
            ));
        }
        Err(format!("cannot convert {} to char", value.type_name()))
    }

    fn convert_to_string_no_checks(&self, value: &ValueWord) -> ValueWord {
        if let Some(s) = value.as_str() {
            return ValueWord::from_string(Arc::new(s.to_string()));
        }
        ValueWord::from_string(Arc::new(self.format_value_default_nb(value)))
    }

    fn canonical_try_into_name(name: &str) -> String {
        match name {
            "boolean" | "Boolean" | "Bool" => "bool".to_string(),
            "String" => "string".to_string(),
            "Number" => "number".to_string(),
            "Int" => "int".to_string(),
            "Decimal" => "decimal".to_string(),
            "Char" => "char".to_string(),
            _ => name.to_string(),
        }
    }

    fn annotation_conversion_name(target: &TypeAnnotation) -> Option<String> {
        match target {
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                Self::annotation_conversion_name(&args[0])
            }
            TypeAnnotation::Basic(name) => Some(Self::canonical_try_into_name(name)),
            TypeAnnotation::Reference(name) => Some(Self::canonical_try_into_name(name)),
            TypeAnnotation::Generic { name, .. } => Some(Self::canonical_try_into_name(name)),
            _ => None,
        }
    }

    fn decode_convert_dispatch(
        target: &TypeAnnotation,
    ) -> Result<(ConvertDispatchKind, Option<String>, String), String> {
        if let TypeAnnotation::Generic { name, args } = target
            && (name == TRY_INTO_DISPATCH_TAG || name == INTO_DISPATCH_TAG)
            && args.len() == 2
        {
            let source = Self::annotation_conversion_name(&args[0]).ok_or_else(|| {
                format!(
                    "invalid conversion dispatch source annotation: {:?}",
                    args[0]
                )
            })?;
            let selector = Self::annotation_conversion_name(&args[1]).ok_or_else(|| {
                format!(
                    "invalid conversion dispatch target selector annotation: {:?}",
                    args[1]
                )
            })?;
            let kind = if name == TRY_INTO_DISPATCH_TAG {
                ConvertDispatchKind::TryInto
            } else {
                ConvertDispatchKind::Into
            };
            return Ok((kind, Some(source), selector));
        }

        if let TypeAnnotation::Generic { name, args } = target
            && name == "Option"
            && args.len() == 1
        {
            let selector = Self::annotation_conversion_name(&args[0])
                .ok_or_else(|| format!("invalid conversion target annotation: {:?}", target))?;
            return Ok((ConvertDispatchKind::TryInto, None, selector));
        }

        let selector = Self::annotation_conversion_name(target)
            .ok_or_else(|| format!("invalid conversion target annotation: {:?}", target))?;
        Ok((ConvertDispatchKind::Into, None, selector))
    }

    fn resolve_try_into_symbol(&self, source_type: &str, target_selector: &str) -> Option<String> {
        self.program
            .lookup_trait_method_symbol("TryInto", source_type, Some(target_selector), "tryInto")
            .or_else(|| {
                self.program
                    .lookup_trait_method_symbol("TryInto", source_type, None, "tryInto")
            })
            .map(|s| s.to_string())
    }

    fn resolve_into_symbol(&self, source_type: &str, target_selector: &str) -> Option<String> {
        self.program
            .lookup_trait_method_symbol("Into", source_type, Some(target_selector), "into")
            .or_else(|| {
                self.program
                    .lookup_trait_method_symbol("Into", source_type, None, "into")
            })
            .map(|s| s.to_string())
    }

    fn build_try_into_error_result(&mut self, message: String, code: &str) -> ValueWord {
        let trace = self.trace_info_single_nb();
        let err = self.build_any_error_nb(
            ValueWord::from_string(Arc::new(message)),
            None,
            trace,
            Some(code),
        );
        ValueWord::from_err(err)
    }

    fn try_convert_no_checks(
        &self,
        value: &ValueWord,
        target_name: &str,
    ) -> Result<ValueWord, String> {
        match target_name {
            "int" => Self::convert_to_int_no_checks(value),
            "number" => Self::convert_to_number_no_checks(value),
            "decimal" => Self::convert_to_decimal_no_checks(value),
            "bool" => Self::convert_to_bool_no_checks(value),
            "string" => Ok(self.convert_to_string_no_checks(value)),
            "char" => Self::convert_to_char_no_checks(value),
            unsupported => Err(format!(
                "unsupported fallible conversion target '{unsupported}'"
            )),
        }
    }

    fn try_into_source_name_for_value(&self, value: &ValueWord) -> Option<String> {
        Self::annotation_conversion_name(&self.type_annotation_for_nb(value))
    }

    pub(in crate::executor) fn op_convert(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let target = match instruction.operand {
            Some(Operand::Const(idx)) => match self.program.constants.get(idx as usize) {
                Some(Constant::TypeAnnotation(annotation)) => annotation.clone(),
                _ => {
                    return Err(VMError::RuntimeError(
                        "Convert expects type annotation constant".to_string(),
                    ));
                }
            },
            _ => return Err(VMError::InvalidOperand),
        };

        let value = self.pop_vw()?;
        let (dispatch_kind, encoded_source, target_selector) =
            match Self::decode_convert_dispatch(&target) {
                Ok(dispatch) => dispatch,
                Err(message) => {
                    let err = self.build_try_into_error_result(message, "CONVERT_DISPATCH");
                    self.push_vw(err)?;
                    return Ok(());
                }
            };

        let source_name = encoded_source
            .or_else(|| self.try_into_source_name_for_value(&value))
            .unwrap_or_else(|| Self::canonical_try_into_name(value.type_name()));

        if source_name == target_selector {
            match dispatch_kind {
                ConvertDispatchKind::TryInto => {
                    self.push_vw(ValueWord::from_ok(value))?;
                }
                ConvertDispatchKind::Into => {
                    self.push_vw(value)?;
                }
            }
            return Ok(());
        }

        match dispatch_kind {
            ConvertDispatchKind::TryInto => {
                let Some(symbol) = self.resolve_try_into_symbol(&source_name, &target_selector)
                else {
                    // Fallback: built-in primitive conversions
                    let converted = match self.try_convert_no_checks(&value, &target_selector) {
                        Ok(result_nb) => ValueWord::from_ok(result_nb),
                        Err(msg) => self.build_try_into_error_result(msg, "TRY_INTO_IMPL_MISSING"),
                    };
                    self.push_vw(converted)?;
                    return Ok(());
                };

                let Some(&func_id) = self.function_name_index.get(&symbol) else {
                    let err = self.build_try_into_error_result(
                        format!(
                            "TryInto dispatch target '{}' is not a compiled function",
                            symbol
                        ),
                        "TRY_INTO_SYMBOL_MISSING",
                    );
                    self.push_vw(err)?;
                    return Ok(());
                };

                let func_nb = ValueWord::from_function(func_id);
                let converted = match self.call_value_immediate_nb(
                    &func_nb,
                    std::slice::from_ref(&value),
                    None,
                ) {
                    Ok(result_nb) => {
                        if result_nb.as_ok_inner().is_some() || result_nb.as_err_inner().is_some() {
                            result_nb
                        } else {
                            self.build_try_into_error_result(
                                format!(
                                    "TryInto impl '{}' returned '{}' instead of Result",
                                    symbol,
                                    result_nb.type_name()
                                ),
                                "TRY_INTO_INVALID_RETURN",
                            )
                        }
                    }
                    Err(err) => {
                        self.build_try_into_error_result(err.to_string(), "TRY_INTO_FAILED")
                    }
                };

                self.push_vw(converted)
            }
            ConvertDispatchKind::Into => {
                let Some(symbol) = self.resolve_into_symbol(&source_name, &target_selector) else {
                    // Fallback: built-in primitive conversions
                    match self.try_convert_no_checks(&value, &target_selector) {
                        Ok(result_nb) => {
                            self.push_vw(result_nb)?;
                            return Ok(());
                        }
                        Err(msg) => {
                            return Err(VMError::RuntimeError(format!(
                                "INTO_IMPL_MISSING: {}",
                                msg
                            )));
                        }
                    }
                };

                let Some(&func_id) = self.function_name_index.get(&symbol) else {
                    return Err(VMError::RuntimeError(format!(
                        "INTO_SYMBOL_MISSING: Into dispatch target '{}' is not a compiled function",
                        symbol
                    )));
                };

                let func_nb = ValueWord::from_function(func_id);
                let converted = self
                    .call_value_immediate_nb(&func_nb, std::slice::from_ref(&value), None)
                    .map_err(|err| VMError::RuntimeError(format!("INTO_FAILED: {}", err)))?;

                if converted.as_ok_inner().is_some() || converted.as_err_inner().is_some() {
                    return Err(VMError::RuntimeError(format!(
                        "INTO_INVALID_RETURN: Into impl '{}' returned Result instead of '{}'",
                        symbol, target_selector
                    )));
                }

                self.push_vw(converted)
            }
        }
    }

    // ===== Typed Conversion Opcodes (zero-dispatch, no operand) =====

    /// ConvertToInt: pop value, convert to int, push result. Panics on failure.
    #[inline]
    pub(in crate::executor) fn op_convert_to_int(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = Self::convert_to_int_no_checks(&value)
            .map_err(|msg| VMError::RuntimeError(format!("INTO_FAILED: {}", msg)))?;
        self.push_vw(result)
    }

    /// ConvertToNumber: pop value, convert to number, push result. Panics on failure.
    #[inline]
    pub(in crate::executor) fn op_convert_to_number(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = Self::convert_to_number_no_checks(&value)
            .map_err(|msg| VMError::RuntimeError(format!("INTO_FAILED: {}", msg)))?;
        self.push_vw(result)
    }

    /// ConvertToString: pop value, convert to string, push result. Always succeeds.
    #[inline]
    pub(in crate::executor) fn op_convert_to_string(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = self.convert_to_string_no_checks(&value);
        self.push_vw(result)
    }

    /// ConvertToBool: pop value, convert to bool, push result. Panics on failure.
    #[inline]
    pub(in crate::executor) fn op_convert_to_bool(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = Self::convert_to_bool_no_checks(&value)
            .map_err(|msg| VMError::RuntimeError(format!("INTO_FAILED: {}", msg)))?;
        self.push_vw(result)
    }

    /// ConvertToDecimal: pop value, convert to decimal, push result. Panics on failure.
    #[inline]
    pub(in crate::executor) fn op_convert_to_decimal(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = Self::convert_to_decimal_no_checks(&value)
            .map_err(|msg| VMError::RuntimeError(format!("INTO_FAILED: {}", msg)))?;
        self.push_vw(result)
    }

    /// ConvertToChar: pop value, convert to char, push result. Panics on failure.
    #[inline]
    pub(in crate::executor) fn op_convert_to_char(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = Self::convert_to_char_no_checks(&value)
            .map_err(|msg| VMError::RuntimeError(format!("INTO_FAILED: {}", msg)))?;
        self.push_vw(result)
    }

    /// TryConvertToInt: pop value, try convert to int, push Result<int, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_int(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = match Self::convert_to_int_no_checks(&value) {
            Ok(v) => ValueWord::from_ok(v),
            Err(msg) => self.build_try_into_error_result(msg, "TRY_INTO_FAILED"),
        };
        self.push_vw(result)
    }

    /// TryConvertToNumber: pop value, try convert to number, push Result<number, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_number(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = match Self::convert_to_number_no_checks(&value) {
            Ok(v) => ValueWord::from_ok(v),
            Err(msg) => self.build_try_into_error_result(msg, "TRY_INTO_FAILED"),
        };
        self.push_vw(result)
    }

    /// TryConvertToString: pop value, try convert to string, push Result<string, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_string(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = ValueWord::from_ok(self.convert_to_string_no_checks(&value));
        self.push_vw(result)
    }

    /// TryConvertToBool: pop value, try convert to bool, push Result<bool, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_bool(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = match Self::convert_to_bool_no_checks(&value) {
            Ok(v) => ValueWord::from_ok(v),
            Err(msg) => self.build_try_into_error_result(msg, "TRY_INTO_FAILED"),
        };
        self.push_vw(result)
    }

    /// TryConvertToDecimal: pop value, try convert to decimal, push Result<decimal, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_decimal(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = match Self::convert_to_decimal_no_checks(&value) {
            Ok(v) => ValueWord::from_ok(v),
            Err(msg) => self.build_try_into_error_result(msg, "TRY_INTO_FAILED"),
        };
        self.push_vw(result)
    }

    /// TryConvertToChar: pop value, try convert to char, push Result<char, AnyError>.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_char(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        let result = match Self::convert_to_char_no_checks(&value) {
            Ok(v) => ValueWord::from_ok(v),
            Err(msg) => self.build_try_into_error_result(msg, "TRY_INTO_FAILED"),
        };
        self.push_vw(result)
    }

    fn type_name_to_annotation(name: &str) -> TypeAnnotation {
        match name {
            "number" | "int" | "decimal" | "string" | "bool" | "row" | "pattern" | "function"
            | "module_function" | "duration" | "datetime" | "time" | "timeframe" | "table"
            | "array" | "object" | "option" | "result" | "Type" | "type" | "i8" | "u8" | "i16"
            | "u16" | "i32" | "u32" | "i64" | "u64" | "isize" | "usize" | "byte" | "char" => {
                TypeAnnotation::Basic(name.to_string())
            }
            "()" | "unit" => TypeAnnotation::Void,
            "None" => TypeAnnotation::Null,
            _ => TypeAnnotation::Reference(name.into()),
        }
    }

    fn type_annotation_for_nb(&self, nb: &ValueWord) -> TypeAnnotation {
        match nb.tag() {
            NanTag::F64 => TypeAnnotation::Basic("number".to_string()),
            NanTag::I48 => TypeAnnotation::Basic("int".to_string()),
            NanTag::Bool => TypeAnnotation::Basic("bool".to_string()),
            NanTag::None => TypeAnnotation::Generic {
                name: "Option".into(),
                args: vec![TypeAnnotation::Basic("unknown".to_string())],
            },
            NanTag::Unit => TypeAnnotation::Void,
            NanTag::Function | NanTag::ModuleFunction => {
                TypeAnnotation::Basic("function".to_string())
            }
            NanTag::Ref => TypeAnnotation::Basic("reference".to_string()),
            NanTag::Heap => {
                if let Some(shape_value::HeapValue::TypeAnnotation(_)) = nb.as_heap_ref() {
                    return TypeAnnotation::Reference("Type".into());
                }

                if let Some(shape_value::HeapValue::TypedObject { schema_id, .. }) =
                    nb.as_heap_ref()
                {
                    let type_name = self
                        .program
                        .type_schema_registry
                        .get_by_id(*schema_id as u32)
                        .map(|schema| schema.name.clone());

                    if let Some(name) = type_name.filter(|n| !n.starts_with("__")) {
                        return Self::type_name_to_annotation(&name);
                    }
                }

                Self::type_name_to_annotation(nb.type_name())
            }
        }
    }

    /// IsNumber: Check if value is a number
    #[inline]
    pub(in crate::executor) fn builtin_is_number(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("is_number", &args, 1)?;
        let result = match args[0].tag() {
            NanTag::F64 | NanTag::I48 => true,
            NanTag::Heap => matches!(
                args[0].heap_kind(),
                Some(HeapKind::Decimal | HeapKind::BigInt)
            ),
            _ => false,
        };
        Ok(ValueWord::from_bool(result))
    }

    /// IsString: Check if value is a string
    #[inline]
    pub(in crate::executor) fn builtin_is_string(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("is_string", &args, 1)?;
        Ok(ValueWord::from_bool(args[0].as_str().is_some()))
    }

    /// IsBool: Check if value is a boolean
    #[inline]
    pub(in crate::executor) fn builtin_is_bool(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("is_bool", &args, 1)?;
        Ok(ValueWord::from_bool(args[0].tag() == NanTag::Bool))
    }

    /// IsArray: Check if value is an array
    #[inline]
    pub(in crate::executor) fn builtin_is_array(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("is_array", &args, 1)?;
        Ok(ValueWord::from_bool(args[0].as_any_array().is_some()))
    }

    /// IsObject: Check if value is an object
    #[inline]
    pub(in crate::executor) fn builtin_is_object(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("is_object", &args, 1)?;
        Ok(ValueWord::from_bool(matches!(
            args[0].heap_kind(),
            Some(HeapKind::TypedObject)
        )))
    }

    /// IsDataRow: Check if value is a data row (always false - legacy)
    #[inline]
    pub(in crate::executor) fn builtin_is_data_row(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("is_data_row", &args, 1)?;
        Ok(ValueWord::from_bool(false)) // DataRow type no longer exists
    }

    /// Dispatch conversion builtins from the main executor loop.
    pub(in crate::executor) fn dispatch_conversion_builtin(
        &mut self,
        builtin: BuiltinFunction,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        match builtin {
            BuiltinFunction::ToString => self.builtin_to_string(args),
            BuiltinFunction::ToNumber => self.builtin_to_number(args),
            BuiltinFunction::ToBool => self.builtin_to_bool(args),
            other => Err(VMError::RuntimeError(format!(
                "conversion dispatch does not support {:?}",
                other
            ))),
        }
    }

    #[inline]
    fn native_result_err(&mut self, message: String, code: &str) -> ValueWord {
        let trace = self.trace_info_single_nb();
        let err = self.build_any_error_nb(
            ValueWord::from_string(Arc::new(message)),
            None,
            trace,
            Some(code),
        );
        ValueWord::from_err(err)
    }

    /// Dispatch native interop builtins from the main executor loop.
    pub(in crate::executor) fn dispatch_native_interop_builtin(
        &mut self,
        builtin: BuiltinFunction,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        match builtin {
            BuiltinFunction::NativePtrSize => self.builtin_native_ptr_size(args),
            BuiltinFunction::NativePtrNewCell => self.builtin_native_ptr_new_cell(args),
            BuiltinFunction::NativePtrFreeCell => self.builtin_native_ptr_free_cell(args),
            BuiltinFunction::NativePtrReadPtr => self.builtin_native_ptr_read_ptr(args),
            BuiltinFunction::NativePtrWritePtr => self.builtin_native_ptr_write_ptr(args),
            BuiltinFunction::NativeTableFromArrowC => self.builtin_native_table_from_arrow_c(args),
            BuiltinFunction::NativeTableFromArrowCTyped => {
                self.builtin_native_table_from_arrow_c_typed(args)
            }
            BuiltinFunction::NativeTableBindType => self.builtin_native_table_bind_type(args),
            other => Err(VMError::RuntimeError(format!(
                "native interop dispatch does not support {:?}",
                other
            ))),
        }
    }

    /// Return native pointer width in bytes.
    pub(in crate::executor) fn builtin_native_ptr_size(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_ptr_size", &args, 0)?;
        Ok(ValueWord::from_native_usize(std::mem::size_of::<usize>()))
    }

    /// Allocate a pointer-sized native cell initialized to null.
    pub(in crate::executor) fn builtin_native_ptr_new_cell(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_ptr_new_cell", &args, 0)?;
        let cell = Box::new(0usize);
        let ptr = Box::into_raw(cell) as usize;
        Ok(ValueWord::from_native_ptr(ptr))
    }

    /// Free a pointer-sized native cell allocated by `__native_ptr_new_cell`.
    pub(in crate::executor) fn builtin_native_ptr_free_cell(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_ptr_free_cell", &args, 1)?;
        let addr = ptr_arg_as_usize(&args[0], "__native_ptr_free_cell", "cell")?;
        if addr != 0 {
            unsafe {
                drop(Box::from_raw(addr as *mut usize));
            }
        }
        Ok(ValueWord::unit())
    }

    /// Read a pointer-sized value from a raw memory address.
    pub(in crate::executor) fn builtin_native_ptr_read_ptr(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_ptr_read_ptr", &args, 1)?;
        let addr = ptr_arg_as_usize(&args[0], "__native_ptr_read_ptr", "addr")?;
        if addr == 0 {
            return Ok(ValueWord::from_native_ptr(0));
        }
        let value = unsafe { std::ptr::read_unaligned(addr as *const usize) };
        Ok(ValueWord::from_native_ptr(value))
    }

    /// Write a pointer-sized value to a raw memory address.
    pub(in crate::executor) fn builtin_native_ptr_write_ptr(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_ptr_write_ptr", &args, 2)?;
        let addr = ptr_arg_as_usize(&args[0], "__native_ptr_write_ptr", "addr")?;
        let value = ptr_arg_as_usize(&args[1], "__native_ptr_write_ptr", "value")?;
        if addr == 0 {
            return Err(VMError::InvalidArgument {
                function: "__native_ptr_write_ptr".to_string(),
                message: "addr must not be null".to_string(),
            });
        }
        unsafe {
            std::ptr::write_unaligned(addr as *mut usize, value);
        }
        Ok(ValueWord::unit())
    }

    /// Import Arrow C pointers as `Result<Table<any>, AnyError>`.
    pub(in crate::executor) fn builtin_native_table_from_arrow_c(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_table_from_arrow_c", &args, 2)?;

        let schema_ptr =
            match ptr_arg_as_usize(&args[0], "__native_table_from_arrow_c", "schema_ptr") {
                Ok(v) => v,
                Err(e) => return Ok(self.native_result_err(format!("{e}"), "NATIVE_ARROW_IMPORT")),
            };
        let array_ptr = match ptr_arg_as_usize(&args[1], "__native_table_from_arrow_c", "array_ptr")
        {
            Ok(v) => v,
            Err(e) => return Ok(self.native_result_err(format!("{e}"), "NATIVE_ARROW_IMPORT")),
        };

        let imported =
            unsafe { shape_runtime::arrow_c::datatable_from_arrow_c_ptrs(schema_ptr, array_ptr) };
        match imported {
            Ok(table) => Ok(ValueWord::from_ok(ValueWord::from_datatable(Arc::new(
                table,
            )))),
            Err(message) => Ok(self.native_result_err(message, "NATIVE_ARROW_IMPORT")),
        }
    }

    /// Import Arrow C pointers and bind to a named row type in one step.
    pub(in crate::executor) fn builtin_native_table_from_arrow_c_typed(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_table_from_arrow_c_typed", &args, 3)?;

        let schema_ptr =
            match ptr_arg_as_usize(&args[0], "__native_table_from_arrow_c_typed", "schema_ptr") {
                Ok(v) => v,
                Err(e) => return Ok(self.native_result_err(format!("{e}"), "NATIVE_ARROW_IMPORT")),
            };
        let array_ptr =
            match ptr_arg_as_usize(&args[1], "__native_table_from_arrow_c_typed", "array_ptr") {
                Ok(v) => v,
                Err(e) => return Ok(self.native_result_err(format!("{e}"), "NATIVE_ARROW_IMPORT")),
            };
        let Some(type_name) = args[2].as_str() else {
            return Ok(self.native_result_err(
                "__native_table_from_arrow_c_typed expects type_name as string".to_string(),
                "NATIVE_TABLE_BIND",
            ));
        };

        let imported =
            unsafe { shape_runtime::arrow_c::datatable_from_arrow_c_ptrs(schema_ptr, array_ptr) };
        let table = match imported {
            Ok(table) => Arc::new(table),
            Err(message) => return Ok(self.native_result_err(message, "NATIVE_ARROW_IMPORT")),
        };

        let Some(schema) = self.program.type_schema_registry.get(type_name) else {
            return Ok(self.native_result_err(
                format!("unknown type schema '{}'", type_name),
                "NATIVE_TABLE_BIND",
            ));
        };
        match schema.bind_to_arrow_schema(&table.schema()) {
            Ok(_) => Ok(ValueWord::from_ok(ValueWord::from_typed_table(
                schema.id as u64,
                table,
            ))),
            Err(err) => Ok(self.native_result_err(
                format!("schema mismatch for '{}': {}", type_name, err),
                "NATIVE_TABLE_BIND",
            )),
        }
    }

    /// Validate/bind a table to a named row type.
    pub(in crate::executor) fn builtin_native_table_bind_type(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("__native_table_bind_type", &args, 2)?;

        let table = match args[0].as_heap_ref() {
            Some(HeapValue::DataTable(dt)) => dt.clone(),
            Some(HeapValue::TypedTable { table, .. }) => table.clone(),
            Some(HeapValue::IndexedTable { table, .. }) => table.clone(),
            _ => {
                return Ok(self.native_result_err(
                    format!(
                        "__native_table_bind_type expects a table value, got '{}'",
                        args[0].type_name()
                    ),
                    "NATIVE_TABLE_BIND",
                ));
            }
        };

        let Some(type_name) = args[1].as_str() else {
            return Ok(self.native_result_err(
                "__native_table_bind_type expects type_name as string".to_string(),
                "NATIVE_TABLE_BIND",
            ));
        };

        let Some(schema) = self.program.type_schema_registry.get(type_name) else {
            return Ok(self.native_result_err(
                format!("unknown type schema '{}'", type_name),
                "NATIVE_TABLE_BIND",
            ));
        };

        match schema.bind_to_arrow_schema(&table.schema()) {
            Ok(_) => Ok(ValueWord::from_ok(ValueWord::from_typed_table(
                schema.id as u64,
                table,
            ))),
            Err(err) => Ok(self.native_result_err(
                format!("schema mismatch for '{}': {}", type_name, err),
                "NATIVE_TABLE_BIND",
            )),
        }
    }

    /// ToString: Convert value to string
    pub(in crate::executor) fn builtin_to_string(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("to_string", &args, 1)?;
        // Fast path for inline types
        if let Some(n) = args[0].as_f64() {
            return Ok(ValueWord::from_string(Arc::new(format!("{}", n))));
        }
        if let Some(i) = args[0].as_i64() {
            return Ok(ValueWord::from_string(Arc::new(format!("{}", i))));
        }
        if let Some(b) = args[0].as_bool() {
            return Ok(ValueWord::from_string(Arc::new(format!("{}", b))));
        }
        if args[0].is_none() {
            return Ok(ValueWord::from_string(Arc::new("none".to_string())));
        }
        if let Some(s) = args[0].as_str() {
            return Ok(ValueWord::from_string(Arc::new(s.to_string())));
        }
        // Fallback: format via ValueWord-native formatter (no ValueWord bridge).
        Ok(ValueWord::from_string(Arc::new(
            self.format_value_default_nb(&args[0]),
        )))
    }

    /// ToNumber: Convert value to number
    pub(in crate::executor) fn builtin_to_number(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("to_number", &args, 1)?;

        // Fast path: already numeric
        if let Some(n) = args[0].as_number_coerce() {
            return Ok(ValueWord::from_f64(n));
        }
        // Bool fast path
        if let Some(b) = args[0].as_bool() {
            return Ok(ValueWord::from_f64(if b { 1.0 } else { 0.0 }));
        }
        // String fast path
        if let Some(s) = args[0].as_str() {
            let n = s.parse::<f64>().map_err(|_| VMError::InvalidArgument {
                function: "to_number".to_string(),
                message: format!("cannot convert string '{}' to number", s),
            })?;
            return Ok(ValueWord::from_f64(n));
        }
        // Fallback for other types
        Err(VMError::TypeError {
            expected: "number, bool, or string",
            got: args[0].type_name(),
        })
    }

    /// ToBool: Convert value to boolean
    #[inline]
    pub(in crate::executor) fn builtin_to_bool(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("to_bool", &args, 1)?;
        Ok(ValueWord::from_bool(args[0].is_truthy()))
    }

    /// TypeOf: Get a first-class `Type` value for a runtime value.
    #[inline]
    pub(in crate::executor) fn builtin_type_of(
        &mut self,
        _args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        // Note: TypeOf uses self.pop_vw() directly, not args
        let nb = self.pop_vw()?;
        let annotation = self.type_annotation_for_nb(&nb);
        Ok(ValueWord::from_type_annotation(annotation))
    }

    /// SomeCtor: Option::Some constructor
    #[inline]
    pub(in crate::executor) fn builtin_some_ctor(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("Some", &args, 1)?;
        Ok(args[0].clone())
    }

    /// OkCtor: Result::Ok constructor
    #[inline]
    pub(in crate::executor) fn builtin_ok_ctor(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("Ok", &args, 1)?;
        Ok(ValueWord::from_ok(args[0].clone()))
    }

    /// ErrCtor: Result::Err constructor
    ///
    /// Stores the raw payload directly — AnyError normalization is deferred to
    /// error propagation sites (try operator, exception handling) so that
    /// `as_err_inner()` returns the original value.
    #[inline]
    pub(in crate::executor) fn builtin_err_ctor(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        check_arity("Err", &args, 1)?;
        Ok(ValueWord::from_err(args[0].clone()))
    }
}
