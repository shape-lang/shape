# stdlib_modules classification

**HEAD:** 82f049dd
**Total tests in binary:** 32
**Passed:** 8 / Failed: 24 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test stdlib_modules --no-fail-fast 2>&1`
**Log source:** `/tmp/audit_logs/stdlib_modules.log` (audit-only; no new cargo runs)

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 2 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 22 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Failure groups

**Group A — `set` module schema missing (11 tests).** Semantic-error SURFACE:
```
Semantic error: module namespace 'set' is not typed.
Missing module schema for export '<add|contains|difference|from_array|
intersection|new|remove|size|to_array|union>'
```
The `set` module's exports are not exposed via the per-container FieldType
module schema — exactly the "W17.3-4 per-container FieldType + phase-2c
host-tier marshal/snapshot rebuild" scope user-pulled-in 2026-05-22.
No dated v0.4 re-disposition has moved module-schema typing out of v0.3.
Class: **SCOPE-RECLAIM** for all 11.

**Group B — W17-marshal-return-arms SURFACE (11 tests).** Identical shape to
the stdlib_json Group A SURFACE:
```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip
surface — TypedReturn::Discriminant(N) container arm needs the per-arm
KindedSlot projection path (typed-Arc ResultData/OptionData/TypedObjectStorage
builders). Tracked as W17-marshal-return-arms follow-up. ADR-006 §2.7.4.
```
Pulled-in 2026-05-22 (W17.3-4 + phase-2c marshal/snapshot rebuild). "follow-up"
is not a dated v0.4 re-disposition. Class: **SCOPE-RECLAIM** for all 11.

**Group C — strict-typing `!=` inference loss (2 tests).** Semantic-error SURFACE:
```
Semantic error: Cannot infer types for binary operation `NotEqual`: operand
types are `unknown` and `unknown`. Strict typing requires both operands to
have a known concrete type at compile time. Add a type annotation to
disambiguate.
```
No W17/§5.16/v0.4 cite. Failure is the compiler refusing plausibly-correct
user code (`!=` on stdlib-returned values). Class: **FN-REG-CORRECTNESS**.
Affected subsystem: type-inference for `BinaryOp::NotEqual` on values
returned from `std::crypto::*`. Minimal repro shape (per fixture line 3):
`let a = std::crypto::random_bytes(n); let b = std::crypto::random_bytes(n); a != b`
(unknown vs unknown → inference can't ground the operand types).

## Per-test classification

### crypto_tests::crypto_ed25519_keypair_generation
Class: **SCOPE-RECLAIM** — Group B, Discriminant(1). Pulled-in 2026-05-22
(W17.3-4 + phase-2c marshal/snapshot). Test asserts on user-facing
keypair-generation semantics.

### crypto_tests::crypto_ed25519_sign_produces_signature
Class: **SCOPE-RECLAIM** — Group B, Discriminant(1). Same pull-in / same
test-asserts-on-semantics.

### crypto_tests::crypto_ed25519_sign_verify_roundtrip
Class: **SCOPE-RECLAIM** — Group B, Discriminant(1). Same pull-in / same
test-asserts-on-semantics.

### crypto_tests::crypto_ed25519_verify_wrong_message
Class: **SCOPE-RECLAIM** — Group B, Discriminant(1). Same pull-in / same
test-asserts-on-semantics.

### crypto_tests::crypto_random_bytes_unique
Class: **FN-REG-CORRECTNESS** — Group C. Compiler rejects `a != b` on two
`random_bytes(...)` returns: "operand types are `unknown` and `unknown`."
User-facing code that any reasonable user would expect to compile.
Affected subsystem: type-inference for `BinaryOp::NotEqual` against
stdlib return types. Bisect: `git log --oneline -- crates/shape-runtime/src/type_system/`
+ `crates/shape-vm/src/compiler/type_tracking.rs` around recent W17 +
strict-typing landings.

### crypto_tests::crypto_sha512_different_inputs
Class: **FN-REG-CORRECTNESS** — Group C. Same `NotEqual` inference loss
on `sha512(a) != sha512(b)`. Same affected subsystem; same bisect target.

### msgpack_tests::msgpack_encode_bytes_returns_result
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4). Same pull-in / same
test-asserts-on-semantics.

### msgpack_tests::msgpack_encode_decode_array
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4).

### msgpack_tests::msgpack_encode_decode_bool
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4).

### msgpack_tests::msgpack_encode_decode_number
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4).

### msgpack_tests::msgpack_encode_decode_string
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4).

### msgpack_tests::msgpack_encode_produces_hex_string
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4).

### msgpack_tests::msgpack_encode_returns_result
Class: **SCOPE-RECLAIM** — Group B, Discriminant(4).

### set_tests::set_add_duplicate
Class: **SCOPE-RECLAIM** — Group A. Missing schema for exports `add`, `size`.
Pulled-in 2026-05-22 (W17.3-4 per-container FieldType + module-schema
typing). Test asserts on user-facing semantics (`set.add` idempotence /
`set.size`).

### set_tests::set_add_item
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `add`, `size`.

### set_tests::set_contains_false
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`contains`.

### set_tests::set_contains_true
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`contains`.

### set_tests::set_difference
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`difference`, `size`.

### set_tests::set_from_array_dedup
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`, `size`.

### set_tests::set_intersection
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`intersection`, `size`.

### set_tests::set_new_empty
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `new`, `size`.

### set_tests::set_remove
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`remove`, `size`, `contains`.

### set_tests::set_to_array
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`to_array`.

### set_tests::set_union
Class: **SCOPE-RECLAIM** — Group A. Missing schema for `from_array`,
`union`, `size`.

## UNKNOWN list

(none)
