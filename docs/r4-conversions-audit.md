# R4.1 Audit: JIT NaN-box Conversion Helper Callers

Phase R4.1 of `/home/dev/.claude/plans/v2-residuals-closeout.md` §R4.

R4 eliminates `box_to_nanboxed` / `unbox_from_nanboxed` / `ensure_nanboxed`
from `crates/shape-jit/src/mir_compiler/conversions.rs` by rewriting their
callers. R4.1 is audit-only: this document classifies every caller so R4.2
can rewrite them mechanically, one FFI family per commit, and R4.3 can
delete the helpers once zero callers remain.

## Counts

```
$ grep -rn 'box_to_nanboxed\|unbox_from_nanboxed\|ensure_nanboxed' \
    crates/ --include='*.rs' | wc -l
43
```

Of the 43 lines:

- **3 defining lines** in `conversions.rs` (`fn box_to_nanboxed`,
  `fn unbox_from_nanboxed`, `fn ensure_nanboxed` signatures) — not call
  sites.
- **3 internal call sites** in `conversions.rs` (`ensure_kind` and
  `convert_between` route through the helpers) — these disappear together
  with the helpers themselves when R4.3 deletes the module.
- **2 documentation references** (ownership.rs:67 comment, object/closure.rs:280
  comment) — no runtime behaviour, will be updated in R4.3.
- **35 actual caller sites** outside `conversions.rs` — the audit targets.

Per-helper:

- `box_to_nanboxed`: 3 total → all inside `conversions.rs`. Zero external
  callers. **Can be deleted immediately by R4.3 once its two in-module
  callers (`ensure_kind`, `convert_between`) are also deleted.**
- `unbox_from_nanboxed`: 5 total → 2 internal (`ensure_kind`,
  `convert_between`), 3 external callers (`mod.rs:413`,
  `compiler/program.rs:420`, `conversions.rs:134`).
- `ensure_nanboxed`: 35 total → 1 definition, 2 internal callers in
  `convert_between`/`ensure_kind` paths, 32 external call sites.

External call-site classification: **C = 35 class-(a)**, **D = 0 pure
class-(b)**. There are **no opaque-handle reinterpretations** that
accidentally route through the helpers — every site that reaches FFI or
the uniform VM stack/ABI is a class-(a) native-signature-rewrite target.

Three sub-groups exist inside class-(a) that R4.2 will route through
different commits:

1. **Class-(a) FFI**: direct FFI calls where the callee NaN-unboxes. R4.2
   rewrites the FFI signature to a native type and deletes the
   `ensure_nanboxed` at the call site.
2. **Class-(a) VM-stack push (call ABI)**: values stored into
   `ctx.stack[sp+i]` for method-dispatch / indirect calls / closure
   captures. R4.2 (paired with the ABI v2 work) defines the VM stack as
   raw typed slots so the box becomes a native typed store.
3. **Class-(a) borrow/deref stack-slot storage**: Cranelift `StackSlot`
   cells backing `&T` / `&mut T` references. The slot currently stores
   NaN-boxed I64 because the borrow may outlive a typed def_var. R4.2
   promotes the reference cell to hold the root local's native type.

The three sub-groups map one-to-one to the commit plan below.

## Callers by file

Line numbers refer to HEAD `11de41e` on branch `jit-v2-phase1`.

### crates/shape-jit/src/mir_compiler/rvalues.rs — 5 calls

- **L71** `ensure_nanboxed(val)` in `Rvalue::Clone`. Feeds `ffi.arc_retain`.
  **Class-(a) FFI / arc family.** `jit_arc_retain` takes the heap-pointer
  payload; signature becomes `fn(*const HeapHeader) -> ()` (or the
  `u64`-as-handle form, depending on what R4.2F's arc rewrite picks).

- **L81** `ensure_nanboxed(raw_val)` in `Rvalue::Borrow`. Stores into a
  Cranelift `StackSlot` as the backing cell for a reference.
  **Class-(a) borrow/deref storage.** Paired with `mod.rs:413`'s unbox.

- **L109** `ensure_nanboxed(raw)` in `Rvalue::Aggregate` array-literal loop.
  Feeds `ffi.array_push_elem`. **Class-(a) FFI / array family.**

- **L370, L371** `ensure_nanboxed(lhs)` / `ensure_nanboxed(rhs)` in
  `compile_binop` (the generic fallback for dynamic binops). Feeds
  `ffi.generic_add` / `generic_sub` / `generic_mul` / etc.
  **Class-(a) FFI / conversion family** (the `generic_builtin.rs` FFIs
  live in the "conversion" tranche of R4.2).

### crates/shape-jit/src/mir_compiler/places.rs — 7 calls

- **L217** `ensure_nanboxed(raw_base)` in `Place::Field` read. Feeds
  `ffi.get_prop` and `inline_typed_field_get`. **Class-(a) FFI / object
  family.**

- **L244** `ensure_nanboxed(raw_base)` in `Place::Index` read. Feeds
  `inline_array_get`. **Class-(a) FFI / array family.**

- **L276, L277** `ensure_nanboxed(raw_base)` and `ensure_nanboxed(val)` in
  `Place::Field` write. Feeds `ffi.set_prop` and
  `ffi.typed_object_set_field`. **Class-(a) FFI / object family** (two
  calls, same site).

- **L306, L309** `ensure_nanboxed(raw_base)` and `ensure_nanboxed(val)` in
  `Place::Index` write. Feeds `inline_array_set`. **Class-(a) FFI / array
  family.**

- **L316** `ensure_nanboxed(val)` in `Place::Deref` write. Stores into the
  reference's `StackSlot` backing cell. **Class-(a) borrow/deref
  storage.** Paired with `mod.rs:413`'s unbox.

### crates/shape-jit/src/mir_compiler/terminators.rs — 6 calls

- **L191** `ensure_nanboxed(val)` pushing method-dispatch args onto
  `ctx.stack`. Feeds `jit_call_method`. **Class-(a) VM-stack push.**

- **L259** `ensure_nanboxed(raw)` for the `print` builtin arg. Feeds
  `ffi.print`. **Class-(a) FFI / object family** (`jit_print` lives with
  other runtime printer/IO FFIs).

- **L294** `ensure_nanboxed(raw)` for enum-constructor payload push.
  Feeds `ffi.array_push_elem`. **Class-(a) FFI / array family.**

- **L339** `ensure_nanboxed(val)` for direct user-function args passed as
  uniform-I64 Cranelift params. **Class-(a) VM-stack push (callee ABI).**
  Paired with `compiler/program.rs:420`'s unbox.

- **L388** `ensure_nanboxed(callee_val)` pushing the closure callee onto
  `ctx.stack` before indirect call. **Class-(a) VM-stack push.**

- **L398** `ensure_nanboxed(val)` pushing indirect-call args onto
  `ctx.stack`. **Class-(a) VM-stack push.**

### crates/shape-jit/src/mir_compiler/statements.rs — 6 calls

- **L68** `ensure_nanboxed(raw)` in `ArrayStore`. Feeds
  `ffi.array_push_elem`. **Class-(a) FFI / array family.**

- **L120** `ensure_nanboxed(raw)` in `ObjectStore`. Feeds
  `ffi.typed_object_set_field`. **Class-(a) FFI / object family.**

- **L168** `ensure_nanboxed(raw)` in `EnumStore` payload. Feeds
  `ffi.array_push_elem`. **Class-(a) FFI / array family.**

- **L287** `ensure_nanboxed(raw)` in the legacy-path `ClosureCapture`
  block, pushing captures to `ctx.stack` before `ffi.make_closure`.
  **Class-(a) VM-stack push (closure call ABI).** This path is marked for
  deletion in closure-spec Phase H5 independently of R4; the rewrite here
  is subsumed if H5 lands first.

- **L659, L682** `ensure_nanboxed(val)` in `coerce_for_capture_store` —
  the I64 branch and the last-resort fallback for typed-closure capture
  storage. **Class-(a) FFI / object family** (heap-closure block layout
  writes — the "cell holds whatever the layout's `FieldKind::I64` means"
  contract). R4.2C makes the capture-cell type follow the layout's native
  type; the fallback branch disappears.

### crates/shape-jit/src/mir_compiler/v2_typed_map.rs — 8 calls

All eight feed `v2_map_len` / `v2_map_has_str` / `v2_map_get_str_i64` /
`v2_map_get_str_f64` / `v2_map_set_str_i64`. **Class-(a) FFI / object
family** (the v2 typed-map FFIs sit next to the object/struct helpers in
R4.2C):

- **L85** `ensure_nanboxed(map_bits)` in `length`/`len`/`size`.
- **L99, L101** `ensure_nanboxed(map_bits)` + `ensure_nanboxed(key_bits)`
  in `has`.
- **L120, L122** `ensure_nanboxed(map_bits)` + `ensure_nanboxed(key_bits)`
  in `get`.
- **L157, L159, L161** `ensure_nanboxed(map_bits)`,
  `ensure_nanboxed(key_bits)`, `ensure_nanboxed(val_bits)` in `set`.

### crates/shape-jit/src/mir_compiler/mod.rs — 1 call

- **L413** `unbox_from_nanboxed(reloaded, kind)` in
  `reload_referenced_locals`. Reloads a borrow's `StackSlot` cell after a
  call that may have mutated through the reference. **Class-(a)
  borrow/deref storage.** Paired with the three borrow/deref
  `ensure_nanboxed` writes at `rvalues.rs:81`, `places.rs:316`, and the
  `&` place-read stub.

### crates/shape-jit/src/compiler/program.rs — 1 call

- **L420** `unbox_from_nanboxed(param_val, kind)` on user-function entry.
  Converts the uniform-I64 callee ABI param to the MIR local's native
  slot kind. **Class-(a) VM-stack push (callee ABI).** Paired with
  `terminators.rs:339`'s box at the caller.

### crates/shape-jit/src/mir_compiler/conversions.rs — 3 internal calls

- **L134** `unbox_from_nanboxed` inside `ensure_kind`'s NaN-boxed → native
  branch.
- **L138** `box_to_nanboxed` inside `ensure_kind`'s native → NaN-boxed
  branch.
- **L158, L159** `box_to_nanboxed` + `unbox_from_nanboxed` inside
  `convert_between`. Routes foreign conversions through the NaN-boxed
  intermediate.

These disappear when R4.3 deletes the module. `ensure_kind` and
`convert_between` themselves may survive in a shrunk form (identity
no-ops or direct Cranelift widen/narrow) if other callers still need
them; otherwise they go with the rest.

### Documentation references (not call sites) — 2

- `crates/shape-jit/src/mir_compiler/ownership.rs:67` — doc comment on
  `compile_constant` mentioning `ensure_nanboxed()`. Update in R4.3.
- `crates/shape-jit/src/ffi/object/closure.rs:280` — comment in closure
  FFI implementation referencing `ensure_nanboxed`. Update in R4.3.

## R4.2 commit plan

One commit per FFI / ABI family. All six are class-(a) rewrites; each
changes the relevant Cranelift signatures in `ffi_refs.rs` +
`ffi_symbols/*` + the Rust-side FFI impl in `crates/shape-jit/src/ffi/*`,
then deletes the now-redundant `ensure_nanboxed` at every matching call
site.

- **R4.2A `jit-ffi: rewrite math/conversion FFI signatures to native types`**
  Covers `generic_add/sub/mul/div/mod/eq/neq/lt/le/gt/ge` (`rvalues.rs`
  L370–L371). Files: `ffi/generic_builtin.rs`, `ffi/conversion.rs`,
  `ffi/math.rs`, `ffi_refs.rs`. ~2 call-site deletions.

- **R4.2B `jit-ffi: rewrite array FFI signatures to native types`**
  Covers `array_push_elem`, `inline_array_get`, `inline_array_set`.
  Call sites: `rvalues.rs:109`, `places.rs:244,306,309`,
  `terminators.rs:294`, `statements.rs:68,168`. ~7 deletions.

- **R4.2C `jit-ffi: rewrite object/typed-map FFI signatures to native types`**
  Covers `get_prop`, `set_prop`, `typed_object_set_field`, `print`, the
  eight `v2_map_*` FFIs, and the two `coerce_for_capture_store` cells.
  Call sites: `places.rs:217,276,277`, `terminators.rs:259`,
  `statements.rs:120,659,682`, all 8 in `v2_typed_map.rs`. ~15 deletions.

- **R4.2D `jit-ffi: rewrite arc FFI signatures to native types`**
  Covers `arc_retain`. Call site: `rvalues.rs:71`. ~1 deletion.

- **R4.2E `jit-abi: route VM-stack / call ABI through native types`**
  Covers the uniform-I64 Cranelift callee ABI, `ctx.stack` indirect-call
  and method-dispatch pushes, and closure-capture pushes (legacy path).
  Call sites: `terminators.rs:191,339,388,398`, `statements.rs:287`,
  `compiler/program.rs:420`. ~6 deletions. Depends on v2 typed-ABI
  work — **may be deferred to R4.2F if typed ABI isn't ready by R4.2A-D
  merge**; in that case the `ensure_nanboxed`/`unbox_from_nanboxed` here
  survive into R4.3 and the helpers stay alive solely for these six
  sites until a follow-up typed-ABI commit.

- **R4.2F `jit-mir: route borrow/deref StackSlot cells through native types`**
  Covers the `&T` / `&mut T` reference-cell backing store. Call sites:
  `rvalues.rs:81`, `places.rs:316`, `mod.rs:413`. ~3 deletions. This
  tranche stands alone because the cell's storage kind is independent of
  any FFI signature — we promote it to follow the root local's
  `SlotKind`.

File-family scope summary (the six sub-commits are all class-(a); no file
contains only class-(b) callers because none of the 35 external sites
classified as class-(b)).

## R4.3 deletion gate

After R4.2A-F merge, confirmation query:

```
$ grep -rn 'box_to_nanboxed\|unbox_from_nanboxed\|ensure_nanboxed' \
    crates/ --include='*.rs'
(empty)
```

At that point:

1. Delete `box_to_nanboxed`, `unbox_from_nanboxed`, `ensure_nanboxed` from
   `crates/shape-jit/src/mir_compiler/conversions.rs`.
2. Delete or shrink `ensure_kind` and `convert_between` depending on
   whether any native→native routing survives R4.2.
3. Delete the two documentation references (`ownership.rs:67`,
   `ffi/object/closure.rs:280`).
4. If the module is empty, delete `conversions.rs` and its
   `mod conversions;` declaration.

Commit: `mir: delete NaN-box conversion helpers (R4.3)`.

## Notes for R4.2 agents

- **Paired rewrites must land together.** The borrow/deref triple
  (`rvalues.rs:81` + `places.rs:316` + `mod.rs:413`) must go in one
  commit; same for the call-ABI pair (`terminators.rs:339` +
  `compiler/program.rs:420`). Otherwise the Cranelift verifier will
  reject the mismatched typed/I64 combination inside one function.
- **Inline helpers wrap FFIs.** `inline_array_get`, `inline_array_set`,
  `inline_typed_field_get`, `inline_typed_field_set` are Rust-side
  emitter helpers that currently box; their internal FFI calls
  (`ffi.get_prop`, `ffi.set_prop`, etc.) still sit behind them. R4.2B/C
  must rewrite both the inline helper and the underlying FFI.
- **No class-(b) callers found.** The plan allowed for opaque-handle
  reinterpretations routed through `ensure_nanboxed`. None exist at
  HEAD — every bit-movement is logically a ValueWord / VM stack /
  reference cell. If R4.2 surfaces one (e.g. a raw pointer passed to a
  new FFI), flag it here before merging so R4.3's grep-zero gate
  remains valid.
