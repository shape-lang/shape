# v0.3.3 Book-Acceptance Gate — Master Truth-Set (22-slice compilation)

Synthesized from **22 vertical slices** (author + adversarial verify each), run against
the shipped release binary at HEAD of the `strict-flip-collection-dispatch` worktree
(`target/release/shape`, ALREADY-BUILT — not rebuilt). Every program run memory-capped
(`ulimit -v 12582912`) + time-bounded (`timeout 30`) under both `--mode vm` and
`--mode jit`. Each slice ships a `small` (hand-readable) and a `large` (~1000-LOC,
machine-proofable, `ALL_CHECKS_PASSED` sentinel) program.

**Gating rule (PLAN.md §Gating):** A `FN-REG-CORRECTNESS`, `SCOPE-RECLAIM`,
`BOOK-WRONG`, or `VM!=JIT` finding is RELEASE-BLOCKING for the v0.3.3 tag.
`BOOK-GAP` findings are documentation-blocking (routed to the shape-web book owner),
not language-blocking. `ANTI-TAUTOLOGY` (green proof against the defect's own wrong
output) is a FAIL-class finding.

**Verdict tally:** 4 PASS · 12 PARTIAL · 6 FAIL (22 total). VM==JIT held on every
slice (`small` and `large` both consistent). Release-blocking findings are present, so
the gate is **NO-GO**.

> NOTE: `objects-arrays` carries a slice-level verdict label of "PASS" but ships 4
> independent release-blocking `FN-REG-CORRECTNESS` findings (D1–D4). The blocking
> findings gate the tag regardless of the slice's own PASS/PARTIAL/FAIL label; that
> label reflects only whether the slice's deliverable programs went green (they did, by
> annotation-binding around the defects). The gate counts blocking findings, not labels.

---

## Per-slice table

| slice | small | large | VM==JIT | verdict | blocking findings |
|-------|-------|-------|---------|---------|--------------------|
| variables | ✅ | ✅ | ✅ | **FAIL** | ~~var CoW aliasing (BOOK-WRONG)~~ FIXED (S1b 2026-06-21: `var copy = data` still-live heap rebind now DEEP-clones — `OwnershipDecision::DeepClone` → `LoadLocalDeepClone`/`DeepCloneTop`; mutating the copy no longer touches the source); ~~generic-struct construction SEGFAULT (ec=139)~~ FIXED (S2, WS-6b base-name re-stamp downgraded the monomorphized schema → FIELD_TAG_OBJECT deref of an inline scalar; `ws6b_name_would_downgrade` guard) |
| types-primitive | ✅ | ✅ | ✅ | PARTIAL | — (gaps only) |
| operators | ✅ | ✅ | ✅ | **FAIL** | object-`+`-merge hijacked by in-scope impl Add; `+=`/`acc=acc+a` on user type rejected; left struct-literal operand of Sub/Mul rejected; decl-order-dependent operator-trait resolution; ANTI-TAUTOLOGY (large asserts buggy output) |
| control-flow | ✅ | ✅ | ✅ | **FAIL** | `while`/`if`-prefixed identifier in a condition derails parser (E0001) |
| functions | ✅ | ✅ | ✅ | FIXED (S4) | named-arg call on a free function now a CLEAN COMPILE ERROR (was silent-discard → wrong result); positional calls unchanged |
| strings | ✅ | ✅ | ✅ | PASS | — |
| objects-arrays | ✅ | ✅ | ✅ | FIXED (S4) | D1 `Array<int>.sum/min/max`→`int` (per-receiver-element); D2 `.map` elem-type OK; D3 empty `HashMap()` len/isEmpty OK (TypedMapLenStack opcode); D4 `?? 0` keeps `int` kind in loop (no `no method add` crash) |
| enums | ✅ | ✅ | ✅ | PARTIAL | `(number)->bool` param annotation fails to parse (BOOK-WRONG); `s.to_int()` fictional (BOOK-WRONG) |
| traits | ✅ | ✅ | ✅ | FIXED (S4) | trait/extend declared return type now propagates to an un-annotated call-site binding (`p.sum()->int` binding tracks `int`; `a+a`→28 not 28.0) |
| generics | ✅ | ✅ | ✅ | PARTIAL | — (gaps only) |
| pattern-matching | ✅ | ✅ | ✅ | PARTIAL | D2 FIXED (S3 2026-06-21: `Some(Enum::Variant(..))` now unwraps the W14 OptionData carrier before the inner variant check); D1 union type-pattern binder typed `unknown` (SCOPE-RECLAIM, v0.4) |
| error-handling | ✅ | ✅ | ✅ | PARTIAL | `(arr_elem as int?)` rejects `Array<string>` element while literal `"42" as int?` Ok (carrier-kind) |
| references | ✅ | ✅ | ✅ | PARTIAL | stored-ref index/method documented as compile-error but actually works (BOOK-WRONG, conservative direction) |
| resource-mgmt | ✅ | ✅ | ✅ | PARTIAL | — (gaps only) |
| modules | ✅ | ✅ | ✅ | PASS (S4 re-verified) | `from std::core::math use { mean as avg }; avg([..])` binds the alias and runs (→2.0); the prior named-alias FAIL no longer reproduces |
| datetime | ✅ | ✅ | ✅ | PARTIAL | — (gaps only) |
| content | ✅ | ✅ | ✅ | PASS | — |
| comptime | ✅ | ✅ | ✅ | PARTIAL | comptime `false` baked/displayed as `null` (BOOK-WRONG + FN-REG); same hits `build_config().debug` |
| jit-compilation | ✅ | ✅ | ✅ | PARTIAL | — (gaps only) |
| ownership | ✅ | ✅ | ✅ | **FAIL** | BW-1 `var copy = data` auto-clone RESOLVED 2026-06-21 (let moves / var auto-clones); var-copy mutate-INDEPENDENCE RESOLVED (S1b 2026-06-21: auto-clone now DEEP, not a refcount alias); D2 reading non-Copy struct elem out of array aliases backing store |
| collections | ✅ | ✅ | ✅ | PARTIAL | — (gaps only) |
| math-core | ✅ | ✅ | ✅ | PASS | — |

`*objects-arrays` verdict label is PASS but ships 4 release-blocking findings — see note above.

---

## RELEASE-BLOCKING findings (FN-REG-CORRECTNESS / SCOPE-RECLAIM / BOOK-WRONG / VM!=JIT / ANTI-TAUTOLOGY)

These gate the v0.3.3 tag. None are VM!=JIT (every defect reproduces identically on both
modes). Grouped by slice.

### variables
- **BOOK-WRONG** — `var shared=[1,2,3]; let alias=shared; shared.push(4)` documented as
  copy-on-write aliasing ("Both reference the same data" / "Copy-on-write: clones if
  aliased"). Binary REJECTS at compile time: `error[SEMANTIC] [B0005] cannot use this
  value after it was moved` (VM + JIT). The documented CoW contract AND the stale
  segfault/dev-print `:::note`/SURFACE caveat are both contradicted by the clean
  compile-error.
- **SEGFAULT / FN-REG-CORRECTNESS** — constructing the book's `type Box<T = int> {
  value: T }` via `Box { value: 9 }` SEGFAULTs (ec=139) on BOTH VM and JIT.
  `let b: Box<int> = Box { value: 9 }` errors "Box is not compatible with Box<int>";
  `Box<int> { value: 9 }` errors "Undefined variable 'int'". A memory-safety crash on a
  documented generic type form.

### operators
- **FN-REG-CORRECTNESS / BOOK-WRONG** — object-literal merge `+` is HIJACKED into a
  structurally-matching in-scope `impl Add for Vec2{x,y}`. `{x:1,y:2} + {y:20,z:30}`
  yields positional add x=21,y=32 (z absent) on BOTH VM and JIT, not book-correct merge
  `{x:1,y:20,z:30}` (operators.mdx 481-489).
- **FN-REG-CORRECTNESS** — `acc = acc + a` AND `acc += a` on a `mut` user-type fail to
  compile ("Vec2 + Vec2 is not compatible with Vec2") though `let c = a + b` compiles;
  breaks the line-52-53 `+=`-desugars-to-Add contract.
- **FN-REG-CORRECTNESS** — inline struct-literal as the LEFT operand of Sub/Mul rejected
  ("Both operands must be numeric") while the variable-bound form works.
- **FN-REG-CORRECTNESS** — operator-trait resolution is declaration-order dependent;
  `fn main` before `impl BitAnd for Flags` fails, impls-first compiles. Contradicts the
  documented two-pass register-then-compile model.
- **ANTI-TAUTOLOGY** — `large.shape` §T asserts `merge_hijack_x==21` / `_y==32` (the
  BUGGY output) so it prints ALL_CHECKS_PASSED against known-wrong expected values. A
  green machine-proof against the defect's own output — FAIL-class.

### control-flow
- **FN-REG-CORRECTNESS (parser/lexer)** — an identifier beginning with keyword `while`
  or `if` (`whileX`, `whileCount`, `ifX`) used in a CONDITION position fails
  maximal-munch and derails parsing → `error[E0001]` "expected a block `{ ... }`".
  Reproduced on BOTH vm and jit (parse-stage, mode-independent). FINE: `for forX in
  0..3`, `if forCount > 1`, `let z = whileCount + 1`. Natural names like
  `whileCount`/`ifCount` in their own loop/branch hit it.

### functions
- **BOOK-WRONG + silent-wrong-output** — named-argument calls on DEFAULT-valued
  functions silently compile and discard the named values, returning a wrong result.
  VM + JIT: `sma(20, threshold:0.02)` → 0.2; `box_volume(w:2,h:3,d:4)` → 1.
  functions.mdx 203-209 asserts named calls "fail to type-check" — false for the
  default-valued case; the language silently miscomputes.

### objects-arrays (4 findings — slice label PASS, findings release-blocking)
- **D1 FN-REG-CORRECTNESS / BOOK-WRONG** — `Vec<int>.sum()/min()/max()` return
  `number`, not `int`; `let s: int = v.sum()` is a compile error (ec=1). Book NumericVec
  advertises these on `Vec<int>` without stating the narrowing.
- **D2 FN-REG-CORRECTNESS** — `.map()` element type lost when the result feeds
  `for`+comparison (`for v in salaries { if v > mx }` → "Greater operands are
  unknown/unknown"). Matches the typed-closure-inference regression cluster.
- **D3 FN-REG-CORRECTNESS** — never-populated empty `HashMap()` materializes as
  `UInt64`; `m.isEmpty()`/`m.len()` raise runtime "no method … on receiver kind
  UInt64".
- **D4 FN-REG-CORRECTNESS** — `<hashmap-readback-int> + <typed-object-field-int>` raises
  runtime "no method add on receiver kind Int64" on the second loop iteration; `cur +
  <literal>` works.

### enums
- **BOOK-WRONG (parser)** — function-type param annotation `(number) -> bool` fails to
  parse (E0001 "unexpected }"); the `find_first(v, predicate: (number)->bool)` Option
  example is not writable as documented.
- **BOOK-WRONG** — `s.to_int()` in the `parse_port` Result/`?` example does not exist
  ("Method to_int not found on type string"); no alternative (`parse_int`, `"8080" as
  int`) works. The documented Result<int,string> + `?` example cannot run as written.

### traits
- **FN-REG-CORRECTNESS** — trait/extend declared return type not propagated to an
  un-annotated call site. VM + JIT: (1) `print("x=" + u.display())` with `display() ->
  string` → "Cannot apply '+' to a 'string' and a 'unknown'"; (2) SILENT-WRONG-OUTPUT —
  `extend Point { method sum() -> int }; let a = p.sum(); print(a + a)` → `14.0` not
  `14`. Slice programs mask it by annotation-binding every method result.

### pattern-matching
- **D2 FN-REG-CORRECTNESS — FIXED (S3, 2026-06-21)** — nested constructor patterns
  silently mis-matched and bound garbage. `match Some(Status::Done(42))` against
  `[Some(Status::Active)=>…, Some(Status::Done(n))=>…]` selected the Active arm.
  Root cause: `Some(x)` is the canonical W14 `Arc<OptionData>` carrier (`SomeCtor`),
  but the pattern-CHECK path's `Some` arm did only `IsNull` then recursed the inner
  pattern against the *wrapper* local — so the inner enum check read field 0 of
  `OptionData` as if it were the payload's `__variant`. Fix (`patterns/checking.rs`
  `Some` arm): unwrap via `UnwrapOption` into a fresh local + `stamp_unwrapped_payload_local`
  before recursing — mirrors the `Result` check arm and the binding path. Verified
  (both modes, capped): nested-tuple-payload, nested-unit-variant, None, 3-level nest,
  struct-variant nested, guard-on-nested-binder. Regression test
  `compiler::patterns::checking::nested_constructor_pattern_tests` (7 cases).
- **D1 SCOPE-RECLAIM (dated 2026-05-21)** — a type pattern (`n: int`) on a union
  scrutinee (`int|string`) binds the payload as `unknown`; any guard/arithmetic on the
  binder is a compile error. Plain non-union scrutinee + guard works.

### error-handling
- **FN-REG-CORRECTNESS (Wave-1 collection-dispatch cluster)** — `(tok as int?)` REJECTS
  a `string` read from an `Array<string>` element (returns Err) while the byte-identical
  literal `("42" as int?)` returns Ok. Carrier-kind defect; the flagged Wave-1 root for
  this strict-flip-collection-dispatch worktree.

### references
- **BOOK-WRONG (conservative/under-promise, low severity)** — references-borrowing.mdx
  225-227 state stored-reference index/method (`let r = &nums; r[0]`, `r.len()`) is a
  v0.3.3 COMPILE ERROR; they actually WORK (prints 1 / 3, ec=0, VM + JIT). Mis-states a
  falsifiable compile-error contract. Doc-only fix, but a BOOK-WRONG.

### modules
- **BOOK-WRONG + named-alias defect** — modules.mdx:42-44 `from math::stats use { mean
  as avg }` is runnable=true but FAILS under BOTH modes with "Undefined function: avg".
  Named-alias import does NOT bind the alias when the target is a STDLIB module function;
  `use { mean }` (no alias) works, and the same alias form on a FILESYSTEM module works.

### comptime
- **BOOK-WRONG + FN-REG-CORRECTNESS** — a comptime block evaluating to boolean `false`
  is displayed as `null` at the print/f-string/literal-bake boundary (VM == JIT).
  `comptime { false }`, `comptime { 3 > 5 }`, `comptime { 2+2==5 }` → `null`;
  `comptime { true }` → `true`. The VALUE is sound; only the literal-bake/display path
  corrupts false→null. Same defect hits `build_config().debug` → `null` (documented LIVE
  in v0.3.3).

### ownership
- **BW-1 RESOLVED (2026-06-21 binding-move reconcile)** — ownership.mdx §Smart Move/Clone
  (79-86) and line 43 document `var copy = data` as auto-clone-on-liveness. The binding-
  move flip (commit 5ba11fe9) had made the move apply to BOTH `let` AND `var`, so `var
  copy = ds; …use ds…` was rejected B0005. FIXED: the move/clone policy is now gated on the
  DESTINATION binding kind — `let` / `let mut` MOVE (B0005 on use-after-move), `var` AUTO-
  CLONES on a still-live source (clone-on-still-live / CoW). `var copy = data; print(data);
  print(copy)` keeps BOTH (VM + JIT, runs clean). See `MirFunction.var_binding_slots` +
  `solver::compute_ownership_decisions` `dest_is_var` gate; shape-tests in
  `tools/shape-test/tests/borrow_refs/move_semantics.rs` (`var_*`). NOTE: the `var` clone is
  a refcount-share, so a subsequent in-place `.push` on the copy still aliases the source
  buffer (no copy-on-write-on-mutation) — distinct, pre-existing array-mutation CoW concern,
  out of scope for the binding-kind gating.
- **D2 FN-REG-CORRECTNESS** — reading a non-Copy (all-Copy-fields, so book-Copy) struct
  element out of an array aliases the backing store: `let mut x = arr[0]; x.balance=999`
  makes `arr[0].balance==999`. Contradicts the single-owner / no-aliased-mutation model.

---

## BOOK-GAP findings (route to shape-web book owner — documentation-blocking, not language-blocking)

Recurring cross-slice gaps (consolidate at the book level):

- **No `assert`/self-check primitive is documented** anywhere (variables, types-primitive,
  control-flow, functions, objects-arrays, enums, traits, modules, comptime, pattern-matching).
  `assert(...)` is not a builtin ("Undefined function: assert. Did you mean sqrt?").
  Every machine-proofable program hand-rolls an `if expected != got { print(...) }`
  checker. Document the conditional-print self-check idiom (or ship an `assert`).
- **Strict-typing annotation requirement on dispatch/empty results is undocumented**
  (types-primitive, control-flow, objects-arrays, traits, resource-mgmt, collections,
  modules, math-core). Results of `.map`/`.filter`/`.sum`/`.pop`/indexing feeding a
  comparison/arithmetic context infer `unknown`/`object` and need an explicit `let x:
  T`/`Array<T>` annotation the chapters never mention. Empty literals (`[]`, `HashMap()`)
  need a binding annotation to pin type args.
- **`V2 … has no FrameDescriptor` / `[jit-fallback]` stderr noise** on idiomatic
  Vec/HashMap/typed-array construction (types-primitive, functions, collections,
  ownership, jit-compilation, math-core). Non-fatal, stdout correct, VM==JIT, but
  undocumented and alarming. Book should note it is benign (or the warning be
  suppressed) — tracked internally (v0.3-r8w6 / W19 / V3-S5), v0.4.

Per-slice book gaps:

- **variables** — generic-struct CONSTRUCTION never demonstrated and natural forms break;
  no guidance on recovering element type after `.pop()`/index on a typed array.
- **types-primitive** — `none` scalar has no usable value form (only `None` works);
  array TYPE annotations (`Vec<int>`/`Array<int>`) undocumented (`[int]` parses as a
  1-tuple); `.length`/`.push`/indexing on `Vec<T>` not introduced.
- **control-flow** — `[0].filled(n)` doesn't exist; empty-typed-array idiom loses element
  type; `"256".chars()` doesn't exist (string iteration absent).
- **functions** — `.reduce`/fold not covered (signature is callback-first); `.len()` on
  Vec never shown; closure-inference asymmetry (param-typed Vec vs let-bound Vec) is
  undocumented and breaks an identical pipeline.
- **strings** — `.length` unit (Unicode scalar values) unspecified (`👨‍👩‍👧`.length==5);
  split()-element typing under-specified for typed sinks.
- **objects-arrays** — NumericVec return types unstated; empty-array / empty-HashMap
  binding-annotation patterns undocumented; `get` returns `Option<V>` but unwrap/match
  never shown; `as` casts absent though NumericVec forces int/number bridging.
- **enums** — Option example signature uses `Vec<number>` but `Vec` is never defined
  (actual is `Array<T>`).
- **traits** — method results need explicit annotation to retain declared return type;
  `Vec<Struct>` doesn't coerce to `Vec<dyn Trait>` param; mixed enum-variant array
  literals require explicit `Array<T>`.
- **generics** — supertrait-method dispatch through a `dyn SubTrait` value is
  book-silent and fails; user-defined HOF taking a 2-arg/numeric closure param is
  inexpressible and book-silent; free-fn vs trait-method name collision book-silent.
- **pattern-matching** — nested constructor patterns undocumented (and mis-behave);
  struct-variant destructuring undocumented; tuple patterns/returns book-silent and
  fail to parse; string char-walking lexing needs unavailable `.chars()`.
- **error-handling** — string tokenization/char-index/`.chars()` API absent though the
  chapter's own examples parse text; `!!`-wrapped error renders a struct dump not its
  `message`; uncaught-`?` exit code is 0 (contract unspecified).
- **resource-mgmt** — comment syntax (`//`) never shown; inherent `impl T { method }`
  is a parse error (methods must go via a trait impl, unstated); `.clear()`/`LOG=[]`
  unsupported.
- **modules** — `Array.sum()`→`number` vs `v.length`→`int` re-annotation walls; empty-
  collection edge cases (divide-by-`v.length`, empty `filter`→`map`→`sum` crash)
  undocumented; annotation-import / namespace-qualified-annotation runnable but
  un-exercised.
- **datetime** — `DateTime` is not array-storable under strict typing (hard compile
  error even with `Array<DateTime>` annotation), undocumented; Duration string form is
  ISO-8601 `PT<seconds>S` (seconds, not the prose's "ms"), never stated.
- **content** — asserting a rendered string needs an explicit `: string` annotation; no
  render/inspection entry point named for non-string assertions.
- **comptime** — `error(msg)` message text dropped from the diagnostic; available
  comptime stdlib string-method surface uninventoried; explicit `return false` / loop
  `return` inside a comptime fn misbehaves.
- **jit-compilation** — chapter silent on all writable Shape syntax and on how to OBSERVE
  tier-up from the CLI; typed-array "single movsd" path trips the V2 verifier and falls
  to interpreter (unwarned); `… as number` cast rejected by JIT preflight.
- **collections** — arithmetic directly on a match-bound `Option` payload
  (`Some(x) => x+100`) fails inference, undocumented.
- **math-core** — variance/std POPULATION (÷N) vs SAMPLE (÷N-1) convention never stated;
  building a dataset array inside a function (`let mut a: Array<number> = []; a.push`)
  trips `NewTypedArrayF64` FrameDescriptor defect.

---

## Go / No-Go

**Decision: NO-GO.**

The gate clears to GO only with ZERO release-blocking findings across all 22 slices.
The compilation surfaces release-blocking findings in **15 of 22 slices** (every slice
except types-primitive, strings, generics, resource-mgmt, datetime, content,
jit-compilation, math-core — and of those, the first seven carry book-gaps and
math-core/strings/content are clean PASS). VM==JIT held on every slice, so there is no
divergence-class blocker; the blockers are correctness/parser/book-wrong defects.

Release-blocking summary (by class):
- **SEGFAULT (memory safety):** variables — generic-struct construction `Box { value: 9
  }` (ec=139, VM + JIT).
- **Parser/lexer FN-REG-CORRECTNESS:** control-flow (`while`/`if`-prefixed identifier in
  a condition); enums (`(number)->bool` param annotation).
- **Silent-wrong-output:** functions (named-args on default fn discard values);
  operators (object-`+`-merge hijacked); traits (declared return type not propagated);
  pattern-matching D2 (nested constructor patterns mis-match); ownership D2 (array
  element aliases backing store); comptime (`false`→`null`).
- **SCOPE-RECLAIM:** pattern-matching D1 (union type-pattern binder `unknown`).
- **Carrier-kind FN-REG-CORRECTNESS (Wave-1 collection-dispatch):** error-handling
  (`arr_elem as int?` rejects array `string`); objects-arrays D1–D4.
- **BOOK-WRONG (language contradicts shipped binary):** variables (var CoW); operators
  (4); functions; enums (2); modules (named-alias); comptime; ownership (BW-1);
  references (conservative direction); objects-arrays D1.
- **ANTI-TAUTOLOGY:** operators `large.shape` asserts the defect's own buggy output.

All release-blocking findings must be resolved (or explicitly user-dispositioned out of
the v0.3.3 release-blocking set) before the tag.
