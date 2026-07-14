export const meta = {
  name: 'wf3c-3c-jit-write-barrier',
  description: 'WF-3C GC 3c: thread the compile-time GC write barrier through the JIT store sites so a JIT-compiled cycle is collectable (the JIT-hot-loop form of Finding #31). Today the JIT BYPASSES/inerts the barrier: inline_typed_field_set (places.rs:781) emits a raw store with NO barrier (JIT form of interior-mutation sink #2, var-field store); and 3 FFI store helpers (ffi/data.rs:462, ffi/object/object_ops.rs:98, ffi/typed_object/field_access.rs:197) call jit_write_barrier but hardcode old_kind_tag=0 (inert). DIAGNOSE-FIRST: enumerate EVERY JIT store site that can create a heap cycle edge (the JIT forms of ALL THREE interior-mutation sinks — #1 SharedCell::set, #2 var-field/TypedObject store, #3 store-into-SharedCow/typed-array element), map each site, and establish how each gets its overwritten slot NativeKind AT COMPILE TIME (the JIT knows the static FieldType) to pass gc_jit_kind_tag(field_kind) as a Cranelift iconst / a threaded FFI param — NOT a runtime schema decode (forbidden). Also resolve HK_JIT_OBJECT (kind 131) participation. Then wire the barrier at every cycle-creating store with the compile-time tag; gc-gate the inline emission so gc-OFF JIT codegen is byte-identical. Verify a JIT-compiled #31-shape cycle collects with gc on. SAFE to build now: gc feature off by default, so a wrong barrier ships as a no-op. Independent Opus verify.',
  phases: [
    { title: 'Diagnose', detail: 'enumerate all cycle-creating JIT store sites (sinks #1/#2/#3 JIT forms) + compile-time kind-tag source per site + HK_JIT_OBJECT role' },
    { title: 'Implement', detail: 'wire barrier at each site w/ compile-time gc_jit_kind_tag; gc-gate inline emission (gc-off byte-identical)' },
    { title: 'Verify', detail: 'independent Opus: JIT-compiled cycle collects gc-on; gc-off JIT byte-identical; no runtime decode / forbidden pattern' },
    { title: 'Finish', detail: 'gates + JIT-#31-collectable regression test (gc-on) + gc-off-parity + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-gc3c'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/gc-3c-jit-barrier, off main HEAD — has GC Phases 0/1/2/3a + §3.5-part1/part2 merged; Finding #31 fully bounded on the INTERPRETER path). Build/test via: ' + DX + ' <cmd>. Design: docs/design/real-gc-cycle-collection.md (§0 memory-only; "Phase 3 (refined)" flags this exact JIT bypass).',
  '',
  'THE GAP (Phase-3-refined finding): the collector sees interpreter cycles but the JIT can create a cycle edge WITHOUT notifying the collector. Sites today:',
  '  - crates/shape-jit/src/mir_compiler/places.rs:781 inline_typed_field_set — emits `builder.ins().store(MemFlags::trusted(), val, to_ptr, offset)` with NO barrier. This is the JIT form of interior-mutation sink #2 (var-field / TypedObject store) and the Finding-#31 hot store.',
  '  - crates/shape-jit/src/ffi/data.rs:462, crates/shape-jit/src/ffi/object/object_ops.rs:98, crates/shape-jit/src/ffi/typed_object/field_access.rs:197 — each calls jit_write_barrier(old_bits, value, 0) with old_kind_tag HARDCODED 0 (barrier body runs but is inert on tag 0).',
  '  The barrier: crates/shape-jit/src/ffi/gc.rs:64 jit_write_barrier(old_bits, new_bits, old_kind_tag) -> shape_value::gc::gc_jit_write_barrier(old_bits, old_kind_tag) (gc-gated; no-op on tag 0 or feature-off). Compile-time tag source: shape_value::gc::gc_jit_kind_tag(NativeKind) -> u64 (gc.rs:459). JIT object kind HK_JIT_OBJECT=131 (ffi/jit_kinds.rs:32).',
  '',
  'DIAGNOSE FIRST (no impl): (1) enumerate EVERY JIT store site that can overwrite a heap-pointer slot forming a cycle edge — the JIT forms of ALL THREE interior-mutation sinks: #1 SharedCell::set, #2 var-field/TypedObject store (inline_typed_field_set + the FFI field_access helper), #3 store-into-SharedCow-array / typed-array element store (the #31 `arr.push`/`arr[i]=` hot loop — find its JIT store site; it may NOT be inline_typed_field_set). Do not assume the 4 known sites are exhaustive; grep the JIT codegen + FFI for pointer-slot stores. (2) For EACH site, establish how to obtain the overwritten slot NativeKind AT COMPILE TIME (the JIT statically knows the field/element FieldType) so the barrier gets gc_jit_kind_tag(kind) as a Cranelift iconst (inline sites) or a threaded old_kind_tag FFI param filled with a compile-time constant at the codegen call site (FFI sites) — NEVER a runtime schema/tag decode inside the FFI helper. (3) Resolve HK_JIT_OBJECT (131): do JIT-domain objects participate in collectable cycles, and if so does their overwritten-slot kind map to a v2 HeapKind for gc_jit_kind_tag, or are they non-cycle-capable (document which). (4) Confirm the collector can act on a JIT-buffered candidate (the JIT store runs inline on the VM thread, so the same-thread safepoint reaches it).',
  '',
  'IMPLEMENT: wire jit_write_barrier at EVERY cycle-creating JIT store, passing the compile-time gc_jit_kind_tag(field_kind). For inline_typed_field_set: load old_bits (current slot) then emit the barrier call before the store, and GC-GATE the emission (cfg(feature="gc") in the codegen) so gc-OFF JIT output is byte-identical. For the 3 FFI helpers: thread a real old_kind_tag (add the param / fill the compile-time constant at the JIT call site) replacing the hardcoded 0. Handle HK_JIT_OBJECT per the diagnosis.',
  '',
  'CONSTRAINTS (CLAUDE.md + design, CRITICAL): behind `gc` feature. **gc-OFF JIT codegen BYTE-IDENTICAL** (no barrier call emitted when gc off — cfg-gate the inline emission; FFI helpers already no-op the barrier body off). The tag is a COMPILE-TIME constant, NEVER a runtime decode (no is_heap/tag-decode/schema-lookup-in-barrier). NO forbidden patterns (ValueWord/Bool-default/generic-opcode/parallel-discriminator/Convert*To). ADR-005/006 preserved. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. NOTE: JIT-heavy tests can SIGILL-race at high parallelism — run JIT tests with --test-threads=1 (and gate any heavy end-to-end behind existing deep-tests cfg if needed).',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): a test that a JIT-COMPILED cycle (the JIT-hot-loop #31 shape — force tier-up or drive the MIR store path directly) is COLLECTED with gc on (barrier fired -> candidate buffered -> collect bounds it), AND that gc-OFF the JIT output/behavior is unchanged (byte-identical or a codegen-parity assertion). Also assert no forbidden runtime decode was introduced.',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['store_sites', 'kind_tag_source', 'jit_object_role', 'coverage'],
  properties: {
    store_sites: { type: 'string', description: 'every cycle-creating JIT store site found, mapped to interior-mutation sink #1/#2/#3 (incl. the #31 array-store site)' },
    kind_tag_source: { type: 'string', description: 'per site: how the compile-time NativeKind is obtained + passed as gc_jit_kind_tag const (iconst vs threaded FFI param); no runtime decode' },
    jit_object_role: { type: 'string', description: 'HK_JIT_OBJECT (131) cycle participation + kind mapping, or non-cycle-capable (documented)' },
    coverage: { type: 'string', description: 'do the 4 known sites + any newly-found cover all 3 sinks JIT forms? gaps?' },
  },
}
const diag = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE ONLY (no impl). Enumerate all cycle-creating JIT store sites + compile-time kind-tag source per site + HK_JIT_OBJECT role + coverage. Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Implement')
const IMPL_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'jit_cycle_collectable', 'gc_off_byte_identical', 'no_runtime_decode', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the barrier wiring per site + compile-time tag threading, brief' },
    jit_cycle_collectable: { type: 'boolean', description: 'a JIT-compiled cycle (#31 shape) is now collected with gc on' },
    gc_off_byte_identical: { type: 'boolean', description: 'gc-off JIT codegen/behavior byte-identical (inline emission cfg-gated)' },
    no_runtime_decode: { type: 'boolean', description: 'tag is a compile-time constant everywhere; no runtime schema/tag decode added' },
    evidence: { type: 'string', description: 'captured: JIT #31 collects gc-on; gc-off parity; check-no-dynamic EXIT 0' },
  },
}
const impl = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — IMPLEMENT. Wire the barrier at every cycle-creating JIT store with the compile-time gc_jit_kind_tag; gc-gate inline emission. Run JIT tests --test-threads=1. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3C 3c: thread compile-time GC write barrier through JIT store sites (JIT-hot-loop #31 collectable, gc-off byte-identical)").',
  { label: 'implement', phase: 'Implement', effort: 'high', schema: IMPL_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'jit_cycle_collectable', 'gc_off_parity', 'no_runtime_decode', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    jit_cycle_collectable: { type: 'boolean', description: 'from YOUR OWN repro: a JIT-compiled cycle collects with gc on (barrier fired)' },
    gc_off_parity: { type: 'boolean', description: 'gc-off JIT codegen/behavior byte-identical (you checked)' },
    no_runtime_decode: { type: 'boolean', description: 'tag is compile-time everywhere; no runtime decode; no forbidden pattern (check-no-dynamic EXIT 0)' },
    evidence: { type: 'string', description: 'your own JIT-cycle-collect repro + gc-off parity check + forbidden-grep; concise' },
  },
}
const verify = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\nIMPL: ' + JSON.stringify(impl) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) does a JIT-COMPILED cycle (JIT-hot-loop #31 shape) actually get collected with gc on — barrier fires, candidate buffers, collection bounds it? Prove the barrier is reached from JIT-emitted code, not just the interpreter. (2) gc-OFF: is the JIT codegen/behavior byte-identical (no barrier emitted)? Verify all shape-jit + shape-vm tests green gc-off AND gc-on. (3) is the kind tag a COMPILE-TIME constant at every site (grep for any runtime schema/tag decode introduced) + check-no-dynamic EXIT 0? Any uncollected JIT cycle, gc-off behavior change, or runtime decode = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-jit/shape-vm (off + gc, --test-threads=1), brief' },
    test_added: { type: 'string', description: 'the JIT-#31-collectable (gc-on) + gc-off-parity regression tests' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nIMPL: ' + JSON.stringify(impl) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add the JIT-#31-collectable (gc-on) + gc-off-parity regression tests; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-jit and -p shape-vm (feature-off) + --features gc, --test-threads=1. Commit (git commit --no-verify -m "WF-3C 3c finalize: JIT write-barrier gate tests (JIT-hot-loop #31 collectable)").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, impl, verify, finish }
