# Book-Acceptance Report — slice: ownership

Chapter (book-PRIMARY): `advanced/ownership-deep-dive.mdx`
Binary: `/home/dev/.../shape-strict-flip-collection-dispatch/target/release/shape` (HEAD, prebuilt)
Determinism strategy: pure (storage classes; var smart-default; escape→RC). No randomness/time/network used.

## Result summary

| Program | LOC | VM EC | JIT EC | VM==JIT stdout | Self-check |
|---------|-----|-------|--------|----------------|------------|
| small.shape | 62  | 0 | 0 | YES | ALL_CHECKS_PASSED |
| large.shape | 790 | 0 | 0 | YES | ALL_CHECKS_PASSED (149/149) |

Both programs are FULLY PASSING after author-error fixes + idiomatic workarounds
for the language defects logged below. Every defect was reproduced in isolation
and FIRST-RUN truth recorded before any workaround was applied.

## small.shape — what it proves (book sections cited)

Exercises the chapter core, all expected values derived from book semantics:
- "Copy Semantics" table — int/bool are Copy (`let m = n`, source stays usable).
- "The Binding System" — `var counter = 0` Direct stack-mutable.
- "Worked Example" — heap struct MOVE on `let q = p` (read the new owner).
- "Clone Trait" — `clone a` is an independent deep copy of an array; mutating
  the source (`a.push(40)` → len 4) leaves the copy frozen (len 3).
- "First-Class References" / "Auto-Referencing" — `let r = &x; r + 1`;
  `rr.len()` / `rr[0]` dispatch through a stored reference.
- "No Explicit Lifetimes" — `fn read_val(&v) { return v }` returns the VALUE.
- "Call Arguments: Share in v0.3.3" — `append(nums)` shares by value; the push
  is caller-visible (`nums.len() == 4`, `nums[3] == 4`) and `nums` stays usable.

All 12 checks pass under VM and JIT, byte-identical.

## large.shape — application

A deterministic, machine-proofable "VaultDB": a versioned in-memory document
store with a columnar commit history, full-Doc deep-copy snapshots, an
undo/restore path, a functional transform layer, and an end-to-end ledger
checksum. 149 assertions, each compared to a hand-computed expected value
written before the first run. Ownership properties proved:

- by-value call sharing (mutators grow the caller's live doc);
- clone / deep_copy isolation (committed snapshots & deep copies stay frozen
  while the live doc grows) — proved both columnar (scenario_store/timeline/
  checksum/ledger) and full-Doc (scenario_snapshot/clone_chain);
- first-class references + auto-ref reads (doc_* accessors take `&Doc`);
- map/filter/reduce produce fresh non-aliasing arrays (scenario_functional);
- scalar Copy semantics (scenario_copy).

The final `ledger.grand_checksum == 1030` is unreachable unless every ownership
rule held exactly.

## Failure classifications (FIRST-RUN truth)

### AUTHOR-ERROR (my typos — fixed, a real user would too)
1. `a.push(...)` on a `let` binding → "Cannot reassign immutable variable" —
   needs `let mut`. (Correct, expected strict behavior.)
2. `&Vec2{...}` (borrow of a struct literal) → "`&` can only be applied to a
   place expression". Correct: refs need a place.
3. `reduce(0, |a,v| ...)` → reduce is `reduce(f, init)`, not `(init, f)`. The
   compiler's error message even states the correct signature. Fixed.
4. `s.live = new_doc(...)` on a `let s` → needs `let mut s`. Correct.

### BOOK-WRONG / FN-REG-CORRECTNESS — `clone` keyword + auto-derived Clone for user TypedObjects
Book line 104: *"Clone is auto-derived for types whose fields are all Clone. The
`clone` keyword and `.clone()` method are equivalent"* and the snapshot idioms
imply cloning a struct parameter works. In practice, for a user TypedObject
`W { tags: Array<int> }` (all-Clone fields):
- `let b = clone a` (a = direct struct-literal local) — WORKS and deep-copies
  correctly (verified: src→4, copy→3).
- `return clone d` / `let c = clone d` where `d` is a `&`-ref param OR a
  by-value param → **"Undefined variable: 'clone'"** (every `return clone X`
  fails regardless of type, including arrays/strings).
- `clone inner` where `inner = v.doc` (move-out-of-field source) →
  **"Method 'clone' not found on type 'W'"**.
- `v.doc.clone()` / `tmp.clone()` (method form on a user struct) →
  **"Method 'clone' not found on type 'W'"** — contradicting "equivalent".
- `.clone()` on an ARRAY (`d.tags.clone()`) WORKS, including in return position.

Net: the `clone` keyword/`.clone()` on user TypedObjects only resolves in the
single `let X = clone <direct-local>` position; the book presents it as a
general auto-derived Clone. Workaround used: an explicit `deep_copy(&Doc)` that
rebuilds the struct field-by-field (scalars Copy; the `Array<int>` field cloned
via the working `.clone()` array method). Classified BOOK-WRONG (book says it
works; followed correctly, it fails) with a real correctness regression
underneath.

### FN-REG-CORRECTNESS — Array-of-TypedObject inside a struct corrupts (V3-S5 WIP)
The natural store shape `Store { history: Array<Version> }` (Version carries a
nested Doc with its own Array<int>) MISCOMPILES once the Store is mutated
through the by-value-shared field path and a committed element is read back.
Observed NON-DETERMINISTICALLY across runs of the same source:
- SIGSEGV (EC=139, "dumped core"),
- `thread 'main' panicked … capacity overflow` (EC=101),
- `Runtime error: Schema <hash> not found in registry`.
Minimal repro (st4.shape family): commit a deep_copy into `Array<Version>`,
mutate live, then read `s.history[0].clock`. This is the documented V3-S5
ckpt-5/6 "heap-element TypedArray" WIP SURFACE. Workaround: store the history as
parallel PRIMITIVE-ARRAY COLUMNS (idiomatic column-store); full-Doc snapshot
isolation proved separately via deep_copy into plain locals. This is a genuine
memory-safety defect on the array-of-nested-struct path.

### FN-REG-CORRECTNESS — empty-array literal `[]` runtime SURFACE
A bare `let x = []` (and `tags: []` in a struct-literal field, even when the
field type is statically known) hits
`Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5`.
A TYPE-ANNOTATED empty local (`let x: Array<int> = []`) works. Workaround:
bind every empty array through an annotated local. Idiomatic; logged as a
defect because the unannotated form is the obvious first thing a user writes.

### FN-REG-CORRECTNESS — `.map()` result will not unify with `Array<int>` param
Passing a `.map()` result (even transitively) into a `&Array<int>` parameter
that calls `.reduce` → spurious
`Could not solve type constraints: (Vec<int>) -> int is not compatible with
(Vec<int>) -> int` (two identical types). `.filter()` results are fine; inline
`.reduce` is fine. Explicit `let r: Array<int> = src.map(...)` fixes it.
A strict-flip inference-loss regression (sibling to the documented typed-closure
inference cluster). Workaround: annotate map-result locals.

## book_gaps (book silent; I had to consult behavior/MCP/reference)
- The book never shows how to ASSERT / self-check results, `print`, f-strings,
  or `.len()` — needed for any machine-proofable program. (Learned by probing;
  these are fundamentals-chapter material but the ownership chapter is
  self-described as a "full specification" yet omits the runnable scaffolding.)
- The book flags that `clone` does not parse in CALL-ARGUMENT position
  (concurrency example) but is SILENT that `clone` also fails in struct-literal
  FIELD position, RETURN position, and on field-access / by-value-param sources
  — i.e. the keyword's accepted positions are far narrower than documented.
- The book shows method bodies like `fn first(&self) -> &int` implying an
  `impl`/method context, but never shows the `impl Type { }` (inherent impl)
  syntax — and at HEAD `impl Type { }` does NOT parse, and trait-impl methods
  must be written `method name(...)` without `self`. The chapter gives no
  guidance on how to actually attach methods to a type.
- The book documents `Mutex`/`Atomic`/`Lazy` and explicit storage-class pins as
  v0.4 (correctly marked not-available) — no gap, just noted.

## book_wrong (book documents behavior the language does not do)
- "the `clone` keyword and `.clone()` method are equivalent" + "Clone is
  auto-derived for types whose fields are all Clone" (line 104): FALSE for user
  TypedObjects outside the single `let X = clone <direct-local>` position
  (details above). This is the primary BOOK-WRONG finding.
