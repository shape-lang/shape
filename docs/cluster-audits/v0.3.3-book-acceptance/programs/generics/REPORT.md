# Book-acceptance REPORT — slice: generics

Book-primary: fundamentals/functions.mdx + fundamentals/traits.mdx.
Binary: target/release/shape (HEAD, not rebuilt). All runs ulimit -v 12582912 + timeout 30, vm and jit.

## Headline
Generics slice is heavily caveated: nearly every runnable generic usage is marked runnable=false
in the book (generic-fn returning T, generic trait-bound dispatch, where-clauses, generic-impl,
user From/TryFrom). Book steers reader to concrete element types (first_int), the trait/extend/
dyn-Trait surface, and higher-order fns + closures. Exercising that steered surface surfaced a
real book-contradicting correctness defect in the HOF path that the book marks runnable=true.

## small.shape (~70 LOC)
VM ec=0 -> ALL_CHECKS_PASSED ; JIT ec=0 -> ALL_CHECKS_PASSED. vm_jit_byte_identical: YES.
Classification: PASS (with masking caveat). Line 59 asserts apply(dbl,21)==42; actual value is
42.0 (number, not int — HOF defect below) but passes because cross-kind numeric equality treats
42.0 == 42 as true (0.5 == 0 is false, so genuine value-equality across kinds). The == masks the
int->number widening. large.shape asserts the same shape unmasked and fails.

## large.shape (~915 LOC, "Shape & Inventory Toolkit")
Non-interactive, deterministic, machine-proofable. Geometry hierarchy over Vec<dyn Geometry>,
inventory over Vec<dyn Priced>, extend blocks, supertrait decls+direct calls, Vec2/Transform
pipeline over Vec<dyn Transform>, stats module over Vec<int> via map/filter/reduce w/ inferred
closures, default params, HOFs. ~85 checks, expected values derived from book formulas BEFORE run.
VM ec=0: CHECK_FAILED pipeline_5 expected=14 got=4622945017495814146 ; CHECK_FAILED pipeline_0
expected=4 got=4611686018427387906 ; FAILURES: 2. JIT ec=0: byte-identical 2 failures.
vm_jit_byte_identical: YES. Other ~83 checks PASS. Classification: FN-REG-CORRECTNESS.

## Defect (FN-REG-CORRECTNESS) — HOF int/number kind leak
Minimal repro (functions.mdx "Higher-Order Functions" 342-347, runnable=true, annotated // 42):
  fn apply(f, x) { f(x) }
  let double = |x| x * 2
  print(apply(double, 21))
Book says 42. Actual (VM and JIT, byte-identical): 42.0. Passing an int arg through an untyped HOF
param to an int-bodied closure loses the int kind and widens to number. double(21) called directly
yields 42 (int); only the HOF-forwarded path widens.
Cascade (pipeline_* failures): widened number forwarded through a 2nd untyped HOF param into an
int-closure, in a fn declared -> int:
  fn twice(f, x) { f(f(x)) }
  fn run_pipeline(start: int) -> int {
    let inc=|x| x+1 ; let dbl=|x| x*2
    let s1=apply(inc,start)  // 6 int OK
    let s2=apply(dbl,s1)     // 12.0 number (widening defect)
    let s3=twice(inc,s2)     // expected 14 ; ACTUAL 4622945017495814146
    s3 }
4622945017495814146 = 0x4022C00000000000 = IEEE-754 double 14.0. Value IS 14.0 but slot kind tracker
labels it int (the -> int return), so the f64 bits leak as a raw i64. Kind-confusion / pointer-as-
float raw-bits leak in the untyped-HOF + -> int-return path. NOT author error: every step is a
book-runnable=true shape; the originating widening is the language defect, garbage is its cascade.
Fix belongs in compiler HOF-parameter kind tracking. VM==JIT (JIT whole-program deopts to interpreter).

## book_wrong
1. functions.mdx "Higher-Order Functions" (342-347): runnable=true snippet apply(double,21) is
   annotated // 42 but shipped binary prints 42.0; the language silently widens int->number through
   an untyped HOF param, and twice/run_pipeline composition leaks a raw-bits garbage i64. Book should
   correct the annotation AND warn (or, preferred, language preserves int kind through HOF params).

## book_gaps
1. No dedicated generics chapter; for the slice core (a generic fn you can call and get T back) the
   book has NO runnable path — fn first<T>(items: Vec<T>) -> T is explicitly runnable=false, and the
   prescribed workaround abandons genericity for a concrete element type (first_int).
2. Cross-kind numeric equality undocumented: 42.0 == 42 is true, 0.5 == 0 is false. Strict docs say
   int and number do not unify but never state == compares across kinds. This silently masks the
   widening defect in naive == <int-literal> assertions.
3. No book guidance for a user-defined HOF taking a multi-arg numeric closure (e.g. fn fold(xs,f,seed)):
   pipe-internal annotations are a parse error and forwarding through an untyped f fails inference;
   only builtin .reduce / single-callee apply(f,x) pin closure param types.
4. Supertrait-method dispatch through dyn SubTrait is silent: Geometry : Named + dispatching the
   inherited Named method through Vec<dyn Geometry> surfaces "Concrete(Dyn([Geometry])) cannot have
   fields". Book never combines Supertraits with Trait Objects; reader is unwarned.

## Noise (non-blocking, output correct)
- .map/.filter emit "V2 bytecode verification failed ... has no FrameDescriptor" on stderr (both modes);
  arrays produced are correct.
- --mode jit emits [jit-fallback] and whole-program deopts to interpreter; stdout matches VM exactly.

## Verdict
small: PASS (masking caveat). large: FN-REG-CORRECTNESS (book-runnable HOF widens int->number and,
composed + returned through -> int, leaks a raw f64 bit-pattern as garbage i64). VM==JIT byte-identical.
