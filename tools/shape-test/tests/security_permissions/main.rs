//! Integration tests for the security and permission system.
//!
//! Shape programs operate under a permission model:
//! - Capability tags map stdlib functions to required permissions (FsRead, FsWrite,
//!   NetConnect, NetListen, Process, Env, Time)
//! - The compiler checks imports against the active PermissionSet at compile time
//! - RuntimePolicy enforces filesystem/network/resource limits at runtime
//!
//! Many of these tests currently fail (TDD) because the ShapeTest builder does not
//! yet expose permission_set configuration. The tests document expected behavior.

mod compile_time;
mod runtime_gating;
