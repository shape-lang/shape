# Resumability slice — book-acceptance report

Binary: target/release/shape (strict-flip-collection-dispatch worktree)
Book sources (PRIMARY): advanced/resumability.mdx, stdlib/core/snapshot.mdx
Determinism strategy: snapshot->resume a deterministic computation; assert resumed == uninterrupted.

## Headline
snapshot() is NON-FUNCTIONAL on this binary. Every documented book pattern that calls
snapshot() aborts at the call site with:
  Error: Runtime error: Suspended on future 18446744073709551615  (ec=1, no stdout, no snapshot persisted)
The book's first-pass contract (return Snapshot::Hash(id) and continue) never occurs, so the
slice determinism strategy cannot be exercised at all. Classification: BOOK-WRONG (also a
functional regression: the same test-arena/advanced/test_resumability_* programs that produced
stored snapshots on 2026-03-10 now fail identically).

## Programs
small.shape         book top-level pattern        VM ec=1 JIT ec=1  stdout empty (identical)
large.shape (~339)  resumable deterministic ETL    VM ec=1 JIT ec=1  stdout empty (identical)
large_control.shape same pipeline, snapshot removed VM ec=0 JIT ec=0  "canonical=1502454789"+ALL_CHECKS_PASSED (identical)

## Defects (first-run truth, no workarounds)
D1 top-level snapshot() first pass aborts (Suspended on future). Reproduces on pre-existing
   test-arena test_resumability_vm_mode.shape -> regression.
D2 function-level book example (runnable=true) fails to COMPILE: error[SEMANTIC]: Undefined
   function: 'snapshot'. use { Snapshot } imports the enum, not the function.
D3 snapshot.mdx qualified form snapshot::snapshot() -> error[SEMANTIC]: module namespace
   'snapshot' is not typed. The two chapters use inconsistent call syntax; neither works.
D4 full --resume <hash> of a pre-existing snapshot -> I/O error: No such file or directory
   (missing saved bytecode artifact).

## Expected-value rationale (large.shape)
~30 invariants, each a theorem from deterministic semantics; none back-filled from output:
I1 LCG purity + hand-computed lcg_next(12345)=87628868 (verified (1664525*12345+1013904223) mod 2^32;
   an initial 87625868 hand-trace was caught as AUTHOR-ERROR pre-run and corrected from the formula).
I2/I3 fixture bounds; I4 adjust(a)=a+a/10 sample points; I5/I6 partition identities;
I7 adjusted bounds/order; I8 pipeline determinism; I9 slice goal post-checkpoint==CANONICAL.
Harness soundness PROVEN by large_control.shape (snapshot removed): ALL_CHECKS_PASSED, ec=0,
byte-identical VM/JIT -> the ONLY defect in large.shape is the snapshot() primitive (D1).

## book_gaps
- Neither chapter shows how to import the snapshot() FUNCTION (vs the Snapshot enum); the
  runnable=true examples assume bare snapshot() resolves from use { Snapshot } (it does not).
- The two chapters disagree on call syntax (bare snapshot() vs snapshot::snapshot()) with no note.

## book_wrong
- resumability.mdx "Snapshot API": first-pass contract not honored (aborts with Suspended on future).
- resumability.mdx "Function-Level Snapshotting" runnable=true example does not compile.
- snapshot.mdx "snapshot::snapshot()" call form errors ("module namespace 'snapshot' is not typed").

## VM vs JIT
All programs byte-identical on stdout. Snapshot programs whole-program-deopt to interpreter
([jit-fallback] on stderr) per the book's "VM vs JIT Note".
