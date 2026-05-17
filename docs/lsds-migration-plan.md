# LSDS migration plan

Phase 2 of ADR-006 (`docs/adr/006-value-and-memory-model.md` §9) migrates
the compiler's diagnostic-emission sites to produce
[`shape_diagnostics::Diagnostic`] (LSDS) as the canonical wire format.
Renderers (terminal, LSP, MCP) consume LSDS rather than produce it.

The first session (2026-05-08) shipped:

- `crates/shape-diagnostics/` — schema crate with serde-derive JSON
  round-trip, terminal renderer, and snapshot tests.
- Vertical slice for the **B-series borrow / lifetime / aliasing
  diagnostics** (B0001..B0014) emitted by the MIR borrow solver:
  - `BytecodeCompiler::borrow_error_to_lsds` builds the LSDS;
  - `mir_borrow_error` consumes the LSDS and derives the legacy
    `ShapeError::SemanticError` via a small bridge (preserves
    `[B00XX]` prefix in test snapshots);
  - JSON round-trip + terminal snapshot tests in
    `crates/shape-diagnostics/tests/lsds_round_trip.rs`.

The remainder of Phase 2 is the mechanical migration of the other
emission sites. This document enumerates them, categorizes by
complexity, and proposes an ordering for subsequent sessions.

## Inventory

Counts are from `rg --count-matches` over `crates/`, `tools/`, `bin/`
on the worktree at session-1 close. Numbers will drift as code
evolves — re-grep before starting a new session.

### By crate (all `ShapeError` variants combined)

| Crate / dir | `SemanticError` | `TypeError` | `ParseError` | `RuntimeError` | Other | Total |
|---|---:|---:|---:|---:|---:|---:|
| `crates/shape-ast` | 9 | 4 | (StructuredParse: 21) | 0 | many | ~488 |
| `crates/shape-vm` | 252 | 0 | many | small | small | ~342 |
| `crates/shape-runtime` | 10 | 13 | small | many | many | ~256 |
| `tools/shape-lsp` | 7 | 1 | 5 | 0 | small | ~11 |
| `bin/shape-cli` | 4 | 0 | small | small | small | ~9 |
| `crates/shape-jit` | small | 0 | 0 | small | small | ~7 |

The `crates/shape-diagnostics` row in the raw rg output is the LSDS
crate's own test sample text; not a migration target.

### By diagnostic family

| Family | Emission focus | Existing structure | Codes | Sites |
|---|---|---|---|---:|
| **B-series — borrow / lifetime / aliasing** | `crates/shape-vm/src/mir/{solver,analysis,lowering,repair}.rs` → `BytecodeCompiler::mir_borrow_error` (functions.rs:636) | `BorrowError` struct with kind + spans + repairs, mapped to `BorrowErrorCode` (`B0001`..`B0014`) | B0001..B0014 (10 codes, 14 kinds) | 1 sink (mir_borrow_error). **DONE — session 1.** |
| **E-series — type-system / type inference** | `crates/shape-runtime/src/type_system/` (much of it via `ShapeError::TypeError(String)`) | Mostly bare `String` errors today; no structured data | None — string-only | ~20 (in shape-runtime; concentrated in checking/ and unification) |
| **E-series — semantic / compile-time** | `crates/shape-vm/src/compiler/{statements,expressions/*,functions,functions_annotations}.rs` | `ShapeError::SemanticError { message, location }` with `SourceLocation { hints, notes, ... }` | `[E0XXX]` ad-hoc; mostly unprefixed | ~252 (largest category) |
| **Parse / Lex** | `crates/shape-ast/src/parser/` | `StructuredParseError` (21 sites) — already structured | Various | ~26 |
| **Pattern / Module / Runtime** | `crates/shape-runtime/src/`, scattered | `ShapeError::PatternError` / `ModuleError` / `RuntimeError` — semi-structured | Various | small per family |
| **MutabilityError (`mir_mutability_error`)** | `functions.rs:661` | Structured | `[mut]` ad-hoc | 1 sink |

### MIR borrow solver — closed in session 1

| Code | Kind | Site (functions.rs) | LSDS via |
|---|---|---|---|
| B0001 | ConflictSharedExclusive, ConflictExclusiveExclusive, ReadWhileExclusivelyBorrowed | mir_borrow_error | borrow_error_to_lsds |
| B0002 | WriteWhileBorrowed | mir_borrow_error | borrow_error_to_lsds |
| B0003 | ReferenceEscape, ReferenceEscapeIntoClosure | mir_borrow_error | borrow_error_to_lsds |
| B0004 | ReferenceStoredInArray, ReferenceStoredInObject, ReferenceStoredInEnum | mir_borrow_error | borrow_error_to_lsds |
| B0005 | UseAfterMove | mir_borrow_error | borrow_error_to_lsds |
| B0006 | ExclusiveRefAcrossTaskBoundary | mir_borrow_error | borrow_error_to_lsds |
| B0007 | InconsistentReferenceReturn | mir_borrow_error | borrow_error_to_lsds |
| B0012 | SharedRefAcrossDetachedTask | mir_borrow_error | borrow_error_to_lsds |
| B0013 | CallSiteAliasConflict | mir_borrow_error | borrow_error_to_lsds (snapshot-tested) |
| B0014 | NonSendableAcrossTaskBoundary | mir_borrow_error | borrow_error_to_lsds |

## Ordering for subsequent sessions

Ranked by leverage (impact ÷ migration cost). Each session lands one or
two adjacent rows, commits LSDS-first emission, and removes the
`String`-message emission path where possible.

### Session 2 — `MutabilityError` + structured parse errors

**Scope:**
- `BytecodeCompiler::mir_mutability_error` (functions.rs:661) — single
  sink; structurally identical to mir_borrow_error.
- `StructuredParseError` (21 sites in `crates/shape-ast/src/parser/`) —
  already structured; mechanical conversion to LSDS via a
  `into_lsds()` impl on `StructuredParseError`.

**Why first:** Both are already structured; no message-string
archaeology required. Establishes the pattern for "structured emitter →
LSDS" before tackling the harder `String`-message sites.

**Estimated cost:** 1 day.

### Session 3 — Type-system errors (`shape-runtime/src/type_system/`)

**Scope:** ~20 sites in:
- `crates/shape-runtime/src/type_system/checking/` — type inference + checking failures
- `crates/shape-runtime/src/type_system/unification.rs` — unification failures
- `crates/shape-runtime/src/type_system/types/` — type construction errors

**Approach:**
1. Introduce a `TypeError` enum with kinds (TypeMismatch, UnboundVar,
   ArityMismatch, KindMismatch, ...) parallel to `BorrowErrorKind`. Each
   carries the spans + types + names needed for an LSDS witness.
2. Map kinds to E-series codes (`E0100`..`E0199`).
3. Produce LSDS at the unification-failure boundary; `expected`/`found`
   `TypeWitness` fields populate from `Type::display()` + simple
   value-witness synthesis (int → 0, string → "", bool → false).
4. Bridge to `ShapeError::TypeError(String)` for compatibility.

**Why this slot:** Type errors are the highest-leverage category for
LLM consumption (the `expected`/`found` witness is exactly what makes
LSDS valuable). Establishes the type-witness shape concretely.

**Estimated cost:** 1.5 weeks.

### Session 4 — Compiler/semantic errors (largest category)

**Scope:** 252 sites in `crates/shape-vm/src/compiler/`. Concentrated in:
- `statements.rs` (63 sites)
- `expressions/function_calls.rs` (30)
- `expressions/assignment.rs`, `expressions/collections.rs` (~40)
- `functions_annotations.rs` (20)
- `expressions/primary.rs` and other expression files (~80)

**Approach:**
1. Catalog the message bodies and group into families (binding-related,
   type-coercion, missing-import, undefined-name, etc.).
2. Mint codes (`E0200`..`E0299` for binding/scope, `E0300`..`E0399` for
   coercion, etc.).
3. Migrate one family at a time. Each migration replaces an
   `Err(ShapeError::SemanticError { message: format!("..."), ... })`
   with `Err(diagnostic_to_shape_error(&build_lsds(...)))`.

**Why later:** Largest mechanical surface. Should ride the patterns
established by sessions 2–3. Splittable across multiple sub-sessions if
needed.

**Estimated cost:** 3–4 weeks (split across 2–3 sub-sessions).

### Session 5 — Runtime / pattern / module / data errors

**Scope:** Remaining `RuntimeError`, `PatternError`, `ModuleError`,
`SimulationError`, `DataProviderError`, `StreamError`, etc. across
`crates/shape-runtime/src/` and `bin/shape-cli/`.

**Approach:** Same as session 4; lower priority because runtime errors
are user-runtime-facing rather than compile-time-LLM-facing.

**Estimated cost:** 1–2 weeks.

### Session 6 — LSP renderer

**Scope:** Add `crates/shape-diagnostics/src/render/lsp.rs` that
produces `lsp_types::Diagnostic` from an LSDS. Wire `tools/shape-lsp/`
to consume LSDS directly rather than going through
`ShapeError::SemanticError`.

**Why this slot:** LSP renderer is on the critical path for the
`var`-inlay-hint integration (Phase 1.C deliverable). Should land soon
after sessions 2–3 establish enough LSDS coverage for the LSP server
to receive only LSDS-emitted diagnostics in the common case.

**Estimated cost:** 1 week.

### Session 7 — MCP renderer + token-budgeted context windows

**Scope:**
- `crates/shape-diagnostics/src/render/mcp.rs` — structured MCP tool
  responses.
- Implement `ContextWindow` population with a real cl100k tokenizer
  (`tiktoken-rs` or equivalent).
- Audit token budget per ADR-006 §13.5 (≤500 cl100k tokens per error).

**Estimated cost:** 1–2 weeks (tokenizer integration is the long pole).

### Session 8 — Suggested-fix code-diff generation

**Scope:** Populate `SuggestedFix.diff` with unified-diff fragments for
common error classes (missing import, type coercion, borrow violation
repair). Per ADR-006 §9.4.

**Estimated cost:** 2 weeks.

## Out of scope for Phase 2

- `var` inlay-hint integration (Phase 1.C territory; LSDS is the
  carrier; the inlay-hint logic is post-1.C).
- Type-witness enumeration for recursive / generic / trait-bounded types
  (defer indefinitely; surface name suffices).
- Removal of `ShapeError` (the bridge stays as long as any consumer
  depends on the legacy variants — that's a future ADR).

## Success metrics (per ADR-006 §13.5)

- ≥95% of compiler errors emit LSDS with `expected`/`found` and at
  least one suggested-fix populated.
- Average LSDS payload ≤500 cl100k tokens per error.
- Zero direct `Err(ShapeError::Foo { message: format!("..."), ... })`
  in `crates/shape-vm/src/compiler/` and `crates/shape-runtime/src/
  type_system/` (verified by lint or grep).

## References

- ADR-006 §9 — binding spec.
- `crates/shape-diagnostics/src/lib.rs` — schema.
- `crates/shape-diagnostics/src/render/terminal.rs` — terminal renderer.
- `crates/shape-diagnostics/tests/lsds_round_trip.rs` — vertical-slice
  test.
- `crates/shape-vm/src/compiler/functions.rs` —
  `borrow_error_to_lsds`, `diagnostic_to_shape_error` bridge.
- `crates/shape-vm/src/mir/analysis.rs` — `BorrowError`,
  `BorrowErrorKind`, `BorrowErrorCode`.
