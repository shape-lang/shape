//! Integration tests for Shape language async and concurrency features.
//!
//! Covers:
//! - Join strategies (all, race, any, settle)
//! - Async scope (structured concurrency boundaries)
//! - Async let (task spawning and binding)
//! - For await (async iteration)

mod async_let;
mod async_scope;
mod for_await;
mod join_strategies;
