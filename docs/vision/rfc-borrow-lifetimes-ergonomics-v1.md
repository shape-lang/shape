# RFC: Borrow/Lifetime Ergonomics v1 (`let mut`, first-class refs, Polonius-inspired checking)

Status: Draft  
Date: 2026-02-26  
Authors: Shape runtime/compiler team

## Summary

This RFC defines a new ownership/borrowing model that keeps Rust-grade memory safety while feeling lightweight in day-to-day code:

- `let` is immutable by default.
- `let mut` enables reassignment.
- `var` is accepted as an alias for `let mut` (onboarding ergonomics).
- Optional `auto_bind` mode allows Python-like `x = ...` to create a binding if none exists.
- References become first-class values (`let r = &x`, `let r = &mut x`).
- Borrow/lifetime analysis moves from lexical-slot checks to place-based, non-lexical, Polonius-inspired constraints.
- Tooling surfaces inferred type, mutability, and effective borrow mode clearly.

The model is strict where safety matters and ergonomic everywhere else via inference + diagnostics + code actions.

## Motivation

Current Shape has useful borrow safety but with important limits:

- `&` references are currently restricted to call-argument contexts.
- `mut` in variable declarations is parsed but not represented semantically.
- assignment to unknown names can create bindings implicitly, which creates ambiguity.
- borrow inference is heuristic and lexical; it is not yet place-based NLL.

Users should not have to fight a checker for common code, but they also should not lose static guarantees.

## Goals

1. Keep memory safety guarantees equivalent to Rust aliasing rules.
2. Make binding semantics obvious and beginner-friendly.
3. Make references first-class (`let r = &x` must work).
4. Infer as much as possible (types, mutability, borrow mode, end-of-borrow points).
5. Provide deterministic, actionable diagnostics and LSP hints.
6. Allow gradual migration from existing Shape behavior.

## Non-goals

- Introducing explicit user lifetime parameters in v1 (`'a`) syntax.
- Reproducing every advanced Rust ownership feature in one release.
- Allowing unsound runtime fallback for memory safety.

## Language Design

### 1. Binding Forms

`let` and `let mut` are canonical:

```shape
let x = 1          // immutable binding
let mut y = 1      // mutable binding
```

`var` is accepted as sugar:

```shape
var y = 1          // exact alias for: let mut y = 1
```

`const` remains a stronger immutability form for compile-time constants.

### 2. Assignment Semantics

Assignment (`name = expr`) never shadows.

- If `name` resolves to an existing binding in lexical scope chain, assignment updates it.
- If `name` does not resolve:
  - `auto_bind = off` (default in strict mode): compile error.
  - `auto_bind = on`: create `let mut name = expr` in current lexical scope.

This resolves shadowing ambiguity while allowing Python-style flow when explicitly enabled.

### 3. Reference Expressions Are First-Class

Reference expressions are valid anywhere expressions are valid:

```shape
let a = 10
let r = &a         // shared reference
let mut b = 20
let rm = &mut b    // exclusive reference
```

References can be passed, stored, and returned only when lifetime constraints prove safety.

### 4. Function Parameter Reference Syntax

Parameters support explicit modes:

```shape
fn read(&x: Vec<int>) -> int { x.len() }      // shared
fn write(&mut x: Vec<int>) { x.push(1) }      // exclusive
fn inferred(x) { ... }                        // inferred by solver
```

Inference remains available for untyped parameters, but explicit syntax is authoritative.

### 5. Reference Type Surface

Reference types are represented explicitly:

- `&T` shared reference
- `&mut T` exclusive reference

Tooling must always show the effective mode, whether explicit or inferred.

## Static Semantics (Exact Rules)

### A. Binding Mutability

- `let` bindings cannot be reassigned.
- `let mut`/`var` bindings can be reassigned.
- Mutability of a binding is independent from mutability of a referenced target.

### B. Borrow Rules

For each place `P` (identifier / field projection / index projection):

1. Any number of shared loans may coexist.
2. At most one exclusive loan may exist.
3. Shared and exclusive loans cannot overlap.
4. Writing `P` requires no active loan conflicting with write.
5. Reading `P` through owner path is disallowed while `P` has an active exclusive loan.
6. Reborrowing is allowed if derived loan is a subtype of existing permissions.
7. Moving a value is disallowed while any active loan of that value/place exists.

### C. Lifetime Rules

Lifetimes are inferred as regions over CFG points:

- Loan starts at borrow expression.
- Loan ends at last use (non-lexical lifetimes).
- A reference cannot outlive its referent region.
- Escapes (return/store/capture) are allowed only when outlives constraints hold.
- Closure capture of references is allowed only when solver proves non-escaping and region-safe; otherwise emit escape diagnostic.

### D. Place Granularity

Borrow tracking is place-based, not slot-only:

- Borrowing `x.a` does not automatically lock all of `x` if projections are proven disjoint.
- Borrowing `x[i]` is conservatively treated as overlapping with other index writes unless proven disjoint.

### E. Drop Interaction

- Dropping a value with active loans is illegal.
- Existing reverse-order drop guarantees are preserved.
- Early exits (`return`, `break`, `continue`) emit drops after borrow validity checks.

## Effect Tiers and Concurrency

Safety is always enforced; strictness of accepted patterns is effect-tiered:

1. `local` effect: single-task scope, no cross-task sharing.
2. `task` effect: async task boundaries (`spawn`, `join` branches, shared captures).
3. `shared` effect: values crossing task/thread boundaries.

Rules:

- In `task/shared` effects, static borrow proof is mandatory (no dynamic borrow fallback).
- Cross-task references require `Send`/`Sync`-like trait constraints (or Shape equivalent).
- Non-`'static` references may not cross detached task boundaries.

## Inference Model

### Constraint Domains

The compiler solves three linked domains:

1. Type constraints (existing HM-like engine).
2. Mutability constraints (binding mutability + reference mode requirements).
3. Borrow/lifetime constraints (regions, loans, invalidations, outlives).

### Solver Architecture

Polonius-inspired relation set (conceptual):

- `loan_issued_at(loan, point)`
- `loan_killed_at(loan, point)`
- `origin_contains_loan_at(origin, loan, point)`
- `cfg_edge(p1, p2)`
- `subset(origin1, origin2, point)`
- `invalidates(point, loan)`
- `use_requires(point, loan)`

Fixpoint derives:

- active loans at each point,
- invalid borrow states,
- minimal required mutability,
- outlives violations.

### Inference Priorities

1. Respect explicit annotations (`&mut`, `let mut`, type annotations).
2. Infer minimal permissions needed by body usage.
3. Prefer shared (`&`) over exclusive (`&mut`) when both satisfy constraints.
4. Prefer immutable binding unless reassignment is required.

## Tooling Contract (LSP + Diagnostics)

### Inlay/Hover Requirements

For each binding/parameter, tooling should show:

- inferred type,
- binding mutability,
- effective reference mode.

Examples:

- `x: int`
- `y: mut Vec<int>`
- `p: &Trade`
- `q: &mut Trade`

### Borrow Visualization

Editor should expose borrow windows:

- start point (borrow creation),
- end point (last use),
- conflict point with primary + secondary notes.

### Code Actions

Required quick-fixes:

1. “Make binding mutable” (`let` -> `let mut`).
2. “Narrow borrow scope” (introduce block).
3. “Change inferred `&mut` to `&`” when writes removed.
4. “Insert explicit borrow mode”.
5. “Enable/disable auto_bind for this file/module” (if policy allows).

## Compatibility and Migration

### Language Flags

Introduce edition/config flags:

- `borrow_model = "v1"` (current behavior)
- `borrow_model = "v2"` (this RFC)
- `auto_bind = true|false`
- `var_alias = true|false` (whether `var` parses as alias)

### Breaking Changes in `v2`

- `let` reassignment becomes compile error.
- unresolved assignment no longer silently creates module/global binding unless `auto_bind = true`.
- reference expressions are legal outside call args, and their escape safety is checked by region solver.

### Migration Strategy

1. Add warnings under `v1` for behaviors that change in `v2`.
2. Provide codemod:
   - rewrite mutable `let` reassignment sites to `let mut`,
   - rewrite implicit-create assignments to explicit declaration or enable `auto_bind`.
3. Flip default to `v2` in next language edition.

## Implementation Plan

### Phase 0: Surface Syntax + AST

- Represent declaration mutability in AST (`VariableDecl.is_mut`).
- Add `&mut` in expression and parameter grammar.
- Keep `var` as parser sugar lowering to `let mut`.

### Phase 1: Binding Resolver Rewrite

- Separate declaration from assignment resolution.
- Add `auto_bind` policy at module/file compiler config.
- Disallow assignment-created shadowing.

### Phase 2: First-Class Reference Lowering

- Remove “call-arg only” restriction for `&`.
- Introduce IR node for borrow of place expressions.
- Preserve existing call-site implicit-borrow ergonomics as sugar.

### Phase 3: MIR + Place Analysis

- Lower to MIR with explicit places/projections and CFG points.
- Generate loan/invalidation constraints.

### Phase 4: Polonius-Inspired Borrow Solver

- Add Datalog-like fixed-point engine (or equivalent relation solver).
- Integrate with existing type solver pass.
- Emit deterministic error codes and conflict notes.

### Phase 5: LSP UX

- Extend inlay hints for mutability + borrow mode.
- Add borrow-window visualizations and new quick-fixes.

### Phase 6: Concurrency Tier Enforcement

- Add effect analysis for task boundaries.
- Enforce stricter borrow/send-sync rules at cross-task edges.

## Open Questions

1. Should `auto_bind` default to `true` in REPL but `false` in files?
2. Should `var` remain permanently or be onboarding-only sugar?
3. How aggressive should disjoint-index analysis be in v2 vs later versions?

## Acceptance Criteria

`v2` is accepted when:

1. `let a = &b` and `let a = &mut b` are supported with sound checks.
2. `let` immutability and `let mut` reassignment rules are enforced.
3. Place-based non-lexical lifetimes eliminate major false positives from lexical model.
4. LSP shows type + mutability + reference mode consistently.
5. Concurrency boundary checks enforce stricter guarantees without unsound fallback.
