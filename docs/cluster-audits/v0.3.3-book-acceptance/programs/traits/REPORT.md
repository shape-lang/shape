# Book-Acceptance Report — Slice: `traits`

Book chapter (PRIMARY source):
`shape-web/book/book-site/src/content/docs/fundamentals/traits.mdx`

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`
(strict-flip-collection-dispatch worktree, release build at HEAD).
Harness (machine-safety capped):
`out=$(bash -c 'ulimit -v 12582912; timeout 30 $BIN run --mode $MODE $FILE' 2>&1); ec=$?`
Modes: `vm` (interpreter) and `jit` (tiered; falls back to interpreter on a
V2-verification SURFACE — see below).

`vm_jit_byte_identical` is computed on **stdout only**. The runtime emits a V2
bytecode-verification diagnostic and (under jit) a `[jit-fallback]` line to
**stderr**; stdout is byte-identical between modes for both programs.

Determinism strategy for this slice: pure (impl, supertraits, dyn). No clocks,
no randomness, no I/O beyond `print`; all fixtures are literals.

Re-verified 2026-06-20 at worktree HEAD: `large.shape` PASS (vm+jit, ec=0,
byte-identical, 3/3 stable, ~70 machine-checked assertions). `small.shape`
required an AUTHOR-ERROR correction on re-verification (an earlier draft used
`+` concatenation of a method result → hard compile error; rewritten to the
book's f-string / annotation-bound idiom → PASS vm+jit byte-identical). Details
under DELIVERABLE 1.

---

## Runnable surface of the chapter (what was exercised)

The chapter marks many snippets `runnable=false` and states the reason inline
for each. Those were correctly NOT exercised as runnable:
- named-impl `using ImplName` dispatch (WrapTypeAnnotation / deleted ValueWord)
- trait-bounded generic call sites `fn f<T: Display>(x)` (W14.2-E dispatch gap)
- default-method dispatch (VM≠JIT divergence, W14.2-E)
- generic-impl `Container<int> for IntList` (generic-arg erasure)
- user-defined `From`/`TryFrom` dispatch (Convert opcode cascade)
- associated-type substitution into return positions (generic-impl erasure)
- `extend Table<Row>` (needs real Table<Row> + row-spread)

The fully-runnable surface that WAS exercised:
- Defining a trait (required signatures)
- Implementing a trait + `method`/`self` direct dispatch
- Multiple traits on one type
- Supertrait `Sub : Super1 + Super2` declaration AND direct use of supertrait
  methods on an implementor
- Extend blocks on concrete types
- Enum implementing a trait via `match self`
- `dyn Trait` objects over homogeneous `Vec<dyn T>` (array-literal / annotated
  local form — the book idiom)
- Primitive conversion `Into` (`true as number`) + fallible `TryInto`
  (`(s as int?)?`)
- Generic-trait DECLARATION (`Container<T>`) and associated-type DECLARATION
  with bound (`Sequence { type Element : Renderable; }`)

---

## DELIVERABLE 1 — `small.shape` (107 LOC)

Trait decl + impl + method dispatch, a second trait on the same type, `extend`
blocks, supertrait declaration, `dyn Display` over an array literal, and the
primitive/fallible conversion traits.

| Mode | ec | stdout (last line) |
|------|----|--------------------|
| vm   | 0  | `ALL_CHECKS_PASSED` |
| jit  | 0  | `ALL_CHECKS_PASSED` |

stdout byte-identical: **YES**. Result: **PASS**.

### FIRST-RUN truth + author-error correction (2026-06-20 re-verification)

An earlier draft of `small.shape` fed trait/extend method results directly into
`+` string concatenation (`"...got=" + u.display()`, `acc + "," + item.display()`).
On first run that draft was a **hard compile error** under BOTH vm and jit
(ec=1):

```
error[SEMANTIC]: Cannot apply `+` to a `string` and a `unknown`. Strict typing
does not implicitly convert `unknown` to a string for concatenation. Use
f-string interpolation, e.g. `f"{...}"`, or convert the value to a string
explicitly before concatenating.
  --> <input>:70:35
```

The trait/extend method's declared `-> string` return type is not propagated to
an un-annotated call site, so the result is typed `unknown` and `+` rejects it.
This is the visible symptom of **book_gap #1** (root: call-site return-type
non-propagation; same W14.2-E trait-method dispatch family the book cites for
its `runnable=false` snippets).

Classified **AUTHOR-ERROR**: the book NEVER shows a method result used in `+`
concatenation — "Implementing a Trait" feeds it straight to `print(...)` and
"Conversion Traits" explicitly recommends `f"{...}"` interpolation. A real user
hitting this error and reading the compiler's own steering (and the book's
f-string recommendation) rewrites using annotation-bound locals
(`let d: string = u.display()`) and f-strings — which is exactly what the
shipped `small.shape` and `large.shape` now do. Probe confirming the three
book-blessed idioms all work (`print(m())`, `f"{m()}"`, `let x: string = m()`):
verified directly. The book-silent gap is recorded under book_gap #1; the
underlying mis-typing is recorded as an incidental defect below.

### Expected-value rationale (book-derived, written before first run)
- `u.display() == "Alice"` — "Implementing a Trait": `method display() -> string
  { self.name }`, `u = User { name: "Alice" }`.
- `u.greeting() == "Hi Alice"` — `f"Hi {self.name}"` (book recommends f-strings
  for embedding values, "Into / TryInto" note).
- `p.magnitude_sq() == 25`, `p.sum() == 7` — "Extend Blocks" exact bodies with
  `Point { x:3, y:4 }`: `3*3+4*4=25`, `3+4=7`.
- `render_all([Alice, Bob]) == 2` — "Trait Objects": iterate `Vec<dyn Display>`,
  two elements.
- `true as number == 1.0` — "Into / TryInto", primitive `as` ships in stdlib.
- `parse_int("42") == Ok(42)` — book's exact `parse_int` pattern `(s as int?)?`.

---

## DELIVERABLE 2 — `large.shape` (682 LOC, ~70 machine-checked assertions)

A deterministic **invoice / ledger rendering engine** rooted entirely in trait
machinery: `Priced` / `Renderable` traits with FIVE concrete implementors
(`LineItem`, `Discount`, `Shipping`, `Tax`, `Tip`), a supertrait
`Ledgerable : Renderable + Priced`, `extend` blocks on four of them, TWO enums
(`Cell`, `Status` — the latter with a struct-payload variant) implementing
traits via `match self`, `dyn`-dispatch aggregation over homogeneous
`Vec<dyn Priced>` / `Vec<dyn Renderable>` collections, primitive + fallible
conversion traits, and declaration-only `Container<T>` (generic trait) +
`Sequence { type Element : Renderable; }` (associated type with bound).

Every numeric/string result is asserted against a value derived from the §0
fixture block and book semantics, written BEFORE the first run. §0 hand-computes
the 2050c subtotal, 2400c base grand total, 3069c full total (with tax+tip),
and all closed-form sweep sums.

| Mode | ec | stdout (last line) |
|------|----|--------------------|
| vm   | 0  | `ALL_CHECKS_PASSED` |
| jit  | 0  | `ALL_CHECKS_PASSED` |

stdout byte-identical: **YES** (single line `ALL_CHECKS_PASSED`).
Stability: 3/3 vm + 3/3 jit ec=0, identical output. Result: **PASS**.

### Expected-value rationale (book-derived; selected)
- `Priced.cents()`: `widget 3*250=750`, `gadget 12*100=1200`, `bolt 4*25=100`
  (subtotal 2050); `Discount.cents()=0-amount` (sign-flip in impl body);
  `Shipping.cents()=fee`; `Tax.cents()=floor(base*rate_bp/10000)`
  (vat: floor(2050*825/10000)=169); `Tip.cents()=amount`.
- `render()`: exact f-string templates per impl body.
- Supertrait `tag()`: `ITEM:`/`DISC:`/`SHIP:`/`TAX:`/`TIP:` prefixes.
- `extend`: `gross_cents`/`is_bulk(qty>=10)`/`is_large(amount>=500)`/
  `half_cents`/`is_high(rate_bp>=1000)`/`doubled(amount*2)` — declared bodies.
- enum `Cell`/`Status` render+cents via `match` — exact arm bodies.
- Conversions: `true/false as number == 1.0/0.0`; `parse_cents("2050")=Ok(2050)`,
  `("xyz")=Err`.
- Grand total base = 2050 + (-300) + 650 = 2400; full = +169 +500 = 3069.
- Sweeps: closed-form `100*N(N+1)/2`, sign-flip sums, render-length sums.

### Author-error corrections made while authoring (a real user would make these)
1. **Mixed enum-variant array literals need a binding annotation.** Arrays
   holding different variants of one enum (`[Status::Open, Status::Paid(2400),
   Status::Refunded{...}]`) fail inference: *"cannot infer the element type of
   this array literal ... annotate the binding (`let a: Array<T> = ...`)"*. A
   real user follows the compiler's own hint and writes
   `let statuses: Array<Status> = [...]`. The chapter never shows a mixed-variant
   array, so this is consistent with (not contradicted by) the book — recorded
   as book_gap #4. The annotated `Array<Status>` local DOES coerce to a
   `Vec<dyn Priced>` parameter, so dyn-dispatch over it works.
2. **`sweep6` expected value**: my first draft mis-counted the `"PAID:"` prefix
   as 6 chars; it is 5 (`P-A-I-D-:`). The book-correct derivation of
   `f"PAID:{c}c".len()` is `5 + digits(c) + 1`, giving 70 (i=0..9) + 320
   (i=10..49) = **390**, which I corrected in BOTH the comment and the assertion
   BEFORE accepting. This is an arithmetic author-error in the expected value
   derived from book semantics — NOT a back-fill from output (the corrected 390
   follows directly from the literal `"PAID:"` having 5 characters).
3. `assert(...)` is not a builtin; hand-written `ck_int/ck_str/ck_bool` helpers
   gate `ALL_CHECKS_PASSED`. The traits chapter makes no `assert` claim — not
   book-wrong.

---

## Classification: PASS (both deliverables)

The chapter's fully-runnable surface works end-to-end under both VM and JIT
with byte-identical stdout. Every `runnable=false` snippet is a
book-acknowledged pre-existing gap and was correctly not exercised. No snippet
the book presents as runnable failed.

---

## book_gaps (book SILENT — fallback/workaround needed)

1. **Trait/extend method results need an explicit type annotation to retain
   their declared return type at the call site.** The book's runnable examples
   only feed a method result straight into `print(...)` or an f-string. The
   natural `u.display() + " and " + v.display()` is a hard compile error
   (`Cannot apply '+' to a 'string' and a 'unknown'`). Worse, `let a = p.sum();
   a + a` for an `extend method sum() -> int` evaluates to **`14.0` (a float)**,
   not `14` — the declared `-> int` is not propagated to the call-site
   inferencer, so the result is mis-typed as `number`. With an explicit
   annotation (`let a: int = p.sum()`) the result is the correct `int` (`14`).
   Both programs annotation-bind every method result for this reason. The book
   gives no hint this annotation is required. (Root: trait/extend method
   return-type not propagated to the call-site inferencer — same W14.2-E
   trait-method dispatch family the book cites for its `runnable=false`
   snippets, but here it surfaces for a pattern the book presents as ordinary.
   Verified: `/tmp/traits_probe/g1.shape`, `g3.shape`.)

2. **A function-returning a concrete `Vec<Struct>` does not coerce to a
   `Vec<dyn Trait>` parameter.** The book's "Trait Objects" example passes a
   local array literal (`let users = [...]; print_all(users)`), which works. A
   `fn build() -> Vec<User>` whose result is passed to
   `print_all(items: Vec<dyn Display>)` fails type-checking
   (`Vec<dyn Display> ... not compatible with Vec<{ name: string }>`). The book
   does not cover dyn-coercion of function-return collections; both programs
   construct every dyn collection as a direct array-literal / annotated local
   (the exact book idiom). Verified: `/tmp/traits_probe/g4.shape`.

3. **No `assert` builtin is documented for self-checking.** The chapter (and the
   acceptance methodology, which asks programs to assert results) gives no
   assertion primitive; hand-written check helpers are required. Minor, but the
   reader who wants to self-verify must invent their own.

4. **Mixed enum-variant array literals require an explicit `Array<T>` binding
   annotation.** `[Status::Open, Status::Paid(1), Status::Blank...]` fails
   element-type inference and must be written `let xs: Array<Status> = [...]`.
   The chapter never shows a heterogeneous-variant array, so a reader building a
   `Vec<dyn Trait>` from enum values (a natural extension of the "Trait Objects"
   idiom) hits an undocumented annotation requirement. The compiler's diagnostic
   is clear and the fix is mechanical, so this is a documentation gap, not a
   defect.

## book_wrong (book DOCUMENTS behavior the language does not do)

None at the level the book CLAIMS runnable. Every snippet presented without
`runnable=false` runs as the book describes. All defects above are in territory
the book is SILENT on (book_gaps), not territory it mis-describes.

## Incidental defects observed (recorded, not slice-root; not in final programs)

- **`let a = p.sum(); a + a` ⇒ `14.0` for an `int`-returning extend method.**
  Strictly a correctness defect (an `int`-declared method result mis-typed as
  `number` when the call site is un-annotated). It is the visible symptom of
  book_gap #1's root (call-site return-type non-propagation). Both programs
  avoid it via annotation, per the compiler's own steering toward annotated
  bindings. Tracked under the W14.2-E trait-method dispatch family; not a
  traits-book-wrong because the book never shows the un-annotated-arithmetic
  pattern. (`/tmp/traits_probe/g3.shape`.)
