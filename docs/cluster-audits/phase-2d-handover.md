# Phase 2d Handover — VM/JIT end-to-end completion

**Branch parent:** `bulldozer-strictly-typed` HEAD `89d5f5f` (W14-variant-codegen close).
**Predecessor:** `docs/cluster-audits/wave-14-15-16-playbook.md` (W14/W15/W16 large-team push, 8 sub-clusters closed 2026-05-10 / 2026-05-11).
**Goal:** finish the original directive — *"make VM and JIT work end-to-end"* — by closing the 7 architecturally-incomplete items the W14/W15/W16 milestone surfaced but did not resolve.

This document is the **shared ruleset and per-item brief** for the next session. Read §0 first. The rules in §0 are not optional — they encode hard-won discipline from W7→W16 that the prior session repeatedly had to recover when individual agents drifted.

---

## §0 — Rules that MUST stick (refuse on sight)

### Forbidden patterns

These are deleted from the codebase. **Do not reintroduce under any name**, including renames. If you encounter one in your own reasoning, stop and surface to the user.

1. **`ValueWord` / `ValueWordExt` / `ValueBits` / `tag_bits::*` resurrection** — the 8-byte tagged-word carrier and its dispatch helpers are deleted. The strict-typing migration's whole point was to delete them. No "shim", "FFI-boundary bridge", "compatibility helper" — those are CLAUDE.md `Renames to refuse on sight` entries.
2. **Bool-default fallback** for unknown kind. The correct response to "I don't have a kind for this slot" is `NotImplemented(SURFACE: …)` with a §-cite, not `kind: NativeKind::Bool` to make the compiler happy.
3. **Generic opcodes** (`Add`, `Sub`, `Lt` etc. without kind suffix). Deleted. Only typed variants exist.
4. **`Convert<X>To<Y>` opcodes** added to paper over a kind-tracker gap (the W4-δ `ConvertBoolToString` pattern).
5. **Resurrecting deleted shape under a renamed alias** (`LegacyResultData`, `OldRangeShape`, `MethodFnLegacy`, etc.). The original names are listed in CLAUDE.md; aliases of them are equally refused.

### Defection-attractor framings (refuse on sight)

Per CLAUDE.md broader-family rule, any descriptor of deleted dispatch using `(bridge|probe|helper|hop|translator|adapter|shim)` framing belongs to this family and is refused. Specifically:

- `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`
- "FFI-boundary bridge", "host-boundary normalization", "tag normalization", "ValueBits shim"
- "MethodFn translator", "dispatch-slice probe", "boundary adapter for handler ABI"

Describe deleted code by name (`tag_bits::is_tagged`) or by deletion-fate (`the deleted W-series pattern`), never by hypothetical role.

### ADR-006 §2.7 discipline

Every new `HeapKind` variant requires (the **4-table lockstep rule**):

1. `crates/shape-vm/src/executor/vm_impl/stack.rs` — arm in `clone_with_kind` AND `drop_with_kind`
2. `crates/shape-value/src/kinded_slot.rs` — arm in `Drop` AND `Clone` impls
3. `crates/shape-value/src/v2/closure_layout.rs` — arm in `SharedCell::drop`
4. `crates/shape-value/src/heap_value.rs` — arm in `TypedObjectStorage::drop`

If you add one and skip the others you have a refcount bug. The grep verification is:

```bash
for kind in NewKind; do
  cnt=$(grep -lE "HeapKind::$kind\b" \
    crates/shape-value/src/heap_value.rs \
    crates/shape-value/src/kinded_slot.rs \
    crates/shape-value/src/v2/closure_layout.rs \
    crates/shape-vm/src/executor/vm_impl/stack.rs | wc -l)
  echo "$kind: $cnt/4"
done
```

Plus knock-on arms in: `printing.rs`, `arithmetic/mod.rs::kind_type_name`, `comparison/mod.rs::kind_type_name`, `typed_access.rs::kind_type_name`, JSON/wire conversion (reject or serialize), and the W16 `op_call_method` PHF classifier in `objects/mod.rs:494`.

### KindedSlot bounded API (ADR-006 §2.7.6 / Q8)

- One constructor per `NativeKind` heap variant: `from_typed_array`, `from_hashmap`, `from_hashset`, etc. ✓
- Scalar accessors only: `as_i64`, `as_f64`, `as_bool`, `as_char`, `as_str` (5 total, matching the scalar `NativeKind` variants). ✓
- **NO per-heap-variant accessors** on `KindedSlot` itself. `as_typed_array()`, `as_hashset()` etc. are forbidden — heap dispatch goes through `slot.slot.as_heap_value()` + `HeapValue` match, OR through the typed-Arc recovery pattern described next.

### The 5-arm receiver-recovery soundness rule (W13→W16 lesson)

`ValueSlot::from_X(arc)` stores `Arc::into_raw(Arc<XData>) as u64` directly. Those bits are NOT a `HeapValue` allocation — they are an `XData` allocation. **Casting to `*const HeapValue` is wrong-type recovery and segfaults.**

Sound recovery pattern (the canonical reference is `iterator_methods::clone_typed_array_arc`):

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

This pattern was applied to set/hashmap/deque/channel/priority_queue handlers in commit `3ac2f11`. Any new method-handler file MUST follow it.

### Surface-and-stop discipline

When you hit a genuine architectural gap (missing kind source, missing cascade dependency, missing ADR ruling):

```rust
return Err(VMError::NotImplemented(format!(
    "Operation X: SURFACE — <one-sentence reason>. \
     Tracked as <sub-cluster name> per <playbook ref>. \
     ADR-006 §2.7.<X>.",
)));
```

The cite must reference a real ADR § paragraph. "Surface-and-stop" is NOT a euphemism for "leak a Bool-kind null"; it is a hard return with a structured error.

### Merge-verification rule (W14/W15/W16 lesson)

**`cargo check ... | grep -c '^error\['` does NOT report cargo's exit status.** With `--message-format=short`, errors are prefixed `path.rs:N:M: error[...]` not `error[...]` at column 0. Three merges in the prior session were declared clean while shape-value actually had 50+ compile errors.

Always verify with explicit exit-code check:

```bash
cargo check --workspace --lib && echo "CLEAN" || echo "FAILED"
```

Or:

```bash
cargo check --workspace --lib 2>&1 | tail -3   # look for "Finished" or "error: could not compile"
```

When merging an agent's branch with the take-both regex (the `python3` script for `<<<<<<<` / `=======` / `>>>>>>>` resolution), the regex misses three common merge artifacts:

1. **Missing closing braces** inside dispatch tables (`HeapKind::X => { body  // next-arm comment  HeapKind::Y => {`). Scan with:
   ```bash
   python3 -c "import re
   for f in ['kinded_slot.rs', 'v2/closure_layout.rs', 'heap_value.rs', 'vm_impl/stack.rs', 'printing.rs']:
       # Look for HeapKind::X => { ... HeapKind::Y => without intervening }
       ..."
   ```
2. **Duplicate import blocks** from stitched `use crate::heap_value::{...}` lines.
3. **Modules with their opening discarded** but tests retained (the `mod result_option_storage` issue in W14 merge).

After any take-both pass, `cargo check --workspace --lib` MUST pass before commit. Do not commit a broken intermediate "for now".

### Ordinal-collision rule (pre-assigned + bump-on-merge)

The current HeapKind ordinal table at HEAD `89d5f5f` is 0..28. Free ordinals: 29+. If a future agent's pre-assigned ordinal collides at merge:

1. Bump to the next free.
2. Add provenance comment: `// 29  (Wave N agent <X>, 2026-MM-DD; renumbered from drafted 27 at merge — <Y> already took 27)`.
3. Update ADR amendment's ordinal mention in the same edit.
4. Update any AGENTS.md row.

Five collisions happened in W14/W15/W16 (Channel 23→24, PQ→25 in playbook revision, Range 30→26, Result 23→27, Option 24→28). Plan for it.

### Audit before rebuild

3 of 8 W15 sub-clusters audit-pivoted away from the playbook's prescribed rebuild:

- **W15-matrix → no new HeapKind** (MatrixData lives under `HeapKind::TypedArray` via `TypedArrayData::Matrix`; adding parallel HeapKind would violate ADR-005 §1 single-discriminator)
- **W15-column → DELETION** (semantics absorbed by `HeapKind::TableView` + `TableViewData::ColumnRef`)
- **W15-range → kept separate from IteratorState** (Range is a value with identity, Iterator is a cursor — different lifecycle)

Before adding any new HeapKind, do the audit. Cite ADR-005 §1 if the candidate is parallel to an existing carrier.

### What NOT to put in commits

- No `Co-Authored-By: Claude` trailer (user preference, see `MEMORY.md`).
- No "blame pre-existing" — feedback memory: own all code quality.

---

## §1 — Item-by-item plan

The 7 items the prior session surfaced as architecturally incomplete, in suggested order. Each item has: trigger, scope, files, gate, risk.

### Item 1 — Audit re-run (DO THIS FIRST)

**Trigger:** the user's original directive at session start was "i want to know if all the todos are in this plan and will be fixed." After 20 commits across W7→W16, the stub/TODO surface has shifted. Need a fresh inventory before allocating work.

**Scope:**

1. Re-run the SURFACE count: `grep -rE "NotImplemented\(.*SURFACE|todo!\(\"phase-2c|unimplemented!\(" crates/`. Current count at `89d5f5f`: 439 hits across ~50 files. Categorize by:
   - **Method handlers** (`objects/*_methods.rs`) — likely the bulk, candidates for new sub-clusters
   - **Opcode handlers** (`executor/*/mod.rs`) — fewer but higher-impact
   - **Storage tier** (`crates/shape-value/`) — should be near-zero; investigate any hits
2. Cross-reference each SURFACE site with its cited § paragraph. Verify the § actually exists in `docs/adr/006-value-and-memory-model.md` (current range: §2.7.1..§2.7.23, all numbers should be in-range).
3. Produce `docs/cluster-audits/phase-2d-stub-inventory.md` with one row per surface site: file:line, method/opcode name, cited §, blocker description, recommended sub-cluster.
4. Group into wave sub-clusters using the same shape as wave-14-15-16-playbook §2 (one sub-cluster per coherent territory).

**Gate:** inventory document exists; categorized by file area; every surface has a §-cite present in ADR-006; one sub-cluster suggestion per group of related surfaces.

**Effort:** 1-2 hours (mostly mechanical grep + categorize). Single agent, no parallelism needed.

**Risk:** low. This is research, not code.

---

### Item 2 — Method-handler SURFACE bodies (the bulk)

**Trigger:** W16 wired the dispatch shell so receivers reach the right `*_METHODS` PHF, but ~50 PHF entries are still `NotImplemented(SURFACE)`. Programs that call e.g. `dt.add_days(7)`, `mu.lock()`, `lazy.get()`, content rendering, table joins all hit SURFACE.

**Scope (high-confidence sub-clusters from the current grep):**

| Sub-cluster | Surface count | Cited § | Recipe template |
|---|---|---|---|
| W17-datetime-methods | 1 in `datetime_methods.rs` | §2.7.4 | `set_methods` (W13 hashset close `0da1477`) |
| W17-instant-methods | (check inventory) | §2.7.4 | same |
| W17-content-methods | 2 in `content_methods.rs` | §2.7.4 | same |
| W17-concurrency-methods | 1 in `concurrency_methods.rs` (Mutex/Atomic/Lazy ctors + methods) | §2.7.4 + cross-task §2.7.4 boundary | new HeapKinds — full 4-table rebuild |
| W17-hashmap-mutation | 2 in `hashmap_methods.rs` (set/delete/merge) | §2.7.15-style | `hashset` mutation arc in `set_methods.rs::v2_add` (commit `3ac2f11`) |
| W17-channel-recv-blocking | 1 in `channel_methods.rs` (cross-task `recv`) | §2.7.4 task-scheduler boundary | requires §2.7.4 wire-up first; defer |
| W17-array-joins/sorts/sets | 11 across array_joins/sort/sets/operations | §2.7.6 / §2.7.10 | mechanical bodies, no new HeapKind |
| W17-indexed-table-methods | 2 in `indexed_table_methods.rs` | §2.7.4 | depends on Table mutation ADR |

**Files to touch per sub-cluster:** typically just `<name>_methods.rs` for the bodies. Carrier (HeapKind / HeapValue arm / dispatch tables) already exists for everything except `concurrency_methods` (Mutex/Atomic/Lazy ctors are still stubbed in `vm_impl/builtins.rs`).

**Required reading per sub-cluster agent:**
1. `wave-14-15-16-playbook.md` §0
2. **This document** §0 (especially the 5-arm receiver-recovery rule)
3. ADR-006 §2.7.6 / Q8 (bounded carrier API)
4. ADR-006 §2.7.10 / Q11 (`MethodFnV2` ABI shape)
5. Canonical recipes: `set_methods.rs` (post-`3ac2f11`), `iterator_methods.rs` (commit `52c8ef5`).

**Gate per sub-cluster:**
- `cargo check --workspace --lib` exit 0 (verified via exit code, not grep)
- `cargo test -p shape-vm --lib <sub_cluster>` 100% pass
- `bash scripts/check-no-dynamic.sh` exit 0
- AGENTS.md row updated
- ADR amendment if a new HeapKind landed

**Effort:** parallelizable — ~6-8 sub-clusters, each ~1-2 days. Total ~1-2 weeks elapsed if sequential; ~2-3 days with 4-6 parallel agents.

**Risk:** medium. The Mutex/Atomic/Lazy rebuild (W17-concurrency) is genuinely new HeapKind territory; same recipe shape as W15-channel but cross-task semantics will surface the §2.7.4 task-scheduler boundary again.

**Bug-class to watch:** the W13 storage-shape mismatch was fixed in 5 files but if a new sub-cluster copy-pastes the OLD `as_X` template (return `&Arc<XData>` via `slot.as_heap_value()`) it will segfault under W16 dispatch. Reviewer must check every new `as_X` helper uses `Arc::from_raw + clone + into_raw` (the post-`3ac2f11` shape).

---

### Item 3 — JIT path end-to-end verification

**Trigger:** the original directive was "VM and JIT work end-to-end". Only VM is partially validated. JIT path (`shape run --mode jit`) was never re-tested after Phase-2c HeapKind expansion.

**Scope:**

1. Map every new HeapKind to its JIT path:
   - `Deque` / `Channel` / `PriorityQueue` / `Range` / `Result` / `Option`: do JIT callsites construct/consume these? If a hot-path function returns `Result<int, string>`, does the JIT-compiled return path retain/release the `Arc<ResultData>` correctly?
   - Check `crates/shape-jit/src/ffi/value_ffi.rs` and `crates/shape-jit/src/ffi/jit_kinds.rs` — these are the JIT-side counterparts of `clone_with_kind` / `drop_with_kind`. Do they have arms for ords 23-28?
2. Smoke programs (run with `--mode jit`, baseline-tier @ 100 calls, optimizing-tier @ 10k):
   ```
   let r = if x > 0 { Ok(42) } else { Err("nope") };  # hits ResultData
   let s = Set(); s.add("a"); s.size()                # hits HashSetData
   let r = 0..10; for i in r { print(i) }             # hits RangeData → IteratorState
   ```
3. Compare JIT output to VM output for each. Divergence = bug.

**Files to touch:**
- `crates/shape-jit/src/ffi/value_ffi.rs` (FFI tag layer — likely needs ord 23-28 arms)
- `crates/shape-jit/src/ffi/jit_kinds.rs` (JIT-side NativeKind)
- `crates/shape-jit/src/mir_compiler/*` (any kind-keyed switch that's now non-exhaustive)
- `crates/shape-jit/src/optimizer/*` (the speculative IC may have ord-keyed branches)

**Gate:**
- For each smoke program: VM result == JIT result == expected (run both modes, diff).
- No deopt loops (compile→deopt→compile cycle on a single function indicates an unhandled type).
- `cargo test -p shape-jit --lib` clean (already 394 tests passing per CLAUDE.md; new ords may add non-exhaustive matches).

**Effort:** ~3-5 days. Single agent (cross-cutting investigation, harder to parallelize). If new HeapKind support requires FFI extension, scope grows.

**Risk:** **high**. Past sessions deferred this and the W-series defections happened in the JIT path. CLAUDE.md "Forbidden Patterns" lists `synthesize_value_word_from_raw` and `is_tagged()` as deleted — but those were FFI-tier helpers. If the JIT FFI was rebuilt without them, the new HeapKinds may need explicit FFI plumbing OR may transparently work via the existing `Arc::into_raw + jit_kinds.rs` path. Audit before assuming.

**Hard rule for this item:** the JIT MUST NOT reintroduce `ValueWord` or any of its W-series renames. If you find yourself reaching for a "tag bits decode at the JIT FFI boundary", surface to user instead.

---

### Item 4 — Collection mutation semantics (language design question)

**Trigger:** the W16 close smoke `let s = Set(); s.add("a"); s.size()` returns 0 because `.add()` is functional (returns a new Arc<HashSetData>, doesn't mutate the binding). This is observable user-facing behavior that may or may not match the language's intent.

**Scope:**

1. **First, get a user ruling.** This is a language-design question, not a bug. Two reasonable answers:
   - **Functional**: `s.add(x)` returns a new Set; users must `let s = s.add(x)` to mutate. Matches Clojure/Elixir. Means the current behavior is correct.
   - **In-place when single-share**: when `Arc<HashSetData>` has refcount==1, `Arc::make_mut` mutates in place AND the slot bits at the binding site update. Requires the dispatch shell to write back the new slot bits to the receiver location after the method returns.
2. If the answer is "in-place mutation", the implementation is:
   - Method handlers that mutate return their result via a different ABI variant (or set a `mutated_receiver: Option<KindedSlot>` slot in the return carrier).
   - `op_call_method` checks for the mutated-receiver signal and writes it back to the receiver's slot location (which it knows because it pop'd the receiver from a specific stack index).
   - This is an ADR-006 §2.7.10 amendment.
3. If "functional", document it: ADR-006 §2.7.10 ruling that method-call dispatch is pure-value-return, no implicit receiver mutation.

**Gate:** ADR amendment landed (either §2.7.10 functional-only clarification or §2.7.10 in-place mutation ABI). No code change without that ruling.

**Effort:** ruling decision is 30 min user conversation. Implementation if in-place: ~2-3 days plus all-method-handler audit for mutation correctness.

**Risk:** medium-high. The mutation question cascades: if Sets mutate, Maps must too, and Deques, and Channels. The W15 close commits all use the functional shape; flipping to in-place is a multi-file edit.

**Recommendation:** present both options to the user with a worked example. Do not implement either side without explicit go-ahead.

---

### Item 5 — Pre-existing bench compilation

**Trigger:** `cargo check -p shape-vm --benches` fails with `error[E0432]: unresolved import shape_value::ValueWordExt` etc. The playbook explicitly accepted this as out-of-scope but it's a real open hole that blocks `cargo bench` and any CI that checks `--all-targets`.

**Files:**
- `crates/shape-vm/benches/vm_benchmarks.rs`
- `crates/shape-vm/benches/typed_access_bench.rs`

**Scope:** rewrite the benches to use the post-strict-typing API. The benchmarks measure VM perf; they should:
- Push values via `vm.push_kinded(KindedSlot::from_*)` (not `ValueWord::from_*`)
- Pop via `vm.pop_kinded` (not `ValueWord` decode)
- Test opcodes by their typed names (`AddInt`, `LtInt`) — the generic `Add` / `Lt` are deleted

**Per CLAUDE.md "Benchmark Integrity":** the *benchmarks* must not be modified to flatter the compiler. The bench *infrastructure* (driver scaffold) is what we're rewriting; the actual workloads must remain semantically identical. Keep diff scoped to "API migration only".

**Gate:** `cargo check --workspace --all-targets` exit 0 (assuming v8 build issue from Item 6 is also resolved or scoped out). `cargo bench --no-run -p shape-vm` succeeds.

**Effort:** ~1 day. Single agent. Mechanical port.

**Risk:** low. Benchmark outputs are not load-bearing for correctness.

---

### Item 6 — `--all-targets` gating

**Trigger:** prior session's gate was only `--lib`. Several bins (`shape-app`, `shape-server`) fail to build because they depend on `rusty_v8` which requires a network-fetched prebuilt binary or v8 source compilation. Unrelated to type system.

**Scope:**

1. Decide the gate's scope. Options:
   - **Tighten gate**: skip v8-dependent crates explicitly via `cargo check --workspace --lib --exclude shape-app --exclude shape-server`. Document the exclusion in `justfile`.
   - **Fix v8**: vendor the v8 binary or document the env-var setup in `CLAUDE.md` build commands section. Heavier.
2. Whichever path, update `justfile` recipes (`test-check`, `test-fast`, `test`, `test-all`) to use the chosen gate consistently. The recipes already exist per CLAUDE.md but may not be aligned.
3. Add a `just check-clean` recipe that runs the canonical clean-check command, so future sessions can verify "workspace clean" with one invocation.

**Gate:** `just check-clean` exits 0 on every push commit on `bulldozer-strictly-typed`. Document in `CLAUDE.md` what's covered and what's excluded.

**Effort:** ~2-4 hours. Single agent.

**Risk:** low.

---

### Item 7 — Merge-process discipline

**Trigger:** the prior session declared "workspace clean" three times when shape-value actually had 50+ compile errors, because `cargo check ... | grep -c '^error\['` doesn't match `--message-format=short`'s error format. Plus 8 take-both regex misses in the W14/W15/PQ merges.

**Scope:**

1. Write `scripts/verify-merge.sh`:
   ```bash
   #!/usr/bin/env bash
   set -euo pipefail
   # Verify workspace lib + tests check cleanly. Uses exit code, not grep.
   cargo check --workspace --lib
   cargo check --workspace --lib --tests
   bash scripts/check-no-dynamic.sh
   # Scan for residual merge markers
   ! grep -rnE '^<<<<<<<|^=======$|^>>>>>>>' --include='*.rs' --include='*.md' crates docs
   # Scan for HeapKind ordinal collisions
   python3 -c "
   import re, sys
   with open('crates/shape-value/src/heap_variants.rs') as f:
       lines = f.readlines()
   seen = {}
   for line in lines:
       m = re.match(r'\s+(\w+),\s*//\s*(\d+)', line)
       if m:
           name, ord_ = m.group(1), int(m.group(2))
           if ord_ in seen:
               print(f'COLLISION: ord {ord_}: {seen[ord_]} vs {name}', file=sys.stderr)
               sys.exit(1)
           seen[ord_] = name
   "
   # Scan for the 5-arm bug pattern: an as_X helper using slot.as_heap_value()
   # while ValueSlot::from_X stores typed-Arc bits.
   python3 scripts/check-receiver-recovery.py  # TODO write this
   ```
2. Make this script the gate for merging any agent branch. Document in CLAUDE.md and any future playbook.
3. Augment the take-both Python script to **fail loudly** if any conflict block survives the regex (current script silently leaves unresolved markers). Add a final pass that scans for the four common merge-survival patterns:
   - Orphan `>>>>>>>` without preceding `<<<<<<<` / `=======`
   - `=======` standalone
   - `HeapKind::X => { body  // next-arm-comment  HeapKind::Y =>` without intervening `}`
   - Triple-stitched `use` blocks with duplicate identifiers

**Gate:** `scripts/verify-merge.sh` exists, is executable, is called from CI / pre-commit, and was actually run before the last 3 merges of the next session.

**Effort:** ~1 day. Single agent. The receiver-recovery checker (item 7 substep) is non-trivial — may want to defer to a follow-up.

**Risk:** low. This is process hardening, not feature work.

---

## §2 — Suggested execution order

1. **Item 1 (audit re-run)** — must come first; everything else depends on the inventory.
2. **Item 7 (merge-process discipline)** — write `verify-merge.sh` second so subsequent items use it.
3. **Item 4 (mutation semantics)** — ask user, get ADR ruling. Decision blocks any method-handler work that mutates receivers.
4. **Item 2 (method handlers)** — parallel sub-clusters once the mutation ruling lands. Most of the remaining work by volume.
5. **Item 5 (benches)** — independent; can run in parallel with Item 2.
6. **Item 6 (--all-targets)** — touch-up after Item 5 (benches were blocking).
7. **Item 3 (JIT verification)** — last, because new method handlers from Item 2 may expand the JIT FFI surface.

---

## §3 — What "done" looks like

Phase 2d is complete when:

- [ ] Stub inventory exists and every entry is either filled, deleted, or has an explicit deferral ADR.
- [ ] `shape run --mode vm <complex-program>` produces correct output for every smoke target in the inventory.
- [ ] `shape run --mode jit <same-program>` produces the same output as VM (mod permissible JIT/VM differences like rounding).
- [ ] `cargo check --workspace --all-targets` exit 0 (or documented exclusion list).
- [ ] Mutation-semantics ADR amendment landed (whichever direction).
- [ ] `scripts/verify-merge.sh` runs as part of CI / pre-commit.
- [ ] HeapKind ordinal table is contiguous 0..N with no collision provenance comments (renames cleaned up).
- [ ] All `NotImplemented(SURFACE)` sites either filled or cite a tracked sub-cluster.

**Tag the close:** `git tag phase-2d-close <hash>` once these all hold.

---

## §4 — Continuity from W14/W15/W16

Branch state at handover (`bulldozer-strictly-typed` HEAD `89d5f5f`):

- HeapKind ordinals 0..28 in use, contiguous, no collisions.
- ADR-006 §2.7.1..§2.7.23 present.
- 33+ storage tests pass across 5 collection types (hashset, deque, channel, priority_queue, result_option).
- 4 dispatch tables in lockstep for every variant.
- W16 op_call_method classifier has arms for every HeapKind in the table.
- `--lib` builds clean, `--all-targets` does not (benches + v8).
- `check-no-dynamic.sh` exit 0.
- AGENTS.md current as of W14 close.

Key commits to read before starting:

| Commit | Purpose |
|---|---|
| `0da1477` | W13-hashset rebuild — the canonical recipe for "add a HeapKind sibling" |
| `52c8ef5` | W13-iterator-state — second canonical recipe (typed-Arc, NOT pure-discriminator) |
| `3ac2f11` | The 5-arm receiver-recovery fix — DO NOT regress this pattern |
| `e4c3a36` | Salvage of stalled W15-priority-queue from worktree; merge-resolution example |
| `89d5f5f` | W14-variant-codegen merge (current HEAD) — Result/Option carriers |

Worktrees from W14/W15/W16 (`shape-w15-*`, `shape-w16-*`, `shape-w14-*`) can be removed via `git worktree remove`. Branches can be deleted after the audit confirms nothing is owed.

---

*End of handover. Read §0 again before starting any sub-cluster.*

## Close

Phase 2d closed on 2026-05-12 at commit `e22bffd2`, tagged `phase-2d-close`. The VM-path strict-typing migration is complete; the JIT path remains pre-existing structurally broken (phase-2c W10 §2.7.14 SURFACE — `jit_new_array` stub at `crates/shape-jit/src/ffi_symbols/array_symbols.rs:30` aborts every JIT compilation; independently confirmed by Item 3 verification on programs that don't touch arrays). JIT rebuild is Phase 3 cluster-0 territory. See `docs/cluster-audits/phase-2d-close-summary.md` for the full close artifact (delivery summary, ADR amendments, HeapKind ordinal table, hardening backlog status, open Wave-3 surfaces, worktree cleanup authorization) and `docs/cluster-audits/phase-3-kickoff-prompt.md` for the Phase 3 cluster-0 supervisor contract.
