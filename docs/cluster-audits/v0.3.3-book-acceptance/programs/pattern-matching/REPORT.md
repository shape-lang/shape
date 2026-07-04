# Book-Acceptance Report — Slice: pattern-matching

Book chapter (PRIMARY source):
`/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/pattern-matching.mdx`

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape` (release, at HEAD).
All runs memory-capped (`ulimit -v 12582912`) + `timeout 30`, under both `--mode vm` and `--mode jit`.

## Summary

| Program | LOC | VM ec | JIT ec | VM/JIT stdout byte-identical | Verdict |
|---------|-----|-------|--------|------------------------------|---------|
| small.shape | 106 | 0 | 0 | YES | PASS |
| large.shape | 879 | 0 | 0 | YES | PASS |

Both print `ALL_CHECKS_PASSED`. large.shape runs 79 machine-checked assertions.

Note: a benign stderr line `V2 bytecode verification failed ... NewTypedArrayString ... in function 'Json.keys' has no FrameDescriptor` appears in BOTH modes for any program — it originates in the stdlib `Json.keys`, not in slice code, and does not affect stdout or exit code. Under `--mode jit`, `main` emits a `[jit-fallback]` diagnostic (same root cause) and runs under the interpreter; stdout is unaffected and byte-identical to VM.

## small.shape

Exercises the chapter core exactly as taught:
- Basic Match with `where`-guards + `_` wildcard ("Basic Match"): `sign`.
- Type-Based Matching on a union type `int | string` ("Type-Based Matching"): `normalize`.
- Enum Matching `Status::Ok(code)` / `Status::Error(msg)` ("Enum Matching"): `render`.
- Constructor Patterns `Some(v)`/`None`, `Ok(value)`/`Err(e)` ("Constructor Patterns"): `unwrap_or`, `describe`.
- Match Is an Expression — `match` value assigned to `let label`.

Expected values derived from the function semantics in the book examples (e.g. `sign(-5)=="negative"` per the `x < 0` guard). All 18 checks pass.

## large.shape — arithmetic expression language (lexer → parser → AST → evaluator)

A real-world, non-interactive, deterministic, machine-proofable application rooted in pattern matching:
- **Lexer**: `match` on single-character strings (literal string patterns + `_`) — `is_digit`, `digit_val`, `punct`; dispatch on a `Lexed` classification enum via `match`.
- **Parser**: recursive descent; `match` on the `Tok` enum (enum/constructor patterns) drives precedence; errors propagate as `Result` `Err` matched via constructor patterns.
- **AST**: recursive `Expr` enum; `eval`, `show` (pretty-printer), `node_count`, `depth`, and a constant-`fold` all dispatch via `match`, including **nested constructor patterns** (`Expr::Lit(x)` inside a `match` on a folded sub-tree; `Neg(Lit(v))`) and **bare-identifier catch-all binding patterns** (`other => ...`).
- **Error recovery**: division/modulo by zero, unbalanced parens, bad characters, trailing input — all surfaced as `Err(msg)` and asserted.

### Expected-value rationale (all derived from book semantics / arithmetic + grammar precedence, BEFORE first run)

Grammar (encoded in the parser), lowest→highest precedence:
`expr := term (('+'|'-') term)*` ; `term := factor (('*'|'/'|'%') factor)*` ;
`factor := unary ('^' factor)?` (right-assoc) ; `unary := '-' unary | atom` ; `atom := Num | '(' expr ')'`.

Representative derivations:
- `2 + 3 * 4` = 14 (`*` binds tighter than `+`). AST = `(+ 2 (* 3 4))`, node_count 5, depth 3.
- `2 ^ 2 ^ 3` = 256 (right-assoc: `2^(2^3)=2^8`). AST = `(^ 2 (^ 2 3))`.
- `10 - 3 - 2` = 5 (left-assoc: `(10-3)-2`). AST = `(- (- 10 3) 2)`.
- `-3 ^ 2` = 9: `parse_unary` consumes `-3` as `Neg(Lit 3)` before `^`, so `(^ (- 3) 2) = (-3)^2 = 9`.
- `7 / 2` = 3 (integer division truncates).
- `1 + 2*3 - 4/2 + (5-1)*2` = `1+6-2+8` = 13.
- `1 / 0` → `Err("division by zero")`; `5 % 0` → `Err("modulo by zero")`.
- `(1 + 2` → `Err("expected ) but found End")`.
- `1 + $` → `Err("unexpected character $")` (`$` where an atom is expected).
- `1 $ 2` → `Err("trailing input: Bad($)")` (`$` after a complete expression; parser stops, top-level End-check fires).
- Constant folding preserves evaluation: `eval(fold(e)) == eval(e)`; `(2+3)*4` folds to `Lit(20)`; `1/0` is left unfolded (preserved for runtime Err).

All 79 expected values were written from these semantics before the first run.

### Author-error encountered and corrected (NOT a language defect)

- `assert(...)` is not a Shape builtin (the binary suggests `sqrt`). I replaced it with explicit `if got != want { print("CHECK_FAILED: ...") }` self-checks. (My harness choice — a real user reading the chapter, which never mentions `assert`, would do the same.)
- Initial expected value for `"1 $ 2"` was `"unexpected character $"`. Re-derived from the grammar: `$` after a complete expression is **trailing input**, so the correct expected is `"trailing input: Bad($)"`; I added a separate `"1 + $"` case for the atom-position bad-char path. (Corrected by re-deriving from grammar semantics, NOT by back-filling observed output.)

## Language defect found (recorded, NOT worked around in a way that hides it)

### DEFECT: statement-position `if/else` forces branch-value type unification

When an `if`/`else` is used in **statement position** (its value discarded) and the two branches' **final statements have different "block-value" types** — one a value-producing expression (e.g. a function call returning `int`, or `arr.push(x)` which yields a non-void value), the other a void statement (an assignment `i = i + 1`) — the type checker rejects the program:

```
error[SEMANTIC]: Could not solve type constraints:
  int is not compatible with void
```

Minimal reproduction (10 lines, fails under both vm and jit):

```shape
fn f(flag: bool) -> int {
  let mut i = 0
  if flag {
    g(5)        // tail: value-producing expression (int)
  } else {
    i = i + 1   // tail: assignment (void)
  }
  return i
}
fn g(x: int) -> int { return x }
print(f(true))
```

Controlled triple (in `/tmp`, all under `--mode vm`):
- then ends in `push`/call, else ends in assignment → **ERROR** (`X is not compatible with void`).
- then ends in assignment, else ends in `push`/call → **ERROR** (`void is not compatible with X`).
- both branches end in assignment → **COMPILES**.

The `if` is in statement position; its value is discarded, so the branches should NOT be required to unify. This is a false-positive rejection of valid code.

Importantly, the **equivalent `match` in statement position does NOT exhibit the bug** — a statement-position `match` whose arm bodies have differing tail types compiles fine. So `match` (this slice's subject) is sound here; the bug is in `if/else` block-value inference. It surfaces in pattern-matching-style code only incidentally (parsers/lexers naturally mix `arr.push(...)` and counter assignments across `if` branches).

Classification: **FN-REG-CORRECTNESS** (compiler false-positive rejection of valid code; a strict-checker root). Scope: an `if/else` block-typing bug, tangential to the pattern-matching slice proper. It did NOT block delivery — large.shape was made idiomatic to the chapter by dispatching the lexer through a `Lexed` enum + `match` (the book's recommended tool) and keeping `if/else` branch tails uniform; the original mixed-tail `if/else` form is the repro above.

## book_gaps (book silent; I had to fall back / probe to proceed)

1. **No `assert` / test-helper documented.** The pattern-matching chapter (and, as far as a reader of this chapter can tell, the surrounding fundamentals) gives no built-in way to assert/verify a result. A self-checking program (the explicit deliverable goal) must hand-roll `if x != want { print("CHECK_FAILED...") }`. The chapter could note the idiomatic verification approach.
2. **Bare-identifier catch-all binding pattern `name =>` is undocumented.** The chapter teaches only `_` as the catch-all. I needed a *binding* catch-all (`other => Expr::Neg(other)`) to keep a matched value while defaulting; I tried it on instinct and it WORKS, but the chapter never shows it. (Fell back to trying it directly; confirmed working.)
3. **Struct/array/tuple destructuring patterns are listed in `llm_keywords` ("destructure") but NOT taught in the chapter body, and behave inconsistently:**
   - `match p { Point { x, y } => ... }` on a *plain struct type* is **rejected**: `variant pattern 'Point' requires an enum-typed value` — i.e. struct-type destructuring in `match` is NOT supported.
   - Struct-payload patterns on an *enum variant* (`Node::Bin { op, l, r }`) **do work**.
   - Array patterns (`[]`, `[x]`, `[x, y]`) **do NOT parse** (parser error at `[x]`).
   The chapter advertises "destructure" via keywords but teaches none of these, and the struct-type vs enum-variant-struct-payload distinction is surprising. A reader expecting "destructuring" from the keyword list would be misled about what works.
4. **String character API undocumented in this chapter.** Building any lexer needs single-char access; the chapter is silent (expected — out of scope), so I fell back to probing: `s.length()`, `s.charAt(i)`, `s.substring(a,b)`, `s.split("")` work; `s.charCodeAt(i)` does NOT exist. (Recorded as a fallback; arguably belongs to a strings chapter, not pattern-matching.)

## book_wrong (book documents behavior the language does not actually do)

None. Every construct the chapter *teaches in its body* (where-guards, `_` wildcard, type-based matching on unions, enum matching, constructor patterns `Some/None`/`Ok/Err`, match-as-expression, exhaustiveness without `_`) works exactly as documented under both VM and JIT.

(The "destructure" item is filed under book_gaps rather than book_wrong because the chapter body never actually *teaches* struct/array destructuring — it only appears in the LLM keyword list. If that keyword is taken as a documentation claim, item book_gaps #3 is the closest to book-wrong.)
