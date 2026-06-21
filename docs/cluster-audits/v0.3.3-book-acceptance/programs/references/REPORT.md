# Book-Acceptance REPORT — slice `references`

Chapter (book-PRIMARY): `fundamentals/references-borrowing.mdx`
Determinism: pure (&, &mut, borrow rules). No stdin/clock/network/RNG.

## Result summary
| Program | LOC | VM ec | JIT ec | VM stdout | JIT stdout | byte-identical |
|---------|-----|-------|--------|-----------|------------|----------------|
| small.shape | 97 | 0 | 0 | ALL_CHECKS_PASSED | ALL_CHECKS_PASSED | YES |
| large.shape | 826 | 0 | 0 | ALL_CHECKS_PASSED | ALL_CHECKS_PASSED | YES |

Both PASS under both modes, byte-identical stdout. large.shape = 97 machine-proofable
assertions, all expected values hand-derived from book semantics before first run.

Stderr emits pre-existing V2 FrameDescriptor verification warnings + a [jit-fallback]
(ADR-006 §2.7.14) note; these are infra diagnostics, not slice-specific, go to stderr,
do not affect stdout, and both modes still reach ALL_CHECKS_PASSED. Classified PASS.

## Book-claim probes
- use-after-move B0005 (book 30-39): `let b=a; print(a)` => B0005 compile error ec=1. CORRECT.
- by-value share (book 49-54): `append(data); data.len()` => 4. CORRECT.
- explicit &/&mut params + call-site &arr[i] index borrow (book 142-186): work. CORRECT.
- clone keeps source valid (book 93-101): CORRECT.

## book_gaps
(none — both deliverables writable from chapter positive guidance alone; no fallback used.)

## book_wrong
1. Stored-reference index `let r=&nums; r[0]` documented (book 225-227) as a v0.3.3
   compile error ("Borrow ... does not support index access") but actually WORKS:
   prints 1, ec=0, both VM+JIT. Book wrong in the SAFE/conservative direction (warns
   off a construct that works); does not break book-following programs. Low severity.

## Classification: PASS (slice). One conservative-direction doc imprecision (book_wrong #1).
