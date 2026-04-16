# Session Prompt: Complete NaN-Boxing Removal & Monomorphization

Execute the plan at `docs/nanbox-removal-plan.md` (also saved as `~/.claude/plans/encapsulated-cuddling-rabin.md`). This is a 30-agent coordinated effort to finish removing NaN-boxing infrastructure from the Shape runtime.

## What's already done (don't redo)

- `ValueWord` is `pub type ValueWord = u64` (newtype struct deleted)
- `NanTag` enum deleted — 0 references in production code
- `push_vw`/`pop_vw` eliminated — 0 call sites, definitions deleted
- Typed array/map/field/enum opcodes wired end-to-end (compiler emits, executor dispatches)
- Monomorphization engine integrated into compiler (generic function calls produce specialized bytecode)
- 5,268 tests pass, 0 failures

## What remains (this is your job)

### Core files to reduce or delete:
- `crates/shape-value/src/value_word.rs` (3,836 lines) — gut to ~1,500
- `crates/shape-value/src/tags.rs` (356 lines) — delete entirely
- `crates/shape-value/src/heap_value.rs` + `heap_variants.rs` (1,715 lines) — shrink HeapValue from 72 to ~25 variants
- `crates/shape-jit/src/nan_boxing.rs` (940 lines) — delete entirely

### Key metrics to drive to zero:
- `as_heap_ref()` calls: currently 127 sites — eliminate from hot paths
- `is_tagged()`/`get_tag()` dispatch: currently 374 sites — simplify
- HeapValue variants: currently 72 — consolidate to ~25

## Execution order

**Wave A (parallel, 14 agents):** Phases 1 + 2 + 6
- Phase 1 (8 agents): Replace `as_heap_ref()` → `HeapValue::Xxx` pattern matches with direct typed access in all method handlers. The PHF method dispatch already knows the receiver type — use that to cast directly to the concrete Arc<T> without going through the HeapValue enum.
- Phase 2 (6 agents): Simplify remaining `is_tagged()`/`get_tag()` dispatch in arithmetic, comparison, variables, control flow, builtins, and shape-runtime.
- Phase 6 (3 agents): Complete monomorphization — method call specialization, stdlib per-type variants, typed closure captures.

**Wave B (parallel, 11 agents):** Phases 3 + 4 + 5
- Phase 3 (4 agents): Shrink HeapValue — remove shadow variants (Number/Bool/Function that are inline-tagged), consolidate 11 typed array variants into one, consolidate 7 temporal variants, remove unused variants.
- Phase 4 (4 agents): Gut value_word.rs — extract ArrayView to own file, collapse 35 constructors to ~5, collapse 42 extractors to ~6, simplify Display from 364 lines.
- Phase 5 (3 agents): Delete nan_boxing.rs — replace box/unbox with value_word free functions, remove UnifiedValue enum, move JIT-specific types to ffi/typed/.

**Wave C (2 agents):** Phase 7
- Delete tags.rs, final sweep for any remaining NaN-boxing references, update docs.

## Key architectural insight

The bit representation doesn't change. Values are still encoded as u64 with NaN-boxing bits. What changes is that the RUNTIME DISPATCH infrastructure (tag checks, HeapValue enum matching) gets replaced by COMPILE-TIME TYPE KNOWLEDGE from typed opcodes. The compiler already proves types — the executor should trust that instead of re-checking at runtime.

For `as_heap_ref()` replacement specifically: each method handler file knows what HeapKind it handles (from the PHF map routing). Instead of `val.as_heap_ref()` → match all 72 variants, do:
```rust
// Before:
match val.as_heap_ref() {
    Some(HeapValue::Array(a)) => { /* use a */ }
    _ => return Err(...)
}

// After:
let a = val.as_array().ok_or(VMError::TypeError(...))?;
// or for max performance:
let ptr = get_payload(val) as *const Arc<Vec<u64>>;
let a = unsafe { &*ptr };
```

## Verification after each wave

```bash
cargo check --workspace 2>&1 | grep "^error" | head -5  # must be 0
just test-fast 2>&1 | grep "test result:"                # all must pass
```

After Wave C (final):
```bash
grep -rn "NanBox\|nan_box\|TAG_NULL\|TAG_BOOL" crates/ --include="*.rs" | grep -v "test\|Test\|//" | wc -l  # must be 0
wc -l crates/shape-value/src/value_word.rs  # target: <1,500
wc -l crates/shape-jit/src/nan_boxing.rs    # target: file deleted
```

## Important caveats

- A linter hook modifies `module_resolution.rs` after edits — work WITH the Result return type
- `ValueWord = u64` means `assert_eq!(val1, val2)` does BITWISE comparison — use `val.as_str().unwrap()` for semantic string comparison in tests
- When using TAG_* constants in `match` arms, they MUST be imported (`use shape_value::tags::TAG_FUNCTION;`) or Rust treats them as catch-all variable bindings (this caused 34 test failures before)
- The `#[cfg(not(feature = "gc"))]` / `#[cfg(feature = "gc")]` dual paths exist throughout — maintain both
- Don't delete `as_heap_ref()` entirely — cold paths (serialization, display, debug) still need it. Target: <10 call sites remaining
