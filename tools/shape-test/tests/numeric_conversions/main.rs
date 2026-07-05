//! # Numeric-conversion CONFORMANCE regression suite (PERMANENT)
//!
//! This is the permanent regression suite for THE RULE (user 2026-06-01,
//! binding) on numeric conversions in strict-typing Shape. It is the
//! executable encoding of
//! `docs/design/numeric-conversion/numeric-conversion-spec.md`.
//!
//! ## THE RULE (verbatim intent)
//!
//! > No "lossless enough." A numeric conversion is implicit ONLY if it is
//! > TRULY LOSSLESS — every value of the source type is exactly representable
//! > in the target (strict widening, e.g. `u8 -> u16`, `i32 -> i64`).
//! > Everything else — `int <-> number` BOTH directions, any lossy width
//! > narrowing (`u16 -> u8`, `i64 -> i32`), `number -> int` — REQUIRES an
//! > EXPLICIT cast (`x as T`). A silent lossy conversion is FORBIDDEN (a
//! > correctness bug).
//! >
//! > NUMERIC LITERALS adopt their context type when the literal value is
//! > losslessly representable in it (a small int literal in a `number` context
//! > IS a number literal — no conversion).
//! >
//! > Value-level `int != number` holds: an int VARIABLE/VALUE is never
//! > silently a number.
//!
//! ## Suite organization (categories A–E)
//!
//! - [`category_a_lossless_widening`] — LOSSLESS-WIDENING -> must ACCEPT and
//!   round-trip the value (the §2 lattice IMPL cells).
//! - [`category_b_lossy_implicit_rejected`] — LOSSY / NON-SUBSET implicit
//!   WITHOUT cast -> must COMPILE-REJECT (`int<->number` value, `number->int`,
//!   `u16->u8`, `i64->i32`, `u8->i8`, ...). Proves no silent conversion.
//! - [`category_c_explicit_casts`] — EXPLICIT CASTS -> must ACCEPT with the
//!   defined §3 result (`int as number`, `number as int` trunc-toward-zero,
//!   `u16 as u8 = wrap mod 256`, ...).
//! - [`category_d_literal_adoption`] — LITERAL ADOPTION -> in-range literal in
//!   a numeric context ACCEPTs and adopts the context type; out-of-range
//!   literal REJECTs.
//! - [`category_e_silent_lossy_forbidden`] — SILENT-LOSSY FORBIDDEN: focused
//!   data-loss witnesses proving the (B) rejects guard a real corruption
//!   (300 -> 44, -5 -> 65531, etc.).
//!
//! ## TDD red-phase posture
//!
//! This module is the RED baseline of the strict numeric-conversion fix
//! workstream. Tests assert THE RULE's *target* behavior. On the
//! `0cfb1b11` baseline binary, the conformance gaps (spec §6) make a subset
//! RED (failing); the fix workstream turns them GREEN. The suite is NEVER
//! relaxed to match a regressed binary — it is the source of truth.
//!
//! Probe binary for the baseline: `target/release/shape run --mode vm`.
//!
//! ## Dual-mode execution (WF-0B, 2026-07-05)
//!
//! This target runs the suite under the bytecode INTERPRETER (`--mode vm`
//! equivalent). The sibling target `numeric_conversions_jit` `#[path]`-includes
//! the SAME five category files (test bodies are byte-identical) and runs them
//! through `shape_jit::JITExecutor` (`--mode jit` equivalent), with an explicit
//! known-divergence list for the audit-2026-07-04 §4.4 JIT correctness gaps.
//! The `crate::suite::ShapeTest` indirection below is what lets each target
//! choose its executor without touching the conformance test bodies.

/// Executor selection shim: in this target `ShapeTest` is the plain
/// (bytecode-VM) builder. The JIT sibling target substitutes a wrapper.
pub(crate) mod suite {
    pub use shape_test::shape_test::ShapeTest;
}

mod category_a_lossless_widening;
mod category_b_lossy_implicit_rejected;
mod category_c_explicit_casts;
mod category_d_literal_adoption;
mod category_e_silent_lossy_forbidden;
