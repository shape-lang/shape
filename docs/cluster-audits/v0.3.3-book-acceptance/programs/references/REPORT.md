# Book-Acceptance REPORT — slice `references`

Chapter (book-PRIMARY): `fundamentals/references-borrowing.mdx`
Determinism: pure (&, &mut, borrow rules). No stdin/clock/network/RNG.

## Result summary
| Program | LOC | assertions | VM ec | JIT ec | VM stdout | JIT stdout | byte-identical |
|---------|-----|-----------|-------|--------|-----------|------------|----------------|
| small.shape | 74  | 17  | 0 | 0 | ALL_CHECKS_PASSED | ALL_CHECKS_PASSED | YES |
| large.shape | 799 | 114 | 0 | 0 | ALL_CHECKS_PASSED | ALL_CHECKS_PASSED | YES |

Both PASS under both modes, byte-identical stdout. large.shape = 114 machine-proofable
assertions, all expected values hand-derived from book semantics before first run.
(Extended this rotation with descendant counts, distinct directed-path counts, flat-array
sum/max, in-place reverse + scalar-add through &mut, clone-before-mutation independence,
and a second 7-node perfect-binary-tree fixture.)

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
(none against the CURRENT chapter text.) A PRIOR rotation recorded that the book
documented stored-reference indexing (`r[0]`) as a v0.3.3 compile error while it actually
works. The current chapter (lines 226-230, `:::tip[Indexing and methods through a stored
reference]`) now DOCUMENTS it as working in v0.3.3 — matching the binary. Re-probed and
confirmed `let r = &nums; r[0]` / `r.len()` work under both modes. The imprecision is
already corrected in the book; no remaining book_wrong.

## Classification: PASS (slice). Book accurate for everything exercised; VM/JIT byte-identical.
