## Dead code suspects (frontiers)

### `crates/shape-jit/src/ffi/window.rs`
- **Path**: `crates/shape-jit/src/ffi/window.rs:1-3`
- **Why suspected**: File body is a 3-line tombstone comment (`v2-boundary: Window FFI functions deleted — no callers from MirToIR or executor. All functions were only registered in ffi_symbols (dead wiring).`); still re-exported via `pub use window::*;` in `ffi/mod.rs:58`.
- **Confidence**: high

### `crates/shape-jit/src/ffi/call_method/signal_builder.rs` and `JITSignalBuilder`
- **Path**: `crates/shape-jit/src/ffi/call_method/signal_builder.rs:1-67`; struct at `crates/shape-jit/src/context.rs:434-468`
- **Why suspected**: `JITSignalBuilder::new` and `::box_builder` have no external callers in `shape-jit`, `shape-vm`, or anywhere else in the workspace. The method-dispatch handler exists but `is_heap_kind(receiver_bits, HK_JIT_SIGNAL_BUILDER)` can never fire unless something boxes a builder, and nothing does. Likely orphaned from finance-removal sweep.
- **Confidence**: high

### `crates/shape-jit/src/numeric_compiler.rs` (`compile_numeric_program`)
- **Path**: `crates/shape-jit/src/numeric_compiler.rs:1-105`; called only from `compiler/program.rs:146`
- **Why suspected**: Module's own header calls itself "Legacy numeric-only JIT compiler" and is "used for simple numeric programs that don't require NaN-boxing or FFI calls" — the whole architecture moved to MirToIR. The `core.rs` module header still describes a finance-strategy-oriented "pure numeric model (f64-only stack)" target that pre-dates v2.
- **Confidence**: medium

### `crates/shape-jit/src/core.rs`
- **Path**: `crates/shape-jit/src/core.rs` (1418 lines; `mod core;` in `lib.rs:22` is private re-export only)
- **Why suspected**: Module doc-comment is the pre-v2 target description ("0.1-1µs per row vs 2-10µs with VM, … Technical indicators (SMA, EMA, RSI), Entry/exit logic with fuzzy comparisons"). Many of the gated `test_jit_width_aware_*` tests live here. Per CLAUDE.md, the legacy `mod core` is described as "Legacy re-exports and tests".
- **Confidence**: medium

### `crates/shape-jit/src/REFACTOR_PLAN.md`
- **Path**: `crates/shape-jit/src/REFACTOR_PLAN.md` (179 lines)
- **Why suspected**: In-tree refactor plan that lists items as `[x]` complete (e.g. `signal_builder.rs` move) but no enforcement keeps it current. Sibling `crates/shape-jit/src/compiler/V2_MIGRATION_STATUS.md` is the same shape — historical not active.
- **Confidence**: medium

### Width-aware JIT tests
- **Path**: `crates/shape-jit/src/core.rs:740-900+` (`test_jit_width_aware_u8_add_wraps`, `test_jit_width_aware_i8_*`, `test_jit_width_aware_u16_*`, `test_jit_width_aware_i32_add_wraps`, `test_jit_width_aware_u32_add_wraps`, …)
- **Why suspected**: Per CLAUDE.md ~23 shape-jit `#[ignore]`'d tests in this family stay ignored under `just test-all`. Tests reference per-width arithmetic that is no longer the primary execution path; behavior may have drifted from what the tests assert.
- **Confidence**: low

### `crates/shape-jit/src/ffi/mod.rs:11-12, :42-43` (DELETED comments)
- **Path**: `crates/shape-jit/src/ffi/mod.rs:11`, `:12`, `:42`, `:43`
- **Why suspected**: Tombstone comments referencing `pub mod indicator;` and `pub use indicator::*;` for the finance-specific JIT module. Unused tombstones in the source.
- **Confidence**: medium

### Finance-specific FFI symbols (tombstones)
- **Path**: `crates/shape-jit/src/ffi_symbols/data_access/mod.rs:12-18`
- **Why suspected**: 4 `// DELETED: jit_market_*` tombstone comments. Pure breadcrumbs.
- **Confidence**: low

### `crates/shape-jit/src/jit_matrix.rs` `JitMatrix`
- **Path**: `crates/shape-jit/src/jit_matrix.rs:32-115`
- **Why suspected**: Used only at `ffi/object/conversion.rs:301, :676-677` and `ffi/call_method/matrix.rs:3, :15` — a thin matrix bridge. Worth checking whether the v2 `Arc<MatrixData>` direct path in `ffi/v2/` makes it redundant. Not obviously dead but a candidate for consolidation.
- **Confidence**: low

### `bin/shape-cli/src/commands/wire_serve_cmd.rs` (vs `serve_cmd.rs`)
- **Path**: `bin/shape-cli/src/commands/wire_serve_cmd.rs:1-100`
- **Why suspected**: `serve_cmd.rs` implements a richer protocol with framing/auth/QUIC and is the path the MCP server spawns. `wire-serve` speaks length-prefixed JSON over TCP only and is missing recent message types. Possibly superseded.
- **Confidence**: low
