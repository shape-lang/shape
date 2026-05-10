//! Integration tests for DateTime builtins, std::time module, and std::io module.
//!
//! - `datetime`: Shape source-level tests for DateTime constructors and methods
//! - `time_module`: Native module API tests for precision timing
//! - `io_module`: Native module API tests for file system operations
//!
//! ADR-006 §2.7.4 (W11-tail, 2026-05-10): the three submodules below were
//! written against the deleted `ValueWord`/`ValueWordExt`/`vmarray_from_vec`
//! API and the pre-§2.7.10 `ModuleExports::invoke_export` shim plus
//! deleted `file_ops::io_*` / `path_ops::io_*` exports. They need a wholesale
//! rewrite onto the §2.7.10/Q11 `KindedSlot` dispatch shape and the
//! current `time`/`io` module surfaces. Gated until that follow-up lands.

#[cfg(any())]
mod datetime;
#[cfg(any())]
mod io_module;
#[cfg(any())]
mod time_module;
