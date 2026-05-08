//! Wave 5d (phase-1b-vm): minimize intrinsic body deferred.
//!
//! `builtin_minimize` (L-BFGS optimizer driving a Shape closure) is a
//! closure-driven intrinsic and lives in Wave 5d's territory alongside
//! `map`/`filter`/`reduce`. The previous body called into deleted
//! `ValueWord` machinery and is unreachable today (the
//! `IntrinsicMinimize` arm of `op_builtin_call` is `todo!`-stubbed for
//! Wave 5d).
//!
//! This file is intentionally empty post-Wave 5b. Wave 5d migrates the
//! body to `(&mut VirtualMachine, &[KindedSlot], &mut ExecutionContext) -> Result<KindedSlot, VMError>`
//! once the closure-call API converges.
