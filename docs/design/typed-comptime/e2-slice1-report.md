# E2 #18 slice-1 report — typed replace-module transport (`CheckedModule`)

Landed on branch `adr009/e2`: commit `9ce54d3d` (typed transport + provenance
sequence + pins) and the follow-up doc/finding commit. Supervisor-ruled
**Option C** (2026-07-17): reuse the existing `item_fn` typed producer through a
fragment-slot route beside the legacy string route (the `parse_extend_items_slot`
precedent) — the ruled E2-D8 staging, not a parallel-implementation defection.

## Transport shape

`replace module (expr)` reaches the compiler by two complete, independent paths
sharing only the surface spelling:

- **legacy (U03)** — `expr` is a `string` (module source or AST-JSON).
  `__emit_replace_module`'s string arm → `parse_module_items_payload` →
  `ComptimeDirective::ReplaceModule` → the raw `*module_items = items` consumer.
  Byte-for-byte UNCHANGED; dies whole in slice 5.
- **typed (this slice)** — `expr` is a typed `__ComptimeItemFragment` (from
  `item_fn(...)`). `__emit_replace_module`'s fragment arm → `parse_extend_items_slot`
  fragment branch → `ComptimeDirective::ReplaceModuleChecked` → the module-target
  consumer routes it through `BytecodeCompiler::build_checked_module`. **No source
  or JSON string ever materializes on this path.**

`__emit_replace_module` is now slot-typed (`unknown` param); the route is
selected by the slot's runtime kind, never by a string round-trip. The two
directive variants + two producer arms are the two staged paths; slice 5 deletes
the legacy variant + string arm + `parse_module_items_payload` in one commit,
leaving the typed path untouched.

## Provenance / hygiene / reserve (ruling obligation 1 — satisfied)

The typed consumer runs the FULL sequence per generated item, identical to the
fresh-generated declaration-discovery pre-pass
(`materialize_computed_comptime_extends`), MINUS `register_function` (the
module-compile flow registers the replacement items itself):

`generated_free_fn_content` (fingerprint) → `anchor_generated_function_decl` →
`GeneratedNodePath::decl_root("module_fn:…")` → `GeneratedOrigin` →
`stamp_generated_closure_provenance` (GeneratedNodeIssuer) →
`reserve_generated_decl_journaled` (E3 hygienic `SymbolId`, through the
`InstallTransaction` journal — rolls back on a failed install). The result is a
`CheckedModule { items, exports }`. The typed route NEVER replicates the legacy
raw swap.

## NAMED FINDING (ruling obligation 2) — the legacy consumer's raw swap

The legacy `ComptimeDirective::ReplaceModule` consumer
(`statements.rs::process_comptime_directives_for_module`) does a **raw
`*module_items = items` with NO provenance stamp, NO hygienic reservation, and NO
anchor** — the exact class of gap E2 exists to fix (an already-checked-elsewhere
value flattened and reinstalled without identity/provenance). It is recorded
here as a finding and **deliberately NOT repaired**: the legacy path dies WHOLE
in the slice-5 deletion (E2-D8), and this program does not repair corpses.
Touchpoint: the `ReplaceModule { items }` arm in
`process_comptime_directives_for_module`.

## Single-item scope (ruling obligation 3)

`CheckedModule.items` is a `Vec<Item>` internally, but slice 1's only typed
producer (`item_fn`) yields exactly one function, so a slice-1 typed replacement
is a single-function module. Multi-item modules are a straight `Vec` extension
once a multi-item producer (`quote module { … }`) lands — no shape change to
`CheckedModule`. Disclosed at the type definition (`comptime_fragments/mod.rs`).

## `Exports` semantics (ruling obligation 4)

`Exports` = the reserved hygienic export symbols, one per generated declaration;
for slice 1 that is the single hygienic exported symbol of the one `item_fn`
function. Stated in the `CheckedModule` doc.

## Pre-analysis wiring (assignment obligation 2) — disposition

The slice-0 finding wired function/type-target directive materialization through
the `materialize_computed_comptime_extends` discovery loop (`functions_annotations.rs`
~:2216) + the `analysis_program` apply channel (~:2143). **That mechanism does
not carry module-target directives**, and the module case is not a bug to route
through it:

1. **Structural:** `declaration_discovery.rs::collect_declaration_discovery_targets`
   collects only `Struct` + `Function` targets and BY DESIGN excludes module
   handlers ("Module handlers and raw module `comptime` blocks are intentionally
   absent: they mutate module topology through separate pass-2 APIs and cannot
   join this fixed point without moving that topology under the same worklist").
   A module-target `replace module` handler runs in PASS-2 via
   `compile_module_decl` → `execute_module_comptime_handlers`, not the pre-pass.
2. **Analyzer visibility already exists:** after a replacement,
   `compile_module_decl` calls `recheck_replaced_module_items`, which re-runs
   `analyze_program_full` with the replacement patched in (`patch_reanalysis_module_items`).
   The analyzer DOES see the replaced items — in pass-2, via recheck.
3. **The C0911 closure-fact gap is moot for slice 1:** the fix that pre-analysis
   materialization exists to deliver (publishing an edited body's closure
   inference facts before analysis) only bites when the replacement carries a
   closure. Slice 1's typed producer `item_fn` mints only a literal-returning,
   closure-free function — there is no analysis-time closure fact to publish, so
   there is nothing for pre-analysis materialization to fix here.

**Disposition:** pre-analysis materialization of module replacements becomes
relevant only when a closure-bearing / multi-item producer (`quote module`)
lands. AT THAT POINT the required decision is **whether to add module targets to
the discovery worklist** — the topology change `declaration_discovery.rs`
explicitly declined — versus extending the existing `recheck_replaced_module_items`
re-analysis to publish generated closure facts. Both are larger than a slice-1
seam and one is a >10-file-STOP-adjacent topology change, so this is surfaced for
a ruling at the producer slice, not improvised now.

## Pins

- `comptime_fragments::CheckedModule` type unit tests.
- `tools/shape-test/tests/annotations_comptime/directives.rs`:
  `replace_module_typed_fragment_installs_and_runs` (=42),
  `replace_module_typed_fragment_rejected_on_function_target`; the existing
  legacy-string (`replace_module_from_source_string_replaces_items`) and
  malformed-source (`replace_module_payload_is_still_source_text`) rows are the
  untouched D6 parity controls.
- Native VM+JIT: `tests/smokes-jit-closure/e2-replace-module-checked.shape` +
  `bin/shape-cli/tests/cli/jit_c2_install_native.rs::e2_typed_replace_module_runs_natively_both_tiers`.

## Files near budget

`comptime_fragments/mod.rs` = ~130 lines (new, well under 500).
`comptime_builtins.rs` +59 (variant + slot producer only — no growth beyond the
minimal wiring). `functions_annotations.rs` +80 (the builder + two reject-arm
merges).
