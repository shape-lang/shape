//! Wave 5e (phase-1b-vm): JSON navigation helper bodies deferred.
//!
//! `builtin_json_object_get`, `builtin_json_array_at`, `builtin_json_object_keys`,
//! `builtin_json_array_len`, `builtin_json_object_len` were VM-side bodies for
//! the `Json*` `BuiltinFunction` arms. The previous bodies relied on
//! `pop_builtin_args -> ArgVec<ValueWord>` plus `ValueWordExt` accessors
//! (`as_hashmap`, `vw_hash`, `vw_equals`) and `ValueWord::from_heap_value` —
//! all deleted.
//!
//! Wave 5a flipped the dispatch shape; the `JsonObjectGet`, `JsonArrayAt`,
//! `JsonObjectKeys`, `JsonArrayLen`, `JsonObjectLen` arms in
//! `vm_impl/builtins.rs` `todo!` for Wave 5e. This file is intentionally
//! empty post-Wave 5b backlog reduction. Wave 5e migrates the bodies to
//! `&[KindedSlot] -> Result<KindedSlot, VMError>` per the §2.7.6 / Q8
//! carrier-API bound — heap dispatch through `slot.as_heap_value()` +
//! `HeapValue` match (ADR-005 §1 single-discriminator), no per-heap-variant
//! accessors on `KindedSlot`.
