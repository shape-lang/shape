# TODO: Migrate to type extraction helpers

## Tasks

- [x] 1. `object_operations.rs`: Replace `Object|ManagedObject` match-and-clone with `as_object(val)?.clone()`
- [x] 2. `object_creation.rs`: Replace `String|ManagedString` match in `op_new_object` with `as_string(&key)?.to_string()`
- [x] 3. `datatable_methods.rs`: Update `require_string_arg` to use `as_string` (handles ManagedString)
- [x] 4. `datatable_methods.rs`: Replace `handle_column` String-only extraction with `as_string`
- [x] 5. `datatable_methods.rs`: Replace `handle_select` String-only extraction with `as_string`
- [x] 6. `datatable_methods.rs`: Replace `apply_comparison` string/bool extractions with `as_string`/`as_bool`
- [x] 7. `datatable_methods.rs`: Replace `parse_agg_spec` String-only extractions with `as_string`
- [x] 8. Verify compilation with `cargo check`
- [x] 9. Run tests to ensure no regressions

## Review

All three files were migrated to use `type_extraction` helpers. Total: 9 replacement sites across 3 files.

### Changes by file

**`object_operations.rs`** (2 replacements):
- Added `use crate::executor::utils::type_extraction::as_object;`
- Replaced two 7-line `Object|ManagedObject` match blocks in `op_merge_object` with single-line `as_object(&val)?.clone()` calls

**`object_creation.rs`** (1 replacement):
- Added `use crate::executor::utils::type_extraction::as_string;`
- Replaced 9-line `String|ManagedString` match block in `op_new_object` with `as_string(&key)?.to_string()`

**`datatable_methods.rs`** (6 replacement sites):
- Added `use crate::executor::utils::type_extraction::{as_bool, as_string};`
- `require_string_arg`: Now delegates to `as_string()` internally (also now handles `ManagedString`)
- `handle_column`: Replaced `VMValue::String` match with `as_string()` (also now handles `ManagedString`)
- `handle_select`: Replaced per-arg `VMValue::String` match with `as_string()` (also now handles `ManagedString`)
- `apply_comparison`: Replaced string value extraction with `as_string()`, bool extraction with `as_bool()`
- `parse_agg_spec`: Replaced inner String-only matches with `as_string()`, added `ManagedString` to outer pattern arm

### What was NOT changed (intentionally preserved)
- Multi-variant matches (e.g., `Number` vs `String` handled differently)
- `extract_dt()` helper: handles `DataTable|TypedTable|IndexedTable` with custom logic per variant
- `handle_aggregate` spec extraction: outer match has distinct Array vs String vs other arms
- Custom error messages preserved via `.map_err()` wrappers around the helpers
- All test code left untouched

### Correctness improvement
Several patterns previously only handled `VMValue::String` but not `VMValue::ManagedString`. The `as_string()` helper handles both, making the code more robust with GC-managed strings.

---

# PREVIOUS: Refactor OHLCV-specific code to use generic field access

## Tasks

- [x] 1. Replace row.close() calls with row.get_field("close") in shape-core/src/runtime/position/api.rs (2 occurrences at lines 174, 230)
- [x] 2. Replace row.open/close/high/low/volume() with row.get_field() in shape-core/src/query_executor.rs (1 occurrence at line 308)
- [x] 3. Replace row.open/close/high/low/volume() with row.get_field() in shape-core/src/engine.rs (check for usage in extract_and_precalculate_indicators, around line 1569-1574)
- [x] 4. Replace row.open/close/high/low/volume() with row.get_field() in other runtime files:
  - [x] shape-core/src/runtime/multi_series.rs
  - [x] shape-core/src/runtime/evaluation/data_eval.rs
  - [x] shape-core/src/runtime/series_ref.rs
  - [x] shape-core/src/runtime/backtest.rs
  - [x] shape-core/src/runtime/evaluation/method_eval.rs
  - [x] shape-core/src/runtime/context.rs (RowValueExt trait impl)
  - [x] shape-core/src/runtime/evaluation/functions/series.rs
  - [x] shape-core/src/runtime/evaluation/functions/data.rs
  - [x] shape-core/src/runtime/evaluation/functions/series_access.rs
  - [x] shape-core/src/vm/executor.rs
  - [x] shape-core/src/data/aggregator.rs
- [x] 5. Replace RowValue::new() with RowValue::new_generic() across remaining files:
  - [x] shape-core/src/data/aggregator.rs (converted to test_row() helper)
  - [x] shape-cli/src/conversion.rs (fixed from_candle and to_candle functions)
- [ ] 6. Remove Candle type definition from shape-core/src/runtime/type_schema.rs (check lines 450-476 in test_ohlcv_schema)
- [x] 7. Verify compilation with `cargo check` - **SUCCESS** (0 errors, 26 warnings)
- [ ] 8. Run tests to ensure no regressions

## Progress

- Replaced 60+ occurrences of .open()/.high()/.low()/.close()/.volume() with get_field() across 15 files
- Fixed AlignedVec type annotations in series_ref.rs
- Converted aggregator.rs to use new_generic() with test_row() helper
- Fixed shape-cli conversion functions (from_candle, to_candle)
- **Compilation successful** - all errors resolved

## Notes
- OHLCV-specific field names (open, high, low, close, volume) belong in stdlib, not in Rust core
- Type definitions should be in stdlib/finance/types.shape, NOT in Rust type_schema.rs
- Use generic row.get_field("field_name") instead of dedicated accessor methods

---

# Migrate array files to use type_extraction helpers

## Plan
- [x] Read all four target files and type_extraction.rs
- [x] Identify replaceable patterns in each file
- [x] Edit array_basic.rs - replace read-only array extractions with `as_array()`
- [x] Edit array_aggregation.rs - replace array extractions with `as_array()`
- [x] Edit array_transform.rs - replace array extractions with `as_array()`
- [x] Edit array_operations.rs - assessed, no safe replacements found
- [x] Run `cargo check` to verify compilation - passed (no new warnings)
- [x] Run `cargo test` to verify tests pass - 254 passed, 21 pre-existing failures unchanged
- [x] Add review section

## Results
- [x] array_basic.rs: 6 replacements
- [x] array_aggregation.rs: 5 replacements
- [x] array_transform.rs: 5 replacements
- [x] array_operations.rs: 0 replacements (all mutations or multi-branch)
- [x] cargo check passed
- [x] cargo test passed

## Analysis

### array_basic.rs
- `handle_len`, `handle_length`, `handle_first`, `handle_last`, `handle_get`, `handle_reverse`: Extract array as `&Arc<Vec<VMValue>>` then use `.len()`, `.first()`, `.last()`, `.get()`, `.as_ref()`. All work on slices, so `as_array()` returning `&[VMValue]` is a valid replacement.
- `handle_push`, `handle_pop`, `handle_set`: Need `Arc::make_mut` for mutation - CANNOT replace.

### array_aggregation.rs
- `handle_sum`, `handle_avg`, `handle_min`, `handle_max`, `handle_count`: Extract array then call `.as_ref()` to get `&[VMValue]` - direct match for `as_array()`.
- Inner loops use custom error messages like "sum() requires array of numbers" - do NOT replace those.

### array_transform.rs
- `handle_slice`, `handle_concat`, `handle_take`, `handle_drop`, `handle_flatten`: Extract array then call `.as_ref()` - replaceable with `as_array()`.
- Inner loops in `handle_concat` and `handle_flatten` intentionally dispatch on Array vs non-Array - do NOT replace.

### array_operations.rs
- `op_array_push`, `op_array_pop`: Mutate via `Arc::make_mut` - CANNOT replace.
- `op_slice_access`: Handles both Array and String with different branches - CANNOT replace.
- No safe replacements in this file.

## Review

Migrated 16 array extraction patterns across 3 files to use the `as_array()` type extraction helper from `crate::executor::utils::type_extraction`. The fourth file (`array_operations.rs`) had no safe replacements because all its patterns either mutate through `Arc::make_mut` or handle both Array and String types differently.

**Files changed:**
- `array_basic.rs`: 6 replacements (handle_len, handle_length, handle_first, handle_last, handle_get, handle_reverse)
- `array_aggregation.rs`: 5 replacements (handle_sum, handle_avg, handle_min, handle_max, handle_count)
- `array_transform.rs`: 5 replacements (handle_slice, handle_concat, handle_take, handle_drop, handle_flatten)
- `array_operations.rs`: 0 replacements

**What was NOT replaced (by design):**
- Mutable array access via `Arc::make_mut` (handle_push, handle_pop, handle_set, op_array_push, op_array_pop)
- Multi-type dispatch branches (op_slice_access handles Array+String)
- Inner loop element extractions with custom error messages (e.g. "sum() requires array of numbers")
- Multi-variant concat/flatten inner loops that intentionally handle Array vs non-Array elements differently

**Bonus from `as_array()`:** The helper also handles `ManagedArray` variants, which the original code already handled manually. No behavior change.

---

# COMPLETED: `format` Keyword Implementation

## Summary

Implemented a new `format` language construct for type-safe, extensible value formatting defined entirely in Shape stdlib (not Rust), consistent with the domain-agnostic philosophy.

## Syntax

```shape
format Percent for Number {
    decimals: number = 2;

    format(value) -> string {
        let multiplied = value * 100;
        return multiplied.toFixed(this.decimals) + "%"
    }

    parse(str) -> number {
        return str.replace("%", "").parseFloat() / 100
    }
}

// Mark as default with keyword
format Currency for Number default { ... }
```

**Usage:**
```shape
let profit = 0.0523;
profit.format()                  // Uses default format
profit.format("Percent")         // "5.23%"
```

## Phases Completed

- [x] **Phase 1**: Add FormatDef AST structures (`shape-ast/src/ast/types.rs`)
- [x] **Phase 2**: Add format grammar rules to pest (`shape-ast/src/shape.pest`)
- [x] **Phase 3**: Implement format parser functions (`shape-ast/src/parser/extensions.rs`)
- [x] **Phase 4**: Add format registry to TypeEnvironment (`shape-runtime/src/type_system/environment.rs`)
- [x] **Phase 5**: Implement format_eval.rs runtime (`shape-runtime/src/evaluation/format_eval.rs`)
- [x] **Phase 6**: Update wire protocol metadata (`shape-wire/src/metadata.rs`, optional `ast-integration` feature)
- [x] **Phase 7**: Create stdlib/core/formats.shape (Number, String, bool, Timestamp, Duration formats)
- [x] **Phase 8**: Simplify Rust formatters to basic fallback (`shape-wire/src/formatter.rs`)

## Files Modified/Created

**New Files:**
- `shape-runtime/src/evaluation/format_eval.rs` - Format execution engine
- `stdlib/core/formats.shape` - Standard format definitions

**Modified Files:**
- `shape-ast/src/ast/types.rs` - FormatDef, FormatParameter, FormatMethod structs
- `shape-ast/src/ast/program.rs` - Item::Format variant
- `shape-ast/src/ast/mod.rs` - Export new types
- `shape-ast/src/shape.pest` - format_def grammar rules
- `shape-ast/src/parser/extensions.rs` - parse_format_def function
- `shape-ast/src/parser/mod.rs` - Format dispatch
- `shape-runtime/src/type_system/environment.rs` - FormatEntry, format registry
- `shape-runtime/src/type_system/inference/items.rs` - Register formats
- `shape-runtime/src/evaluation/method_eval/mod.rs` - .format() method support
- `shape-runtime/src/evaluator.rs` - RuntimeFormatEntry, format methods
- `shape-runtime/src/lib.rs` - Format registration
- `shape-runtime/src/semantic/mod.rs` - Item::Format handling
- `shape-runtime/src/visitor.rs` - Walk format definitions
- `shape-wire/src/metadata.rs` - FormatRegistry from AST (feature-gated)
- `shape-wire/src/formatter.rs` - Simplified to basic fallback
- `shape-wire/Cargo.toml` - ast-integration feature

## Key Design Decisions

1. **Wire protocol**: Reference-only (format names, not implementations) - external tools need Shape runtime
2. **Default format**: Designated via `default` keyword in syntax
3. **Built-in formats**: All in Shape stdlib (including Number, Timestamp, etc.)
4. **Domain-agnostic**: No finance-specific formatting in Rust core

## Documentation

- [x] **Book Chapter**: Created `book/src/fundamentals/formatting.md` (~350 lines)
  - Overview of the formatting system
  - Using formats with `.format()` method
  - Built-in formats (Number, String, bool, Timestamp, Duration)
  - Defining custom formats with `format` keyword
  - Format parameters and the `this` context
  - Parse methods for bidirectional conversion
  - Domain-specific format examples
  - Best practices
  - Wire protocol metadata

- [x] **SUMMARY.md**: Added "Value Formatting" chapter to Language Fundamentals section
- [x] **variables.md**: Added tip referencing formatting chapter in Type Conversion section

---

# Migrate builtins to use type_extraction helpers (Batch 2)

## Plan

Target files (adjusted from original names to actual file locations):
- `shape-vm/src/executor/builtins/type_ops.rs` - type checking/conversion builtins
- `shape-vm/src/executor/builtins/special_ops.rs` - print, reflect, fold (IO-equivalent)
- `shape-vm/src/executor/builtins/native_intrinsics.rs` - math/statistical intrinsics
- `shape-vm/src/executor/builtins/runtime_delegated.rs` - delegation dispatch

## Tasks

- [x] Read and analyze all four files for replaceable patterns
- [x] Edit special_ops.rs - replace `builtin_reflect` string extraction with `as_string()`, replace `builtin_control_fold` array extraction with `as_array()`
- [x] Edit type_ops.rs - no replaceable patterns (multi-variant matches, custom error messages)
- [x] Edit native_intrinsics.rs - no replaceable patterns (custom helpers, range validation, custom errors)
- [x] Edit runtime_delegated.rs - no replaceable patterns (dispatch table only)
- [x] Run `cargo check` to verify compilation -- passed (no new warnings)
- [x] Run `cargo test` to verify tests pass -- 254 passed, 21 pre-existing failures (annotations/extend_blocks)
- [x] Add review section

## Analysis

### type_ops.rs
- `builtin_to_number` (lines 78-95): Multi-variant match (Number, Bool, String) with different handling per type. Do NOT replace.
- `is_*` functions: Use `matches!()` for type checking, not extraction. Do NOT replace.
- `builtin_to_bool`, `builtin_to_string`, `builtin_type_of`: No simple extraction pattern. Do NOT replace.
- **Result: 0 replacements**

### special_ops.rs
- `builtin_reflect` (lines 189-194): Extracts string from `VMValue::String(s)` only. `as_string()` would also handle `ManagedString`, which is more correct. Generic TypeError message is sufficient. **Replace.**
- `builtin_control_fold` (lines 248-255): Extracts array from `VMValue::Array | ManagedArray`. Exactly matches `as_array()`. **Replace.**
- **Result: 2 replacements**

### native_intrinsics.rs
- `extract_f64_data` (lines 18-47): Handles Array, ManagedArray, ColumnRef, Number -- more variants than `as_array`. Do NOT replace.
- `extract_window` (lines 50-61): Single-type extraction with custom range validation. Do NOT replace.
- `vm_intrinsic_shift` (line 406-408): Number-only extraction with custom error. Do NOT replace.
- `vm_intrinsic_percentile` (lines 575-583): Interleaved range validation. Do NOT replace.
- **Result: 0 replacements**

### runtime_delegated.rs
- Pure dispatch table with no type extraction patterns.
- **Result: 0 replacements**

## Review

### Changes Made
Only `special_ops.rs` had patterns qualifying for replacement. The other three files had no simple single-type extraction patterns.

**File: `shape-vm/src/executor/builtins/special_ops.rs`** (2 replacements)
1. Added import: `use crate::executor::utils::type_extraction::{as_string, as_array};`
2. `builtin_reflect`: Replaced 6-line match block with `as_string(&args[0])?.to_string()`. Also improves correctness by handling `ManagedString` inputs.
3. `builtin_control_fold`: Replaced 8-line match block with `as_array(&args[0])?`. The helper returns `&[VMValue]` which is directly iterable.

**Files with no replacements (and why):**
- `type_ops.rs`: All matches are multi-variant, type-checking only, or use custom conversion logic.
- `native_intrinsics.rs`: Extractions handle more variants than type_extraction helpers (ColumnRef, range validation), or have function-specific error messages.
- `runtime_delegated.rs`: Pure dispatch table with no type extraction at all.

### Test Results
254 tests passed. 21 pre-existing failures in annotations/extend_blocks modules (verified identical before and after changes).
