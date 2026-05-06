# Defections Log

A running record of considered-but-rejected compromises in the strict-typing work (`~/.claude/plans/stop-native-vs-tagged-tax.md`). Future sessions read this to recognize the pattern in real time.

## Why this log exists

The `v2-nanbox-removal-plan.md` Step 6 ("delete `ValueWord`") was originally a one-line deletion. Mid-execution it was renamed to "ValueBits shim retained as FFI-boundary bridge" and became permanent. That single rationalization compounded into ~6 weeks of W-series cleanup, deferred v2-raw-heap aliasing tests, ignored shape-jit tests, and ~48 shape-test failures.

Rationalizations sound reasonable in the moment. They look obvious in hindsight. This log captures them while they're fresh so the next session can spot the same shape faster.

## How to use

When you (agent or human) consider a fallback / shim / bridge / decode hop / "follow-up" disposition for the strict-typed work, **before** implementing it, log the consideration here. Even if you ultimately reject it. Logging takes 60 seconds; the discipline pays back when the next session reads the log on day one.

Cross-reference: `shape/CLAUDE.md` "Forbidden Patterns" section enumerates the patterns. This log records the *attempts* at those patterns.

## Format

```
## YYYY-MM-DD — <one-line summary>

**Considered:** <what you almost did>

**Rationalization:** <why it sounded reasonable in the moment>

**Pattern recognized:** <which forbidden pattern from CLAUDE.md this matches>

**Alternative taken:** <what you did instead>

**Cost saved:** <estimated days/weeks of W-series-style cleanup avoided>
```

## Historical defections (pre-log, reconstructed)

These were not logged at the time. Reconstructed from commit history and plan archaeology so the pattern is on record.

### 2026-04-18 — `v2-nanbox-removal-plan.md` Step 6 quietly downgraded

**Considered:** delete `crates/shape-value/src/value_word.rs`, replace with `pub type ValueWord = u64`, no methods.

**Rationalization:** "comptime, polyglot, and unproven-type sites need a dynamic representation; retain `ValueBits` shim as documented FFI-boundary bridge."

**Pattern recognized:** "Rename to a less suspicious name" (`ValueBits shim`, `FFI-boundary bridge`).

**Alternative taken (at the time):** retained `ValueWord` as ~2,650-line "dynamic fallback". Plan status edited from "delete `ValueWord`" to "ValueBits shim landed; dynamic-fallback residuals tracked".

**Actual cost incurred:** the W-series (W1–W4, α/δ follow-ups, 9 commits over multiple sessions); 4 deferred v2-raw-heap aliasing tests; ~48 shape-test failures in the same bug class; ~23 ignored shape-jit tests. Estimate: 4–6 weeks of cumulative cleanup that this rename made permanent. Resulting plan (`stop-native-vs-tagged-tax.md`) reverses the decision and bulldozes first.

### 2026-05-05 — W4-δ `ConvertBoolToString` opcode

**Considered:** add a dedicated `ConvertBoolToString` opcode to handle `bool as string` casts at runtime.

**Rationalization:** "the existing convert path loses type info; one new opcode is small and surgical (74 LoC, 1 test closed)."

**Pattern recognized:** "Add a new opcode for this specific conversion" — a `Convert<X>To<Y>` opcode added to paper over a compiler kind-tracker gap.

**Alternative taken (at the time):** the new opcode was added (commit `3fa7456`).

**Should have done:** fix the compiler so the convert path doesn't lose type info. The bool source's kind was statically knowable at the convert site; `last_emitted_native_kind` had a propagation gap.

**Cost incurred:** one more opcode in `OpCode` enum; another decode site to delete in Phase 1 of the strict-typing bulldozer.

---

## 2026-05-06 — heap_value.rs Phase-2 reconstruction: rejected `u64` and `HeapValue` substitution

**Considered (option A):** mechanically replace every `ValueWord` reference in `crates/shape-value/src/heap_value.rs` and `heap_variants.rs` with `u64`. This unblocks the file compile fastest. The 13 heap-side data structures (`HashMapData`, `SetData`, `DequeData`, `PriorityQueueData`, `IteratorState`, `IteratorTransform`, `GeneratorState`, `ConcurrencyData`, `SimulationCallData`, `RefProjection::Index`, `ProjectedRefData`) keep their shape; the `Some`/`Ok`/`Err`/`Range`/`TraitObject`/`FunctionRef` variants keep their `Box<u64>` payloads.

**Rationalization (option A):** "It's the smallest mechanical change. Drop/Clone impls keep working — the `vw_clone`/`vw_drop` calls become bare bit copies. We can move on to shape-vm and clean up the semantics later."

**Pattern recognized (option A):** classic compromise pattern — keep the dynamic data structures, just rename the type to `u64` so it looks like primitive bits. The Drop/Clone refcount semantics quietly break (no longer paired retain/release on heap pointers stored in collections), and now the codebase has live ref leaks / double-frees in collection paths. This is option A from `~/.claude/plans/strict-typing-phase-2-handover.md`'s analysis. It is dynamic-runtime semantics rebranded as typed bits — the W-series footgun.

**Considered (option B):** substitute `Vec<HeapValue>` for `Vec<ValueWord>` and `Box<HeapValue>` for `Box<ValueWord>` throughout the heap-side data structures. The hetero-collections (`HashMapData`, etc.) stay, just become typed sum-type holding `HeapValue` recursively.

**Rationalization (option B):** "It's strict-typed in the sense that `HeapValue` is a typed enum. The collections become heterogeneous typed-sum-type containers, which is what the plan literature describes as the canonical encoding for heterogeneous data."

**Pattern recognized (option B):** misreads the plan. Heterogeneous collections aren't strict-typed in any meaningful sense — they preserve dynamic dispatch by promoting the runtime-tag-decode dispatch from `ValueWord`'s tag bits to the `HeapValue` enum's discriminant. The dispatch site in `find_key`/`contains`/`vw_hash` doesn't get cheaper; it just dispatches on `match heap_value { ... }` instead of `match tag { ... }`. The `runtime-v2-spec.md:180` direction (monomorphized typed buckets per `HashMap<K, V>` instantiation) is incompatible with this representation. Picking B locks in heterogeneous-by-default at the heap level, which is the very thing strict-typing exists to remove.

**Alternative taken (option C):** delete every HeapValue variant whose payload depends on `ValueWord` or holds a heterogeneous-typed collection. The variants `Some`/`Ok`/`Err`/`Range`/`TraitObject`/`FunctionRef`/`HashMap`/`Set`/`Deque`/`PriorityQueue`/`Iterator`/`Generator`/`ProjectedRef`/`Concurrency`/`SimulationCall` are removed from `HeapValue` along with their `*Data` structs. The cascade surfaces every consumer in shape-vm/shape-runtime/shape-jit; they will be redesigned as monomorphized typed structures (typed buckets for HashMap, monomorphized `Option<T>` / `Result<T, E>` / `Range<T>` as TypedStructs) in a later phase or as part of the cascade fix.

**Cost saved:** option A would have rebuilt the `vw_clone`/`vw_drop` machinery within months under a different name (the W-series pattern reproduced). Option B would have locked in heterogeneous-by-default heap representation, blocking the v2 typed-buckets migration. Option C aligns the bulldozer with `runtime-v2-spec.md`'s direction. Estimated avoided cost: 4–8 weeks of follow-up cleanup. Acknowledged immediate cost: significantly larger Phase 2 cascade in shape-vm.

## 2026-05-06 — shape-runtime Phase-2 reconstruction: TypedReturn ValueWord hatches deleted

**Considered:** retain `TypedReturn::ValueWord(ValueWord)`, `TypedReturn::ArrayValueWord(Vec<ValueWord>)`, and `TypedReturn::HashMapValueWord { keys, values }` as escape hatches in the typed stdlib return ABI, mechanically substituting `ValueWord` → `u64` so they compile against the post-bulldozer shape-value crate.

**Rationalization:** "TypedReturn already documents these as 'escape hatches narrowed module-by-module across the migration' (`typed_module_exports.rs:124-130`). They were never load-bearing for the typed return shapes; renaming the inner type to `u64` (with an attached `NativeKind` discriminator if needed) is mechanically the smallest change to keep the marshalling layer compiling. Each consumer (set/parallel/parsers/plugins) has a known follow-up workstream — we'd be honest about the deferral."

**Pattern recognized:** classic W-series rename. The variants exist precisely *because* the function bodies need a polymorphic return — `set_module` returns the user's element type, parsers return arbitrary user data trees, the plugin ABI is by definition opaque. Substituting `ValueWord` for `u64` does not remove the polymorphism; it relabels it. The `into_value_word()` marshalling boundary then has to dispatch on whatever kind discriminator `u64` carries, which means reintroducing tag-decode dispatch under a different name. This is "Rename to a less suspicious name" from the CLAUDE.md forbidden list, applied to the return-type ABI.

**Alternative taken:** delete `TypedReturn::ValueWord`, `TypedReturn::ArrayValueWord`, and `TypedReturn::HashMapValueWord`. `HashMapValueWord` has zero callers (already dead). For `ValueWord`/`ArrayValueWord`, every consumer falls into one of three buckets (audited 2026-05-06):

1. **Mechanically migratable** (13 sites in http/archive/csv/regex/arrow): use existing `TypedReturn::ObjectPairs` / `ArrayObjectPairs` / `DataTable` variants. Done in Step 3.
2. **Architecturally cut** (set/parallel/plugin): see follow-up entries below — the modules are deleted from the strict-typed build with explicit follow-up workstreams.
3. **Architecturally rebuilt** (json/yaml/toml/msgpack/xml parsers): see `JsonValue` entry below — typed sum-type enum replaces ValueWord-tree return.

**Cost saved:** keeping the hatches would have forced the marshalling boundary to carry `NativeKind` per `u64`, reproducing `ValueBits`-shim machinery under the `TypedReturn` enum. Estimate 2–4 weeks of follow-up cleanup avoided. Acknowledged immediate cost: ~30 consumer sites to migrate or delete, plus 4 follow-up workstreams logged.

---

## 2026-05-06 — shape-runtime: `set_module` deleted from strict-typed build

**Considered:** keep `crates/shape-runtime/src/stdlib/set_module.rs` and rename its `TypedReturn::ValueWord` returns to `TypedReturn::RawBits { kind, bits }` (or equivalent). The eight `Set<T>` operations (new/insert/delete/contains/union/intersect/difference/to_array) all return either a `Set` heap object or its element type, both of which are user-parametric.

**Rationalization:** "Sets are fundamental container types and shipping a strict-typed compile without `Set` is a feature regression. A `RawBits` discriminator wrapper around the existing implementation preserves the API."

**Pattern recognized:** `Set<T>` is parametric in element type. The strict-typed answer per `runtime-v2-spec.md:180` is monomorphized per-instantiation typed buckets — the same shape as the typed-`HashMap<K, V>` direction. A `RawBits` wrapper keeps the heterogeneous-by-default dispatch alive under a new name (the option-B pattern from heap_value.rs reconstruction, applied to a different layer). It also preserves the `HashMapValueWord`-shaped storage that the bulldozer just deleted from `HeapValue` — re-creating in stdlib what the bulldozer removed from the runtime would be the W-series defection in a different file.

**Alternative taken:** delete `crates/shape-runtime/src/stdlib/set_module.rs` and remove its registration from the stdlib registry. Add a follow-up workstream `set-module-strict-monomorphization` to `CLAUDE.md`'s "Known Constraints" section: rebuild Set as monomorphized per-element-type buckets when the compiler can pin element type at the registration site (same prerequisite as typed-buckets `HashMap`).

**Cost saved:** the `RawBits` rename would compound across the typed-collections subsystem (Deque, PriorityQueue, … all already deleted from `HeapValue` for the same reason). Honest deletion makes the absence visible; a renamed wrapper would hide the gap. Estimate: 2-week monomorphization workstream deferred, but cleanly. Acknowledged user-visible cost: `import { Set } from std::core::collections` stops working until the workstream lands.

---

## 2026-05-06 — shape-runtime: `parallel` module deleted from strict-typed build

**Considered:** keep `crates/shape-runtime/src/stdlib/parallel.rs` (parallel_map/filter/chunks/reduce/sort over a user closure) and have its `TypedReturn::ValueWord` returns dispatch on the closure's runtime return kind.

**Rationalization:** "Parallel collection ops are a perf headline feature. Closures already return `ValueWord`-shaped values via the VM call convention; the `parallel_*` wrapper just threads them through. A small dispatch on the closure's last-emitted kind is enough to pick the right typed marshal."

**Pattern recognized:** "small dispatch on the closure's last-emitted kind" is `last_program_return_kind` reborn — exactly the Pattern A defection that bulldozer commit `90fc2e9` removed. The closure return type is parametric; without monomorphizing the call wrapper per closure-return-type, any solution at the stdlib layer is dynamic dispatch on a kind discriminator. Identical structural shape to the `set_module` case.

**Alternative taken:** delete `crates/shape-runtime/src/stdlib/parallel.rs` and remove its registration. Add `parallel-module-strict-monomorphization` follow-up workstream alongside `set-module-strict-monomorphization`. Both share the same prerequisite (compiler pins element/return type at the registration site); they should land together.

**Cost saved:** preserved the bulldozer-deleted `last_program_return_kind` infrastructure from sneaking back in through the stdlib closure-call wrapper. Estimate: 1–2 week parallel-monomorphization workstream deferred. Acknowledged user-visible cost: `parallel_map`/`parallel_filter`/etc. unavailable until rebuilt.

---

## 2026-05-06 — shape-runtime: plugin native-call passthrough disabled

**Considered:** preserve `plugins/module_capability.rs:155` (`Result<ValueWord> → TypedReturn::ValueWord` passthrough) by routing the plugin's return through the renamed `RawBits` discriminator, since the plugin ABI is by definition opaque to the host runtime.

**Rationalization:** "The plugin returns whatever it wants — there is no static type for that. A passthrough `RawBits` is genuinely all the host can know."

**Pattern recognized:** the same dispatch-by-rename pattern. "The plugin ABI is opaque" is true today *because* it was designed to thread `ValueWord` through. The strict-typed answer is that plugins must declare typed signatures at registration, just like the typed-stdlib already does. Keeping a `RawBits` passthrough makes the typed registration optional — and optional defection mechanisms reliably become the default.

**Alternative taken:** delete the `TypedReturn::ValueWord` line at `plugins/module_capability.rs:155`. The single call site is the optional plugin native-call dispatcher; disabling it means plugins that registered for native-call routing no longer dispatch through this path. Add `plugin-typed-abi` follow-up workstream to `CLAUDE.md` Known Constraints. Plugins are not load-bearing for the strict-typed compile (extensions/python and extensions/typescript flow through `LanguageRuntimeVTable`, which is unaffected — `docs/strictly-typed-baseline.md:36` documents 0 ValueWord references in either extension).

**Cost saved:** prevented the optional-defection-becomes-default dynamic. Estimate: 1-week plugin typed-ABI workstream deferred. Acknowledged user-visible cost: the specific `register_plugin_native_call` codepath is non-functional until rebuilt; the broader plugin system remains intact.

---

(Add new entries above this line. Newest first.)
