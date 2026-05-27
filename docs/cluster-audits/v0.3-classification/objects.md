# objects classification

**HEAD:** 82f049dd
**Total tests in binary:** 24
**Passed:** 21 / Failed: 3 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test objects --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 2 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 1 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### operations::object_computed_key

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Semantic error: Assignment to 'obj.name'
requires compile-time field resolution. Generic runtime property lookup is
disabled.")
```

Minimal repro (`tools/shape-test/tests/objects/operations.rs:108`):
```shape
let mut obj = { name: "default" }
obj.name = "Bob"
print(obj.name)
```

Plain static field assignment on a `let mut` object literal is rejected. Field
`name` is statically present on the TypedObject schema; this is not a computed
key despite the test's title. Any reasonable user expects `obj.name = "Bob"` to
work. Affected subsystem: bytecode compiler property-assignment path
(`crates/shape-vm/src/compiler/` — `SetProp` lowering / TypedObject
field-resolution). Bisect: `git log --oneline -- crates/shape-vm/src/compiler/`
(post-W16.2 PHF-retirement work most likely culprit).

### operations::object_destructuring_in_function

Class: **SCOPE-RECLAIM**

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`. Strict
typing requires both operands to have a known concrete type at compile time.
Add a type annotation to disambiguate.")
```

Repro (`operations.rs:59`):
```shape
fn sum_coords({ x, y }) { return x + y }
print(sum_coords({ x: 3, y: 7 }))
```

Dated user disposition: **2026-05-21 — "Object destructuring must fully
work."** TAXONOMY explicitly names this row as SCOPE-RECLAIM trigger.
SURFACE text cites no v0.4 anchor — it's a generic strict-typing error
because the destructured-param path doesn't propagate the call-site object
type into the `{ x, y }` binding kinds, leaving `x` and `y` as `unknown`.
Test asserts on user-facing semantics (output `10`), so the test stays the
same after fix.

### access::object_bracket_access_dynamic_key

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Some("Runtime error: TypeError: expected string
property name, got non-string key (line 4)")
```

Repro (`access.rs:65`):
```shape
let obj = { a: 1, b: 2, c: 3 }
let keys = ["a", "b", "c"]
print(obj[keys[1]])
```

Runtime TypeError on `obj[keys[1]]` where `keys[1]` is a string literal
element of an inferred `Array<string>`. The inner `keys[1]` is losing its
string kind on the way to the outer object-bracket-access opcode (likely
arriving as a raw element-slot without `NativeKind::String` stamp), so the
bracket-access handler rejects it. Affected subsystem: array-index opcode
kind-propagation into TypedObject bracket-access (intersects 2026-05-21
"Array<string> must work" pull-in but the failure is at runtime in a typed
access path, not a SURFACE — classifying as correctness regression).

## UNKNOWN list

None.
