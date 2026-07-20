//! ADR-009 C3 #14 (slice 1) — template specialization: the INSTALL-side twin of
//! the `CheckedTemplate` construction chokepoint.
//!
//! # The construction/install split (mirrors `comptime_fragments/checked_body.rs:8-35`)
//!
//! The C2/E1 checked-body surface established the binding split this module
//! inherits: `comptime_fragments::checked_template` is the CONSTRUCTION
//! chokepoint (typestate builder, `finish()` only on complete states, no string
//! constructor); THIS module is where a constructed template meets a frozen
//! target — per-specialization checking (C3-G4/G10: the template body fn
//! type-checked against the bound Sig, riding
//! `ensure_monomorphic_function_for_callsite`, EMISSION tier + MIR battery) and
//! the G9 pseudo-tuple resolution (constant `args[i]` → the i-th typed
//! parameter slot; `args.length` → a constant; mutation-return → a
//! compiler-internal per-target aggregate at the weave boundary).
//!
//! Slice-1 staging: this stage (S1b) lands the module home + the single
//! pseudo-tuple traversal core ([`pseudo_tuple`]); the specialization driver
//! (`specialize`-shaped entry, S1c) lands in the next stage and composes with
//! the ALREADY-OPEN C2 `InstallTransaction` (E1-D6b atomicity-by-composition —
//! never a second transaction; origin threaded as a parameter).
//!
//! # Not a foundation: the legacy weave
//!
//! The legacy hook machinery (`compile_specialized_annotation_handler`,
//! `specialize_annotation_runtime_handlers`, `compile_annotation_wrapper`, the
//! homogeneous args array) is a C3-G7 DELETION target. Nothing in this module
//! may call into it, extend it, or depend on its carriers; the new path is
//! built BESIDE it and the S6 capstone deletes it whole.

pub(in crate::compiler) mod pseudo_tuple;
