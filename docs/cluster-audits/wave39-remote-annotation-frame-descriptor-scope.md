# Wave-39O: `@remote` implementation frame descriptor

## Finding

The descriptor is already empty on the sender. The receiver is not dropping
metadata.

1. `compile_wrapped_function` and `compile_chained_annotations` clone the
   original signature into `{name}___impl`, register it, and call
   `compile_function_body` (`crates/shape-vm/src/compiler/functions_annotations.rs:2476-2493`,
   `2368-2385`). The body compiler does call
   `capture_function_local_storage_hints_with_def` before finalizing the blob
   (`crates/shape-vm/src/compiler/functions.rs:2576-2585`; the early implicit
   return has the same call at `2439-2446`).
2. That capture currently builds `proven_hints` only from
   `type_tracker.get_local_storage_hint` (`crates/shape-vm/src/compiler/helpers.rs:3728-3730`).
   Its WF-3E parameter-prefix fallback repeats that lookup
   (`helpers.rs:3799-3827`). `Array<int>` is a proven structural
   `ConcreteType::Array`, but `VariableTypeInfo::named/known` does not put a
   storage hint on structural heap types. Therefore the generated
   implementation has no proven parameter prefix and its descriptor has zero
   slots (or no usable descriptor), matching the receiver error.
3. `FunctionBlobBuilder::finalize` copies `func.frame_descriptor` into the
   blob (`crates/shape-vm/src/compiler/mod.rs:524-537`); the descriptor is also
   hash-covered (`crates/shape-vm/src/bytecode/content_addressed.rs:133-172`).
   `program_from_blobs_by_hash` retains the blob, and both linker paths copy
   `blob.frame_descriptor` into the linked function (`crates/shape-vm/src/linker.rs:500-513,
   628-641`). The receiver then reads those slots and rejects a short prefix
   at `crates/shape-vm/src/remote.rs:1278-1303`. There is no transfer-side
   erase point. The inbound `blobs=3` observation is consistent with a
   correctly transferred but already-empty `compute___impl` descriptor.

## Smallest truthful fix

Change only the parameter-prefix derivation in
`crates/shape-vm/src/compiler/helpers.rs` near lines 3820-3827:

- Keep `type_tracker.get_local_storage_hint(slot)` as the first choice.
- When it is absent and `func_def` is available, resolve the corresponding
  declared parameter annotation with the existing
  `declared_annotation_concrete_type` helper, then map it with
  `shape_value::v2::closure_layout::native_kind_from_concrete_type`.
- Collect only fully resolved parameters. Do not add an `Unknown` sentinel or
  infer a kind from runtime values. For `data: Array<int>` this yields
  `FrameDescriptor.slots[0] = Ptr(HeapKind::TypedArray)`.
- Leave full-local `function_local_storage_hints` behavior unchanged. The
  remote ABI needs only the proven parameter prefix; later unproven locals
  must remain polymorphic.

This is narrower and safer than teaching every structural `VariableTypeInfo`
constructor to carry a storage hint: that would change local/JIT ownership and
typed-opcode decisions outside the remote ABI. It also avoids fabricating a
kind for unresolved, generic, borrowed, or dynamic annotations.

## Why `compile_function` is not the fix

The prior generated-method change at
`crates/shape-vm/src/compiler/functions_annotations.rs:1416-1422` switched
comptime-generated methods from `compile_function_body` to `compile_function`
to produce MIR for JIT Phase 4. It does not make structural parameter storage
hints appear, so applying the same change to annotation implementations would
not repair this descriptor. It would also broaden the annotation path merely
to obtain MIR.

The generated `___impl` has no annotations, so a full-driver call would not
recursively re-wrap it, but it would change its MIR/JIT metadata surface. The
annotated public wrapper is intentionally left without JIT MIR when it carries
runtime hooks (`functions.rs:1130-1160`), forcing the wrapper through VM bytecode
so `@remote` hooks are preserved. The descriptor fix must not alter that
wrapper/JIT policy. The receiver ABI is VM-side and needs only the impl blob's
declared parameter kind.

## Regression boundary

1. **Compiler assertion:** add a focused compiler test beside the existing
   frame metadata tests in `crates/shape-vm/src/compiler/helpers.rs` (the
   `frame_return_metadata_tests` module near lines 8227-8320). Compile an
   annotated function with `data: Array<int>`, locate `compute___impl`, and
   assert its descriptor has one slot of
   `NativeKind::Ptr(HeapKind::TypedArray)`. Assert the wrapper separately only
   for its existing hook/JIT policy; do not require wrapper parameter slots.
2. **Real socket boundary:** add or extend the loopback CLI E2E boundary in
   `bin/shape-cli/tests/distributed_snapshot_polyglot_e2e.rs` with
   `@remote` `fn compute(data: Array<int>)` and a result that reads the array.
   Assert the receiver returns the expected value. This specifically proves
   the `compute___impl` blob, content-addressed reconstruction, linker, and
   receiver marshal path, beyond the existing nested-array positional-pack
   unit proof.
3. **Book boundary:** rerun the three currently failing `@remote` rows in the
   sibling book pages `stdlib/core/remote.mdx` and `advanced/annotations.mdx`.
   The close condition is the existing full release-binary book gate moving
   from `562/565` with exactly those three failures to `565/565`; no unrelated
   disabled rows should change.

## Ordered implementation and verification

1. Add the annotation-to-`NativeKind` parameter-prefix fallback in
   `helpers.rs`; preserve tracker-first precedence and reject unresolved
   projections by returning `None` from the collection.
2. Add the focused `compute___impl` descriptor assertion.
3. Run the focused compiler test under the single verification lane:

   ```sh
   systemd-run --user --wait --collect --pipe \
     --property=MemorySwapMax=0 --property=MemoryMax=12G --property=TasksMax=256 \
     env CARGO_BUILD_JOBS=2 cargo test -p shape-vm \
     frame_return_metadata_tests::annotated_array_parameter_stamps_frame_descriptor -- --exact
   ```

4. Add/run the real-socket regression under the same lane:

   ```sh
   systemd-run --user --wait --collect --pipe \
     --property=MemorySwapMax=0 --property=MemoryMax=12G --property=TasksMax=256 \
     env CARGO_BUILD_JOBS=2 cargo test -p shape-cli \
     --test distributed_snapshot_polyglot_e2e remote_annotation_typed_array_argument_over_shape_serve \
     -- --exact --nocapture
   ```

5. Run the authoritative release-binary book gate with the supervisor's
   existing book-site command, using the broad lane limits
   `MemoryMax=24G`, `TasksMax=512`, `CARGO_BUILD_JOBS=2`; require `565/565`.
   Finish with `git diff --check`.

No production, test, book, or script changes were made by this scout.
