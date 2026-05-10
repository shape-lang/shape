//! Type checking and conversion builtin implementations.
//!
//! W9-builtins-type-ops (`docs/cluster-audits/wave-9-method-refill-playbook.md`
//! row D-builtins-type-ops): kind-narrowed cast bodies for the
//! `ConvertTo*` / `TryConvertTo*` opcode family. Source kind comes from
//! the §2.7.7 stack parallel-kind track via `pop_kinded()`; result kind
//! is fixed by the opcode's target. Bodies dispatch per the §2.7.6 / Q8
//! heterogeneous-kind body pattern — `match args[0].kind` at the call
//! site, no decode-from-bits, no `is_heap()` probe.
//!
//! These opcodes are emitted by the compiler at
//! `compiler/expressions/type_ops.rs:607-631` after `validate_infallible_cast`
//! / `validate_fallible_cast` has confirmed an `Into<T>` / `TryInto<T>`
//! impl for the source/target pair (or accepted an identity cast). Per
//! the stdlib's `core/into.shape` + `core/try_into.shape`, the proven
//! source kinds for each opcode are all primitive scalars (`Int*`,
//! `Float64`, `Bool`, `String`, `Char`) plus the heap-backed numeric
//! family (`Decimal`, `BigInt`). The bodies enumerate that proven set
//! and surface `VMError::RuntimeError` for any unproven source — never
//! a Bool-default fallback (forbidden by the playbook's defection-
//! attractor list).
//!
//! `op_convert` (the `Convert` opcode without a static target selector)
//! still surfaces to Phase-2c: it dispatches on a `TypeAnnotation`
//! operand and routes through the trait-dispatch machinery + AnyError
//! TypedObject construction (see `executor/exceptions/mod.rs`'s
//! `build_any_error` Phase-2c surface).

use crate::bytecode::{Constant, Instruction, Operand};
use crate::executor::VirtualMachine;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};
use std::sync::Arc;

/// Read a `KindedSlot` carrier as i64, dispatching on `slot.kind` per
/// §2.7.6 / Q8. Returns `Err` when the kind is not a proven `as int`
/// source.
fn read_as_i64(slot: &KindedSlot) -> Result<i64, VMError> {
    match slot.kind {
        NativeKind::Bool => Ok(if slot.slot.as_bool() { 1 } else { 0 }),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => Ok(slot.slot.as_i64()),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => Ok(slot.slot.as_u64() as i64),
        NativeKind::Float64 | NativeKind::NullableFloat64 => {
            let n = slot.slot.as_f64();
            if !n.is_finite() {
                return Err(VMError::RuntimeError(
                    "cannot convert non-finite number to int".to_string(),
                ));
            }
            let i = n as i64;
            if (i as f64 - n).abs() > f64::EPSILON {
                return Err(VMError::RuntimeError(format!(
                    "cannot convert non-integer number '{n}' to int"
                )));
            }
            Ok(i)
        }
        NativeKind::String => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "cannot convert null string to int".to_string(),
                ));
            }
            // SAFETY: `NativeKind::String` means the slot bits are
            // `Arc::into_raw::<String>` and the carrier owns one share.
            let s: &String = unsafe { &*(bits as *const String) };
            s.parse::<i64>().map_err(|_| {
                VMError::RuntimeError(format!("cannot convert string '{s}' to int"))
            })
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(Decimal)` bits are `Arc::into_raw::<Decimal>`.
            let d: &rust_decimal::Decimal =
                unsafe { &*(bits as *const rust_decimal::Decimal) };
            use rust_decimal::prelude::ToPrimitive;
            d.to_i64().ok_or_else(|| {
                VMError::RuntimeError(format!("cannot convert decimal '{d}' to int"))
            })
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(BigInt)` bits are `Arc::into_raw::<i64>`.
            let i: &i64 = unsafe { &*(bits as *const i64) };
            Ok(*i)
        }
        NativeKind::Ptr(HeapKind::Char) => {
            // `Char`-kind stores codepoint bits inline (no Arc<T>).
            Ok(slot.slot.raw() as i64)
        }
        _ => Err(VMError::RuntimeError(format!(
            "cannot convert kind {:?} to int",
            slot.kind
        ))),
    }
}

/// Read a `KindedSlot` carrier as f64, dispatching on `slot.kind` per
/// §2.7.6 / Q8.
fn read_as_f64(slot: &KindedSlot) -> Result<f64, VMError> {
    match slot.kind {
        NativeKind::Bool => Ok(if slot.slot.as_bool() { 1.0 } else { 0.0 }),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => Ok(slot.slot.as_i64() as f64),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => Ok(slot.slot.as_u64() as f64),
        NativeKind::Float64 | NativeKind::NullableFloat64 => Ok(slot.slot.as_f64()),
        NativeKind::String => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "cannot convert null string to number".to_string(),
                ));
            }
            // SAFETY: `NativeKind::String` => `Arc::into_raw::<String>` bits.
            let s: &String = unsafe { &*(bits as *const String) };
            s.parse::<f64>().map_err(|_| {
                VMError::RuntimeError(format!("cannot convert string '{s}' to number"))
            })
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(Decimal)` => `Arc::into_raw::<Decimal>` bits.
            let d: &rust_decimal::Decimal =
                unsafe { &*(bits as *const rust_decimal::Decimal) };
            use rust_decimal::prelude::ToPrimitive;
            d.to_f64().ok_or_else(|| {
                VMError::RuntimeError(format!("cannot convert decimal '{d}' to number"))
            })
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(BigInt)` => `Arc::into_raw::<i64>` bits.
            let i: &i64 = unsafe { &*(bits as *const i64) };
            Ok(*i as f64)
        }
        _ => Err(VMError::RuntimeError(format!(
            "cannot convert kind {:?} to number",
            slot.kind
        ))),
    }
}

/// Read a `KindedSlot` carrier as bool, dispatching on `slot.kind`.
fn read_as_bool(slot: &KindedSlot) -> Result<bool, VMError> {
    match slot.kind {
        NativeKind::Bool => Ok(slot.slot.as_bool()),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => Ok(slot.slot.as_i64() != 0),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => Ok(slot.slot.as_u64() != 0),
        NativeKind::Float64 | NativeKind::NullableFloat64 => Ok(slot.slot.as_f64() != 0.0),
        NativeKind::String => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "cannot convert null string to bool".to_string(),
                ));
            }
            // SAFETY: see `read_as_i64` String arm.
            let s: &String = unsafe { &*(bits as *const String) };
            match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(VMError::RuntimeError(format!(
                    "cannot convert string '{s}' to bool"
                ))),
            }
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(Decimal)` => `Arc::into_raw::<Decimal>` bits.
            let d: &rust_decimal::Decimal =
                unsafe { &*(bits as *const rust_decimal::Decimal) };
            Ok(!rust_decimal::prelude::Zero::is_zero(d))
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(BigInt)` => `Arc::into_raw::<i64>` bits.
            let i: &i64 = unsafe { &*(bits as *const i64) };
            Ok(*i != 0)
        }
        _ => Err(VMError::RuntimeError(format!(
            "cannot convert kind {:?} to bool",
            slot.kind
        ))),
    }
}

/// Read a `KindedSlot` carrier as a fresh `Arc<Decimal>`, dispatching
/// on `slot.kind`.
fn read_as_decimal(slot: &KindedSlot) -> Result<Arc<rust_decimal::Decimal>, VMError> {
    match slot.kind {
        NativeKind::Bool => Ok(Arc::new(rust_decimal::Decimal::from(
            if slot.slot.as_bool() { 1 } else { 0 },
        ))),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => {
            Ok(Arc::new(rust_decimal::Decimal::from(slot.slot.as_i64())))
        }
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => {
            Ok(Arc::new(rust_decimal::Decimal::from(slot.slot.as_u64())))
        }
        NativeKind::Float64 | NativeKind::NullableFloat64 => {
            let n = slot.slot.as_f64();
            rust_decimal::Decimal::from_f64_retain(n)
                .map(Arc::new)
                .ok_or_else(|| {
                    VMError::RuntimeError(format!("cannot convert number '{n}' to decimal"))
                })
        }
        NativeKind::String => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "cannot convert null string to decimal".to_string(),
                ));
            }
            // SAFETY: see `read_as_i64` String arm.
            let s: &String = unsafe { &*(bits as *const String) };
            s.parse::<rust_decimal::Decimal>()
                .map(Arc::new)
                .map_err(|_| {
                    VMError::RuntimeError(format!("cannot convert string '{s}' to decimal"))
                })
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(Decimal)` => `Arc::into_raw::<Decimal>` bits;
            // the carrier owns one share. Bump strong count for the
            // returned `Arc` (the carrier's `Drop` retires the original).
            unsafe {
                Arc::increment_strong_count(bits as *const rust_decimal::Decimal);
                Ok(Arc::from_raw(bits as *const rust_decimal::Decimal))
            }
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(BigInt)` => `Arc::into_raw::<i64>` bits.
            let i: &i64 = unsafe { &*(bits as *const i64) };
            Ok(Arc::new(rust_decimal::Decimal::from(*i)))
        }
        _ => Err(VMError::RuntimeError(format!(
            "cannot convert kind {:?} to decimal",
            slot.kind
        ))),
    }
}

/// Read a `KindedSlot` carrier as a `char`, dispatching on `slot.kind`.
fn read_as_char(slot: &KindedSlot) -> Result<char, VMError> {
    match slot.kind {
        NativeKind::Ptr(HeapKind::Char) => {
            char::from_u32(slot.slot.raw() as u32).ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "invalid Unicode code point: {}",
                    slot.slot.raw() as u32
                ))
            })
        }
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => {
            let i = slot.slot.as_i64();
            let code = i as u32;
            char::from_u32(code).ok_or_else(|| {
                VMError::RuntimeError(format!("invalid Unicode code point: {code}"))
            })
        }
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => {
            let code = slot.slot.as_u64() as u32;
            char::from_u32(code).ok_or_else(|| {
                VMError::RuntimeError(format!("invalid Unicode code point: {code}"))
            })
        }
        NativeKind::String => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "cannot convert null string to char".to_string(),
                ));
            }
            // SAFETY: see `read_as_i64` String arm.
            let s: &String = unsafe { &*(bits as *const String) };
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(c),
                _ => Err(VMError::RuntimeError(format!(
                    "cannot convert string '{s}' to char (must be single character)"
                ))),
            }
        }
        _ => Err(VMError::RuntimeError(format!(
            "cannot convert kind {:?} to char",
            slot.kind
        ))),
    }
}

/// Format a `KindedSlot` to a `String` for `as string` casts. Inline
/// scalars and the simple heap-numeric kinds (`Decimal`, `BigInt`) are
/// formatted in-place; identity (`String` source) clones the inner
/// `Arc<String>`'s payload.
fn read_as_string(slot: &KindedSlot) -> Result<String, VMError> {
    match slot.kind {
        NativeKind::Bool => Ok(slot.slot.as_bool().to_string()),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => Ok(slot.slot.as_i64().to_string()),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => Ok(slot.slot.as_u64().to_string()),
        NativeKind::Float64 | NativeKind::NullableFloat64 => Ok(slot.slot.as_f64().to_string()),
        NativeKind::String => {
            let bits = slot.slot.raw();
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "cannot convert null string to string".to_string(),
                ));
            }
            // SAFETY: see `read_as_i64` String arm.
            let s: &String = unsafe { &*(bits as *const String) };
            Ok(s.clone())
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(Decimal)` => `Arc::into_raw::<Decimal>` bits.
            let d: &rust_decimal::Decimal =
                unsafe { &*(bits as *const rust_decimal::Decimal) };
            Ok(d.to_string())
        }
        NativeKind::Ptr(HeapKind::BigInt) => {
            let bits = slot.slot.raw();
            // SAFETY: `Ptr(BigInt)` => `Arc::into_raw::<i64>` bits.
            let i: &i64 = unsafe { &*(bits as *const i64) };
            Ok(i.to_string())
        }
        NativeKind::Ptr(HeapKind::Char) => {
            // `Char`-kind stores codepoint bits inline.
            let code = slot.slot.raw() as u32;
            char::from_u32(code)
                .map(|c| c.to_string())
                .ok_or_else(|| VMError::RuntimeError(format!("invalid Unicode code point: {code}")))
        }
        _ => {
            // Heterogeneous heap kinds (TypedObject, TypedArray, HashMap,
            // Content, Temporal, …) need the full kinded formatter
            // (`executor/printing.rs::ValueFormatter`), which is itself
            // partially Phase-2c (TypedObject / HashMap / Temporal arms
            // surface to `todo!("phase-2c")`). Surface here so the gap
            // is visible at the cast site rather than panicking deep
            // inside the formatter.
            Err(VMError::NotImplemented(format!(
                "phase-2c — `as string` cast for kind {:?} depends on the \
                 ValueFormatter Phase-2c heap-arm rebuild \
                 (executor/printing.rs `format_heap_kind`). ADR-006 §2.7.4.",
                slot.kind
            )))
        }
    }
}

/// Helper used by every `op_convert*` / `op_try_convert*` body: pop
/// one kinded slot off the §2.7.7 stack parallel-kind track, wrap as
/// a `KindedSlot` carrier (transferring the heap share into the
/// carrier so it is retired by `Drop` at the end of the body).
#[inline]
fn pop_one_kinded(vm: &mut VirtualMachine) -> Result<KindedSlot, VMError> {
    let (bits, kind) = vm.pop_kinded()?;
    Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
}

impl VirtualMachine {
    /// `Convert` opcode: trait-dispatch driven cast through `Into<T>` /
    /// `TryInto<T>` impls (compiler emits this when the target is
    /// non-primitive — see `compiler/expressions/type_ops.rs:714`). The
    /// dispatch-annotation operand carries `(source, target)` selector
    /// names; the runtime resolves the matching `into()` / `tryInto()`
    /// trait method symbol and calls it.
    ///
    /// SURFACE (Phase-2c per ADR-006 §2.7.4): wiring this end-to-end
    /// requires (1) the kinded trait-dispatch resolution path
    /// (`lookup_trait_method_symbol` + a kinded `call_value_immediate_nb`
    /// shape on the resolved closure), and (2) the AnyError TypedObject
    /// builder for fallible-path failures (currently a Phase-2c
    /// surface in `executor/exceptions/mod.rs::build_any_error`). Until
    /// both land the dispatch shell stays a NotImplemented surface
    /// rather than reintroducing the deleted W-series dispatch
    /// pattern.
    pub(in crate::executor) fn op_convert(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // Drop the popped value carrier (kind-dispatched refcount retire
        // via `KindedSlot::Drop`) so the stack stays balanced even at the
        // surface boundary.
        let _carrier = pop_one_kinded(self)?;
        let target_desc = match instruction.operand {
            Some(Operand::Const(idx)) => match self.program.constants.get(idx as usize) {
                Some(Constant::TypeAnnotation(ann)) => format!("{:?}", ann),
                other => format!("{:?}", other),
            },
            other => format!("{:?}", other),
        };
        Err(VMError::NotImplemented(format!(
            "phase-2c — `Convert` opcode (TryInto/Into trait dispatch) \
             needs the kinded trait-method lookup + AnyError TypedObject \
             builder. ADR-006 §2.7.4. target_desc={}",
            target_desc
        )))
    }

    /// `ConvertToInt` (`expr as int`): pop one kinded slot, convert to
    /// `i64`, push as `NativeKind::Int64`. Source kinds are pre-proven
    /// by `validate_infallible_cast` (stdlib `Into<int>` impls, plus
    /// identity casts).
    #[inline]
    pub(in crate::executor) fn op_convert_to_int(&mut self) -> Result<(), VMError> {
        let src = pop_one_kinded(self)?;
        let i = read_as_i64(&src)?;
        // `src` drops here — kind-dispatched refcount retire for heap
        // sources (Decimal / BigInt / String).
        drop(src);
        self.push_kinded(i as u64, NativeKind::Int64)
    }

    /// `ConvertToNumber` (`expr as number`): pop, convert to `f64`,
    /// push as `NativeKind::Float64`.
    #[inline]
    pub(in crate::executor) fn op_convert_to_number(&mut self) -> Result<(), VMError> {
        let src = pop_one_kinded(self)?;
        let n = read_as_f64(&src)?;
        drop(src);
        self.push_kinded(n.to_bits(), NativeKind::Float64)
    }

    /// `ConvertToString` (`expr as string`): pop, format to `String`,
    /// push as a fresh `Arc<String>` with `NativeKind::String`.
    #[inline]
    pub(in crate::executor) fn op_convert_to_string(&mut self) -> Result<(), VMError> {
        let src = pop_one_kinded(self)?;
        let s = read_as_string(&src)?;
        drop(src);
        let arc = Arc::new(s);
        let bits = Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::String)
    }

    /// `ConvertToBool` (`expr as bool`): pop, convert to `bool`, push
    /// as `NativeKind::Bool`.
    #[inline]
    pub(in crate::executor) fn op_convert_to_bool(&mut self) -> Result<(), VMError> {
        let src = pop_one_kinded(self)?;
        let b = read_as_bool(&src)?;
        drop(src);
        self.push_kinded(if b { 1 } else { 0 }, NativeKind::Bool)
    }

    /// `ConvertToDecimal` (`expr as decimal`): pop, convert to
    /// `Arc<Decimal>`, push as `NativeKind::Ptr(HeapKind::Decimal)`.
    #[inline]
    pub(in crate::executor) fn op_convert_to_decimal(&mut self) -> Result<(), VMError> {
        let src = pop_one_kinded(self)?;
        let d = read_as_decimal(&src)?;
        drop(src);
        let bits = Arc::into_raw(d) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal))
    }

    /// `ConvertToChar` (`expr as char`): pop, convert to `char`, push
    /// as `NativeKind::Ptr(HeapKind::Char)` (codepoint bits inline).
    #[inline]
    pub(in crate::executor) fn op_convert_to_char(&mut self) -> Result<(), VMError> {
        let src = pop_one_kinded(self)?;
        let c = read_as_char(&src)?;
        drop(src);
        self.push_kinded(c as u64, NativeKind::Ptr(HeapKind::Char))
    }

    // ── TryConvertTo* family ─────────────────────────────────────────
    //
    // The fallible variants share the success path with their
    // infallible siblings — the compiler handles the `Result<T, E>` /
    // `Option<T>` shape externally (`emit_option_lift_fallible` /
    // `emit_result_lift_fallible` in `compiler/expressions/type_ops.rs`)
    // by wrapping with the `Ok` builtin and/or null-checking at the
    // call site. The opcode body itself produces the unwrapped
    // converted value; runtime conversion failures surface as
    // `VMError::RuntimeError` and propagate through the standard
    // exception handler. The pre-strict-typing AnyError-wrap path
    // (`build_any_error`) is itself a Phase-2c surface, but is not
    // needed here because the compiler emits the wrapping separately.

    /// `TryConvertToInt`: see `op_convert_to_int`.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_int(&mut self) -> Result<(), VMError> {
        self.op_convert_to_int()
    }

    /// `TryConvertToNumber`: see `op_convert_to_number`.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_number(&mut self) -> Result<(), VMError> {
        self.op_convert_to_number()
    }

    /// `TryConvertToString`: see `op_convert_to_string`.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_string(&mut self) -> Result<(), VMError> {
        self.op_convert_to_string()
    }

    /// `TryConvertToBool`: see `op_convert_to_bool`.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_bool(&mut self) -> Result<(), VMError> {
        self.op_convert_to_bool()
    }

    /// `TryConvertToDecimal`: see `op_convert_to_decimal`.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_decimal(&mut self) -> Result<(), VMError> {
        self.op_convert_to_decimal()
    }

    /// `TryConvertToChar`: see `op_convert_to_char`.
    #[inline]
    pub(in crate::executor) fn op_try_convert_to_char(&mut self) -> Result<(), VMError> {
        self.op_convert_to_char()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_as_i64_from_bool() {
        let s = KindedSlot::from_bool(true);
        assert_eq!(read_as_i64(&s).unwrap(), 1);
        let s = KindedSlot::from_bool(false);
        assert_eq!(read_as_i64(&s).unwrap(), 0);
    }

    #[test]
    fn read_as_i64_from_int_identity() {
        let s = KindedSlot::from_int(42);
        assert_eq!(read_as_i64(&s).unwrap(), 42);
    }

    #[test]
    fn read_as_i64_from_finite_float() {
        let s = KindedSlot::from_number(3.0);
        assert_eq!(read_as_i64(&s).unwrap(), 3);
    }

    #[test]
    fn read_as_i64_from_non_integer_float_errors() {
        let s = KindedSlot::from_number(3.14);
        assert!(read_as_i64(&s).is_err());
    }

    #[test]
    fn read_as_f64_from_int_widens() {
        let s = KindedSlot::from_int(7);
        assert_eq!(read_as_f64(&s).unwrap(), 7.0);
    }

    #[test]
    fn read_as_f64_from_bool() {
        let s = KindedSlot::from_bool(true);
        assert_eq!(read_as_f64(&s).unwrap(), 1.0);
    }

    #[test]
    fn read_as_bool_from_zero_int_is_false() {
        let s = KindedSlot::from_int(0);
        assert_eq!(read_as_bool(&s).unwrap(), false);
    }

    #[test]
    fn read_as_bool_from_nonzero_int_is_true() {
        let s = KindedSlot::from_int(7);
        assert_eq!(read_as_bool(&s).unwrap(), true);
    }

    #[test]
    fn read_as_string_from_int() {
        let s = KindedSlot::from_int(42);
        assert_eq!(read_as_string(&s).unwrap(), "42");
    }

    #[test]
    fn read_as_string_from_bool() {
        let s = KindedSlot::from_bool(true);
        assert_eq!(read_as_string(&s).unwrap(), "true");
    }

    #[test]
    fn read_as_decimal_from_int() {
        let s = KindedSlot::from_int(5);
        let d = read_as_decimal(&s).unwrap();
        assert_eq!(*d, rust_decimal::Decimal::from(5));
    }

    #[test]
    fn read_as_char_from_int() {
        let s = KindedSlot::from_int('A' as i64);
        assert_eq!(read_as_char(&s).unwrap(), 'A');
    }

    #[test]
    fn read_as_char_from_string_single() {
        use std::sync::Arc;
        let s = KindedSlot::from_string_arc(Arc::new("Z".to_string()));
        assert_eq!(read_as_char(&s).unwrap(), 'Z');
    }

    #[test]
    fn read_as_char_from_string_multi_errors() {
        use std::sync::Arc;
        let s = KindedSlot::from_string_arc(Arc::new("AB".to_string()));
        assert!(read_as_char(&s).is_err());
    }
}
