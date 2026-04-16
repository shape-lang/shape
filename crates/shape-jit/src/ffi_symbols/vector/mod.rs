// ============================================================================
// Vector Intrinsics
// ============================================================================

use super::super::context::JITContext;
use super::super::ffi::object::conversion::{jit_bits_to_nanboxed_with_ctx, nanboxed_to_jit_bits};
use super::super::nan_boxing::*;
use shape_value::{ValueWord, ValueWordExt};

fn jit_to_nb(bits: u64, ctx: *mut JITContext) -> ValueWord {
    jit_bits_to_nanboxed_with_ctx(bits, ctx)
}

fn nb_to_bits(nb: ValueWord) -> u64 {
    nanboxed_to_jit_bits(&nb)
}

pub extern "C" fn jit_intrinsic_vec_abs(ctx: *mut JITContext, arg_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let arg = jit_to_nb(arg_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_abs(&[arg], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_sqrt(ctx: *mut JITContext, arg_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let arg = jit_to_nb(arg_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_sqrt(&[arg], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_ln(ctx: *mut JITContext, arg_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let arg = jit_to_nb(arg_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_ln(&[arg], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_exp(ctx: *mut JITContext, arg_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let arg = jit_to_nb(arg_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_exp(&[arg], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_add(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_add(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_sub(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_sub(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_mul(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_mul(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_div(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_div(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_max(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_max(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_vec_min(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::vector::intrinsic_vec_min(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_matmul_vec(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::matrix::intrinsic_matmul_vec(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}

pub extern "C" fn jit_intrinsic_matmul_mat(ctx: *mut JITContext, a_bits: u64, b_bits: u64) -> u64 {
    if ctx.is_null() {
        return TAG_NULL;
    }
    let a = jit_to_nb(a_bits, ctx);
    let b = jit_to_nb(b_bits, ctx);
    let mut exec_ctx = shape_runtime::context::ExecutionContext::new_empty();
    match shape_runtime::intrinsics::matrix::intrinsic_matmul_mat(&[a, b], &mut exec_ctx) {
        Ok(res) => nb_to_bits(res),
        Err(_) => TAG_NULL,
    }
}
