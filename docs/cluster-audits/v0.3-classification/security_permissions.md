# security_permissions classification

**HEAD:** 82f049dd
**Total tests in binary:** 8
**Passed:** 6 / Failed: 2 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test security_permissions --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 1 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### compile_time::net_connect_denied_with_pure_permissions

Class: **FN-REG-CORRECTNESS**

```
Expected run error, but got: Some(Object {"Bool": Bool(false)})
```

- Minimal repro: a program that calls a net-connect stdlib symbol under a "pure" permission profile that should statically reject the load. Compile-time capability checking is returning `Bool(false)` (i.e. running and succeeding) instead of refusing.
- Bisect: not run (audit-only).
- Affected subsystem: compile-time capability checker (`shape-runtime::stdlib::capability_tags` + linker permission union). Either the net-connect stdlib call is no longer tagged with `NetConnect`, or the linker permission check has regressed. Plausibly-correct user-facing security guarantee — this is a release-blocker on security.

### compile_time::process_spawn_denied_with_pure_permissions

Class: **SCOPE-RECLAIM**

```
FromSlot<Vec<Arc<HeapValue>>>: V3-S5 ckpt-5-prime²c SURFACE — the polymorphic Vec<Arc<HeapValue>> marshal path needs a per-element-T dispatcher over the v2-raw *mut TypedArray<T> carrier. Round 2 `Vec<Arc<HeapValue>>` rewire follow-up (pairs with from_typed_array_<T> constructor wave). ADR-006 §2.7.24 Q25.A SUPERSEDED.
```

- Dated user pull-in: 2026-05-22 — W17.3-4 per-container FieldType + phase-2c host-tier marshal/snapshot rebuild explicitly pulled into v0.3.
- SURFACE text: cites "V3-S5 ckpt-5-prime²c" + "Round 2 Vec<Arc<HeapValue>> rewire follow-up" / "ADR-006 §2.7.24 Q25.A SUPERSEDED" — pure construction-cascade work.
- Incorrect anchor cited: the SURFACE does not name "v0.4" but is gating on V3-S5 ckpt-5 work that is in-scope per the 2026-05-18 user pull-in row.
- Why incorrect: V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade was pulled into v0.3 by 2026-05-18 disposition. SURFACE-and-stop on the marshal path is a SCOPE-RECLAIM, not a defer.
- Test asserts on user-facing semantics (`Expected run error` for process-spawn denial). Test stays the same after fix.
