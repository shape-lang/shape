export const meta = {
  name: 'wf-jit-typed-pointer-migration-bc',
  description: 'COMPLETE the JIT typed-pointer migration (Phase B + C), building on the validated first phase (branch wave7/jit-typed-pointer-migration, commits 2ea5887a + 143d59d9 in worktree shape-w7-jitmig). The first phase migrated the mainstream ObjectStore producer + its consumer cluster to the v2-raw *mut TypedObjectStorage carrier (repr(C) HeapHeader@0, out-of-line slot buffer, v2_retain/release) and PROVED gc-off parity + gc-on JIT-cycle collection (freed==2). But it left ~19-22 legacy sites on the OLD JIT-internal inline-cell UnifiedValue carrier — a MIXED-CARRIER state that must NOT ship (the carrier flip is NOT gc-gated, so a v2 object flowing into a still-legacy consumer, or vice versa, is a wrong-layout misread = memory corruption on the default gc-off path). This workflow makes the carrier UNIFORM: Phase B migrates the sibling TypedObject consumers (property_access HK_TYPED_OBJECT arm property_access.rs:227, jit_get_field_typed/set_field_typed data.rs:382/452) to slots()/write_slot_in_place on *const TypedObjectStorage; Phase C migrates the secondary producers (jit_typed_merge_object + jit_typed_object_from_hashmap merge_ops.rs, legacy jit_new_typed_object, manual inc_ref/dec_ref allocation.rs, ffi_exports box producers), threads heap/Option-field share-transfer + the write-barrier overwritten-slot kind (the 3c old_kind_tag=0 gap — now fixable since the carrier is the real v2 header) so JIT-hot-loop cycles collect, then DELETES the JIT-private TypedObject struct + box/unbox/is_typed_object + inc_ref/dec_ref. Draft the ADR-006 §2.3 amendment (single cross-crate carrier + authorized JIT-struct deletion — a clean path, NOT a contested fork per the first-phase design). NO mixed-carrier path may survive; NO kind-injection bridge (§2.7.10/Q11 defection). Independent Opus + valgrind verify. gc-off byte-identical; gc-on JIT-hot-loop #31 collectable end-to-end (completes 3c).',
  phases: [
    { title: 'PhaseB', detail: 'migrate sibling TypedObject consumers (property_access, get/set_field_typed) to the v2 carrier' },
    { title: 'PhaseC', detail: 'migrate secondary producers + inc/dec_ref + write-barrier kind (completes 3c); delete the JIT struct' },
    { title: 'Verify', detail: 'independent Opus + valgrind: no mixed carrier, gc-off byte-identical, gc-on JIT-hot-loop cycle collectable' },
    { title: 'Finish', detail: 'ADR-006 §2.3 amendment + full gates + regression tests + merge-ready' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-jitmig'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/jit-typed-pointer-migration — ALREADY has the validated first phase 2ea5887a + Phase-4 hardening 143d59d9). Build/test via: ' + DX + ' <cmd>. Continue ON this branch. Required reading: docs/cluster-audits/w17-jit-typed-object-arc-storage-migration-audit.md, docs/adr/006-value-and-memory-model.md (§2.3, §2.7.5, §2.7.7), docs/design/real-gc-cycle-collection.md (the 3c write-barrier gap), and the first-phase commits\' diff (git show 2ea5887a 143d59d9).',
  '',
  'THE CARRIER (target, established + validated by phase 1): the v2-raw *mut TypedObjectStorage (repr(C): HeapHeader@0 = refcount:AtomicU32@0/kind:u16@4/flags:u8@6, schema_id:u64@8, slot_cells fat-ptr@16 to an OUT-OF-LINE Box slot buffer, heap_mask@32, field_kinds:Arc<[NativeKind]>@40; allocated by TypedObjectStorage::_new, freed by _drop, refcounted by v2_retain/v2_release on the offset-0 header). This is what the VM already produces (post Wave-2 D1/D2/D4) and what the GC barrier (cycle_capable_direct_header / for_each_heap_child) + collector assume. NOT a literal Arc<TypedObjectStorage> (its real refcount at -16 would make the barrier misread the offset-0 header). Field addressing on this carrier: storage_ptr directly -> load slot-buffer base at storage+16 (JIT_OFFSET_SLOT_DATA) -> field at [slot_data + byte_off] (byte_off = idx*8), no UNIFIED_PTR_MASK, no NaN-box.',
  '',
  'WHY MIXED CARRIER IS UNSAFE (the reason this phase is mandatory before merge): the carrier change is NOT behind the gc feature — it is the always-on JIT representation. Phase 1 migrated the ObjectStore producer + inline hot path + field_access FFIs + the Ptr(TypedObject) retain/release arm. If a still-legacy consumer (property_access.rs:227, jit_get_field_typed/set_field_typed data.rs) dereferences a v2-produced object as the OLD inline-cell layout, OR a still-legacy producer (merge/from_hashmap) feeds an old-carrier object into the migrated inline consumer, it reads a HeapHeader/field at the WRONG offset = UB / heap corruption on the DEFAULT gc-off path. So EVERY producer and EVERY consumer must be on the v2 carrier before merge — no partition is safe because TypedObjects are fungible across access paths.',
  '',
  'PHASE B — migrate the sibling TypedObject CONSUMERS to the v2 carrier: property_access HK_TYPED_OBJECT arm (crates/shape-jit/src/ffi/object/property_access.rs:227), jit_get_field_typed (data.rs:382), jit_set_field_typed (data.rs:452) — move from the inline get_field/set_field on the JIT struct to slots() / write_slot_in_place on *const TypedObjectStorage (same shape the field_access FFIs already migrated to in phase 1). After Phase B, NO consumer reads the old inline-cell layout. Run gc-off tests green.',
  '',
  'PHASE C — migrate the secondary PRODUCERS + finish + delete: (1) jit_typed_merge_object + jit_typed_object_from_hashmap (merge_ops.rs) allocate via TypedObjectStorage::_new (schema-derived field_kinds/heap_mask) returning the raw storage pointer; (2) legacy jit_new_typed_object + the ffi_exports box producers likewise (or delete if truly dead — confirm first); (3) replace manual jit_typed_object_inc_ref/dec_ref (allocation.rs) with v2_retain/v2_release; (4) thread heap/Option-FIELD share-transfer (the store-into-object-field Arc retain on the v2 carrier) AND the JIT write-barrier overwritten-slot kind — inline_typed_field_set + the field_access set FFIs currently pass old_kind_tag=0 (the 3c gap); now that the carrier is the real v2 header, pass the compile-time gc_jit_kind_tag(field_kind) so a JIT-hot-loop object-field cycle is collectable (this COMPLETES 3c for the object-field sink #2); (5) DELETE the JIT-private TypedObject struct + box_typed_object/unbox_typed_object/is_typed_object + jit_typed_object_inc_ref/dec_ref once no site references them. NO kind-injection bridge, NO dual carrier (§2.7.10/Q11 + CLAUDE.md broader-family regex — refuse on sight).',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): NO forbidden patterns. gc-OFF JIT behavior semantically identical + hot path stays two loads (no perf regression). The write-barrier kind is a COMPILE-TIME gc_jit_kind_tag constant, NEVER a runtime decode. gc-gate only the barrier emission (gc-off codegen byte-identical there). The producer/consumer carrier flip for any shared object MUST leave no mixed path. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0 at each committed step. JIT tests --test-threads=1.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): (a) gc-off — object-spread {...base,z} + HashMap->object conversion + dynamic property access all correct in JIT (the previously-mixed paths, now uniform); (b) gc-on — a JIT-HOT-LOOP object-field cycle (the sink-#2 form: a JIT-compiled loop that stores a self-referential object field) is COLLECTED (barrier fires -> buffered -> freed), proving 3c complete for object fields. No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('PhaseB')
const B_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'consumers_migrated', 'no_legacy_consumer_left', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    consumers_migrated: { type: 'string', description: 'property_access + get/set_field_typed moved to v2 slots(), brief' },
    no_legacy_consumer_left: { type: 'boolean', description: 'no consumer still reads the old inline-cell layout' },
    evidence: { type: 'string', description: 'gc-off tests green after Phase B; check-no-dynamic EXIT 0' },
  },
}
const b = await agent(CTX + '\n\nPHASE B — migrate the sibling TypedObject consumers to the v2 carrier. gc-off tests green. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "JIT typed-pointer migration Phase B: sibling TypedObject consumers -> v2 carrier").',
  { label: 'phase-b', phase: 'PhaseB', effort: 'xhigh', schema: B_SCHEMA })

phase('PhaseC')
const C_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'producers_migrated', 'barrier_kind_threaded', 'struct_deleted', 'no_mixed_carrier', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    producers_migrated: { type: 'string', description: 'merge/from_hashmap/new_typed_object + inc/dec_ref -> v2, brief' },
    barrier_kind_threaded: { type: 'boolean', description: 'the JIT write-barrier overwritten-slot kind now uses compile-time gc_jit_kind_tag (3c object-field sink complete)' },
    struct_deleted: { type: 'boolean', description: 'the JIT-private TypedObject struct + box/unbox/is_typed_object + inc/dec_ref deleted' },
    no_mixed_carrier: { type: 'boolean', description: 'EVERY producer + consumer on the v2 carrier — no mixed path remains' },
    evidence: { type: 'string', description: 'object-spread/hashmap/dynamic-prop correct gc-off; JIT-hot-loop cycle collected gc-on; check-no-dynamic EXIT 0' },
  },
}
const c = await agent(CTX + '\n\nPHASE B RESULT: ' + JSON.stringify(b) + '\n\nPHASE C — migrate secondary producers + thread the write-barrier kind (complete 3c object-field sink) + delete the JIT struct. NO mixed carrier may remain. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "JIT typed-pointer migration Phase C: secondary producers + write-barrier kind (completes 3c) + delete JIT TypedObject struct").',
  { label: 'phase-c', phase: 'PhaseC', effort: 'xhigh', schema: C_SCHEMA })

phase('Verify')
const V_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'no_mixed_carrier', 'gc_off_no_regression', 'jit_hotloop_cycle_collected', 'no_unsafe', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    no_mixed_carrier: { type: 'boolean', description: 'you confirmed EVERY TypedObject producer/consumer is on the v2 carrier — no old-layout deref anywhere' },
    gc_off_no_regression: { type: 'boolean', description: 'full shape-jit + shape-vm suites green gc-off; object-spread/hashmap/dynamic-prop correct; hot path no perf regression' },
    jit_hotloop_cycle_collected: { type: 'boolean', description: 'from YOUR OWN repro: a JIT-hot-loop object-field cycle is collected gc-on (3c complete)' },
    no_unsafe: { type: 'boolean', description: 'valgrind clean on JIT-produced-object programs (gc-off AND gc-on) — no UB/misread/leak/double-free' },
    evidence: { type: 'string', description: 'your own from-scratch runs + valgrind + a grep proving no box/unbox_typed_object / old inline-cell deref remains; concise' },
  },
}
const v = await agent(CTX + '\n\nPHASE B: ' + JSON.stringify(b) + '\nPHASE C: ' + JSON.stringify(c) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). This is a NOT-gc-gated carrier migration touching the default path — be maximally skeptical about mixed-carrier misreads. (1) grep-prove NO old carrier survives: no box_typed_object/unbox_typed_object/is_typed_object/jit_typed_object_inc_ref/dec_ref references, no old inline-cell layout deref — the JIT struct is deleted. (2) gc-OFF: run the FULL shape-jit + shape-vm suites (--test-threads=1) + your own object-spread + HashMap->object + dynamic-property-access + object-passed-to-function programs — all correct? (3) gc-ON: build your own JIT-hot-loop object-field cycle and confirm it is COLLECTED (3c complete). (4) valgrind (--error-exitcode=99 --leak-check=full) on JIT-produced-object programs gc-off AND gc-on — ANY misread/UB/leak/double-free = REFUTED. Any surviving old-carrier reference, any regression, any uncollected hot-loop cycle, or any valgrind error = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'xhigh', schema: V_SCHEMA })

phase('Finish')
const F_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'adr_amendment', 'tests_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-jit/shape-vm (off + gc) + valgrind, brief' },
    adr_amendment: { type: 'string', description: 'the ADR-006 §2.3 amendment drafted (single cross-crate carrier + JIT-struct deletion)' },
    tests_added: { type: 'string', description: 'gc-off mixed-path-now-uniform + gc-on JIT-hot-loop-cycle-collected regression tests' },
    merge_ready: { type: 'boolean' },
  },
}
const f = await agent(CTX + '\n\nPHASE C: ' + JSON.stringify(c) + '\nVERDICT: ' + JSON.stringify(v) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Draft the ADR-006 §2.3 amendment in docs/adr/006-value-and-memory-model.md (single cross-crate v2 carrier for JIT TypedObject; JIT-private struct deleted; a defection-attractor note). Add the regression tests. no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-jit and -p shape-vm (feature-off) + --features gc --test-threads=1, and valgrind on a JIT-object program. Commit (git commit --no-verify -m "JIT typed-pointer migration finalize: ADR-006 §2.3 amendment + regression tests (carrier uniform, 3c object-field sink complete)").',
  { label: 'finish', phase: 'Finish', effort: 'high', schema: F_SCHEMA })

return { b, c, v, f }
