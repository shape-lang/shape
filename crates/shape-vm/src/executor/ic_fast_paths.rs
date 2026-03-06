//! Inline cache (IC) fast paths for the VM executor.
//!
//! Each IC-eligible site (method dispatch, property access, arithmetic, dyn method call)
//! records type observations into a `FeedbackVector`. When a site is monomorphic
//! (single type/target observed), we can skip the generic dispatch cascade and go
//! directly to the cached handler/offset/type specialization.
//!
//! IC state transitions: Uninitialized → Monomorphic → Polymorphic (2-4) → Megamorphic (>4)

use crate::executor::VirtualMachine;
use crate::executor::objects::method_registry::MethodFn;
use crate::feedback::{FeedbackSlot, ICState};
use shape_value::ValueWord;
use shape_value::heap_value::HeapKind;

// ---------------------------------------------------------------------------
// Method IC fast path
// ---------------------------------------------------------------------------

/// Result of a monomorphic method IC check.
pub(crate) struct MethodIcHit {
    pub handler: MethodFn,
}

/// Check the method IC for a monomorphic hit.
///
/// If the feedback slot at `ip` is monomorphic and the receiver's `HeapKind`
/// matches the cached entry, returns the cached `MethodFn` handler pointer
/// so the caller can skip the PHF lookup.
///
/// Returns `None` on miss (uninitialized, polymorphic, megamorphic, or kind mismatch).
#[inline]
pub(crate) fn method_ic_check(
    vm: &VirtualMachine,
    ip: usize,
    receiver_kind: HeapKind,
    method_name_id: u32,
) -> Option<MethodIcHit> {
    let func_id = vm.call_stack.last()?.function_id? as usize;
    let fv = vm.feedback_vectors.get(func_id)?.as_ref()?;
    let slot = fv.get_slot(ip)?;
    match slot {
        FeedbackSlot::Method(fb) if fb.state == ICState::Monomorphic => {
            let entry = fb.entries.first()?;
            if entry.receiver_kind == receiver_kind as u8
                && entry.method_name_id == method_name_id
                && entry.handler_ptr != 0
            {
                // SAFETY: handler_ptr was stored from a valid MethodFn function pointer.
                let handler: MethodFn = unsafe { std::mem::transmute(entry.handler_ptr) };
                Some(MethodIcHit { handler })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Record a method IC observation and store the handler pointer for future fast-path hits.
#[inline]
pub(crate) fn method_ic_record(
    vm: &mut VirtualMachine,
    ip: usize,
    receiver_kind: u8,
    method_name_id: u32,
    handler: MethodFn,
) {
    if let Some(fv) = vm.current_feedback_vector() {
        fv.record_method(ip, receiver_kind, method_name_id, handler as usize);
    }
}

// ---------------------------------------------------------------------------
// Property IC fast path
// ---------------------------------------------------------------------------

/// Result of a monomorphic property IC check.
pub(crate) struct PropertyIcHit {
    pub field_idx: u16,
    pub field_type_tag: u16,
}

/// Check the property IC for a monomorphic hit on schema mismatch path.
///
/// When `op_get_field_typed` encounters a schema mismatch, this checks if the
/// feedback slot records a monomorphic mapping from the runtime schema_id to
/// a cached field_idx + field_type_tag — avoiding the double schema lookup.
#[inline]
pub(crate) fn property_ic_check(
    vm: &VirtualMachine,
    ip: usize,
    runtime_schema_id: u64,
) -> Option<PropertyIcHit> {
    let func_id = vm.call_stack.last()?.function_id? as usize;
    let fv = vm.feedback_vectors.get(func_id)?.as_ref()?;
    let slot = fv.get_slot(ip)?;
    match slot {
        FeedbackSlot::Property(fb) if fb.state == ICState::Monomorphic => {
            let entry = fb.entries.first()?;
            if entry.schema_id == runtime_schema_id {
                Some(PropertyIcHit {
                    field_idx: entry.field_idx,
                    field_type_tag: entry.field_type_tag,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check the megamorphic cache for a property lookup.
///
/// When a property access site has gone megamorphic (>4 schemas), we use the
/// global direct-mapped cache as a last resort before doing a full name lookup.
#[inline]
pub(crate) fn megamorphic_property_check(
    vm: &VirtualMachine,
    ip: usize,
    runtime_schema_id: u64,
    field_name: &str,
) -> Option<PropertyIcHit> {
    // Only use megamorphic cache if the slot is actually megamorphic
    let func_id = vm.call_stack.last()?.function_id? as usize;
    let fv = vm.feedback_vectors.get(func_id)?.as_ref()?;
    let slot = fv.get_slot(ip)?;
    match slot {
        FeedbackSlot::Property(fb) if fb.state == ICState::Megamorphic => {
            let key =
                crate::megamorphic_cache::MegamorphicCache::hash_key(runtime_schema_id, field_name);
            let (field_idx, field_type_tag) = vm.megamorphic_cache.probe(key)?;
            Some(PropertyIcHit {
                field_idx,
                field_type_tag,
            })
        }
        _ => None,
    }
}

/// Insert an entry into the megamorphic cache after a successful name-based lookup.
#[inline]
pub(crate) fn megamorphic_property_insert(
    vm: &mut VirtualMachine,
    runtime_schema_id: u64,
    field_name: &str,
    field_idx: u16,
    field_type_tag: u16,
) {
    let key = crate::megamorphic_cache::MegamorphicCache::hash_key(runtime_schema_id, field_name);
    vm.megamorphic_cache.insert(key, field_idx, field_type_tag);
}

// ---------------------------------------------------------------------------
// Arithmetic IC fast path
// ---------------------------------------------------------------------------

/// NanTag constants for IC comparison. These must match `NanTag as u8` values.
const NANTAG_I48: u8 = 1; // NanTag::I48
const NANTAG_F64: u8 = 0; // NanTag::F64

/// Arithmetic IC specialization hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArithmeticIcHint {
    /// Both operands are always I48 integers.
    BothI48,
    /// Both operands are always F64 floats.
    BothF64,
    /// Mixed or other types — no specialization available.
    None,
}

/// Check the arithmetic IC for a monomorphic type-pair specialization.
///
/// If the feedback slot shows a single type pair (e.g., always I48+I48),
/// the caller can branch directly to the typed fast path before popping operands.
#[inline]
pub(crate) fn arithmetic_ic_check(vm: &VirtualMachine, ip: usize) -> ArithmeticIcHint {
    let hint = (|| -> Option<ArithmeticIcHint> {
        let func_id = vm.call_stack.last()?.function_id? as usize;
        let fv = vm.feedback_vectors.get(func_id)?.as_ref()?;
        let slot = fv.get_slot(ip)?;
        match slot {
            FeedbackSlot::Arithmetic(fb) if fb.state == ICState::Monomorphic => {
                let pair = fb.type_pairs.first()?;
                if pair.left_tag == NANTAG_I48 && pair.right_tag == NANTAG_I48 {
                    Some(ArithmeticIcHint::BothI48)
                } else if pair.left_tag == NANTAG_F64 && pair.right_tag == NANTAG_F64 {
                    Some(ArithmeticIcHint::BothF64)
                } else {
                    Some(ArithmeticIcHint::None)
                }
            }
            _ => None,
        }
    })();
    hint.unwrap_or(ArithmeticIcHint::None)
}

// ---------------------------------------------------------------------------
// DynMethodCall IC fast path
// ---------------------------------------------------------------------------

/// Result of a monomorphic dyn method call IC check.
pub(crate) struct DynMethodIcHit {
    /// Cached VTableEntry function_id or closure.
    pub function_id: u16,
}

/// Check the dyn method call IC for a monomorphic hit.
///
/// If the trait object dispatch has always seen the same concrete receiver kind
/// and method, we can skip the vtable HashMap lookup and call the cached function
/// directly.
#[inline]
pub(crate) fn dyn_method_ic_check(
    vm: &VirtualMachine,
    ip: usize,
    concrete_kind: u8,
    method_hash: u32,
) -> Option<DynMethodIcHit> {
    let func_id = vm.call_stack.last()?.function_id? as usize;
    let fv = vm.feedback_vectors.get(func_id)?.as_ref()?;
    let slot = fv.get_slot(ip)?;
    match slot {
        FeedbackSlot::Method(fb) if fb.state == ICState::Monomorphic => {
            let entry = fb.entries.first()?;
            if entry.receiver_kind == concrete_kind && entry.method_name_id == method_hash {
                // handler_ptr stores function_id for dyn dispatch (cast from u16)
                if entry.handler_ptr != 0 {
                    Some(DynMethodIcHit {
                        function_id: entry.handler_ptr as u16,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Record a dyn method IC observation with the resolved function id.
#[inline]
pub(crate) fn dyn_method_ic_record(
    vm: &mut VirtualMachine,
    ip: usize,
    concrete_kind: u8,
    method_hash: u32,
    resolved_function_id: u16,
) {
    if let Some(fv) = vm.current_feedback_vector() {
        fv.record_method(
            ip,
            concrete_kind,
            method_hash,
            resolved_function_id as usize,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::FeedbackVector;

    #[test]
    fn test_arithmetic_ic_hint_i48() {
        let mut fv = FeedbackVector::new(0);
        fv.record_arithmetic(10, NANTAG_I48, NANTAG_I48);
        assert_eq!(fv.slots.len(), 1);
        match fv.get_slot(10).unwrap() {
            FeedbackSlot::Arithmetic(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.type_pairs[0].left_tag, NANTAG_I48);
                assert_eq!(fb.type_pairs[0].right_tag, NANTAG_I48);
            }
            _ => panic!("expected Arithmetic slot"),
        }
    }

    #[test]
    fn test_arithmetic_ic_hint_f64() {
        let mut fv = FeedbackVector::new(0);
        fv.record_arithmetic(10, NANTAG_F64, NANTAG_F64);
        match fv.get_slot(10).unwrap() {
            FeedbackSlot::Arithmetic(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.type_pairs[0].left_tag, NANTAG_F64);
                assert_eq!(fb.type_pairs[0].right_tag, NANTAG_F64);
            }
            _ => panic!("expected Arithmetic slot"),
        }
    }

    #[test]
    fn test_method_ic_handler_roundtrip() {
        // Verify function pointer can be stored and recovered via transmute
        fn dummy_handler(
            _vm: &mut VirtualMachine,
            _args: Vec<ValueWord>,
            _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
        ) -> Result<(), shape_value::VMError> {
            Ok(())
        }
        let ptr = dummy_handler as MethodFn as usize;
        assert_ne!(ptr, 0);
        let recovered: MethodFn = unsafe { std::mem::transmute(ptr) };
        assert_eq!(recovered as usize, ptr);
    }
}
