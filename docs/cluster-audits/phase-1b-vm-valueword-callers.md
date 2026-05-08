# Phase 1.B-vm audit — `ValueWord` consumer-side migration

**Date:** 2026-05-08
**Audit scope:** `crates/shape-vm/src/` and `crates/shape-jit/src/` (consumer-side analog of Phase 1.B)
**Method:** Read-only grep + classification per use site, post §2.7.5.1 ruling
**Outcome:** Drives 10-wave dispatch plan to close ~4000-5500 errors total across both crates.

## Headline numbers

- **shape-vm:** ~199 source files reference deleted APIs.
- **shape-jit:** ~40 source files reference deleted APIs.
- **`ValueWord\b`** mentions across both: **4385**
- **`ArgVec`**: 289 sites
- **`vmarray_*`**: 168 sites
- **`tag_bits::*`**: 188 sites
- **`value_word_drop::*`**: 63 sites
- **`RareHeapData`**: 23 sites
- **`NativeKind::Unknown`**: 63 sites (44 shape-vm + 19 shape-jit) — all TYPE_TRACKER_INTERMEDIATE per §2.7.5.1
- **`register_test_function`**: 27 sites in shape-vm tests/comptime
- **`TypedReturn::Bool`**: 5 sites in `compiler/comptime_builtins.rs` — the variant doesn't exist; correct shape is `TypedReturn::Concrete(ConcreteReturn::Bool(...))`
- **`ValueWordDisplay`**: 2 sites (`compiler/comptime_builtins.rs:217`, `executor/objects/content_methods.rs:27`)
- **`PrintResult` / `PrintSpan`** consumers: `executor/builtins/special_ops.rs:15,70,76`, `executor/printing.rs:292`, `remote.rs`
- **`KindedSlot::as_str` calls** (~10 sites) — accessor needs adding (DETAIL per §2.7.4)

## §1 Per-file table (top sites, full audit at agent transcript)

| Crate | File | Refs | STATIC | CLOSURE | GENERIC | VM_RAW | JIT_FFI | TYPE_TRACK | TEST_REG | DEPR | OTHER |
|---|---|---|---|---|---|---|---|---|---|---|---|
| shape-vm | executor/variables/mod.rs | 165 | 30 | 35 | 0 | 95 | 0 | 5 | 0 | 0 | 0 |
| shape-vm | executor/objects/typed_array_methods.rs | 154 | 154 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/builtins/type_ops.rs | 140 | 100 | 0 | 30 | 10 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/objects/datatable_methods/tests.rs | 136 | 130 | 0 | 0 | 6 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/objects/property_access.rs | 123 | 80 | 0 | 0 | 43 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/vm_impl/builtins.rs | 108 | 0 | 0 | 108 | 0 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/printing.rs | 101 | 0 | 0 | 70 | 0 | 0 | 0 | 0 | 11 | 20 |
| shape-vm | executor/control_flow/native_abi.rs | 100 | 95 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 5 |
| shape-vm | executor/vm_impl/stack.rs | 94 | 0 | 0 | 0 | 94 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/builtins/math.rs | 94 | 80 | 0 | 14 | 0 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/objects/mod.rs | 89 | 50 | 0 | 0 | 39 | 0 | 0 | 0 | 0 | 0 |
| shape-jit | ffi/object/conversion.rs | 70 | 0 | 0 | 0 | 0 | 70 | 0 | 0 | 0 | 0 |
| shape-vm | executor/builtins/special_ops.rs | 69 | 30 | 0 | 30 | 0 | 0 | 0 | 0 | 9 | 0 |
| shape-vm | executor/v2_handlers/typed_array.rs | 65 | 5 | 0 | 0 | 60 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/arithmetic/mod.rs | 65 | 0 | 0 | 0 | 65 | 0 | 0 | 0 | 0 | 0 |
| shape-vm | executor/v2_handlers/v2_array_detect.rs | 64 | 0 | 0 | 0 | 64 | 0 | 0 | 0 | 0 | 0 |
| shape-jit | ffi/object/closure.rs | 63 | 0 | 0 | 0 | 0 | 63 | 0 | 0 | 0 | 0 |
| shape-vm | type_tracking.rs | 8 | 0 | 0 | 0 | 0 | 0 | **8** | 0 | 0 | 0 |
| shape-vm | bytecode/verifier.rs | 3 | 0 | 0 | 0 | 0 | 0 | 3 | 0 | 0 | 0 |
| shape-jit | mir_compiler/types.rs | 12 | 0 | 0 | 0 | 0 | 0 | 12 | 0 | 0 | 0 |
| shape-jit | mir_compiler/v2_call_abi.rs | 8 | 0 | 0 | 0 | 0 | 6 | 2 | 0 | 0 | 0 |

(Remaining ~140 shape-vm + ~30 shape-jit files cluster into the same shape — STATIC dominant in `objects/*`, `executor/builtins/*`, `compiler/*` test/registration sites; VM_RAW dominant in `executor/typed_handlers/*`, `executor/v2_handlers/*`, `executor/stack_ops/`, `executor/loops/`, `bytecode/*`. shape-jit clusters: JIT_FFI dominant in `ffi/*` and `ffi_symbols/*`; TYPE_TRACK in `mir_compiler/{types,v2_call_abi,statements,rvalues}.rs`.)

## §2 GENERIC_CARRIER + TYPE_TRACKER_INTERMEDIATE catalog

### A. TYPE_TRACKER_INTERMEDIATE (the most §2.7.5.1-binding category)

**A1. `shape-vm/src/type_tracking.rs:135-197`** (Serde, Hash via FunctionBlob) — `FrameDescriptor { pub slots: Vec<NativeKind>, ..., pub return_kind: NativeKind }`. §2.7.5.1 binding: `slots` stays `Vec<NativeKind>`, no Option, no `Unknown`. `with_unknown_slots`, `from_slots` (return_kind), `slot()`, `is_all_unknown()`, `new()` use `Unknown` as sentinel — all migrate to local `Option<NativeKind>` analysis state in a separate tracker. **8 references, all in this file.**

**A2. `shape-vm/src/bytecode/verifier.rs:5,24,156`** — `FrameVerificationError::TrustedSlotUnknown` triggers on `*slot == NativeKind::Unknown`. Wire-format integrity assertion; keep as wire-side guard, optionally rename to `TrustedSlotUnproven`.

**A3. `shape-vm/src/executor/osr.rs:119,154,194,241`** — 4 sites of `unwrap_or(NativeKind::Unknown)` reading `FrameDescriptor.slots.get(idx)`. Out-of-range is verifier-caught bug; use `panic!("verifier should reject")` or `ProofGap` propagation.

**A4. `shape-vm/src/executor/variables/mod.rs:2445,2538,2705`** — same `unwrap_or(NativeKind::Unknown)` shape. Same recipe.

**A5. `shape-vm/src/executor/control_flow/mod.rs:184,192`** — `param_kinds = [NativeKind::Unknown; 256]` and `return_kind = NativeKind::Unknown` initialization. Local intermediate state during call setup; migrate to `Option<NativeKind>` locally.

**A6. `shape-vm/src/executor/dispatch.rs:71`** — `NativeKind::Unknown => None` in match. Local intermediate; convert to `Option::None`.

**A7. `shape-jit/src/mir_compiler/types.rs:27,120,124,136,253,301,352-353,567,578,607,626`** (12 sites, JIT-Codegen) — `Vec<NativeKind>` analysis tracker. Migrate to `Vec<Option<NativeKind>>` in the analysis pass; `cranelift_type_for_slot` consumer takes proven `NativeKind`.

**A8. `shape-jit/src/mir_compiler/v2_call_abi.rs:57,69,118,186,196,210,221,222`** (JIT-Codegen) — `NativeKind::Unknown` as "no signature" marker. Replace with `NativeKind::Dynamic`-only or local `Option<NativeKind>`.

**A9. `shape-jit/src/mir_compiler/v2_field.rs:80,106`** (JIT-Codegen) — Cranelift type-of-slot fallback `String|Dynamic|Unknown => I64`. Drop `Unknown`.

**A10. `shape-jit/src/mir_compiler/{rvalues,statements,mod}.rs`, `worker.rs`, `osr_compiler.rs`, `executor.rs`** — single-line predicate sites. Local analysis defaults.

**A11. test_utils sites** — test helpers mirroring `unwrap_or(Unknown)`. Move to `Option<NativeKind>` in tests.

**A12. `shape-vm/src/executor/control_flow/jit_abi.rs:103,164,228,290,334-335,370,372`** — JIT_FFI_RAW + TYPE_TRACKER_INTERMEDIATE crosscut. `String | Dynamic | Unknown => raw_bits()` arm in `marshal_arg_to_jit` / `unmarshal_jit_result`. Per §2.7.5 boundary, raw u64 + parallel `NativeKind` required; remove `Unknown` from match (`Dynamic` stays).

**A13. `shape-vm/src/compiler/{v2_array_emission,typed_emission,v2_typed_emission,array_emission}.rs`** — `kind == Unknown || kind == Dynamic` predicate. Convert to `Option<None> | Some(Dynamic)`.

### B. GENERIC_CARRIER (heterogeneous runtime values without static kind)

**B1. `executor/printing.rs`** (101 refs, GENERIC_CARRIER + DEPR) — formatter API takes `&KindedSlot`; internal recursion dispatches on `kind`. RareHeapData::PrintResult arm at :292 deleted per §2.7.4.

**B2. `executor/builtins/special_ops.rs:15,70,76`** — `PrintResult` / `PrintSpan` import switches to `shape_runtime::output_adapter::*`. Trait sig: `fn print(&mut self, result: PrintResult) -> KindedSlot`.

**B3. `executor/control_flow/foreign_marshal.rs`** (53 refs, GENERIC_CARRIER) — ValueWord ↔ MessagePack marshaling for foreign function calls. Migrates to `KindedSlot` per §2.7.5 internal Rust ABI.

**B4. `executor/exceptions/mod.rs`** (62 refs, GENERIC_CARRIER) — `Exception { error: ValueWord, ... }` and `build_trace_frame_nb` family carry kind-erased error payloads. Migrate carrier to `KindedSlot`; trace-frame TypedObject construction is STATIC_KIND.

**B5. `executor/state_builtins/{core.rs:38, introspection.rs:57}`** (95 refs combined) — state-snapshot APIs surfacing heterogeneous module/binding state. Maps to `Vec<KindedSlot>` / `HashMap<String, KindedSlot>`. Parallels shape-runtime's `module_exports.rs:42-88` `FrameInfo`.

**B6. `executor/builtins/{remote_builtins.rs:47, transport_builtins.rs:44}`** + tests — wire serialization payload carriers with WB2.4 retain-on-read discipline. Migrate to `KindedSlot` (Drop/Clone preserves refcount).

**B7. `executor/vm_state_snapshot.rs`** (16 refs, WB2.4-discipline) — `VmStateAccessor::current_args()`/`current_locals()`/`module_bindings()` return `Vec<ValueWord>`/`Vec<(String, ValueWord)>`. Migrate to `Vec<KindedSlot>`/`Vec<(String, KindedSlot)>`.

**B8. `executor/vm_impl/builtins.rs`** (108 refs, GENERIC_CARRIER vector) — `pop_builtin_args -> Vec<KindedSlot>`. `ArgVec` typedef → `Vec<KindedSlot>`. Largest single mechanical site in the crate.

**B9. `executor/builtins/math.rs`** (94 refs, GENERIC_CARRIER dispatch slice) — `fn body(args: ArgVec, ...) -> Result<ValueWord>` → `Fn(&[KindedSlot], ...) -> Result<KindedSlot>`. Body interior is mostly STATIC_KIND once the slice is `KindedSlot`.

**B10. `executor/builtins/type_ops.rs`** (140 refs) — type-conversion builtins. Args heterogeneous (caller-sourced); returns STATIC_KIND. `&KindedSlot` input + `KindedSlot::from_*` output.

**B11. `executor/objects/property_access.rs`** (123 refs, GENERIC_CARRIER) — generic property access on heterogeneous receiver. Internal `HeapKind` match is STATIC_KIND once carrier is `KindedSlot`.

**B12. `executor/objects/{hashmap,column,iterator,array_*,datatable_methods/*,...}.rs`** — uniformly the same shape: receiver statically a TypedXxx (STATIC_KIND), but heterogeneous arguments / per-element values are GENERIC_CARRIER. Most refs STATIC; ~5–15 sites/file are GENERIC.

**B13. `shape-jit/src/ffi_symbols/data_access/mod.rs:10,68-89`** (cross-crate ABI, JIT_FFI_RAW) — calls `align_tables`. Per §2.7.5: conversion happens at the JIT side boundary (raw u64 + per-column NativeKind registry → KindedSlot for runtime call → unpack back to raw u64).

**B14. `shape-jit/src/ffi/object/conversion.rs`** (70 refs, JIT_FFI_RAW) — JIT-emitted u64 → ValueWord materialization. Per §2.7.5, JIT side stays raw; conversion happens on shape-runtime side. Delete the materialization path; replace with raw-u64 + parallel NativeKind dual-pass.

**B15. `shape-jit/src/ffi/object/closure.rs`** (63 refs) — closure capture FFI carrier. `tag_bits::*` constant peeks + `ValueWord::clone_from_bits`. Two-shape: JIT_FFI_RAW for capture-cell ABI; CLOSURE_CAPTURE for captured-value side (NativeKind in OwnedClosureBlock.field_kinds).

**B16. `shape-jit/src/ffi/control/mod.rs`** (38 refs) — control-flow exit/resume FFI between JIT and VM. JIT_FFI_RAW; NativeKind from FrameDescriptor.

### C. CLOSURE_CAPTURE catalog

**C1. `shape-vm/src/executor/variables/mod.rs`** (165 refs, biggest file) — OwnedMutableCapture / SharedCapture / LocalMutablePtr / SharedCell machinery. Capture cells store raw `*mut u64` bits; NativeKind in `OwnedClosureBlock.field_kinds`. Keep raw u64 cell payload, add per-capture NativeKind from field_kinds. ~95 sites VM_RAW_U64, 35 CLOSURE_CAPTURE, 30 STATIC_KIND.

**C2. `shape-jit/src/ffi/object/closure.rs:103-1576`** (alongside B15) — same closure machinery, JIT side. **Cross-side discipline**: cell ABI stays raw u64 + parallel kind from FrameDescriptor / OwnedClosureBlock.

### D. VM_RAW_U64 (kind via opcode operand or FrameDescriptor)

- `executor/typed_handlers/*.rs` and `executor/v2_handlers/*.rs` — typed opcode handlers; operand carries kind. VM_RAW_U64 by design (§2.7.5 ¶3).
- `executor/{arithmetic,comparison,logical,loops,stack_ops}/mod.rs` — opcode handlers on raw stack slots.
- `executor/vm_impl/stack.rs` (94 refs) — stack ABI. **HIGHEST-RISK site**: WB2.4 retain-on-read pattern depends on knowing when a slot holds heap-tagged ValueWord. Post-deletion, kind comes from FrameDescriptor — `vw_clone(self.stack[idx])` becomes `clone_slot_with_kind(self.stack[idx], frame.slot_kind(idx))` (new helper). **Wave 6 architectural surface.**
- `bytecode/opcode_defs.rs` (39 refs) — same WB2 pattern.
- `executor/{call_convention, control_flow/mod}.rs` — non-FFI function call ABI.

### E. JIT_FFI_RAW catalog

- `shape-jit/src/ffi/value_ffi.rs` (19 refs) — FFI tag constants, `tag_bits::*` re-exports. **Architectural surface**: shape-jit owns its tag layout independently now (CLAUDE.md confirms ffi/value_ffi.rs and ffi/jit_kinds.rs as JIT-specific home). 19 sites self-define locally; do NOT re-export from deleted shape-value.
- `shape-jit/src/ffi/jit_kinds.rs` (6 refs) — JIT kind constants. JIT_FFI_RAW.
- `shape-jit/src/ffi_refs.rs` — JIT reference handle marshalling.
- `shape-jit/src/ffi/{arc,array,async_ops,call_method,control,conversion,data,generic_builtin,object,typed_object,v2,v2_typed}*.rs` — all JIT FFI symbol bridges.
- `shape-jit/src/foreign_bridge.rs` (14 refs), `compiler/accessors.rs` (1 ref).

### F. TEST_REGISTRATION catalog

- 27 sites use `register_test_function` → variadic `register_typed_function` (§2.7.4) with `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>` body.
- 5 sites use `TypedReturn::Bool(b)` → `TypedReturn::Concrete(ConcreteReturn::Bool(b))`.
- 2 sites use `ValueWordDisplay(*nb)` → `format!("{:?}", kinded_slot)` or `KindedSlot::display()`.
- ~10 sites use `nb.as_str()` → add `KindedSlot::as_str() -> Option<&str>` accessor (kind-dispatch on String arm).

Sites: `executor/tests/mod.rs`, `lib_tests_parts/extension_{integration,system}_tests.rs`, `compiler/{compiler_tests,comptime,comptime_builtins}.rs`.

### G. DEPRECATED-comment-only files

~40 files where the only references are top-of-file `use` lines pointing to deleted symbols. Mostly `executor/builtins/*_tests.rs`, `executor/objects/*_tests.rs`. Pure cleanup.

## §3 Wave-bounded dispatch plan (10 waves)

| Wave | Files | Direct errors | Cascade absorbed | Cumulative | Pre-reqs |
|---|---|---|---|---|---|
| **1** type_tracking.rs cleanup + verifier | 3 | ~10 | 200-400 | 200-400 | none |
| **2** JIT compile-time analysis tier (mir_compiler) | 8 | 50-80 | 100-200 | 350-680 | Wave 1 |
| **3** Test registration hygiene (register_test_function/TypedReturn::Bool/ValueWordDisplay/as_str) | 7 | 100-150 | 50 | 500-880 | none |
| **4** printing.rs + output_adapter cutover (PrintResult/Span) | 6 | 150 | 100-200 | 750-1230 | Wave 3 |
| **5** Builtin dispatch slice (vm_impl/builtins.rs + arms) — **largest** | 25+ | 700-900 | 200-300 | 1650-2430 | Waves 1, 3 |
| **6** Stack ABI + WB2 retain-on-read — **highest-risk** | 12 | 400-500 | 300 | 2350-3230 | Waves 1, 2 |
| **7** Closure capture cell ABI (VM + JIT coordination) | 4 | 250-300 | 100-200 | 2700-3730 | Wave 6 |
| **8** Exceptions + state snapshot + foreign marshal (GENERIC_CARRIER cluster) | 12 | 250 | 50-100 | 3000-4080 | Waves 5, 6 |
| **9** Compiler emission + objects mass migration (STATIC_KIND-dominant cleanup) | 140+ | 600-800 | 100 | 3700-4980 | Waves 1, 2, 5, 6 |
| **10** JIT FFI surface (closes shape-jit) | 30 | 300-500 | — | 4000-5480 | Waves 1, 2, 5, 8 |

**Total: ~4000-5500 errors closed**. Wave 1 must close fully before Wave 6 (Wave 6 reads clean FrameDescriptor).

## §4 Surprises / outliers

1. **Stack ABI WB2 retain-on-read** (Wave 6, highest-risk) — kind for clone-with-retain comes from FrameDescriptor.slot_kind(idx). Per CLAUDE.md "Forbidden code", generic `LoadLocal` opcodes are deleted — typed `LoadLocal{Bool,Int,...}` carry the kind operand-side. Confirm in Wave 6 that no caller is on a generic dispatch path.

2. **shape-jit::ffi::value_ffi.rs parallel tag layout** (Wave 10) — CLAUDE.md confirms ffi/value_ffi.rs and ffi/jit_kinds.rs as the home for JIT-specific tags. 19 sites self-define locally. Defection-attractor: re-exporting deleted `shape_value::tag_bits::*` as a "compatibility shim" — refuse on sight.

3. **`KindedSlot::as_str()` accessor** (Wave 3) — comptime_builtins.rs uses `nb.as_str()`. Add as a per-kind accessor on `KindedSlot` (kind-dispatch on String arm). DETAIL per existing rules — accessor methods on a carrier struct are fine, since they don't violate the carrier-not-discriminator constraint.

4. **`compiler/comptime_builtins.rs:217` and `executor/objects/content_methods.rs:27` `ValueWordDisplay(*nb)`** (Wave 3) — call site formats stack-side ValueWord from arbitrary user input; kind-source not statically known. GENERIC_CARRIER, requires `KindedSlot` thread-through. Multi-line formatter case allowed by §2.7.4.

5. **`shape-vm/src/remote.rs`** uses `SV::PrintResult(pr)` and `shape_runtime::snapshot::PrintSpanSnapshot` — wire-protocol arm. Per §2.7.5.1, post-proof shape; `SV::PrintResult` deserialization arm should not change shape.

6. **Wire-format Serde structs reaching `FunctionBlob`**: `FrameDescriptor`, `BindingOwnershipClass`, `Aliasability`, `MutationCapability`, `EscapeStatus`, `BindingStorageClass`, `BindingSemantics`. All fields stay post-proof. Verify Wave 1 doesn't slip Option/Unknown into any.

7. **`executor/snapshot.rs`** (2 refs) — phase-2c snapshot rebuild deferral surface. `todo!("phase-2c snapshot rebuild")` placeholders, not real migration. Verify in Wave 8.

8. **`align_tables` cross-crate site** — `shape-jit/src/ffi_symbols/data_access/mod.rs:95` is the single shape-jit consumer of a shape-runtime API the Phase 1.B audit listed (Cluster M). Coordinate Wave 10 with shape-runtime side (already `&[KindedSlot]`).

9. **Dual test_utils** — `shape-vm/src/test_utils.rs` and `executor/tests/test_utils.rs` both have `NativeKind::Unknown => None` arms. Verify after Wave 1 that the test-helper API surfaces `Option<NativeKind>` (not "synthetic Unknown").

10. **`executor/control_flow/jit_abi.rs`** — VM_RAW_U64 *and* JIT_FFI_RAW dual-classification. `marshal_arg_to_jit`/`unmarshal_jit_result` for the JIT call boundary. Per §2.7.5, conversion on the runtime side; per VM↔JIT slot ABI rule it stays raw. Remove `Unknown` from match, keep raw-bits semantics.

## §5 Definition-of-done (per wave)

- All errors in the wave's file list closed
- `cargo check -p shape-vm --lib` and `-p shape-jit --lib` errors strictly decreasing wave-over-wave
- `bash scripts/check-no-dynamic.sh` exit 0 throughout
- AGENTS.md row updated to `idle` with wave-close commit hash
- Concise wave-close commit message at HEAD
