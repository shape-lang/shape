//! Tests for the `http` stdlib module.
//!
//! The http module provides async functions: http::get, http::post, http::put,
//! http::delete. All require network access and NetConnect permission.
//! Imported via `use std::core::http`.

mod basic;
