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

## book_gaps
1. No assert/self-check primitive documented (and none in build) -> if+CHECK_FAILED+ALL_CHECKS_PASSED convention.
2. Arrays of enum values require explicit `Array<Message>` annotation; bare literal -> "cannot infer element type".
   Chapter shows no collection of enum values.
3. Recursive enums undocumented though they are the canonical enum use case (the Expr AST relies on them; work directly).
4. Struct-variant Display format undocumented; renders `Move { x: 1, y: 2 }` (only unit/tuple Display shown in chapter).
5. `s as int?` Err carries a structured conversion-error object ({category, code: CONVERSION_FAILED, ...}), not a plain
   string as the Result section implies; large program uses explicit Err(string) for deterministic asserts.

## book_wrong
None. Every behavior the chapter explicitly teaches worked, including the parse_port standalone `if { return Err }` +
tail `Ok` pattern (verified).

## Side language defect (FN-REG-CORRECTNESS) — NOT in delivered programs, NOT book-taught code
A fn returning Result<T,E> fails to compile ("void is not compatible with Result<TypeVar, E>") when an if/else INSIDE
a loop has a then-branch ending in a void statement (i = i + 1) and a sibling `else { return Err(...) }`, with tail
Ok(out). The else-branch return poisons inference of the if/else expression type. Minimal repro:
  enum E { Bad(string) }
  fn f(src: string) -> Result<Array<int>, E> {
    let mut out: Array<int> = []; let mut i = 0
    while i < src.len() {
      if src[i] == "a" { out.push(1); i = i + 1 } else { return Err(E::Bad("other")) }
    }
    Ok(out)
  }
Does NOT reproduce with the book's standalone-guard form (`if cond { return Err } ; ... ; Ok(out)`), which the chapter
teaches and which lex was written to use (continue + single_op(c)->Option<Token> + match). Idiomatic, book-faithful,
not a workaround. Slice result remains PASS.

## JIT/VM diagnostics (cosmetic, stderr, output unaffected)
Both modes emit "V2 bytecode verification failed: ... StringLenTyped/StringConcatTyped/TypedArrayPush* ... has no
FrameDescriptor" on stderr; output correct + byte-identical. --mode jit emits one [jit-fallback] line (R8 W7 G.5
SURFACE, ADR-006 §2.7.14; tracked v0.4) and runs under the interpreter so the surface agrees with --mode vm.
