# v0.3.3 Book-Acceptance Gate — Master Truth-Set (22-slice compilation)

Synthesized from **22 vertical slices** (author + adversarial verify each), run against
the shipped release binary at HEAD of the `strict-flip-collection-dispatch` worktree
(`target/release/shape`, ALREADY-BUILT — not rebuilt). Every program run memory-capped
(`ulimit -v 12582912`) + time-bounded (`timeout 30`) under both `--mode vm` and
`--mode jit`. Each slice ships a `small` (hand-readable) and a `large`
(~1000-LOC, machine-proofable) program.

**Gating rule:** A `FN-REG-CORRECTNESS`, `SCOPE-RECLAIM`, `BOOK-WRONG`, or `VM!=JIT`
finding is RELEASE-BLOCKING for the v0.3.3 tag. `BOOK-GAP` findings are
documentation-blocking (routed to the shape-web book owner), not language-blocking.

**Verdict tally:** 9 PASS · 8 PARTIAL · 5 FAIL (22 total). `small` and `large`
deliverables went green on every slice (by annotation-binding / workaround around the
defects where present). VM==JIT held byte-identical on every slice EXCEPT one f-string
cast divergence in `error-handling` (see VM!=JIT section). Release-blocking findings are
present across 7 slices, so the gate is **NO-GO**.

> NOTE: A slice's PASS/PARTIAL/FAIL label reflects only whether its deliverable programs
> went green. The gate counts **release-blocking findings**, not labels — slices whose
> programs went green only by working around a defect still gate the tag.

---

## Per-slice table

| slice | small | large | VM==JIT | verdict | blocking findings |
|-------|:-----:|:-----:|:-------:|:-------:|-------------------|
| variables        | ✓ | ✓ | ✓ | PASS    | — |
| types-primitive  | ✓ | ✓ | ✓ | PARTIAL | — |
| operators        | ✓ | ✓ | ✓ | PARTIAL | — |
| control-flow     | ✓ | ✓ | ✓ | PASS    | — |
| functions        | ✓ | ✓ | ✓ | PASS    | BOOK-WRONG: named arguments documented but non-functional |
| strings          | ✓ | ✓ | ✓ | PASS    | — |
| objects-arrays   | ✓ | ✓ | ✓ | FAIL    | BOOK-WRONG NumericVec methods missing; D2 map-element-type; D4 HashMap get→set int |
| enums            | ✓ | ✓ | ✓ | PASS    | — |
| traits           | ✓ | ✓ | ✓ | PASS    | — |
| generics         | ✓ | ✓ | ✓ | PARTIAL | — |
| pattern-matching | ✓ | ✓ | ✓ | PASS    | — |
| error-handling   | ✓ | ✓ | ✗ | FAIL    | `as int?` on Array<string> elem FIXED (c4 StringV2/DecimalV2 arms); `true as int` f-string VM!=JIT; BOOK-WRONG §Infallible |
| references       | ✓ | ✓ | ✓ | FAIL    | BOOK-WRONG: `var` shared-CoW aliasing does not exist (B0005 move-reject) |
| resource-mgmt    | ✓ | ✓ | ✓ | PARTIAL | — |
| modules          | ✓ | ✓ | ✓ | PARTIAL | — |
| datetime         | ✓ | ✓ | ✓ | FAIL    | string-method GetProp on Temporal in f-string SURFACE-errors; loop-local tz binding `unknown` |
| content          | ✓ | ✓ | ✓ | PASS    | — |
| comptime         | ✓ | ✓ | ✓ | PASS    | — |
| jit-compilation  | ✓ | ✓ | ✓ | PARTIAL | — |
| ownership        | ✓ | ✓ | ✓ | FAIL    | BW1 struct Copy table wrong (move-always); D1 missing FrameDescriptor; D2 struct-elem aliasing |
| collections      | ✓ | ✓ | ✓ | PARTIAL | SCOPE-RECLAIM: HashMap.get Option<V> match payload; HashMap.forEach value param `unknown` |
| math-core        | ✓ | ✓ | ✓ | PARTIAL | BOOK-WRONG: coefficient_of_variation mis-gated as v0.4 (works in v0.3.3) |

---

## RELEASE-BLOCKING findings

Each entry below blocks the v0.3.3 book-acceptance tag. Class in brackets.

### objects-arrays

1. **[BOOK-WRONG + FN-REG-CORRECTNESS]** NumericVec `cumsum()` / `diff()` / `abs()` /
   `dot(other)` / `norm()` / `normalize()` documented as provided
   (objects-arrays.mdx:273-276) but raise `no method <m> on receiver kind Ptr(TypedArray)`
   at runtime for `Vec<int>` AND `Vec<number>`, VM and JIT, even with explicit
   `Array<number>` annotation. Author originally claimed no book-wrong at HEAD;
   adversarial verify reversed that.
2. **[FN-REG-CORRECTNESS D2]** `roster.map(|e| e.salary)` element type not threaded
   downstream; `if v > mx` errors `Cannot infer types for binary operation Greater:
   operand types are unknown and unknown` unless the binding carries an explicit
   `Array<int>` annotation. Book Vector-Methods table promises the element type flows
   through the chain. Reproduced at HEAD (VM).
3. **[FN-REG-CORRECTNESS D4]** HashMap immutable accumulation
   `m = m.set(k, cur + e.salary)` (cur read back via `match m.get(...)`, e.salary a typed
   int field) raises `no method add on receiver kind Int64` on the 2nd iteration. Book's
   HashMap section documents exactly this get→Option→set update idiom. Reproduced VM and
   JIT (fallback, same error).

### error-handling

4. **[FN-REG-CORRECTNESS — FIXED 2026-06-22, strict-flip c4]** `(elem as int?)` on a valid
   integer string read from an `Array<string>` element previously ALWAYS returned `Err`.
   ROOT: `read_as_i64` / `read_as_f64` in `executor/builtins/type_ops.rs` had ZERO
   `NativeKind::StringV2` / `NativeKind::DecimalV2` arms — Array-element strings flow as the
   v2-raw `*const StringObj` carrier (kind=StringV2), distinct from the `Arc<String>`
   carrier (kind=String) used by literals / let-bound / f-string-reconstructed strings.
   FIX: added StringV2 + DecimalV2 arms that borrow the proven carrier's bytes/value and
   parse (no bit-reinterpret; the kind label drives the read). Verified VM+JIT:
   `probe_carrier` arrelem→123, `probe_arr_cast` arr0 ok 123 / arr1 err, `evidence_*` cast
   ok 77. 5 unit tests added (`read_as_{i64,f64}_from_{string,decimal}_v2*`).
5. **[VM!=JIT + BOOK-WRONG]** `f"{true as int}"` / `f"{someBool as int}"` render `1`
   under `--mode vm` but `true` under `--mode jit` — the JIT drops the bool→int infallible
   cast inside f-string interpolation. The book's §Infallible example is literally
   `true as int`. Bare `print(true as int)` and `let v:int = true as int` are correct in
   both modes; only the f-string-interpolated cast diverges. JIT compiled it (no fallback)
   and produced wrong output under the DEFAULT mode.

### references

6. **[BOOK-WRONG]** references-borrowing.mdx lines 18 + 69-87 document `var` as
   copy-on-write SHARED ownership (`let alias = data` references the same data;
   `data.push(4)` triggers CoW). ACTUAL: `let alias = data` is a compile-time B0005
   move-rejection (ec=1) under both VM and JIT — `data` is consumed and the later
   `data.push(4)` is rejected before execution. The book's own SURFACE note (lines 77-80)
   also misdescribes the failure as a JIT segfault / VM future-ptr auto-print, whereas it
   is a static SEMANTIC error. The entire `var` shared-ownership narrative is false as
   written.

### datetime

7. **[FN-REG-CORRECTNESS]** Calling a string-returning DateTime method
   (`format` / `iso8601` / `rfc2822` / `timezone` / `offset`) directly inside an f-string
   fails at RUNTIME under BOTH VM and JIT: `Not implemented: SURFACE: GetProp on
   Ptr(Temporal) not yet kinded ... (ADR-006 §2.7.24 Q25.B)`, ec=1. Repro:
   `let a = DateTime.parse("2024-06-15T12:00:00+00:00"); print(f"{a.format('%H:%M')}")`.
   `f"{a.year()}"` (int method) works — defect is string-returning GetProp on a Temporal
   receiver in an f-string. Breaks book VERBATIM runnable idioms at datetime.mdx:350 and
   :399. Not rescuable by `: DateTime` annotation. Author-missed; the large.shape stays
   green only by pre-binding every formatted string.
8. **[FN-REG-CORRECTNESS]** A loop-local `let x = cur.to_utc()` / `cur.to_timezone(...)`
   inside a while/for body is inferred `unknown`, so `x.format(...)` poisons
   `string + unknown` (`Cannot apply + to a string and a unknown`). Top-level intermediate
   `let` and direct method chains work; `: DateTime` annotation rescues the loop form.
   Matches the book's "Date Range Iteration" / "Formatting for Display" patterns; also
   surfaced as a BOOK-WRONG since the documented loop-local snippet only survives via
   f-string tolerance of the mis-inference.

### ownership

9. **[BOOK-WRONG BW1]** Copy-Semantics table (line 100) claims TypedObjects are "Copy if
   all fields are Copy (auto-derived)". The shipped binary MOVES all-Copy-field structs:
   `type Point{x:int,y:int}; let p=...; let q=p; print(p.x)` → error[B0005] cannot use
   this value after it was moved. The table row and the worked example (lines 119-136,
   which moves and never re-reads) are mutually inconsistent; the binary follows
   move-always. Either the table must change to "structs always move in v0.3.3" or the
   compiler must auto-derive struct Copy.
10. **[FN-REG-CORRECTNESS D1]** V2 typed opcodes (NewTypedArrayI64/TypedObject,
    TypedArrayPush*, GetElemI64, SetElemI64) emitted inside non-main functions and stdlib
    `Vec.slice` / `Vec.clone` monomorphizations have NO FrameDescriptor →
    `V2 bytecode verification failed` on stderr. Reproduced with a 6-line probe (3
    violations, output still correct, ec=0). large.shape trips 13 violations
    (enumerate_and_tally builds an int array in a non-main fn), so the "construct at
    top-level" workaround is only partially effective. Genuine codegen defect — the
    verifier is meant to gate emitted bytecode.
11. **[FN-REG-CORRECTNESS D2]** `let mut a = arr[0]` for a STRUCT element ALIASES the
    array's backing store: `arr=[Account{balance:100},...]; let mut a=arr[0];
    a.balance=999; print(arr[0].balance)` prints 999. Struct values are not copied on
    element read (scalar elements are correctly Copy). Silent wrong-result class.

### collections

12. **[SCOPE-RECLAIM]** HashMap.get() `Option<V>` value type does not propagate into the
    `Some(n)` match-binding payload: `match m.get(k){Some(n)=>n+1}` fails strict-typing
    with `operand types are unknown and int`. A plain user-fn `Option<int>` `Some(n)+1`
    works, so the defect is HashMap.get's `Option<V>` losing V. (hashmap.md Family F.)
13. **[SCOPE-RECLAIM, AUTHOR-MISSED]** HashMap.forEach closure value param loses V type:
    `m.forEach(|k,v|{ let w=v+1 ... })` and `m.forEach(|k,v|{ t=t+v })` both fail
    (`unknown and int` / `no method add on receiver kind Int64`). Documented method
    (objects-arrays.mdx:323). Inconsistent with HashMap.map/filter which DO infer V. VM==JIT
    both fail. (hashmap.md Family H.)

#### strict-flip runtime-carrier sweep (2026-06-22) — c3 + c7 SURFACE-AND-STOP

- **c3 HashMap `forEach`/`keys`/`values` on a typed-annotated map — SURFACED (not fixed).**
  `let m: HashMap<string,int> = HashMap(); m.forEach(...)` errors `no method forEach on
  receiver kind UInt64`. The diagnosis prescribed "stamp the binding-load kind as
  `Ptr(HeapKind::HashMap)` instead of `UInt64`". REFUSED: the typed-map carrier is a raw
  `*mut TypedMapStringI64` (a distinct `repr(C)` struct allocated by `NewTypedMap*`, kind
  intentionally `UInt64` per `v2_handlers/typed_map.rs:56`), NOT an `Arc<HeapValue::HashMap>`.
  Stamping `Ptr(HeapKind::HashMap)` would route to HASHMAP_METHODS, which would deref the
  `*mut TypedMap` as a HashMap HeapValue — a wrong-type bit-reinterpret (5-arm
  receiver-recovery soundness rule, CLAUDE.md Forbidden). The typed-map fast path only
  emits dedicated stack opcodes for `set`/`get`/`has`/`delete`/`len`/`isEmpty`
  (`compiler/expressions/function_calls.rs::try_compile_typed_map_method`); `forEach`/`keys`/
  `values`/`map`/`filter` return `Ok(None)` and fall through to generic CallMethod, which
  sees `UInt64`. The sound fix is dedicated typed-map iteration opcodes per K/V kind (6
  variants × 3 methods) + a `TypedMap` entries iterator + forEach closure-invocation
  plumbing — a multi-table opcode workstream needing the verify-merge lockstep gate, NOT a
  binding-load kind-stamp. ALSO: the real `Arc<HeapValue::HashMap>` carrier path (untyped
  `let m = HashMap()`) is ITSELF broken at this HEAD — `set` doesn't persist (`get`→null,
  `len`→0), and `keys` is a separate V3-S5 ckpt-5 `TypedArrayData`-deletion SURFACE. c3 is
  blocked on the V3-S5 ckpt-6 close, consistent with the existing collections gate note
  (`runnable=false, V3-S5 ckpt-6`).

- **c7 NumericVec `cumsum`/`diff`/`abs`/`dot`/`norm`/`normalize` — SURFACED (not fixed).**
  The diagnosis prescribed "RESTORE element-kind routing in
  `objects/mod.rs::typed_array_method_registry` (F64→FLOAT_ARRAY_METHODS)". The routing flip
  is necessary but INSUFFICIENT and actively HARMFUL: (1) EVERY handler in
  `executor/objects/typed_array_methods.rs` (the FLOAT_ARRAY_METHODS / INT_ARRAY_METHODS
  targets) is an `Err(ckpt3_surface(...))` STUB pending the V3-S5 ckpt-6 v2-raw
  `TypedArray<T>` per-T carrier migration (~40 entry points) — so `cumsum`/`dot`/`norm`/etc.
  produce a SURFACE error even after routing, never the documented result; (2) worse,
  FLOAT_ARRAY_METHODS's `sum`/`avg`/`min`/`max` are ALSO ckpt3 stubs, while the WORKING
  implementations are the kind-generic `array_aggregation::handle_{sum,avg,min,max}_v2` in
  ARRAY_METHODS — routing F64 to FLOAT_ARRAY_METHODS REGRESSED `[1.0,2.0,3.0].sum()` /
  `.avg()` into the stub (measured). REVERTED the routing flip; left the F64/I64 arms at
  `None` (→ working ARRAY_METHODS) with a SURFACE-AND-STOP comment. The numeric-transform
  surface is genuinely blocked on V3-S5 ckpt-6, not a dispatch-table routing fix.

### functions

14. **[BOOK-WRONG]** Named arguments are documented as a supported call shape
    (fundamentals/functions.mdx: all-named `f(a:1,b:2,c:3)`, positional-then-named
    `f(1,b:2,c:3)`) but are fully non-functional. No-default fns: all-named →
    error[SEMANTIC] "expects between 3 and 3 arguments, got 0"; positional-then-named →
    "got 1". defaults+named is now an EXPLICIT compile error "Named call arguments are not
    supported on functions ... Pass arguments positionally" (a correctness improvement over
    the earlier silent-wrong behavior), but the "supported call shapes" claim remains
    wrong. Failure at compile/lowering; VM and JIT identical.

### math-core

15. **[BOOK-WRONG]** `coefficient_of_variation` is documented under core/math.mdx's
    v0.4-preview :::caution block ("not available in v0.3.3") and tagged _(v0.4)_ in the
    reference table (line 177), but the shipped v0.3.3 binary computes it correctly:
    `coefficient_of_variation([2.0,4.0,6.0]) = 0.408248290463863` (== std/mean) under VM and
    JIT. Under-claim (conservative direction, no runtime break for a book-follower), but the
    book misdocuments v0.3.3 behavior. Fix: move it into the working set or actually gate it.

---

## VM!=JIT divergences

One slice exhibited a VM/JIT behavioral divergence:

- **error-handling** — `f"{true as int}"` / `f"{someBool as int}"`: `1` under
  `--mode vm`, `true` under `--mode jit` (default mode). The JIT compiled the f-string
  interpolation and dropped the bool→int infallible cast (no fallback). Bare
  `print(true as int)` and `let v:int = true as int` agree in both modes. (Same finding
  as release-blocking item 5.)

> All other slices were byte-identical VM vs JIT on both `small` and `large`. The
> objects-arrays D4, datetime f-string SURFACE, and collections findings fail
> *identically* under both modes (JIT falls back to the interpreter) — same wrong
> result, not a divergence.

---

## BOOK-GAP findings (route to shape-web book owner)

Documentation gaps — non-language-blocking, but route to the book owner before the
book-acceptance tag.

### Cross-cutting (recurs on most slices)

- **No assert / self-check primitive in the prelude.** `assert(...)` →
  `Undefined function: assert. Did you mean 'sqrt'?`. Surfaced by variables,
  types-primitive, control-flow, strings, objects-arrays, traits, modules. Self-checks must
  hand-roll a fail-counter or import `std::core::utils::testing`.
- **Imported testing helpers are top-level-only.** `assert` from
  `std::core::utils::testing` is NOT visible inside an `fn` body
  (`Undefined function: 'assert'`); it resolves only at module top level. (strings)
- **Empty-array element-type inference.** `let mut a = []` / `let x: Array<T> = []`
  built up by `.push`/concat needs an explicit annotation, and nested empty arrays
  (`let mut a: Vec<Vec<int>> = []`) are REJECTED even WITH the annotation (D1). Empty `[]`
  is a hard error even annotated (V3-S5 WIP surface-stop). Surfaced by types-primitive,
  control-flow, objects-arrays, modules.
- **A local variable named `data` cannot be index-accessed.** `let data: Array<int> = [...];
  data[0]` → `data[...] requires explicit data binding ... Set a DataSchema`; identical code
  named `nums`/`buf` → ec=0. The reserved-name interaction is documented to exist but the
  error never mentions it, and the book uses `data` as an array name in multiple examples.
  (variables, references)
- **Dispatch-result inference loss.** `.len()` / index / `.map(...)` / call-result element
  types frequently infer `unknown` when fed directly into a comparison or arithmetic;
  fixed by an explicit `let n: int = ...` annotation or annotated param. Surfaced by
  variables, objects-arrays, resource-mgmt, modules, references, datetime.

### Per-slice

- **types-primitive:** `[]` infers as Vec<T> per Notes but deferred-push / nested empty
  needs annotation (D1 rejects nested even annotated); bitwise operators (`& | ^ << >>`)
  on int undocumented (they work); `-2 as u8` parses as `-(2 as u8) = -2` (unary minus
  binds looser than `as`) — book worked-examples sidestep without flagging.
- **control-flow:** `[0].filled(n)` does not exist; `"256".chars()` does not exist;
  empty-array element-type loss on `pop()`.
- **functions:** `.reduce(f, init)` (callback FIRST) never taught; `.len()`/`.push()` on
  Vec not covered; multi-arg lambda inference limit applies to user-HOF params (undocumented);
  returned-closure inference asymmetry (`clamper{|x| if x<lo{..}}` fails) has no stated rule;
  benign "V2 bytecode verification failed" stderr warning on closures in `.map`/`.filter`
  undocumented.
- **strings:** `length` (property) vs `len()` inconsistency in-chapter — `llm_summary` uses
  `s.len()` but the working form is the `.length` property.
- **objects-arrays:** Option<V> unwrap/match not shown; NumericVec return types (int vs
  number) unstated; `as` casts not mentioned in-chapter.
- **traits / generics:** mixed enum-variant array literals need `Array<T>` annotation;
  heterogeneous-concrete dyn arrays (`[Dog,Cat]`) fail ("not compatible"); dyn-Trait method
  result erases to `unknown` and must be re-bound through an explicit type annotation before
  `+`/arithmetic; the book's "hetero Vec<dyn Trait> not yet end-to-end" framing
  under-promises — 3-distinct-type hetero Vec actually runs when the result is consumed via
  f-string/typed-let. `-> ()` unit return does not type-check.
- **pattern-matching:** struct / array / nested-constructor patterns WORK but are entirely
  undocumented; tuple patterns `(a,b)` PARSE-ERROR with no stated unsupport; exhaustiveness
  section has no example diagnostic; union type-pattern arm (`n: int =>`) syntax unconnected
  to guard subjects.
- **error-handling:** §Uncaught Exception Display format is NOT what the runtime emits
  (actual `Error: Runtime error: Uncaught error: <msg>` + `Caused by: ... (line N)`, no code
  tag, no frame trace); §Inference Rules `Could not infer generic type arguments for 'Result'`
  is NOT reported for an unused `Err(...)` binding (let-generalization absorbs it).
- **resource-mgmt:** comment syntax (`//`) never shown (`#` is a parse error); inherent
  methods on user types (`impl T { method foo() }`) is a parse error — methods must route
  through a trait impl, unstated; collection accumulation API (.push/.len/clear) never shown.
- **modules:** array-literal element-type inference fails for struct/non-literal elements even
  with annotation (inline literals DO infer); iterating a call result loses element type;
  empty `[]` hard error even annotated; closure param annotation `|x: T|` does not parse as a
  method-call argument; prelude globals silently shadow bare member imports (`variance`).
- **datetime:** Duration display format undocumented (actual ISO-8601 `PT432000S`); no
  documented accessor on a Duration value (no `.days()`/`.seconds()`) — forces
  `unix_timestamp()` subtraction.
- **content:** `Border.none` exact layout must be inferred; chart `.x_label`/`.y_label` text
  not in terminal `.toString()` rendering; `.toString()` terminal-only not stated (ContentFor
  v0.4-fenced).
- **comptime:** `println` undefined (prelude name is `print`); `comptime for i in 0..3` is a
  hard parse error (likely parser bug); `comptime for` over an array literal emits a benign V2
  verifier warning; `build_config().debug` bool loses its type inside a `comptime if`
  (`unknown is not compatible with bool`).
- **jit-compilation:** chapter teaches ZERO writable Shape surface syntax (reader must leave
  for fundamentals); no documented way to OBSERVE tier-up from the CLI; typed-array kernels
  trip the V2 verifier and fall through to the interpreter under `--mode jit` (result still
  correct, undocumented); supported-ops list omits casts (`as number` → ConvertToNumber
  rejected by JIT preflight); `--mode vm` also emits the V2 warning on stderr for typed-array
  programs; source-construct → fall-through mapping undocumented.
- **collections:** Option<V> payload type-flow under `match` undocumented; HashMap
  iteration over keys/values/entries is not runnable (documented runnable=false, V3-S5
  ckpt-6) so no runnable frequency-table pattern is shown.
- **math-core:** variance/std do not state POPULATION vs SAMPLE (binary uses population /
  divide-by-N).
- **ownership:** array-element value category unspecified (`let a = arr[i]` of a
  Copy-eligible struct — copies/moves/borrows undefined; it aliases, see D2); `clone`/`move`
  keyword is statement/binding-position-only, never stated (clone-in-call-arg parse-errors).
- **operators:** (no gaps recorded).

---

## Go / No-Go

**NO-GO.**

15 release-blocking findings across 7 slices (objects-arrays, error-handling, references,
datetime, ownership, collections, math-core, functions): 8 FN-REG-CORRECTNESS,
2 SCOPE-RECLAIM, 5 BOOK-WRONG, and 1 VM!=JIT divergence (the `true as int` f-string cast,
which is also one of the BOOK-WRONG items). The gate requires ZERO release-blocking
findings; the present count is non-zero, so the v0.3.3 book-acceptance tag is **NO-GO**.
