# Book-Acceptance REPORT — slice: enums

Chapter (book-PRIMARY): fundamentals/enums.mdx
Determinism: pure. Binary: target/release/shape at HEAD. Harness: ulimit -v 12582912 + timeout 30, both --mode vm and --mode jit.

## Summary
- small.shape (154 LOC): VM ec=0, JIT ec=0, ALL_CHECKS_PASSED, VM==JIT stdout byte-identical. Result: PASS.
- large.shape (936 LOC): VM ec=0, JIT ec=0, 144 checks ALL_CHECKS_PASSED, VM==JIT stdout byte-identical. Result: PASS.
(The [jit-fallback] / V2-verification lines are on stderr and excluded from stdout comparison.)

## small.shape
Unit/tuple/struct variants, exhaustive match, `_` wildcard, Option<T>/Result<T,E> + `?`, auto-derived Display.
Expected values cited to enums.mdx: area(Circle 5.0)=78.53975, area(Rectangle 3,4)=12.0 (Tuple Variants);
Move{x:10,y:20} -> "move to (10, 20)" (Struct-Style Variants); Status::Pending/Banned -> "not active" (Wildcard);
Display prints North and Circle(3.0) (Auto-Derived Display). No assert builtin -> if+CHECK_FAILED self-check.

## large.shape — expression-language interpreter
source -> lex (Array<Token>) -> parse (recursive Expr AST) -> eval (Result<Value, EvalError>). Enums everywhere:
Token, recursive Expr (tuple+struct), BinOp/UnOp, Value, EvalError; Option/Result + `?`. 144 asserts: token-tag
exhaustiveness, char classification, lexer counts+errors, arithmetic precedence/assoc, div/mod-by-zero via `?`,
record-array environment lookup (Option<int>), comparisons, boolean/logical (and<or), type errors, parse errors,
AST pretty-printer proving precedence, table-driven eval batch, deep `?` propagation. ALL expected values derived
from book/arithmetic semantics BEFORE first run; all matched. One authoring miscount fixed before locking:
lex("(x + y) * 2") = 7 tokens + TEnd = 8 (not 9) — AUTHOR-ERROR in the expected value, corrected.

## book_gaps (re-verified at HEAD 2026-06-22)
1. No assert/self-check primitive documented (and none in build) -> if+CHECK_FAILED+ALL_CHECKS_PASSED convention.
2. Recursive enums undocumented though they are the canonical enum use case (the Expr AST relies on them; work directly).
3. Struct-variant Display format undocumented; renders `Move { x: 1, y: 2 }` (only unit/tuple Display shown in chapter).
   VERIFIED: `print(Message::Move { x: 1, y: 2 })` -> `Move { x: 1, y: 2 }`.
4. `s as int?` Err carries a structured conversion-error object (category="RuntimeError", code="CONVERSION_FAILED",
   payload/message="cannot convert string '...' to int"), not a plain string as the Result section implies; large
   program uses explicit Err(string) for deterministic asserts. VERIFIED at HEAD.

### Prior-run gap RETRACTED after re-verification
- (was gap 2) "Arrays of enum values require explicit annotation; bare literal -> cannot infer element type."
  NOT REPRODUCED at HEAD: `let xs = [Color::Red, Color::Green]; print(xs.len())` -> `2`. Bare enum-array literals
  infer fine now. Dropped from book_gaps.

## book_wrong
None. Every behavior the chapter explicitly teaches worked, including the parse_port standalone `if { return Err }` +
tail `Ok` pattern (verified), Option/Result + `?`, exhaustive match, wildcard, and auto-derived Display.

## Prior-run side defect (FN-REG-CORRECTNESS) — RETRACTED, fixed at HEAD
A previous run logged a fn-returning-Result inference failure for an if/else-in-loop with a void then-branch and a
sibling `else { return Err(...) }`. RE-VERIFIED at HEAD 2026-06-22 and it NO LONGER REPRODUCES:
  fn f(src: string) -> Result<Array<int>, E> { ...
    while i < src.len() { if src[i] == "a" { out.push(1); i = i + 1 } else { return Err(E::Bad("other")) } }
    Ok(out) }
  match f("aa") { Ok(v) => print(v.len()), Err(_) => print("err") }  -> prints `2`, ec=0.
The checker root has been fixed since the prior run. No live defect in this slice. Slice result: PASS.

## JIT/VM diagnostics (cosmetic, stderr, output unaffected)
Both modes emit "V2 bytecode verification failed: ... StringLenTyped/StringConcatTyped/TypedArrayPush* ... has no
FrameDescriptor" on stderr; output correct + byte-identical. --mode jit emits one [jit-fallback] line (R8 W7 G.5
SURFACE, ADR-006 §2.7.14; tracked v0.4) and runs under the interpreter so the surface agrees with --mode vm.
