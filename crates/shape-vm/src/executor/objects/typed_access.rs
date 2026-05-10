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
use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;
use shape_value::heap_value::{HeapKind, HeapValue};
use shape_value::{NativeKind, VMError};
use std::sync::Arc;

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

    /// Borrow the receiver slot at `slot_idx` as `&HeapValue`, requiring kind
    /// == `Ptr(HeapKind::HashMap)`. Returns `&HashMapData` borrowed from the
    /// slot (the slot retains its share — no refcount change).
    #[inline]
    fn borrow_hashmap_slot(
        &self,
        slot_idx: u16,
    ) -> Result<&shape_value::heap_value::HashMapData, VMError> {
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
                // SAFETY: kind == Ptr(HashMap) means bits = `Arc::into_raw::<HashMapData>`
                // and the slot owns one strong share. Borrow through the live Arc.
                let arc_ptr = bits as *const shape_value::heap_value::HashMapData;
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
    fn op_map_get_str_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        // Pop the string key, look it up by string-borrowed slice.
        let lookup = self.pop_string_key(|key_str| {
            // Re-fetch the map borrow inside the closure — borrow_hashmap_slot
            // takes &self and the closure runs after pop_kinded mutated the
            // stack, so re-borrow here.
            // (We can't borrow `&self` outside and call `pop_string_key` with
            // `&mut self` simultaneously.)
            key_str.to_owned()
        })??;

        let map = self.borrow_hashmap_slot(slot_idx)?;
        match map.get(&lookup) {
            // SURFACE: ADR-006 §2.7.4 — the v2 HashMapData stores values as
            // `Arc<HeapValue>` (polymorphic). Pulling a typed `i64` out of an
            // `Arc<HeapValue>` requires the storage protocol to specialize
            // homogeneous-int maps to a `TypedBuffer<i64>` shape (cluster
            // E-builtins-backlog / Wave 5b). Until that lands, the typed-Get
            // fast path cannot satisfy the opcode contract.
            Some(_value_arc) => Err(VMError::NotImplemented(
                "MapGetStrI64: phase-2c — Arc<HeapValue> → typed i64 extraction \
                 awaits homogeneous-typed HashMap storage. See ADR-006 §2.7.4."
                    .into(),
            )),
            None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool),
        }
    }

    /// MapGetStrF64: get value from HashMap<string, float>. Key on stack, map in local slot.
    /// Pushes the value (float) or none if key not found.
    fn op_map_get_str_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let slot_idx = Self::extract_local_slot(instruction)?;

        let lookup = self.pop_string_key(|key_str| key_str.to_owned())??;

        let map = self.borrow_hashmap_slot(slot_idx)?;
        match map.get(&lookup) {
            // SURFACE: same shape as MapGetStrI64 — Arc<HeapValue> values
            // need a typed-extraction path. Phase-2c.
            Some(_value_arc) => Err(VMError::NotImplemented(
                "MapGetStrF64: phase-2c — Arc<HeapValue> → typed f64 extraction \
                 awaits homogeneous-typed HashMap storage. See ADR-006 §2.7.4."
                    .into(),
            )),
            None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool),
        }
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
            // Push as Char-kind slot (codepoint inline; HeapKind::Char dispatch
            // arm is no-op for Drop per kinded_slot.rs).
            self.push_kinded(ch as u64, NativeKind::Ptr(HeapKind::Char))
        } else {
            Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: s.chars().count(),
            })
        }
    }

    /// StringConcatTyped: concatenate two strings from the stack. Pushes result string.
    fn op_string_concat_typed(&mut self) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let result = match (a_kind, b_kind) {
            (
                NativeKind::String | NativeKind::Ptr(HeapKind::String),
                NativeKind::String | NativeKind::Ptr(HeapKind::String),
            ) => {
                // SAFETY: kind == String means bits = `Arc::into_raw::<String>` and
                // pop_kinded transferred ownership of one share each.
                let a_arc: Arc<String> = unsafe { Arc::from_raw(a_bits as *const String) };
                let b_arc: Arc<String> = unsafe { Arc::from_raw(b_bits as *const String) };
                let s = format!("{}{}", a_arc.as_str(), b_arc.as_str());
                drop(a_arc);
                drop(b_arc);
                Ok(s)
            }
            _ => Err(VMError::TypeError {
                expected: "string",
                got: kind_type_name(if !matches!(
                    a_kind,
                    NativeKind::String | NativeKind::Ptr(HeapKind::String)
                ) {
                    a_kind
                } else {
                    b_kind
                }),
            }),
        };
        // If types didn't match we still own (b_bits, b_kind) and (a_bits,
        // a_kind); release them via drop_with_kind on the error path.
        let result = match result {
            Ok(s) => s,
            Err(e) => {
                if matches!(
                    a_kind,
                    NativeKind::String | NativeKind::Ptr(HeapKind::String)
                ) {
                    drop_with_kind(a_bits, a_kind);
                }
                if matches!(
                    b_kind,
                    NativeKind::String | NativeKind::Ptr(HeapKind::String)
                ) {
                    drop_with_kind(b_bits, b_kind);
                }
                return Err(e);
            }
        };
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

        // Pop string.
        let (s_bits, s_kind) = self.pop_kinded()?;
        let s_arc: Arc<String> = match s_kind {
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                unsafe { Arc::from_raw(s_bits as *const String) }
            }
            _ => {
                drop_with_kind(s_bits, s_kind);
                return Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(s_kind),
                });
            }
        };
        let result = format!("{}{}", s_arc.as_str(), i);
        drop(s_arc);
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
        let s_arc: Arc<String> = match s_kind {
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                unsafe { Arc::from_raw(s_bits as *const String) }
            }
            _ => {
                drop_with_kind(s_bits, s_kind);
                return Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(s_kind),
                });
            }
        };
        let n_str = if n.fract() == 0.0 && n.is_finite() {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        };
        let result = format!("{}{}", s_arc.as_str(), n_str);
        drop(s_arc);
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
        let s_arc: Arc<String> = match s_kind {
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                unsafe { Arc::from_raw(s_bits as *const String) }
            }
            _ => {
                drop_with_kind(s_bits, s_kind);
                return Err(VMError::TypeError {
                    expected: "string",
                    got: kind_type_name(s_kind),
                });
            }
        };
        let result = format!("{}{}", s_arc.as_str(), b);
        drop(s_arc);
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
        NativeKind::Bool => "bool",
        NativeKind::Float64 | NativeKind::NullableFloat64 => "number",
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
    }
}

// Suppress dead-import warning when no test arms use `HeapValue` directly.
#[allow(dead_code)]
fn _heap_value_marker(_: &HeapValue) {}

#[cfg(test)]
mod tests {
    // ADR-006 §2.7.4: the existing tests in this file relied on the deleted
    // `ValueWord` constructors and the legacy `Box<HeapValue>` HashMap shape
    // (`HashMap(Box<HashMapData>)` with a now-removed `shape_id` field plus
    // `make_str_int_map` helpers that constructed inline `i64` values).
    //
    // Those construction shapes are forbidden post-§2.7.7. The migrated
    // operations here are exercised via the bytecode-level integration suites
    // (cluster E test files), not via unit-test harness cells that hand-build
    // ValueWord values. The tests therefore stand down at the kinded-API
    // boundary; reinstating equivalent unit coverage is a phase-2c follow-up
    // tracked under the homogeneous-typed HashMap / typed value-extraction
    // workstream cited in the body surfaces above.
}
