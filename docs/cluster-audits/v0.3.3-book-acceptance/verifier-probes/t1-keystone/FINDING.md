# T1-keystone adversarial verification (expr-type-table)

Verifier run against `target/release/shape` at branch `strict-flip-collection-dispatch`
HEAD `387c3739` (rebuilt). The keystone under test is `1685c177`
(`feat(strict-flip T1 KEYSTONE): post-solve per-expression type table`).

All shape runs: `ulimit -v 12582912` + `timeout 20`, both `--mode vm` and default(jit).

## Verdict: SOUND (keystone did NOT open an any-sink, did NOT regress a patched context)

### No any-sink for un-inferable exprs (table drops free-var entries, no Unknown-default)
- `UNINFERABLE_empty_array_x0_plus1.shape` (`let x=[]; x[0]+1`)  => compile-error, rc=1
- `UNINFERABLE_get_none_plus1.shape` (`get_none()` then `+1`)    => compile-error, rc=1
  - Same under `--mode vm` (rc=1).
- `adv_generic2` (`id<T>` instantiated; generic body var dropped post-solve; call-site
  concrete) => `id(5)+1.0` rejects `int != number`, rc=1.

### int != number preserved through every dispatch
- `REJECT_vec_int_plus_float.shape`  (`Array<int>` elem `+ 1.0`)            => reject, rc=1
- `REJECT_vec_number_plus_intvar.shape` (`Array<number>` elem `+ int var`) => reject, rc=1
- `REJECT_mapdispatch_int_plus_float.shape` (map-result into for-in `+1.0`) => reject, rc=1
- NOTE: `Array<number>` elem `+ 1` (int LITERAL) prints `2.0` — that is the documented
  lossless-literal-context adoption rule, independent of the keystone (verified: a non-literal
  int var rejects). NOT an any-sink.

### Patched contexts still work (table-first did not break them); c1/c2 fixed
- c1 map-into-for-in => 200 ; pop => 35 ; filter => 3  (keystone's own probes)
- c2 match-arm binder (`match m.get(k){Some(n)=>n+1}`) => 11
- string-method-return (`"hello".length()+1`) => 6
- function-return Phase-3e (`double(21)+1`) => 43
- closure-return (`|x:int|{x+100}` then `+1`) => 106
- VM==JIT byte-identical on all keystone probes.

### T2 StringV2-as-int: no bit reinterpret
- `string + int` and `string == int`(elem) both compile-error, rc=1. No reinterpretation.

### T3 struct-copy: no double-free
- `var b = a; b.x=100` => a.x=1, b.x=100 (independent); clean exit rc=0.

### type_inference suite blast-radius (parent 490d4ea1 vs HEAD)
- parent: 285 passed / 19 failed ; HEAD: 292 passed / 18 failed.
- **0 NEW failures** introduced by the keystone; **+1 fixed** (`test_complex_nested_array_flatten`).
- check-no-dynamic: exit 0 (no forbidden symbols introduced).

## PRE-EXISTING HOLE surfaced (NOT caused by the keystone — present at parent too)
`PREEXISTING_HOLE_stagef1_fieldread_anysink.shape`:

```
type Run { n: int }
let mut rs = []
rs = rs.push(Run { n: 1 })
let z = rs[0].n + 1   // prints 2 — should be a compile-error
```

The unit test `collections::stage_f1_unannotated_empty_push_accumulator_field_read_is_compile_error`
EXPECTS a compile-error but the program RUNS and prints `2`. Confirmed identical behavior at the
keystone PARENT `490d4ea1` (also prints `2`) => this any-sink is PRE-EXISTING, not a keystone
regression. The keystone's `PropertyAccess`-consult-exclusion does not close it because the
element type is recovered through a path independent of the table. Tracked for team-lead as a
live STAGE-F1 any-sink in the branch (release-relevant), separate from the T1-keystone change.
