# Book-Acceptance REPORT — slice: ownership

Book PRIMARY: advanced/ownership-deep-dive.mdx. Binary: release HEAD (not rebuilt).
Determinism: pure (storage classes; var smart-default; escape->RC). No randomness/time/IO.

## Result summary
| Program | LOC | VM ec | JIT ec | stdout byte-identical | self-check |
|---------|-----|-------|--------|-----------------------|------------|
| small.shape | 100 | 0 | 0 | YES | ALL_CHECKS_PASSED |
| large.shape | 939 | 0 | 0 | YES | ALL_CHECKS_PASSED (111/111) |

Classification: PASS for both. JIT emits a stderr [jit-fallback] line (main fails
JIT V2 verification -> bytecode interpreter; documented). stdout byte-identical to VM.

## small.shape exercises (all pass)
move (struct) / clone keyword+method identity / array clone independence / scalar Copy /
call-arg share-by-value v0.3.3 / var smart-default Direct(mut) / let-mut field mutation /
owned Array<int> param + auto-ref dispatch / var-copy independence.

## large.shape — Ownership/Borrow-Checker Simulator
Pure machine-proofable model, 111 book-derived assertions, 15 sections:
var lattice / Copy-Clone classification / smart move-vs-clone / NLL loan liveness /
Datafrog conflict rules / repair precedence / concurrency three-rules / integration
liveness->conflict->repair / ownership ergonomics / usage-profile tally / place model
disjointness / NLL CFG worked example + B0001 perturbation / lifetime elision /
"What Shape Eliminates" 12-row table / Mutex-Atomic-Lazy v0.3.3 availability gate.

## Direct book-claim probes (all confirmed)
B0005 use-after-move (struct + string No-Copy) -> ec=1. call-arg share-by-value: push
caller-visible, binding usable. first-class ref read-through r+1 -> 43. disjoint &mut
obj.a/obj.b no conflict. B0001 shared-then-&mut conflict + repair suggestion -> ec=1.
Mutex/Atomic/Lazy construct (<mutex>/<atomic:0>/<lazy:pending>) but m.lock() -> "Method
'lock' not found" (methods not wired in v0.3.3, matches book caution).

## book_wrong
1. v0.3.3 limitation note (lines 209-216) "method/index through a stored reference is a
   compile error (Array cannot have fields)" is STALE. At HEAD `let r=&arr; r.len()`->3
   and `let r=&arr; r[0]`->10 both work (ec=0, VM==JIT). Benign (language MORE capable
   than documented), but the note is wrong and should be deleted.

## book_gaps
1. Exit-code / stderr-diagnostic semantics undocumented: compile errors exit ec=1 (never
   stated); a benign "V2 bytecode verification failed: ... Vec.slice/Vec.clone has no
   FrameDescriptor" line prints to STDERR whenever array .clone()/.slice() run (stdout
   correct, ec=0, VM==JIT) — a reader can't tell it's expected noise from the chapter.
2. var-copy independence under-specified: book never states the observable rule that
   `var b = a` (a still live) yields an INDEPENDENT value (verified: a len 3, b len 4).

## Prior-run defensive workarounds RE-VERIFIED at HEAD — both claimed defects GONE
- "TypedObject array literal inside non-main fn -> hard ec=1": does NOT reproduce
  (4-elem Array<Loan> from helper -> 4, ec=0; only benign stderr V2 line).
- "struct read out of array + mutate local aliases backing store": does NOT reproduce
  (`let mut a=arr[0]; a.balance=999` leaves arr[0].balance==100; struct copied on read).
No FN-REG-CORRECTNESS defect for this slice at HEAD. Program structure is over-cautious
but harmless; passes 111/111.
