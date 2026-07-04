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
| Compatibility restore / wire decode | VM whole-snapshot restore without a kind track maps legacy `SerializableVMValue::ResultData/OptionData` to `Ptr(HeapKind::TypedObject)`. W88B extends that policy to explicit old expected `Ptr(HeapKind::Result/Option)` restore, so runtime snapshot restore no longer rebuilds old `Arc<ResultData>` / `Arc<OptionData>` carriers. Runtime wire projection still reads old `HeapKind::Result/Option` slots for compatibility and tests, but no W88-normalized restore path creates them. |
| JIT FFI legacy surfaces | At W87D time, `jit_v2_make_result_ok`, `jit_v2_make_result_err`, `jit_v2_make_option_some`, and `jit_v2_make_option_none` still allocated `Arc<ResultData>` / `Arc<OptionData>`; W88A supersedes this producer classification. JIT retain/release, predicate, payload, print, stack-kind-code, and ownership paths still consume old kinds for compatibility, but the JIT old producer path now deopts/fails closed before allocation. |
| Printing | Schema-backed `__Result` / `__Option` typed objects already format as `Ok` / `Err` / `Some` / `None`. Legacy `HeapKind::Result/Option` formatter arms remain compatibility consumers until old producers are gone. |
| Trait objects | Schema-backed `__Result` / `__Option` rewrap is live. Legacy `HeapKind::Result/Option` returns surface and drop their owned share without inspecting `ResultData` / `OptionData`, which is the correct containment posture. |
| Tests / storage tables | Old carrier tests and shape-value / VM stack clone-drop tables remain required while compatibility restore and, at W87D time, JIT producers could still create old slots. |

W87D patch decision: contain, do not delete. `heap_value_to_wire` now projects
`HeapValue::Result(Arc<ResultData>)` and
`HeapValue::Option(Arc<OptionData>)` through the carrier's embedded
`KindedSlot` payload kind, matching the existing `HeapKind::Result/Option`
slot projection. This removes the opaque `<result:phase-2c>` /
`<option:phase-2c>` fallback from generic `HeapValue` wire projection without
creating old carriers, probing tags, or inferring from payload bits.

## W88B Addendum: Snapshot Restore Policy

Date: 2026-07-02
Branch: `strict-flip-w88b-snapshot-result-option-policy`
Base: `46608080`

W88B closes the runtime snapshot-restore producer described in W87D. Legacy
`SerializableVMValue::ResultData` / `SerializableVMValue::OptionData` are still
accepted as wire-format compatibility arms, but restore is typed-object-only:

- `serializable_to_slot(..., Ptr(HeapKind::TypedObject))` builds the existing
  schema-backed `__Result` / `__Option` `TypedObjectStorage` carriers.
- `serializable_to_slot(..., Ptr(HeapKind::Result/Option))` is treated as an
  old persisted kind-track compatibility input and also returns
  `Ptr(HeapKind::TypedObject)`. It does not allocate `Arc<ResultData>` or
  `Arc<OptionData>`.
- Nested `TypedObject` field restore now maps legacy `ResultData` /
  `OptionData` arms to `Ptr(HeapKind::TypedObject)`, so snapshot bytes cannot
  recreate old carriers through `expected_heap_field_kind`.

No runtime inference, tag probing, or kind-from-bits was added. Snapshot
serialization and wire projection remain compatibility consumers while old
carrier definitions and tests exist.

## W88A Addendum: JIT Producer Containment

Date: 2026-07-02
Branch: `strict-flip-w88a-jit-result-option-typed-object`
Base: `46608080`

W88A contains the active JIT old-carrier producers without changing runtime
snapshot/wire policy:

- `crates/shape-jit/src/mir_compiler/statements.rs` now deopts Result/Option
  `EnumStore` construction for `Ok`, `Err`, `Some`, and `None` before emitting
  the old FFI imports. The error explicitly requires a future schema-backed
  `__Result` / `__Option` TypedObject helper ABI with statically known schema
  ids.
- `crates/shape-jit/src/ffi/result.rs` keeps the four symbol names
  (`jit_v2_make_result_ok`, `jit_v2_make_result_err`,
  `jit_v2_make_option_some`, `jit_v2_make_option_none`) registered only as
  stale-reference backstops. Each body fails closed before allocating
  `Arc<ResultData>` / `Arc<OptionData>`.
- JIT legacy consumers (`jit_arc_result_*`, `jit_arc_option_*`, print,
  ownership retain/release, stack kind decode) remain compatibility surfaces
  until snapshot/wire compatibility and old-carrier tests are retired.

No cargo/rustc/nextest/just/Shape binary lane was run by W88A.

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
| Live old producers | Normal VM constructors are schema-backed. Runtime snapshot restore no longer rebuilds old carriers as of W88B; old expected `Ptr(HeapKind::Result/Option)` inputs normalize to `Ptr(HeapKind::TypedObject)`. W88A contains the former JIT FFI producers: `EnumStore` deopts before emitting them, and the four FFI bodies fail closed before allocation. | Active runtime/JIT old-carrier producer allocation is closed/contained. Old definitions and test/direct constructors remain until consumers are deleted. |
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

Remaining old-carrier producer status:

- Runtime snapshot restore was a producer before W88B. It now treats expected
  `HeapKind::Result` / `HeapKind::Option` as old persisted kind-track
  compatibility inputs and returns schema-backed `Ptr(HeapKind::TypedObject)`
  carriers instead. In the VM whole-snapshot restore path, old
  `SerializableVMValue::ResultData/OptionData` without a kind track were
  already mapped to `Ptr(HeapKind::TypedObject)`; W88B extends the same policy
  to snapshots that do carry old stack/module kind tracks.
- W88A contains the former JIT FFI producer path. `EnumStore` for `Ok`, `Err`,
  `Some`, and `None` now surfaces before FFI call emission, and direct calls to
  `jit_v2_make_result_ok`, `jit_v2_make_result_err`,
  `jit_v2_make_option_some`, or `jit_v2_make_option_none` fail closed before
  any old-carrier allocation. These symbols remain registered only so stale
  `FFIFuncRefs` resolve to the explicit surface.
- Tests and direct library constructors can still deliberately allocate
  `ResultData` / `OptionData`; those remain compatibility fixtures until the
  old consumers and core carrier definitions are deleted.

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

Snapshot/restore still reads old carriers in compatibility paths, but W88B
removed the runtime restore path that recreated them:

- Runtime snapshot defines `SerializableVMValue::ResultData` and
  `SerializableVMValue::OptionData` (`crates/shape-runtime/src/snapshot.rs:481-497`).
- Serialization reads old `Ptr(HeapKind::Result/Option)` slots into those
  serializable arms (`snapshot.rs:1294-1320`).
- Generic runtime restore translates those same serializable arms into
  schema-backed typed objects when expected kind is `HeapKind::TypedObject`,
  `HeapKind::Result`, or `HeapKind::Option`. The old expected kinds are
  compatibility inputs only; restore returns `Ptr(HeapKind::TypedObject)`.
- Nested `TypedObject` field restore now maps old `ResultData` / `OptionData`
  serializable arms to `Ptr(HeapKind::TypedObject)`, closing the old-carrier
  recreation path through `expected_heap_field_kind`.
- VM whole-snapshot restore with or without an explicit old stack/module kind
  track restores old `ResultData` / `OptionData` serializable arms as
  `Ptr(HeapKind::TypedObject)`. Focused W88B tests assert that old stack kinds
  do not recreate `Arc<ResultData>` / `Arc<OptionData>`.

Wire conversion:

- `slot_to_wire` reads old carriers when the caller gives old carrier kinds
  (`crates/shape-runtime/src/wire_conversion.rs:165-194`). This remains a live
  compatibility consumer because tests and direct old-carrier constructors
  still construct them deliberately.
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
   W88A has made old-carrier-producing JIT paths explicit deopt/surface before
   allocation. W88B has made runtime snapshot restore typed-object-only for old
   `ResultData` / `OptionData` arms, including old expected
   `Ptr(HeapKind::Result/Option)` inputs. Active old-carrier producer
   allocation is therefore closed/contained, but the core old definitions and
   compatibility constructors remain until consumers are deleted.

2. **Keep snapshot compatibility typed-object-only.**
   W88B completed the runtime restore side: `SerializableVMValue::ResultData`
   / `OptionData` restore to `Ptr(HeapKind::TypedObject)` for typed-object,
   old Result, old Option, and nested field cases. Future compatibility work
   must preserve that invariant until the serializable arms can be deleted.

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
  returns, because JIT FFI old producers were outside W87D scope.

For the W88A JIT producer containment commit:

- `rg -n "Arc::new\\((ResultData|OptionData)::|ResultData::(ok|err)|OptionData::(some|none)" crates/shape-jit/src/ffi/result.rs crates/shape-jit/src/mir_compiler/statements.rs crates/shape-jit/src/ffi_symbols/object_symbols.rs`
- `rg -n "jit_v2_make_result_ok|jit_v2_make_result_err|jit_v2_make_option_some|jit_v2_make_option_none" crates/shape-jit/src`
- `rustfmt --edition 2024 crates/shape-jit/src/ffi/result.rs crates/shape-jit/src/mir_compiler/statements.rs crates/shape-jit/src/ffi_refs.rs crates/shape-jit/src/ffi_symbols/object_symbols.rs crates/shape-jit/src/ffi/conversion.rs crates/shape-jit/src/ffi/value_ffi.rs crates/shape-jit/src/compiler/ffi_builder.rs crates/shape-jit/src/ffi/v2/collection_arc.rs`
- `git diff --check`
- No cargo/rustc/nextest/just/Shape binary lane was used in W88A; the global
  lane remains supervisor-owned.

Focused supervisor verification recommended for W88A:

- `cargo test -p shape-jit --lib ffi::result --no-fail-fast`
- `cargo test -p shape-jit --lib mir_compiler::statements --no-fail-fast`
- A Shape-level JIT seed for each constructor (`Ok(1)`, `Err("e")`,
  `Some(1)`, `None`) should show JIT deopt/fallback or a structured SURFACE,
  not old `Ptr(HeapKind::Result/Option)` allocation.

For the W88B snapshot-restore policy commit:

- `rustfmt --edition 2024 crates/shape-runtime/src/snapshot.rs crates/shape-vm/src/executor/snapshot.rs`
- `git diff --check`
- `cargo test -p shape-runtime --lib --no-fail-fast l5_typed_object_result_option_snapshot_tests`
- `cargo test -p shape-vm --lib --no-fail-fast test_w17_snapshot_result_option_roundtrip_normalizes_legacy_carriers`
- `cargo test -p shape-vm --lib --no-fail-fast test_l5_legacy_result_option_snapshot_restore_uses_typed_objects_without_kind_track`

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
