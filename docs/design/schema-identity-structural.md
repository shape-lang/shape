# Structural (content-based) schema identity — WF-3A

Status: DESIGN / Phase 1 (Diagnosis) complete. Phases 2 (Blast-radius) and 3
(Design) to follow. This is a design pass — no compiler/runtime code changes.

Worktree: `shape-wf3a-schema` (branch `wave3/schema-identity-design`).

---

## Diagnosis

### 0. TL;DR

`SchemaId` (`type SchemaId = u32`, `mod.rs:102`) is a **monotonic counter**, and
there are **two independent counters** that both feed the **same** `by_id` map:

- **Counter A** — the compile-time `TypeTracker`-owned registry's
  `next_id` (`registry.rs:41`, `allocate_id` at `registry.rs:111`). Inline
  object literal schemas are allocated here via `register_type_scoped`.
- **Counter B** — the process-ambient `current_registry()` counter
  (task-local → thread-local → `DEFAULT_SCHEMA_REGISTRY`, `current.rs:69`).
  Object-**merge** schemas are allocated here via `register_type` →
  `TypeSchema::new` → `allocate_current_id()` (`schema.rs:20`).

Both schema kinds are then `register()`-ed into the **same** TypeTracker
registry's `by_id` map. `register()` (`registry.rs:145`) does a bare
`by_id.insert(id, name)` — no collision detection, silent overwrite. Because A
and B are independent and both seed at `1`, a B-issued merged id routinely
equals an A-issued inline id already in `by_id`. The later insert wins the
slot; `lookup_schema(id)` then returns the **wrong** schema, and every consumer
keyed on `SchemaId` (runtime field layout, wire/JSON projection, snapshot
resume) resolves a mismatched layout.

This is the "counter allocation across registry boundaries" root. Any change in
*which* schemas register, or *in what order*, shifts every subsequent id and
moves the collision — which is why an **unrelated** `Snapshot` enum
registration flipped the object-spread test from green to red at the Wave-2
merge (923a8857).

### 1. The counter mechanism

`SchemaId = u32` is issued by `fetch_add(1)` on an `AtomicU32`. Three seed/patch
facts:

- Every fresh registry seeds at `INITIAL_SCHEMA_ID = 1` (`registry.rs:19`,
  `default_next_id` at `registry.rs:61`).
- `TypeSchema::new` (`schema.rs:85`) draws from the **ambient** registry
  (`allocate_current_id` → `current_registry().allocate_id()`).
- `register_type_scoped` / `register_enum_scoped` / `TypeSchemaBuilder::register`
  draw from the **target** registry's own counter (`self.allocate_id()`).

So the id a schema receives depends entirely on *how many allocations already
happened on that specific counter*. There is no structural relationship between
a schema's identity and its content — identity is purely temporal/ordinal.

### 2. Reproduced object-spread trace

Repro (`/scratchpad/spread.shape`, VM interpreter path):

```
let base = { x: 1, y: 2 }
let extended = { ...base, z: 3 }
print(extended.z)
```

Observed at worktree HEAD:

```
Error: Runtime error: MakeFieldRef field_idx 2 out of bounds (slot count 1) (line 3)
```

(This is the exact `#[ignore]`d repro at
`crates/shape-jit/src/mir_compiler/integration_tests.rs:~2096`
`aggregate_object_spread_simple_baseline` and
`tools/shape-test/tests/jit/tiering.rs:~276`
`vm_aggregate_object_spread_simple`.)

**Compile-time (bytecode) trace** — `compile_dynamic_object`
(`crates/shape-vm/src/compiler/expressions/collections.rs:937`):

1. `base = { x, y }` — no spread → the non-spread `compile_typed_object` path
   (`collections.rs:761`) → `register_inline_object_schema_typed([x,y])` →
   `register_type_scoped` on **Counter A** → inline schema `A2` (2 fields).
2. `extended = { ...base, z: 3 }` — has spread → `compile_dynamic_object`:
   - Spread entry: `has_initial_object=false`, so an empty inline is registered
     first — `register_inline_object_schema_typed([])` → **Counter A** → `A0`
     (0 fields), `NewTypedObject field_count 0`.
   - Compiles `base` → `last_expr_schema = A2`.
   - `register_object_merge_schema(A0, A2)` (`collections.rs:1096`) →
     `schema_registry_mut().register_type("__merged_A0_A2", [x,y])` →
     **Counter B** (ambient) → merged schema `M1` (2 fields). `MergeObject`.
   - Field entry `z` → finalize → `register_inline_object_schema_typed([z])` →
     **Counter A** → `A1` (1 field), `NewTypedObject field_count 1`.
   - `register_object_merge_schema(M1, A1)` → `register_type("__merged_M1_A1",
     [x,y,z])` → **Counter B** → merged schema `M2` (3 fields). `MergeObject`.
   - `extended`'s static schema is `M2` (3 fields → `z` at field_idx 2).

**The clash.** `A0`, `A1`, `A2` come from Counter A; `M1`, `M2` come from
Counter B. Both sets land in the *same* TypeTracker `by_id` map. Because the two
counters are independent, a B-issued value (`M1`/`M2`) coincides numerically
with an A-issued value already present (e.g. `M2 == A1`, the 1-field `[z]`
schema). `register()`'s `by_id.insert` silently overwrites, so `by_id[M2]`
resolves to a **1-field** schema.

**Runtime.** `op_merge_object` (`objects/object_operations.rs:137`) reads
`target.schema_id` / `source.schema_id` stamped in the storages, and calls
`self.lookup_schema(id)` (`executor/vm_impl/schemas.rs:36`, a `by_id` lookup) to
compute `keep_left_indices` and `right_count`. When `lookup_schema` returns the
colliding wrong-arity schema, the built `TypedObjectStorage`
(`build_named_merged_storage`, `object_operations.rs:283`) gets the wrong slot
count. `extended` materializes as a **1-slot** object. Then `extended.z` lowers
to `MakeFieldRef field_idx 2`, whose bounds check
(`executor/variables/mod.rs:2759`) is against the **runtime**
`receiver.slots().len()` (== 1) → "field_idx 2 out of bounds (slot count 1)".

Note `NewTypedObject`'s operand truncates `schema_id as u16`
(`collections.rs:966`), a *second* latent collision surface once ids exceed
65535 — not the active cause here but part of the same fragility.

### 3. The regression signature (why an unrelated enum broke it)

The `#[ignore]` reason records: the Wave-2 snapshot-resume branch registers the
`Snapshot` enum, "advancing `schema_registry.next_id`", after which the
object-spread inline schema "collides with a 1-slot schema". Concretely:
registering `Snapshot` advances **Counter B** (the ambient/default registry) by
a fixed amount, which shifts the numeric value the next B-allocated merged
schema receives — moving the collision onto a 1-field A-schema. The test
regressed at 923a8857 with **no change to object-spread code**. Registration-
order sensitivity that turns an unrelated edit into a remote breakage is the
defining symptom of counter identity.

### 4. The remote::execute / JsonValue-projection connection

The same collision manifests on the **projection/wire** side. A
`TypedObject` carries only a numeric `schema_id`; the JSON/wire projection
`typed_object_to_json_value` (`crates/shape-runtime/src/json_value.rs`,
consumed by `stdlib/http.rs` response rendering and the distributed
`remote::execute` result path that renders a returned value as `WireValue`)
resolves `schema_id → TypeSchema` via `get_by_id` / `lookup_schema_by_id_public`
to walk the object's fields.

`resolve_typed_object_schema` (`json_value.rs:435`, dated WF-2E 2026-07-05) is
an explicit point-patch documenting the collision: *"the schema registries can
map one id to MORE THAN ONE schema — observed: a predeclared `XmlNode`-shaped
schema and the builtin `Json` enum both answer to id 41, and `get_by_id` /
`lookup_schema_by_id_public` return the `Json` schema (2 fields) for an XmlNode
node (3 fields)"*. The patch works around it by gathering all candidate schemas
under the id from three registries (execution by-id, ambient by-id, ambient
predeclared) and preferring the one whose `fields.len() == slot_count`. That is
the same **runtime-arity heuristic** as `ensure_next_id_above`: a symptom
suppressor, not identity. This is the "remote::execute JsonValue polymorphic
projection residual (WireValue rendered via the wrong schema)" — it shares the
exact root with the object-spread bug.

### 5. Prior point-patches (the recurring band-aids)

All of the following exist *because* identity is a counter; none fix the root:

1. **`ensure_next_id_above`** (`registry.rs:121`) + module-level
   `ensure_next_schema_id_above` (`mod.rs:111`) — bump a registry's counter past
   the max already-loaded id so a *new* allocation cannot collide with a
   *loaded* one. Call sites: `executor/vm_impl/program.rs:79` (load_program),
   `compiler/statements.rs:2211` (extension-module load),
   `compiler/compiler_impl_initialization.rs:914` (`seed_persistent_schemas`,
   REPL/cell resume), and twice inside `merge()` (`registry.rs:521`, `:567`).
   Suppresses new-vs-loaded collisions only; does nothing for the A-vs-B
   two-counter overlap that breaks object spread.

2. **`merge()` id-collision reallocation loop** (`registry.rs:536-545`) — when an
   incoming schema's id already maps to a different name in `self.by_id`,
   allocate a fresh id and return an `id_remap: HashMap<SchemaId, SchemaId>`
   that *callers must apply to bytecode operands*. A whole remap protocol whose
   sole reason to exist is counter overlap across merged registries.

3. **`reserved` flag** (`schema.rs:61`) + reserved-skip in field-set/field-order
   inference (`lookup_predeclared_schema_id`, `mod.rs:146`; `TypeSchema`
   doc-comment §4.1.4/§4.3). WF-1B comptime-descriptor recurrence: stops an
   ordinary `{name, kind, …}` object from binding to a contract schema
   (`__ComptimeTarget`, `TypeInfo`, …) by structural inference. A by-name guard
   against a structural collision.

4. **WF-2E `resolve_typed_object_schema`** (`json_value.rs:435`) — the json/xml
   node recurrence (§4 above): runtime slot-count disambiguation among colliding
   candidates.

Plus two ad-hoc *name-based* dedup caches that are already partial, per-registry
structural identity: `__inline_obj_N` dedup by ordered `(field_name, field_type)`
match (`type_tracking.rs:1135`) and `__merged_L_R` dedup by name
(`collections.rs:1101`). They dedup within one registry but do not survive a
registry boundary and do not give the *same* content the *same* id across
registries — which is exactly what structural identity must do.

The `#[ignore]`d object-spread test is the **4th recurrence** of this family
(WF-1B comptime descriptors / WF-2E json-xml nodes / jitfb object-merge /
object-spread), and is the reason WF-3A exists.

### 6. Why this is a family, not four bugs

Every instance is the same shape: content-independent identity + a shared id
keyspace + a consumer that trusts `by_id`. The counter makes identity depend on
global registration history; the shared `by_id` insert silently overwrites on
overlap; the consumer (runtime layout, JSON/wire projection, snapshot resume)
reads back the wrong structure. Each patch adds a *disambiguator* downstream
(arity heuristic, reserved flag, remap table, counter bump) instead of making
identity a function of content. Structural identity removes the whole class:
identical structure → identical id (dedup, no duplicate registration), distinct
structure → distinct id (no overwrite), independent of registration order,
process, or node.

---

## Blast-radius (Phase 2)

Total `SchemaId`/`schema_id` reference sites across `crates/` + `tools/` +
`bin/`: **~159** typed-`SchemaId` mentions, and hundreds more `schema_id: u64`
runtime carriers. Grouped below by role. "Order-sensitive" = the site's
correctness depends on *which numeric id* a schema received, i.e. on registration
history. Counts are grep-derived and include some test/comment noise; the
load-bearing subset is called out per group.

### (a) Compile-time id ASSIGNMENT — the SOURCE of order-sensitivity

| Site | Count | Counter | Notes |
|------|-------|---------|-------|
| `allocate_current_id()` → `current_registry().allocate_id()` (`schema.rs:20`), used by `TypeSchema::new` (`schema.rs:85`) | 3 defn/call | **B** (ambient) | object-**merge** schemas, builtin seeds |
| `self.allocate_id()` = `next_id.fetch_add(1)` (`registry.rs:111`) | ~33 call sites | **A** (per-registry) | inline object / enum / scoped registration |
| `register_type_scoped` / `register_enum_scoped` | ~30 | **A** | inline object literals, comptime types |
| ambient `register_type(` | ~15 | **B** | merges, xml/csv/json node schemas |
| `TypeSchema::new(` | ~50 (incl. tests) | **B** | any freshly-built schema |
| `with_id(` (explicit id, bypasses counter) | ~34 | fixed | builtin/predeclared schemas pinned to a constant id; **collide directly with any counter that reaches that value** (the id-41 XmlNode/Json case) |
| `TypeSchemaBuilder` `.register()` | ~70 mentions | A or B | builder finalize |

**All of (a) is order-sensitive by construction** — this is the root. The two
independent counters (A per-registry, B ambient) seeding at `1` into a shared
`by_id` keyspace, plus `with_id` constants that both counters eventually reach,
are the entire defect surface.

### (b) Runtime LOOKUP (`by_id` / `by_name` / field-offset)

| Site | Count | Order-sensitive? |
|------|-------|------------------|
| `by_id` map access (`registry.rs:148`, `lookup_schema`, `get_by_id`, `lookup_schema_by_id_public`) | ~198 grep / ~117 real read sites | **YES** — resolves id→structure; the exact wrong-schema surface |
| `by_name` map access | ~251 grep | Mostly **NO** (name is stable) — BUT `__merged_L_R` / `__inline_obj_N` synthetic names are themselves partial structural keys, and name-collision on merges is a parallel bug |
| field-offset / field-index resolution (`field_idx`, `slot_index`, `MakeFieldRef` bounds) | ~351 grep | **YES, transitively** — offset comes from the schema the id resolved to (`variables/mod.rs:2759` is the object-spread crash site) |

The load-bearing read is `lookup_schema(id)` in `op_merge_object`
(`objects/object_operations.rs:137`) and every `.get_by_id` in the JSON/wire
projection path.

### (c) SNAPSHOT serialize / restore

- `SV::TypedObject { schema_id, .. }` persists the raw id (`snapshot.rs`, id
  stored as the numeric value; test asserts `schema_id: 42` round-trips at
  `snapshot.rs:2062/2081`). ~59 `schema_id` sites in `snapshot.rs`.
- Resume does **not** re-derive identity — `decode_vmstate_typed_object`
  (`resume.rs:430-442`) reads the persisted `schema_id` and calls
  `schemas.get_by_id(schema_id)` against the **resuming program's** registry,
  erroring if absent. Result/Option/Snapshot/SnapshotError enums are matched by
  comparing the persisted id to the resuming program's `schemas.result` /
  `.option` ids (`snapshot.rs:1582-1655`).
- **Order-sensitive + cross-process fragile: YES.** The persisted id is only
  correct if the resuming process registered the identical schema at the
  identical counter position. Any divergence in registration order between
  snapshot-time and resume-time silently maps the id to a different structure.
  This is the same requirement counters cannot meet, now spanning a process
  boundary.

### (d) WIRE serialize (`shape-wire`, 2 files)

- `crates/shape-wire/src/value.rs:115` — `WireValue { schema_id: Option<u32> }`;
  default `None` (`:231`).
- `crates/shape-wire/src/codec.rs:149` — currently a `Some(1)` placeholder.
- **Small surface today** (the field exists but is barely populated), but it is
  the exact channel the `remote::execute` projection uses to carry structure
  cross-node. **Order-sensitive if ever populated with a counter id** — a wire
  `schema_id` minted on node X is meaningless on node Y under counter identity.
  Structural identity is a precondition for this field to be usable at all.

### (e) CONTENT-ADDRESSED hash — **CRITICAL, already order-unstable**

`FunctionBlobHashInput` (`content_addressed.rs:122-160`) feeds `instructions`
into the SHA-256. Instructions carry embedded schema ids in their operands
(`TypedObjectAlloc { schema_id }` — `bytecode/program_impl.rs:35`;
`NewTypedObject`, `MergeObject`). `NewTypedObject` even truncates
`schema_id as u16` (`collections.rs:966`). Therefore:

- **Blob content hashes ALREADY depend on counter-allocated ids** → two nodes
  compiling byte-identical source under different registration order produce
  **different blob hashes** → cross-node dedup and content-addressed cache
  hits silently fail. This is a pre-existing latent correctness/perf bug, not
  introduced by structural identity.
- `type_schemas: Vec<String>` (`:105`) hashes schema *names* (stable), so that
  sub-field is fine; the instability is purely the operand ids.
- **Structural ids make blob hashes DETERMINISTIC across processes/nodes** →
  strictly BETTER for cross-node dedup. This is a positive interaction, and a
  strong argument for the migration. (It does mean blob hashes CHANGE once —
  a one-time cache invalidation on rollout; see Phase 3 migration.)

### (f) TYPE-EQUALITY / comparison / inline caches

| Site | Count | Order-sensitive? |
|------|-------|------------------|
| `EqTypedObject`: `a.schema_id == b.schema_id` (`comparison/mod.rs:390`) | 1 | **YES** — two structurally-identical objects from different registries compare **unequal**; structural identity fixes this |
| property inline cache `PropertyCacheEntry.schema_id` (`feedback.rs:63`, match `feedback.rs:245`) | several | **YES** — IC keyed on id; misses/mis-hits across registries |
| megamorphic cache `hash_key(schema_id, field)` (`megamorphic_cache.rs:44`) | 1 | **YES** — field-access cache key |
| IC fast path `entry.schema_id == runtime_schema_id` (`ic_fast_paths.rs:101`) | 1 | **YES** |
| `any_error_schema_id` compare (`execution.rs:1102`) | 1 | **YES** (works today only because seeded consistently) |
| type-assertion `schema_id == type_id` (`typed_object_ops.rs:836`) | 1 | **YES** |

Type equality is nominal-by-id, which is *only sound* if id is a function of
structure. Under counters it is accidentally sound within one registry and
unsound across registries.

### (g) JIT (18 shape-jit files, ~83 sites)

`crates/shape-jit/src/{mir_compiler/{places,rvalues,statements,mod},executor,
context,foreign_bridge,ffi_symbols/object_symbols,ffi/{data,conversion,
object/property_access,call_method/mod},ffi/typed_object/{mod,allocation,
field_access,ffi_exports,merge_ops}}.rs`. The JIT lowers field access to a
fixed byte offset resolved from the schema the `schema_id` operand names, and
embeds `schema_id` into compiled FFI allocation/merge calls
(`ffi/typed_object/allocation.rs`, `merge_ops.rs`).

- **Order-sensitive: YES, transitively** — every JIT offset/alloc derives from
  the same `by_id` resolution as the interpreter, so a colliding id miscompiles
  identically (or worse, bakes the wrong offset into native code).
- No JIT site *mints* ids; they all consume the compile-time operand. So the JIT
  needs **no logic change** under structural identity — it just receives stable,
  correct ids. This bounds the migration: (g) is downstream-only.

### Blast-radius summary

| Group | Load-bearing sites | Order-sensitive | Mints ids? |
|-------|--------------------|-----------------|------------|
| (a) assignment | ~50 (2 counters + `with_id` consts) | YES (root) | YES |
| (b) lookup | ~117 reads | YES (`by_id`); by_name mostly no | no |
| (c) snapshot | ~5 core (persist + resume decode) | YES + cross-process | no (persists) |
| (d) wire | 3 | YES if populated | no |
| (e) content hash | 1 hash input (via `instructions`) | YES — already unstable | no |
| (f) equality/IC | ~7 | YES | no |
| (g) JIT | ~83 across 18 files | YES (transitive) | no |

Only group (a) mints identity. Everything else consumes it. That is the shape
of a clean single-source-of-truth fix: **change how ids are derived in (a)**,
and (b)-(g) become correct without per-site edits — no parallel discriminator,
no shim. The one site that materially *improves* is (e): blob hashes go from
order-unstable to deterministic, which is a cross-node dedup win.

## Design

### 0. Shape of the fix (one sentence)

Make schema identity a **function of schema structure**, minted at exactly one
place, so that identical structure always yields the same identity and distinct
structure never shares one — independent of registration order, registry,
process, or node — while keeping the hot-path `u32` operand that bytecode / JIT
offsets / inline caches already depend on.

### 1. Two layers: canonical content id + derived intern handle

The design is **not** "make `SchemaId` a 256-bit hash everywhere". That would
blow up every bytecode operand, every JIT field-offset lowering, and every IC
key from 4 bytes to 32, and it would collide catastrophically with the existing
`schema_id as u16` truncations (`collections.rs:895/966/1025/1052/1592`). Instead
identity is layered:

- **`SchemaContentId([u8; 32])` — the canonical identity.** A SHA-256 over the
  schema's *structure*. This is the single source of truth. It is what crosses
  process / node boundaries (wire, snapshot, content-addressed blob hashing).
- **`SchemaId = u32` — a derived, registry-local intern handle.** Kept exactly
  as today for hot paths, but its allocation changes: it is minted **only** by
  interning a `SchemaContentId` in the registry's content→handle table, never by
  a blind counter.

The `u32` handle is a **pure function of the canonical content id within a
registry** — the same relationship `StringId(u32)` already has to the interned
string (`crates/shape-vm`, string interning). It is a cache/index, not a second
identity, so it **cannot drift**: there is no code path that assigns a handle
except `intern(content_id)`. This is the load-bearing distinction from a
parallel discriminator (see ADR assessment §C).

```text
schema structure ──SHA-256──▶ SchemaContentId([u8;32])   (canonical, cross-node)
                                      │
                         registry.intern(content_id)
                                      ▼
                               SchemaId(u32)              (local handle, hot path)
```

### 2. Interning replaces both counters

`register()` (`registry.rs:145`) and the two counters (A per-registry
`allocate_id`, B ambient `allocate_current_id`) are replaced by one operation:

```text
fn intern(&mut self, content_id: SchemaContentId, schema: TypeSchema) -> SchemaId {
    if let Some(&h) = self.by_content.get(&content_id) {
        return h;                       // identical structure ⇒ identical handle (dedup)
    }
    let h = self.next_handle;           // dense; only advances on a NEW structure
    self.next_handle += 1;
    self.by_content.insert(content_id, h);
    self.by_id.insert(h, schema.name.clone());
    self.by_name.insert(schema.name.clone(), schema);   // (name-collision handled per §7)
    h
}
```

Why this kills the family:

- **Object-spread (the repro).** The merged `[x,y,z]` schema (Counter B) and the
  inline `[z]` schema (Counter A) have **different** content ids, so
  `intern` gives them **different** handles. `by_id.insert` can never place a
  different structure at an occupied handle, because a handle is only ever handed
  out for one content id. The silent-overwrite step is gone; `lookup_schema(M2)`
  returns the 3-field layout. `MakeFieldRef field_idx 2` passes.
- **json/xml id-41 collision (WF-2E).** `XmlNode` (3 fields) and `Json` (2
  fields) have different content ids ⇒ different handles. `resolve_typed_object_
  schema`'s arity heuristic (`json_value.rs:435`) becomes dead code.
- **Cross-registry equality (blast-radius (f)).** `EqTypedObject` compares
  handles. Two structurally identical objects intern to the **same** handle
  within a registry ⇒ compare equal. (Cross-*runtime* equality resolves on the
  content id — see Open Question OQ-1.)
- The numeric handle value still depends on encounter order **within a
  registry**, but *nothing keys correctness on the numeric value matching across
  a boundary* anymore: every boundary re-derives the handle by interning the
  content id it carries.

Dense allocation also keeps handles small, so the latent `schema_id as u16`
truncations stay safe in practice (a raw hash-as-id would have made them
catastrophic — a second reason to keep the `u32` handle dense rather than
hash-derived).

### 3. Computing the canonical content id

`compute_content_hash` (`schema.rs:247`) already hashes `{name, fields (name +
type string), enum variants}` with SHA-256 — 90% of the canonical id. Two
required changes for it to be a *layout* identity:

1. **Hash fields in DECLARATION order, not sorted.** The current impl sorts by
   field name (`schema.rs:256`). For a memory-layout identity that is wrong:
   `{x:int, y:int}` and `{y:int, x:int}` have different offsets but would hash
   identically. Field order is layout-significant (see OQ-3 for whether Shape
   *wants* order-significant object identity — it is required for correctness of
   the offset tables regardless).
2. **Include the resolved `FieldType` layout, not just its `to_string()`.**
   Fold in `size`/`alignment`/`offset` (or a canonical encoding of `FieldType`)
   so two types that print the same but lay out differently cannot alias. Recurse
   on nested `Object("Foo")` by *name only* (as today) to bound recursion; nested
   structural change is caught because the nested type has its own content id and
   any layout-affecting difference reaches the outer offsets.

Canonical bytes are versioned (a 1-byte scheme tag prefix) so the hash function
can evolve without silently changing identity — see Migration §M4.

### 4. Collision resistance

- **Handle collisions: structurally impossible.** A `u32` handle is a dense
  intern index handed out once per distinct content id; two different content ids
  never receive the same handle.
- **Content-id collisions: 2⁻¹²⁸ birthday bound** on the full 32-byte SHA-256.
  Even at 10⁹ live schemas the probability is ~10⁻²². We keep the **full 32
  bytes** as the `by_content` key (not a truncation) so there is no engineered
  fallback path to maintain — a SHA-256 collision is treated as
  cryptographically unreachable, the same assumption the content-addressed blob
  system (`content_addressed.rs`) already makes for `FunctionBlob.content_hash`.
- No probabilistic fallback, no rehash-on-collision loop. Introducing one would
  reintroduce an order-dependent disambiguator — precisely the `ensure_next_id_
  above` anti-pattern we are deleting.

### 5. Determinism across processes / nodes

Because the content id is a pure function of structure and SHA-256 is fixed, a
schema built from the same declaration on any node yields the **same
`SchemaContentId`**. This is the property counters can never have and is the
precondition for three things that are broken or unusable today:

- **Wire (blast-radius (d)).** `WireValue.schema_id: Option<u32>` becomes
  `Option<SchemaContentId>` (or carries both — see OQ-2). A value shipped by
  `remote::execute` from node X is re-interned on node Y to Y's local handle;
  the projection resolves the *correct* structure. The current `Some(1)`
  placeholder (`codec.rs:149`) is replaced by the real content id.
- **Snapshot (blast-radius (c)).** `SV::TypedObject` persists the content id
  instead of the raw handle. Resume re-interns against the resuming program's
  registry (`resume.rs:430`) instead of trusting a counter position to line up.
  This removes the "identical registration order at snapshot-time and
  resume-time" requirement entirely.
- **Content-addressed blob hash (blast-radius (e)) — the biggest win.** Today
  `FunctionBlobHashInput.instructions` embed counter-allocated handle operands,
  so byte-identical source compiled under different registration order produces
  **different** blob hashes → cross-node dedup silently fails. Under structural
  identity, when computing the blob hash we substitute each schema-id operand
  with the referenced schema's **content id** (the blob already ships a schema
  table, `type_schemas`). Blob hashes become **deterministic across nodes** →
  cross-node dedup and cache hits start working. This is strictly better, and is
  the strongest argument that structural identity is not just a bug-fix but an
  enabler for the distributed lane.

### 6. Where the handle stays u32 (no format churn)

Runtime bytecode operands, JIT field-offset lowering (all 18 shape-jit files,
blast-radius (g)), IC keys, and the interpreter's `by_id`/field tables keep
using `SchemaId = u32` unchanged. They are downstream-only consumers: they
receive stable, correct, deduped handles and need **no logic change**. Only the
boundary channels (§5) learn about `SchemaContentId`. This bounds the migration.

### 7. Name collisions become visible (a latent second bug)

Interning by content dedups structure, but `by_name` still keys on name. The
synthetic `__merged_L_R` / `__inline_obj_N` names (`collections.rs:1101`,
`type_tracking.rs:1135`) are partial structural keys already; with a real content
id they become redundant and can be dropped, and any genuine name collision
(two different structures, same user type name across modules) surfaces as an
explicit registry decision instead of a silent `by_name.insert` overwrite. This
is scoped as follow-up, not part of the root fix (OQ-5).

---

## Migration

### M1. The minimal correct root-fix (one lane, in-process)

Land these together; they fix the object-spread repro, the json/xml id-41 case,
and cross-registry equality without touching the wire/snapshot format:

1. Add `SchemaContentId([u8;32])` and `TypeSchema::content_id()` (declaration-
   order hash, §3), reusing the existing `compute_content_hash` machinery.
2. Add `by_content: HashMap<SchemaContentId, SchemaId>` to `TypeSchemaRegistry`;
   route `register` / `register_type` / `register_type_scoped` / `register_enum_
   scoped` / `TypeSchema::new` through **`intern`** (§2). Delete the ambient
   Counter B path (`allocate_current_id`, `schema.rs:20`); make the per-registry
   dense counter serve only intern-index allocation.
3. **Delete the point-patches** made dead by the fix:
   `ensure_next_id_above` / `ensure_next_schema_id_above` and their 5 call sites;
   the `merge()` id-collision reallocation loop + `id_remap` protocol
   (`registry.rs:536-545`); `resolve_typed_object_schema`'s arity heuristic
   (`json_value.rs:435`) → collapse back to a single `get_by_id`. (The `reserved`
   flag's *inference-guard* role may stay pending OQ-5; its *id-collision* role
   is gone.)
4. Un-`#[ignore]` the object-spread repros
   (`mir_compiler/integration_tests.rs:~2096`, `jit/tiering.rs:~276`).

`with_id` callers keep working: `with_id(id, name, fields)` becomes
`intern(content_id_of(name, fields))` at the mint site. Builtin comparisons
(`storage.schema_id == schemas.result`, `result_option_carrier.rs`) compare
against the `SchemaCollection`'s interned handles, not literals, so they are
correct as long as `schemas.result` is itself an interned handle — which it is
once the builtins are registered through `intern`. **Audit item:** any site that
compares a `schema_id` to a *hardcoded numeric literal* must switch to comparing
against a named interned handle or a content id. (Grep found none in the hot set;
`schemas.*` fields are the compare targets.)

### M2. The cross-node follow-up (bounded, separable)

Serialization-format changes are a coherent second lane, gated on the distributed
work:

- Wire: `WireValue.schema_id: Option<u32>` → carry `SchemaContentId`; re-intern
  on receive (blast-radius (d), 3 sites).
- Snapshot: persist content id in `SV::TypedObject`; re-intern on resume
  (blast-radius (c), ~5 core sites). **Snapshot format version bump** — old
  counter-id snapshots are not resumable under the new scheme (acceptable:
  snapshots are ephemeral; see OQ-4).
- Blob-hash input: substitute content ids for handle operands in
  `FunctionBlobHashInput` (blast-radius (e), 1 site). **Blob hashes change once**
  → one-time content-addressed cache invalidation on rollout.

### M3. Is a compat shim needed — and is it a forbidden pattern?

**No runtime shim, and none is needed.** This is important because the codebase's
whole failure history (CLAUDE.md §Forbidden) is compat shims that became
permanent. Here:

- The `u32` handle is **not** a shim — it is the real hot-path identity, derived
  from the canonical id by interning. It is the blessed `StringId` interning
  relationship, not a `ValueWord`-style dynamic-fallback carrier.
- The migration is a **mint-point change + boundary re-serialization + deletion
  of patches**, with a **format-version bump** on snapshot/blob. A format version
  is not the forbidden class (that class is runtime dynamic dispatch / tag decode
  / dual-carrier bridges).
- **The one thing that WOULD be a forbidden pattern** and must be refused: a
  "resolve old counter-id by scanning candidates / arity heuristic" fallback kept
  alive next to the new path. That is `ensure_next_id_above` / `resolve_typed_
  object_schema` reborn under a new name. The migration **deletes** those; it must
  not re-add a dual-resolution path. Clean cutover, single source of truth.

### M4. Hash-scheme versioning

Prefix the canonical bytes with a 1-byte scheme tag. If the hash inputs ever
change (e.g. adding annotations to identity), the tag bumps and old content ids
remain interpretable as scheme-0. This avoids a silent identity change and is the
disciplined alternative to an ad-hoc migration.

---

## ADR assessment — an amendment IS required

### A. ADR-005 (single-discriminator)

`HeapValue` remains the value discriminator; `SchemaId` is layout metadata on
`TypedObject`, not a value discriminator, so ADR-005 §1 is not violated. But the
content-id/handle **duality** must be explicitly blessed and bounded, or a future
reader will (correctly, by CLAUDE.md's own rules) suspect a parallel
discriminator. The amendment states: *the `u32` handle is a derived intern index
of the single canonical `SchemaContentId`; it is minted only by `intern`; blind
counter allocation of schema identity is forbidden and re-introducing it (or an
order-dependent disambiguator such as `ensure_next_id_above`) is a defection.*

### B. ADR-006 §2.7.29 (wire)

The wire section specifies the on-wire carrier for values; it must be amended to
say a `TypedObject`'s schema is carried as `SchemaContentId` and re-interned on
receive, replacing the `Option<u32>` placeholder. Snapshot serialization
(§2.7.7/Q9 kind track is unaffected) gains the parallel statement for its
`schema_id` field.

### C. Content-addressed section

Record that blob-hash inputs substitute content ids for handle operands, that
blob hashes become deterministic across nodes (a dedup improvement), and that
rollout invalidates the existing blob cache once.

**Recommendation:** a single ADR-006 amendment (with an ADR-005 cross-note)
covering (A)+(B)+(C). `adr_needed = true`.

---

## Open questions for ratification

- **OQ-1 (handle scope): per-registry, per-Runtime, or process-wide intern
  table?** A process-wide table makes cross-registry *and* cross-runtime-in-
  process equality free, but reintroduces a process-global static — directly
  against the B1 de-globalization direction that just moved `NEXT_SCHEMA_ID` /
  `PREDECLARED_SCHEMA_CACHE` off statics. **Recommend per-Runtime** (thread the
  `by_content` table with the registry): matches the de-globalization direction;
  cross-runtime equality then resolves on the content id, not the handle. Needs
  a user call.
- **OQ-2 (wire carrier): content id only, or content id + handle hint?** Carrying
  only the 32-byte content id is cleanest but 8× the current `u32`. A content-id
  interning table shipped once per session + `u32` references is smaller but is a
  second mechanism. Recommend content-id-only first, optimize later if measured.
- **OQ-3 (object identity semantics): is `{x:int, y:int}` the same type as
  `{y:int, x:int}`?** Declaration-order hashing (required for offset-table
  correctness) makes them *distinct* identities. If Shape's type system considers
  them the *same* structural type, identity and type-equality diverge for object
  literals. Recommend order-significant identity (matches layout); confirm it
  matches the intended nominal/structural semantics.
- **OQ-4 (snapshot format break):** old counter-id snapshots become
  non-resumable under structural identity. Acceptable (snapshots are ephemeral
  dev/distributed artifacts)? Or is a one-release dual-read window wanted (noting
  that a dual-read *resolution heuristic* edges toward the forbidden pattern; a
  clean version-gated reject is safer)?
- **OQ-5 (name-collision hardening + `reserved` flag):** drop the synthetic
  `__merged_*` / `__inline_obj_*` names and the `reserved` id-guard as part of
  this lane, or defer to a follow-up? The id-collision role dies with the counter;
  the by-name inference-guard role is orthogonal and can stay.
- **OQ-6 (scope of lane):** ratify the M1/M2 split — land the in-process root-fix
  (M1) now, stage wire/snapshot/blob-hash serialization (M2) with the distributed
  lane? Or require the full cross-node story in one lane?
