# stdlib_crypto classification

**HEAD:** 82f049dd
**Total tests in binary:** 13
**Passed:** 8 / Failed: 5 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test stdlib_crypto --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 4 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### encoding::crypto_base64_decode

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip surface — TypedReturn::Discriminant(3) container arm needs the per-arm KindedSlot projection path (typed-Arc ResultData/OptionData/TypedObjectStorage builders). Tracked as W17-marshal-return-arms follow-up. ADR-006 §2.7.4.
```

- Dated user pull-in: 2026-05-22 — phase-2c host-tier marshal/snapshot rebuild + W17.3-4 per-container FieldType.
- SURFACE text: "W17-snapshot-roundtrip surface ... W17-marshal-return-arms follow-up".
- Incorrect anchor: SURFACE labels itself "follow-up" without citing a dated v0.4 re-disposition; W17 marshal work is in-scope for v0.3 per 2026-05-22.
- Test asserts on user-facing semantics (`Expected run ok` — base64 decode returning a `Result`); stays the same after fix.

### encoding::crypto_hex_decode

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip surface — TypedReturn::Discriminant(3) container arm needs the per-arm KindedSlot projection path ...
```

- Same SURFACE shape as crypto_base64_decode; same dated pull-in (2026-05-22 W17.3-4 / phase-2c marshal); same disposition.

### encoding::crypto_hex_roundtrip

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip surface ...
```

- Same SURFACE shape; same dated pull-in (2026-05-22); same disposition.

### encoding::crypto_base64_roundtrip

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip surface ...
```

- Same SURFACE shape; same dated pull-in (2026-05-22); same disposition.

### hashing::crypto_sha256_different_inputs_different_hashes

Class: **FN-REG-CORRECTNESS**

```
Semantic error: Cannot infer types for binary operation `NotEqual`: operand types are `unknown` and `unknown`. Strict typing requires both operands to have a known concrete type at compile time.
```

- Minimal repro: comparing two `crypto.sha256(...)` results with `!=` — both operands inferred as `unknown`. The sha256 builtin's return type is not being propagated through the inference engine.
- Bisect: not run.
- Affected subsystem: type inference + stdlib return-type annotation for crypto.sha256 (similar pattern observed across multiple tests where stdlib symbols return un-inferred types after strict-typing rollout). Strict-typing migration regression on plausibly-correct code.
