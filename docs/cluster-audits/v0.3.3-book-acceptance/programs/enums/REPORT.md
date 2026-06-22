# Book-Acceptance REPORT — slice: enums

Chapter (book-PRIMARY): fundamentals/enums.mdx
Binary: target/release/shape (release, HEAD, prebuilt)
Date: 2026-06-21
Determinism strategy: pure (no I/O, time, randomness, network).

## Summary verdict

Both programs PASS under VM and JIT, byte-identical stdout. The enums chapter is
ACCURATE: unit/tuple/struct variants, EnumName::Variant construction, exhaustive
match, the _ wildcard, the non-exhaustive-match compile error, built-in
Option<T>/Result<T,E>, the ? operator, and auto-derived Display all behave
exactly as written. Verbatim book examples (Vec<number> find_first; parse_port
with `s as int?`; print(Direction::North) -> "North"; print(Shape::Circle(3.0))
-> "Circle(3.0)") all run and produce the book-stated output.

Classification: PASS (both programs).

## small.shape (~95 LOC)
- VM ec=0: North / Circle(3.0) / ALL_CHECKS_PASSED
- JIT ec=0: identical stdout
- vm_jit_byte_identical (stdout): TRUE
- Covers unit (Direction), tuple (Shape; book area values 78.53975, 12.0),
  struct-style (Message incl Move/ChangeColor), wildcard _ (Status),
  Result+?, Option+?, auto-derived Display.
- Expected values from book: area(Circle 5.0)=78.53975, area(Rectangle 3,4)=12.0
  (book "Tuple Variants"); Display "North"/"Circle(3.0)" (book "Auto-Derived
  Display").

## large.shape (~810 LOC) — arithmetic expression interpreter
Recursive-descent (precedence-climbing) parser + tree-walking evaluator rooted
entirely in enums: recursive Expr (tuple variants), ParseErr (struct+tuple+unit),
EvalErr (unit), ParseResult (struct+tuple), built-in Option/Result + ?
propagation. Parser, evaluator, pretty-printer, depth/node/leaf metrics,
constant-folder (AST->AST), RPN compiler — all exhaustive match folds over the
recursive enum. 103 assertions, every expected value hand-derived from grammar
precedence BEFORE first run.
- VM ec=0: checks: pass=103 fail=0 / ALL_CHECKS_PASSED
- JIT ec=0: identical stdout
- vm_jit_byte_identical (stdout): TRUE

## Independent verification of prior-draft defect claims (NONE reproduce)
The pre-existing draft baked in THREE "defect" claims to justify design choices.
Re-probed at HEAD; none reproduce:
1. 3-function mutual recursion (a->b->c->a): links+runs, prints 6. NOT REPRODUCED.
2. Array<EnumType> literal [Color::Red,Color::Green,Color::Blue]: .length==3. NOT REPRODUCED.
3. arr[0].substring(0,1) on Array<string>: prints "a". NOT REPRODUCED.
Also probed "block-arm match over a ?-using Result fn mis-infers binder" — does
NOT reproduce (returns correct value). small.shape now uses natural block-arm
match. Misleading comments in large.shape corrected to record the re-probe;
working design retained on its own merits. None affect enums-chapter accuracy.

## VM/JIT note (not a defect)
Int array literals emit a STDERR-only "V2 bytecode verification failed ...
NewTypedArrayI64 ... no FrameDescriptor" warning; ?-using programs emit a
STDERR-only "[jit-fallback] c4-4B TryUnwrap SURFACE" diagnostic under --mode jit
(ADR-006 §2.7.14, c4-4B). Both fall through to the interpreter so stdout is
correct and VM==JIT; ec=0 throughout. Pre-existing, slice-orthogonal.

## book_gaps
(none — every construct the programs needed is taught in the chapter, and the
chapter's own examples run verbatim.)

## book_wrong
(none — chapter accurate: non-exhaustive-match compile error, Option/Result
definitions, ? semantics, and auto-derived Display output strings all match
observed behavior.)
