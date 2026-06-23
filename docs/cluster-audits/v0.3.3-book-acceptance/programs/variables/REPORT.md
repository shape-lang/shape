# Book-Acceptance Report — slice: `variables`

- Chapters (book-PRIMARY): fundamentals/variables.mdx, fundamentals/names-and-scope.mdx
- Binary: /home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape (prebuilt at HEAD)
- Determinism: pure (no I/O/time/randomness/network). Both programs fully self-checking.
- Methodology: book-primary. Every construct used is taught in the two chapters. No reference/MCP fallback needed.

## Results

| Program | LOC | Asserts | VM ec | JIT ec | VM stdout | JIT stdout | byte-identical |
|---------|-----|---------|-------|--------|-----------|------------|----------------|
| small.shape | 110 | 28  | 0 | 0 | ALL_CHECKS_PASSED | ALL_CHECKS_PASSED | YES |
| large.shape | 886 | 106 | 0 | 0 | ALL_CHECKS_PASSED | ALL_CHECKS_PASSED | YES |

Both PASS under both modes with byte-identical stdout.

### stderr verifier diagnostics (NOT failures)
Both runs emit V2-bytecode-verifier warnings on stderr (e.g. NewTypedArrayI64 ... has no
FrameDescriptor; plus an unrelated Json.keys/NewTypedArrayString warning that fires even on a
trivial program). Per `shape run --help` (--mode jit semantics) and the in-binary SURFACE note
(ADR-006 §2.7.14, R8 W7 G.5), these are non-fatal: the JIT refuses the unverified V2 typed opcode
and falls through to the bytecode interpreter so VM and JIT surfaces agree. Execution completes,
all assertions pass, ec=0, stdout=ALL_CHECKS_PASSED under both modes. Pre-existing JIT-coverage
debt (v0.4 candidate, docs/cluster-audits/v0.3-r8w6-hashmap-key-kind-audit.md), NOT a defect of
these programs and does not affect the slice verdict.

## small.shape — expected-value rationale (book-derived)
Binding forms (let/let mut/const), inference (int/number/string/bool), annotations (incl u8
0xFF=255), Option/no-null (Some(7)/None -> 7/-1), tuples [int,int], named types, type alias as
constructor (type P = Point), generic id<T>, scope + shadowing (inner let s=2 shadows outer s=1,
outer unchanged). All expected values from variables.mdx + names-and-scope.mdx semantics.

## large.shape — design + expected-value rationale (book-derived)
~886-LOC deterministic machine-proofable "MiniVM" (stack+register bytecode interpreter) built
from the chapter's binding forms: const opcode table, let/let mut accumulators, named type
records, type alias, [int,int] tuples, Option lookup, block scope+shadowing, generics, enum
associated-namespace access (OpName::Push). 106 assertions, all expected values hand-derived
BEFORE first run (no back-fill): arithmetic opcodes, dup/swap, registers, HALT/RunResult,
polynomial "compiler" f(x)=3x^2+2x+7 cross-validated vs host for x in 0..8, factorial/fib/gcd via
VM opcodes, running-stats, Sieve (pi(100)=25), digit-sum/reverse via VM MOD/DIV, Option symbol
table.

## book_gaps
None. Everything needed is taught in the two assigned chapters; no MCP/reference fallback used.

## book_wrong
None. Every book-documented behavior used behaves as the chapters describe.

## Author-corrected stale comment (not a defect)
A prior draft of large.shape claimed a bare `let b = stack.pop()` inside an if/else branch in a
function infers `unknown` on this build, requiring `: int` annotations. Re-verified directly: bare
pop-in-branch works both at top level and inside a fn-with-while if/else (ec=0, correct result).
The claim was stale/inaccurate on current HEAD. Annotations kept (valid, book-taught "Type
Annotations" syntax; harmless) but the comment was corrected to "belt-and-braces, not a
workaround". Author-comment correction only — not a language defect, not a book finding.

## Classification
- small.shape: PASS
- large.shape: PASS
- Slice verdict: PASS (book-faithful, byte-identical VM/JIT stdout, all 134 assertions green).
