# E1 #17 slice-6 — pure-deletion report

ADR-009 E1 #17 slice 6 (close-out). Worktree
`/home/dev/dev/shape-lang/shape-adr009-a3`, branch `adr009/e1`. Sequencing per
E2-D8: per-commit-green **A → pure-deletion → B**.

## Commits

| Phase | Commit | What |
|---|---|---|
| A (pre-existing) | `05a62a91` | strengthened anti-walk-back sentinel through the full consumer (`e1_s5_stamped_unresolvable_ref_errs_through_full_consumer_never_reparses_valid_source`) |
| Pure deletion | `07638332` | the three fully-dead JSON-protocol orphans — **production code only, 0 additions / 39 deletions** |
| B (pins + docs) | this commit | absence sentinel + inventory + this report |

## Scope — exactly three fully-dead items (nothing else)

The slice-6 deletable set is EXACTLY the three carried-forward fully-dead items.
The four **primary orphan targets** (`.source` field, the reparse arm,
`parse_type_annotation_payload` the fn, `__type_probe`) **SURVIVE E1** — they are
still live for unstamped (`identity == INVALID`) refs (module/expression targets,
the unresolved fallback, optional fields, every non-reconstructable type), the
user-ratified E1-D8 stamp-gate residual **bound to B4/B5 → E5**. Deleting any of
them was out of slice-6 scope.

## Fresh-context sweep (re-run at HEAD `2d1f627f`, pre-deletion)

The capstone re-ran the closure sweep independently and found **zero divergence**
from the slice-5/6 orphan inventory. No inventory-missed hit surfaced.

| Target | Deleted range (pre-deletion `2d1f627f`) | Uniqueness evidence |
|---|---|---|
| (a) `fn serialize_directive_payload` | `statements.rs:144-161` (doc 144-147 + fn 148-161) | `rg 'serialize_directive_payload'` minus the `fn` line = **0 invocations**; all 7 other hits are prose comments (`statements.rs:621,660,724`; `comptime_builtins.rs:174,191,1717,1718`). |
| (b) `"__emit_extend"` builtin | `comptime_builtins.rs:1534-1548` (comment + `register_typed_fn_1` block incl. inline consumer + `serde_json::from_str` of `ExtendStatement` @ 1543) | exact `"__emit_extend"` string = **1 site** (the registration @ 1537); emit side emits the typed-index `"__emit_extend_checked"` (`statements.rs:607`). |
| (c) `serde_json::from_str::<TypeAnnotation>` first branch of `parse_type_annotation_payload` | `comptime_builtins.rs:365-368` (branch + trailing blank) | `serde_json::from_str::<…TypeAnnotation>` = **1 site** (365); fn sig (364) + `__type_probe` remainder (369-381) survive byte-unchanged. |

The `serde_json::from_str::<…ExtendStatement>` turbofish sweep returned empty —
expected: the `ExtendStatement` parse is a `let`-annotation
(`let extend: shape_ast::ast::ExtendStatement = serde_json::from_str(...)`) inside
the deleted (b) block, not a turbofish. Not a divergence.

Cross-cutting: **zero** test / `.shape` / toml / json / pest references to any of
the three targets (only the two production `.rs` files matched). Nothing to
migrate or retire.

## Deletion evidence (commit `07638332`)

```
0  20  crates/shape-vm/src/compiler/comptime_builtins.rs
0  19  crates/shape-vm/src/compiler/statements.rs
2 files changed, 39 deletions(-)
```

Pure deletion — **0 insertions**. No survivor-set edits beyond what the deletion
mechanically forces. Both files referenced serde via fully-qualified paths only
(`serde_json::` / `serde::Serialize`); there was no `use serde_json` / `use serde`
line to orphan. Post-deletion, **zero `serde_json`/`serde` code uses remain** in
either file (only prose comments), so no unused-import warning.

## Survivor-set compile proof (post-deletion, `07638332`)

The E1-D8 residual survivors compile and pass byte-unchanged:

- `parse_type_annotation_payload` (fn sig `364` + `__type_probe` remainder
  `369-381`) — survivor callers at `431` (string-slot entry) and `490` (`.source`
  reparse arm).
- `type_annotation_from_string_or_type_ref_slot`, `string_field_from_typed_object`,
  `reconstruct_type_annotation` — the stamped identity route + unstamped `.source`
  arm both intact (short-circuit `487`, `.source` read `489`).
- `__emit_extend_checked` / `__emit_extend_items` typed carriers untouched;
  `ComptimeDirective::Extend` survives (consumed by `__emit_extend_checked`).

### Lane gates (green)

```
cargo check -p shape-vm --all-targets   → 0 errors.
    The serialize_directive_payload dead-code warning is GONE (expected —
    deleting the caller-less fn clears it). Remaining 16 warnings are
    pre-existing slice-1 CheckedBody greenfield (checked_body.rs / module
    exports), untouched by this deletion; 0 new warnings introduced.
cargo test -p shape-vm --lib e1_        → 21 passed; 0 failed; 0 ignored.
    Includes the A-phase anti-walk-back sentinel and the unstamped-path guard
    e1_s5_unstamped_typeref_falls_through_to_source_arm_bytewise (proves the
    surviving .source reparse arm is still reached).
```

## B-phase absence sentinel

`crates/shape-vm/src/executor/tests/no_json_comptime_protocol.rs` (registered in
`crates/shape-vm/src/executor/tests/mod.rs`). Mirrors `no_dynamic.rs`: scans the
`crates/`, `bin/`, `tools/`, `extensions/` source trees at the Rust-test layer and
fails the build if either deleted symbol reappears. Two precise (non-prefix)
needles, assembled from fragments so the sentinel never self-trips:

- `no_serialize_directive_payload_fn` — the `fn serialize_directive_payload`
  definition (the `fn ` keyword excludes surviving backtick prose mentions).
- `no_emit_extend_json_builtin_registration` — the exact double-quoted
  `"__emit_extend"` literal (the closing quote excludes the survivors
  `"__emit_extend_checked"` / `"__emit_extend_items"`).

Both needles verified to **0 hits** across the scan scope at deletion time.
