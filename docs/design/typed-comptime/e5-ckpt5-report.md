# E5 CKPT-5 report — the `.source` reparse-fallback DELETION

**Branch:** `adr009/e5` · **Base:** `f5c51332` (CKPT-4) · **User ruling:** #61
(2026-07-24, Option 1 — delete the `.source` FALLBACK, preserve the item_fn parser
→ #88).

This is THE deletion the CLAUDE.md Forbidden-Patterns apparatus exists to protect:
a DYNAMIC-REPARSE FALLBACK removed TOTALLY. CKPT-1/2/3/4 made every reconstructable
type STAMP and every producer stamp-or-reject-LOUD, so the `.source` reparse arm
was already runtime-UNREACHED. CKPT-5 deletes it — no shim, no rename survives.

## The precise deletion (3 targets)

| # | Target | Location | State |
|---|--------|----------|-------|
| 1 | `.source` SCHEMA FIELD | `builtin_schemas.rs` `__ComptimeTypeRef` `.string_field("source")` | DELETED (offsets now name=0,kind=1,identity_high=2,identity_low=3; readers name-keyed) |
| 2 | PRODUCER EMIT | `comptime_target.rs` `build_type_ref_descriptor` `("source", nb_string(...))` | DELETED; param `source:&str` → `spelling:&str` (PRESERVED, load-bearing for name/kind) |
| 3 | REPARSE ARM | `comptime_builtins.rs` `type_annotation_from_string_or_type_ref_slot` `let source = string_field_from_typed_object(storage, &schema, "source")?; parse_type_annotation_payload(&source)` | DELETED; `if identity != INVALID { return reconstruct… }` collapsed to unconditional `reconstruct_type_annotation(overlay, identity)` |

### Deletion proof (grep, 0 hits in real code)

```
.string_field("source")          → 0   (crates/ bin/ tools/ extensions/)
("source", nb_string             → 0
&schema, "source")               → 0   (real code; sentinel has only assembled fragments)
```

### The `source → spelling` param rename (surfaced deviation)

The task text's literal wording said "drop the `source:&str` PARAM (from the
signature + all callers)." That was **mechanically impossible**: the param feeds
`type_ref_name_from_source` / `type_ref_kind_from_source`, which populate the
SURVIVING `name`/`kind` reflect-only fields — read by the U02 corpus (serde
`derive.shape`: `field.type_ref.kind`) and by the consumer's INVALID rejection
message. Dropping it would delete `name`/`kind`, which are out of CKPT-5 scope and
user-facing. It is NOT a renamed `.source` fallback (no field is stored for
reparse). Resolution: DELETE the `.source` field emit, RENAME `source` → `spelling`
(kills the walk-back-attractor name; documents "display spelling used ONLY to
derive name/kind, never stored for reparse"). Surfaced here rather than silently
done.

## Preservation proof (#88 — over-deletion is a REVIEW FAIL)

```
fn parse_type_annotation_payload  → present (comptime_builtins.rs:547)
fn __type_probe (snippet)         → present (comptime_builtins.rs:557)
return parse_type_annotation_payload(payload)  → present (bare-string item_fn arm, :637)
```

Over-deletion tripwire (pin (g),
`e1_s5_ckpt4_typeref_producers_stamp_invalid_rejects_loud_string_arm_surfaced`):
the bare-string arm parses BOTH `"int"` (leaf) AND — added this checkpoint —
`"Array<int>"` (applied generic → `Array`/`Generic`), proving the item_fn parser
survives. `item_fn(name, "Array<int>", value)` still parses.

## Pins rewritten (structurally-impossible walk-back)

- `e1_s5_stamped_unresolvable_ref_errs_through_full_consumer_never_reparses_valid_source`
  (primary anti-walk-back) — STRENGTHENED. Its trap (a stamped-unresolvable ref
  silently reparses `"int"`) is now impossible: no `.source` field, no arm. The
  pin proves the identity route Errs through the FULL consumer and now guards
  RE-INTRODUCTION of the deleted arm. First-arg `"int"` feeds only name/kind.
- `e1_s5_leaf_/composite_/applied_generic_identity_route_resolves_past_garbage_source`
  (a/b/f) — reframed: `"###unparseable###"` is now a garbage spelling feeding only
  name/kind; the identity route is the SOLE route (no fallback in existence).
- `e1_s5_ckpt4_unstamped_typeref_is_named_surface_and_stop_not_source_reparse` (d)
  — reframed: even a perfectly-parseable spelling (`"string"`) can NEVER reparse;
  the arm is deleted; locals `valid_source`/`garbage_source` → `valid_spelling`/
  `garbage_spelling`.
- `e1_s5_stamped_unresolvable_identity_is_named_semantic_error_no_fallback` (c) —
  UNCHANGED (no `.source` fixture; invariant holds before AND after).

**Shape-corpus `.source` FIXTURE found + rewritten:**
`tools/shape-test/tests/annotations_comptime/type_mutation.rs::
target_params_and_return_expose_type_refs` read `target.return_type_ref.source`
from USER Shape code (in a Rust raw string — missed by an initial `.shape`-only
grep; caught by the annotations_comptime gate going 117→116/11). Rewritten to
`target.return_type_ref.kind == "String"` (mirrors the param assertion above it).
Sole live `.source` reader outside the deleted arm.

## Sentinel extended (`no_json_comptime_protocol.rs`)

- Header note (c) FLIPPED: the "`__type_probe` source-reparse remainder SURVIVES
  (E1-D8 residual)" claim is now the CKPT-5 deletion note + the explicit "these
  needles do NOT ban the preserved item_fn parser" caveat.
- `no_source_field_on_comptime_type_ref_schema` — forbids re-intro of the
  `.string_field("source")` field (structural guard).
- `no_reparse_from_type_ref_source_field` — forbids re-intro of the
  `&schema, "source")` field-read arm.
- Both needles assembled from fragments (the sentinel never spells them
  contiguously); both green (0 hits) post-deletion.

## Docstrings corrected

CKPT-4 MEDIUM finding + collateral: `comptime_target.rs`
`build_type_ref_descriptor` / `stamp_for` / `build_field_descriptor_array` /
`from_module` / `to_nanboxed`; `comptime_builtins.rs` consumer comments +
`reconstruct_type_annotation` docstring + the `Parameter`-variant error MESSAGE
(was "an unstamped ref reparses .source") + STAGE-1/STAGE-2 historical headers;
`type_reflection.rs` reflection comment; `e5_spelling.rs` module docstring. None
now assert a LIVE `.source` fall-through.

## Gate table

| Gate | Baseline `f5c51332` | Post-CKPT-5 | Verdict |
|---|---|---|---|
| `shape-vm --lib` (parallel) | 3590 / 6 | 3591 / 7 | +2 pass sentinel; the +1 fail = `nested_exact_calls` FLAP (below) |
| `shape-vm --lib` (`--threads=1`) | — | run A 3592/6 · run B 3591/7 | same binary, serial → flaps 6↔7 = non-deterministic |
| `e1_s5` | 31 / 0 | 31 / 0 | green (rewritten pins pass) |
| `no_json` | 2 / 0 | 4 / 0 | green (+2 needles) |
| `comptime` | 271 / 3 | 271 / 3 | 3 named stay; ZERO flips |
| `annotations_comptime` | 117 / 10 | 117 / 10 | restored after type_mutation fix |
| `just check-clean` | exit 0 | exit 0 | green |
| `just check-no-dynamic` | success | success | green |
| `just verify-merge` | — | 15 / 0 | ALL CHECKS PASSED |

### The one FAILED-name delta: `nested_exact_calls` is a documented flap

`monomorphization::cache::route_tests::nested_exact_calls_close_outer_arguments_
before_inner_compilation` is KNOWN pre-existing flaky — the **CKPT-4 decisions gate
table already lists it** among the pre-existing comptime-unrelated `shape-vm --lib`
fails. It is non-hermetic (fails ALONE, cache-empty, 0 passed) and flaps run-to-run
even at `--test-threads=1` (two serial runs of the same post-CKPT-5 binary → 6 then
7). My only new tests (the 2 sentinel needles) are pure `.rs`-file scanners that
compile/run NO Shape code, so they cannot behaviorally affect the monomorphization
cache — they only reshuffle parallel test-scheduling of the non-hermetic
route_tests. Comptime blast radius (e1_s5, no_json, comptime, annotations_comptime)
is 100% green. NOT a regression.

## Verdict

FINISHED. `.source` fallback DELETED (field + emit + arm); item_fn parser
PRESERVED (#88); walk-back structurally impossible; sentinel extended; gates green.
