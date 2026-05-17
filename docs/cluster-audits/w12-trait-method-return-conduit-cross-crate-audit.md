# W12-trait-method-return-conduit-cross-crate audit

Phase 3 cluster-0 Round 13 T1' audit-first deliverable. Absorbs the ADR
amendment territory surfaced by Round 12 T1 surface-and-stop (`76b01cf8`)
and determines the fix shape for the three conduit gaps T1 named.

Branch: `bulldozer-strictly-typed-w12-trait-method-return-conduit-cross-crate`
Parent: `3db6e820` (post-Round-12-merge + Round 13 dispatch metadata).
Date: 2026-05-13.

## §0. Scope recap and binding precedent

Kickoff Smoke 3 JIT (`trait T { name(): string } type X {} impl T for X
{ method name() { "x" } } let t = X {} print(t.name())` → `x`) fails JIT
with `Route A surface-and-stop: NotImplemented(SURFACE) — print Call-
terminator operand NativeKind is None`. T1 (`76b01cf8`) closed
surface-and-stop with 3 conduit gaps named, 3 pin tests landed in
`crates/shape-jit/src/mir_compiler/types.rs::tests` (post-T1 baseline
shape-jit 376/0/26), and an extended doc block on
`parametric_method_return_kind_from_receiver` that traced each gap to
its file:line. Round 13 T1' must close the surface or determine that
closing is ADR-amendment territory and surface for Round 14.

Round 6A close (`9cd5bbe0`) + audit (`f58abc8d`) are the binding
precedent. 6A landed `BytecodeProgram.function_return_concrete_types:
Vec<ConcreteType>` (a per-function-index side-table) populated at
`compile_post_assembly` from `FunctionDef.return_type` via
`concrete_type_from_annotation`, threaded through
`Program` / `LinkedProgram` / `program_from_blobs_by_hash` /
`create_stub_program` / `JIT compile_function_with_user_funcs` /
`worker::build_sub_program`, and consumed by the resolver-aware
`infer_top_level_concrete_types_from_mir_with_returns` conduit producer
in `crates/shape-vm/src/compiler/helpers.rs`. Not serialised (`#[serde(skip,
default)]`) per sibling `*_concrete_types` side-table rationale —
`ConcreteType` isn't wire-stable.

## §1. Round 6A precedent fit — gap-by-gap

The agent prompt asks: does trait method return resolution fit the same
shape as 6A, with key `(trait_id, method_id)` instead of `function_id`?

The answer is split. 6A's design fits the **trait-method-return data
path** (a side-table populated at compile time, threaded cross-crate
through the linker/remote/content-addressed shapes, consumed at JIT
MIR-builder time). But the **consumer-side lookup shape** at the JIT MIR
builder cannot key on `(trait_id, method_id)` because neither trait
identity nor method-id (in the sense of a stable per-method index) is
threaded into the JIT MIR conduit. Walking through each gap separately:

### §1.1 Gap 1 — Receiver struct identity erasure

T1's surface analysis traces this to two stamping sites that both
default to `Struct(StructLayoutId(0))`:

- **Producer at MIR-walk time**:
  `crates/shape-vm/src/compiler/helpers.rs:508` — when
  `StatementKind::ObjectStore { container_slot, .. }` is walked, the
  destination slot is stamped `ConcreteType::Struct(StructLayoutId(0))`
  unconditionally. `ObjectStore`'s MIR shape
  (`crates/shape-vm/src/mir/types.rs:433`) carries `container_slot:
  SlotId`, `operands: Vec<Operand>`, `field_names: Vec<String>` — but
  NOT the struct's type name. So the conduit producer cannot stamp a
  type-specific layout id even if a registry existed.
- **Producer at annotation-walk time**:
  `crates/shape-vm/src/compiler/v2_map_emission.rs:357`
  (`concrete_type_from_annotation`) — when the annotation is
  `TypeAnnotation::Basic(name)` with `name` not in the scalar/built-in
  set, the function returns `None` (line 378) with the comment "Phase
  1.1 Agent 3 will fill this in". So user struct annotations don't
  reduce to a `Struct(_)` layout id at all when going through this
  path; they fall through to `Void`.

The first producer dominates for the receiver slot in Smoke 3: `let t =
X {}` lowers to `StatementKind::ObjectStore { container_slot: t_slot,
operands: [], field_names: [] }` (empty operands because `X` has no
fields) — the helper at `helpers.rs:508` stamps
`Struct(StructLayoutId(0))` on `t_slot` regardless of the struct name.

**Structural fit with 6A**: 6A's producer (resolver-aware
`infer_top_level_concrete_types_from_mir_with_returns`) keys
Call-terminator destination stamping on the callee's name. The trait
method return path's analog is keying ObjectStore destination stamping
on the struct's name. The struct name is observable at MIR-lowering
time (the AST `Expr::StructLiteral { name, fields, .. }` carries it)
but the lowering at `mir/lowering/expr.rs:1892-1913` drops it before
emitting `ObjectStore`. **Closing gap 1 requires either**:

  (i) Threading the struct name onto `ObjectStore` as a new field
  (e.g. `ObjectStore { container_slot, operands, field_names, type_name:
  Option<String> }`), populated at MIR lowering from the AST. The
  producer at `helpers.rs:508` then stamps
  `Struct(struct_layout_id_for_name(&type_name))` via a name→id
  registry. **MIR shape extension**.

  (ii) Adding a `Struct(StringInternId)` variant — `ConcreteType` grows
  a string-keyed variant for user structs. Same MIR shape extension
  obligation (carry the name on `ObjectStore`), but the name resolution
  happens at consume time rather than producer side. **Same MIR shape
  extension**.

  (iii) Threading a parallel `Vec<Option<String>> = local_struct_names`
  side-table at MIR-function level, populated at MIR lowering time and
  consulted at producer time. **Same MIR shape extension**, encoded
  more loosely.

All three options expand `ObjectStore` (or a parallel side-table) with
struct identity. No structural alternative avoids this — the MIR-time
data simply doesn't carry the receiver type today.

### §1.2 Gap 2 — Trait registry not persisted in BytecodeProgram

`TypeRegistry::traits: HashMap<String, TraitDef>`
(`crates/shape-runtime/src/type_system/environment/registry.rs:111`)
holds declared trait method return types via
`TraitMember::Required(InterfaceMember::Method { return_type:
TypeAnnotation, .. })` and `TraitMember::Default(MethodDef {
return_type: Option<TypeAnnotation>, .. })`.

`BytecodeProgram`
(`crates/shape-vm/src/bytecode/core_types.rs`) carries
`trait_method_symbols: HashMap<String, String>` (line 373, the
`(trait_name, type_name, impl_selector, method_name)` → `function_name`
map) and `trait_vtables: HashMap<String, Arc<VTable>>` (line 446,
keyed by `"Trait::ConcreteType"`). Neither carries the declared trait
method return type.

The `BytecodeCompiler` (`crates/shape-vm/src/compiler/mod.rs:956`) does
hold `trait_defs: HashMap<String, TraitDef>` and uses it at impl-block
compilation (`statements.rs:421`, `statements.rs:501`) to install
default methods. So the trait return types are available at impl-block
compile time inside the compiler — the issue is purely propagation to
`BytecodeProgram` and the JIT MIR builder.

**Structural fit with 6A**: 6A added a single new `BytecodeProgram`
field of type `Vec<ConcreteType>` keyed by function index, threaded
through every cross-crate shape (Program, LinkedProgram,
ContentAddressedProgram remote stubs, JIT worker subprogram). The same
shape closes gap 2 — a new `BytecodeProgram` field, say
`trait_method_declared_return_concrete_types`, keyed on a stable
identifier per trait method declaration. Per §1.3 below, the cleanest
key is `(type_name: String, method_name: String)` rather than
`(trait_id, method_id)` — see §1.3 for the reasoning.

### §1.3 Gap 3 — Impl method return type fallback insufficient

`function_return_concrete_types[X::name] = ConcreteType::Void` because
`desugar_impl_method` (`crates/shape-vm/src/compiler/statements.rs:1687`)
copies `method.return_type.clone()` directly from the impl AST. For
Smoke 3's `impl T for X { method name() { "x" } }`, the impl source
doesn't repeat the trait's `: string` annotation, so `method.return_type
= None` and the resulting `FunctionDef.return_type` is also `None`. The
6A populator at `compiler_impl_reference_model.rs:1474-1487` looks up
each function's return annotation via `expanded_function_defs` and
falls through to `Void` when the annotation is `None`.

**Critical observation**: gap 3 can be closed at the source — by making
`desugar_impl_method` consult the trait declaration when the impl's
`return_type` is `None`. The compiler has `self.trait_defs` available;
the impl block carries `trait_name` (already resolved via
`resolve_trait_name` at `statements.rs:371`); the trait member by name
is `O(n)` to find. This fix flows the trait's declared return type
into the impl's `FunctionDef.return_type`, which already feeds the 6A
side-table population at line 1474-1487 — so
`function_return_concrete_types[X::name]` becomes `ConcreteType::String`
once the FunctionDef carries the right annotation.

**Why this fix alone isn't sufficient**: closing gap 3 makes
`function_return_concrete_types[X::name] = String`, but the 6A
consumer in `infer_top_level_concrete_types_from_mir_with_returns`
only stamps destinations for `MirConstant::Function(name)`
terminators (`helpers.rs:658`). The MIR lowering at
`mir/lowering/expr.rs:1866` emits `MirConstant::Method(name)` for
`Expr::MethodCall` — so the conduit producer would NOT consume the
fixed entry. Two paths to close the consumer side:

  (i) **Extend the conduit producer's Call-terminator pass** to handle
  `MirConstant::Method(name)`. Receiver type derives from
  `concrete_types[args[0].root_local()]`; if the receiver is a
  `Struct(_)` with a resolvable type name (depends on gap 1 closure),
  look up the function symbol via
  `BytecodeProgram::find_default_trait_impl_for_type_method(type_name,
  method_name)` then look up `function_return_concrete_types[fn_idx]`.

  (ii) **MIR-level resolution at lowering time** — when the compiler
  resolves `t.name()` to `X::name` (already done at the bytecode-emit
  layer in `expressions/function_calls.rs:2204-2218`), emit
  `MirConstant::Function("X::name")` directly into MIR instead of
  `MirConstant::Method("name")`. Then the existing 6A
  Call-terminator pass picks it up unchanged. This bypasses gap 1
  entirely on the trait-dispatch path because the function name is
  fully resolved at MIR-lowering time.

(ii) is structurally simpler but changes MIR semantics on the trait
dispatch path. (i) preserves the abstract method dispatch in MIR but
requires gap 1 to be closed. Both fit the 6A "single resolver
closure threaded through `compile_post_assembly`" shape — neither
requires a new MIR statement kind or a new ABI.

## §2. Disposition

**Option (a) — same shape**, with the following structural augmentation:

1. **Gap 3 source-side fix** (smallest delta, highest leverage):
   `desugar_impl_method` consults `self.trait_defs` when
   `method.return_type` is `None`, substituting the trait's declared
   return type. This populates `function_return_concrete_types[X::name]
   = ConcreteType::String` for Smoke 3 via the existing 6A pipeline. NO
   new BytecodeProgram field, NO cross-crate threading delta for this
   piece.

2. **Gap 1 closure** (structural MIR delta): extend
   `StatementKind::ObjectStore` with `type_name: Option<String>`,
   populated at MIR lowering from `Expr::StructLiteral { name, .. }` /
   `Expr::Object`'s container type annotation. The producer at
   `helpers.rs:508` stamps `Struct(struct_layout_id_for_name(&name))`
   via a name→id registry. Adding the registry itself requires picking
   a stable id space — the simplest landing is to extend
   `BytecodeProgram` with a `struct_layout_names: Vec<String>`
   side-table where `StructLayoutId(i) = i-th name`. This is the
   "Phase 1.1 Agent 3" work
   `v2_map_emission.rs:376` referenced as deferred.

3. **Gap 2 closure** (cross-crate threading, mirrors 6A exactly): NEW
   `BytecodeProgram` field
   `trait_method_declared_return_concrete_types: HashMap<String,
   ConcreteType>` keyed by the existing `trait_method_symbols` key
   shape `"Trait::Type::Selector::Method"` (the same format
   `BytecodeProgram::trait_method_symbol_key` uses). Populated at
   impl-block compile time at `statements.rs:392` and
   `statements.rs:433`, from the trait declaration's return type via
   `concrete_type_from_annotation`. Threaded through `linker.rs`,
   `remote.rs`, `content_addressed.rs::Program/LinkedProgram`,
   `worker.rs::build_sub_program`, `compiler/program.rs`. Same
   `#[serde(skip, default)]` disposition as the sibling
   `*_concrete_types` side-tables.

4. **JIT MIR consumer** (close the surface T1 pinned): extend
   `infer_slot_kinds_with_concrete` in
   `crates/shape-jit/src/mir_compiler/types.rs` to handle
   `MirConstant::Method(name)` with a `Struct(StructLayoutId(n))`
   receiver: resolve `n` → type name via the new
   `struct_layout_names` registry, look up the new
   `trait_method_declared_return_concrete_types` side-table by
   `find_default_trait_impl_for_type_method(type_name,
   method_name)`-shaped key (or directly by walking the
   `Trait::Type::*::Method` keys when the trait is unambiguous), and
   stamp the destination slot's `NativeKind` from the resolved
   `ConcreteType`. This is the consumer-side companion to the
   producer-side §2.1-§2.3 plumbing.

**Why option (a) not (b)**: option (a) reuses the 6A side-table shape
exactly for piece 3 (gap 2), extends one MIR statement field for
piece 2 (gap 1), and applies an in-compiler return-type backfill for
piece 1 (gap 3). None of these are ADR-amendment territory:

- The `ObjectStore` extension adds an optional string field — fits the
  exhaustive-match-rule discipline (CLAUDE.md "Exhaustive Match Rule"),
  not a new variant or new ABI.
- The new BytecodeProgram side-table mirrors `function_return_concrete_types`
  exactly. No new wire format, no new HeapKind, no new ConcreteType
  variant.
- The trait-return-type backfill at `desugar_impl_method` is an
  in-compiler bug fix; trait method declarations in source code already
  carry the contract, the compiler simply wasn't propagating it.
- The consumer-side extension in `mir_compiler/types.rs` is a new arm in
  an existing function, the same shape as Round 11-trinity Part (b)
  extending the parametric classifier with new receiver+method pairs.

**LoC estimate**: ~300-500 LoC across:
- `core_types.rs` (~30 LoC: new field + doc block).
- `compiler/statements.rs` (~30 LoC: trait return-type backfill in
  `desugar_impl_method` + side-table population at
  `register_trait_method_symbol` callsites).
- `compiler/v2_map_emission.rs` (~15 LoC: struct layout id resolution
  via name → id registry).
- `compiler/helpers.rs` (~40 LoC: ObjectStore producer uses real
  layout id; conduit producer extension to handle MirConstant::Method).
- `compiler/compiler_impl_reference_model.rs` (~10 LoC: populate new
  side-table at `compile_post_assembly`).
- `mir/types.rs` (~10 LoC: ObjectStore `type_name` field).
- `mir/lowering/expr.rs` (~25 LoC: populate `type_name` at
  StructLiteral / Object lowering sites — 3-4 sites).
- `bytecode/program_impl.rs` (~30 LoC: helper methods for new
  side-table).
- `bytecode/content_addressed.rs` (~25 LoC: Program/LinkedProgram
  mirrors).
- `linker.rs` (~10 LoC: 3 threading sites).
- `remote.rs` (~10 LoC: 4 threading sites).
- `compiler/program.rs` + `worker.rs` (~10 LoC: stub initialization).
- `mir_compiler/types.rs` (~80 LoC: consumer extension + update 3 T1
  pin tests).
- `compiler/helpers.rs` test module (~30 LoC: new producer tests).
- `mir_compiler/types.rs` test module (~30 LoC: new consumer tests).

This is the high end of "mechanical side-table extension" — a
~10-touchpoint cross-crate plumb mirroring 6A exactly, plus one MIR
shape extension (`ObjectStore::type_name`) needed because the trait
dispatch return-kind path is the first conduit user that needs receiver
type identity at MIR producer time.

**ADR amendment NOT required.** The fix shape stays within ADR-006
§2.7.5 (producing-site classification at compile time), §2.7.10/Q11
(method dispatch ABI is unchanged — this is kind classification at the
JIT MIR conduit, not runtime dispatch), and ADR-005 §1 (single-
discriminator preserved — `ConcreteType::Struct(_)` and
`HeapValue::TypedObject(_)` remain the canonical discriminators).

## §3. Threading the 3 gaps

Per §2, the 3 gaps close together as follows. Implementation order matters
to keep the per-commit gates green:

**Commit 1 — Gap 3 source-side fix** (~30 LoC):
1. `compiler/statements.rs::desugar_impl_method` — when
   `method.return_type` is `None`, look up `self.trait_defs.get(trait_name)`,
   find the `TraitMember::Required(InterfaceMember::Method { name,
   return_type, .. })` matching `method.name`, substitute the trait's
   `return_type` into the synthesized `FunctionDef.return_type`. The
   `TraitMember::Default(MethodDef { return_type, .. })` path (already
   hit at line 423) carries its own return_type from the trait body and
   doesn't need this fix — verify against the `register_trait_method_symbol`
   call at line 433 and confirm.
2. New unit test in `compiler/statements.rs` test module:
   `desugar_impl_method_backfills_return_type_from_trait_declaration`.
   Verifies that for `trait T { name(): string }` `impl T for X { method
   name() { "x" } }`, the synthesized `FunctionDef.return_type` is
   `Some(TypeAnnotation::Basic("string"))`.
3. After commit 1: `function_return_concrete_types[X::name] = String`
   in the 6A side-table — but the JIT MIR consumer still doesn't pick
   it up because the MIR uses `MirConstant::Method`, not
   `MirConstant::Function`. The T1 pin test
   `user_defined_trait_method_call_terminator_remains_unstamped` still
   passes (correctly — gap 1 + gap 2 still hold).

**Commit 2 — Gap 1 closure** (~80 LoC, MIR shape extension):
1. `mir/types.rs::StatementKind::ObjectStore` — add `type_name:
   Option<String>` field.
2. `mir/lowering/expr.rs` — populate `type_name` at three sites:
   `Expr::StructLiteral { name, .. }` (line ~1892), `Expr::Object`
   (line ~1793, type_name is `None`), and any other ObjectStore-emit
   site found via `grep -n "ContainerStoreKind::Object"`.
3. `bytecode/core_types.rs::BytecodeProgram` — add
   `struct_layout_names: Vec<String>` side-table (`#[serde(skip,
   default)]`, mirrors `function_return_concrete_types` field
   discipline).
4. `compiler/statements.rs` — populate `struct_layout_names` at struct
   type definition compile time (where `type X {}` is processed).
5. `compiler/helpers.rs:508` — stamp `Struct(struct_layout_id_for_name(
   &type_name))` instead of the literal `StructLayoutId(0)`, using a
   reverse-lookup helper on the new side-table.
6. `compiler/v2_map_emission.rs:357` — when `TypeAnnotation::Basic(name)`
   is a known user struct name, return `Some(ConcreteType::Struct(id))`
   via the same reverse-lookup helper. The `_ => None` arm at line 378
   becomes `_ => struct_layout_id_for_name(name).map(ConcreteType::Struct)`.
7. Thread `struct_layout_names` through `linker.rs`, `remote.rs`,
   `content_addressed.rs::Program/LinkedProgram`, `worker.rs::build_sub_program`,
   `compiler/program.rs`. ~6 threading sites mirroring 6A exactly.
8. After commit 2: receiver slot `t` carries
   `ConcreteType::Struct(StructLayoutId(n))` where `n` resolves to
   `"X"` via the registry. Two T1 pin tests update:
   `user_defined_trait_method_on_struct_returns_none` becomes
   `user_defined_trait_method_on_struct_resolves_via_trait_registry`
   (asserts the new positive behavior — receiver type known + trait
   method declared return → destination kind stamped).
   `parametric_classifier_remains_silent_for_struct_receiver_with_known_method_names`
   stays as-is (the parametric classifier still doesn't claim `name`
   — only the new trait-method classifier does).

**Commit 3 — Gap 2 closure + consumer extension** (~200 LoC, the
6A-shape cross-crate plumb):
1. `bytecode/core_types.rs::BytecodeProgram` — add
   `trait_method_declared_return_concrete_types: HashMap<String,
   ConcreteType>` field with doc block citing §2.7.5, ADR-006
   §2.7.10/Q11, the 6A precedent, and the T1 surface close.
2. `bytecode/program_impl.rs` — add
   `register_trait_method_declared_return_concrete_type` /
   `lookup_trait_method_declared_return_concrete_type` paired helpers
   keyed by the existing `trait_method_symbol_key` format.
3. `compiler/statements.rs` — at impl-block compilation (lines 384-418
   for `Required` methods, lines 421-444 for `Default` methods),
   populate the new side-table from `trait_def.members` via
   `concrete_type_from_annotation` on each method's
   `InterfaceMember::Method { return_type, .. }` /
   `MethodDef.return_type`. Use the same key format as
   `register_trait_method_symbol`.
4. Thread the new field through `linker.rs::link`,
   `linker.rs::linked_to_bytecode_program`, `remote.rs::program_from_blobs_by_hash`,
   `remote.rs::create_stub_program`, `content_addressed.rs::Program`
   + `LinkedProgram`, `worker.rs::build_sub_program`,
   `compiler/program.rs::JITCompiler::stub_initialization` — exactly
   the 6A pattern. Update `linker_tests.rs` initializer + 4 test
   helper sites in `remote.rs::mod tests`.
5. `mir_compiler/types.rs::infer_slot_kinds_with_concrete` — extend the
   `MirConstant::Method(name)` arm (lines ~342-348) to chain:
   `well_known.or_else(parametric).or_else(trait_method_declared)`
   where `trait_method_declared` looks up
   `concrete_types[args[0].root_local()]`, extracts the type name via
   the new `struct_layout_names` reverse-lookup (gap 1 dependency),
   and looks up the side-table by
   `trait_method_symbol_key`-shaped key. The disambiguation rule for
   multi-trait `name()` conflicts is "exactly one trait's
   declaration matches" (else `None` — surface to downstream JIT
   producers).
6. Update T1 pin tests as noted in commit 2 step 8. The third pin test
   `parametric_classifier_remains_silent_for_struct_receiver_with_known_method_names`
   stays AS-IS: the parametric classifier still doesn't claim user
   methods — the new trait-method classifier is a separate `.or_else(.)`
   leg. Document this in the pin test's docstring.
7. Add new pin test
   `trait_method_declared_return_kind_stamps_destination` exercising
   the full Smoke 3 surface end-to-end at MIR-builder level (the
   integration is then validated by the kickoff Smoke 3 JIT run).

**Commit 4 — close report**: Update `phase-3-cluster-0-status.md` with
a `### W12-trait-method-return-conduit-cross-crate close (2026-05-13)`
subsection, update `AGENTS.md` row to closed, update
`docs/defections.md` if any considered-but-rejected compromises
surface during implementation.

## §4. Refuse-on-sight rationale captured

Per agent prompt + CLAUDE.md "Renames to refuse on sight":
- The new `BytecodeProgram` field is named
  `trait_method_declared_return_concrete_types` — describes what it is
  (the declared return ConcreteType per trait method, populated from
  the trait declaration at impl-block compile time). NOT a "bridge",
  "probe", "helper", "hop", "translator", "adapter", or "shim" — those
  framings would be the §2.7.10/Q11 dispatch-ABI defection-attractor
  family or §2.7.11/Q12 value-call-ABI family extended to the trait-
  return-kind threading layer. Refused.
- The new MIR field `ObjectStore::type_name` describes what it is (the
  AST-time struct type name threaded through the ObjectStore for
  producer-time ConcreteType stamping). Not "type-id bridge",
  "struct-identity translator", etc. — same family rule. Refused.
- The new compiler-side backfill in `desugar_impl_method` is described
  as "trait declaration return-type substitution" or "trait return-type
  backfill" — describes the operation. Not "trait-impl bridge",
  "return-type probe", etc. Refused.
- "Hard-code the kickoff Smoke 3 case" / "default to `string` for
  unknown trait return kinds" / "Bool-default for unproven trait-method
  return kind" — refused per the agent prompt's forbidden-rationalization
  list and ADR-006 §2.7.7 #4.
- "Mark gap 1 / gap 2 / gap 3 as separate sub-clusters" — refused.
  Per T1's surface analysis they must close together; surfacing one
  without the others is the W-series walk-back pattern.
- "Add a `Convert<TraitMethod>To<NativeKind>` opcode" — refused per
  CLAUDE.md "Forbidden code" / the W4-δ `ConvertBoolToString` precedent.
- "trait-id/method-id resolution as a bridge over function-id" — refused
  per agent prompt's refuse-on-sight list. The disposition does NOT key
  the new side-table on `(trait_id, method_id)` — it keys on the
  existing `trait_method_symbol_key` format
  (`"Trait::Type::Selector::Method"`), which is a string built from
  already-stable names. No new bridge / no new translation layer.

## §5. Multi-impl disambiguation surface

The disposition's consumer-side lookup (§2 piece 4) uses
`find_default_trait_impl_for_type_method`-shaped logic. The
multi-trait case where a user struct implements two traits both
declaring `name()` with different return types must not silently
collapse — `find_default_trait_impl_for_type_method` falls back to
"any named impl (first match)" at line 168-174, which is non-
deterministic for the kind classifier. Disposition: the new
trait-method classifier returns `None` (not "first match") when
multiple traits declare `name()` with different return ConcreteTypes
for the same receiver type — the downstream JIT producer surfaces and
the user gets a `NotImplemented(SURFACE)` rather than a wrong-kind
stamp. This is the same discipline §2.7.7 #4 / forbidden #9 applies
across the conduit. Pin this behavior with a dedicated test
`multi_trait_name_with_different_return_types_surfaces_unstamped`.

## §6. Round 12 T1 pin tests disposition

T1 landed 3 pin tests at
`crates/shape-jit/src/mir_compiler/types.rs::tests`. Post-T1' status:

1. **`user_defined_trait_method_on_struct_returns_none`**: renamed to
   `user_defined_trait_method_on_struct_resolves_via_trait_registry`.
   Asserts the new positive behavior — `t.name()` on a `Struct(X)`
   receiver with `trait T { name(): string }` and
   `impl T for X { method name() { "x" } }` stamps the destination
   slot's kind as `Some(NativeKind::String)`. Pin against future
   regressions back to the surface-and-stop posture.

2. **`user_defined_trait_method_call_terminator_remains_unstamped`**:
   renamed to `user_defined_trait_method_call_terminator_stamps_string`.
   Same integration pin updated to assert positive behavior. The
   well-known cohort assertion on `"name"` stays AS-IS — `"name"` must
   still NOT be in `well_known_method_return_kind` (the soundness rule
   the test docstring captures still applies; closing the surface
   doesn't change which classifier owns `"name"`'s return kind — the
   new trait-method classifier does, not the well-known one).

3. **`parametric_classifier_remains_silent_for_struct_receiver_with_known_method_names`**:
   stays AS-IS. The parametric classifier covers collection-shape
   method-name+receiver-ct pairs (Array.sum, HashMap.get, etc.); it
   correctly does NOT claim user-defined trait methods on user struct
   receivers. The new trait-method classifier is a separate `.or_else(.)`
   leg; the parametric classifier's silent-on-Struct(_) behavior is
   preserved verbatim. Update the test's docstring to clarify this
   coexistence with the new trait-method leg.

No test is deleted. All 3 pins migrate from surface pin → positive
pin via rename + assertion update.

## §7. Smoke 3 JIT close projection

After commits 1-3 land:
- `let t = X {}` lowers to `ObjectStore { container_slot: t, ..,
  type_name: Some("X") }`. Producer stamps
  `concrete_types[t] = Struct(StructLayoutId(n))` where `n` resolves
  to `"X"` via `struct_layout_names`.
- `t.name()` lowers to Call terminator with `func =
  MirConstant::Method("name")`. JIT MIR consumer's
  `infer_slot_kinds_with_concrete` chains
  `well_known.or_else(parametric).or_else(trait_method_declared)`. The
  trait-method-declared leg looks up
  `find_default_trait_impl_for_type_method("X", "name")` (or the new
  side-table directly), finds the trait return ConcreteType
  `ConcreteType::String`, converts via
  `native_kind_from_concrete_type` to `NativeKind::String`. Destination
  slot is stamped.
- `print(t.name())` Call-terminator operand carries
  `NativeKind::String` from the conduit; the JIT-side print FFI
  dispatches to `jit_print_string_arc` (the §2.7.5 producer migrated
  during Round 12 T2/T3 close at `61687564`). Output: `x`. VM == JIT.

The Round-12 T2/T3 close (W12-jit-string-carrier-unification) is
binding for the carrier-shape side: `MirConstant::Str` already emits
the `Arc::into_raw(Arc<String>)` raw-pointer carrier post-T2/T3, so the
print FFI consumer receives the right memory layout. T1' closes the
kind-classification side; the two together close Smoke 3 JIT
end-to-end.

## §8. Coordination with parallel Round 13 sub-clusters

T4 (`W17-vm-intrinsic-sum-wave-5d-migration`) touches
`crates/shape-vm/src/executor/vm_impl/builtins.rs:472` — zero file-
territory overlap.

T5 (`W17-vm-call-value-closure-kind-mismatch`) is audit-first; if
implementation commits land, they touch
`crates/shape-vm/src/executor/call_convention.rs` and possibly upstream
MIR-emit sites that produce closure call values. Closure call values
flow through `MirConstant::Function` / `MirConstant::ClosurePlaceholder`
and per-call-convention frame setup — distinct from T1's trait method
dispatch path which flows through `MirConstant::Method` + the new
trait-method declared return side-table. **Possible overlap point**:
if T5's audit surfaces an upstream MIR-emit change at
`mir/lowering/expr.rs`, T1' may need to coordinate the
`ObjectStore::type_name` extension's MIR-shape touchpoint. Disposition:
T1' lands its MIR shape extension cleanly without touching the closure
call-value emission path; if T5 also extends MIR shape, the merger
sequencing follows supervisor disposition at Round 13 close.

## §9. Close gates (projected, §2 (a))

- Kickoff Smoke 3 JIT (`trait T { name(): string } type X {} impl T for
  X { method name() { "x" } } let t = X {} print(t.name())` → `x`)
  produces `x` matching VM output.
- 3 T1 pin tests renamed and updated to positive pins per §6.
- New pin tests landed:
  `desugar_impl_method_backfills_return_type_from_trait_declaration`
  (compiler/statements.rs),
  `trait_method_declared_return_kind_stamps_destination`
  (mir_compiler/types.rs),
  `multi_trait_name_with_different_return_types_surfaces_unstamped`
  (mir_compiler/types.rs).
- `cargo check --workspace --lib --tests` EXIT=0 inside devenv.
- `cargo test -p shape-vm --lib` no new regressions vs post-Round-12
  baseline.
- `cargo test -p shape-jit --lib` no regressions from baseline 376 + new
  pin tests (projected 376 + 2 new = 378 OR with renames 376 same total
  + assertion updates).
- `bash scripts/verify-merge.sh` 12/12.
- `bash scripts/check-no-dynamic.sh` EXIT=0.
- `AGENTS.md` row → `closed`.
- Status doc subsection `### W12-trait-method-return-conduit-cross-crate
  close (2026-05-13)`.

## §10. Audit decision

**Disposition: §2 (a) — same shape, proceeded with the 3-gap closure.**

The 3 gaps fit the 6A precedent shape with one structural augmentation
(MirFunction `local_struct_type_names` field). No ADR amendment required.

## §11. Implementation summary (close, 2026-05-13)

Implementation contracted from the original §2 plan after deeper
review: gap 2 closes by **deduction** from existing data rather than a
new BytecodeProgram side-table.

Commit sequence landed:

**Commit 1** (`119c4f7e`) — Gap 3: trait declaration return-type
substitution at `desugar_impl_method`. ~30 LoC + 3 unit tests. The
impl method's `FunctionDef.return_type` is backfilled from the trait
declaration when the impl source omits it; `function_return_concrete_types["X::name"]`
now carries `ConcreteType::String` for Smoke 3.

**Commit 2** (`1f9f757a`) — Gaps 1+2: MIR struct identity + conduit
producer trait-method dispatch + 4 unit tests. ~770 LoC including the
41 MirFunction construction site updates for the new field.

Key insight not in the original audit: gap 2 doesn't need a new
BytecodeProgram side-table. The existing `trait_method_symbols` map
(populated at impl-block compile time) + `function_return_concrete_types`
(populated by commit 1's gap 3 closure) together carry the trait
method declared return ConcreteType. The new `method_returns` resolver
chains `find_default_trait_impl_for_type_method(type_name, method_name)`
through `function_return_concrete_types[fn_idx]` — pure deduction
from existing data, no new cross-crate threading. This contracted the
LoC budget from ~365 to ~200 net source LoC.

The original audit's §2 piece 3 (NEW `BytecodeProgram` side-table
`trait_method_declared_return_concrete_types`) is NOT needed — the
data is already there.

## §12. NEW SURFACE uncovered: W17 receiver classification

End-to-end Smoke 3 JIT does NOT print `x` post-T1'. The kind-classification
surface T1 named is closed (verified via debug instrumentation:
`concrete_types[result_slot] = ConcreteType::String` flows through;
JIT `kind_hint = Some(NativeKind::String)` at the print Call-terminator;
dispatch routes to `jit_print_string_arc`). But the JIT trampoline
returns bits that segfault `Arc::from_raw(bits as *const String)`.

Root cause traced via debug instrumentation:
`crates/shape-jit/src/ffi/call_method/mod.rs::receiver_type_name`
(line 51-81) classifies the receiver via legacy NaN-box tag-decode:
`is_number(receiver_bits)`, `heap_kind(receiver_bits)`. With Round 12
T2/T3's producer-side migration to raw `Arc::into_raw(Arc<TypedObjectStorage>)`
pointer bits, the receiver bits no longer carry the NaN-box tag —
the raw heap-pointer bits look like a "number" to `is_number()`
(which returns true for non-TAG_BASE bits). So `receiver_type_name`
returns `"number"` instead of `"X"`,
`find_function_by_name("number::name")` returns None,
`try_call_user_method` returns None, `jit_call_method` returns
TAG_NULL (`0xfffb000000000000`), and `print(t.name())` reads TAG_NULL
as `*const String` → segfault.

This is the **W17-jit-typed-object-arc-storage-migration** class
named as a cluster-1 follow-up in the Round 12 T2/T3 close report
(`61687564`). The Round 12 T2/T3 close commit explicitly documented:
"`box_typed_object` at `value_ffi.rs:516-518` returning `unified_box(
HK_TYPED_OBJECT, *const u8)` over a JIT-owned `TypedObject` struct,
NOT the VM-side `Arc<TypedObjectStorage>` ... Migrating the JIT-side
TypedObject to the VM-side `Arc<TypedObjectStorage>` is a larger
surgery (W11 TypedArray family invariant + 17+ JIT-internal consumers
in `typed_object/`, `data.rs`, `property_access.rs`, etc.)."

T1' has done its 3-gap closure (gaps 1, 2, 3 named by T1
surface-and-stop). The deeper carrier-shape mismatch at the JIT
trampoline's receiver classification (`receiver_type_name` decoding
NaN-box tags from raw Arc pointer bits) prevents Smoke 3 end-to-end
JIT success. This is exactly the same surface-and-stop posture T1
took: T1' is one tier downstream of T1, with the same
necessary-but-not-sufficient relationship to the Smoke 3 close.

**Recommendation**: Round 14 absorbs **W17-jit-typed-object-arc-storage-migration**
sub-cluster to migrate `receiver_type_name` (and the broader JIT-internal
TypedObject struct consumers) to read receiver type identity from
the parallel-kind track + heap-pointer dereference rather than NaN-box
tag-decode.

Audit complete. Implementation closed at commit 2 (`1f9f757a`); the
W17 receiver-classification surface is documented here and in the
commit 2 message for supervisor dispatch.
