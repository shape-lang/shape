//! VirtualMachine `impl` blocks, split by responsibility.
//!
//! - `init`     — constructor, VM configuration, JIT/tiered compilation setup
//! - `modules`  — stdlib/extension module registration and invocation
//! - `schemas`  — typed object creation, schema lookup and derivation
//! - `program`  — program loading, linking, hot-patching, reset
//! - `output`   — output capture, error info, module binding helpers
//! - `builtins` — `op_builtin_call` dispatch table
//! - `stack`    — stack push/pop, enum creation, hash helpers

// W18.5 (R8 W4, 2026-05-24 — supervisor D4): visibility widened from
// private `mod` to `pub(in crate::executor)` so the Content builder
// method handlers in `objects/content_methods.rs` can call the
// `read_string_array` / `read_content_arc` free-function helpers
// defined here for `Content.table` / `Content.kv` / `Content.fragment`
// arg-list parsing. No new public crate surface — `executor`-internal only.
pub(in crate::executor) mod builtins;
mod init;
mod modules;
mod output;
mod program;
mod schemas;
pub(crate) mod stack;
