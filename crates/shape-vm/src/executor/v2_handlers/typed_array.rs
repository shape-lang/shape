//! v2 handler functions for typed array operations (Phase 2c stubs).
//!
//! ## Status
//!
//! The legacy `shape_value::v2_typed_array` primitive layer was deleted
//! during the strict-typing bulldozer cycles. These handler functions are
//! orphaned dispatch surface — Phase 2c is the natural reentry point to
//! either re-emit them on top of the kinded `Arc<TypedArrayData>` model or
//! delete the dispatch surface entirely.
//!
//! Until then the handlers exist as kinded-API stubs that return
//! `VMError::NotImplemented` if reached. The 8-byte slot ABI is preserved:
//! every entry uses `push_kinded` / `pop_kinded` so the parallel kind track
//! stays in lockstep (ADR-006 §2.7.7).

#![allow(unsafe_op_in_unsafe_fn, dead_code, unused_unsafe)]

use shape_value::{NativeKind, VMError};

use crate::executor::vm_impl::stack::drop_with_kind;
use crate::executor::VirtualMachine;

#[inline(always)]
fn alloc_stub(vm: &mut VirtualMachine, _capacity: u32) -> Result<(), VMError> {
    vm.push_kinded(0u64, NativeKind::UInt64)?;
    Err(VMError::NotImplemented(
        "v2_typed_array primitive layer deleted — pending Phase 2c re-emission".into(),
    ))
}

#[inline(always)]
fn unary_stub(vm: &mut VirtualMachine, op: &str) -> Result<(), VMError> {
    let (b, k) = vm.pop_kinded()?;
    drop_with_kind(b, k);
    Err(VMError::NotImplemented(format!(
        "v2_typed_array primitive layer deleted — {} pending Phase 2c re-emission",
        op
    )))
}

#[inline(always)]
fn binary_stub(vm: &mut VirtualMachine, op: &str) -> Result<(), VMError> {
    let (b1, k1) = vm.pop_kinded()?;
    let (b2, k2) = vm.pop_kinded()?;
    drop_with_kind(b1, k1);
    drop_with_kind(b2, k2);
    Err(VMError::NotImplemented(format!(
        "v2_typed_array primitive layer deleted — {} pending Phase 2c re-emission",
        op
    )))
}

#[inline(always)]
fn ternary_stub(vm: &mut VirtualMachine, op: &str) -> Result<(), VMError> {
    let (b1, k1) = vm.pop_kinded()?;
    let (b2, k2) = vm.pop_kinded()?;
    let (b3, k3) = vm.pop_kinded()?;
    drop_with_kind(b1, k1);
    drop_with_kind(b2, k2);
    drop_with_kind(b3, k3);
    Err(VMError::NotImplemented(format!(
        "v2_typed_array primitive layer deleted — {} pending Phase 2c re-emission",
        op
    )))
}

// ---------------------------------------------------------------------------
// Alloc / Free
// ---------------------------------------------------------------------------

pub fn op_typed_array_alloc_f64(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    alloc_stub(vm, capacity)
}

pub fn op_typed_array_alloc_i64(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    alloc_stub(vm, capacity)
}

pub fn op_typed_array_alloc_i32(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    alloc_stub(vm, capacity)
}

pub fn op_typed_array_free(vm: &mut VirtualMachine) -> Result<(), VMError> {
    unary_stub(vm, "TypedArrayFree")
}

// ---------------------------------------------------------------------------
// Get (element access)
// ---------------------------------------------------------------------------

pub fn op_typed_array_get_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    binary_stub(vm, "TypedArrayGetF64")
}

pub fn op_typed_array_get_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    binary_stub(vm, "TypedArrayGetI64")
}

pub fn op_typed_array_get_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    binary_stub(vm, "TypedArrayGetI32")
}

// ---------------------------------------------------------------------------
// Set (element mutation)
// ---------------------------------------------------------------------------

pub fn op_typed_array_set_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    ternary_stub(vm, "TypedArraySetF64")
}

pub fn op_typed_array_set_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    ternary_stub(vm, "TypedArraySetI64")
}

pub fn op_typed_array_set_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    ternary_stub(vm, "TypedArraySetI32")
}

// ---------------------------------------------------------------------------
// Push (append element)
// ---------------------------------------------------------------------------

pub fn op_typed_array_push_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    binary_stub(vm, "TypedArrayPushF64")
}

pub fn op_typed_array_push_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    binary_stub(vm, "TypedArrayPushI64")
}

pub fn op_typed_array_push_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    binary_stub(vm, "TypedArrayPushI32")
}

// ---------------------------------------------------------------------------
// Len
// ---------------------------------------------------------------------------

pub fn op_typed_array_len(vm: &mut VirtualMachine) -> Result<(), VMError> {
    unary_stub(vm, "TypedArrayLen")
}
