# Enhanced Escape Analysis v2: Interprocedural Ownership Inference

## Status (as of 2026-04-18)

**Alignment: 100% (landed; Phase 5.D superseded)**

- **Phase 5.A — Per-function `ReturnOwnershipMode` inference**: LANDED in `184a48e`.
- **Phase 5.B — Propagate return ownership hint to let-bindings**: LANDED in `15395d7`.
- **Phase 5.C — `ReturnOwned` opcode + caller-skip activation**: LANDED in `99a36a6`.
- **V0.b — Shared/Static escape emission**: LANDED in `e597e79`. Expanded the escape lattice so `var`-style and module-level slots are classified as Shared, and true constants are classified as Static — feeding the ownership solver's Mode selection.
- **Phase 5.D — Closure-capture storage-class tweak**: SUPERSEDED by `docs/v2-closure-specialization.md`. The narrow `SharedCow`-to-`Direct` change proposed in 5.D is replaced by the v2-native per-closure monomorphization + stack-allocation design. No action remaining under this document.

Interprocedural ownership information now flows through `CalleeSummaries` / `FunctionBorrowSummary` end-to-end: owned returns stay Box-backed across A→B→C pipelines, non-escaping closures keep captures owned, and escape-classified slots (Shared/Static) drive the correct allocation mode at emit time.

**V3-deferred items**: none in this document. Cross-references:

- Closure §14.7 residual producers → `docs/v2-closure-specialization.md`.
- Full out-of-scope list → `/home/dev/.claude/plans/i-want-a-complete-foamy-eich.md` §out-of-scope.

## Executive Summary

Shape's current escape analysis is **intraprocedural** — each function is analyzed in isolation. This means:
- Returns always allocate via `Arc<T>` even when the caller could use `Box<T>` immediately
- Function parameters always use liveness-based Move/Clone at call sites, but the callee doesn't know if it received an owned or shared value
- Closure captures escape conservatively when the closure could be inlined/specialized

Phase 5 extends ownership analysis **across function boundaries** via per-function summaries, enabling:
- **Returned values** to stay `Box`-backed when the caller immediately stores them in a `let` binding
- **Parameters** to be specialized for owned-vs-shared callsites via monomorphization
- **Local closures** (non-escaping) to keep captures as owned

## Current State (Baseline)

### What works today (intraprocedural)

- `detect_escape_status(slot)` computes `{Local, Captured, Escaped}` per slot within a function
- `BorrowAnalysis.ownership_decisions` picks `{Move, Clone, Copy}` per assignment
- `StoragePlan.slot_classes` picks `{Direct, UniqueHeap, SharedCow, Reference, Deferred}`
- `FunctionBorrowSummary` already exists for borrow-checking (conflict pairs, reference returns)
- `CalleeSummaries: HashMap<String, ReturnReferenceSummary>` flows through the solver
- Phase 3 `PromoteToOwned` wires let-bindings to Box-backed allocation

### What's missing (interprocedural gap)

| Gap | Example | Current behavior | Wanted behavior |
|-----|---------|------------------|-----------------|
| Owned returns | `fn make() -> Array<int> { [1,2,3] }` | Return wrapped in Arc | Return Box; caller owns immediately |
| Unique-param specialization | `fn push_all(arr, items)` called with owned `arr` | Clone arr at callsite | Move arr into callee |
| Non-escaping closure | `nums.map(\|x\| x * 2)` | Closure escapes via Array<Fn> | Inline and keep captures owned |
| Module-level pipelines | Function A → B → C returning arrays | Arc cascade | Box chain, zero atomic ops |

### Available infrastructure to leverage

- `FunctionBorrowSummary` in `crates/shape-vm/src/mir/analysis.rs:286` — already passed through solver
- `CalleeSummaries` type in `crates/shape-vm/src/mir/solver.rs:61` — already threads data across calls
- `BindingSemantics` struct in `type_tracking.rs:514` — ready to extend with return mode
- `ReturnReferenceSummary` — the proven pattern for interprocedural info

## Design

### Core concept: ReturnOwnershipMode

Each function gets a return-ownership classification:

```rust
pub enum ReturnOwnershipMode {
    /// Returns a newly-allocated owned value — caller gets Box<T>.
    /// Examples: `fn make() -> Array<int> { [1,2,3] }`
    NewlyOwned,
    
    /// Returns a reference/alias to a parameter — caller keeps ownership of source.
    /// Examples: `fn first(arr: &Array<int>) -> &int { &arr[0] }`
    BorrowedFromParam(usize),
    
    /// Returns a shared (Arc) value — reference-counted across callers.
    /// Examples: `fn get_singleton() -> Arc<Config> { ... }`
    Shared,
    
    /// Returns a value proven to escape into global/static storage.
    Static,
    
    /// Could not infer — falls back to Arc (current behavior).
    Unknown,
}
```

### Core concept: ParamPassMode

Each parameter gets a per-callsite classification:

```rust
pub enum ParamPassMode {
    /// Caller transfers ownership. Callee can mutate or consume.
    /// Emitted when: callsite is last use of the value (liveness).
    Move,
    
    /// Caller retains ownership, callee gets a borrow.
    /// Emitted when: parameter is declared `&T` or inferred as read-only.
    Borrow,
    
    /// Caller shares ownership via Arc. Default for `var` bindings and closure captures.
    Share,
    
    /// Caller deep-clones the value before call.
    /// Emitted when: callee is suspected to mutate (without `&mut`) and callsite isn't last use.
    Clone,
}
```

### How they compose

1. **Callee analysis** produces a `FunctionOwnershipSummary`:
   - Per-parameter: expected pass mode (what the function assumes about each param)
   - Return: `ReturnOwnershipMode`

2. **Caller compilation** uses callee summary to:
   - Match caller's value mode with callee's expected parameter mode (may insert conversion)
   - Propagate callee's return mode into the caller's storage plan

3. **Fixed-point iteration** until all module functions converge.

### Non-goals for Phase 5

- **NOT** introducing new syntax — `fn foo(arr: Array<int>)` still works
- **NOT** requiring user annotations — all ownership is inferred
- **NOT** breaking existing code — conservative fallback is current behavior (Arc everywhere)
- **NOT** implementing GC — that's Phase 6

## Implementation Phases

Phase 5 is broken into 5 sub-phases. Each is independently testable and can be committed separately.

---

### Phase 5.A: Extend summaries with return ownership

**Goal**: Every function gets a `ReturnOwnershipMode` computed. No behavior change yet — just data flowing.

#### 5.A.1: Define types

**Files**:
- `crates/shape-vm/src/mir/analysis.rs` — add `ReturnOwnershipMode` enum
- `crates/shape-vm/src/mir/analysis.rs:286` — extend `FunctionBorrowSummary`:

```rust
pub struct FunctionBorrowSummary {
    pub param_borrows: Vec<Option<BorrowKind>>,
    pub conflict_pairs: Vec<(usize, usize)>,
    pub return_reference_summary: Option<ReturnReferenceSummary>,
    pub return_ownership_mode: ReturnOwnershipMode,  // NEW
}
```

#### 5.A.2: Infer return mode from MIR

**New pass**: `infer_return_ownership_mode(mir: &MirFunction) -> ReturnOwnershipMode`

Algorithm:
1. Find all basic blocks ending in `Terminator::Return`
2. For each, trace back what slot gets assigned to `SlotId(0)` (return slot)
3. Classify based on the source:
   - `Rvalue::Aggregate { .. }` (array/struct literal) → `NewlyOwned`
   - `Rvalue::Call { .. }` → look up callee's return mode (recursive)
   - `Rvalue::Use(Operand::Copy(param_slot))` where `param_slot` corresponds to a parameter → `BorrowedFromParam(idx)`
   - `Rvalue::Use(Operand::Move(param_slot))` where param is moved → inherit param's incoming mode
   - Anything else → `Unknown`

If function has multiple return paths with different modes, use the "weakest" (Unknown wins).

**Location**: New file `crates/shape-vm/src/mir/return_ownership.rs` with a single entry function.

#### 5.A.3: Wire into compilation pipeline

**File**: `crates/shape-vm/src/compiler/functions.rs:844-892`

After MIR analysis, compute `return_ownership_mode` and store in `BorrowAnalysis.summary`.

#### 5.A.4: Tests

Add tests in the new `return_ownership.rs`:
- `fn make() -> Array<int> { [1,2,3] }` → `NewlyOwned`
- `fn get(arr: &Array<int>) -> &int { &arr[0] }` → `BorrowedFromParam(0)`
- `fn route(cond: bool, a: Array<int>, b: Array<int>) -> Array<int> { if cond {a} else {b} }` → conservative: `Unknown` (borrowed from differing params)
- `fn make() -> Array<int> { if cond { [1] } else { [2] } }` → `NewlyOwned` (both branches allocate)

**Verification**: `cargo test -p shape-vm --lib return_ownership` — all pass. No existing tests regress.

---

### Phase 5.B: Use return mode in callers (NewlyOwned path)

**Goal**: When a caller does `let x = callee()` and callee's return mode is `NewlyOwned`, skip the Arc→Box promotion (the value is already owned).

#### 5.B.1: Extend BindingSemantics

**File**: `crates/shape-vm/src/type_tracking.rs:514-537`

Add field:
```rust
pub struct BindingSemantics {
    // ... existing fields
    pub return_ownership_hint: Option<ReturnOwnershipMode>,  // NEW
}
```

When a binding is assigned from a function call, look up the callee's `return_ownership_mode` and store here.

#### 5.B.2: Compiler uses the hint

**File**: `crates/shape-vm/src/compiler/statements.rs` (around the `PromoteToOwned` emission site)

Current logic:
```rust
if is_owned_binding && has_direct_storage {
    emit(OpCode::PromoteToOwned);
}
```

Extended logic:
```rust
if is_owned_binding && has_direct_storage {
    if matches!(binding.return_ownership_hint, Some(ReturnOwnershipMode::NewlyOwned)) {
        // Callee already returned Box — no conversion needed
        // (Requires callee to actually return Box — Phase 5.C)
    } else {
        emit(OpCode::PromoteToOwned);  // Arc → Box at callsite (current behavior)
    }
}
```

#### 5.B.3: Tests

Verify that `let arr = make_array()` where `make_array` returns `NewlyOwned` doesn't emit `PromoteToOwned`:

```rust
// Test: bytecode should not contain PromoteToOwned after call to NewlyOwned fn
#[test]
fn test_newly_owned_return_skips_promote() {
    let src = r#"
        fn make() -> Array<int> { [1, 2, 3] }
        fn use_it() -> int {
            let arr = make()
            arr.reduce(|a, b| a + b)
        }
    "#;
    let program = compile(src);
    let use_it = program.function_by_name("use_it").unwrap();
    assert!(!contains_opcode(use_it, OpCode::PromoteToOwned));
}
```

**Verification**: `just test-fast` passes. No performance regression.

---

### Phase 5.C: Callee emits owned return (wiring the promise)

**Goal**: Functions with `ReturnOwnershipMode::NewlyOwned` actually return Box-backed values, not Arc.

#### 5.C.1: Add `ReturnOwned` opcode

**File**: `crates/shape-vm/src/bytecode/opcode_defs.rs`

```rust
/// Return with owned semantics — promotes Arc→Box if refcount=1 just before return.
ReturnOwned = 0x11A, ControlFlow, pops: 1, pushes: 0;
```

#### 5.C.2: Executor handler

Similar to `PromoteToOwned` but happens at function return boundary. Pop return value, try `Arc::try_unwrap`, push Box-encoded bits, then return.

#### 5.C.3: Compiler emits based on return mode

**File**: `crates/shape-vm/src/compiler/statements.rs` (return statement emission)

When emitting return for a function with `return_ownership_mode: NewlyOwned`:
- Emit `ReturnOwned` instead of `Return`

#### 5.C.4: Runtime correctness

Critical invariant: the `NewlyOwned` promise must hold. The callee's return expression must be a newly-allocated value with refcount=1. If this isn't true (e.g., the solver made an incorrect inference), the `ReturnOwned` handler gracefully falls back to Arc (same as PromoteToOwned: if refcount > 1, leave as Arc).

#### 5.C.5: Tests

- `fn make_array() -> Array<int> { [1,2,3] }` — after call, returned value is Box-backed (check bit 0 of payload)
- Verify Phase 5.B caller and Phase 5.C callee work together — no Arc allocation in the round-trip

---

### Phase 5.D: Non-escaping closure specialization

**Goal**: When a closure is defined, called, and discarded in the same scope, it doesn't need to escape — captures stay owned.

#### 5.D.1: Detect non-escaping closures

**New pass**: `detect_non_escaping_closures(mir: &MirFunction) -> HashSet<SlotId>`

A closure is non-escaping if:
- Its slot is assigned exactly once (at creation)
- All uses are direct calls (`call closure(...)` or method arguments on known higher-order functions like `arr.map(f)`)
- The closure value doesn't flow to return (`escape_status: Local`)
- The closure is never stored in a collection

**Location**: `crates/shape-vm/src/mir/storage_planning.rs` — add alongside existing `collect_closure_captures`.

#### 5.D.2: Specialize closure allocation

For non-escaping closures:
- Captures can be `Direct` (owned) instead of `SharedCow`
- The closure itself doesn't need Arc wrapping — callsite knows the exact body

#### 5.D.3: Future: Inline the closure body

Non-escaping closures passed to monomorphizable higher-order functions (`map`, `filter`, `reduce`) can be **inlined** — their bodies expand into the loop, eliminating the closure call entirely.

This is an ambitious follow-up (Phase 5.E or later). For Phase 5.D, just get the ownership optimization.

#### 5.D.4: Tests

- `nums.map(|x| x * 2)` — closure is non-escaping, captures stay owned
- `fn make_adder(n: int) -> Closure { |x| x + n }` — closure ESCAPES (returned), captures must be Arc

---

### Phase 5.E: Parameter-pass specialization (optional, monomorphization key extension)

**Goal**: For generic functions called with owned vs shared values at different sites, generate separate specialized versions.

This is the **highest complexity, lowest certainty** sub-phase. Recommended to evaluate after 5.A-5.D land.

#### Design sketch

Monomorphization key extended:
```rust
struct MonoKey {
    function_name: String,
    type_args: Vec<ConcreteType>,
    param_modes: Vec<ParamPassMode>,  // NEW
}
```

Trade-off:
- **Pro**: Hot paths get fully-specialized code with no Arc overhead
- **Con**: 2-4x code bloat for generic functions

Mitigation: Only specialize when:
- Function is called from a hot site (profile-guided)
- The owned vs shared distinction actually affects codegen (e.g., mutation happens)
- Otherwise: fall back to the "shared" variant (conservative)

**Decision point**: After Phase 5.A-D are validated, measure how much residual Arc overhead remains. If < 5%, skip 5.E.

---

## Fixed-Point Convergence

When function A calls function B, A's return mode may depend on B's return mode. Cycles are possible (A calls B calls A).

### Resolution strategy

1. **Initialize** all functions to `ReturnOwnershipMode::Unknown`
2. **Iterate** over the call graph:
   - Compute each function's return mode based on current summaries of callees
   - If changed, mark as dirty
3. **Repeat** until no changes in a full pass
4. **Terminate** with either:
   - Success (converged)
   - Failure (> N iterations — fall back all to Unknown)

Similar to how the borrow solver already handles cycles via Datafrog.

**Implementation**: Augment the existing `solve_borrow_facts` to include ownership propagation.

---

## Testing Strategy

### Unit tests

Per sub-phase, add tests to `crates/shape-vm/src/mir/return_ownership.rs` and `storage_planning.rs`:

1. **Isolated functions** — cover each ReturnOwnershipMode variant
2. **Caller-callee pairs** — verify propagation
3. **Multi-function chains** — verify fixed-point convergence
4. **Cycles** — mutual recursion doesn't hang
5. **Conservative fallback** — ambiguous cases default to Unknown/Arc

### Integration tests

Add end-to-end tests in `crates/shape-vm/src/compiler/compiler_tests.rs`:

1. **No PromoteToOwned after NewlyOwned call**:
   ```shape
   fn make() -> Array<int> { [1,2,3] }
   fn main() -> int {
       let arr = make()
       arr.reduce(|a,b| a+b)
   }
   ```
   Verify: `main`'s bytecode doesn't contain `PromoteToOwned`.

2. **Non-escaping closure keeps owned captures**:
   ```shape
   fn main() -> int {
       let n = 5
       [1,2,3].map(|x| x + n).reduce(|a,b| a+b)
   }
   ```
   Verify: `n` is not wrapped in `SharedCell<Arc<...>>`.

3. **No regression** — all existing 5,383 tests continue to pass.

### Performance verification

Run the existing benchmarks. The key measurement:
- **Before Phase 5**: Every function call involves at least one atomic op (Arc increment/decrement)
- **After Phase 5**: Pure let-binding flow has zero atomic ops

Expected improvement on array-heavy workloads: 10-30%.

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Fixed-point doesn't converge | Low | High | Iteration cap + fallback to Unknown |
| Incorrect NewlyOwned inference causes UAF | Low | Critical | ReturnOwned opcode has runtime check (Arc::try_unwrap fallback) |
| Code bloat from monomorphization specialization (5.E) | High | Medium | Profile-guided; skip if benefit < 5% |
| Module boundaries break analysis | Medium | Medium | Each module analyzed independently; cross-module calls default to Unknown |
| Closure inlining regresses debuggability | Low | Low | Keep closure bodies addressable for stack traces |

---

## Execution Plan

### Recommended order (fresh session)

1. **Session 1: Phase 5.A** (1-2 days)
   - Add `ReturnOwnershipMode` enum
   - Implement `infer_return_ownership_mode` pass
   - Wire into `FunctionBorrowSummary`
   - Tests for per-function return mode inference
   - **Commit**: "Phase 5.A: infer return ownership mode per function"

2. **Session 2: Phase 5.B** (1 day)
   - Extend `BindingSemantics` with `return_ownership_hint`
   - Skip `PromoteToOwned` emission when hint is `NewlyOwned`
   - Tests for caller-side behavior
   - **Commit**: "Phase 5.B: skip PromoteToOwned when callee returns NewlyOwned"

3. **Session 3: Phase 5.C** (1-2 days)
   - Add `ReturnOwned` opcode + handler
   - Compiler emits it for NewlyOwned functions
   - End-to-end tests — verify Box return + direct owned binding
   - **Commit**: "Phase 5.C: callees with NewlyOwned return emit ReturnOwned"

4. **Session 4: Phase 5.D** (1-2 days)
   - Non-escaping closure detection
   - Owned captures for non-escaping closures
   - Tests
   - **Commit**: "Phase 5.D: non-escaping closures keep owned captures"

5. **Session 5 (optional): Phase 5.E** (2-3 days)
   - Parameter-pass specialization in monomorphization
   - Only if 5.A-D don't cover enough ground
   - **Commit**: "Phase 5.E: parameter-pass specialization in mono keys"

### Parallelization notes

5.A must be first (data source for everything else). 5.B and 5.C must be done together or in close succession (5.B without 5.C is safe but ineffective; 5.C without 5.B causes double-promotion).

5.D is independent — can run in parallel with 5.B-C.

5.E depends on 5.A-D.

**Solo agent required** for 5.A, 5.C, 5.D — these modify MIR analysis and opcode infrastructure. Per Option 1 lesson, parallel agents can revert each other's in-progress work on shared files.

---

## Key Files to Modify

| File | Change | Phase |
|------|--------|-------|
| `crates/shape-vm/src/mir/analysis.rs` | Add `ReturnOwnershipMode` enum, extend summary | 5.A |
| `crates/shape-vm/src/mir/return_ownership.rs` | New file — inference pass | 5.A |
| `crates/shape-vm/src/mir/solver.rs` | Thread summaries through fixed-point | 5.A |
| `crates/shape-vm/src/compiler/functions.rs` | Compute and store summary per function | 5.A |
| `crates/shape-vm/src/type_tracking.rs` | Add `return_ownership_hint` to BindingSemantics | 5.B |
| `crates/shape-vm/src/compiler/statements.rs` | Skip PromoteToOwned conditionally | 5.B |
| `crates/shape-vm/src/bytecode/opcode_defs.rs` | Add `ReturnOwned` opcode (0x11A) | 5.C |
| `crates/shape-vm/src/executor/control_flow/mod.rs` | Handler for `ReturnOwned` | 5.C |
| `crates/shape-vm/src/compiler/functions.rs` | Emit `ReturnOwned` for NewlyOwned funcs | 5.C |
| `crates/shape-vm/src/mir/storage_planning.rs` | Non-escaping closure detection | 5.D |
| `crates/shape-vm/src/compiler/expressions/closures.rs` | Use non-escape info for capture storage | 5.D |
| `crates/shape-vm/src/compiler/monomorphization/cache.rs` | Optional: extend mono key with param modes | 5.E |

## Success Metrics

After full Phase 5 implementation:

- Zero atomic operations for pure `let` binding chains across function boundaries
- Pipeline `fn a() -> Array<int> → fn b(Array<int>) -> Array<int> → fn c(Array<int>) -> int` runs without a single `Arc::increment_strong_count` in the hot path
- Array-heavy benchmarks show 10-30% improvement
- Code size increase from specialization: < 20% (if 5.E is implemented)
- All 5,383 existing tests continue to pass
- 40+ new tests covering escape analysis scenarios

## Open Questions

1. **Module-level summaries**: Should we persist function summaries across module compilations? (Content-addressed bytecode makes this feasible but not implemented.)

2. **Trait method dispatch**: When a method is called via trait dispatch, the callee's ownership mode may not be statically known. Should we default to Unknown, or use a feedback vector to track actual callees?

3. **Polymorphic closures**: `arr.map(f)` where `f` is a user-provided closure — can we infer its ownership properties from the arg type?

4. **Cross-module edge cases**: What if module A calls function `f` from module B, and B is compiled later? We'd need forward declarations of ownership summaries.

These can be addressed in Phase 6 or deferred indefinitely depending on usage patterns.

---

## References

- Current baseline research: see `detect_escape_status` at `crates/shape-vm/src/mir/storage_planning.rs:464-487`
- `FunctionBorrowSummary` at `crates/shape-vm/src/mir/analysis.rs:286-294`
- `BindingSemantics` at `crates/shape-vm/src/type_tracking.rs:514-537`
- Phase 3 PromoteToOwned implementation: commit `39f5a16`
- Ownership-aware runtime plan: `docs/ownership-aware-runtime-v2.md`
- Related: `docs/v2-monomorphization-design.md` for mono key extension in Phase 5.E
