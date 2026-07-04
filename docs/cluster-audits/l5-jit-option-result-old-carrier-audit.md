# L5 JIT Option/Result old-carrier audit

Date: 2026-06-26
Worktree: `shape-l5-jit-carrier`
Branch: `strict-flip-l5-jit-carrier`

## Scope

Audited JIT-owned references to the retired `HK_OK` / `HK_ERR` / `HK_SOME`
`UnifiedValue<u64>` carrier family and the modern typed carrier paths for
`Result` / `Option`.

No VM trait-object code, shape-value carrier definitions, wire/snapshot code,
or provenance tooling was edited.

## Containment landed

- `crates/shape-jit/src/ffi_symbols/object_symbols.rs`
  no longer registers or declares `jit_make_ok`, `jit_make_err`, or
  `jit_make_some` as Cranelift imports.
- `crates/shape-jit/src/ffi_refs.rs` and
  `crates/shape-jit/src/compiler/ffi_builder.rs` no longer expose
  `make_ok` / `make_err` / `make_some` `FuncRef` fields. New JIT code cannot
  select the retired producers through the usual FFI reference set.
- `crates/shape-jit/src/ffi/call_method/mod.rs` now treats legacy
  `HK_OK` / `HK_ERR` / `HK_SOME` receivers on the `UInt64` JIT-format method
  path as a structured surface: it records `pending_call_error` and deopts
  instead of dispatching to the old `UnifiedValue<u64>` Result method helper.
- `crates/shape-jit/src/ffi/call_method/result.rs` was deleted because it was
  only reachable through that retired dispatch arm.

## Remaining old-carrier hits

These are the remaining intentional `HK_OK` / `HK_ERR` / `HK_SOME` or
`jit_make_*` hits after containment:

| File | Hits | Disposition |
|---|---:|---|
| `crates/shape-jit/src/ffi/value_ffi.rs` | `HK_SOME`/`HK_OK`/`HK_ERR` constants and helper functions at lines 201-203, 367-429 | Compatibility definitions only. They are no longer reachable from generated JIT code through `FFIFuncRefs` or Cranelift imports. |
| `crates/shape-jit/src/ffi/result.rs` | legacy `jit_make_ok` / `jit_make_err` / `jit_make_some` and `jit_is_*` / unwrap helpers at lines 51-201; legacy test references at 491-533 | Rust-side compatibility/test surface. The canonical producer/accessor path in the same file is `jit_v2_make_result_*`, `jit_v2_make_option_*`, and `jit_arc_*` over `Arc<ResultData>` / `Arc<OptionData>`. |
| `crates/shape-jit/src/ffi/conversion.rs` | `is_ok_tag` / `is_err_tag` checks at lines 31 and 183; old-carrier formatting arms at lines 247-255 | Kind-blind conversion/format compatibility. Typed print paths dispatch by `NativeKind::Ptr(HeapKind::Option|Result)` to `jit_print_option` / `jit_print_result`; replacing these remaining helpers requires a larger `(bits, NativeKind)` conversion API, not a local carrier patch. |
| `crates/shape-jit/src/ffi/jit_release.rs` | reclaim arm for `HK_OK | HK_ERR | HK_SOME` at line 74 | Reclaim-only safety for any legacy `UnifiedValue<u64>` allocation that exists in compatibility tests or boundary conversion. Removing this before deleting every old producer would risk leaks. |
| `crates/shape-jit/src/ffi/call_method/mod.rs` | `HK_OK | HK_ERR | HK_SOME` at line 1040 | New containment: structured deopt, not a method implementation. |
| `crates/shape-jit/src/ffi_symbols/object_symbols.rs`, `ffi_refs.rs`, `compiler/ffi_builder.rs` | comments only | Documents that legacy producer imports are intentionally absent. |

## Typed paths audited

- `StatementKind::EnumStore` lowers `Ok` / `Err` / `Some` / `None` to
  `jit_v2_make_result_ok`, `jit_v2_make_result_err`,
  `jit_v2_make_option_some`, and `jit_v2_make_option_none`.
- `Rvalue::EnumTest` and `Rvalue::EnumPayload` use `jit_arc_result_*` and
  `jit_arc_option_*` accessors against `Arc<ResultData>` / `Arc<OptionData>`.
- `HashMap.get` return typing remains `NativeKind::Ptr(HeapKind::Option)`.
- Ownership/refcount dispatch uses `jit_arc_result_retain/release` and
  `jit_arc_option_retain/release` for `Ptr(Result)` / `Ptr(Option)` slots.
- `?` and `??` deopts remain intact through `has_try_unwrap_residual` and
  `has_null_coalesce_residual`.

## Deferred design work

The remaining conversion/format compatibility hits should be removed only with
a typed conversion design that threads `NativeKind` through the relevant FFI
surface. A local rewrite from old wrapper bits to schema-backed enum lowering
would cross the VM/value carrier boundary and is outside this L5 JIT-only scope.

## Verification

- `nix shell nixpkgs#clang -c direnv exec /home/dev/dev/shape-lang/shape-l5-jit-carrier cargo check -p shape-jit --lib`
- `nix shell nixpkgs#clang -c direnv exec /home/dev/dev/shape-lang/shape-l5-jit-carrier cargo test -p shape-jit --lib ffi::result::tests::arc_ -- --nocapture`

Both commands passed. The `nix shell nixpkgs#clang` wrapper was needed because
the ambient NixOS shell had no `cc` linker in `PATH`.
