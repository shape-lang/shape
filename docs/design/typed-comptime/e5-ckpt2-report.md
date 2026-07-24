# E5 CKPT-2 — DESCRIPTOR substitution (Piece 1), A8-OUT — report

**Base:** `e57a8acd` (branch `adr009/e5`, worktree `shape-adr009-e5`, = CKPT-1).
**Ruling:** F1 = **A8 OUT** (arity-only), F2 = IN (bounded), F3 = IN, F4 owner:head,
F6 HashMap→Container. **Additive — deletes nothing** (the `.source` deletion is CKPT-5).

## What landed

`substituted_applied_nominal` (`type_reflection.rs`) now answers a COMPLETE,
non-fabricating `NominalDescriptor` for the two applicable-head families the struct
path did not cover, so `payload_of(applied builtin/enum head)` resolves instead of
returning `applied_nominal_pending_rejection`. `reflect(Array<int>)`,
`reflect(Option<int>)`, `reflect(Result<int,string>)` now complete — the reflect
asymmetry (applied struct substituted while applied builtin/enum pended) is gone.

### Branch A — builtin head (static template)

New `builtin_nominal_templates: HashMap<FrozenTypeIdentity, BuiltinNominalTemplate>`
on `FrozenTypeIndex`, populated in the SAME 11-head intern loop that already interns
builtin arity (one source, no second table). 9 containers (`Array` `Vec` `HashMap`
`Set` `Deque` `PriorityQueue` `Mutex` `Slice` `Future`) ⇒ `Opaque{owner: head}`;
`Option`/`Result` ⇒ arity-only `Enum{owner: head, variants}` with TRUE variant
names (owner-bound hygienic member identity) + arities. NO payload TYPE in the
descriptor — every applied argument is recovered via the orthogonal `type_argument`
query (A7-uniform).

### Branch B — user ENUM head (reuse-base)

A user enum head reuses the param-AGNOSTIC base arity-only enum descriptor
(`frozen_nominal_descriptors.get(head)` matched to `Enum`). SOUND under A8-OUT: member
identities + arities are name-derived (`T`-free), so the base head descriptor IS the
applied answer. Enum payloads recovered via `type_argument`.

### F2 — alias-of-applied (bounded; stayed bounded)

New `base_applied_nominals: HashMap<FrozenTypeIdentity, RefinedApplication>`, threaded
write-once in the alias fixpoint (guarded by `canonical_refine` → only genuine `applied:`
descriptors store). The base `Nominal` arm of `payload_for_identity` calls
`substituted_applied_nominal` before the pending rejection — lazy, symmetric with the
overlay memo arm. Covers `type Ints = Array<int>` (builtin) AND `type PageOfInt =
Page<int>` (struct). **F2 stayed within the bounded fixpoint + base-arm change; no
CKPT-2b split needed.**

### F3 — Phantom guard

The struct rebuild loop excludes a struct that declares NON-EMPTY generic parameters
from `frozen_nominal_descriptors`. The `is_empty` check is load-bearing (every struct
gets a `struct_generic_param_kinds` entry, empty for non-generic, so `contains_key`
alone wrongly excludes monomorphic structs — caught by the
`e1_s5_reconstruct_bare_user_nominal_spells_as_basic` regression during implementation,
then fixed). Both `payload_of` and `bare_nominal_name_of` read that map, so a
`Phantom<T>{tag:int}` head no longer reflects/spells as monomorphic — it stays the
named `unapplied_generic_head_rejection` (A3).

## Soundness posture (binding invariants — all upheld)

- **No branch fabricates a member type.** Containers ⇒ `Opaque` (no fields — none to
  mis-state). Option/Result/user-enums ⇒ `Enum` with TRUE names + arities. Every type
  ARGUMENT lives in `type_argument`, never invented into the descriptor.
- **owner: head** in every branch (verified in SP-1/2/3/6/7).
- **A2 identity-indirected** — descriptors carry `FrozenTypeIdentity`; SP-7 proves
  nesting terminates.
- **A1** — no `FrozenPayloadDescriptor::AppliedNominal` variant (grep-confirmed); one
  method-internal branch + one static table + one additive F2 fact.
- **`builtin_nominal_templates` is STATIC builtin data**, not a per-type freeze fact —
  the binding record/freeze invariant + identity/dedup computation are UNTOUCHED.
- **The named `applied_nominal_pending_rejection` STAYS the loud fallback** for any head
  neither builtin nor struct/enum-resolved.
- **SP-5 (A8-OUT):** `Result<int,string>` and `Result<string,int>` produce IDENTICAL
  descriptors; the swap is visible ONLY via `arg_identities` order. The enum-payload
  mis-type surface under A8-OUT is `type_argument` — pinned there.
- **`.source`/parse machinery UNTOUCHED** (grep of the diff: zero `.source` changes).

## Pins (spec §4, A8-OUT column)

Unit (`mod e1_s5_ckpt2_descriptor_substitution` in `type_reflection/tests.rs`, plus
SP-1 in `e1_s5_boundary`):

- **SP-1** (pin 3747 successor, POSITIVE flip) — `Array<int>` ⇒ `Opaque{owner:
  Array-head}`; element `int` recovered via `type_argument` + `reconstruct`.
- **SP-2** — `HashMap<string,int>` ⇒ `Opaque`; args `[string,int]` IN ORDER.
- **SP-3** (headline) — `Result<int,string>` ⇒ `Enum`, Ok/Err arity 1, name-bound
  member ids; payloads via `type_argument`.
- **SP-4** — `Option<int>` ⇒ `Enum`, None arity 0 / Some arity 1.
- **SP-5** (WRONG-DESC control) — swap descriptors EQUAL; visible only via `arg`
  ORDER.
- **SP-6** — user enum `Either<int,string>` reuses the base arity-only descriptor
  (Branch B); payloads via `type_argument`.
- **SP-7** — `Array<Result<int,string>>` ⇒ `Opaque{owner: Array}`; arg IS the Result
  applied id; `payload_of(arg)` ⇒ Enum; TERMINATES.
- **SP-8** — non-generic `Color{Red,Green(int),Blue}` descriptor unchanged.
- **A3+Phantom** — bare `Box<T>{value:T}` AND `Phantom<T>{tag:int}` both stay the named
  A3 rejection (+ don't spell monomorphic).
- **F2** — `type Ints = Array<int>` ⇒ Opaque via base arm; `type PageOfInt = Page<int>`
  ⇒ substituted Struct via base arm.

e2e (`tools/shape-test/tests/comptime/nominal.rs`, dual-engine VM+JIT):

- `reflect_on_applied_builtin_enum_generic_substitutes_to_arity_only_enum_on_vm_and_jit`
  (rewrite of the former pending-rejection e2e) — `Option<int>` ⇒ Enum, total 3.
- `reflect_on_applied_container_generic_is_opaque_on_vm_and_jit` — `Array<int>` ⇒ opaque.
- `reflect_on_applied_result_generic_is_enum_on_vm_and_jit` — `Result<int,string>` ⇒ enum.
- `reflect_on_unapplied_builtin_head_stays_the_named_rejection` (STAY guard).

## Gate (FAILED-name sets vs Step-0 baseline)

| Gate | Baseline | After CKPT-2 | Verdict |
|---|---|---|---|
| `shape-vm --lib` | 3574 pass / 6 fail (stable-6) | 3584 pass / 6 fail | +10 pins; FAILED set = stable-6 EXACTLY |
| `-- e1_s5` | 15 / 0 | 25 / 0 | +10, all green |
| `-- no_json` | 2 / 0 | 2 / 0 | unchanged |
| comptime (`--test-threads=1`) | 267 / 3 (3 named) | 270 / 3 | +3 e2e; FAILED set = the 3 named EXACTLY |
| `just check-clean` | exit 0 | exit 0 | green |
| `just check-no-dynamic` | clean | clean | no dynamic path added |

3 pre-existing comptime failures (unchanged): `annotations::b6_annotation_iterates_
callable_parameters_on_vm_and_jit`, `annotations::b6_annotation_reads_callable_param_
modes_on_vm_and_jit`, `callable::hash_tracer_does_not_disturb_formatted_strings`.
Stable-6 shape-vm failures unchanged (project_bulk_test_hang family + monomorphization
route fixtures).

## Deferred

Issue #87 — enum-variant self-description (payload TYPES in the variant descriptor +
the enum generic-param-name freeze fact). A8-OUT ships arity-only + `type_argument`
recovery; A8-IN is the richer packaging, deferred as a clean comptime-ABI slice.
