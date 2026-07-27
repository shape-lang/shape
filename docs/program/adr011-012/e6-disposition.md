# E6 salvage / quarantine disposition

**Authority:** issue #111 (AUTHORITY-BASELINE), step 3 of
`docs/design/typed-comptime/adr011-012-execution-rulings.md` §"#90 authority
enactment — ten required steps".
**Date:** 2026-07-27.
**Governing rules:** ADR-011, ADR-012, and the CLAUDE.md Forbidden Patterns
section.

## What this document is

A durable per-commit disposition for every paused ADR-009 E6 commit, plus the
required tracker amendments that make the stale #20 / #22 / #83 work
unclaimable.

## What this document is not

It does not perform any tracker mutation and does not move, merge, cherry-pick,
rebase, or delete any commit or branch. §5 specifies the issue-body amendments
that a writer with tracker authority must apply; until they are applied, the
stale bodies remain live and this document is the only record that they are
superseded. Nothing here has been verified by a build or test run.

## 1. Branch identity

| Fact | Value |
|---|---|
| Branch | `adr009/e6` |
| Tip | `1cd789898218540e6f2436b859a119eff22205d0` |
| Merge base with `main` | `7e343c20b7f03d303ee35f613fc8a2e72278b78e` |
| Commits not on `main` | 13 |
| Checkout observed at | `/home/dev/dev/shape-lang/shape-adr009-e6` (read-only; not modified) |

None of the 13 commits is reachable from `main`. Every "landed" claim about E6
work refers to this branch only.

## 2. Disposition tally

| Disposition | Count | Commits |
|---|---|---|
| salvage | 3 | `f58a0d85`, `aa2b472a`, `5252b7d5` |
| rewrite | 3 | `7b847950`, `ef191907`, `d8dc3daa` |
| evidence-only | 4 | `4efcef09`, `63393d60`, `50cd99b3`, `1cd78989` |
| reject | 3 | `2c90159d`, `09801835`, `a7c4370c` |
| **total** | **13** | |

Disposition meanings, fixed here so later readers cannot re-interpret them:

- **salvage** — the change is independently valid under ADR-011/012 and should
  be re-landed substantially as written.
- **rewrite** — the requirement is legitimate and must survive, but the
  implementation binds to a carrier or intrinsic form the replacement
  architecture deletes; a named replacement ticket re-does it.
- **evidence-only** — nothing to land; specific findings, proofs, or rulings are
  retained as acceptance inputs to named tickets.
- **reject** — the change is a form ADR-011/012 forbids. It is not re-landed in
  any shape, under any rename. Independently valid findings inside it are
  retained separately.

## 3. Per-commit disposition

### `f58a0d85` — delete dead legacy `type_info(T)` + `implements`-string — **SALVAGE**

Deletes the `type_info(T)` reflection cascade, the `__ComptimeTypeInfo` schema,
and the `implements("T","name")` string builtin, and replaces the confinement
sentinel with a structural deletion sentinel
(`legacy_type_info_vocabulary_is_gone`) that fails the build if any deleted
definition form reappears.

Rationale: the deleted surfaces are exactly the name-selected intrinsic and
string-typed trait query ADR-011 forbids; the deletion and its anti-walk-back
sentinel are valid independently of which replacement architecture lands. This
is the "preserve independently valid deletion/identity evidence" case in step 3.

Caveat for the re-lander: this commit deliberately KEEPS `comptime_target.rs`
and the five descriptor schemas as "Tranche 2 / shared live infra". That
retention is not salvaged — `comptime_target.rs` is the universal target that
#110 deletes. Salvage the deletion; do not inherit the retention decision.

### `aa2b472a` — sync `check-no-dynamic` `capture_as_value` ratchet 12→4 — **SALVAGE**

A baseline lower-bound tightening recording deletion progress that already
happened. Architecture-independent, and losing it silently permits regression
back to 12. `main` still carries the loose bound: `docs/check-no-dynamic-baseline.txt`
reads `12` on `main` and `4` on `adr009/e6`.

### `5252b7d5` — `StringV2`/`DecimalV2` arms in the comptime-result materializers (#89) — **SALVAGE**

Highest-value salvage. `nb_to_literal` / `nb_to_expr` had a `NativeKind::String`
arm but none for `StringV2` / `DecimalV2`, so a `StringV2` slot fell through to
`slot.as_heap_value()` — reinterpreting `*const StringObj` bits as
`Arc::into_raw(Arc<HeapValue>)`, an ADR-006 §2.7.16 receiver-recovery violation
that landed on `HeapValue::Decimal` by garbage discriminator. The fix routes
`StringV2` through the existing `as_str()` accessor and gives `DecimalV2` its
own `DecimalObj::value` arm, removing an unsound reinterpretation rather than
adding a shim.

Standing risk this disposition exists to surface: **issue #89 is CLOSED, and its
only fix is this unmerged commit.** Verified at `main`: `nb_to_literal`
(`crates/shape-vm/src/compiler/comptime.rs:3601`) and `nb_to_expr` (`:3682`)
contain no `StringV2` or `DecimalV2` arm; the sole `StringV2`/`DecimalV2`
mention in that file on `main` is an unrelated comment at `:3924-3925`. The
unsoundness is live on `main`. Re-landing this commit is therefore not
contingent on any E6 architecture decision and should not wait for one.

The same commit also deletes a duplicate `__ComptimeAnnotationDescriptor`
registration whose surviving legacy `args: Array<Any>` row was shadowing (last
registration wins, `registry.rs:267`) the typed row. That half is entangled with
the rejected GAP-3 carrier and is subsumed by deleting the carrier outright;
salvage the materializer arms, not the schema bookkeeping.

### `7b847950` — GAP-1 spell/reflect-only source name on Field/Param descriptors — **REWRITE**

Requirement is legitimate: a generator needs a struct field's source name for
JSON keys and a callable parameter's name for prompts, and the commit is careful
— the name is read-only data, never threaded into the identity, member, or
callable pre-image, with R1 (no string selector) and R2 (no ordinal selector)
left in force and a byte-identity pin proving the 128-bit identities unchanged.

Why not salvage: the name is added to the E6 descriptor payload carriers
(`NominalFieldDescriptor`, `param_descriptor_slot`) that #110 deletes, and it is
populated from a pre-elaboration freeze. The fact must be re-expressed as a
field of the resolved typed reflection surface.

Retained acceptance cases → **#165 (TARGET-METADATA)**: field source name
reflects identically on VM and JIT; a callable-type parameter is honestly
anonymous (`Option<string>`, not an invented name); adding the name changes no
nominal/member/callable identity byte.

### `ef191907` — GAP-2 typed per-descriptor `TypeRef` accessors — **REWRITE**

Requirement is legitimate and the semantic is right: the accessors re-issue an
already-interned identity through the same minting path rather than fabricating
one, so `reflect(field.type_ref())` composes recursively and terminates on a
primitive leaf.

Why not salvage: the surface is delivered as four SOH-prefixed intrinsics
declared with empty parameter lists whose carrier is validated at run time.
ADR-011 requires an intrinsic to be selected by a resolved `IntrinsicId` from the
canonical catalog with its exact type contract validated — a polymorphic
intrinsic that defers validation to a runtime carrier check is the form being
deleted, regardless of how well it behaves.

Retained acceptance cases → **#92 (resolve one live intrinsic by catalog
identity)** and **#177 (INTRINSIC-INVENTORY)** for the intrinsic form;
**#165 (TARGET-METADATA)** for the accessor semantics: recursive composition
terminates, and a re-issued `TypeRef` identity is byte-equal to the stored one.

### `d8dc3daa` — CKPT-2 GAP-D field `optional` + unwrapped `type_ref` — **REWRITE**

Requirement is legitimate and carries a real finding: a wrapped `Option<T>`
identity does not reflect (fails `[C0001]`), so an optional field's stored
identity must point at the inner `T` while a separate flag carries the
optionality. Deriving both from the same `TypeAnnotation::is_option()` /
`option_inner()` call so they cannot desync is the right invariant.

Why not salvage: same carrier problem as GAP-1 — it extends
`NominalFieldDescriptor` and `COMPTIME_FIELD_DESCRIPTOR_SCHEMA`.

Retained acceptance cases → **#165 (TARGET-METADATA)**: optionality is
identity-insignificant (nominal/member/record identities byte-unchanged); the
optional field's type reference reflects to the unwrapped inner; flag and inner
derive from one source and cannot desync.

### `4efcef09` — GAP-1/2/3 implementation report — **EVIDENCE-ONLY**

`docs/design/typed-comptime/e6-gap123-report.md`. Retain as history. Its
architecture claims are superseded by ADR-011/012 and may not be cited as
current authority.

### `63393d60` — ratified E6 program-of-record (R-C1/R-C2 + 8-checkpoint plan) — **EVIDENCE-ONLY**

The 8-checkpoint plan is superseded by the ADR-011/012 ticket graph and is
unclaimable as a work plan. Two user rulings from 2026-07-25 are retained as
acceptance inputs, because they are semantic decisions independent of the
carrier design:

- **R-C1** — `@llm_tool` rejects a unit-return target by type, with no presence
  bit → **#171 (LLM-TOOL-CONSUMER)**.
- **R-C2** — enum derivation is in scope (`#87` + variant name + enum handler
  execution path) → **#170 (JSON-SCHEMA-CONSUMER)**, with #87 remaining the
  open enum-variant self-description ticket.

### `50cd99b3` — CKPT-1 ratification + standing 3-failure gate baseline — **EVIDENCE-ONLY**

A decision-doc record of a ratification and a test baseline taken against a
branch that will not land. Retain as history; the baseline numbers bind nothing.

### `1cd78989` — CKPT-2 byte-identity pin + dual-engine optional proof — **EVIDENCE-ONLY**

The pins themselves are valid identity evidence — hardcoded 128-bit
`nominal:U`, both `member:U:{id,email}`, and an equivalently-shaped
`record:{...}` identity, asserted byte-unchanged, plus a VM-and-JIT proof
asserting on the program return value rather than on captured output. No code
salvage: they pin carriers that are rewritten or rejected.

Retained as **method**, to be re-applied by the replacement slices: any
reflect-surface addition must carry a hardcoded byte-identity pin over the
pre-existing identities, and dual-engine proofs must assert on return values,
not on print capture (native JIT bypasses capture). → **#165**, **#170**.

### `2c90159d` — GAP-3 typed `AnnotationDescriptor { name, args: Array<string> }` — **REJECT**

Two independently disqualifying forms, both named verbatim in step 3:

1. **String-backed annotation carrier.** `args: Array<string>` with
   `render_annotation_arg` evaluating and rendering each annotation argument to
   a string (string→content, scalar→literal, identifier→name). The disciplined
   rejection of complex arguments (`NonScalarAnnotationArgument` rather than a
   `{expr:?}` Debug fallback) makes it an honest string carrier — but a string
   carrier is what ADR-012 replaces with typed elaboration.
2. **Pre-elaboration generator input.** `predeclare_struct_schema` snapshots the
   raw struct AST into `comptime_context_struct_defs` before the freeze barrier,
   and the freeze reads annotations from that AST snapshot. ADR-011's stage order
   forbids raw AST shape selecting semantic behavior.

Retained findings (not the implementation):

- The freeze-ordering fact — a Phase-1 snapshot is too late for the freeze
  barrier — is real and will recur in the replacement's stage ordering →
  **#170 (JSON-SCHEMA-CONSUMER)**, **#165 (TARGET-METADATA)**.
- The reject-don't-render rule for non-scalar annotation arguments → **#170**.
- The VM defect this commit surfaced is separately dispositioned under
  `5252b7d5` above.

### `09801835` — CKPT-1 GAP-A callable-target "bridge" (`target.signature()`) — **REJECT**

Extends the universal `__ComptimeTarget` carrier: `to_nanboxed` grows two
additive identity-half slots (8 → 10) so a handler can recover its target's
signature. The universal target is on the CLAUDE.md forbidden list, and the
commit describes itself as a bridge into the very surface the replacement
deletes. That the identity is canonicalized through the shared path and pinned
byte-identical makes it a well-built instance of a rejected form, not an
exception to it.

Retained acceptance case → **#147 (ARGUMENT-PACK)** and **#97 (run one ordinary
around transform with affine `Next`)**: an interception handler must be able to
see its target's exact signature — in the replacement it comes from the typed
`Sig` of `ArgumentPack<Sig>` / `Next<Sig>`, not from slots on a universal
target. The dual-engine proof shape (arity, one parameter type category, return
category, asserted on VM and JIT) is a usable acceptance template.

### `a7c4370c` — update `target_to_nanboxed` structural test for +2 fields — **REJECT**

Sole content is the slot-count assertion 8→10 for the rejected `09801835` slot
extension. It has no independent value; it is rejected with its parent.

## 4. Re-land instructions for the salvage set

Ordering, and the reason for it:

1. `5252b7d5` first, and independently of everything else — it removes a live
   unsound reinterpretation on `main` and is not architecture-contingent.
   Re-land only the `nb_to_literal` / `nb_to_expr` `StringV2` and `DecimalV2`
   arms plus the `nb_string_v2_slot_renders_as_string_not_decimal` unit pin.
   Drop the `__ComptimeAnnotationDescriptor` de-duplication half (its carrier is
   rejected) and drop `field_description_annotation_arg_reflects_on_vm_and_jit`
   (it exercises the rejected GAP-3 surface).
2. `f58a0d85` — the deletion plus the structural deletion sentinel; do not carry
   over its Tranche-2 retention of `comptime_target.rs`.
3. `aa2b472a` — one-line ratchet tightening; land with or after (2).

Each is a normal reviewed change on a fresh branch. Cherry-picking from
`adr009/e6` is not required and drags rejected context; the content above is the
specification.

## 5. Making the stale tickets unclaimable

These three issues describe work in forms ADR-011/012 forbids. An agent picking
one up today would build a rejected architecture. The required tracker
amendments are specified here; applying them needs tracker authority this
document's author did not have (read-only GitHub access).

### #20 — "ADR009-E4: `HookPlan<Sig, State>` typed hook carriers; delete Any hook shapes (U13)"

State observed: OPEN, label `adr-009`, body blockers `#9 (B6)`, `#14 (C3)`.

Superseded because its central mechanism is `HookDecision` (Proceed/Return),
which ADR-012 replaces with ordinary typed `ArgumentPack<Sig>` and affine
`Next<Sig>` Callable Transforms; CLAUDE.md states plainly: do not extend
spelling-recognized `HookDecision`.

Required amendment: prepend a superseded banner naming ADR-012 and this
disposition, and redirect to **#147 (ARGUMENT-PACK)**, **#97**, **#148
(AROUND-SYNC)**, **#149 (AROUND-RECEIVER)**, **#109 (delete legacy runtime
annotation magic)**. Retain as acceptance cases in those tickets: the R1–R7
behaviour-parity matrix, and the deletion of the `("state", FieldType::Any)` /
`("event_log", Array<Any>)` carriers at
`functions_annotations.rs:694-698,2872-3279`.

### #22 — "ADR009-E6: Stdlib generator migration + `string_lit` deletion (U08/U14)"

State observed: OPEN, label `adr-009`, no blockers, `ready: false` in the
publication record (`special_dispositions."#22"`, mutation
`replace-blocked-by-section-only`, blocking `#23` retained).

Superseded because the body prescribes the rejected carriers directly:
`field.type == "string"` comparisons, `field.type_ref.kind` strings, untyped
`ann.args[0]`, and f-string `extend` source. An agent following it would rebuild
the string-backed surface #110 exists to delete.

Required amendment: superseded banner, and per-generator redirect —
`serde/serialize.shape` → **#107**; `serde/derive.shape` (`@json_schema`) →
**#170**; `llm/tools.shape` (`@llm_tool`) → **#171**; `@prompt` → **#108**; the
`string_lit` + `render_shape_string_literal` deletion
(`comptime_builtins.rs:1129-1148`) → **#106** (typed generated-item
construction) with **#169** (TYPE-CONSUMER-INVENTORY) owning the consumer sweep
and **#110** owning the final deletion.

Retained acceptance case, and it must not be lost: all four generators are
VM+JIT-proven today, so each replacement owes a **regression differential
against current behaviour on previously-working inputs**, not only new-path
tests.

### #83 — "E4-S5 `@remote` residual signatures & flavors: async, 0-ary, heterogeneous multi-arg"

State observed: OPEN, no labels.

The three *requirements* are valid and must survive. Every *fix* the body
prescribes is unclaimable, because each names machinery ADR-012 deletes:
`pseudo_tuple::substitute_remote_markers`, `build_impl_shadow_call`,
`specialize_polymorphic_decision`, `pseudo_tuple::build_remote_arg_pack`, and
the homogeneous `Array<T>` `serialize_arg_pack` arm.

Required amendment: superseded banner, keep the three cases as acceptance
criteria, delete the prescribed fixes, and redirect — async `@remote` → **#151
(AROUND-ASYNC)**; 0-ary `@remote` → **#147 (ARGUMENT-PACK)**; heterogeneous
multi-argument `@remote` → **#147**, with **#103** (rebuild remote as the
ordinary stdlib consumer) owning the composed result. The loud-reject discipline
(each unsupported flavor fails at compile time pointing at a tracked issue,
never a silent no-op) is retained as a requirement on those tickets.

### Also affected

- **#87** (enum-variant self-description) stays open on its own merits; R-C2
  pulled it into the E6 plan, and that plan is superseded, so #87 must not be
  described anywhere as "gating ADR-009 completion" through E6. Its acceptance
  content routes to **#170**.
- **#89** is CLOSED but unfixed on `main` — see `5252b7d5` above. It should be
  reopened, or a replacement filed, unless the salvage lands first.

## 6. What this disposition does not claim

- No commit was moved, merged, cherry-picked, reverted, or deleted.
- No issue was edited, labeled, commented on, or closed.
- No build or test was run; every code claim above is read from committed
  content at the SHAs named, and the `main`-side claims about `nb_to_literal` /
  `nb_to_expr` / the ratchet baseline were read from `main` at
  `cdb0be5de0f3aa346c99549f2beb8afa13918fd6`.
- The salvage set is a specification, not a verified re-land. Each re-landed
  commit owes its own gate evidence.
