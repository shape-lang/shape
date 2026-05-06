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

## 2026-05-06 — shape-runtime parsers: typed `JsonValue` over ValueWord-tree return

**Considered (option α):** make `parse_json(s: string) -> ValueWord` (and parallels for yaml/toml/msgpack/xml) return a `ValueWord` whose tag bits encode the parsed shape (string/number/bool/array/object). The stdlib body would build the tree by `ValueWord::from_*` and `from_hashmap_pairs` — unchanged from pre-bulldozer code modulo the `ValueWord` type alias.

**Rationalization (option α):** "Parsers return arbitrary user data — there is *literally* no static type for the result of `parse_json` because the input can be anything. A `ValueWord`-tree return is honest about that. Trying to introduce a typed enum is just rebranding the same dynamic dispatch."

**Pattern recognized (option α):** confuses "the input is dynamic" with "the runtime representation must be dynamic." JSON's own specification has six concrete value kinds (null/bool/number/string/array/object) and pattern-matching on those six is exactly the strict-typed answer the plan calls out (`stop-native-vs-tagged-tax.md` line ~17, the parsers entry). Returning `ValueWord` makes `match parse_json(s)` impossible from Shape user code (no exhaustive case analysis); returning a typed enum makes it natural and forces the compiler to verify the user handled every variant.

**Considered (option β):** different per-parser typed enum (`JsonValue`, `YamlValue`, `TomlValue`, `MsgPackValue`, `XmlValue`) with each parser owning its own variant set.

**Rationalization (option β):** "TOML has a `DateTime` variant JSON doesn't have; MsgPack has a `Bytes` variant; YAML has tag annotations. Preserving each format's expressive surface lets users pattern-match on format-specific cases."

**Pattern recognized (option β):** five near-identical sum types with overlapping cases is structural duplication. Users serializing data through multiple formats would need conversion adapters between every pair. The right grain is *one* shared type with the union of variants — formats that don't have a given variant simply never construct it.

**Alternative taken (option γ):** define `crate::json_value::JsonValue` as a single concrete sum-type enum:
```rust
pub enum JsonValue {
    Null,
    Bool(bool),
    Int(i64),
    Number(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}
```
Each parser's stdlib body returns `Result<JsonValue, ParseError>`; the typed-stdlib registry materialises that into the user-facing `JsonValue` Shape type via a new `TypedReturn::JsonValue(JsonValue)` variant. Insertion order preserved in `Object` via `Vec<(String, JsonValue)>` (not `HashMap`). The name `JsonValue` (over `ParsedValue` / `DataTree`) follows the de-facto industry convention and matches the user's stated direction; format-specific extensions (TOML `DateTime`, YAML tag annotations) either project losslessly into existing variants (DateTime → `Object` with a sentinel field, or `String` ISO-8601) or trigger a follow-up if the lossy projection is unacceptable.

**Cost saved:** option α reproduces the dynamic-dispatch dispatch-on-tag pattern at the parser layer — exactly the W-series footgun in fresh skin. Option β fragments the parsed-value surface into 5 redundant types. Option γ matches `runtime-v2-spec.md` direction (concrete typed sum types over heterogeneous bytes). Estimate: ~1 week parser migration vs. ~3-4 weeks of follow-up cleanup if option α landed.

---

## 2026-05-06 — JsonValue user-facing surface: Shape enum over fluent accessor methods

**Considered:** expose `JsonValue` to Shape user code as a *fluent accessor object* — `parse_json(s).is_object()`, `parse_json(s).as_string()`, `parse_json(s).get("key")`, `parse_json(s).length()`. No exhaustive pattern matching from Shape; the type's "shape" is discovered at runtime via accessor probes.

**Rationalization:** "Pattern matching on a sum type with eight variants is ergonomic noise for the common case (`json["users"]["0"]["name"]`). Fluent accessors mirror what JS / Python / Ruby users expect from a JSON library. The exhaustive-match enum forces users to handle variants they don't care about."

**Pattern recognized:** the fluent surface preserves runtime dispatch *inside the accessor methods* — `as_string()` is a per-call tag-decode probe returning `Option<&str>`, identical to the `nb.as_str()` decoder pattern that the strict-typing plan deletes from the runtime. The compiler cannot verify exhaustiveness because there are no cases to verify; users discover their parser wasn't returning what they thought via runtime `None`. This is the runtime-tag-decode pattern at the Shape-language level — same shape as the `set_module` and `parallel_module` polymorphism that we deleted, just dressed up as method calls. Per CLAUDE.md "No `any` type" rule, dispatch on parsed-data shape is exactly the kind of "discover-at-runtime" pattern that Shape's static typing exists to remove.

**Alternative taken:** expose as a Shape-level typed sum-type enum (Phase 2c when wired up):
```shape
enum JsonValue {
    Null,
    Bool(bool),
    Int(int),
    Number(number),
    String(string),
    Bytes(Array<int>),
    Array(Array<JsonValue>),
    Object(HashMap<string, JsonValue>),
}
```
Users pattern-match exhaustively; the compiler verifies every case is handled. Convenience accessors (`obj.get("key")`, etc.) can be added as ordinary methods once the enum is in place — they compose on top, they don't replace exhaustive matching.

**Cost saved:** keeping fluent accessors as the only surface would have re-introduced runtime tag-decode at the language level — exactly what the strict-typing plan removes from the runtime. Estimate: 2-3 weeks of follow-up cleanup avoided when downstream user code starts pattern-matching parsed values exhaustively. Acknowledged immediate cost: Shape user code becomes more verbose for "I just want the string" cases until convenience methods land alongside the enum.

---

## 2026-05-06 — TypedReturn recursive variants: structural Concrete/Container split

**Considered:** keep `TypedReturn` as one flat enum; rely on registration-time validation to ensure that `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(...)))` and similar nested-defection patterns never appear in production code. Add a debug assertion or test sweep that walks the enum tree and panics on `ValueWord` nested inside `Ok`/`Err`/`Some`/etc.

**Rationalization:** "The marshal layer can detect the violation at runtime and reject. Tests can sweep registered functions for the bad shape and fail loudly. The Rust type system has limits and a runtime assertion is good enough — every other strict-typed compiler invariant is enforced this way."

**Pattern recognized:** "trust registration validation" is the runtime-discipline pattern, and runtime discipline is the same shape as runtime tag-decode dispatch. The W-series defection ("ValueBits shim retained as documented FFI-boundary bridge") was protected by the same kind of runtime-discipline argument — "we'll narrow it module-by-module, validation will catch backsliding." Five sessions later it was permanent. The strict-typing plan's mechanical-enforcement section (`CLAUDE.md` line 261) is explicit: "make the forbidden state unrepresentable, not just unreachable" — the `ProofGap` private-constructor pattern. Applying that same discipline here means making `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(...)))` a *type error*, not a runtime check.

**Alternative taken:** structurally split `TypedReturn` into a two-tier enum:
```rust
/// Strictly-typed leaf values. No recursion; no escape hatches.
pub enum ConcreteReturn {
    I64(i64), F64(f64), Bool(bool), Unit, String(String),
    Instant(std::time::Instant),
    ArrayI64(Vec<i64>), ArrayF64(Vec<f64>), ArrayString(Vec<String>),
    Bytes(Vec<u8>),
    HashMapStringString(Vec<(String, String)>),
    DataTable(std::sync::Arc<DataTable>),
    // (post Phase 2c) JsonValue(JsonValue) — typed-tree parsed data.
}

/// Container variants. Payload is *only* a ConcreteReturn — by construction.
pub enum TypedReturn {
    Concrete(ConcreteReturn),
    Ok(ConcreteReturn),
    Err(ConcreteReturn),
    Some(ConcreteReturn),
    None,
    ObjectPairs(Vec<(String, ConcreteReturn)>),
    TypedObject(Vec<(String, ConcreteReturn)>),
    ArrayObjectPairs(Vec<Vec<(String, ConcreteReturn)>>),
}
```
The Rust type system enforces that `Ok`/`Err`/`Some` cannot wrap another `Ok`/`Err`/`Some` (which is correct — `Result<Result<T,E>,F>` would be a registration bug regardless), and cannot wrap a `ValueWord` escape hatch (because no such variant exists in `ConcreteReturn`). Shape-language types like `Result<Result<T,E>,F>` aren't first-class today (`CLAUDE.md` Known Constraints: "Generic impls parse but are not first-class end-to-end"); if they become first-class later, the split grows a third tier rather than reverting.

**Cost saved:** prevented the optional-defection-becomes-default dynamic that put the prior plan in the W-series death spiral. Estimate: 1-2 weeks of "audit nested TypedReturn" follow-up cleanup avoided. Acknowledged immediate cost: every consumer that built `TypedReturn::Ok(Box::new(TypedReturn::String(...)))` becomes `TypedReturn::Ok(ConcreteReturn::String(...))` — slightly more verbose, but the verbosity *is* the proof.

---

## 2026-05-06 — Phase 2b unified marshal + wire/snapshot kind threading

The strict-typed runtime needs a single mechanism for projecting typed values across **every** ABI exit: the stdlib dispatch boundary (return side AND arg side), the wire-serialization boundary, and the snapshot/state-diff boundary. These are not three independent problems — they are three points where a typed slot crosses a non-typed boundary, and the strict-typed answer is the same at each point: **`(u64 bits, NativeKind kind)` paired**, threaded from compile-time slot-kind metadata, no runtime tag-decode.

This entry covers all four cuts and the alternatives rejected at each.

---

**Considered (option α, RETURN side):** restore `TypedReturn::into_value_word(self) -> ValueWord` (or its successor `into_some_intermediate_value`) — a synthesized 8-byte intermediate that the stack-push logic later decodes. Decode-on-push, encode-on-marshal.

**Rationalization (option α):** "The intermediate value is `u64`, not `ValueWord` — there's no tag dispatch, just a width-uniform transport. The stack-push logic already knows the slot's kind from the FunctionBlob. The intermediate is invisible to user code."

**Pattern recognized (option α):** identical to `ValueWord` semantically — an 8-byte word that carries a value whose interpretation is determined elsewhere. The fact that the discriminant moves from "tag bits in the same word" to "kind table in the FunctionBlob" doesn't change the dispatch shape. Worse: it adds a temporary that exists only at the marshal boundary, asking future readers to remember "this `u64` is post-marshal pre-push and the kind comes from a separate table." Identical defection shape to the W4 ConvertBoolToString opcode — synthesizing an intermediate to paper over a kind-tracker gap. The right fix is to project directly into the typed slot.

**Considered (option β, ARG side):** `TypedArgReader` trait with methods `read_i64(idx)`, `read_f64(idx)`, `read_str(idx)`, etc. Bodies pick the right reader per arg based on what they declared at registration. Registration validation enforces that the body's `read_*` calls match the declared param kinds.

**Rationalization (option β):** "The trait gives the body no way to *probe* — it must commit to a kind per call. That's structural enforcement at the call boundary."

**Pattern recognized (option β):** committal at the call site is not the same as committal at the type level. A body declared with `params: [Int]` that calls `read_f64(0)` is a registration-time bug, not a type-checker error — the trait permits it. "Registration validation catches it" is the runtime-discipline pattern; identical shape to the rejected "trust registration validation" approach for the recursive `TypedReturn` variants in the 2026-05-06 split entry. The same defection in a different file.

**Considered (option γ, ARG side):** macro-per-function that emits `fn read_arg0_i64(&self) -> i64`, `fn read_arg1_str(&self) -> &str`, etc., one per registered function, with kinds fixed at macro-expansion time.

**Rationalization (option γ):** "Macros emit per-function readers tied to the registration declaration, so kinds match by construction at the per-function call site."

**Pattern recognized (option γ):** structural enforcement, but with macro machinery doing what the type system can do directly. The trait-based generic approach below achieves the same property with no macros — and macros forfeit the readability of `fn parse_json(s: Arc<String>, ctx: &ModuleContext) -> Result<TypedReturn, MarshalError>`.

**Considered (option δ, RETURN+ARG):** one-tier discriminated union `enum SlotValue { Int(i64), Float(f64), Bool(bool), Heap(Arc<HeapValue>), Unit }` carried across the marshal boundary.

**Rationalization (option δ):** "It's a typed sum-type, not a tagged word. The variants are concrete; consumers `match` exhaustively."

**Pattern recognized (option δ):** `ValueWord` reborn. The dispatch moves from "tag bits in `u64`" to "enum discriminant in `SlotValue`," but the dispatch *exists* — every consumer pattern-matches on the discriminant. The entire deletion of HeapValue's `HashMap`/`Some`/`Ok`/`Err`/`Range`/etc. variants (commit `7d6dc27`, the option-C heap_value cut) was about removing exactly this kind of heterogeneous-by-default sum type from the runtime. Re-creating it at the marshal layer is the same defection in a higher layer.

**Considered (option ε):** Rust generics with phantom-typed `Slot<K: NativeKind>`, encoding the kind at compile time and eliminating the runtime discriminator entirely.

**Rationalization (option ε):** "Maximum strict-typing — the kind is in the type."

**Pattern recognized (option ε):** sound but out of scope. The VM stack is monomorphic 8-byte slots; phantom-typed slots would require a full executor-stack rewrite. The cost-benefit doesn't fit Phase 2b's budget. Filed as a hypothetical follow-up workstream `phantom-typed-stack` should the strict-typed approach show frequent reader-error patterns.

---

**Alternative taken (the unified Phase 2b shape):** every ABI exit becomes a `(u64 bits, NativeKind kind)` pair, threaded from compile-time `NativeKind` metadata on the calling side. Three concrete sub-mechanisms:

**Sub-mechanism A — stdlib dispatch (return side):**
```rust
pub fn marshal(ret: TypedReturn, expected: NativeKind, push: &mut SlotWriter)
    -> Result<(), MarshalError>;
```
`expected` comes from the function's registered `ConcreteType.to_native_kind()`. Mismatch is `MarshalError::Mismatch { expected, got }` — typed error, not panic. The marshaller projects directly to the typed slot via `push`; no synthesized intermediate.

**Sub-mechanism B — stdlib dispatch (arg side):**
```rust
pub trait FromSlot: Sized {
    const NATIVE_KIND: NativeKind;
    fn from_slot(bits: u64) -> Self;
}
impl FromSlot for i64    { const NATIVE_KIND: NativeKind = NativeKind::I64;  fn from_slot(bits: u64) -> Self { bits as i64 } }
impl FromSlot for f64    { const NATIVE_KIND: NativeKind = NativeKind::F64;  fn from_slot(bits: u64) -> Self { f64::from_bits(bits) } }
impl FromSlot for bool   { const NATIVE_KIND: NativeKind = NativeKind::Bool; fn from_slot(bits: u64) -> Self { bits != 0 } }
impl FromSlot for Arc<String>     { /* HeapValue::String pointer cast */ }
impl FromSlot for Arc<DataTable>  { /* HeapValue::DataTable pointer cast */ }
// …

pub trait TypedFn<Args, R>: Send + Sync + 'static {
    fn invoke(&self, slots: &[u64], ctx: &ModuleContext) -> Result<R, MarshalError>;
    fn arg_kinds() -> Vec<NativeKind>;
}
// blanket impl for Fn(P0) -> R, Fn(P0, P1) -> R, ..., Fn(P0..P7) -> R
// where each Pi: FromSlot, R: ToSlot.

pub fn register_typed_fn<F, Args, R>(
    module: &mut ModuleExports,
    name: &str,
    description: &str,
    param_names: &[&str],
    body: F,
) where F: TypedFn<Args, R>, R: ToSlot;
```
Param kinds derive from `Pi::NATIVE_KIND` at compile time. A body declared `fn parse_int(s: Arc<String>, base: i64) -> Result<i64, ParseError>` registers with arg kinds `[Ptr(HeapKind::String), I64]` automatically — the function's Rust argument types **are** the typed signature. A body declared `fn parse_int(s: Arc<String>, base: f64)` registered against `params: [string, int]` is a Rust trait-bound error at the `register_typed_fn` call site. No registration validator runs; the type system already did.

**Sub-mechanism C — wire/snapshot kind threading:**
```rust
pub fn slot_to_wire(bits: u64, kind: NativeKind, ctx: &Context) -> WireValue;
pub fn slot_to_serializable(bits: u64, kind: NativeKind, store: &SnapshotStore)
    -> SerializableVMValue;
pub fn slot_to_state_diff(bits: u64, kind: NativeKind, …) -> …;
```
Callers thread `kind` from the FunctionBlob's per-slot kind table (which already exists at compile time for typed-opcode emission). For heap kinds, `bits` is `Arc<HeapValue>` raw pointer; the per-`HeapValue` arms take over the dispatch.

---

**Why these three are one cut, not three:** the discriminator (`NativeKind`) is the same; the source of the discriminator (FunctionBlob's compile-time slot-kind metadata) is the same; the projection target differs only in the destination (typed VM slot vs. `WireValue` vs. `SerializableVMValue`). A single landing of `NativeKind` as the universal ABI-exit discriminator is the right granularity. Three separate landings would risk the discriminators drifting (one calling it `NativeKind`, another `SlotKind`, another `MarshalKind`) — the "two parallel discriminators" trap.

**Cost saved:** the trait-based arg side eliminates the entire `read_*` plumbing surface (~12 methods) of option β; eliminates the registration-validation runtime check; eliminates the macro infrastructure of option γ; and unifies the three boundaries into one mechanism (vs three near-identical implementations). Estimate: 5–8 days for full Phase 2b vs. ~3 weeks if each boundary is rebuilt independently with its own discriminator. Acknowledged immediate cost: every stdlib registration site rewrites from `|args, ctx| { let s = args[0].as_str()…; … }` to `|s: Arc<String>, ctx: &ModuleContext| -> Result<…> { … }` — verbose-once, structurally enforced thereafter.

**Calibration:** if the canary stdlib migration (chosen module: `random.rs`) does NOT drop the lib error count materially after marshal infra + one module's consumer migration, the diagnosis "most errors are downstream of the marshal layer" is wrong and we stop to surface before mass migration.

---

(Add new entries above this line. Newest first.)
