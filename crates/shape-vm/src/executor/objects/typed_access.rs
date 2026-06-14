//! Typed HashMap and String access opcodes — local-slot based, skip HeapValue dispatch.
//!
//! These handlers operate on HashMap / String values stored in local variable
//! slots, accessed via `Operand::Local(slot)`. The key/index comes from the
//! stack. This avoids the full `GetProp` / `CallMethod` dispatch overhead for
//! statically-typed access patterns the compiler can prove.
//!
//! ADR-006 §2.7.6/§2.7.7 / Wave 6.5 sub-cluster D-typed-access: kinded API.
//! The receiver lives in a local slot with kind sourced from `self.kinds[idx]`
//! in lockstep (Q9). Heap dispatch goes through `slot.as_heap_value()` +
//! `HeapValue::*` match per Q8 — no `tag_bits::*`, no `ValueWord` decode, no
//! `raw_helpers::extract_*` tag-probing.
//!
//! Opcodes that the v2 storage model has not yet rewired are surfaced as
//! `NotImplemented(SURFACE: ...)` per playbook §7 REVISED — the agent's
//! mandate is to migrate or surface, never to keep the error count down by
//! reintroducing forbidden patterns.

use crate::bytecode::{Instruction, OpCode, Operand};
use crate::executor::VirtualMachine;
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::heap_value::{HeapKind, HeapValue};
use shape_value::v2::string_obj::StringObj;
use shape_value::{NativeKind, VMError};
use std::sync::Arc;

/// True iff `kind` is one of the three string carriers the compiler may
/// stamp on a slot whose static type is `string`:
///
/// - `NativeKind::String` / `NativeKind::Ptr(HeapKind::String)` — the
///   `Arc<String>` carrier (scalar string locals, string literals).
/// - `NativeKind::StringV2` — the v2-raw `*const StringObj` carrier
///   produced when a `string` is read out of an `Array<string>` element
///   slot (`v2_array_detect::load_elem` → `NativeKind::StringV2`) or any
///   other v2-raw string producer.
///
/// All three are statically `string`. This predicate does NOT widen the
/// `+` operator: the compiler only reaches `StringConcatTyped` /
/// `StringConcat*` after proving both operands are `string` (see
/// `binary_ops.rs:1262`). The fix is purely a runtime carrier-recognition
/// gap — the `StringV2` carrier was missing from the concat handlers'
/// accepted set even though the compiler had already proven `string`.
#[inline]
fn is_string_carrier(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::String | NativeKind::Ptr(HeapKind::String) | NativeKind::StringV2
    )
}

/// Borrow a string operand from any of the three string carriers and copy
/// its bytes into an owned `String`. Does NOT consume the strong-count
/// share — the caller still owns the share that `pop_kinded` transferred
/// and MUST release it via `drop_with_kind(bits, kind)` (which already
/// handles all three carriers). Returns `None` for any non-string kind.
///
/// SAFETY contract: when `kind` is a string carrier, `bits` is the raw
/// pointer the matching producer stamped (`Arc::into_raw::<String>` for
/// the Arc carriers, `*const StringObj` for `StringV2`).
#[inline]
fn read_string_operand(bits: u64, kind: NativeKind) -> Option<String> {
    match kind {
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            // Borrow the inner `String` without reconstructing/consuming
            // the Arc (the caller owns the share and drops it later).
            let s = unsafe { &*(bits as *const String) };
            Some(s.clone())
        }
        NativeKind::StringV2 => {
            // Borrow the v2-raw StringObj's UTF-8 bytes; copy into an owned
            // String. The caller's drop_with_kind releases the share.
            let s = unsafe { StringObj::as_str(bits as *const StringObj) };
            Some(s.to_string())
        }
        _ => None,
    }
}

impl VirtualMachine {
    // =====================================================================
    // Typed HashMap access (local-slot based)
    // =====================================================================

    /// Dispatch for typed HashMap access opcodes (MapGetStrI64, MapGetStrF64,
    /// MapSetStrI64, MapHasStr, MapLenTyped).
    pub(in crate::executor) fn exec_typed_map_access(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::MapGetStrI64 => self.op_map_get_str_i64(instruction),
            OpCode::MapGetStrF64 => self.op_map_get_str_f64(instruction),
            OpCode::MapSetStrI64 => self.op_map_set_str_i64(instruction),
            OpCode::MapHasStr => self.op_map_has_str(instruction),
            OpCode::MapLenTyped => self.op_map_len_typed(instruction),
            _ => unreachable!(
                "exec_typed_map_access called with non-map opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// Helper: read the local slot index from the instruction operand.
    #[inline(always)]
    fn extract_local_slot(instruction: &Instruction) -> Result<u16, VMError> {
        match instruction.operand {
            Some(Operand::Local(idx)) => Ok(idx),
            _ => Err(VMError::InvalidOperand),
        }
    }

    /// Pop the topmost slot and require kind == `NativeKind::String`. Returns
    /// the borrowed `&str` via a closure (lifetime is bounded by the popped
    /// `Arc<String>` share, which the closure may not retain). The popped
    /// share is retired via `drop_with_kind` after the closure returns.
    ///
    /// Used at every site that previously pulled a string key off the stack
    /// via `raw_helpers::extract_str` (a forbidden tag-decoding probe).
    #[inline]
    fn pop_string_key<R>(
        &mut self,
        f: impl FnOnce(&str) -> R,
    ) -> Result<Result<R, VMError>, VMError> {
        let (key_bits, key_kind) = self.pop_kinded()?;
        match key_kind {
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                // SAFETY: kind == String means bits = `Arc::into_raw::<String>`,
                // and pop_kinded transferred the share to us. Reconstruct the
                // Arc, borrow `&str` through the closure, then retire.
                let arc: Arc<String> = unsafe { Arc::from_raw(key_bits as *const String) };
                let result = f(arc.as_str());
                drop(arc);
                Ok(Ok(result))
            }
            _ => {
                drop_with_kind(key_bits, key_kind);
                Ok(Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(key_kind),
                }))
            }
        }
    }

    /// Borrow the receiver slot at `slot_idx` as `&HashMapKindedRef`,
    /// requiring kind == `Ptr(HeapKind::HashMap)`. Returns the borrowed
    /// kinded ref (the slot retains its share — no refcount change).
    ///
    /// **Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):** signature flipped
    /// from `&HashMapData` to `&HashMapKindedRef` per ADR-006 §2.7.24
    /// Q25.B SUPERSEDED.
    #[inline]
    fn borrow_hashmap_slot(
        &self,
        slot_idx: u16,
    ) -> Result<&shape_value::heap_value::HashMapKindedRef, VMError> {
        let bp = self.current_locals_base();
        let (bits, kind) = self.stack_read_kinded_raw(bp + slot_idx as usize);
        match kind {
            NativeKind::Ptr(HeapKind::HashMap) => {
                if bits == 0 {
                    return Err(VMError::TypeError {
                        expected: "HashMap",
                        got: "null",
                    });
                }
                // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): bits are
                // `Arc::into_raw(Arc<HashMapKindedRef>)`. SAFETY: slot
                // owns one outer Arc share — borrow through the live
                // outer Arc's payload.
                let arc_ptr = bits as *const shape_value::heap_value::HashMapKindedRef;
                Ok(unsafe { &*arc_ptr })
            }
            _ => Err(VMError::TypeError {
                expected: "HashMap",
                got: kind_type_name(kind),
            }),
        }
    }

    /// Borrow the receiver slot at `slot_idx` as `&str`, requiring the slot's
    /// kind to be `String` or `Ptr(HeapKind::String)`. The slot retains its
    /// share (no refcount change).
    #[inline]
    fn borrow_string_slot(&self, slot_idx: u16) -> Result<&str, VMError> {
        let bp = self.current_locals_base();
        let (bits, kind) = self.stack_read_kinded_raw(bp + slot_idx as usize);
        match kind {
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                if bits == 0 {
                    return Err(VMError::TypeError {
                        expected: "string",
                        got: "null",
                    });
                }
                // SAFETY: kind == String means bits = `Arc::into_raw::<String>`
                // and the slot owns one strong share. Borrow `&str` through it.
                let s_ptr = bits as *const String;
                Ok(unsafe { (*s_ptr).as_str() })
            }
            _ => Err(VMError::TypeError {
                expected: "string",
                got: kind_type_name(kind),
            }),
        }
    }

    /// MapGetStrI64: get value from HashMap<string, int>. Key on stack, map in local slot.
    /// Pushes the value (int) or none if key not found.
    ///
    /// Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V dispatch.
    /// Receiver MUST be HashMapKindedRef::I64; mismatched V surfaces as a
    /// TypeError per playbook §6 (no fallback coercion). On miss, pushes
    /// `0i64` (the typed default for HashMap<string, int> per the typed
    /// fast-path's "no Option indirection at storage layer" invariant).
    fn op_map_get_str_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        let lookup = self.pop_string_key(|key_str| key_str.to_owned())??;

        let map = self.borrow_hashmap_slot(slot_idx)?;
        let value: i64 = match map {
            shape_value::heap_value::HashMapKindedRef::I64(arc) => {
                arc.get_share(&lookup).unwrap_or(0)
            }
            other => {
                return Err(VMError::TypeError {
                    expected: "HashMap<string, int>",
                    got: hashmap_v_kind_name(other),
                });
            }
        };
        self.push_kinded(value as u64, NativeKind::Int64)
    }

    /// MapGetStrF64: get value from HashMap<string, float>. Key on stack, map in local slot.
    /// Pushes the value (float) or none if key not found.
    ///
    /// Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V dispatch. Mirror
    /// of `op_map_get_str_i64`. On miss returns 0.0.
    fn op_map_get_str_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        let lookup = self.pop_string_key(|key_str| key_str.to_owned())??;

        let map = self.borrow_hashmap_slot(slot_idx)?;
        let value: f64 = match map {
            shape_value::heap_value::HashMapKindedRef::F64(arc) => {
                arc.get_share(&lookup).unwrap_or(0.0)
            }
            other => {
                return Err(VMError::TypeError {
                    expected: "HashMap<string, number>",
                    got: hashmap_v_kind_name(other),
                });
            }
        };
        self.push_kinded(value.to_bits(), NativeKind::Float64)
    }

    /// MapSetStrI64: set value in HashMap<string, int>. Key and value on stack, map in local slot.
    /// Mutates the map in-place (or clones on write).
    fn op_map_set_str_i64(&mut self, _instruction: &Instruction) -> Result<(), VMError> {
        // SURFACE: ADR-006 §2.7.4 — the v2 HashMapData (Arc<TypedBuffer<…>>)
        // dropped the legacy in-place mutation API (`as_hashmap_mut`,
        // `Arc::make_mut`-driven `keys.push` / `values.push` / shape-id
        // transition). Rewiring this against the buffer-based storage is a
        // phase-2c rewrite tracked alongside the homogeneous-typed HashMap
        // workstream. The opcode is currently unreachable from compiled code
        // pending that rewire; if it is emitted, return cleanly to the
        // caller rather than executing pre-§2.7.7 forbidden helpers.
        Err(VMError::NotImplemented(
            "MapSetStrI64: phase-2c — v2 HashMapData mutation API (Arc<TypedBuffer>) \
             awaits buffer-aware insert path. See ADR-006 §2.7.4."
                .into(),
        ))
    }

    /// MapHasStr: check if key exists in HashMap. Key on stack, map in local slot.
    /// Pushes bool.
    fn op_map_has_str(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        let lookup = self.pop_string_key(|key_str| key_str.to_owned())??;

        let map = self.borrow_hashmap_slot(slot_idx)?;
        let found = map.contains_key(&lookup);
        // Result kind is always Bool (playbook §2 comparison row).
        self.push_kinded(found as u64, NativeKind::Bool)
    }

    /// MapLenTyped: get HashMap length. Map in local slot. Pushes int.
    fn op_map_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let len = self.borrow_hashmap_slot(slot_idx)?.len();
        // Push native i64 with kind Int64 (playbook §2 — opcode-suffix
        // selects result kind; "Typed" length opcode → native int).
        self.push_kinded(len as u64, NativeKind::Int64)
    }

    // =====================================================================
    // Typed String access (local-slot based or stack-based)
    // =====================================================================

    /// Dispatch for typed String access opcodes (StringLenTyped, StringCharAt,
    /// StringConcatTyped, and R5.5's StringConcat{Int,Number,Bool}).
    pub(in crate::executor) fn exec_typed_string_access(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::StringLenTyped => self.op_string_len_typed(instruction),
            OpCode::StringCharAt => self.op_string_char_at(instruction),
            OpCode::StringConcatTyped => self.op_string_concat_typed(),
            OpCode::StringConcatInt => self.op_string_concat_int(),
            OpCode::StringConcatNumber => self.op_string_concat_number(),
            OpCode::StringConcatBool => self.op_string_concat_bool(),
            _ => unreachable!(
                "exec_typed_string_access called with non-string opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// StringLenTyped: get string length (char count). String in local slot. Pushes int.
    fn op_string_len_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;
        let count = self.borrow_string_slot(slot_idx)?.chars().count();
        // Result kind: Int64 (typed length).
        self.push_kinded(count as u64, NativeKind::Int64)
    }

    /// StringCharAt: get char at index. Index on stack, string in local slot. Pushes char.
    fn op_string_char_at(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        // Pop index (kinded — must be Int64-family).
        let (index_bits, index_kind) = self.pop_kinded()?;
        let index = match index_kind {
            NativeKind::Int8
            | NativeKind::Int16
            | NativeKind::Int32
            | NativeKind::Int64
            | NativeKind::IntSize
            | NativeKind::UInt8
            | NativeKind::UInt16
            | NativeKind::UInt32
            | NativeKind::UInt64
            | NativeKind::UIntSize => index_bits as i64 as usize,
            _ => {
                drop_with_kind(index_bits, index_kind);
                return Err(VMError::TypeError {
                    expected: "int",
                    got: kind_type_name(index_kind),
                });
            }
        };
        // Inline scalars: drop is no-op, but stay symmetric with playbook §3.
        drop_with_kind(index_bits, index_kind);

        let s = self.borrow_string_slot(slot_idx)?;
        if let Some(ch) = s.chars().nth(index) {
            // Push as scalar `NativeKind::Char` slot — the codepoint is an
            // inline 4-byte scalar (UTF-32), NOT an `Arc<HeapValue>` pointer.
            // ADR-006 §2.7.5 producer-side stamp-at-compile-time: the prior
            // `Ptr(HeapKind::Char)` label mislabeled a scalar codepoint as a
            // heap pointer; consumers without a Char arm (e.g.
            // `wire_conversion::heap_to_wire`) would cast the codepoint bits
            // to `*const HeapValue` and dereference → misaligned-pointer
            // abort. `NativeKind::Char` is the canonical post-amendment
            // scalar kind (mirror of `v2_string_char_at` / `KindedSlot::from_char`).
            self.push_kinded(ch as u64, NativeKind::Char)
        } else {
            // Out-of-bounds index pushes the NUL char, mirroring the
            // `v2_string_char_at` handler contract (string_methods.rs:271):
            // both negative and past-end indices return '\0'. (A negative
            // `index: int` wraps to a huge `usize` above via `as usize`, so
            // it also lands here.) This keeps the typed StringCharAt fast-path
            // semantically identical to the PHF-dispatched form rather than
            // raising IndexOutOfBounds for what the language defines as run-ok.
            self.push_kinded('\0' as u64, NativeKind::Char)
        }
    }

    /// StringConcatTyped: concatenate two strings from the stack. Pushes result string.
    fn op_string_concat_typed(&mut self) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        // Accept all three string carriers (Arc<String> and v2-raw StringV2).
        // `read_string_operand` borrows (no consume); the popped shares are
        // released via drop_with_kind on both success and error paths.
        let result = match (
            read_string_operand(a_bits, a_kind),
            read_string_operand(b_bits, b_kind),
        ) {
            (Some(a), Some(b)) => Ok(format!("{}{}", a, b)),
            _ => Err(VMError::TypeError {
                expected: "string",
                got: kind_type_name(if !is_string_carrier(a_kind) {
                    a_kind
                } else {
                    b_kind
                }),
            }),
        };
        // pop_kinded transferred a share for each heap-bearing operand; release
        // both regardless of success/error now that the bytes are copied.
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        let result = result?;
        let bits = Arc::into_raw(Arc::new(result)) as u64;
        self.push_kinded(bits, NativeKind::String)
    }

    // ===== R5.5: String + scalar concat =====
    //
    // Typed siblings of the dynamic `AddDynamic` handler's "string + scalar"
    // branch (see `try_heap_arithmetic` Case 2 at arithmetic/mod.rs). Semantics
    // are preserved byte-for-byte for `int` and `number`. The `bool` variant is
    // new (the pre-R5.5 fallback coerced bool via `as_f64` and produced a
    // garbage numeric tail; R5.5 emits the canonical `"true"`/`"false"`
    // textual form — see R5.5 commit body).
    //
    // All three opcodes pop (string, scalar) with the string produced first
    // by the compiler (LHS), scalar second (RHS), matching the
    // `StringConcatTyped` convention: stack top = RHS.

    /// StringConcatInt: pop (string, i64 int), push `format!("{}{}", s, i)`.
    fn op_string_concat_int(&mut self) -> Result<(), VMError> {
        // Pop scalar (any int family).
        let (i_bits, i_kind) = self.pop_kinded()?;
        let i = match i_kind {
            NativeKind::Int8
            | NativeKind::Int16
            | NativeKind::Int32
            | NativeKind::Int64
            | NativeKind::IntSize
            | NativeKind::UInt8
            | NativeKind::UInt16
            | NativeKind::UInt32
            | NativeKind::UInt64
            | NativeKind::UIntSize => i_bits as i64,
            _ => {
                drop_with_kind(i_bits, i_kind);
                return Err(VMError::TypeError {
                    expected: "int",
                    got: kind_type_name(i_kind),
                });
            }
        };
        // Inline scalar — drop is a no-op but stays symmetric.
        drop_with_kind(i_bits, i_kind);

        // Pop string (any string carrier — Arc<String> or v2-raw StringV2).
        let (s_bits, s_kind) = self.pop_kinded()?;
        let s = match read_string_operand(s_bits, s_kind) {
            Some(s) => s,
            None => {
                drop_with_kind(s_bits, s_kind);
                return Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(s_kind),
                });
            }
        };
        // read_string_operand borrowed (did not consume) — release the share.
        drop_with_kind(s_bits, s_kind);
        let result = format!("{}{}", s, i);
        let bits = Arc::into_raw(Arc::new(result)) as u64;
        self.push_kinded(bits, NativeKind::String)
    }

    /// StringConcatNumber: pop (string, raw f64), push formatted concat.
    /// Mirrors the legacy fallback's integer-fast-path: whole-valued floats
    /// render without a decimal (e.g. `2.0` → `"2"`); other values use the
    /// default `{}` format for f64.
    fn op_string_concat_number(&mut self) -> Result<(), VMError> {
        let (n_bits, n_kind) = self.pop_kinded()?;
        let n = match n_kind {
            NativeKind::Float64 | NativeKind::NullableFloat64 => f64::from_bits(n_bits),
            _ => {
                drop_with_kind(n_bits, n_kind);
                return Err(VMError::TypeError {
                    expected: "number",
                    got: kind_type_name(n_kind),
                });
            }
        };
        drop_with_kind(n_bits, n_kind);

        let (s_bits, s_kind) = self.pop_kinded()?;
        let s = match read_string_operand(s_bits, s_kind) {
            Some(s) => s,
            None => {
                drop_with_kind(s_bits, s_kind);
                return Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(s_kind),
                });
            }
        };
        drop_with_kind(s_bits, s_kind);
        let n_str = if n.fract() == 0.0 && n.is_finite() {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        };
        let result = format!("{}{}", s, n_str);
        let bits = Arc::into_raw(Arc::new(result)) as u64;
        self.push_kinded(bits, NativeKind::String)
    }

    /// StringConcatBool: pop (string, bool), push `format!("{}{}", s, b)`
    /// where `b` renders as `"true"` / `"false"`.
    fn op_string_concat_bool(&mut self) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let b = match b_kind {
            NativeKind::Bool => b_bits != 0,
            _ => {
                drop_with_kind(b_bits, b_kind);
                return Err(VMError::TypeError {
                    expected: "bool",
                    got: kind_type_name(b_kind),
                });
            }
        };
        drop_with_kind(b_bits, b_kind);

        let (s_bits, s_kind) = self.pop_kinded()?;
        let s = match read_string_operand(s_bits, s_kind) {
            Some(s) => s,
            None => {
                drop_with_kind(s_bits, s_kind);
                return Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(s_kind),
                });
            }
        };
        drop_with_kind(s_bits, s_kind);
        let result = format!("{}{}", s, b);
        let bits = Arc::into_raw(Arc::new(result)) as u64;
        self.push_kinded(bits, NativeKind::String)
    }
}

/// Static name for a `NativeKind` for use in `VMError::TypeError`.
/// Local helper to avoid `raw_helpers::type_name_from_bits` (which probes
/// `tag_bits::*`, a §2.7.7 forbidden tag-decoding probe).
#[inline]
fn kind_type_name(kind: NativeKind) -> &'static str {
    match kind {
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): canonical absence-of-value discriminator.
        NativeKind::Null => "null",
        NativeKind::Bool => "bool",
        NativeKind::Float64 | NativeKind::NullableFloat64 => "number",
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment.
        NativeKind::Float32 => "f32",
        NativeKind::Char => "char",
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14): same surface as Arc-wrapped siblings.
        NativeKind::StringV2 => "string",
        NativeKind::DecimalV2 => "decimal",
        NativeKind::Int8
        | NativeKind::NullableInt8
        | NativeKind::Int16
        | NativeKind::NullableInt16
        | NativeKind::Int32
        | NativeKind::NullableInt32
        | NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize
        | NativeKind::UInt8
        | NativeKind::NullableUInt8
        | NativeKind::UInt16
        | NativeKind::NullableUInt16
        | NativeKind::UInt32
        | NativeKind::NullableUInt32
        | NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => "int",
        NativeKind::String => "string",
        NativeKind::Ptr(HeapKind::String) => "string",
        NativeKind::Ptr(HeapKind::TypedArray) => "array",
        NativeKind::Ptr(HeapKind::TypedObject) => "object",
        NativeKind::Ptr(HeapKind::HashMap) => "hashmap",
        NativeKind::Ptr(HeapKind::Decimal) => "decimal",
        NativeKind::Ptr(HeapKind::BigInt) => "int",
        NativeKind::Ptr(HeapKind::DataTable) => "datatable",
        NativeKind::Ptr(HeapKind::IoHandle) => "io_handle",
        NativeKind::Ptr(HeapKind::NativeView) => "native_view",
        NativeKind::Ptr(HeapKind::Content) => "content",
        NativeKind::Ptr(HeapKind::Instant) => "instant",
        NativeKind::Ptr(HeapKind::Temporal) => "temporal",
        NativeKind::Ptr(HeapKind::TableView) => "table_view",
        NativeKind::Ptr(HeapKind::TaskGroup) => "task_group",
        NativeKind::Ptr(HeapKind::Char) => "char",
        NativeKind::Ptr(HeapKind::Closure) => "closure",
        NativeKind::Ptr(HeapKind::Future) => "future",
        NativeKind::Ptr(HeapKind::NativeScalar) => "native_scalar",
        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment).
        NativeKind::Ptr(HeapKind::FilterExpr) => "filter_expr",
        // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26).
        NativeKind::Ptr(HeapKind::Reference) => "ref",
        // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment, 2026-05-10).
        NativeKind::Ptr(HeapKind::SharedCell) => "shared_cell",
        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16, 2026-05-10).
        NativeKind::Ptr(HeapKind::HashSet) => "set",
        // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10).
        NativeKind::Ptr(HeapKind::Iterator) => "iterator",
        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10).
        NativeKind::Ptr(HeapKind::Deque) => "deque",
        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21, 2026-05-10).
        NativeKind::Ptr(HeapKind::Channel) => "channel",
        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19, 2026-05-10).
        NativeKind::Ptr(HeapKind::PriorityQueue) => "priority_queue",
        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10).
        NativeKind::Ptr(HeapKind::Range) => "range",
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10).
        NativeKind::Ptr(HeapKind::Result) => "result",
        NativeKind::Ptr(HeapKind::Option) => "option",
        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11).
        NativeKind::Ptr(HeapKind::Mutex) => "mutex",
        NativeKind::Ptr(HeapKind::Atomic) => "atomic",
        NativeKind::Ptr(HeapKind::Lazy) => "lazy",
        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11).
        NativeKind::Ptr(HeapKind::TraitObject) => "trait_object",
        // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12).
        NativeKind::Ptr(HeapKind::ModuleFn) => "module_fn",
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13).
        NativeKind::Ptr(HeapKind::Matrix) => "matrix",
        NativeKind::Ptr(HeapKind::MatrixSlice) => "matrix_slice",
    }
}

// Suppress dead-import warning when no test arms use `HeapValue` directly.
#[allow(dead_code)]
fn _heap_value_marker(_: &HeapValue) {}

/// Static name for a `HashMapKindedRef` variant (the V discriminator) for
/// use in `VMError::TypeError`. Wave 2 Round 3b C2-joint ckpt-4
/// (2026-05-14). ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4.
#[inline]
fn hashmap_v_kind_name(kref: &shape_value::heap_value::HashMapKindedRef) -> &'static str {
    use shape_value::heap_value::HashMapKindedRef;
    match kref {
        HashMapKindedRef::I64(_) => "HashMap<string, int>",
        HashMapKindedRef::F64(_) => "HashMap<string, number>",
        HashMapKindedRef::Bool(_) => "HashMap<string, bool>",
        HashMapKindedRef::Char(_) => "HashMap<string, char>",
        HashMapKindedRef::String(_) => "HashMap<string, string>",
        HashMapKindedRef::Decimal(_) => "HashMap<string, decimal>",
        HashMapKindedRef::TypedObject(_) => "HashMap<string, TypedObject>",
        HashMapKindedRef::TraitObject(_) => "HashMap<string, TraitObject>",
        HashMapKindedRef::HashMap(_) => "HashMap<string, HashMap>",
    }
}

#[cfg(test)]
mod tests {
    // ADR-006 §2.7.4: the historical tests in this file relied on the deleted
    // `ValueWord` constructors and the legacy `Box<HeapValue>` HashMap shape
    // (`HashMap(Box<HashMapData>)` with a now-removed `shape_id` field plus
    // `make_str_int_map` helpers that constructed inline `i64` values).
    // Those construction shapes are forbidden post-§2.7.7 and were stood down.
    //
    // The tests below are kind-API-clean bytecode-level coverage for
    // `op_string_char_at` (β-fix CKPT-A char-carrier): they assert the
    // operation stamps the canonical scalar `NativeKind::Char` on its result
    // slot — NOT `NativeKind::Ptr(HeapKind::Char)`, which mislabeled a scalar
    // codepoint as an `Arc<HeapValue>` pointer and aborted the process when
    // the slot reached `wire_conversion::heap_to_wire` as a program return
    // value (misaligned-pointer non-unwinding panic).

    use crate::executor::tests::test_utils::eval_with_prelude;
    use shape_value::NativeKind;

    /// `charAt` result slot carries the scalar `NativeKind::Char` kind,
    /// not the pre-fix `Ptr(HeapKind::Char)` heap-pointer mislabel.
    #[test]
    fn char_at_result_slot_is_scalar_char_kind() {
        let slot = eval_with_prelude(r#""abc".charAt(0)"#);
        assert_eq!(
            slot.kind,
            NativeKind::Char,
            "charAt must stamp scalar NativeKind::Char, got {:?}",
            slot.kind
        );
        assert_eq!(slot.as_char(), Some('a'));
    }

    /// `reverse().charAt(0)` — the exact SIGABRT reproducer chain. The
    /// reversed string is "cba", so char 0 is 'c'. The result slot must
    /// carry the scalar `NativeKind::Char` kind.
    #[test]
    fn reverse_then_char_at_result_slot_is_scalar_char_kind() {
        let slot = eval_with_prelude(r#""abc".reverse().charAt(0)"#);
        assert_eq!(
            slot.kind,
            NativeKind::Char,
            "reverse().charAt must stamp scalar NativeKind::Char, got {:?}",
            slot.kind
        );
        assert_eq!(slot.as_char(), Some('c'));
    }

    /// Non-ASCII multi-byte codepoint flows through `charAt` as a scalar
    /// `NativeKind::Char` slot with its codepoint intact.
    #[test]
    fn char_at_unicode_codepoint_is_scalar_char_kind() {
        let slot = eval_with_prelude(r#""λxy".charAt(0)"#);
        assert_eq!(slot.kind, NativeKind::Char);
        assert_eq!(slot.as_char(), Some('λ'));
    }

    // ── R4-stringiter: StringV2 carrier through StringConcatTyped ────────
    //
    // `Array<string>` elements load with `NativeKind::StringV2` (the v2-raw
    // `*const StringObj` carrier — see `v2_array_detect::load_elem`), NOT the
    // `Arc<String>` carrier. Pre-fix, `op_string_concat_typed` only accepted
    // `NativeKind::String` / `Ptr(HeapKind::String)`, so iterating a
    // string-array and concatenating the bound element produced a runtime
    // "TypeError: expected string, got string" (both ARE string; the
    // StringV2 carrier was the unrecognized one). The fix recognizes all
    // three string carriers in the concat handlers — a runtime
    // carrier-recognition extension, NOT a widening of `+` (the compiler
    // still proves both operands `string` before emitting StringConcatTyped).

    /// for-loop over a string-array literal, concatenating the bound element:
    /// the element slot's StringV2 carrier must be accepted by StringConcatTyped.
    #[test]
    fn for_loop_string_array_concat_accepts_stringv2_element() {
        let slot = eval_with_prelude(
            r#"
let mut result = ""
for s in ["a", "b", "c"] {
    result = result + s
}
result
"#,
        );
        assert_eq!(slot.as_str(), Some("abc"));
    }

    /// reduce-concat over a string array: accumulator (Arc<String>) + element
    /// (StringV2) through StringConcatTyped.
    #[test]
    fn reduce_concat_string_array_accepts_stringv2_element() {
        let slot = eval_with_prelude(
            r#"
let words = ["foo", "bar", "baz"]
words.reduce(|acc, w| acc + w, "")
"#,
        );
        assert_eq!(slot.as_str(), Some("foobarbaz"));
    }
}
