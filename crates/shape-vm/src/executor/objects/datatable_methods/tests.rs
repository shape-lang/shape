//! DataTable method tests.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! The pre-Wave-6.5 test suite drove the handler bodies through the
//! deleted `ValueWord` carrier (`ValueWord::from_datatable`,
//! `ValueWord::from_array`, `as_typed_object`, etc.) and called handlers
//! through the legacy `&mut [u64]` MethodFnV2 ABI. With every handler
//! body now stubbed as `NotImplemented(SURFACE)` (playbook §7.4 REVISED)
//! the assertions cannot run — every call returns `Err(NotImplemented)`.
//!
//! The full integration suite is a phase-2c follow-up alongside the
//! handler body migrations. The kinded test harness will use
//! `KindedSlot::from_*` constructors for argument synthesis and
//! `read_owned_kinded` / `pop_kinded` for result inspection (no
//! ValueWord materialization at the test boundary).
//!
//! This module is intentionally empty in the post-bulldozer state.

#![allow(dead_code)]
