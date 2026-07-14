# Wave 26 State Diff/Patch Targets

Date: 2026-07-09

Scope: supervisor target note for the `state.diff` / `state.patch` lane after
Wave 25.

## Current Manifest

Source:
`/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`

- Generated: `2026-07-09T22:09:25.488Z`
- Total snippets: 745
- Runnable snippets: 536
- Disabled snippets: 209
- Deferred snippets: 0

## Direct Rows

The first rows that can plausibly move from this lane are:

- `stdlib/core/state.mdx:371`: `state::diff(before, after)` on a typed
  `Portfolio` object.
- `stdlib/core/state.mdx:391`: `state::patch(before, delta)` continuation.

These should flip only if the implementation can honestly represent the shown
domain. A narrower scalar/string replace-root implementation may justify a
rewritten current-capability example, but not the existing Portfolio field
delta text.

## Adjacent Rows

Rows that mention diff/patch but should remain disabled unless broader surfaces
land:

- `stdlib/core/state.mdx:399`: diff/patch over transport.
- `stdlib/core/state.mdx:507`: `capture_module` plus diff-based sync.
- `advanced/content-addressed-bytecode.mdx:226`: broad Portfolio example using
  many state APIs.
- `advanced/content-addressed-bytecode.mdx:396`: migratable annotation
  pseudocode using diff.

## Honest Criteria

- Register or otherwise expose a real `Delta` carrier before returning it from
  `state.diff`.
- `state.patch(base, delta)` must validate that the delta came from the
  supported carrier shape.
- Identical values should produce a no-op delta; changed values should patch
  back to the new value in the supported domain.
- Unsupported value kinds must surface with a clear diagnostic naming the kind
  or domain, not fabricate `Any`.
- Typed-object field deltas are valuable only if patching preserves schema and
  field kind/heap-mask invariants.

## Outcome

Wave 26 closed the bounded scalar/string slice, not the original typed-object
Portfolio surface.

- `state.diff` now returns a schema-backed `Delta` typed object for homogeneous
  root replacement over `int`, `number`, `bool`, and `string`.
- Identical values produce an empty no-op `Delta`; changed values store the new
  value at `Delta.changed["$"]` with empty `Delta.removed`.
- `state.patch` validates the `Delta` schema id, rejects non-empty `removed`,
  rejects non-root/multi-path deltas, and returns the supported scalar/string
  value.
- Typed objects, arrays, maps, field/path deltas, removed paths, and
  heterogeneous deltas remain active implementation gaps.
- The public compiler needed an explicit `Delta` entry in the post-inference
  `FieldType::Any` whitelist because the carrier uses `HashMapKindedRef` plus
  `TypedObjectStorage::field_kinds` for the concrete value kind.
- The state book diff/patch snippets were rewritten to current scalar/string
  root replacement behavior and flipped runnable; the transport and broad
  content-addressed examples stayed disabled.

Verification:

- `run-p290748-i30923408.service`: `state_diff` 4/0.
- `run-p291995-i30924703.service`: `state_patch` 1/0.
- `run-p292114-i30924828.service`: state builtins 33/0/1 ignored.
- `run-p294738-i30927597.service`: `Delta` verifier whitelist test 1/0.
- `run-p297883-i30930892.service`: extracted 745 total / 538 runnable / 207
  disabled / 0 deferred.
- `run-p299900-i30933083.service`: release slice-B book gate 232/232.
- `run-p332249-i30965551.service`: full release book gate 538/538, report
  `/tmp/shape-wave26-book-truth-report.json`.
