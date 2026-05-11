# Phase 2d — Sub-Cluster Playbook (agent dispatch contract)

**Status:** binding for Phase 2d sub-cluster dispatch.
**Companion docs:** `phase-2d-handover.md` (rules), `phase-2d-stub-inventory.md` (sites), `docs/adr/006-value-and-memory-model.md` §2.7.24 (carrier shapes), `scripts/verify-merge.sh` (merge gate), `AGENTS.md` (roster).
**Branch parent:** `bulldozer-strictly-typed` at HEAD `45bd827` (or later — verify with `git rev-parse HEAD`).

This document is what supervisors hand to agents at dispatch time. Each sub-cluster has a complete, self-contained prompt. Agents read **§0 (shared discipline)** + their assigned **§N (sub-cluster prompt)**, in that order, before touching code.

---

## §0 — Shared discipline (every agent reads this first)

### Required reading (in order, no skipping)

1. **`docs/cluster-audits/phase-2d-handover.md` §0** (the rules section). Re-read in full. Hard-won discipline from W7→W16. The rules are not optional.
2. **`CLAUDE.md`** sections "Forbidden Patterns" + "Renames to refuse on sight" + "Single-discriminator discipline (ADR-005)" + "Value & memory model (ADR-006)". In their entirety. The forbidden-pattern and rename-refusal lists are immutable — do not add or remove entries; that is supervisor territory.
3. **`docs/adr/006-value-and-memory-model.md`** sections cited in your sub-cluster's "Required reading" line (typically §2.3, §2.7.6, §2.7.7, §2.7.8, §2.7.10, §2.7.11, §2.7.15, plus §2.7.24 if you touch storage carriers).
4. **Canonical recipe commits** (read with `git show <hash>`):
   - `0da1477` — W13-hashset rebuild — the canonical recipe for adding a HeapKind sibling.
   - `52c8ef5` — W13-iterator-state — second canonical recipe (typed-Arc dispatch label).
   - `3ac2f11` — the 5-arm receiver-recovery soundness fix. DO NOT regress this pattern.
   - `89d5f5f` — W14-variant-codegen merge (Result/Option carriers).

### Refuse-on-sight forbidden patterns

If you encounter any of these in code, commit messages, your own reasoning, or another agent's PR, **stop and surface to supervisor**:

1. **`ValueWord` / `ValueWordExt` / `ValueBits` / `tag_bits::*` resurrection.** Deleted. Under any name.
2. **Bool-default fallback** when a kind source is missing. The correct response is `Err(VMError::NotImplemented(SURFACE: …))` with §-cite, never `kind: NativeKind::Bool` to make the compiler happy.
3. **Generic opcodes** (`Add`, `Sub`, `Lt`, etc. without kind suffix). Deleted.
4. **`Convert<X>To<Y>` opcodes** added to paper over a kind-tracker gap (the W4-δ `ConvertBoolToString` pattern).
5. **Resurrecting deleted shape under a rename:** `LegacyResultData`, `OldRangeShape`, `MethodFnLegacy`, `TypedArrayData::HeapValue` (deleted by §2.7.24 Q25.A), `HashMapData::values: Arc<TypedBuffer<Arc<HeapValue>>>` (deleted by §2.7.24 Q25.B), `Box<u64>` data half of trait-object carrier (deleted by §2.7.24 Q25.C).

### Defection-attractor framings (refuse on sight)

Any descriptor of deleted dispatch using `(bridge|probe|helper|hop|translator|adapter|shim)` framing. Specifically:

- `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`
- "FFI-boundary bridge", "host-boundary normalization", "tag normalization", "ValueBits shim"
- "MethodFn translator", "dispatch-slice probe", "boundary adapter for handler ABI"
- "polymorphic fallback", "catch-all element buffer", "any-shaped array carrier" (new under §2.7.24)
- `(array|hashmap|trait.object) (catchall|polymorphic.fallback|any.element|heap.value.element|generic.element) (arm|variant|carrier|buffer)`

Describe deleted code by name (`tag_bits::is_tagged`, `TypedArrayData::HeapValue`) or by deletion-fate (`the deleted W-series pattern`), never by hypothetical role.

### Surface-and-stop discipline

When you hit a genuine architectural gap (missing kind source, missing cascade dependency, missing ADR ruling):

```rust
return Err(VMError::NotImplemented(format!(
    "Operation X: SURFACE — <one-sentence reason>. \
     Tracked as <sub-cluster name> per <playbook ref>. \
     ADR-006 §2.7.<X>.",
)));
```

The cite must reference a real ADR § paragraph (current range §2.7.1..§2.7.24). "Surface-and-stop" is NOT a euphemism for "leak a Bool-kind null"; it is a hard return with a structured error.

### 4-table HeapKind lockstep rule

If you add a new `HeapKind` variant, you MUST add arms in all 4 dispatch tables:

1. `crates/shape-vm/src/executor/vm_impl/stack.rs` — `clone_with_kind` AND `drop_with_kind`
2. `crates/shape-value/src/kinded_slot.rs` — `Drop` AND `Clone` impls
3. `crates/shape-value/src/v2/closure_layout.rs` — `SharedCell::drop`
4. `crates/shape-value/src/heap_value.rs` — `TypedObjectStorage::drop`

Plus knock-on arms in: `printing.rs`, `arithmetic/mod.rs::kind_type_name`, `comparison/mod.rs::kind_type_name`, `typed_access.rs::kind_type_name`, JSON/wire conversion (reject or serialize), and the W16 `op_call_method` PHF classifier in `objects/mod.rs`.

Verification: `bash scripts/verify-merge.sh` (CHECK 6 — HeapKind 4-table lockstep) catches misses automatically.

### 5-arm receiver-recovery soundness rule

`ValueSlot::from_X(arc)` stores `Arc::into_raw(Arc<XData>) as u64` directly. Those bits are NOT a `HeapValue` allocation — they are an `XData` allocation. **Casting to `*const HeapValue` is wrong-type recovery and segfaults.**

Sound recovery pattern (canonical reference: `iterator_methods::clone_typed_array_arc`, post-`3ac2f11`):

```rust
let bits = slot.slot.raw();
if bits == 0 { return Err(type_error("null slot bits")); }
// SAFETY: per the construction-side contract on KindedSlot::from_X,
// kind=Ptr(HeapKind::X) bits are Arc::into_raw(Arc<XData>) and the slot
// owns one strong-count share. Reconstruct, clone, restore.
let arc = unsafe { Arc::<XData>::from_raw(bits as *const XData) };
let cloned = Arc::clone(&arc);
let _ = Arc::into_raw(arc);
Ok(cloned)  // owned Arc<XData>, NOT &Arc<XData>
```

Any new `as_X` helper that uses `slot.as_heap_value()` for typed-Arc slot bits is WRONG. The `verify-merge.sh` CHECK 10 flags suspicious sites as a heuristic; reviewer must verify the pattern matches `3ac2f11` shape.

### Merge verification

**`cargo check ... | grep -c '^error\['` does NOT report cargo's exit status.** Always use:

```bash
cargo check --workspace --lib && echo CLEAN || echo FAILED
```

Or simply:

```bash
bash scripts/verify-merge.sh    # the full Phase-2d merge gate, 11 checks
just verify-merge               # same via justfile
just verify-merge-fast          # skips --tests pass for iteration speed
```

Run before every commit, not just before merge. A failing intermediate commit blocks merging the branch.

### Worktree / branch naming

For parallel agent dispatch:

```
branch:   bulldozer-strictly-typed-w17-<sub-cluster-slug>
worktree: ../shape-w17-<sub-cluster-slug>
```

Examples: `bulldozer-strictly-typed-w17-make-closure`, `../shape-w17-make-closure`.

Create with:

```bash
git worktree add ../shape-w17-<slug> -b bulldozer-strictly-typed-w17-<slug>
```

### AGENTS.md row update

At sub-cluster start: edit your row in `AGENTS.md` from `idle` to `migrating`, set `active cluster` to the W17-* slug, set `files owned` to the rg pattern matching your territory. Supervisor confirms no overlap before greenlighting.

At sub-cluster close: flip back to `idle`, clear `active cluster`, update "last update" date, note the close commit hash.

On surface-and-stop: flip to `blocked` and stash WIP. Supervisor triages stashes at session start.

### Commit message hygiene

- **No `Co-Authored-By: Claude` trailer.** (User preference.)
- **No "blame pre-existing" framings.** Own all code quality.
- **No `--no-verify` / `--no-gpg-sign` / hook skipping** unless supervisor explicitly authorizes.
- **Never `--amend` after a failed pre-commit hook** — fix the issue, re-stage, create a NEW commit.

### HeapKind ordinal pre-assignment

Current HeapKind ordinal table at HEAD `45bd827`: 0..28 in use. Pre-assigned for Phase 2d:

| Ord | Name | Sub-cluster | Status |
|---|---|---|---|
| 29 | TraitObject | W17-typed-carrier-monomorphization (P3) | reserved |
| 30 | Mutex | W17-concurrency | reserved |
| 31 | Atomic | W17-concurrency | reserved |
| 32 | Lazy | W17-concurrency | reserved |

Free for unexpected new HeapKind: 33+. If your pre-assigned ordinal collides at merge:

1. Bump to the next free.
2. Add provenance comment: `// 33  (Wave 17 agent <X>, 2026-MM-DD; renumbered from drafted 29 at merge — <Y> already took 29)`.
3. Update ADR-006 amendment's ordinal mention in the same edit.
4. Update your AGENTS.md row.

### Close-gate checklist (every sub-cluster)

Before marking your sub-cluster `idle`:

- [ ] `cargo check --workspace --lib` exits 0 (verified by exit code)
- [ ] `cargo test -p shape-vm --lib <relevant_test_module>` 100% pass for tests in your territory
- [ ] `bash scripts/verify-merge.sh` exits 0
- [ ] `bash scripts/check-no-dynamic.sh` exits 0
- [ ] AGENTS.md row updated (status `idle`, close commit hash noted)
- [ ] If you added a HeapKind: ADR-006 amendment landed; all 4 lockstep tables have the arm; ordinal-collision rule applied if needed
- [ ] If you migrated SURFACEs: the inventory's §1.A / §1.B row count for affected files reduces accordingly (regenerate `/tmp/phase2d-audit/surface-structured.tsv` if needed and diff)
- [ ] If your sub-cluster validates downstream tests (§3 of inventory): tests in the named files flipped from `todo!("phase-2c")` to actual bodies, ALL passing
- [ ] No `Co-Authored-By` in commits; no "blame pre-existing"; no `--no-verify`

---

## §1 — Wave 1 sub-clusters (gates)

These two run in parallel as the leading wave. Everything downstream benefits.

### T1 — `T1-host-tier-marshal-rebuild`

**Sub-cluster slug:** `t1-host-tier-marshal-rebuild`
**Branch / worktree:** `bulldozer-strictly-typed-t1-host-marshal` / `../shape-t1-host-marshal`
**Estimated effort:** 6-8h.
**Risk:** low (test infrastructure).
**Leverage:** unblocks ~200-300 test bodies for parallel re-fill in downstream sub-clusters.

**Territory.** Test-tier eval/marshal API rebuild. Functions like `eval()` / `eval_int()` / `eval_float()` / `eval_string()` / `eval_bool()` and the `Constant::Value(ValueWord)` carrier they depended on are deleted; ~290 `todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord) carrier)")` sites in 16 test files are blocked on this.

**Files in scope:**
- Helpers: wherever `eval()` / `eval_int()` / `eval_float()` / `eval_string()` / `eval_bool()` are defined (search with `rg 'pub fn eval' crates/shape-vm/src/`).
- `Constant::Value(...)` carrier (in compiler-tier constant-pool plumbing) — convert to a kinded shape.

**Smoke target.** After T1 lands, a representative test file (e.g. `crates/shape-vm/src/executor/tests/typed_array_ops.rs`) compiles with the test bodies still as `todo!()`, and the test helpers compile without `Constant::Value(ValueWord)`. Bonus: one or two test bodies actually re-filled with the new helper shape and passing.

**Required reading.**
- This playbook §0.
- ADR-006 §2.7.4 (KindedSlot heap accessors).
- ADR-006 §2.7.5 (cross-crate ABI policy) — host-tier IS post-proof.
- ADR-006 §2.7.5.1 (wire-format = post-proof shapes; same applies to test helpers).
- Recent test-helper-using commits for shape and idioms.

**ADR-006 implications.** None — implementation lands within the existing §2.7.4 / §2.7.5 framework. The rebuild produces KindedSlot-returning helpers; the test caller does the obvious extraction (`.as_i64()?` etc).

**Forbidden in this sub-cluster.** Do NOT re-introduce `Constant::Value(ValueWord)` under a renamed alias. Do NOT add a generic `eval_to_value()` returning a polymorphic carrier — every test helper has a strict expected kind; if a kind mismatch happens at runtime, that's the test's bug.

**Close gate.** §0 checklist + a representative test re-fill demonstrating the new helper shape.

---

### W17-make-closure

**Sub-cluster slug:** `w17-make-closure`
**Branch / worktree:** `bulldozer-strictly-typed-w17-make-closure` / `../shape-w17-make-closure`
**Estimated effort:** 4-6h.
**Risk:** medium-high.
**Leverage:** gates W17-array-closure-callback (14 sites) + several others.

**Territory.** `op_make_closure` at `executor/control_flow/mod.rs:~447` returning `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)`. The §2.7.8 `ClosureCell` cell layout (in `shape-value/v2/closure_raw.rs`) is the consumer-side foundation — already built. The gate is the *make* path: capture-cell construction at closure-creation time, threading per-capture `NativeKind` from the §2.7.7 stack parallel-kind track into the new `ClosureCell.kinds` parallel track.

**Files in scope.**
- `crates/shape-vm/src/executor/control_flow/mod.rs::op_make_closure` (primary).
- `crates/shape-value/src/v2/closure_raw.rs` — `ClosureCell` already implements the layout; verify your impl uses `alloc_typed_closure` + `write_capture_kinded` correctly per §2.7.8 Q10.
- Any compiler-side `OpCode::MakeClosure` emit-site quirks (search with `rg 'OpCode::MakeClosure' crates/shape-vm/src/compiler/`).

**Smoke target.**
```shape
let f = |x| x + 1
print(f(5))                                          // 6

let xs = [1, 2, 3]
let doubled = xs.map(|x| x * 2)                      // gates W17-array-closure-callback
print(doubled)                                       // [2, 4, 6]
```

**Required reading.**
- This playbook §0.
- ADR-006 §2.7.8 (Q10 cell-storage kind-awareness) — in full.
- ADR-006 §2.7.11 (Q12 value-call ABI) — the consumer side, already landed.
- Canonical commit `b7c9770` (W7-cv-method execute_closure refinement).
- `executor/call_convention.rs::call_value_immediate_nb` body — already kinded; your op_make_closure feeds this.

**ADR-006 implications.** None — implementation lives within existing §2.7.8 / §2.7.11. If you discover a genuine architectural gap (capture-kind source missing for some emit pattern), surface-and-stop with §2.7.8 cite.

**Forbidden in this sub-cluster.** Bool-default fallback for capture kinds (§2.7.8 #4). Any "frame-setup probe" / "callee-kind helper" / "capture-injection adapter" framing (CLAUDE.md "Renames to refuse on sight"). Restoring `_upvalue_bits: Vec<u64>` or any kind-blind capture vector.

**Close gate.** §0 checklist + the smoke target above runs via VM and prints expected output.

---

## §2 — Wave 2 sub-clusters (independent body-fills)

Up to 8 agents in parallel after Wave 1 lands. Each is self-contained.

### W17-array-typed-receiver

**Slug:** `w17-array-typed-receiver` · **Effort:** 3-5h · **Risk:** low.

**Territory.** `array_operations.rs` ArrayPush / ArrayPushLocal / ArrayPop / SliceAccess for non-`Ptr(HeapKind::TypedArray)` receivers.

**Sites (7):** `array_operations.rs:108,148,189,260,270,381,419,484`.

**Smoke target.**
```shape
let xs = [1, 2, 3]; xs.push(4); print(xs)            // [1,2,3,4]
let ys: Array<String> = ["a"]; ys.push("b"); print(ys)
let s = xs[0..2]; print(s)                           // [1,2]
```

**Required reading.** §0 + ADR-006 §2.7.7 + §2.7.8. Reference impl: existing `TypedArrayData::I64` ArrayPush body.

**ADR-006 implications.** None.

**Forbidden.** Reintroducing legacy `HeapValue::Array` carrier. Generic-VW-array fallback.

**Close gate.** §0 checklist + smoke + the 7 sites no longer SURFACE.

---

### W17-iterator-tableview

**Slug:** `w17-iterator-tableview` · **Effort:** 4-6h · **Risk:** medium.

**Territory.** `executor/loops/mod.rs` op_iter_done / op_iter_next for TableView/DataTable rows, HashMap key/value pairs, TypedArrayData::HeapValue iteration.

**Sites (~9):** `loops/mod.rs:229,239,253,379,392,399,410,522` + `iterator_methods.rs:224`. NOTE: 2 of the sites are ADR-005 §1 violation diagnostics (iter_kind ≠ heap arm) — those are kind-tracking bugs to **fix**, not SURFACEs to **fill**.

**Smoke target.**
```shape
for row in table.rows() { print(row.name) }          // TableView row iteration
for (k, v) in map { print(k, v) }                    // HashMap pair iteration
```

**Required reading.** §0 + ADR-006 §2.7.4 + §2.7.16 (Iterator typed-Arc shape).

**ADR-006 implications.** Possibly minor — if `TypedArrayData::HeapValue` iteration needs material from §2.7.24 Q25.A, you must coordinate with W17-typed-carrier-monomorphization (probably wait for P1 of that branch to land first; surface-and-stop if you hit it).

**Forbidden.** Generic Array iteration shape; the deleted `HeapValue::Array` arm.

**Close gate.** §0 checklist + smoke + the 9 sites resolved (2 fixed as bugs, 7 filled).

---

### W17-references-mutation

**Slug:** `w17-references-mutation` · **Effort:** 4-6h · **Risk:** medium · **Gates:** W17-typed-object-mutation.

**Territory.** `executor/variables/mod.rs` DerefStore/SetIndexRef and DerefLoad through TypedIndex — the Wave-β B6 continuation cited in `closure_raw.rs:1617`.

**Sites (4):** `variables/mod.rs:3113`, `:3676` + 2 CITE-MISSING.

**Smoke target.**
```shape
let mut p = P { x: 1 }; let r = &mut p.x; *r = 2; print(p.x)   // 2
```

**Required reading.** §0 + ADR-006 §2.7.13 (Q14 kinded `RefTarget`) + §2.7.8 (cell-storage kind invariant).

**ADR-006 implications.** None — §2.7.13 already covers the design.

**Forbidden.** Generic "ref decode bridge" framings. Polymorphic `RefTarget`.

**Close gate.** §0 checklist + smoke + the 4 sites filled. Document CITE-MISSING fixes in the close commit.

---

### W17-typed-object-mutation

**Slug:** `w17-typed-object-mutation` · **Effort:** 6-8h · **Risk:** medium · **Blocked by:** W17-references-mutation.

**Territory.** TypedObjectStorage field write paths + property-access for non-`Ptr(TypedObject)` receivers + heterogeneous receiver-kind GetProp/SetProp.

**Sites (~11):** `typed_object_ops.rs:164,179,248,429`; `property_access.rs:193,302,396,418,444`; `objects/object_creation.rs:×2`.

**Smoke target.**
```shape
type P { x: int, y: string }
let p = P { x: 1, y: "a" }
p.x = 2; print(p.x)                                  // 2
p.y = "b"; print(p.y)                                // "b"
```

**Required reading.** §0 + ADR-006 §2.7.4 + §2.7.7 + §2.7.8. W17-references-mutation commit (close hash from that branch).

**ADR-006 implications.** None.

**Forbidden.** `clone_slots_with_update` shortcut paths that bypass kind threading.

**Close gate.** §0 checklist + smoke + all sites filled.

---

### W17-builtin-coercions

**Slug:** `w17-builtin-coercions` · **Effort:** 3-5h · **Risk:** low.

**Territory.** `executor/builtins/type_ops.rs` — `as string` for heap kinds + `Convert` opcode (TryInto/Into trait dispatch).

**Sites (2):** `type_ops.rs:451,503`.

**Smoke target.**
```shape
print((123).toString())                              // "123"
print((decimal("1.5")).toString())                   // "1.5"
let r: Result<int, MyErr> = (10 as int).tryInto()
```

**Required reading.** §0 + ADR-006 §2.7.4. `executor/printing.rs::format_heap_kind` (which needs Phase-2c rebuild — see if it's still SURFACE; if so this sub-cluster cascades).

**ADR-006 implications.** None expected. Surface-and-stop if format_heap_kind is itself SURFACE.

**Forbidden.** Generic stringification helpers that take `Arc<HeapValue>` and decode per-arm at runtime — use per-kind formatters dispatched by `KindedSlot.kind` per §2.7.6 Q8 carrier-API bound.

**Close gate.** §0 checklist + smoke + 2 sites filled.

---

### W17-foreign-ffi

**Slug:** `w17-foreign-ffi` · **Effort:** 6-8h · **Risk:** medium.

**Territory.** `extern C` FFI rebuild — `foreign_marshal` (marshal_args / unmarshal_result) + `op_call_foreign`.

**Sites (3):** `control_flow/foreign_marshal.rs:57,74` + `control_flow/mod.rs:680`.

**Smoke target.**
```shape
extern C fn libm_sin(x: number) -> number
print(libm_sin(0.0))                                 // 0.0
```

**Required reading.** §0 + ADR-006 §2.7.4 + §2.7.5 (cross-crate ABI policy). Pre-rebuild msgpack body in commit `afb1651` (cited in CLAUDE.md `v2-raw-heap-audit`).

**ADR-006 implications.** None — §2.7.5 already specifies the wire boundary.

**Forbidden.** Resurrecting the msgpack-based `vw_clone` / `vw_drop` retain pattern (CLAUDE.md `v2-raw-heap-audit` known-constraint). Use kinded retain/release per §2.7.5.

**Close gate.** §0 checklist + smoke + 3 sites filled.

---

### W17-typed-module-exports

**Slug:** `w17-typed-module-exports` · **Effort:** 6-8h · **Risk:** medium.

**Territory.** Typed module export round-trip across wire protocol — `transport_builtins.rs`, `remote_builtins.rs`, `shape-vm/remote.rs`.

**Sites (~5 production todo!()s):** `transport_builtins.rs:81`, `remote_builtins.rs:84`, `remote.rs:640,655`.

**Smoke target.** Module export across QUIC wire protocol round-trips with kinds preserved.

**Required reading.** §0 + ADR-006 §2.7.4 + §2.7.5.1 (wire-format post-proof shapes). Wire protocol v1 docs at `crates/shape-wire/src/lib.rs:51`.

**ADR-006 implications.** Likely none. The "+ addendum" cite in the existing todo!() messages may reference a §2.7.4 extension already drafted — search for it before designing.

**Forbidden.** Generic `Value` carrier across the wire (kind-blind). Use KindedSlot or its wire-equivalent per §2.7.5.1.

**Close gate.** §0 checklist + smoke + 5 sites filled.

---

### W17-array-closure-callback

**Slug:** `w17-array-closure-callback` · **Effort:** 4-6h · **Risk:** medium · **Blocked by:** W17-make-closure.

**Territory.** Array methods consuming `op_make_closure`: joins, sorts, query family.

**Sites (~14):**
- `array_joins.rs:76,117,148` — innerJoin, leftJoin, crossJoin
- `array_sets.rs:539` — distinctBy
- `array_sort.rs:163,189` — orderBy, thenBy
- `array_query.rs:271,285,482,498` + 2 — where, select, takeWhile, skipWhile, find, forEach

**Smoke target.**
```shape
print([1,2,3].orderBy(|x| -x))                       // [3,2,1]
print([{a:1},{a:2},{a:1}].distinctBy(|x| x.a))       // 2-elem
```

**Required reading.** §0 + ADR-006 §2.7.10 (Q11 MethodFnV2) + §2.7.11 (Q12 value-call). W17-make-closure close commit.

**ADR-006 implications.** Possibly — `TYPED_ARRAY_BUILDER_SURFACE` constant indicates a missing output-buffer factory. If the heterogeneous-result case is genuinely needed, surface-and-stop and cite §2.7.6 Q8.

**Forbidden.** Closure-callback ABI shims (`call_value_with_u64_slice`, etc. per CLAUDE.md §2.7.11 forbidden list).

**Close gate.** §0 checklist + smoke + 14 sites filled.

---

### W17-native-scalar-carrier

**Slug:** `w17-native-scalar-carrier` · **Effort:** 2-4h · **Risk:** low · **Pre-step:** verify scope.

**Territory.** 3 production `todo!("phase-2c — NativeScalar carrier")` sites in `executor/v2_stack_tests.rs:163,169,212`. **First action: confirm these are production helpers** (not test fixtures). If they're test fixtures only, drop the sub-cluster.

**Required reading.** §0 + ADR-006 §2.7.4.

**Close gate.** §0 checklist + sites resolved (filled or audit shows they're test-only and they're moved to T1 territory).

---

### W17-method-bodies-misc

**Slug:** `w17-method-bodies-misc` · **Effort:** 2-3h · **Risk:** low. **Absorbs:** the deque-ctor 5-line cleanup at `vm_impl/builtins.rs:636`.

**Territory.** Singleton SURFACEs + the stale Deque ctor.

**Sites:**
- `objects/bool_methods.rs:29` — bool.toString
- `vm_impl/schemas.rs` × 1, `vm_impl/modules.rs` × 1 — [CITE-MISSING]
- `vm_impl/builtins.rs:636` — DequeCtor (stale; HeapKind::Deque=23 already exists, mirror the ChannelCtor body)

**Smoke target.**
```shape
print(true.toString())                               // "true"
let d = Deque(); d.push_back(1); print(d.size())     // 1
```

**Required reading.** §0 + ADR-006 §2.7.6 Q8. Adjacent body `vm_impl/builtins.rs:ChannelCtor` as recipe.

**Close gate.** §0 checklist + smoke + all listed sites filled.

---

### C1-temporal-lowering

**Slug:** `c1-temporal-lowering` · **Effort:** 4-6h · **Risk:** low · **Blocked by:** T1.

**Territory.** `compiler/expressions/temporal.rs` — 14 bytecode-emit sites for DateTime/Timespan operations citing §2.7.4 KindedSlot heap accessors.

**Sites (14):** `temporal.rs:108,113,118,123,128,...`

**Smoke target.**
```shape
let d = date(2026,1,1)
let later = d + days(30)
print(later)                                         // 2026-01-31
```

**Required reading.** §0 + ADR-006 §2.7.4. T1 close commit. Existing `KindedSlot::as_*` heap-accessor methods.

**Close gate.** §0 checklist + smoke + 14 sites filled.

---

## §3 — Wave 2.5 sub-clusters (large independent / ADR-amending)

### W17-snapshot-resume

> **RESCOPED 2026-05-11** (Wave 2.5 close). The original "snapshot/resume round-trip + comptime introspection" scope split into two sub-clusters after the agent's audit identified upstream blockers that prevent a single-session bundled rebuild:
>
> - **W17-snapshot-surfaces** (this sub-cluster's actual close — commit `0db5920` merged in Wave 2.5) — converts 27 production `todo!()` panic sites to structured `Err(NotImplemented(W17 surface))` returns. VM no longer process-aborts on snapshot operations; callers receive auditable structured errors. Identifies the `SerializableVMValue` wire-format gap (post-W14/W15/W16 HeapKinds lack arms).
> - **W17-snapshot-roundtrip** (queued for Wave 2.6 or later) — the actual round-trip rebuild. Blocked on: (a) `invoke_module_fn_id_stub` at `vm_impl/modules.rs:75` (upstream, out of W17-snapshot scope); (b) ADR-006 §2.7.5.1 SerializableVMValue extension for post-W14/W15/W16 HeapKinds (`HashSet`, `Iterator`, `Result`, `Option`, `Deque`, `Channel`, `PriorityQueue`, `Reference`, `FilterExpr`, `SharedCell`) — must land its own §2.7.5.1 amendment; (c) kind-threaded `slot_to_serializable` / `serializable_to_slot` API alignment. Effort estimate: 12-18h.
>
> The original `W17-snapshot-resume` row in `AGENTS.md` is marked "rescoped"; see the new `W17-snapshot-surfaces` and `W17-snapshot-roundtrip` rows. The rest of this entry preserved verbatim as historical context.

**Slug:** `w17-snapshot-resume` · **Effort:** 8-16h · **Risk:** medium-high · **Independent.**

**Territory.** Snapshot/resume rebuild — 27 production `todo!()` sites that panic the VM on snapshot/restore operations.

**Sites (~25):**
- `executor/state_builtins/introspection.rs` × 9
- `executor/state_builtins/core.rs` × 8
- `executor/vm_state_snapshot.rs` × 8
- `executor/snapshot.rs` × 2
- `executor/resume.rs` × 2 SURFACE (uses `PHASE_2C_SNAPSHOT_SURFACE` constant)
- `executor/mod.rs:476` — `apply_pending_resume` stub
- `runtime/data/cache.rs` × 2 + `runtime/context/mod.rs` × 2

**Smoke target.**
```shape
let s = snapshot()
// ... mutate state ...
resume(s)
// state restored
```

**Required reading.** §0 + ADR-006 §2.7.4 + §2.7.5.1 (wire-format post-proof shapes — same shape applies to snapshot serialization).

**ADR-006 implications.** Possibly — the snapshot format MUST serialize every NativeKind. If the wire-format spec needs extension for new kinds (e.g. Result/Option), surface-and-stop and cite §2.7.5.1.

**Forbidden.** Generic snapshot serializer that decodes `Arc<HeapValue>` at runtime. Per-kind serialization via §2.7.6 Q8 carrier-API bound.

**Close gate.** §0 checklist + smoke + 27 sites filled. Comptime introspection (`type_info`, `implements`, etc.) restored.

---

### W17-typed-carrier-monomorphization

> **RESCOPED 2026-05-11** (Wave 2.5 close). The bundled 24-32h single-branch P1+P2+P3 framing was determined unexecutable in a single session after the agent's audit surfaced:
>
> - **Scale mismatch**: 119 references to `TypedArrayData::HeapValue` across 33 files (vs playbook's "~18 SURFACE sites"); 1104 total `TypedArrayData` arm references workspace-wide. Adding 9 new variants ripples through exhaustive matches.
> - **Stale playbook line numbers**: cited array_transform.rs lines (486/547/589/693/722/775/1140/1310) now point at unrelated `concat_typed_array` arms / sort comparators (post-W17-array-closure-callback file shifted).
> - **P3 upstream gap**: `OpCode::BoxTraitObject` is not emitted by the compiler. The P3 smoke target cannot be exercised without companion compiler-emission work not in original P3 scope.
>
> The agent surfaced-and-stopped without commits. ADR-006 §2.7.24 spec itself is correct — only this playbook's scope framing was off. Split into 3 sub-clusters for Wave 2.6:
>
> - **W17-typed-carrier-bundle-A** (replaces P1 + P2): single branch, 4 internal checkpoint commits — (1) ADDITIVE: add specialized `TypedArrayData` variants (Decimal/BigInt/DateTime/Timespan/Duration/Instant/Char/TypedObject/TraitObject) + `HashMapValueBuf` enum + `HashMapData` refactor (HeapValue arm still present; clean compile); (2) Migrate construction sites; (3) Migrate match/iteration/marshal/JSON/XML/printing sites to exhaustive-match new variants; (4) DELETE `HeapValue` arm + verify-merge.sh exit 0 + `check-no-dynamic.sh` exit 0. Each checkpoint passes verify-merge.sh independently. Q25.A "no partial migration" satisfied because commit 4 deletes the arm; intermediate compileable states are fine. Effort: ~16-20h. No new HeapKind ordinals.
> - **W17-trait-object-storage** (replaces P3 storage tier only): pre-assigned HeapKind ord **29 (TraitObject)**. Storage-tier work per §2.7.24 Q25.C — `TraitObjectStorage`, `HeapValue::TraitObject` arm, 4-table lockstep, 6-variant `VTableEntry` enum, `Erase_T` compiler-side type rewriting, per-(impl,method) thunk generation at vtable-construction time. Smoke: storage-tier-only via test harness (`Arc<TraitObjectStorage>` roundtrip). End-to-end deferred to trait-object-emission. Effort: ~8-12h.
> - **W17-trait-object-emission** (NEW — missed by original P3 framing): compiler-side dyn-coerce grammar (verify or extend), type inference for dyn coercion, bytecode emit for `OpCode::BoxTraitObject` + `DynMethodCall`, compiler-side vtable lookup at coerce-to-dyn time. **BLOCKED BY** `W17-trait-object-storage` (needs the carrier + vtable shapes to exist). Smoke: end-to-end §2.7.24 Q25.C example (`let a: dyn Animal = box(Dog{...}); print(a.name())`). Effort: ~10-14h.
>
> ADR-006 §2.7.24 stays unchanged; only this playbook's scope framing was off. The original `W17-typed-carrier-monomorphization` row in `AGENTS.md` is marked "rescoped"; see the three new rows. The rest of this entry preserved verbatim as historical context.

**Slug:** `w17-typed-carrier-monomorphization` · **Effort:** 24-32h · **Risk:** high · **Consumes ADR-006 §2.7.24.**

**Territory.** Bundled storage-tier rebuild per ADR-006 §2.7.24:
- **P1 (8-10h)** — Q25.A: delete `TypedArrayData::HeapValue` arm; add specialized variants (Decimal, BigInt, DateTime, Timespan, Duration, Instant, Char) + TypedObject + TraitObject catch-alls; migrate all SURFACE sites in `array_transform.rs`, `array_aggregation.rs`, `array_basic.rs`, `iterator_methods.rs`, `string_methods.rs`, `objects/concat.rs`.
- **P2 (4-6h)** — Q25.B: refactor `HashMapData` to parametric `HashMapValueBuf`; migrate `objects/typed_access.rs:172,192,212` + `datetime_methods.rs:426` cascade.
- **P3 (12-16h)** — Q25.C: re-introduce `HeapKind::TraitObject = 29` with `Arc<TraitObjectStorage>` carrier; implement the richer `VTableEntry` enum (6 variants); implement the `Erase_T` substitution + per-(impl,method) thunks per §2.7.24 Q25.C.1–C.7; migrate `trait_object_ops.rs:77,99,124,142` + `objects/mod.rs:251`. Apply 4-table lockstep rule for HeapKind::TraitObject.

**Sites (~33):** see §1.A of inventory, marked W17-typed-carrier-monomorphization.

**Pre-assigned HeapKind ordinal:** TraitObject = 29.

**Smoke targets per phase:**

```shape
// P1 (Q25.A)
print([1n, 2n, 3n].sum())                            // BigInt array
[date(2026,1,1), date(2026,2,1)].forEach(|d| print(d))

// P2 (Q25.B)
let m = HashMap<String, int>(); m["a"] = 1; print(m["a"])   // 1, via I64 buffer

// P3 (Q25.C)
trait Animal { fn name(&self) -> String; fn clone_me(&self) -> Self; }
impl Animal for Dog { fn name(&self) -> String { "dog" }; fn clone_me(&self) -> Dog { ... } }
let a: dyn Animal = box(Dog { ... })
print(a.name())                                      // "dog" [vtable]
let b = a.clone_me()                                 // [vtable + boxed-return]
print(b.name())                                      // "dog"
```

**Required reading.** §0 + ADR-006 §2.7.24 (in full) + §2.3 + §2.7.6 + §2.7.7 + §2.7.8 + §2.7.10 + §2.7.15 (W13-hashset recipe). Canonical commits `0da1477`, `52c8ef5`, `3ac2f11`, `89d5f5f`.

**ADR-006 implications.** §2.7.24 IS the implication. No further amendment expected. If you hit a genuinely-new gap during P3 (auto-boxing thunk shape disagrees with §2.7.24 Q25.C.4 wrap-target encoding for some real test case), surface-and-stop with a §2.7.24 amendment proposal.

**Forbidden (this sub-cluster especially).** Reintroducing `TypedArrayData::HeapValue` under any rename. `HashMapData::values: Arc<TypedBuffer<Arc<HeapValue>>>` shape. `Box<u64>` data half of trait-object carrier. Object-safety compile-rejection (universal `dyn` per §2.7.24 Q25.C.1). Any of CLAUDE.md "Renames to refuse on sight" §2.7.10 / §2.7.11 ABI defection-attractors (MethodFnLegacy, dispatch-slice probe, etc.).

**Close gate.** §0 checklist + all 3 phase smokes + verify-merge.sh CHECK 6 confirms HeapKind::TraitObject in 4/4 dispatch tables + 33 sites filled. ADR-006 §2.7.24 status updated from "binding for Phase 2d onward" to "implemented in commit <hash>".

---

### W17-concurrency

**Slug:** `w17-concurrency` · **Effort:** 8-12h · **Risk:** high · **Partial gate:** W17-make-closure for Lazy.get.

**Territory.** Mutex / Atomic / Lazy as new HeapKinds. ADR-006 §2.3 typed-Arc shape.

**Sites (~11):**
- `concurrency_methods.rs:40,52,64,78,90,102,114,126,144,156` (10) — Mutex.{lock,try_lock,set}, Atomic.{load,store,fetch_add,fetch_sub,compare_exchange}, Lazy.{get,is_initialized}
- `vm_impl/builtins.rs:678` (1) — Mutex/Atomic/Lazy constructor SURFACE

**Pre-assigned HeapKind ordinals:** Mutex=30, Atomic=31, Lazy=32.

**Smoke target.**
```shape
let m = Mutex(0)
m.lock(); m.set(5); print(m.value)                   // 5

let a = Atomic(0)
print(a.fetch_add(1))                                // 0
print(a.load())                                      // 1

let l = Lazy(|| expensive_computation())             // [closure-call gate: W17-make-closure]
print(l.get())                                       // result; cached
print(l.is_initialized())                            // true
```

**Required reading.** §0 + ADR-006 §2.3 + §2.7.15 (W13-hashset HeapKind add recipe in full) + §2.7.20 (W15-channel cross-task carrier — closest analog). Canonical commit `0da1477`.

**ADR-006 implications.** New ADR amendment in this sub-cluster's close commit: §2.7.25 (Mutex/Atomic/Lazy HeapKinds, mirror of §2.7.20 Channel rebuild structure). 3 new HeapValue arms (`Mutex(Arc<MutexData>)`, `Atomic(Arc<AtomicData>)`, `Lazy(Arc<LazyData>)`). 3 × 4-table lockstep applications. Use the W13-hashset recipe verbatim.

**Forbidden.** Generic "concurrency primitive" wrapper. Inline-scalar Mutex/Atomic (these are always heap). Re-using `HeapKind::SharedCell` for Mutex (different semantics — SharedCell is binding storage, Mutex is a runtime synchronization primitive).

**Close gate.** §0 checklist + smoke (Lazy.get depends on W17-make-closure landing; if it hasn't, leave Lazy.get with SURFACE and finish Mutex+Atomic) + 4-table lockstep verified for all 3 new ords + ADR-006 §2.7.25 landed.

---

### C2-comptime-rebuild

**Slug:** `c2-comptime-rebuild` · **Effort:** 6-10h · **Risk:** medium · **Independent.**

**Territory.** `compiler/comptime.rs` + `compiler/comptime_target.rs` — comptime evaluator rebuild against typed-Arc HeapValue layout. ADR-006 §2.4.

**Sites (~7):** `compiler/comptime.rs:392,520,557,580,592,...` + `compiler/comptime_target.rs:207`.

**Smoke target.**
```shape
comptime {
    let x = 1 + 2
    let arr = [x, x*2, x*3]
}
```

**Required reading.** §0 + ADR-006 §2.3 + §2.4 + §2.7.6.

**Forbidden.** Generic comptime-value carrier. The comptime evaluator must use the same KindedSlot interpretation as runtime.

**Close gate.** §0 checklist + 7 sites filled + a representative comptime block compiles + runs.

---

### C3-expr-lowering-misc

**Slug:** `c3-expr-lowering-misc` · **Effort:** 4-6h · **Risk:** low · **Independent.**

**Territory.** Misc compiler-tier bytecode-emit paths citing §2.4 / §2.7.4.

**Sites (~10):**
- `compiler/expressions/function_calls.rs:152,157,163,431`
- `compiler/expressions/property_access.rs:201,326`
- `compiler/expressions/mod.rs:1079`
- `compiler/expressions/misc.rs:751`
- `compiler/loops.rs:1410`
- `compiler/helpers.rs:4836`

**Smoke target.** Generic shape programs that exercise these compile paths compile cleanly.

**Required reading.** §0 + ADR-006 §2.4 + §2.7.4.

**Close gate.** §0 checklist + all sites filled + a smoke program exercising each compile path runs.

---

## §4 — Wave 3 sub-clusters (dependent)

### W17-datatable-results

**Slug:** `w17-datatable-results` · **Effort:** 6-10h · **Risk:** medium · **Blocked by:** W17-typed-object-mutation + W17-typed-carrier-monomorphization + W17-array-closure-callback.

**Territory.** DataTable methods that need property-access + monomorphic carriers + closure-callback to land first.

**Sites (7):**
- `datatable_methods/joins.rs:43,61` — innerJoin, leftJoin
- `datatable_methods/query.rs:396,460` — group_by, map
- `datatable_methods/aggregation.rs:573,591` — aggregate, parse_agg_spec
- `datatable_methods/simulation.rs:51` — simulate

**Smoke target.**
```shape
let dt = table([{x:1, y:2}, {x:3, y:4}])
print(dt.groupBy("x").aggregate({sum: "y"}))
```

**Required reading.** §0 + close commits of the 3 blocking sub-clusters + ADR-006 §2.7.4.

**Close gate.** §0 checklist + smoke + 7 sites filled.

---

### W17-window-join

**Slug:** `w17-window-join` · **Effort:** 4-6h · **Risk:** medium · **Blocked by:** W17-datatable-results.

**Sites (2):** `window_join.rs:385,480`.

**Smoke target.**
```shape
print(dt.windowJoin(other, "ts", duration("1h")))
```

**Required reading.** §0 + W17-datatable-results close commit + ADR-006 §2.7.6 (Temporal carrier).

**Close gate.** §0 checklist + smoke + 2 sites filled.

---

## §5 — Deferred sub-clusters (later handover items)

### W17-channel-blocking — deferred to post-Item-4

`channel_methods.rs:156,245` — Channel.recv() blocking-on-empty + Channel.is_sender role check.

Defer until Item 4 (mutation ADR) lands and §2.7.4 cross-task task-scheduler boundary has a home.

### W17-jit-stubs — deferred to Item 3 (JIT verification)

`shape-jit/src/ffi_symbols/vector/mod.rs:30`, `data_access/mod.rs:65`, `ffi/object/conversion.rs`.

Defer until W17 method-handler outcomes reveal which kinds flow through hot paths.

---

## §6 — Dispatch protocol (supervisor side)

1. **Pre-merge audit before dispatch.** Confirm no AGENTS.md row conflict on territory. Pre-assign HeapKind ordinal if relevant (29-32 reserved per §0).
2. **Worktree + branch creation** per §0 naming.
3. **Agent prompt** = this playbook §0 + the agent's assigned §N sub-cluster section.
4. **Status flip** at start: `idle → migrating` with active cluster slug.
5. **Mid-work surface-and-stop:** agent flips `migrating → blocked` and stashes WIP. Supervisor triages at session start (check `git stash list` in worktree).
6. **Close audit:** agent runs §0 close-gate checklist + `bash scripts/verify-merge.sh`. Both must exit 0.
7. **Merge:** supervisor merges agent's branch into `bulldozer-strictly-typed`. Runs `bash scripts/verify-merge.sh` again post-merge. If anything fails, fix-on-main (do not revert) and re-run.
8. **Inventory delta:** supervisor regenerates `/tmp/phase2d-audit/surface-structured.tsv` after the merge; diff against pre-merge to confirm the expected site count reduced.

---

## §7 — "Phase 2d done" checklist

Mirrors handover §3:

- [ ] Stub inventory exists and every entry is either filled, deleted, or has an explicit deferral ADR.
- [ ] `shape run --mode vm <complex-program>` produces correct output for every smoke target in the inventory.
- [ ] `shape run --mode jit <same-program>` produces the same output as VM.
- [ ] `cargo check --workspace --all-targets` exit 0 (or documented exclusion list).
- [ ] Mutation-semantics ADR amendment landed (Item 4 ruling).
- [ ] `scripts/verify-merge.sh` is part of CI / pre-commit.
- [ ] HeapKind ordinal table is contiguous 0..N with no collision provenance comments (renames cleaned up).
- [ ] All `NotImplemented(SURFACE)` sites either filled or cite a tracked sub-cluster.

**Tag the close:** `git tag phase-2d-close <hash>` once these hold.

---

*End of playbook.* For changes to discipline (§0 forbidden patterns, defection-attractor list, ordinal table), edit `CLAUDE.md` + this playbook + the handover doc in lockstep. Discipline drift is the failure mode every prior session has had.
