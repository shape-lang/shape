# stdlib_regex classification

**HEAD:** 82f049dd
**Total tests in binary:** 16
**Passed:** 11 / Failed: 5 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test stdlib_regex --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 5 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 5 failures share one root: the host-tier marshal/snapshot
`project_typed_return` path SURFACEs on `Discriminant(7)`
(`TypedReturn::Discriminant(7)` container arm) and `Discriminant(8)`
(`ConcreteReturn::Discriminant(8)` arm), both citing the
**W17-marshal-return-arms follow-up** / **W17-snapshot-roundtrip** under
ADR-006 §2.7.4. Both `regex::match_all` (returns `Array<object>`) and
`regex::split` (returns `Array<string>`) flow through that arm.

Per TAXONOMY, the **2026-05-22 row** explicitly pulls into v0.3 the
"W17.3-4 per-container FieldType + phase-2c host-tier marshal/snapshot
rebuild" scope. The SURFACE cites only "W17-followup" / "W17-marshal-
return-arms follow-up" — neither is a dated re-disposition to v0.4. Per
the TAXONOMY §SCOPE-RECLAIM rule, SURFACE messages that cite a follow-up
anchor without a dated user re-disposition route to SCOPE-RECLAIM.

The 11 passing tests confirm the scalar-return regex path
(`is_match` returning `bool`, `replace`/`replace_all` returning `string`)
works end-to-end; the failures isolate to the `Array<T>` return marshal.

## Per-test classification

### basic::regex_match_all_multiple

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-
roundtrip surface — TypedReturn::Discriminant(7) container arm needs
the per-arm KindedSlot projection path (typed-Arc ResultData/OptionData/
TypedObjectStorage builders). Tracked as W17-marshal-return-arms
follow-up. ADR-006 §2.7.4. (line 3)
```

- Dated pull-in: 2026-05-22 (W17.3-4 per-container FieldType +
  phase-2c host-tier marshal/snapshot rebuild).
- SURFACE text: "project_typed_return: W17-snapshot-roundtrip surface
  — TypedReturn::Discriminant(7) container arm ... W17-marshal-return-
  arms follow-up. ADR-006 §2.7.4."
- Incorrect v0.4 anchor cited: "W17-marshal-return-arms follow-up"
  (no dated re-disposition to v0.4; same workstream named by the
  2026-05-22 pull-in).
- Why cite-as-SCOPE-RECLAIM: 2026-05-22 row binds the W17 marshal/
  snapshot rebuild as v0.3-gating; no later dated authorization
  re-dispositions it to v0.4.
- Test asserts on: user-facing semantics (`expect_run_ok`,
  `print(matches)`). Stays the same after fix.

### basic::regex_match_all_no_results

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-
roundtrip surface — TypedReturn::Discriminant(7) container arm needs
the per-arm KindedSlot projection path (typed-Arc ResultData/OptionData/
TypedObjectStorage builders). Tracked as W17-marshal-return-arms
follow-up. ADR-006 §2.7.4. (line 3)
```

- Dated pull-in: 2026-05-22 (W17.3-4 + phase-2c marshal/snapshot rebuild).
- SURFACE text: same `Discriminant(7)` container-arm cite.
- Incorrect v0.4 anchor cited: "W17-marshal-return-arms follow-up".
- Why cite-as-SCOPE-RECLAIM: 2026-05-22 row binds the workstream.
- Test asserts on: user-facing semantics. Stays the same after fix.

### operations::regex_split_by_comma

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-marshal-
return-arms residual — ConcreteReturn::Discriminant(8) arm has no
in-session KindedSlot projection. Tracked as W17-followup. ADR-006
§2.7.4. (line 3)
```

- Dated pull-in: 2026-05-22 (W17.3-4 + phase-2c marshal/snapshot rebuild).
- SURFACE text: "ConcreteReturn::Discriminant(8) arm has no in-session
  KindedSlot projection ... W17-followup."
- Incorrect v0.4 anchor cited: "W17-followup" (no dated re-disposition).
- Why cite-as-SCOPE-RECLAIM: 2026-05-22 row binds the W17 marshal/
  snapshot rebuild as v0.3-gating.
- Test asserts on: user-facing semantics (`expect_run_ok`).
  Stays the same after fix.

### operations::regex_split_by_whitespace

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-marshal-
return-arms residual — ConcreteReturn::Discriminant(8) arm has no
in-session KindedSlot projection. Tracked as W17-followup. ADR-006
§2.7.4. (line 3)
```

- Dated pull-in: 2026-05-22.
- SURFACE text: same `Discriminant(8)` arm cite.
- Incorrect v0.4 anchor cited: "W17-followup".
- Why cite-as-SCOPE-RECLAIM: 2026-05-22 row.
- Test asserts on: user-facing semantics.

### operations::regex_split_returns_array

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-marshal-
return-arms residual — ConcreteReturn::Discriminant(8) arm has no
in-session KindedSlot projection. Tracked as W17-followup. ADR-006
§2.7.4. (line 3)
```

- Dated pull-in: 2026-05-22.
- SURFACE text: same `Discriminant(8)` arm cite. Test calls
  `parts.length()` after `regex::split(...)`, expects `"3"`.
- Incorrect v0.4 anchor cited: "W17-followup".
- Why cite-as-SCOPE-RECLAIM: 2026-05-22 row binds the W17 marshal
  workstream.
- Test asserts on: user-facing semantics (`expect_output("3")`).
  Stays the same after fix.
