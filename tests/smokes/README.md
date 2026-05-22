# tests/smokes — workspace-wide language smoke fixtures

In-repo, git-tracked smoke fixtures for the s1–s5 canonical smoke matrix.
Migrated from out-of-repo `/tmp/smokes/` per Phase 4b Round 4 D ratify
(option ii) 2026-05-18 — retires the out-of-repo fixture-drift class
(Reading 6 META substrate: fixture changes between runs invisibly
affecting gate measurements).

## F' canonical smoke-gate harness (binding 2026-05-18)

The corrected per-mode harness shape:

```bash
out=$(timeout 30 ./target/release/shape run --mode $mode $file 2>&1)
ec=$?
last=$(echo "$out" | tail -1)
```

**Forbidden** (per imprecision #109 — masks SURFACE-and-stop exit codes):

- `out=$(... | tail -N); ec=$?` — captures the exit code of `tail`, not `shape`.
- `ec=${PIPESTATUS[0]}` — bash-only; non-portable; conflates pipe arrangement
  with capture-then-tail discipline.

Drive both modes by invoking the snippet above against each fixture with
`mode=vm` then `mode=jit`; compare `(last, ec)` for convergence.

## Fixtures

Each row lists the file, the canonical Shape source (semantics, not byte
form), and the expected `(last, ec)` for VM and JIT under the F'
canonical harness at branch HEAD `5d842283`.

| File | Source intent | VM `(last, ec)` | JIT `(last, ec)` | Status |
|------|---------------|------------------|-------------------|--------|
| `s1.shape` | scalar `for` loop, `sum += i` over `0..100` | `(4950, 0)` | `(4950, 0)` | PASS |
| `s2.shape` | typed-array `[1,2,3,4,5].map(|x|x*2).sum()` | `(30, 0)` | `(30, 0)` | PASS |
| `s2-oneliner.shape` | one-line equivalent of `s2` | `(30, 0)` | `(30, 0)` | PASS |
| `s3.shape` | UFCS-dispatch `impl HasX for Foo` returning `"x"` | `(x, 0)` | `(x, 0)` | PASS |
| `s4.shape` | `Set()` basics: `add("a"); add("b"); len()` | `(2, 0)` | `(2, 0)` | PASS (R7-F: `.size` deleted, `.len` only) |
| `s5.shape` | `Array<dyn HasX>` trait-object reproducer | SURFACE `op_new_array(2)` | SURFACE `op_new_array(2)` | SURFACE — W16.2-B target |
| `s5-kickoff-literal.shape` | kickoff prose Rust-syntax variant (`box(X{})`) | parse error | parse error | informational — NOT in canonical s1-s5 matrix |

`s5` SURFACE is the V3-S5 ckpt-5 consumer-cascade tier 3 surface (per
ADR-006 §2.7.24 Q25.A SUPERSEDED + W12-typed-array-data-deletion audit
§3.5 + §3.6). It is the **W16.2-B target**: preserve fixture as-is until
ckpt-6 STRICT close (after ckpt-5-prime + ckpt-5-prime² land). Refused on
sight: `TypedArrayData` resurrection under any rename (Refusal #1).

## Reproducer recipe (5 smokes × 2 modes = 10 results)

```bash
SHAPE=./target/release/shape
for s in s1 s2 s3 s4 s5; do
    for mode in vm jit; do
        file=tests/smokes/$s.shape
        out=$(timeout 30 $SHAPE run --mode $mode $file 2>&1)
        ec=$?
        last=$(echo "$out" | tail -1)
        printf "%s %s ec=%s last=%s\n" "$s" "$mode" "$ec" "$last"
    done
done
```

Expected output at branch HEAD `5d842283`:

```
s1 vm ec=0 last=4950
s1 jit ec=0 last=4950
s2 vm ec=0 last=30
s2 jit ec=0 last=30
s3 vm ec=0 last=x
s3 jit ec=0 last=x
s4 vm ec=0 last=2
s4 jit ec=0 last=2
s5 vm ec=1 last=Error: Runtime error: Not implemented: op_new_array(2): SURFACE ...
s5 jit ec=1 last=Error: Runtime error: Not implemented: op_new_array(2): SURFACE ...
```

## Discipline

- **Fixture-immutability** (Reading 6 META): never edit a fixture to make
  a gate pass. Surface drift to the user instead. Fixtures here are
  reproducible-by-construction inputs to the smoke matrix.
- **In-repo only**: do not re-introduce `/tmp/smokes/` references in new
  scripts, workflows, or audit close documents. Historical audit docs at
  `docs/cluster-audits/` reference `/tmp/smokes/` as the path used at
  audit time — those are immutable history and must not be rewritten.
- **Out-of-scope**: `s5-kickoff-literal.shape` is the original kickoff
  prose with Rust-shaped syntax (`fn name(&self)`, `box(X{})`); it
  does not parse under current Shape grammar and is preserved as
  historical context, NOT a member of the canonical s1-s5 smoke matrix.
