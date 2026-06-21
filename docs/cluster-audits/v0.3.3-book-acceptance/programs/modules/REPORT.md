# Book-Acceptance Report — slice: modules

Book chapter (PRIMARY source): fundamentals/modules.mdx
Binary: target/release/shape (HEAD, not rebuilt). Runs ulimit -v 12 GiB + 30s timeout.
Determinism: pure (filesystem import/export/use/from..use; multi-file in slice dir).

## Summary
| Program | VM ec | JIT ec | stdout byte-identical | result |
|---------|-------|--------|-----------------------|--------|
| small (111 LOC: main + 3 mathx files) | 0 | 0 | yes (ALL_CHECKS_PASSED) | PASS |
| large (849 LOC, 4 files, 121 asserts) | 0 | 0 | yes (ALL_CHECKS_PASSED) | PASS |

Slice classification: PASS. All documented module mechanics work under VM and JIT.

RE-RUN AT CURRENT HEAD (2026-06-20, strict-flip-collection-dispatch worktree):
both deliverables PASS again under VM and JIT, byte-identical stdout. ONE NEW
first-run-truth defect surfaced at HEAD and was worked AROUND (not in the module
system): the bare empty-array literal `[]` (`op_new_array(0)`) now hits a
surface-and-stop runtime error. It is the V3-S5 TypedArray-deletion WIP, fully
orthogonal to modules. Recorded under "Defect log" + book_gaps. The large program
originally used `[]` for empty-program edge cases; those were re-expressed via
`[0].slice(0,0)` (which works) so the module coverage is unchanged.

One in-chapter example is non-compiling, but its root cause is the numeric-conversion
rule (a different slice), not the module machinery — logged under book_wrong.

## small/ — import-forms tour
Files: small/main.shape (entry, top-level stmts) + mathx/stats.shape (clamp,gcd,sum_to)
+ mathx/geom.shape (pub type Rect, area, perimeter) + mathx/derived.shape (imports
mathx::stats internally). Exercises: named import, named alias (sum_to as triangular),
type import (Rect), namespace import (use mathx::stats -> stats::gcd(...)), cross-module
import inside a library file. Expected values derived from defs before running: clamp
piecewise; gcd Euclid (48,18)=6 / (17,5)=1 / (100,75)=25; sum_to(n)=n(n+1)/2 -> 55,1;
double_triangular(10)=110; Rect area/perimeter 24/20/25. VM=JIT=ALL_CHECKS_PASSED.

## large/ — RPN stack-machine on a 4-file module library
rpnvm::codes (opcode/status constants as pub fn + arity), rpnvm::ops (pure i64 prims),
rpnvm::machine (evaluator; imports BOTH ops and codes inside its function bodies),
main.shape (driver, 121 CHECK_FAILED guards, 14 sections). Module coverage: nested
rpnvm/ -> rpnvm::* filesystem mapping; from..use named imports; cross-module imports
inside library function bodies; analysis helpers imported into entry file + called
inline. All expected values hand-computed from RPN semantics before running (arith,
errors: underflow/overflow/div-zero/bad-op/empty, static analysis: max_depth /
count_arity / validate / literal fingerprints, i64-range programs e.g. 2^40,5!,7!).
VM=JIT=ALL_CHECKS_PASSED, stdout byte-identical. JIT emits one benign [jit-fallback]
line on STDERR (typed-object construction, W17-narrow-follow-up-A) then runs correctly
under the interpreter — does not affect stdout, so parity holds.

## book_gaps
1. No complete runnable multi-file program. The chapter shows only fragments
   (runnable=false snippets + a mean/variance library) — never a full main.shape +
   helper-file pair with literal data and reproducible output. A user must borrow
   Array<T> method idioms (.reduce(f,init), .sum(), `as number` on .length) from OTHER
   chapters to make the library compile. (Fell back to prior slice knowledge/reference
   to settle reduce callback-first ordering and the .length:int cast.)
2. Visibility of non-pub items undocumented. Chapter teaches pub for exports but never
   says whether a NON-pub item is importable. Empirically `from m use { secret_helper }`
   on a non-pub fn SUCCEEDS (ec=0) — pub is not enforced as an access boundary here.
   Book is silent, so a user can't tell if that's intended. (Not book-wrong: no claim
   is contradicted.)
3. Minimal shape.toml for `run` unspecified. Chapter shows [modules].paths but never the
   minimal config needed for `shape run main.shape` to resolve sibling mathx::* files.
   In practice plain `shape run` resolves filesystem modules relative to the entry file's
   dir with NO shape.toml — undocumented.

4. Silent stdlib shadowing of user module paths is undocumented (the mechanism behind
   book_wrong #2). The "Resolution Order" list does not state that a user filesystem
   module whose path matches an embedded stdlib module (e.g. `math::linalg`, `math::stats`)
   is silently overridden with NO diagnostic. There is no documented way to see which
   `math::*` / namespace prefixes are already claimed by the embedded stdlib, so a user
   cannot predict the collision. (Fell back to empirical probing to discover this.)

## book_wrong
1. The chapter's own Two-File Library Example (lines 160-168) does NOT compile:
     pub fn mean(v: Vec<number>) -> number { v.sum() / v.length }
   `v.sum()`:number / `v.length`:int is rejected — "Could not solve type constraints:
   number is not compatible with int" — because v0.3.3 strict numeric-conversion forbids
   implicit int<->number mixing in `/`. The book presents this as runnable library code;
   following it verbatim yields a compile error. Correct idiom: `v.sum()/(v.length as number)`.
   ROOT CAUSE is the numeric-conversion rule (separate slice, see
   project_numeric_conversion_rule.md), NOT the module system — module mechanics around it
   all work. Recorded so the book example gets fixed; module slice itself is PASS.
   (Also: example uses Vec<number> from std::core::intrinsics while the rest of the book
   uses Array<T>; the failing constraint reproduces identically with Array<number>, so the
   bug is independent of which name is used.)

2. The chapter's flagship "Two-File Library Example" (lines 145-181) uses the module
   paths `math::stats` and `math::linalg`. BOTH collide with EMBEDDED stdlib modules of
   the same name. A user who creates `math/stats.shape` + `math/linalg.shape` exactly as
   shown finds their files SILENTLY SHADOWED: `from math::linalg use { normalize }`
   resolves to the stdlib `math::linalg` (a vector module: add/sub/cross/normalize where
   `normalize` is L2-normalization), NOT the user's mean-centering `normalize`. Verified:
   the example prints L2-normalized output `[0.396.., 0.476.., ...]` instead of the
   mean-centered `[-1.2, 0.8, 0.3, 1.8, -1.7]` the source code implies, and leaks
   `V2 bytecode verification failed ... math::linalg::cross has no FrameDescriptor`
   diagnostics to stdout. The "Resolution Order" section (lines 80-87) lists stdlib
   above filesystem, but never warns that the book's OWN example paths are pre-occupied.
   A user following the chapter literally gets wrong results with no error. The book
   should either pick non-stdlib example paths or warn about the stdlib namespace.

## Defect log (first-run truth)
No language defect attributable to the MODULE SYSTEM. Every documented import form works
under VM and JIT (namespace use, namespace alias, named from..use, named alias, annotation
import, pub fn/type/annotation exports, nested filesystem paths, cross-module imports inside
library bodies).

D-EMPTY-ARRAY (NEW, surfaced on 2026-06-20 HEAD re-run; NOT a module defect):
the bare empty-array literal `[]` fails at runtime with
  "Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 ...
   per-T v2-raw TypedArray<T> flat-struct monomorphization ... lands at ckpt-6 STRICT close"
First-run truth, isolated minimally outside any module:
  fn count(xs: Array<int>) -> int { xs.length }   print(count([]))   -> ec=1, op_new_array(0)
Non-empty literals (`[1,2,3]`) and empty arrays produced by `.filter`/`.slice(0,0)`
all work (ec=0). So the defect is specifically the zero-length array-literal
construction path. The error message itself states the construction-site rebuild
lands at V3-S5 ckpt-6 — i.e. a known transient WIP checkpoint on this worktree,
classified V0.4-DEFER (orthogonal to the module slice). Worked around in large.shape
by deriving an empty Array<int> as `[0].slice(0,0)` (see main.shape:43-49). Module
mechanics around empty programs (run/validate/count_* over an empty token array)
all pass once the array is constructed via the working route.

A COLLISION TRAP also confirmed (see book_wrong #2 below): the book's flagship
"Two-File Library Example" uses module paths `math::stats` / `math::linalg`, which
collide with EMBEDDED stdlib modules of the same name. A user's local
`math/linalg.shape` is SILENTLY shadowed by the stdlib `math::linalg` (a vector
module with add/sub/cross/normalize); `from math::linalg use { normalize }`
resolves to stdlib's L2-normalize, not the user's mean-centering normalize, and
stray `V2 bytecode verification failed` diagnostics for `math::linalg::cross` etc.
leak to stdout. Deliverables therefore use non-colliding namespaces
(`mathx::*`, `rpnvm::*`).

A prior session's scaffolding asserted a "D5" restriction (imported call result can't bind
to let; imported call can't appear in an entry-file fn). RETESTED — FALSE: `let r =
clamp(15,0,10)` -> 10, and `fn wrap(x){ clamp(x,0,10) }` -> 10, both ec=0. The misleading
IDIOM NOTE in large/main.shape was corrected to a DESIGN NOTE (the res-cell convention is a
deliberate status+value out-param shape, not a forced workaround).

Self-author errors fixed during authoring (a real user would too): reduce(init,f) ->
reduce(f,init); alias `var` is a reserved keyword (renamed); annotated closure params + a
second positional arg parse-failed (used `let sq = v.map(...); sq.sum()` split form).
