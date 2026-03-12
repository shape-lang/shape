# Borrow Redesign Follow-up Plan

Status: in progress  
Last updated: 2026-03-11  
Primary reference: `shape/docs/vision/rfc-borrow-lifetimes-ergonomics-v1.md`

## Objective

Finish the remaining high-value work after the direct `let` / `let mut` / `var` ownership split and MIR-first borrow analysis landed.

The priority is not to add more syntax. The priority is to remove the remaining semantic boundaries that still make references feel less than fully first-class, then cash in the analysis work with better runtime representations.

## Current Boundaries

### Boundary A: Ref escape is still partial

- Local containers can hold references when the container stays local.
- Non-escaping closures can now capture references when the closure stays local.
- Reference returns now track parameter-root provenance plus field/index projections, but broader general outlives solving is still incomplete.

This is the biggest user-facing limitation because it prevents refs from crossing abstraction boundaries.

### Boundary B: Storage classes collapse at runtime

- The storage planner now computes `Direct`, `UniqueHeap`, `SharedCow`, and `Reference`.
- `UniqueHeap` and `SharedCow` still lower to the same `SharedCell` runtime path today.

This is mostly a performance and representation-fidelity gap, not a language-semantic gap.

### Boundary C: Property/place borrowing is not yet fully general

- The language is statically typed. There is no dynamic-type feature gap here.
- The real gap is that borrowing still requires the compiler to resolve the place into a concrete typed field/index path up front.
- Chained typed fields and index borrows work now.
- Remaining unsupported cases are place shapes the compiler/runtime do not yet resolve as borrowable typed places.

This is an expressiveness boundary in place resolution, not a dynamic-typing boundary.

### Boundary D: Task sendability is heuristic

- Detached-task checks now reject all refs and certain mutable-capture closures.
- This is still a heuristic approximation, not a principled sendability model.

## Priority Order

1. `#4` Full ref escape and outlives work
2. `#3` Real runtime split for `UniqueHeap` vs `SharedCow`
3. `#5` Expand statically-resolved place borrowing
4. `#8` Tighten sendability beyond the current heuristic

## Progress Update

### Implemented on 2026-03-11

- Track 1 / Patch Set 1.1 landed: MIR now records unified loan sinks for local containers, closure environments, returns, and task boundaries, and the solver derives sink errors from one post-solve path instead of sink-specific rejection loops.
- Track 1 / Patch Set 1.2 landed for non-escaping closures: refs captured by local closures are now accepted when the closure sink stays local; returned/escaping closures with ref captures still reject.
- Track 1 / Patch Set 1.3 partially landed: return summaries now carry parameter-root provenance and projection chains, return analysis merges compatible projected returns through one solver rule, and compiler lowering now treats ref-returning calls as raw refs with implicit auto-deref in ordinary value contexts. The implementation still intentionally limits interprocedural acceptance to single parameter-root summaries.
- Track 1 / Patch Set 1.4 landed: the old compiler-only return-contract path was removed, MIR/compiler/LSP now share return-reference-summary terminology, and inconsistent-return diagnostics now talk about borrowed origin and borrow kind instead of legacy contract wording.
- Focused regression coverage was updated for the new local-sink behavior: local array/object/enum/property/index ref storage is accepted, non-escaping closure capture is accepted, and returned local containers / returned closures with ref captures still fail.
- Verification for the landed slice: `cargo test -q -p shape-vm --lib`.

### Still pending

- Track 2 runtime/storage split for `UniqueHeap` vs `SharedCow`.
- Track 3 broader statically-resolved place borrowing work.
- Track 4 principled sendability summaries beyond the current heuristic.

## Track 1: Full Ref Escape and Outlives (`#4`)

### Goal

Make references fully first-class across local abstraction boundaries:

- refs can be stored in local aggregates when safe
- refs can be captured by non-escaping closures when safe
- refs can be returned whenever the solver can prove the origin outlives the return boundary

### Patch Set 1.1: Unify escape reasoning

Status: implemented

Replace the current special-case local-container relaxation with a single escape/outlives model shared by:

- local array/object/enum storage
- closure capture
- return flow

Code touchpoints:

- `crates/shape-vm/src/mir/solver.rs`
- `crates/shape-vm/src/mir/analysis.rs`
- `crates/shape-vm/src/mir/storage_planning.rs`
- `crates/shape-vm/src/mir/types.rs`

Work:

- Generalize the current `relax_local_container_errors()` logic into a post-solve escape validator.
- Classify each sink of a loan:
  - local container
  - closure environment
  - return slot
  - task boundary
- Evaluate outlives constraints against the sink instead of hard-coding sink-specific rejections.
- Keep existing B0003/B0004/B0007 style codes, but derive them from unified sink analysis.

Acceptance:

- Existing local-container relaxation tests remain green.
- A single loan sink framework drives aggregate-store, closure-capture, and return diagnostics.

### Patch Set 1.2: Closure capture region analysis

Status: implemented for non-escaping closures

Permit reference capture into closures that provably do not outlive the referenced place.

Code touchpoints:

- `crates/shape-vm/src/mir/lowering.rs`
- `crates/shape-vm/src/mir/solver.rs`
- `crates/shape-vm/src/compiler/expressions/closures.rs`
- `crates/shape-vm/src/executor/control_flow/mod.rs`
- `crates/shape-vm/src/executor/variables/mod.rs`

Work:

- Distinguish local closures from escaping closures in MIR facts.
- Model closure environment lifetime as a region/sink rather than a blanket escape.
- Allow capture of refs into closures whose environment remains within the borrow region.
- Preserve referent identity and write-through behavior for captured refs at runtime.

Acceptance:

- Non-escaping closure captures of `&x` and `&mut x` compile when the last use proves safety.
- Escaping closures still reject captures whose referents do not outlive the closure.
- Runtime tests confirm captured refs read/write the original referent.

### Patch Set 1.3: General return outlives solving

Status: partially implemented

Move beyond the current single-summary return path.

Code touchpoints:

- `crates/shape-vm/src/mir/solver.rs`
- `crates/shape-vm/src/compiler/helpers.rs`
- `crates/shape-vm/src/compiler/statements.rs`
- `crates/shape-vm/src/compiler/expressions/function_calls.rs`

Work:

- Track return provenance for reference values:
  - parameter-origin
  - local-origin
  - projected-origin (`field` / `index`)
- Allow return of a reference whenever the solver proves the returned origin outlives the function boundary.
- Keep the current return-reference summary as a fast path and tooling surface, not as the only legal return mechanism.
- Ensure all return paths agree on borrow kind and compatible provenance when required.

Acceptance:

- `return &param_field`-style cases can compile when the source outlives the call.
- `return &local` still fails.
- Multi-branch reference returns are accepted or rejected from one unified solver rule.

### Patch Set 1.4: Diagnostics and docs

Status: implemented

Code touchpoints:

- `crates/shape-vm/src/compiler/functions.rs`
- `tools/shape-lsp/src/diagnostics.rs`
- `docs/vision/rfc-borrow-lifetimes-ergonomics-v1.md`
- book docs under `shape-web/book/book-site/src/content/docs/`

Work:

- Stop telling users to always capture owned values for every closure case.
- Make diagnostics explicitly explain which borrowed origin / sink caused the outlives failure.
- Document the accepted vs rejected closure/return/container cases and the raw-ref plus implicit auto-deref return model.

Acceptance:

- Diagnostics reference the actual failing sink and origin.
- RFC/book examples match implemented closure and return behavior.

## Track 2: Runtime Storage Split (`#3`)

### Goal

Make `Direct`, `UniqueHeap`, and `SharedCow` mean different runtime things instead of collapsing `UniqueHeap` and `SharedCow` into the same `SharedCell` path.

### Patch Set 2.1: Add distinct runtime representations

Code touchpoints:

- `crates/shape-value/src/heap_value.rs`
- `crates/shape-value/src/heap_variants.rs`
- `crates/shape-vm/src/executor/variables/mod.rs`
- `crates/shape-vm/src/executor/control_flow/mod.rs`
- `crates/shape-vm/src/executor/printing.rs`

Work:

- Introduce a unique owned heap wrapper for `UniqueHeap`.
- Introduce an explicit copy-on-write wrapper for `SharedCow`.
- Keep `SharedCell` only for the cases that truly require shared mutable indirection.

Acceptance:

- `UniqueHeap` loads/stores do not route through `SharedCell`.
- `SharedCow` performs copy-on-write only when aliasing requires it.

### Patch Set 2.2: Lowering and opcode consumption

Code touchpoints:

- `crates/shape-vm/src/compiler/expressions/identifiers.rs`
- `crates/shape-vm/src/compiler/expressions/closures.rs`
- `crates/shape-vm/src/compiler/helpers.rs`
- `crates/shape-vm/src/bytecode/opcode_defs.rs`

Work:

- Drive load/store/boxing choices directly from the storage plan.
- Stop treating `UniqueHeap` and `SharedCow` as the same lowering decision.
- Add new opcodes only if the existing `BoxLocal` / `BoxModuleBinding` split becomes too overloaded.

Acceptance:

- Closure capture lowering preserves `UniqueHeap` vs `SharedCow`.
- Identifier loads use the planned storage class without silently degrading to shared-cell behavior.

### Patch Set 2.3: Performance validation

Add targeted regression tests and profiling checks:

- mutable closure capture of unique owned bindings
- aliased `var` mutation that should use COW
- simple owned pipelines that should avoid hidden shared-cell churn

Acceptance:

- No `SharedCell` allocation in simple uniquely-owned paths.
- COW behavior only appears when aliasing actually exists.

## Track 3: Expand Statically-Resolved Place Borrowing (`#5`)

### Goal

Allow borrowing for any place shape the compiler can resolve statically. This is not about dynamic typing. It is about broadening the set of statically-known places that lower into the unified MIR/runtime place model.

### Patch Set 3.1: Replace ad hoc typed-field resolution with a general place resolver

Code touchpoints:

- `crates/shape-vm/src/compiler/helpers.rs`
- `crates/shape-vm/src/compiler/expressions/property_access.rs`
- `crates/shape-vm/src/mir/types.rs`
- `crates/shape-vm/src/mir/lowering.rs`

Work:

- Introduce one compiler-side place builder that resolves:
  - root locals/module bindings
  - chained field projections
  - chained index projections
  - mixtures of field/index projections where statically known
- Make source borrow lowering and MIR place lowering use the same place builder where possible.

Acceptance:

- Borrow support is defined in terms of “statically-resolved place”, not a small list of AST forms.
- Remaining rejections are only for place shapes that truly cannot be resolved statically.

### Patch Set 3.2: Runtime support for richer projected refs

Code touchpoints:

- `crates/shape-value/src/heap_value.rs`
- `crates/shape-value/src/value_word.rs`
- `crates/shape-vm/src/executor/variables/mod.rs`

Work:

- Extend projected refs as needed for deeper place chains.
- Ensure reads, writes, printing, and method dispatch preserve auto-deref behavior through the richer projection model.

Acceptance:

- Nested projected refs behave the same whether used directly or through intermediate refs.

### Patch Set 3.3: Clarify non-goals

Do not add runtime string-key property borrowing or any “best effort” dynamic property borrow path.

The boundary should remain:

- accepted: statically-resolved place
- rejected: place not statically resolvable

## Track 4: Tighten Sendability (`#8`)

### Goal

Replace the current detached-task heuristic with a more principled sendability check while keeping the simple user-facing model.

### Patch Set 4.1: Explicit sendability summary

Code touchpoints:

- `crates/shape-vm/src/mir/solver.rs`
- `crates/shape-vm/src/mir/storage_planning.rs`
- `crates/shape-vm/src/compiler/functions.rs`
- `tools/shape-lsp/src/diagnostics.rs`

Work:

- Compute sendability from concrete properties:
  - contains refs
  - mutable closure capture
  - storage class requiring shared mutable state
- Use this summary for detached-task diagnostics instead of only checking mutable captures.

Acceptance:

- Detached-task errors explain why a value is non-sendable.
- Structured-task vs detached-task rules stay distinct and testable.

### Patch Set 4.2: Keep the surface simple

The language does not need a Rust-style trait surface for `Send`/`Sync` right now.

Non-goal for this phase:

- no user-visible `Send` / `Sync` trait syntax

Goal:

- enforce a principled internal sendability model and explain it in simple language-level terms.

## Test Plan

### Escape and outlives

- local array/object holding refs and never escaping: accepted
- local array/object later returned: rejected
- non-escaping closure capturing shared/exclusive ref: accepted when region allows
- escaping closure capturing ref: rejected unless origin outlives closure
- return of borrowed param projection: accepted when provenance is valid
- return of local borrow: rejected

### Storage split

- unique mutable capture uses unique runtime box, not shared-cell
- aliased `var` mutation uses COW behavior
- simple owned local pipeline avoids hidden shared indirection

### Place borrowing

- chained field borrow over statically known nested object
- field+index mixed place borrow where statically resolvable
- unresolved place borrow remains a compile error with explicit reason

### Sendability

- detached task rejects refs
- detached task rejects non-sendable closure environment
- structured task permits shared refs when no detached escape occurs

## Done Criteria

This follow-up is complete when:

1. Refs can cross local abstraction boundaries when the solver can prove safety.
2. `UniqueHeap` and `SharedCow` are distinct runtime behaviors.
3. Borrowable places are defined by static place resolution, not ad hoc AST cases.
4. Detached-task safety is enforced by a principled sendability summary.
5. The RFC and book describe the implemented behavior without aspirational mismatches.
