# C3 #14 — Slice 6 report: S6 completion (collapse → dark window → pure deletion → absence pins)

Authority: `c3-decisions.md` C3-G7 (the deletion charter) + C3-G14 (A′,
user-ratified 2026-07-21: the @remote cut). Slice-5 wave-1 ledger and the
S3/S5 rejection-sentence conventions carried over. CLAUDE.md Forbidden
Patterns at maximum binding throughout — no walk-back phrase survived
review, no compatibility shim exists, nothing was renamed to keep it.

## 1. The commit chain (all append-only on `adr009/c3`)

| Commit | Head | Content |
|---|---|---|
| `7a7bd0ff` | S6-A | A-phase pin typing (9/13), collapse HELD |
| `6ee58b35`..`ae6309a4` | S6 fixlet | exit-soundness class CLOSED (4 commits) |
| `10fcf533` | S6-b | the @remote dark window (C3-G14 A′) |
| `cf108a70` | S6-c | the classification collapse — one surface |
| `d2ad9c5f` | S6-d | **the pure deletion** (this capstone, commit 1) |
| (this commit) | S6-e | absence sentinel + this report (commit 2) |

The capstone ran FRESH-CONTEXT: the pre-deletion sweep was re-derived
from the tree at `cf108a70` before any edit. **Zero divergences** from
the dispatched sweep inventory were found; one addition surfaced by the
collapse itself (the `weave.rs` defensive invariant calling
`find_compiled_annotations` — deleted with its callee, see §2.9).

## 2. Deletion evidence (commit `d2ad9c5f`: +117 / −2161 across 19 files)

Production deletions + mechanically-forced edits ONLY. E4's fence
untouched (remote.shape keep-set, `__call_*` elaboration, HookDecision
territory all as at `cf108a70`).

1. **`compiler/functions_annotations.rs` (−1185; 6761 → 5576 lines).**
   Three contiguous ranges, closed under callers:
   - the homogeneous-args-array derivation trio (element-annotation
     derivation with its heterogeneous-element hard error, the
     array-kind derivation with its heterogeneous-carrier hard error,
     the args-array emitter). `annotation_param_type_annotation`
     SURVIVES — non-legacy reader `template_specialization/mod.rs`
     (the new weave's per-param typing);
   - the two legacy hook-chain hygienic name minters (impl-body +
     chain-wrapper). `original_body_shadow_name` (C2 replace-body)
     SURVIVES beside them;
   - the whole legacy weave block: `find_compiled_annotations`, the
     chained-annotations compiler, the wrapped-function compiler, the
     five ctx/result/literal/expr/simple-parameter annotation helpers,
     the specialized runtime-handler compiler (**self-as-f64** receiver
     + the **name-keyed args/result/ctx magic params**), the per-target
     runtime-handler specializer (sole reader of the handler-template
     clones), and the raw-bytecode wrapper emitter (homogeneous args
     array, **per-invocation config eval**, the `{result:}`
     before-short-circuit — the HookDecision-protocol precursor C3-G14
     names).
2. **`compiler/functions.rs`.** The dead selector branch head (the
   `len()==1` / `len()>1` routing + the now-callee-less
   `body_pass_modes` binding) and the `has_runtime_annotation_hooks`
   mir_data-suppression arm. `compile_function_inner` has ONE
   compilation path; the replacing invariant is the typed weave's
   wrapper-mir pin (`weave.rs`
   `concrete_before_mutation_is_observed_in_output` asserts
   `wrapper.mir_data.is_some()`).
3. **`compiler/functions_foreign.rs` (−99).** The foreign-fn
   annotation-wrapping block — @remote's only consumer (dark per
   C3-G14). See §6.2 for the resulting surface fact.
4. **`compiler/expressions/mod.rs` (−577).** Both legacy emission
   blocks in the expression/await annotation sites (each keeps its
   target-validation + comptime-handler head) + the legacy-closed
   before-result-contract helper family (plain, with-short-circuit,
   inner).
5. **`bytecode/core_types.rs`.** `CompiledAnnotation` loses the two
   serialized compiled-handler-id slots (`Option<u16>`) and the two
   per-target handler-template AST-clone fields (`#[serde(skip)]`).
   Serde impact: fields removed from the serialized shape; serde's
   default unknown-field tolerance covers old data (greenfield surface,
   no compat weight per the 2026-07-20 ruling).
6. **`compiler/compiler_impl_reference_model.rs`.** The two
   legacy-closed annotation-name helpers (match-by-compiled-name,
   args-for-compiled-name). `lookup_compiled_annotation` survives (many
   non-legacy callers).
7. **`compiler/template_specialization/install_registry.rs`.** The
   C3-G7-transitional mixed-legacy one-weave-owner rejection —
   unconstructible post-deletion (its pin was retired-unconstructible
   at `cf108a70`); step numbering + module docs mechanically renumbered.
8. **`compiler/comptime_builtins/expansion_provenance.rs`.** The 9
   orphaned legacy `HygienicRole` variants (args/ctx/subject/result/
   before-result locals; specialized-handler + foreign-wrapper registry
   labels; both S4 hook-chain identities) + their `canonical_descriptor`
   arms. The two role-exemplar unit tests rewritten IN PLACE (same
   names) onto surviving roles (`ComptimeTargetBinding`/
   `ComptimeCtxBinding`; `TemplateWeaveImplBody`/
   `AnnotationSugarHookBody`). Roles are compile-time name-mint inputs
   only — nothing persisted references the deleted variants.
9. **`compiler/template_specialization/weave.rs`.** The defensive
   legacy-classification internal error (it called the deleted
   `find_compiled_annotations`; the collapse made the guarded state
   unconstructible — installer never populates handler slots). Module
   docs rewritten to deletion-fate.
10. **Comment/doc rewires (mechanically forced — they named deleted
    symbols):** `planner.rs`, `installer.rs`, `pseudo_tuple.rs`,
    `template_specialization/mod.rs`, `statements.rs` (the two
    field-nonexistence asserts retired — the fields do not exist; the
    absence sentinel §5 guards re-introduction), and the 5 test-file
    `CompiledAnnotation` literals dropping deleted initializers
    (e1_param_selection, handler_helper_authority,
    imported_handler_resolution, import_permissions/denial,
    annotation_import_pipeline).

## 3. The one-implementation proof

At `d2ad9c5f` (re-verified at this commit), grep over `crates/ tools/
bin/ extensions/` `*.rs` for EVERY deleted name — the six weave fns, the
five type-annotation helpers, the args-array trio, the two name minters,
the two reference-model helpers, the before-result-contract family, the
four carrier fields, the 9 HygienicRole variants, the S4 classification
enum path — returns **ZERO hits**. Not even tombstones spell them
contiguously: surviving comments describe deleted code by deletion-fate
("the deleted legacy weave", "the S6 capstone deleted…"), per the
Forbidden Patterns naming rule. The only near-spelling in the tree is
the unrelated import-permissions test fn
`denied_annotation_import_stops_before_handler_publication` (a
substring collision in a snake_case name, not a reference; the sentinel
needle is field-declaration-shaped and does not match it).

Runtime `before`/`after` hooks now have exactly ONE implementation: the
typed hook-template weave (sugar lowering onto the public comptime API →
per-target specialization with baked captures → ordinary typed-AST
wrapper through the ordinary pipeline — bytecode AND MIR from the same
wrapped definition, zero mir_data suppression, zero selector).

## 4. Suite arithmetic (pre-declared → actual; lane, `-j1`/`--test-threads=1`)

Pre-declared for commit 1: **zero test-count movement, FAILED name-sets
byte-identical to the `cf108a70` baseline** (commit 1 deletes no test
fn and adds none; two expansion_provenance tests rewritten in place,
two statements.rs tests lose one assert each). Pre-declared for commit
2: **+4 shape-vm lib tests** (the absence sentinel), nothing else.

| Suite | `cf108a70` baseline | `d2ad9c5f` actual | commit 2 |
|---|---|---|---|
| shape-vm `--lib` | 3496 passed / 36 ignored / 6-name FAILED | **3496 / 36 / 6-name BYTE-IDENTICAL** | 3501 / 36 / 6-name (+4 total; flap member green this run) |
| annotations_runtime | 36/36 | **36/36** | — |
| annotation_targets | 24/24 | **24/24** | — |
| annotations_comptime | 116 passed / 10-name FAILED | **116 / 10-name byte-identical** | — |
| comptime | 260 passed / 3-name FAILED | **260 / 3-name byte-identical** | — |
| lsp (shape-test) | 502/502 | **502/502** | — |
| shape-lsp `--lib` | 882/882 | **882/882** | — |
| cli_tests | 57/57 | **57/57** (311s) | — |

c3 filters at `d2ad9c5f`: template_specialization 259/259,
functions_annotations 91/91, annotation_declarations 34/34.
`just check-clean` exit 0 (both commits); `just check-no-dynamic` exit 0.
Flap accounting: the vmlib run showed +1 red of the DOCUMENTED
`nested_exact` flap member; protocol run (4× isolated `--exact` on the
same binary) = 2 green / 2 red — the S4d nondeterminism signature,
KNOWN FLAP, not of record. Refused-regex grep over both diffs: zero
hits. The one `cargo check` warning is the PRE-EXISTING
comptime_builtins.rs test-scope unused import, on record since S2.

## 5. B-pins: the absence sentinel

`crates/shape-vm/src/executor/tests/no_legacy_annotation_weave.rs` — 4
tests on the `no_json_comptime_protocol.rs` precedent (fs-walk over
`crates/ bin/ tools/ extensions/` `*.rs`; needles assembled from
fragments at runtime; prose describes needles by part only):

1. `no_legacy_weave_functions` — the five deleted fn-name needles
   (specialized-handler compiler, runtime-handler specializer, wrapper
   emitter, both args-array derivation names).
2. `no_legacy_handler_carrier_fields` — the two handler-id field
   declarations (type-anchored `Option<u16>` needles, immune to the
   snake_case test-name substring) + the two handler-template names.
3. `no_legacy_surface_classification` — the S4 evidence-enum path
   needle (`::`-anchored so the prose tombstone in
   `annotation_declarations.rs` does not trip it).
4. `remote_stdlib_module_carries_no_annotation_block` — C3-G14 A′:
   `std::core/remote.shape` contains no annotation declaration (both
   `pub annotation` and the named declaration-head needles), with a
   positive keep-set guard (`remote.execute` builtin export present) as
   the non-vacuity anchor.

**Negative control (non-vacuity proven mechanically):** a needle planted
in `tools/xtask/` flipped `no_legacy_weave_functions` to FAILED; probe
removed; suite green again. 4/4 at this commit.

## 6. The dark-window ledger (C3-G14 A′ — everything dark, and why)

### 6.1 `@remote` (the cut, commit `10fcf533`)

`std::core/remote.shape` lines 165–191 (the `pub annotation remote(addr)`
block: untyped config + `ctx.target` + the `{result:}` short-circuit)
replaced by the C3-G14 dark-window comment. KEEP-set stays live:
`remote.execute`, `remote.ping`, `__call_raising`, `__call_result`,
`__call_async_result`, `enum RemoteError` (native registrations + export
pins untouched; `__call_raising` kept for E4 per the S6-b disposition).
No autoload edit was needed (build.rs embeds stdlib-src/ wholesale; only
the `std::core::*` filter autoloads).

**21 tests `#[ignore = "dark window: E4 re-implements @remote on typed
HookDecision — see issue #68"]`** — E4's acceptance suite:

- `distributed_snapshot_polyglot_e2e.rs` (6):
  `remote_snapshot_returns_receiver_hash_over_remote_call`,
  `remote_extern_c_transfer_executes_and_strict_node_refuses_ffi`,
  `remote_python_transfer_self_skips_without_extension_and_refuses_without_opt_in`,
  `remote_typescript_transfer_self_skips_without_extension_and_refuses_without_opt_in`,
  `remote_snapshot_hash_is_saved_in_selected_receiver_store`,
  `remote_snapshot_hash_can_be_resumed_from_receiver_store`.
  (The file's other 8 tests ride `remote.execute`/`remote::call` — running.)
- `distributed_extern_c_snapshot_e2e.rs` (1, whole file):
  `remote_extern_c_snapshot_hash_can_be_resumed_from_receiver_store`.
- `distributed_composition_e2e.rs` (2):
  `tls_remote_python_snapshot_hash_can_be_resumed_from_selected_receiver_store`,
  `tls_remote_typescript_snapshot_hash_can_be_resumed_from_selected_receiver_store`.
- `distributed_matrix_e2e.rs` (4):
  `remote_python_call_refuses_receiver_without_language_opt_in`,
  `remote_typescript_call_refuses_receiver_without_language_opt_in`,
  `plaintext_remote_snapshot_uses_receiver_store_not_caller_store`,
  `tls_remote_snapshot_uses_receiver_store_not_caller_store`.
  (The `tls_remote_call_refuses_*` pair rides `remote::call` — running.)
- `distributed_dynamic_snapshot_e2e.rs` (2):
  `remote_python_snapshot_hash_can_be_resumed_from_receiver_store`,
  `remote_typescript_snapshot_hash_can_be_resumed_from_receiver_store`.
- `serve_cmd.rs` in-crate (3):
  `test_remote_foreign_extern_c_transfer_over_tcp`,
  `test_remote_foreign_python_transfer_over_tcp`,
  `test_remote_foreign_typescript_transfer_over_tcp`.
- `modules_visibility/scoped_contract.rs` (3, the W9 cross-module
  annotation-import trio — @remote was its only stdlib exemplar):
  `scoped_contract_namespace_annotation_refs_use_double_colon`,
  `scoped_contract_named_annotation_import_enables_bare_annotation`,
  `scoped_contract_namespace_import_binds_bare_annotations`.
  **Coverage note:** W9 stdlib-annotation import coverage is ZERO during
  the dark window; recommended follow-up (named at S6-b): re-target ONE
  row onto `@json_schema`/`@to_json` import.

### 6.2 The S2-F3 ctx pins (RETIRED, not ignored)

`injection.rs::before_hook_passes_ctx_info` and
`before_after.rs::ctx_target_calls_original_impl_from_after_hook`
DELETED with #68 dark-window comments at their sites (G14's wording:
"retired … with the same #68 pointer"). Rationale: their untyped
fixtures cannot compile post-collapse and E4's typed HookDecision
surface will differ — an ignored fixture could never be un-ignored
verbatim. Same class, same treatment: `comptime/annotations.rs`
`ct_30_annotation_ctx` (printed ctx). The `#[ignore]` alternative
remains available if the supervisor wants symmetry with the e2e suite.

### 6.3 `@indicator` (finance) — the second @remote-class case

Zero-param + on_define + ctx-cached `{result:}` short-circuit — cannot
fit the typed surface (ctx + HookDecision = E4). S6-b disposition: the
before/after cache hooks + the on_define registry registration went dark
with a #68 note; the lifecycle `metadata()` handler survives and now
truthfully reports `cacheable: false`. No test consumers; module not
autoloaded.

### 6.4 Foreign-target hooks (sweep Risk 5 — probed, documented FACT)

CLI probe at `d2ad9c5f`: a typed declarative-hook annotation
(`before(args)`) applied to `extern "C" fn` **compiles and runs with the
hook as a silent no-op** (no rejection, hook does not fire, foreign call
proceeds). NOT introduced by the deletion: the deleted foreign-wrapper
block fired only on populated legacy handler slots, which the `cf108a70`
collapse made permanently `None` — the block was already unreachable, so
behavior is identical before/after commit 1. The typed weave targets
ordinary `FunctionDef`s only; hooks-on-foreign-targets is E4/#68
re-implementation territory (E4's serve_cmd acceptance tests above cover
exactly this surface). **Open disposition surfaced to the supervisor:**
whether to add a named surface-and-stop rejection for
declarative-hook-annotation-on-foreign-target during the dark window
(commit-1 purity forbade adding product behavior; this report is the
loud surface).

### 6.5 Out-of-gate residue (accepted-but-ledgered)

- `tools/vmjit-diff/corpus/` legacy/@remote fixtures (corpus data, in no
  gate).
- The book (`../shape-web`) documents @remote + untyped-annotation
  examples — an explicit #68/E4 obligation under the book truth-gate
  rule (out of this worktree).
- `docs/cluster-audits/` archived programs mention the deleted names
  (docs trees are not scanned by the sentinel, by design).

### 6.6 Surfaced findings from the collapse (not capstone-caused, tracked)

- **C0902 provenance drop (2 ignores, needs issue + disposition):**
  `functions/reference_provenance_tests.rs`
  `single_runtime_annotation_preserves_shared_reference_provenance` +
  `chained_runtime_annotations_preserve_exclusive_reference_provenance`
  — the weave impl shadow drops inferred pass-mode provenance for
  stamped declared-capture closures; needs shadow provenance threading
  (own workstream).
- The `modules_visibility` 1 pre-existing failure
  (`scoped_contract_snapshot_requires_explicit_import`, proven at
  `dfcad3bd` via stash differential — main-merge snapshot surface, not
  C3).

## 7. Forbidden-patterns audit of this capstone

No fallback was kept "for one edge case"; no rename; no feature flag; no
"document as out-of-scope" for anything the charter named. The two
judgment calls both went the DELETE direction: (i) the weave's defensive
legacy-classification check — deleted (guarded state unconstructible),
not rewritten to a new probe; (ii) the 9 HygienicRole variants — deleted
(G7: deletion is cleaner), not left inert. The foreign-target no-op
(§6.4) is surfaced, not rationalized. Refused-regex over both diffs:
zero hits.
