# E5 CKPT-3 — record field-name preservation + record spelling — report

**Base:** `1d54eb67` (branch `adr009/e5`, worktree `shape-adr009-e5`, = CKPT-2).
**Scope:** B2 in-scope (records ENTER E5). **Additive — deletes nothing** (the
`.source` deletion stays CKPT-5). Single writer, append-only on `1d54eb67`.

## What landed (CKPT-0 mechanism)

Records now SPELL and REFLECT their field names. This is the last reconstruction
gap before the migrate+delete checkpoints — a structural record type-ref now
reconstructs → stamps → stops reparsing `.source`, the same auto-widen CKPT-1
gave applied generics.

### 1. STORE + POPULATE (the freeze fact) — invariant-respecting

- `RecordFieldDescriptor` (payloads.rs) grows `name: String` — a
  SPELL/REFLECT-ONLY freeze fact (doc'd as such; mirrors `ParamDescriptor.name`).
- Populated in `canonical_record` (type_reflection.rs) in the SAME rebuild that
  mints the record identity + the hygienic `member` identity, from the plain
  `name` already in the entry closure.
- **The identity descriptor string (`rendered`) and `record_member_identity` are
  BYTE-UNTOUCHED.** `name` is set on the descriptor value only, never threaded
  into either computation.

### 2. SPELL the Record arm (load-bearing)

`reconstruct_type_annotation`'s Record arm (comptime_builtins.rs) FLIPS from the
CKPT-1 interim named Err to building
`TypeAnnotation::Object({name: <recurse field type identity>}…)`:

- Field NAMES from the preserved `RecordFieldDescriptor.name`.
- Optionality PRESERVED per field (record identity is optionality-significant —
  `{x?:int} != {x:int}`; the `?` must survive).
- **A2 identity-indirected:** each field type recurses on its own finite frozen
  `type_identity` — a nested record spells its own fields, an applied/bare arg
  spells by head/name, never eager field-expansion. Terminates on the finite type
  expression.
- **Stamp-gate AUTO-WIDENS with no `stamp_for` edit:** `stamp_for` admits an
  identity iff `reconstruct(...).is_ok()`; the moment the arm reconstructs a
  record, the SAME predicate stamps it (E1-D7(b), one code path).

### 3. REFLECT ABI

- `COMPTIME_RECORD_FIELD_SCHEMA` (builtin_schemas.rs) + `record_field_slot`
  (payloads.rs) grow an additive `name` string field (`KindedSlot::from_string`,
  refcount-cloned into the object by the schema builder — the `nb_str →
  __ComptimeTypeInfo.name` pattern). Dec-55-class additive comptime-ABI field.
- The **type-checker view** of `RecordField` (`comptime.rs` `struct_item`, whose
  contract is "field names + order match the value carrier exactly") grows the
  matching `name: string` field — the ABI-ripple sync point required so
  `reflect(...).fields[i].name` type-checks + reads. (Bounded ripple: schema +
  value builder + type-checker mirror, all three the same field, same append
  position.)

## The binding invariant (CKPT-0 SAFE) — HELD with PROOF

The record IDENTITY + hygienic MEMBER identity strings stayed BYTE-IDENTICAL. A
permanent pin
(`e1_s5_reconstruction::e1_s5_ckpt3_record_identity_and_member_ids_are_byte_identical`)
asserts the concrete pre-CKPT-3 128-bit values captured on HEAD `1d54eb67`:

| fixture | identity (high, low) | fields (name → member high, low, optional) |
|---|---|---|
| `{x:int,y:string}` | `(4972967358956473603, -5404863359470070500)` | `x → (5117747860848310177, 1031105497090630829, false)`; `y → (-9035473693977959263, 304561787195158326, false)` |
| `{x?:int}` | `(-1802259954908786269, -200733891727391745)` | `x → (7472345934218968096, -929543014868829712, true)` |

The pin PASSES at workspace HEAD (post-CKPT-3) → field-name preservation added
ZERO information to the identity/member algebra. The `{x?:int}` identity is
DISTINCT from `{x:int}` (optionality-significant), also pinned. The existing
`record_identity_is_field_name_sorted_and_optionality_significant`
descriptor-string pin is a second independent guard on the identity bytes.

## Pins (all green)

Unit (`mod e1_s5_reconstruction`, plain `#[cfg(test)]` → runs under the standard gate):

- `e1_s5_ckpt3_record_identity_and_member_ids_are_byte_identical` — the
  NON-PERTURBATION invariant pin (concrete pre-CKPT-3 bytes).
- `e1_s5_ckpt3_record_spells_names_and_optionality` — `{x:int,y:string}` and
  `{x?:int}` round-trip to their `Object` spelling (names + `?` preserved,
  byte-sorted).
- `e1_s5_ckpt3_record_with_nested_record_and_applied_field_terminates` — A2:
  `{inner: {a: int}, items: Array<int>}` spells its nested record + applied field
  and TERMINATES.
- `e1_s5_ckpt3_stamp_gate_predicate_auto_widens_for_records` — the shared
  `reconstruct(...).is_ok()` predicate now ADMITS records.
- `e1_s5_reconstruct_covers_frozen_payload_descriptor_totally` (case 2) FLIPPED —
  record now SPELLS (was a named rejection).

e2e (`tools/shape-test/tests/comptime/reflect.rs`, dual-engine VM+JIT):

- `record_reflects_field_names_ckpt3` — `reflect(type_ref({beta:int, alpha?:string}))`
  exposes `fields[0].name`/`fields[1].name` = `alpha`/`beta` (byte-sorted) → prints
  `alpha:beta`.
- `record_reflects_normalized_fields_with_optionality` (kept green) — the extra
  `name` field does not disturb `.optional` / `.len()` reads.

## Gate (FAILED-name sets vs Step-0 baseline)

| Gate | Baseline (`1d54eb67`) | After CKPT-3 | Verdict |
|---|---|---|---|
| `shape-vm --lib` | 3584 pass / 6 fail (stable-6) | 3587 pass / 7 fail | +3 net pass; DETERMINISTIC FAILED set = stable-6 EXACTLY; the 7th (`route_tests::nested_exact_calls_close_outer_arguments_before_inner_compilation`) is the documented FLAP |
| `-- e1_s5` | 25 / 0 | 29 / 0 | +4 CKPT-3 pins, all green |
| `-- no_json` | 2 / 0 | 2 / 0 | unchanged |
| comptime (`--test-threads=1`) | 270 / 3 (3 named) | 271 / 3 | +1 e2e; FAILED set = the 3 named EXACTLY |
| `just check-clean` | exit 0 | exit 0 | green |
| `just check-no-dynamic` | clean | clean (exit 0) | no dynamic path added |

**Flap proof** (`nested_exact_calls_close_outer_arguments_before_inner_compilation`):
its outcome is test-ordering/global-state dependent, INDEPENDENT of this diff —
- CLEAN HEAD, isolation (`--exact`) → PASSES;
- CLEAN HEAD, `route_tests` module-together → FAILS;
- WITH CKPT-3, `route_tests` module-together → PASSES;
- WITH CKPT-3, full `--lib` (multithreaded) → FAILS.
The deterministic stable-6 FAILED-name set is byte-identical baseline↔after
(diff = empty). Family: `project_bulk_test_hang` / monomorphization route fixtures.

3 pre-existing comptime failures (unchanged): `annotations::b6_annotation_iterates_
callable_parameters_on_vm_and_jit`, `annotations::b6_annotation_reads_callable_param_
modes_on_vm_and_jit`, `callable::hash_tracer_does_not_disturb_formatted_strings`.

## Forbidden-Patterns / anti-walk-back posture

- **NO `.source` touch.** Grep of the diff: every `.source` / `stamp_for` /
  `reparse` hit is a COMMENT or the interim rejection message string that flips to
  spelling — zero machinery lines. `parse_type_annotation_payload` / `__type_probe`
  untouched (CKPT-5 territory).
- **Additive.** One new descriptor field + one arm flip + two schema/mirror field
  appends + pins. No sealed-enum variant (A1). No dynamic/tag/decode path.
- **Loud failure preserved.** A record that cannot spell can only be one whose
  field type cannot reconstruct — surfaced by the recursive `?` propagating that
  field type's named rejection. Never a silent reparse.
- `just check-no-dynamic` exit 0. (Advisory: the "closure capture deletion
  progress" counter reads 4 vs stale baseline 12 — a pre-existing decrease
  unrelated to CKPT-3, which touches zero closure-capture code; the recipe still
  succeeds.)
