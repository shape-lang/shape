# Cluster 5 audit — JsonValue marshal (N7 universal-intermediate residual)

**Scope**: `crates/shape-runtime/src/json_value.rs` (465 LoC),
`crates/shape-runtime/src/typed_module_exports.rs` (`ConcreteReturn::JsonValue`,
`ConcreteType::JsonValue`), and 6-7 stdlib bodies that walk a HeapValue
tree to produce a target-format string/bytes:
`crates/shape-runtime/src/stdlib/{json,yaml,toml_module,msgpack_module,http,xml}.rs`.

**Predicted error drop**: -7 to -14 in shape-runtime --lib (per N7
commit-sequence calibration C2-C13 in `docs/defections.md:856`). Plus
-10 to -20 from the 3.C reverse-direction cascade (yaml.parse,
toml.parse, msgpack.decode, msgpack.decode_bytes) when it lands.
**Combined N7 + 3.C: -17 to -34 errors.** Anchored to the dev2-n7
worktree fresh `cargo check -p shape-runtime --lib` baseline of 67 at
HEAD `5f637e1` per defections.md:945.

**Audit performed by**: scout-2026-05-07

## Supervisor caveat (2026-05-08) — O1 ruling is near-term, not final

Supervisor accepted **O1** (JsonValue → HeapValue projection walker
in `json_value.rs`) as the cluster #5 architectural sub-decision, **with
explicit reservation**. O1 is correct as a near-term unblock for C7-C13
mechanical drain, but is **not the final architectural shape**.

The walker is a **transitional ABI artifact**. The deeper fix lives in
**cluster #7** (named in ADR-005 §Implementation roadmap): folding
`ConcreteReturn`'s heap-arm variants — including `JsonValue` —
into a single `Heap(Arc<HeapValue>)` arm. Once cluster #7 lands:

- Consumers that today produce `ConcreteReturn::JsonValue(jv)` will
  instead produce `Heap(Arc::new(HeapValue::HashMap(...)))` (or the
  correct `HeapValue` arm) **directly** — no JsonValue intermediate at
  the ABI boundary.
- The `json_value_to_heap_value` walker either (a) survives as an
  internal parser-only helper (legitimate; not crossing an ABI
  boundary), or (b) gets deprecated entirely if parsers move to
  producing HeapValue directly.

### Marker comment for the migrator (use verbatim)

When implementing `json_value_to_heap_value` in `json_value.rs`, place
this comment block immediately above the function definition:

```rust
// ADR-005: transitional. The JsonValue ConcreteReturn variant is
// scheduled for cluster #7 folding (ADR-005 §Implementation roadmap);
// once that lands, consumers produce HeapValue directly and this
// walker survives only as a parser-internal helper (if at all). Not
// a permanent ABI shape. See docs/defections.md (2026-05-08, cluster
// #5 O1 with caveat) for the supervisor reservation.
```

This caveat is the on-record reason future sessions should NOT treat
the walker as the canonical shape, even if cluster #5 ships
successfully and the walker becomes load-bearing for parser output in
the meantime. Cluster #7 closes the loop.

## Audit 1 — consumer call shape

The N7 architectural-shape disposition (ε JsonValue universal
intermediate) is **already signed off** at supervisor PB 1/4. The C2
walker (`heap_to_json_value`) is **landed** in
`crates/shape-runtime/src/json_value.rs:82-138` with all 18 HeapValue
arms explicitly handled (Mechanical-yes 5 + Categorically-non-data
Reject 5 + Architectural-choice deferred 7 + TypedObject schema-aware
1; plus 13/15 TypedArrayData inner sub-variants).

Reverse helpers C3/C4/C5/C6 are **landed**: `json_value_to_serde_json`
(json_value.rs:311), `json_value_to_serde_yaml` (json_value.rs:350),
`json_value_to_toml_value` (json_value.rs:405),
`json_value_to_msgpack_bytes` (json_value.rs:462).

What remains is **per-consumer body migration C7-C13** (one commit per
consumer file). For each, the consumer body's call shape is:

### C7 — `json.stringify` (json.rs:377-418)

Currently calls deleted `value.to_json_value()`. Migration: call
`heap_to_json_value(&hv)?` then `json_value_to_serde_json(&jv)` then
`serde_json::to_string(&v)?` / `to_string_pretty(&v)?`. Body input is
`Arc<HeapValue>` via FromSlot; output is `String`.

### C8/C9 — `http.post_json` / `http.put_json` (http.rs:496-523)

Same migration shape — currently in deferral block (deleted body).
Mirrors `post_text` / `put_text` from commit `d0a73e7`. Body input
is `(Arc<String> url, Arc<HeapValue> data, ...)`; output is
`OkObjectPairs(response_pairs)` (already cluster #4 β shape).

### C10 — `yaml.stringify` (yaml.rs:146-170)

**Currently broken**: returns `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(...)))`
on line 110 / 140 (legacy escape-hatch variants `TypedReturn::ValueWord`
and `TypedReturn::ArrayValueWord`). The yaml.rs file's body for
`yaml.stringify` at line 168 itself uses
`TypedReturn::Ok(Box::new(TypedReturn::String(output)))` which is also
the deleted recursive shape. Migration: replace with
`heap_to_json_value(&hv)? → json_value_to_serde_yaml(&jv) →
serde_yaml::to_string(&v)?` and return
`TypedReturn::Ok(ConcreteReturn::String(s))`.

### C11 — `toml.stringify` (toml_module.rs:141-165)

Currently calls deleted `value.to_json_value()` and the `nanboxed_to_toml_value`
walker at lines 67-107 (also broken — walker uses deleted ValueWord
accessors). Migration: delete `nanboxed_to_toml_value` walker entirely;
use `heap_to_json_value(&hv)? → json_value_to_toml_value(&jv) →
toml::to_string(&v)?`. **TOML root-level Table check** stays in C11's
body per `json_value.rs:396` ("returns a `toml::Value` of any shape;
the consumer is responsible for verifying root-level Table").

### C12/C13 — `msgpack.encode` / `msgpack.encode_bytes` (msgpack_module.rs:86-111, 148-173)

Same broken pattern — body has `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(...)))`
plus deleted `value.to_json_value()`. Migration: use
`heap_to_json_value(&hv)? → json_value_to_msgpack_bytes(&jv)?` for C13;
hex-string-encode the bytes for C12.

### Adjacent xml.rs

Per N7 consumer table, xml is not in the 7-consumer list; xml.parse
returns `OkObjectPairs` (cluster #4 β shape) and is already migrated
or deferred via different path. **Confirm during cluster execution
that no xml.* function uses `to_json_value()`.**

### 3.C reverse-direction consumers (post-N7-C6)

Sequenced after C6 lands (the rmpv-via-serde_json bridge). Consumers:

- `yaml.parse` (yaml.rs) — currently uses deleted `to_value_word`-style
  reconstruction.
- `yaml.parse_all` — same.
- `toml.parse` — currently calls deleted `nanboxed_to_toml_value` reverse
  direction or similar.
- `msgpack.decode` (msgpack_module.rs:142, 214) — uses
  `TypedReturn::ValueWord` legacy escape-hatch.
- `msgpack.decode_bytes` — same.

3.C migration: each consumer parses input via `serde_*` to produce a
`serde_*::Value`, then converts to `JsonValue` via the **reverse helper
(C2-mirror in 3.C-C2/C3/C4)**, then projects to
`TypedReturn::Ok(ConcreteReturn::JsonValue(jv))`. The C2-mirror
reverse helpers (`serde_yaml_to_json_value`, `toml_value_to_json_value`,
`rmpv_to_json_value`) are 3.C scope, not N7 scope.

## Audit 2 — marshal-API readiness

### What's already in place

- **`heap_to_json_value(&HeapValue) -> Result<JsonValue, String>`** —
  C2 walker, json_value.rs:82-138. All 18 HeapValue arms handled per
  REFINEMENT-1A + REFINEMENT-1B-ITEM-A; the 7 architectural-choice
  deferred + 5 categorically-non-data + 5 mechanical-yes + 1
  TypedObject schema-aware partition is final.
- **`json_value_to_*` reverse helpers** — json/yaml/toml/msgpack-bytes
  already landed (C3/C4/C5/C6).
- **`ConcreteReturn::JsonValue(JsonValue)`** (typed_module_exports.rs:126)
  — leaf marshal variant already wired; carries the recursive JsonValue
  payload.
- **`ConcreteType::JsonValue(String)`** (typed_module_exports.rs:280) —
  return-type descriptor with caller-provided type-name (`"Json"` for
  `json.parse`, `"any"` for `json.__parse_typed`-style polymorphic).
- **`json.parse`** is the in-tree migrated example using
  `TypedReturn::Ok(ConcreteReturn::JsonValue(...))` — see json.rs:310;
  per-consumer migration template is established and proven.
- **JsonValue.Bytes variant** (json_value.rs:26) — defined; reserved for
  msgpack-binary parse path. **Not currently produced by C2 walker**;
  emerges in 3.C msgpack-decode path.
- **`stdlib-src/core/json_value.shape`** — the user-facing `Json` enum
  with 7 variant schemas already declared and stdlib-pre-registered.
  B1 sub-decision #3 effectively settled (per defections.md:4804).

### What's missing

- **Per-consumer C7-C13 body migrations** — 7 commits, mostly
  mechanical now that C2 + reverse helpers are landed.
- **3.C reverse helpers** (3.C-C2/C3/C4): `serde_yaml_to_json_value`,
  `toml_value_to_json_value`, `rmpv_to_json_value`. Not yet landed.
- **3.C per-consumer migrations** — yaml.parse / yaml.parse_all /
  toml.parse / msgpack.decode / msgpack.decode_bytes (5+ commits).
- **HashMap-marshal interlock for JsonValue::Object representation**
  (B1 sub-decision #2) — the runtime-side `HashMap` representation
  is settled at the HashMap-marshal cluster (P1(b) Vec-of-pairs +
  HashMap heap variant); JsonValue::Object's projection at the **dispatcher
  side** still needs to navigate from `JsonValue::Object(Vec<(String, JsonValue)>)`
  back to `HeapValue::HashMap(...)` or to a TypedObject. **This is
  cluster #5's primary residual architectural sub-decision.**
- **Dispatcher projection of `ConcreteReturn::JsonValue(jv)` → typed
  slot bits** — when shape-vm cascade lands, the projection step needs
  to walk the JsonValue tree to produce a `HeapValue` tree and box it
  in an `Arc<HeapValue>`. This walker is the inverse of `heap_to_json_value`.
- **`TypedReturn::ValueWord` / `TypedReturn::ArrayValueWord` legacy
  variants** still referenced by yaml/msgpack/toml stringify bodies
  (per the citations above) need deletion. Per defections.md they are
  the "long-deleted" escape hatch but the bodies still mention them
  — they're definition-deleted but consumer-body-still-citing. **Each
  C7-C13 commit deletes the legacy citation as part of body
  rewriting.**
- **Architectural-choice deferred policies** for the 7 deferred HeapValue
  arms (Decimal, DataTable, Content, Temporal, TableView, Instant,
  NativeScalar) plus 2 TypedArrayData inner sub-variants (Matrix,
  FloatSlice) — per consumer demand, not pre-decided.

## Architectural-shape options

The N7 architectural shape is **already signed off (ε)**. Three sub-
decisions remain.

### Option O1 — JsonValue → HeapValue projection walker (cluster #5 owns)

**Shape**: Add `pub fn json_value_to_heap_value(jv: &JsonValue) -> Arc<HeapValue>`
in json_value.rs. Mirrors C2 in reverse direction. JsonValue::Object
projects to `HeapValue::HashMap` per the HashMap-marshal P1(b)
storage shape (Vec<(Arc<String>, Arc<HeapValue>)>). JsonValue::Array
projects to `HeapValue::TypedArray(TypedArrayData::HeapValue)` per
Phase 2d Array landing.

**Pros**:
- Symmetry with C2 walker; both directions live in `json_value.rs`.
- Reuses already-landed HashMap-marshal + Phase 2d Array storage shapes.
- Single architectural choice point; per-consumer body migration is
  mechanical.
- Decouples from shape-vm dispatcher concerns — consumer bodies that
  produce `ConcreteReturn::JsonValue(jv)` don't need to know how it'll
  be projected to slot bits; the walker handles it.

**Cons / risks**:
- Forces JsonValue::Object → HeapValue::HashMap conversion at every
  return — for consumers that already had a HashMap-shaped result,
  this is a parse → JsonValue → HashMap round-trip. Performance
  consideration only on the Json::Object hot path.

**Effort**: 1-2 days.

### Option O2 — Dispatcher-side recursive projection in shape-vm

**Shape**: shape-vm dispatcher's `TypedReturn::Concrete(ConcreteReturn::JsonValue(jv))`
arm walks the JsonValue tree and emits per-arm `HeapValue` directly,
plumbed through the ValueSlot projection step. JsonValue stays
shape-runtime-side; the projection lives in shape-vm.

**Pros**:
- shape-runtime side remains pure value-tree without HeapValue
  construction concerns.
- shape-vm cascade owns the projection, matching its role for other
  ConcreteReturn variants.

**Cons / risks**:
- Cross-crate coupling: shape-vm needs to depend on the JsonValue
  variant set; any future JsonValue extension touches shape-vm.
- Doesn't decouple cluster #5 from the shape-vm cascade — N7-C7-C13
  consumer migrations must wait for the shape-vm dispatcher to know
  how to project JsonValue. **This blocks cluster #5 progress until
  shape-vm cascade.**

**Effort**: 3-4 days (plus cross-crate coordination).

### Option O3 — Defer projection; consumers produce HeapValue directly

**Shape**: keep the C2 walker for serialization-direction (ConcreteReturn
producers) but for the *parse* direction (3.C), have consumer bodies
construct HeapValue directly (skipping the JsonValue intermediate),
and use `ConcreteReturn::OpaqueTypedObject(Arc::new(hv))` for the
dispatcher projection.

**Pros**:
- Avoids the JsonValue → HeapValue walker entirely.
- Reuses the N8 OpaqueTypedObject path that's already proven for
  `json.__parse_typed`.

**Cons / risks**:
- **DIRECTLY MATCHES the N7 disposition's REFUSED candidate**:
  "❌ HeapValue→bytes-direct walker bypassing JsonValue intermediate
  (defeats ε structural-enforcement)" (defections.md:918). The reverse
  direction (parse-side) inherits the same structural-enforcement
  argument.
- **REJECT.**

**Effort**: N/A (forbidden).

### Option O4 — JsonValue extension for missing variants

**Shape**: when 3.C parsers (yaml/toml/msgpack) hit format-specific
data that doesn't map cleanly to existing JsonValue variants (YAML
Tagged, TOML Datetime, msgpack Extension), add new JsonValue variants
(`YamlTagged`, `TomlDatetime`, `MsgpackExtension`).

**Pros**:
- Round-trip fidelity per format.

**Cons / risks**:
- **DIRECTLY MATCHES the N7 disposition's REFUSED candidate**:
  "❌ JsonValue extension (e.g. `YamlTagged`, `TomlDatetime` variants)
  in first-landing" (defections.md:922). The 3.C cascade explicitly
  uses lossy mappings (yaml Tagged → unwrap, toml Datetime → String).
- **REJECT.**

**Effort**: N/A (forbidden).

## Recommendation

**Rank**: O1 > O2; O3 / O4 rejected.

If I were the supervisor I'd take O1 — add `json_value_to_heap_value`
as a sibling to the existing C2/C3/C4/C5/C6 helpers in `json_value.rs`.
This keeps the cluster #5 surface contained in shape-runtime, matches
the symmetric-helper-pattern that's already established, and decouples
the per-consumer migrations C7-C13 from the shape-vm cascade timing.

**Critical sequencing note**: per defections.md:856-870, the cluster
sequence is `C2 → C3-C6 (helpers) → C7-C13 (consumers)`, and C2 is
**already landed**. The handover-listed cluster #5 work is therefore
the C7-C13 mechanical sequence + 3.C cascade + the dispatcher-projection
sub-decision (O1/O2). Most of the architectural surface is already
decided; the remaining decisions are O1-vs-O2 + per-deferred-arm policy
sign-offs as consumers demand them.

## Open questions for supervisor

1. **(O1/O2)** Should the JsonValue → HeapValue projection walker live
   in shape-runtime (`json_value.rs` sibling helper, O1) or in shape-vm
   dispatcher (O2)? O1 keeps cluster #5 self-contained; O2 follows the
   "dispatcher owns projection" pattern of other ConcreteReturn variants.
2. **(yes/no)** Is cluster #5 strictly post-N7-C2-landing, or does it
   include a re-audit of C2's 18-arm partition (the 7 architectural-choice
   deferred arms)? Per defections.md, each deferred arm gets its own
   sub-decision when first consumer demands it — these are not
   cluster #5 sub-decisions.
3. **(A/B/C)** Order of C7-C13 commits: alphabetical (json.stringify
   first, ...), by complexity (yaml/toml first since they have the
   broken-body residual; msgpack last since it has dual sync/bytes
   variants), or by error-drop (which file has the most independent
   E0425/E0433 sites)?
4. **(yes/no)** Does cluster #5 include 3.C cascade (yaml.parse /
   toml.parse / msgpack.decode), or is 3.C a separate cluster sequenced
   post-N7-C6? Per defections.md:872 they're separate but combined
   prediction is given (-17 to -34).
5. **(yes/no)** Should the cluster's predicted error-drop range be
   re-anchored to the current shape-runtime --lib baseline (was 96 at
   Phase 2d handover; should be lower after sub-cluster 1 / cluster #4
   / Phase 2d Array landings — the 67-baseline cited in defections.md
   was from the dev2-n7 worktree at HEAD `5f637e1`)?
