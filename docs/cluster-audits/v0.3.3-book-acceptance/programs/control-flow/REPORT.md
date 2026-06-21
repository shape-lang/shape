# Book-Acceptance Report — slice: control-flow

Worktree: shape-strict-flip-collection-dispatch (v0.3.3 cumulative strict-flip)
Binary: target/release/shape (HEAD, prebuilt)
Book source (PRIMARY): fundamentals/control-flow.mdx
Determinism strategy: pure (no clocks/network/random; LCG seeded with fixed constants)

## Programs

### small.shape — PASS (vm + jit, stdout byte-identical)
Exercises every construct the chapter teaches: if/else-if/else as an expression;
block-returns-last-expression; `for` over exclusive `0..5` and inclusive `0..=5`
ranges; destructuring loop binding `for {x,y} in points`; `while`; break/continue;
`loop`; break-with-value. Self-check uses ONLY chapter constructs (if-expression +
print) — `assert` is NOT taught (and does not exist; see book_gaps). On all-pass
prints ALL_CHECKS_PASSED.

Expected-value rationale (all derived from book semantics before running):
- if-expression: score=84, 80<=84<90 -> "B" (§If Expressions).
- block value: `{ let base=10; base*2 }` -> 20 (§Blocks Return Values).
- exclusive `0..5` sums 0+1+2+3+4 = 10; inclusive `0..=5` adds 5 -> 15
  (§For Loops + llm_common_mistakes: 0..n exclusive, 0..=n inclusive).
- destructuring: (1+2)+(3+4)=10 (§For Loops destructuring example).
- while: 3 iterations for `i<3` (§While Loops).
- break/continue: skip 2, stop at 6 -> visits 0,1,3,4,5 summed = 13 (§Break and Continue).
- loop break at k>=5 -> k=5 (§Loop).
- break-with-value: first n*4 > 10 is n=3 -> 12, *2 = 24 (§Break with Value, matches
  the book's own "// first x*2 where x>10" comment).

vm ec=0, jit ec=0, both stdout = "ALL_CHECKS_PASSED" (byte-identical).

### large.shape — PASS (vm + jit, stdout byte-identical), 686 LOC
A non-interactive, machine-proofable control-flow application with seven engines,
each asserting spec-derived expected values written before first run:
- PART A — Collatz/hailstone (loop + break-with-value + while + if-chain).
  Step counts hand-derived: n=3->7, n=6->8, n=7->16, n=27->111, max(27)=9232,
  sum(1..=10)=67, longest in 1..=20 is 20 steps first at n=18.
- PART B — elementary cellular automaton rule lookup + 1 and 2 step evolutions
  (nested for + if-chains; toroidal wrap). Rule-bit tables for 90 and 110 derived
  from their binary expansions; populations hand-simulated.
- PART C — RPN calculator over an Array<int> token stream (operators encoded as
  negative sentinels -1..-4) using while + if-chain + fixed-capacity int stack.
  Results: 3 4 + =7; classic 14; 2 3 4 * + =14; 10 2 / =5; 7 2 - 3 * =15.
- PART D — traffic-light FSM (if-chain state machine, while). G3/Y1/R2 durations;
  T=12 -> green6/yellow2/red4/cycles2; T=7 -> green4/yellow1/red2/cycles1.
- PART E — histogram + classification with continue (skip negatives) and break
  (sentinel 999). sum=27, counts derived; classify(27)="odd-large"; nested-loop
  even-sum pairs=4; first k with k*k>=50 is 8.
- PART F — deterministic LCG x=(5x+3) mod 16, seed 7 (full period 16, outputs a
  permutation of 0..15). sum=120, evens=8, max=15, min=0, count>9=6, buckets
  [4,4,4,4], period-wrap=6, oddSum=64. All derived from the recurrence by hand.
- PART G — Rule-90 population trajectory (toroidal w8, seed idx4). Hand-simulated
  via next[i]=row[i-1] XOR row[i+1]: pops gen0..8 = 1,2,2,4,0,0,0,0,0 (sum 9; first
  all-zero at gen4); gen3 row = 01010101; plus a 3x3 row-major grid whose main and
  anti diagonals both sum to 15.

Negative control: flipping one expected (collatz n=27 111->110) produced
"CHECK_FAILED: collatz n=27 expected=110 got=111" and suppressed ALL_CHECKS_PASSED,
proving the assertions fire (not vacuously passing).

vm ec=0, jit ec=0, both stdout = "ALL_CHECKS_PASSED" (byte-identical). Under both
modes stderr carries a non-fatal "V2 bytecode verification failed ... has no
FrameDescriptor" diagnostic for the typed-array opcodes; the program nonetheless
executes correctly under the interpreter and stdout is unaffected (same known
JIT/V2-verifier surface noted in the run --help text and MEMORY notes; not a
control-flow defect).

## Classification

Both deliverables: PASS. vm_jit_byte_identical = true for both.

## book_wrong (book followed correctly, language disagrees)

1. KEYWORD-PREFIXED IDENTIFIER IN A SAME-KEYWORD CONDITION FAILS TO PARSE.
   The chapter teaches `if <cond> { }` and `while <cond> { }` with arbitrary
   condition expressions and gives no naming restriction on variables. But an
   identifier whose prefix is the SAME control-flow keyword as the enclosing
   construct, used in that construct's CONDITION position, derails the parser:
     var whileCount = 3
     if whileCount != 3 { print("a") }   // condition uses `whileCount` -> ok here
   Actually the trigger is the identifier appearing as the condition head of the
   matching keyword. Confirmed minimal cases (all on this binary, vm AND jit,
   parse stage so mode-independent):
     - `while whileX < 3 { ... }`  -> error[E0001] "expected a block { }, found
        identifier `print`" (the loop body is mis-attached).
     - `if ifX > 1 { ... }`        -> same family of error.
     - `if whileCount != 3 { ... }` (whileCount in an `if` condition) -> error
        "expected a block, found keyword `var`/`else`".
   Cross-keyword is FINE: `for forX in ...` ok; `if forCount > 1 { }` ok;
   `if breakX != 3 { }` ok. A bare `let z = whileCount + 1` (non-condition
   expression position) is ALSO ok. So the lexer fails maximal-munch /
   word-boundary checking for the keyword that introduces the current construct
   when scanning its condition expression: `while`/`if` are matched as a keyword
   prefix inside `whileX`/`ifX`, and the grammar then never sees the block.
   IMPACT on a real book reader: natural variable names like `whileCount`,
   `ifCount`, `whileLeft` used directly in the loop/branch that bears the same
   name produce a confusing parse error the chapter gives no reason to expect.
   Classification: FN-REG-CORRECTNESS (parser/lexer correctness defect). Worked
   around in the deliverables by avoiding keyword-prefixed identifiers in
   condition positions (documented inline). First-run truth recorded above.

## book_gaps (book silent; had to reach outside the chapter)

The control-flow chapter is intentionally narrow, but a realistic control-flow
APPLICATION needs data containers it never mentions. Each of these forced a
fallback (probed via direct experimentation against the binary):

1. Array construction beyond literals. `[0].filled(n)` — the obvious way to build
   an n-element zero array — does NOT exist ("Method 'filled' not found on type
   'Vec'"). The chapter shows only literal arrays `[1,2,3]` and iteration. Worked
   around with fixed-size literal arrays.
2. Array element type after empty-array clear. Seeding `var s=[0]` then `s.pop()`
   to get an "empty typed stack" loses the element type: a later `s.pop()` feeding
   `a + b` yields "operand types are unknown and unknown" (strict-typing inference
   gap). The chapter teaches no stack idiom; worked around with a fixed-capacity
   array + explicit `top` index (reading an int-literal array element is typed int).
3. String iteration / digit parsing. `"256".chars()` does NOT exist ("no method
   'chars' on receiver kind String"). The chapter teaches strings only via
   f-interpolation. The RPN parser was redesigned to take an Array<int> token
   stream (operators as negative sentinels), removing string parsing entirely.
4. `assert`. There is no `assert` builtin ("Undefined function: assert. Did you
   mean 'sqrt'?"). The chapter's only output primitive is `print`. Self-checks use
   if-expression comparisons + a CHECK_FAILED print, which the chapter DOES teach.

None of these gaps is a control-flow defect; they are chapter-scope omissions that
surface the moment a reader writes a non-trivial program rooted in control flow.

## Notes
- vm and jit stdout are byte-identical for both programs. JIT emits extra stderr
  diagnostics ([jit-fallback] / V2 FrameDescriptor verification) but falls through
  to the interpreter and produces identical stdout, consistent with documented
  --mode jit fall-through semantics.
