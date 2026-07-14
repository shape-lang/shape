# W91A Typed Opcode Proof Coverage Audit

Date: 2026-07-03

Worker: W91A (`strict-flip-w91a-typed-opcode-proof-coverage`)

Scope:

- `crates/shape-vm/src/compiler/**`
- `crates/shape-vm/src/bytecode/**`
- `crates/shape-vm/src/type_tracking.rs`

Guard landed:

- `scripts/check-typed-opcode-proof-coverage.py`

The guard is source-only and non-cargo. It scans compiler Rust sources for
direct typed opcode/static-kind mentions, classifies each production mention
into the categories below, and fails on unclassified hits or count drift. The
counts are source mentions, not semantic-site counts; this keeps the check cheap
and sensitive to new direct stamp paths.

Current guard baseline:

| Classification | Source hits | Disposition |
|---|---:|---|
| Covered by `prove_native_kind` | 22 | Typed return-value opcode mapping/walkback tied to the exact scalar return proof path. |
| Covered by equivalent static proof helper | 519 | Compiler helpers that prove the kind through typed AST/schema/storage metadata before choosing a typed opcode. |
| Metadata-only / non-executing | 138 | Test bodies, bytecode opcode definitions, verifier metadata, docs, and assertions. |
| Unproven gap | 0 | RowView fallback typed column stamps were removed in the redrive. |

Book-truth-100 source-only refresh lowered the classified mention counts after
later cleanup removed or rewrote typed-opcode mentions. The enforced
invariant remains unchanged: `unproven_gap` must stay zero, and new production
typed-opcode stamp paths must be classified before the guard can pass.

The same campaign added one `SetFieldTyped` emission site for typed-object
`Option<T>` field mutation. That path is classified here because the compiler
sources the `TypedField` operand from schema metadata and the runtime validates
the canonical `__Option.Some/None` carrier before mutating storage metadata.

Wave 18 added four content typed-array emission mentions for content-array
construction, get, push, and set. Those paths stay in the equivalent static
proof bucket because the compiler selects them from `TypedArrayKind::Content`,
`ConcreteType::Content`, or tracked `Array<content>` receiver metadata before
emitting the typed opcode; the VM handlers preserve content heap slots without
falling back to tag probing.

## Inventory

### Covered by `prove_native_kind`

Typed scalar return emission is covered by:

- `helpers_binding.rs::emit_return_value_with_ownership`
- `helpers_binding.rs::exact_scalar_return_kind_for_expr`
- `helpers_binding.rs::prove_exact_scalar_return_kind`
- `helpers.rs::typed_return_value_opcode`

The emitting path proves the claimed `NativeKind` from the expression's
`ConcreteType` with `prove_native_kind` before selecting `ReturnValueI64`,
`ReturnValueF64`, `ReturnValueBool`, and related typed return opcodes. The
top-level metadata helper intentionally treats a `ProofGap` as absence of
metadata, but that path does not emit a typed return opcode.

### Covered by Equivalent Static Proof Helpers

These groups stamp typed opcodes only after a compile-time proof path other
than `prove_native_kind`:

- Scalar arithmetic, comparison, equality, bitwise, unary, and string-concat
  opcodes in `compiler/expressions/**`: guarded by `NumericType`,
  `EqOperandType`, `typed_opcode_for`, strict numeric defaults, and coercion
  helpers.
- Typed array construction, element access, push/set, and length opcodes in
  `compiler/typed_emission.rs` and `compiler/v2_typed_emission.rs`: guarded by
  `TypedArrayKind`, `ConcreteType`, slot-kind checks, and typed receiver
  resolution.
- Local, module-binding, return-slot, and capture typed load/store opcodes in
  `compiler/helpers.rs`: guarded by `StorageHint` to `FieldKind` conversion,
  width-specific operands, capture layout metadata, and cell inner-kind
  metadata.
- Typed object, typed struct, and field load/store opcodes: guarded by schema
  registry metadata, `TypedField` operands, `field_type_tag`, and
  `StructLayout::FieldKind`.
- Pattern/control-flow internal integer opcodes: generated from known constant
  indexes, lengths, discriminants, or compiler-managed temporaries.
- RowView column opcodes when schema metadata resolves a field to
  `FieldType::F64`, `FieldType::I64`, `FieldType::Timestamp`,
  `FieldType::Bool`, or `FieldType::String`.

### Metadata-Only / Non-Executing

`crates/shape-vm/src/bytecode/**` is not a production emission source. Its
typed opcode mentions define opcode values, verifier categories, debug/metadata
contracts, and test fixtures. Those sites do not choose an opcode for
compiler-emitted bytecode.

`crates/shape-vm/src/type_tracking.rs` owns the proof machinery itself:
`ProofGap` has a private constructor and `prove_native_kind` is a real
projection check. Test-only mentions there exercise the proof contract.

### RowView Redrive Closure

The initial W91A audit found two live default paths in
`helpers.rs::row_view_field_opcode` that stamped `LoadColF64` without proving
the RowView field is `F64`:

- unsupported field type in a resolved RowView schema
- missing RowView/schema metadata

The redrive changed `row_view_field_opcode` to return `Result<OpCode,
ShapeError>`. Supported schema field types still map only through schema proof:

- `FieldType::F64` -> `LoadColF64`
- `FieldType::I64` / `FieldType::Timestamp` -> `LoadColI64`
- `FieldType::Bool` -> `LoadColBool`
- `FieldType::String` -> `LoadColStr`

Unsupported field types, unknown fields, missing schema registrations, and
locals without RowView schema type information now produce compile-time
`SemanticError`s. `compiler/expressions/property_access.rs` propagates that
error before emitting the typed `ColumnAccess` instruction.

Focused tests were added in `crates/shape-vm/src/compiler/helpers.rs`:

- `row_view_field_opcode_maps_supported_schema_types`
- `row_view_field_opcode_rejects_unsupported_schema_type`
- `row_view_field_opcode_rejects_missing_schema_proof`

Exact supervisor test target when cargo/rustc checks are allowed:

- `cargo test -p shape-vm row_view_field_opcode --lib`

## Supervisor Gate

No remaining W91A typed-opcode proof coverage gate is known. The guard baseline
now expects `unproven_gap: 0`; any future RowView default typed opcode stamp
fails `scripts/check-typed-opcode-proof-coverage.py`.

This audit did not weaken `prove_native_kind` and did not add runtime
inference, tag probing, dynamic fallback, or old carrier paths.
