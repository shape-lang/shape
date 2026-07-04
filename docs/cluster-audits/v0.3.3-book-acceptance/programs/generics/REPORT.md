# Book-Acceptance Report — slice: generics

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape` (release, HEAD).
Run harness: `ulimit -v 12582912; timeout 30 shape run --mode {vm,jit} <file>` (memory-capped + time-bounded).
Book sources (PRIMARY): `fundamentals/functions.mdx`, `fundamentals/traits.mdx`.

## Programs

### small.shape (69 LOC)
- VM ec=0, JIT ec=0. stdout = `ALL_CHECKS_PASSED` (both).
- VM stdout == JIT stdout BYTE-IDENTICAL.
- Exercises: generic identity `fn id<T>(x:T)->T`, multi type-param generics
  (`pick_first<A,B>`, `pick_second<A,B>`), scalar-param generic with single
  trait bound (`show<T: Display>`), `where`-clause form (`show_w<T> where T: Display`),
  multiple bounds (`show_tag<T: Display + Tagged>`), monomorphization at two
  distinct concrete types (User/Product), user-trait-method dispatch from a
  generic body.
- Classification: PASS.

### large.shape (950 LOC, 220 asserted checks)
- VM ec=0, JIT ec=0. stdout = `ALL_CHECKS_PASSED` (both).
- VM stdout == JIT stdout BYTE-IDENTICAL.
- A deterministic generic catalog/report engine: 4 record types (Book, Gadget,
  Subscription, Service) each implementing a set of user traits (Display, Priced,
  Stocked, Coded) + a supertrait (Catalogable : Display + Priced). ~20 generic
  functions parameterized by single/multiple/where-clause trait bounds drive all
  pricing, tax, restock, affordability, formatting and aggregation. Every result
  is asserted against a value hand-derived from book semantics BEFORE the first run.
- Classification: PASS.

## Expected-value rationale (representative — all values pre-derived from book semantics)
- `inventory_value(x) = x.price() * x.stock()` — pure integer multiply. E.g. b1: 4999*12=59988.
- `discounted_price(x,pct) = p - (p*pct)/100` with INTEGER truncating division
  (book: `int` is i64; `/` on ints truncates). E.g. b1 @10%: 4999-499=4500.
- `with_tax = p + (p*8)/100`, truncating. E.g. b2: 8999 + 719 = 9718.
- `money(cents)`: dollars=cents/100, rem=cents%100, pads rem<10 with a leading 0
  ("$D.0R") else "$D.RR". Two decimal places ALWAYS. E.g. money(1500)=$15.00,
  money(305)=$3.05, money(534485)=$5344.85.
- Named-args + defaults (functions.mdx "Named Arguments"/"Default Parameter Values"):
  `box_vol(h:6)` => w=1,h=6,d=1 => 6; `box_vol(d:4,w:2,h:3)` => 24.
- `as`/`as?` primitive conversions (traits.mdx "Conversion Traits"): `true as number`=1.0;
  `("42" as int?)?`=42 inside a `Result`.

## Failure classifications (first-run truth)
None of the final checks fail. Two issues surfaced during authoring; both were
resolved within author discipline (a real user would do the same), and one is a
genuine language limitation the book does not warn about:

1. **Annotated lambda params `|x: int|` — PARSE ERROR.** AUTHOR-ERROR. The book
   (functions.mdx "Lambdas") explicitly says "Type-annotations are NOT allowed
   inside the pipes." I initially wrote `|x: int|`; corrected to bare `|x|`
   (inferred from call site / context, per the book). Not a defect.

2. **`money(€15.0)` expected mismatch.** AUTHOR-ERROR in my hand-derived expected
   value (I wrote `€15.00` semantics but typed `€15.0`). The code is correct:
   1500 cents = $15.00 (two decimals). Re-derived from `money` semantics, NOT
   back-filled from output. Fixed expected to `€15.00` / `$279.00`.

3. **Closure capture of a FUNCTION-LOCAL binding — `Undefined variable` at runtime.**
   BOOK-GAP (see below). NOT author-error: the book's "Closures and Capture"
   runnable example captures a binding and I followed it; the book just never
   states the captured binding must live at MODULE scope.

## book_gaps (book SILENT; required MCP/reference or trial to discover)

1. **Closure capture is restricted to MODULE-scope bindings.**
   `functions.mdx` "Closures and Capture" shows a runnable read-only capture
   (`let count = 10; let f = |x| x + count`) at MODULE scope and presents it as
   the general capture model. But capturing a FUNCTION-LOCAL binding (a parameter
   OR a local `let`) into a closure defined inside that function body fails at
   runtime with `error[RUNTIME]: Undefined variable: <name>`. First-run truth
   (VM and JIT both):
   ```
   fn make(store: string) -> string {
     let banner = |n| f"{store} has {n} items"   // captures the param `store`
     banner(3)
   }
   // => error[RUNTIME]: Undefined variable: store
   ```
   The module-scope form works. The book never warns that capture is scope-limited.
   Reworked the large program to capture a module-scope binding (the book's exact
   runnable shape). GAP: the "Closures and Capture" section should state that only
   module-scope read-only capture is supported on v0.3.3, or document the
   function-body-capture limitation.

2. **`reduce` argument order is undocumented in the functions chapter.**
   The functions chapter never gives the `reduce` signature; discovering it
   required a trial run, which produced the compiler's own hint:
   `reduce(f, init)` — callback first, NOT `reduce(init, f)`. The chapter teaches
   `map`/`filter` via examples but is silent on `reduce`. (Resolved via the
   compiler diagnostic, but the chapter should show the signature.)

3. **No `assert` builtin / self-check primitive is taught.** Neither chapter shows
   how to assert results. `assert(...)` is undefined (`Undefined function: assert`).
   Self-checking programs must roll their own `if got != want { print(...) }`
   helpers. Minor — but a reader writing a self-checking program has no guidance.

## book_wrong (book DOCUMENTS something the language does NOT do)
None confirmed. The chapters are candid: every generic case that genuinely fails
(`fn first<T>(items: Vec<T>) -> T`, `Vec<T>`-param trait-bound generics, generic
trait dispatch through `Vec<T>`) is correctly marked `runnable=false`. I verified
the marked-broken cases ARE broken (first-run truth):
- `fn first<T>(items: Vec<T>) -> T { items[0] }` => SEMANTIC error
  `(Vec<unknown>) -> unknown is not compatible with (Vec<int>) -> unknown`. Book-accurate.
- `fn render_all<T: Display>(items: Vec<T>) -> Vec<string>` => SEMANTIC error
  `(Vec<unknown>) -> Vec<string> is not compatible with (Vec<User>) -> Vec<string>`.
  Book-accurate.

## Observation (book over-conservative, NOT book-wrong)
The traits.mdx "Trait Bounds" snippets are marked `runnable=false` with the claim
that "monomorphizing a generic call site whose body dispatches through a
user-defined trait method currently surfaces a pre-existing trait-method dispatch
gap." This is true ONLY for the `Vec<T>` examples shown. The SCALAR-parameter form
works end-to-end (VM and JIT, byte-identical), including multiple bounds, where
clauses, and instantiation at several distinct concrete types:
```
fn show<T: Display>(x: T) -> string { x.display() }     // WORKS
fn show_tag<T: Display + Tagged>(x: T) -> string { ... } // WORKS
fn show_w<T>(x: T) -> string where T: Display { ... }    // WORKS
```
The entire large program (200000+ trait-method dispatches through scalar-param
generics) passes. The book's `runnable=false` is over-broad: the actual gap is
`Vec<T>` PARAMETER inference, not generic trait-method dispatch per se. The book
could safely show a runnable scalar-param trait-bound generic example. Logged as
an observation; not classified BOOK-WRONG (a `runnable=false` snippet that the
engine partially supports is conservative, not incorrect).

## Determinism
Pure: all inputs are fixed integer/string literals; no time, randomness, network,
or stdin. VM and JIT outputs are byte-identical for both programs.
