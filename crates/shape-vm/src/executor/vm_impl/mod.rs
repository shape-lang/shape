//! VirtualMachine `impl` blocks, split by responsibility.
//!
//! - `init`     — constructor, VM configuration, JIT/tiered compilation setup
//! - `modules`  — stdlib/extension module registration and invocation
//! - `schemas`  — typed object creation, schema lookup and derivation
//! - `program`  — program loading, linking, hot-patching, reset
//! - `output`   — output capture, error info, module binding helpers
//! - `builtins` — `op_builtin_call` dispatch table
//! - `stack`    — stack push/pop, enum creation, hash helpers

mod init;
mod modules;
mod schemas;
mod program;
mod output;
mod builtins;
mod stack;
