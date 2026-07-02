# W84C Result/Option Legacy Carrier Audit

Date: 2026-07-02
Branch: `strict-flip-w84c-result-option-legacy`
Base: `227764d0`

Scope:

- Primary writable files audited: `crates/shape-vm/src/type_tracking.rs`,
  `crates/shape-vm/src/executor/snapshot.rs`,
  `crates/shape-vm/src/executor/trait_object_ops.rs`,
  `crates/shape-runtime/src/wire_conversion.rs`.
- Read-only audit files included where needed for classification:
  `shape-value`, `shape-jit`, and VM `exceptions`, `modules`, `printing`,
  `result_option_carrier`.
- No cargo/rustc/nextest/just/Shape binaries were run.

## W87D Addendum: Post-W86B Containment Pass

Date: 2026-07-02
Branch: `strict-flip-w87d-result-option-legacy-containment`
Base: `2997b11b`

Cheap audit rerun after W86B confirms the old carriers still cannot be
deleted locally. Normal VM execution remains schema-backed, while old carriers
survive in compatibility and JIT-only surfaces:

| Area | Current W87D classification |
|---|---|
| Normal VM execution | Canonical producers are `result_option_carrier::build_ok/build_err/build_some/build_none`, which create fixed-layout `__Result` / `__Option` `TypedObjectStorage`. Owned normal VM paths did not show live `ResultData::ok/err` or `OptionData::some/none` producers outside tests. |
| Compatibility restore / wire decode | VM whole-snapshot restore without a kind track maps legacy `SerializableVMValue::ResultData/OptionData` to `Ptr(HeapKind::TypedObject)`. Runtime snapshot restore can still rebuild old `Arc<ResultData>` / `Arc<OptionData>` when explicitly called with expected `Ptr(HeapKind::Result/Option)`; that file is outside W87D write scope. Runtime wire projection still reads old `HeapKind::Result/Option` slots because JIT/snapshot compatibility producers remain. |
| JIT FFI legacy surfaces | `jit_v2_make_result_ok`, `jit_v2_make_result_err`, `jit_v2_make_option_some`, and `jit_v2_make_option_none` still allocate `Arc<ResultData>` / `Arc<OptionData>`. JIT retain/release, predicate, payload, print, stack-kind-code, and ownership paths still consume those old kinds. This is read-only for W87D. |
| Printing | Schema-backed `__Result` / `__Option` typed objects already format as `Ok` / `Err` / `Some` / `None`. Legacy `HeapKind::Result/Option` formatter arms remain compatibility consumers until old producers are gone. |
| Trait objects | Schema-backed `__Result` / `__Option` rewrap is live. Legacy `HeapKind::Result/Option` returns surface and drop their owned share without inspecting `ResultData` / `OptionData`, which is the correct containment posture. |
| Tests / storage tables | Old carrier tests and shape-value / VM stack clone-drop tables remain required while compatibility restore and JIT producers can still create old slots. |

W87D patch decision: contain, do not delete. `heap_value_to_wire` now projects
`HeapValue::Result(Arc<ResultData>)` and
`HeapValue::Option(Arc<OptionData>)` through the carrier's embedded
`KindedSlot` payload kind, matching the existing `HeapKind::Result/Option`
slot projection. This removes the opaque `<result:phase-2c>` /
`<option:phase-2c>` fallback from generic `HeapValue` wire projection without
creating old carriers, probing tags, or inferring from payload bits.

## Executive Classification

There is no safe W84C-local deletion of `HeapKind::Result`,
`HeapKind::Option`, `ResultData`, or `OptionData` from current HEAD.

The VM's normal runtime producers are already schema-backed typed objects:
`result_option_carrier.rs` documents the canonical carrier as fixed-layout
`TypedObjectStorage` (`__Result` / `__Option`) and exposes `build_ok`,
`build_err`, `build_some`, and `build_none`
(`crates/shape-vm/src/executor/result_option_carrier.rs:1-70`).
The builtins/modules/exceptions producers use those helpers
(`executor/vm_impl/builtins.rs:648-696`,
`executor/vm_impl/modules.rs:491-520`,
`executor/exceptions/mod.rs:592-657`).

Remaining old-carrier uses fall into four classes:

| Class | Sites | W84C classification |
|---|---|---|
| Live old producers | JIT FFI still allocates `Arc<ResultData>` / `Arc<OptionData>` in `jit_v2_make_result_ok`, `jit_v2_make_result_err`, `jit_v2_make_option_some`, and `jit_v2_make_option_none` (`crates/shape-jit/src/ffi/result.rs:278-315`). Runtime snapshot restore can also rebuild old carriers when explicitly called with expected kind `Ptr(HeapKind::Result/Option)` (`crates/shape-runtime/src/snapshot.rs:2053-2088`). | Not W84C writable. Active JIT producer and legacy snapshot compatibility producer. |
| Live old consumers | VM stack clone/drop, shape-value `KindedSlot` clone/drop, closure-layout capture clone/drop, VM printing, runtime wire projection, runtime snapshot serialization, JIT `arc_*` predicates/payload/retain/release/print/ownership, trait-object old-kind surface/drop. | Must remain until producers are gone. |
| Tests / compatibility | Type-tracking legacy descriptor tests (`type_tracking.rs:1483-1520`), VM snapshot legacy tests (`executor/snapshot.rs:1029-1134`), wire/printing/trait old-carrier tests, shape-value storage tests. | Keep until compatibility policy changes and producers are migrated. |
| Docs / comments / diagnostics | Compiler comments, exceptions module header comments, resume comments, method dispatch exclusions, diagnostic labels in arithmetic/comparison/typed access. | Remove only after code paths are gone. |

## Question 1: Remaining Use Classification

### Live Producers

VM user-facing constructors are no longer old-carrier producers:

- `Some`, `Ok`, and `Err` builtins push schema-backed typed objects through
  `result_option_carrier` (`executor/vm_impl/builtins.rs:648-696`).
- Typed module returns produce schema-backed typed objects
  (`executor/vm_impl/modules.rs:491-520`).
- Error-context wrapping and `?`-related paths build schema-backed typed
  objects (`executor/exceptions/mod.rs:592-657`).

Remaining live old producers are outside W84C write scope:

- JIT FFI `jit_v2_make_result_ok`, `jit_v2_make_result_err`,
  `jit_v2_make_option_some`, and `jit_v2_make_option_none` allocate
  `Arc<ResultData>` / `Arc<OptionData>` and return raw bits stamped by callers
  as `Ptr(HeapKind::Result/Option)` (`crates/shape-jit/src/ffi/result.rs:278-315`).
- Runtime snapshot restore can rebuild old carriers when the caller requests
  expected `HeapKind::Result` or `HeapKind::Option`
  (`crates/shape-runtime/src/snapshot.rs:2053-2088`). In the VM whole-snapshot
  restore path, old `SerializableVMValue::ResultData/OptionData` without a kind
  track are mapped to `Ptr(HeapKind::TypedObject)` instead
  (`crates/shape-vm/src/executor/snapshot.rs:670-682`), so this is a legacy
  compatibility producer rather than the active VM restore path.

### Live Consumers

- `shape-value` still defines `ResultData` / `OptionData` and the
  `KindedSlot::from_result/from_option` convenience constructors
  (`crates/shape-value/src/heap_value.rs:2639-2716`,
  `crates/shape-value/src/kinded_slot.rs:429-441`). Clone/drop dispatch for
  old carrier kinds remains in shape-value and VM stack code.
- Runtime wire conversion reads old carriers in `slot_to_wire` when the
  supplied kind is `Ptr(HeapKind::Result/Option)`
  (`crates/shape-runtime/src/wire_conversion.rs:165-194`). The reverse path
  does not create old carriers for public wire `Result`/`Null`; it creates
  schema-backed typed objects (`wire_conversion.rs:860-865`, `909-914`).
- Runtime snapshot serialization reads old carriers into
  `SerializableVMValue::ResultData/OptionData`
  (`crates/shape-runtime/src/snapshot.rs:1294-1320`).
- VM printing reads old carriers for display compatibility
  (`crates/shape-vm/src/executor/printing.rs:548-571`).
- Trait object return rewrap checks old carrier kinds only to surface and drop
  the share, not to inspect old carrier payloads
  (`crates/shape-vm/src/executor/trait_object_ops.rs:853-855`,
  `1060-1090`).
- JIT consumers include `jit_arc_result_*`, `jit_arc_option_*`, JIT ownership
  retain/release dispatch, JIT printing, and variant rvalue extraction. These
  are read-only for W84C.

### Tests, Legacy Compatibility, Docs

Tests intentionally constructing old carriers remain in:

- `type_tracking.rs` legacy descriptor tests (`1483-1520`).
- `executor/snapshot.rs` old-carrier snapshot round-trip and legacy restore
  tests (`1029-1134`).
- `wire_conversion.rs` WS-3 F2b wire tests and typed-object wire tests.
- `printing.rs` and `trait_object_ops.rs` compatibility/surface tests.
- `shape-value` storage/refcount tests.

Docs/comments-only references remain in compiler pattern comments, exception
module header comments, resume comments, and diagnostic label maps. They are
not producers.

## Question 2: Frame Return Metadata

Yes. `FrameDescriptor` has the structural discriminator already:
`return_wrapper: FrameReturnWrapper` (`type_tracking.rs:183-186`), and the
compiler stamps this separately from the ABI carrier kind. New descriptors
should use `return_kind = Ptr(HeapKind::TypedObject)` plus
`return_wrapper = Option/Result` (`type_tracking.rs:155-160`).

However, `effective_return_wrapper()` and `abi_return_kind()` still accept old
serialized descriptors that encoded wrapper semantics as
`return_kind = Ptr(HeapKind::Option/Result)` (`type_tracking.rs:231-263`).
This is a compatibility discriminator, not a new producer.

Blocker to deleting that fallback in W84C:

- `effective_return_wrapper()` is used by `op_try_unwrap`'s None propagation
  path for old descriptors (`executor/exceptions/mod.rs:873-880`).
- Read-only JIT has an explicit legacy normalization test that constructs a
  `FrameDescriptor` with `return_kind = Ptr(HeapKind::Result)` and relies on
  `abi_return_kind()` normalizing it to `Ptr(HeapKind::TypedObject)`
  (`crates/shape-jit/src/mir_compiler/v2_call_abi.rs:93-104`).
- Old serialized `FunctionBlob` descriptors can deserialize with
  `return_wrapper = Unknown`; the current type-tracking test covers that
  (`type_tracking.rs:1509-1520`).

Local patch decision: no code patch. The structural discriminator is already
present and used by new producers. Removing the fallback requires a JIT test
and serialized-descriptor compatibility policy change outside W84C.

## Question 3: Snapshot / Restore / Wire Conversion

Snapshot/restore still reads and can create old carriers in compatibility
paths:

- Runtime snapshot defines `SerializableVMValue::ResultData` and
  `SerializableVMValue::OptionData` (`crates/shape-runtime/src/snapshot.rs:481-497`).
- Serialization reads old `Ptr(HeapKind::Result/Option)` slots into those
  serializable arms (`snapshot.rs:1294-1320`).
- Generic runtime restore can recreate old `Arc<ResultData>` /
  `Arc<OptionData>` if expected kind is `HeapKind::Result/Option`
  (`snapshot.rs:2053-2088`).
- Generic runtime restore can also translate those same serializable arms into
  schema-backed typed objects when expected kind is `HeapKind::TypedObject`
  (`snapshot.rs:2125-2145`).
- VM whole-snapshot restore without an explicit kind track maps old
  `ResultData` / `OptionData` serializable arms to `Ptr(HeapKind::TypedObject)`
  (`crates/shape-vm/src/executor/snapshot.rs:670-682`). The L5 test asserts
  that old snapshots restore as typed objects (`executor/snapshot.rs:1109-1134`).

Wire conversion:

- `slot_to_wire` reads old carriers when the caller gives old carrier kinds
  (`crates/shape-runtime/src/wire_conversion.rs:165-194`). This is a live
  consumer because old JIT/runtime snapshot producers still exist.
- `wire_to_slot` creates schema-backed typed objects for `WireValue::Result`
  and `WireValue::Null` expected as `Ptr(HeapKind::TypedObject)`
  (`wire_conversion.rs:860-865`), and recursive result payload projection also
  builds typed objects (`wire_conversion.rs:909-914`).
- There is no W84C-owned active wire producer of old `OptionData`/`ResultData`.

## Question 4: Trait Object Ops

Trait object ops do not inspect old `ResultData` / `OptionData` payloads.
They match old result/option heap kinds only to retire the share and return a
structured `SURFACE` (`executor/trait_object_ops.rs:853-855`,
`1060-1090`).

That surface is correct until schema-backed typed-object carriers are universal:

- The live rewrap path for Result/Option descends through
  `result_option_carrier::read_result/read_option` on
  `Ptr(HeapKind::TypedObject)` (`trait_object_ops.rs:1029-1048`).
- Re-introducing old `ResultData` / `OptionData` inspection here would make
  trait-object rewrap a second active old-carrier consumer, which contradicts
  the L5 carrier cleanup direction already encoded in the error messages.

## Exact Safe Deletion Order From Current HEAD

1. **Eliminate active old producers.**
   Migrate read-only JIT `jit_v2_make_result_*` / `jit_v2_make_option_*` and
   their MIR lowering sites to schema-backed `__Result` / `__Option`
   `TypedObjectStorage`, or make old-carrier-producing JIT paths explicit
   deopt/surface. In parallel, decide whether runtime snapshot
   `serializable_to_slot(..., Ptr(HeapKind::Result/Option))` remains a public
   compatibility API or should restore typed objects / refuse old expected
   kinds.

2. **Move snapshot compatibility to typed-object-only restore.**
   Keep reading `SerializableVMValue::ResultData/OptionData`, but make every
   restore path produce `Ptr(HeapKind::TypedObject)` or a structured
   compatibility error. This includes nested typed-object field restore, where
   `expected_heap_field_kind` currently maps `ResultData` / `OptionData` back
   to old heap kinds.

3. **Delete old JIT consumers.**
   Remove `jit_arc_result_*`, `jit_arc_option_*`, old Result/Option stack kind
   code mappings, JIT ownership retain/release arms, print arms, MIR enum
   payload consumers, and legacy HK_OK/HK_ERR/HK_SOME bridges only after step 1
   leaves no old JIT producer.

4. **Delete VM/runtime compatibility consumers.**
   Remove old-carrier reads from runtime `slot_to_wire`, runtime snapshot
   serialization, VM printing, VM stack clone/drop, trait-object surface/drop
   arms, method-dispatch exclusions, and diagnostic label maps once no old slots
   can be produced or restored.

5. **Remove frame-metadata fallback.**
   Keep `FrameDescriptor.return_wrapper`; delete only the fallback that derives
   wrapper semantics from `return_kind = Ptr(HeapKind::Option/Result)`. Update
   the JIT `v2_call_abi` legacy normalization test and any serialized-descriptor
   compatibility tests at the same time.

6. **Remove core carrier definitions last.**
   Delete `ResultData`, `OptionData`, `HeapKind::Result`, `HeapKind::Option`,
   `HeapValue::Result`, `HeapValue::Option`, `ValueSlot::from_result`,
   `ValueSlot::from_option`, `KindedSlot::from_result`, `KindedSlot::from_option`,
   and all clone/drop/retain/release dispatch-table entries in one final
   lockstep commit. Run the heap-kind exhaustive guard after this step.

## Recommended Supervisor Guards

For the W87D containment commit:

- `rg -n "\bOptionData\b|\bResultData\b|HeapKind::Option|HeapKind::Result|HeapValue::Option|HeapValue::Result" crates/shape-vm/src crates/shape-runtime/src crates/shape-value/src crates/shape-jit/src`
- `rustfmt --edition 2024 crates/shape-runtime/src/wire_conversion.rs`
- `git diff --check`
- No cargo/rustc/nextest/just/Shape binary lane was used in W87D; the global
  lane remains supervisor-owned.

Focused supervisor verification recommended for this containment patch:

- `cargo test -p shape-runtime --lib wire_conversion --no-fail-fast`
- `cargo test -p shape-vm --lib printing trait_object_ops snapshot --no-fail-fast`
- A VM/JIT differential seed with top-level `Ok` / `Err` / `Some` / `None`
  returns, because JIT FFI old producers remain outside W87D scope.

For the future deletion wave, the minimum guards should be:

- `rg -n "\bOptionData\b|\bResultData\b|HeapKind::Option|HeapKind::Result|NativeKind::Ptr\(HeapKind::Option|NativeKind::Ptr\(HeapKind::Result" crates/shape-vm/src crates/shape-runtime/src crates/shape-value/src crates/shape-jit/src`
- `bash scripts/check-heapkind-wildcards.sh`
- `cargo test -p shape-vm --lib result_option_carrier trait_object_ops printing snapshot exceptions --no-fail-fast`
- `cargo test -p shape-runtime --lib wire_conversion snapshot --no-fail-fast`
- `cargo test -p shape-value --lib kinded_slot heap_value --no-fail-fast`
- `cargo test -p shape-jit --lib ffi::result mir_compiler::v2_call_abi mir_compiler::rvalues mir_compiler::terminators mir_compiler::ownership --no-fail-fast`
- A focused Shape-level `Result`/`Option` gate covering top-level `Ok`/`Err`,
  `Some`/`None`, `?`, `!!`, trait-object boxed returns, snapshot restore, and
  VM/JIT differential seeds that exercise `Result`/`Option` returns.
