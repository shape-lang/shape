//! Feedback vectors for inline cache (IC) type profiling.
//!
//! Each function gets an optional `FeedbackVector` that records observed types
//! at IC-eligible sites (calls, property accesses, arithmetic, method dispatch).
//! The JIT compiler reads this feedback to generate speculative optimizations.

use std::collections::HashMap;

/// Maximum number of entries tracked before transitioning to Megamorphic.
const MAX_POLYMORPHIC_ENTRIES: usize = 4;

/// IC state machine: tracks how polymorphic a site has become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ICState {
    /// No observations yet.
    Uninitialized,
    /// Single type/target observed — optimal for speculation.
    Monomorphic,
    /// 2-4 types/targets observed — can use multi-way dispatch.
    Polymorphic,
    /// >4 types/targets — too many to specialize, use generic path.
    Megamorphic,
}

/// A feedback slot records type observations at a single IC site.
#[derive(Debug, Clone)]
pub enum FeedbackSlot {
    Uninitialized,
    Call(CallFeedback),
    Property(PropertyFeedback),
    Arithmetic(ArithmeticFeedback),
    Method(MethodFeedback),
}

/// Call site feedback: which function targets have been called.
#[derive(Debug, Clone)]
pub struct CallFeedback {
    pub state: ICState,
    pub targets: Vec<CallTarget>,
    pub total_calls: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallTarget {
    pub function_id: u16,
    pub count: u64,
}

/// Property access feedback: which schemas and field indices observed.
#[derive(Debug, Clone)]
pub struct PropertyFeedback {
    pub state: ICState,
    pub entries: Vec<PropertyCacheEntry>,
}

/// Receiver kind discriminator for property feedback.
/// Tells the JIT whether to emit a TypedObject schema guard or a HashMap shape guard.
pub const RECEIVER_TYPED_OBJECT: u8 = 0;
pub const RECEIVER_HASHMAP: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyCacheEntry {
    pub schema_id: u64,
    pub field_idx: u16,
    pub field_type_tag: u16,
    pub hit_count: u64,
    /// Receiver heap kind: 0 = TypedObject (schema guard), 1 = HashMap (shape guard).
    pub receiver_kind: u8,
}

/// Arithmetic feedback: which operand type pairs observed.
#[derive(Debug, Clone)]
pub struct ArithmeticFeedback {
    pub state: ICState,
    pub type_pairs: Vec<ArithmeticTypePair>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArithmeticTypePair {
    pub left_tag: u8,
    pub right_tag: u8,
    pub count: u64,
}

/// Method dispatch feedback: which receiver kinds and method handlers observed.
#[derive(Debug, Clone)]
pub struct MethodFeedback {
    pub state: ICState,
    pub entries: Vec<MethodCacheEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodCacheEntry {
    pub receiver_kind: u8,
    pub method_name_id: u32,
    pub handler_ptr: usize,
    pub hit_count: u64,
}

/// Per-function feedback vector: maps bytecode offsets to IC slots.
#[derive(Debug, Clone)]
pub struct FeedbackVector {
    pub function_id: u16,
    pub slots: HashMap<usize, FeedbackSlot>,
    pub generation: u32,
}

/// Compute the next IC state given the current state and entry count.
fn next_state(current: ICState, entry_count: usize) -> ICState {
    match current {
        ICState::Uninitialized => ICState::Monomorphic,
        ICState::Monomorphic => {
            if entry_count <= 1 {
                ICState::Monomorphic
            } else {
                ICState::Polymorphic
            }
        }
        ICState::Polymorphic => {
            if entry_count > MAX_POLYMORPHIC_ENTRIES {
                ICState::Megamorphic
            } else {
                ICState::Polymorphic
            }
        }
        ICState::Megamorphic => ICState::Megamorphic,
    }
}

impl FeedbackVector {
    /// Creates an empty feedback vector with generation 0.
    pub fn new(function_id: u16) -> Self {
        Self {
            function_id,
            slots: HashMap::new(),
            generation: 0,
        }
    }

    /// Rebase slot keys by subtracting `base_offset`.
    ///
    /// The interpreter records feedback at absolute bytecode IPs, but the JIT
    /// compiles sub-programs with 0-based instruction indices. This method
    /// remaps slots so the JIT can look up feedback by local offset.
    ///
    /// Slots with offsets below `base_offset` are dropped (they belong to
    /// a different function or the program preamble).
    pub fn rebase(&mut self, base_offset: usize) {
        if base_offset == 0 {
            return;
        }
        let old_slots = std::mem::take(&mut self.slots);
        for (offset, slot) in old_slots {
            if offset >= base_offset {
                self.slots.insert(offset - base_offset, slot);
            }
        }
    }

    /// Merge another feedback vector's slots into this one at a given offset.
    ///
    /// Used when inlining a callee: the callee's feedback slots are mapped
    /// into the outer function's sub-program index space by adding `offset`
    /// to each slot key.
    pub fn merge_at_offset(&mut self, other: &FeedbackVector, offset: usize) {
        for (ip, slot) in &other.slots {
            self.slots.insert(ip + offset, slot.clone());
        }
    }

    /// Records a call target at the given bytecode offset.
    pub fn record_call(&mut self, offset: usize, target_function_id: u16) {
        let slot = self
            .slots
            .entry(offset)
            .or_insert(FeedbackSlot::Uninitialized);

        match slot {
            FeedbackSlot::Uninitialized => {
                *slot = FeedbackSlot::Call(CallFeedback {
                    state: ICState::Monomorphic,
                    targets: vec![CallTarget {
                        function_id: target_function_id,
                        count: 1,
                    }],
                    total_calls: 1,
                });
            }
            FeedbackSlot::Call(fb) => {
                fb.total_calls += 1;
                if let Some(target) = fb
                    .targets
                    .iter_mut()
                    .find(|t| t.function_id == target_function_id)
                {
                    target.count += 1;
                } else if fb.state != ICState::Megamorphic {
                    fb.targets.push(CallTarget {
                        function_id: target_function_id,
                        count: 1,
                    });
                    fb.state = next_state(fb.state, fb.targets.len());
                } else {
                    // Megamorphic: don't add new entries
                    fb.state = ICState::Megamorphic;
                }
            }
            _ => {}
        }
    }

    /// Records a property access at the given bytecode offset.
    ///
    /// `receiver_kind`: 0 = TypedObject (schema guard), 1 = HashMap (shape guard).
    pub fn record_property(
        &mut self,
        offset: usize,
        schema_id: u64,
        field_idx: u16,
        field_type_tag: u16,
        receiver_kind: u8,
    ) {
        let slot = self
            .slots
            .entry(offset)
            .or_insert(FeedbackSlot::Uninitialized);

        match slot {
            FeedbackSlot::Uninitialized => {
                *slot = FeedbackSlot::Property(PropertyFeedback {
                    state: ICState::Monomorphic,
                    entries: vec![PropertyCacheEntry {
                        schema_id,
                        field_idx,
                        field_type_tag,
                        hit_count: 1,
                        receiver_kind,
                    }],
                });
            }
            FeedbackSlot::Property(fb) => {
                if let Some(entry) = fb
                    .entries
                    .iter_mut()
                    .find(|e| e.schema_id == schema_id && e.receiver_kind == receiver_kind)
                {
                    entry.hit_count += 1;
                } else if fb.state != ICState::Megamorphic {
                    fb.entries.push(PropertyCacheEntry {
                        schema_id,
                        field_idx,
                        field_type_tag,
                        hit_count: 1,
                        receiver_kind,
                    });
                    fb.state = next_state(fb.state, fb.entries.len());
                }
            }
            _ => {}
        }
    }

    /// Records arithmetic operand types at the given bytecode offset.
    pub fn record_arithmetic(&mut self, offset: usize, left_tag: u8, right_tag: u8) {
        let slot = self
            .slots
            .entry(offset)
            .or_insert(FeedbackSlot::Uninitialized);

        match slot {
            FeedbackSlot::Uninitialized => {
                *slot = FeedbackSlot::Arithmetic(ArithmeticFeedback {
                    state: ICState::Monomorphic,
                    type_pairs: vec![ArithmeticTypePair {
                        left_tag,
                        right_tag,
                        count: 1,
                    }],
                });
            }
            FeedbackSlot::Arithmetic(fb) => {
                if let Some(pair) = fb
                    .type_pairs
                    .iter_mut()
                    .find(|p| p.left_tag == left_tag && p.right_tag == right_tag)
                {
                    pair.count += 1;
                } else if fb.state != ICState::Megamorphic {
                    fb.type_pairs.push(ArithmeticTypePair {
                        left_tag,
                        right_tag,
                        count: 1,
                    });
                    fb.state = next_state(fb.state, fb.type_pairs.len());
                }
            }
            _ => {}
        }
    }

    /// Records a method dispatch at the given bytecode offset.
    pub fn record_method(
        &mut self,
        offset: usize,
        receiver_kind: u8,
        method_name_id: u32,
        handler_ptr: usize,
    ) {
        let slot = self
            .slots
            .entry(offset)
            .or_insert(FeedbackSlot::Uninitialized);

        match slot {
            FeedbackSlot::Uninitialized => {
                *slot = FeedbackSlot::Method(MethodFeedback {
                    state: ICState::Monomorphic,
                    entries: vec![MethodCacheEntry {
                        receiver_kind,
                        method_name_id,
                        handler_ptr,
                        hit_count: 1,
                    }],
                });
            }
            FeedbackSlot::Method(fb) => {
                if let Some(entry) = fb.entries.iter_mut().find(|e| {
                    e.receiver_kind == receiver_kind && e.method_name_id == method_name_id
                }) {
                    entry.hit_count += 1;
                } else if fb.state != ICState::Megamorphic {
                    fb.entries.push(MethodCacheEntry {
                        receiver_kind,
                        method_name_id,
                        handler_ptr,
                        hit_count: 1,
                    });
                    fb.state = next_state(fb.state, fb.entries.len());
                }
            }
            _ => {}
        }
    }

    /// Retrieves the feedback slot at the given bytecode offset.
    pub fn get_slot(&self, offset: usize) -> Option<&FeedbackSlot> {
        self.slots.get(&offset)
    }

    /// Returns true if the slot at the given offset is in Monomorphic state.
    pub fn is_monomorphic(&self, offset: usize) -> bool {
        match self.slots.get(&offset) {
            Some(FeedbackSlot::Call(fb)) => fb.state == ICState::Monomorphic,
            Some(FeedbackSlot::Property(fb)) => fb.state == ICState::Monomorphic,
            Some(FeedbackSlot::Arithmetic(fb)) => fb.state == ICState::Monomorphic,
            Some(FeedbackSlot::Method(fb)) => fb.state == ICState::Monomorphic,
            _ => false,
        }
    }

    /// Clears all slots and increments the generation counter.
    pub fn reset(&mut self) {
        self.slots.clear();
        self.generation += 1;
    }

    /// Number of active IC slots in this vector.
    pub fn slot_count(&self) -> usize {
        self.slots
            .values()
            .filter(|s| !matches!(s, FeedbackSlot::Uninitialized))
            .count()
    }

    /// Fraction of IC slots that are monomorphic (0.0 to 1.0).
    ///
    /// Used by the TierManager to decide whether feedback quality is
    /// sufficient to warrant optimizing JIT compilation. A high ratio
    /// (>0.7) suggests the function has stable types worth specializing.
    pub fn monomorphic_ratio(&self) -> f64 {
        let total = self.slot_count();
        if total == 0 {
            return 0.0;
        }
        let mono_count = self
            .slots
            .values()
            .filter(|s| match s {
                FeedbackSlot::Call(fb) => fb.state == ICState::Monomorphic,
                FeedbackSlot::Property(fb) => fb.state == ICState::Monomorphic,
                FeedbackSlot::Arithmetic(fb) => fb.state == ICState::Monomorphic,
                FeedbackSlot::Method(fb) => fb.state == ICState::Monomorphic,
                FeedbackSlot::Uninitialized => false,
            })
            .count();
        mono_count as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_empty_vector() {
        let fv = FeedbackVector::new(42);
        assert_eq!(fv.function_id, 42);
        assert!(fv.slots.is_empty());
        assert_eq!(fv.generation, 0);
    }

    #[test]
    fn test_record_call_first_observation_becomes_monomorphic() {
        let mut fv = FeedbackVector::new(0);
        fv.record_call(10, 1);

        let slot = fv.get_slot(10).unwrap();
        match slot {
            FeedbackSlot::Call(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.targets.len(), 1);
                assert_eq!(fb.targets[0].function_id, 1);
                assert_eq!(fb.targets[0].count, 1);
                assert_eq!(fb.total_calls, 1);
            }
            _ => panic!("expected Call slot"),
        }
    }

    #[test]
    fn test_record_call_same_target_stays_monomorphic() {
        let mut fv = FeedbackVector::new(0);
        fv.record_call(10, 1);
        fv.record_call(10, 1);
        fv.record_call(10, 1);

        let slot = fv.get_slot(10).unwrap();
        match slot {
            FeedbackSlot::Call(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.targets.len(), 1);
                assert_eq!(fb.targets[0].count, 3);
                assert_eq!(fb.total_calls, 3);
            }
            _ => panic!("expected Call slot"),
        }
    }

    #[test]
    fn test_record_call_different_target_becomes_polymorphic() {
        let mut fv = FeedbackVector::new(0);
        fv.record_call(10, 1);
        fv.record_call(10, 2);

        let slot = fv.get_slot(10).unwrap();
        match slot {
            FeedbackSlot::Call(fb) => {
                assert_eq!(fb.state, ICState::Polymorphic);
                assert_eq!(fb.targets.len(), 2);
            }
            _ => panic!("expected Call slot"),
        }
    }

    #[test]
    fn test_record_call_five_targets_becomes_megamorphic() {
        let mut fv = FeedbackVector::new(0);
        for i in 0..5 {
            fv.record_call(10, i);
        }

        let slot = fv.get_slot(10).unwrap();
        match slot {
            FeedbackSlot::Call(fb) => {
                assert_eq!(fb.state, ICState::Megamorphic);
                assert_eq!(fb.targets.len(), 5);
            }
            _ => panic!("expected Call slot"),
        }

        // 6th target should NOT be added
        fv.record_call(10, 99);
        match fv.get_slot(10).unwrap() {
            FeedbackSlot::Call(fb) => {
                assert_eq!(fb.state, ICState::Megamorphic);
                assert_eq!(fb.targets.len(), 5);
                assert_eq!(fb.total_calls, 6);
            }
            _ => panic!("expected Call slot"),
        }
    }

    #[test]
    fn test_record_property_state_transitions() {
        let mut fv = FeedbackVector::new(0);

        // First observation -> Monomorphic
        fv.record_property(20, 100, 0, 1, 0);
        match fv.get_slot(20).unwrap() {
            FeedbackSlot::Property(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.entries.len(), 1);
            }
            _ => panic!("expected Property slot"),
        }

        // Same schema -> stays Monomorphic, increments count
        fv.record_property(20, 100, 0, 1, 0);
        match fv.get_slot(20).unwrap() {
            FeedbackSlot::Property(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.entries[0].hit_count, 2);
            }
            _ => panic!("expected Property slot"),
        }

        // Different schemas -> Polymorphic
        fv.record_property(20, 200, 1, 2, 0);
        match fv.get_slot(20).unwrap() {
            FeedbackSlot::Property(fb) => {
                assert_eq!(fb.state, ICState::Polymorphic);
                assert_eq!(fb.entries.len(), 2);
            }
            _ => panic!("expected Property slot"),
        }

        // Add more schemas until Megamorphic
        fv.record_property(20, 300, 2, 3, 0);
        fv.record_property(20, 400, 3, 4, 0);
        fv.record_property(20, 500, 4, 5, 0);
        match fv.get_slot(20).unwrap() {
            FeedbackSlot::Property(fb) => {
                assert_eq!(fb.state, ICState::Megamorphic);
                assert_eq!(fb.entries.len(), 5);
            }
            _ => panic!("expected Property slot"),
        }

        // 6th schema not added
        fv.record_property(20, 600, 5, 6, 0);
        match fv.get_slot(20).unwrap() {
            FeedbackSlot::Property(fb) => {
                assert_eq!(fb.state, ICState::Megamorphic);
                assert_eq!(fb.entries.len(), 5);
            }
            _ => panic!("expected Property slot"),
        }
    }

    #[test]
    fn test_record_arithmetic_state_transitions() {
        let mut fv = FeedbackVector::new(0);

        // First -> Monomorphic
        fv.record_arithmetic(30, 1, 1);
        match fv.get_slot(30).unwrap() {
            FeedbackSlot::Arithmetic(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.type_pairs.len(), 1);
            }
            _ => panic!("expected Arithmetic slot"),
        }

        // Same pair -> stays Monomorphic
        fv.record_arithmetic(30, 1, 1);
        match fv.get_slot(30).unwrap() {
            FeedbackSlot::Arithmetic(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.type_pairs[0].count, 2);
            }
            _ => panic!("expected Arithmetic slot"),
        }

        // Different pair -> Polymorphic
        fv.record_arithmetic(30, 1, 2);
        match fv.get_slot(30).unwrap() {
            FeedbackSlot::Arithmetic(fb) => {
                assert_eq!(fb.state, ICState::Polymorphic);
                assert_eq!(fb.type_pairs.len(), 2);
            }
            _ => panic!("expected Arithmetic slot"),
        }

        // Fill to Megamorphic
        fv.record_arithmetic(30, 2, 2);
        fv.record_arithmetic(30, 3, 3);
        fv.record_arithmetic(30, 4, 4);
        match fv.get_slot(30).unwrap() {
            FeedbackSlot::Arithmetic(fb) => {
                assert_eq!(fb.state, ICState::Megamorphic);
                assert_eq!(fb.type_pairs.len(), 5);
            }
            _ => panic!("expected Arithmetic slot"),
        }
    }

    #[test]
    fn test_record_method_state_transitions() {
        let mut fv = FeedbackVector::new(0);

        // First -> Monomorphic
        fv.record_method(40, 10, 100, 0xDEAD);
        match fv.get_slot(40).unwrap() {
            FeedbackSlot::Method(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.entries.len(), 1);
                assert_eq!(fb.entries[0].handler_ptr, 0xDEAD);
            }
            _ => panic!("expected Method slot"),
        }

        // Same receiver+method -> stays Monomorphic
        fv.record_method(40, 10, 100, 0xDEAD);
        match fv.get_slot(40).unwrap() {
            FeedbackSlot::Method(fb) => {
                assert_eq!(fb.state, ICState::Monomorphic);
                assert_eq!(fb.entries[0].hit_count, 2);
            }
            _ => panic!("expected Method slot"),
        }

        // Different receiver -> Polymorphic
        fv.record_method(40, 20, 100, 0xBEEF);
        match fv.get_slot(40).unwrap() {
            FeedbackSlot::Method(fb) => {
                assert_eq!(fb.state, ICState::Polymorphic);
                assert_eq!(fb.entries.len(), 2);
            }
            _ => panic!("expected Method slot"),
        }

        // Fill to Megamorphic
        fv.record_method(40, 30, 100, 0xCAFE);
        fv.record_method(40, 40, 100, 0xF00D);
        fv.record_method(40, 50, 100, 0xBAAD);
        match fv.get_slot(40).unwrap() {
            FeedbackSlot::Method(fb) => {
                assert_eq!(fb.state, ICState::Megamorphic);
                assert_eq!(fb.entries.len(), 5);
            }
            _ => panic!("expected Method slot"),
        }
    }

    #[test]
    fn test_is_monomorphic() {
        let mut fv = FeedbackVector::new(0);

        // No slot -> false
        assert!(!fv.is_monomorphic(10));

        // Monomorphic call -> true
        fv.record_call(10, 1);
        assert!(fv.is_monomorphic(10));

        // Polymorphic call -> false
        fv.record_call(10, 2);
        assert!(!fv.is_monomorphic(10));

        // Monomorphic property -> true
        fv.record_property(20, 100, 0, 1, 0);
        assert!(fv.is_monomorphic(20));

        // Monomorphic arithmetic -> true
        fv.record_arithmetic(30, 1, 1);
        assert!(fv.is_monomorphic(30));

        // Monomorphic method -> true
        fv.record_method(40, 10, 100, 0xDEAD);
        assert!(fv.is_monomorphic(40));
    }

    #[test]
    fn test_reset_clears_and_increments_generation() {
        let mut fv = FeedbackVector::new(7);
        fv.record_call(10, 1);
        fv.record_property(20, 100, 0, 1, 0);
        assert_eq!(fv.slots.len(), 2);
        assert_eq!(fv.generation, 0);

        fv.reset();
        assert!(fv.slots.is_empty());
        assert_eq!(fv.generation, 1);
        assert_eq!(fv.function_id, 7);

        fv.reset();
        assert_eq!(fv.generation, 2);
    }
}
