# stdlib_json classification

**HEAD:** 82f049dd
**Total tests in binary:** 17
**Passed:** 3 / Failed: 14 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test stdlib_json --no-fail-fast 2>&1`
**Log source:** `/tmp/audit_logs/stdlib_json.log` (audit-only; no new cargo runs)

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 14 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Group A — W17-marshal-return-arms SURFACE (14 tests)

All 14 failures share one SURFACE shape, varying only in the
`TypedReturn::Discriminant(N)` ordinal (N = 3 or 4) and the line number
in the fixture:

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip
surface — TypedReturn::Discriminant(N) container arm needs the per-arm
KindedSlot projection path (typed-Arc ResultData/OptionData/TypedObjectStorage
builders). Tracked as W17-marshal-return-arms follow-up. ADR-006 §2.7.4.
```

Class: **SCOPE-RECLAIM** (uniform — see per-test rows below).

- **Dated user disposition:** 2026-05-22 — "W16.2-J PHF-retirement + **W17.3-4
  per-container FieldType + phase-2c host-tier marshal/snapshot rebuild** + 6
  Known Constraints + doc-truth round." The SURFACE explicitly names
  `W17-marshal-return-arms` and `W17-snapshot-roundtrip` and the
  `TypedReturn::Discriminant(N)` container arms (`ResultData` / `OptionData` /
  `TypedObjectStorage`) — exactly the host-tier marshal/snapshot rebuild scope
  the 2026-05-22 disposition pulled into v0.3.
- **(Incorrect) v0.4 anchor cited by SURFACE:** "W17-marshal-return-arms
  follow-up." No dated re-disposition has moved W17-marshal work to v0.4
  per CLAUDE.md / TAXONOMY.md table. Per TAXONOMY.md §SCOPE-RECLAIM, SURFACE
  messages citing "follow-up" or "planned" without a dated v0.4
  re-disposition are MIS-CITES; the failure routes here, not to V0.4-DEFER.
- **Why the cite is incorrect:** "follow-up" is not a dated v0.4
  re-disposition. The 2026-05-22 W17.3-4 + phase-2c marshal/snapshot pull-in
  remains the most recent dated authorization governing this work; it places
  it inside v0.3 release-blocking scope.
- **Test asserts on user-facing semantics** (parse / stringify / roundtrip
  return values). Test stays the same after fix — only the runtime is
  expected to project the typed-Arc container arms instead of panicking.

## Per-test classification

### parse::json_parse_array
Class: **SCOPE-RECLAIM** — Discriminant(3) variant of the Group A SURFACE.
```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip
surface — TypedReturn::Discriminant(3) container arm needs the per-arm
KindedSlot projection path ... ADR-006 §2.7.4. (line 3)
```
Pulled-in by 2026-05-22 W17.3-4 + phase-2c marshal/snapshot rebuild;
SURFACE-cite "follow-up" is not a dated v0.4 re-disposition.
Test asserts on user-facing parse semantics.

### parse::json_parse_boolean
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Verbatim shape matches the
Group A excerpt above (line 3).

### parse::json_parse_nested_object
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Verbatim shape matches the
Group A excerpt above (line 3).

### parse::json_parse_null
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Verbatim shape matches the
Group A excerpt above (line 3).

### parse::json_parse_number
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Verbatim shape matches the
Group A excerpt above (line 3).

### parse::json_parse_object
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Verbatim shape matches the
Group A excerpt above (line 3).

### parse::json_parse_string_value
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Verbatim shape matches the
Group A excerpt above (line 3).

### stringify::json_roundtrip
Class: **SCOPE-RECLAIM** — Discriminant(3). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 4 in this fixture.

### stringify::json_stringify_array
Class: **SCOPE-RECLAIM** — Discriminant(4). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 4 in this fixture.

### stringify::json_stringify_boolean
Class: **SCOPE-RECLAIM** — Discriminant(4). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 3 in this fixture.

### stringify::json_stringify_null
Class: **SCOPE-RECLAIM** — Discriminant(4). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 3 in this fixture.

### stringify::json_stringify_number
Class: **SCOPE-RECLAIM** — Discriminant(4). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 3 in this fixture.

### stringify::json_stringify_pretty
Class: **SCOPE-RECLAIM** — Discriminant(4). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 3 in this fixture.

### stringify::json_stringify_string
Class: **SCOPE-RECLAIM** — Discriminant(4). Same Group A SURFACE / same
pull-in / same test-asserts-on-semantics. Line 3 in this fixture.

## UNKNOWN list

(none)
