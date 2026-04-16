//! Native C ABI linking and invocation for `extern C` foreign functions.

use crate::bytecode::{NativeAbiSpec, NativeStructLayoutEntry};
use libffi::{
    low,
    middle::{Arg, Cif, Closure, CodePtr, Type},
};
use libloading::Library;
use shape_runtime::module_exports::RawCallableInvoker;
use shape_value::{
    ValueWord, ValueWordExt,
    heap_value::{HeapValue, NativeLayoutField, NativeScalar, NativeTypeLayout},
};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CType {
    I8,
    U8,
    I16,
    U16,
    I32,
    I64,
    U32,
    U64,
    Isize,
    Usize,
    F32,
    F64,
    Bool,
    CString,
    NullableCString,
    CSlice(Box<CType>),
    CMutSlice(Box<CType>),
    CView(String),
    CMut(String),
    Ptr,
    Callback(Box<CallbackSignature>),
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallbackSignature {
    params: Vec<CType>,
    ret: CType,
}

/// Extract the inner type argument from a generic type like `name<inner>`.
///
/// Given `compact` (whitespace-stripped original) and `type_name` (e.g. "cview"),
/// returns the trimmed inner string. Returns an error if the angle-bracket
/// extraction fails or the inner string is empty.
fn parse_generic_type_arg<'a>(
    compact: &'a str,
    type_name: &str,
) -> Result<&'a str, String> {
    compact
        .split_once('<')
        .and_then(|(_, rest)| rest.strip_suffix('>'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{}<T> requires a type argument", type_name))
}

impl CType {
    fn parse(token: &str) -> Result<Self, String> {
        let compact = token
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        let normalized = compact.to_ascii_lowercase();

        if normalized.starts_with("callback(") && normalized.ends_with(')') {
            let inner = compact
                .strip_prefix("callback(")
                .and_then(|rest| rest.strip_suffix(')'))
                .ok_or_else(|| format!("invalid callback type syntax '{}'", token))?;
            let parsed = parse_signature(inner)?;
            if matches!(
                parsed.ret,
                CType::CString | CType::NullableCString | CType::CSlice(_) | CType::CMutSlice(_)
            ) {
                return Err("callback return type `cstring`/`cstring?`/`cslice<_>`/`cmut_slice<_>` is not supported".to_string());
            }
            return Ok(Self::Callback(Box::new(CallbackSignature {
                params: parsed.params,
                ret: parsed.ret,
            })));
        }

        if normalized.starts_with("cview<") && normalized.ends_with('>') {
            let inner = parse_generic_type_arg(&compact, "cview")?;
            return Ok(Self::CView(inner.to_string()));
        }

        if normalized.starts_with("cmut<") && normalized.ends_with('>') {
            let inner = parse_generic_type_arg(&compact, "cmut")?;
            return Ok(Self::CMut(inner.to_string()));
        }

        if normalized.starts_with("cslice<") && normalized.ends_with('>') {
            let inner = parse_generic_type_arg(&compact, "cslice")?;
            let elem = CType::parse(inner)?;
            if !is_supported_slice_element_type(&elem) {
                return Err(format!(
                    "cslice<T> does not support element type '{}'",
                    inner
                ));
            }
            return Ok(Self::CSlice(Box::new(elem)));
        }

        if normalized.starts_with("cmut_slice<") && normalized.ends_with('>') {
            let inner = parse_generic_type_arg(&compact, "cmut_slice")?;
            let elem = CType::parse(inner)?;
            if !is_supported_slice_element_type(&elem) {
                return Err(format!(
                    "cmut_slice<T> does not support element type '{}'",
                    inner
                ));
            }
            return Ok(Self::CMutSlice(Box::new(elem)));
        }

        match normalized.as_str() {
            "i8" => Ok(Self::I8),
            "u8" => Ok(Self::U8),
            "i16" => Ok(Self::I16),
            "u16" => Ok(Self::U16),
            "i32" => Ok(Self::I32),
            "i64" => Ok(Self::I64),
            "u32" => Ok(Self::U32),
            "u64" => Ok(Self::U64),
            "isize" => Ok(Self::Isize),
            "usize" => Ok(Self::Usize),
            "f32" => Ok(Self::F32),
            "f64" => Ok(Self::F64),
            "bool" => Ok(Self::Bool),
            "cstring" => Ok(Self::CString),
            "cstring?" => Ok(Self::NullableCString),
            "ptr" => Ok(Self::Ptr),
            "void" => Ok(Self::Void),
            other => Err(format!(
                "unsupported native C type '{}'; supported: i8, u8, i16, u16, i32, i64, u32, u64, isize, usize, f32, f64, bool, cstring, cstring?, cslice<...>, cmut_slice<...>, cview<...>, cmut<...>, ptr, callback(...), void",
                other
            )),
        }
    }
}

fn is_supported_slice_element_type(ctype: &CType) -> bool {
    matches!(
        ctype,
        CType::I8
            | CType::U8
            | CType::I16
            | CType::U16
            | CType::I32
            | CType::I64
            | CType::U32
            | CType::U64
            | CType::Isize
            | CType::Usize
            | CType::F32
            | CType::F64
            | CType::Bool
            | CType::CString
            | CType::NullableCString
            | CType::Ptr
    )
}

#[derive(Debug, Clone)]
struct CSignature {
    params: Vec<CType>,
    ret: CType,
}

fn build_native_layout_map(
    entries: &[NativeStructLayoutEntry],
) -> HashMap<String, Arc<NativeTypeLayout>> {
    let mut layouts = HashMap::with_capacity(entries.len());
    for entry in entries {
        let mapped = NativeTypeLayout {
            name: entry.name.clone(),
            abi: entry.abi.clone(),
            size: entry.size,
            align: entry.align,
            fields: entry
                .fields
                .iter()
                .map(|field| NativeLayoutField {
                    name: field.name.clone(),
                    c_type: field.c_type.clone(),
                    offset: field.offset,
                    size: field.size,
                    align: field.align,
                })
                .collect(),
        };
        layouts.insert(entry.name.clone(), Arc::new(mapped));
    }
    layouts
}

fn collect_layout_references<'a>(ctype: &'a CType, out: &mut Vec<&'a str>) {
    match ctype {
        CType::CView(name) | CType::CMut(name) => out.push(name.as_str()),
        CType::CSlice(elem) | CType::CMutSlice(elem) => collect_layout_references(elem, out),
        CType::Callback(sig) => {
            for param in &sig.params {
                collect_layout_references(param, out);
            }
            collect_layout_references(&sig.ret, out);
        }
        _ => {}
    }
}

fn validate_layout_references(
    signature: &CSignature,
    layouts: &HashMap<String, Arc<NativeTypeLayout>>,
) -> Result<(), String> {
    let mut refs = Vec::new();
    for param in &signature.params {
        collect_layout_references(param, &mut refs);
    }
    collect_layout_references(&signature.ret, &mut refs);

    for layout_name in refs {
        if !layouts.contains_key(layout_name) {
            return Err(format!(
                "native signature references unknown `type C` layout '{}'",
                layout_name
            ));
        }
    }
    Ok(())
}

/// Linked native function handle used by VM foreign-call dispatch.
pub struct NativeLinkedFunction {
    signature: CSignature,
    cif: Cif,
    code_ptr: CodePtr,
    layouts: HashMap<String, Arc<NativeTypeLayout>>,
    /// Keep the dynamic library alive for symbol/call lifetime.
    _library: Arc<Library>,
}

pub fn link_native_function(
    spec: &NativeAbiSpec,
    native_layouts: &[NativeStructLayoutEntry],
    library_cache: &mut HashMap<String, Arc<Library>>,
) -> Result<NativeLinkedFunction, String> {
    if spec.abi != "C" {
        return Err(format!(
            "unsupported native ABI '{}'; only \"C\" is currently supported",
            spec.abi
        ));
    }

    let signature = parse_signature(&spec.signature)?;
    let layouts = build_native_layout_map(native_layouts);
    validate_layout_references(&signature, &layouts)?;
    let library = if let Some(existing) = library_cache.get(&spec.library) {
        existing.clone()
    } else {
        let opened = unsafe { Library::new(&spec.library) }
            .map_err(|e| format!("failed to open native library '{}': {}", spec.library, e))?;
        let shared = Arc::new(opened);
        library_cache.insert(spec.library.clone(), shared.clone());
        shared
    };

    let mut symbol_bytes = spec.symbol.as_bytes().to_vec();
    if !symbol_bytes.ends_with(&[0]) {
        symbol_bytes.push(0);
    }
    let symbol_ptr = unsafe { library.get::<*const c_void>(&symbol_bytes) }
        .map_err(|e| {
            format!(
                "failed to resolve native symbol '{}' from '{}': {}",
                spec.symbol, spec.library, e
            )
        })
        .map(|sym| *sym)?;

    let arg_types: Vec<Type> = signature.params.iter().map(c_type_to_ffi_type).collect();
    let cif = Cif::new(arg_types, c_type_to_ffi_type(&signature.ret));
    let code_ptr = CodePtr::from_ptr(symbol_ptr as *mut c_void);

    Ok(NativeLinkedFunction {
        signature,
        cif,
        code_ptr,
        layouts,
        _library: library,
    })
}

#[derive(Debug, Clone)]
enum MutableArgWritebackPlan {
    Slice {
        arg_index: usize,
        target_slot: usize,
        elem_type: CType,
    },
}

fn resolve_arg_value_for_native_call(
    value: &ValueWord,
    arg_idx: usize,
    vm_stack: Option<&[ValueWord]>,
) -> Result<(ValueWord, Option<usize>), String> {
    if let Some(slot) = value.as_ref_slot() {
        let stack = vm_stack.ok_or_else(|| {
            format!(
                "native call arg#{arg_idx} received a reference argument but no VM stack context is available"
            )
        })?;
        let source = stack.get(slot).ok_or_else(|| {
            format!(
                "native call arg#{arg_idx} references invalid stack slot {} (stack len {})",
                slot,
                stack.len()
            )
        })?;
        Ok((source.clone(), Some(slot)))
    } else {
        Ok((value.clone(), None))
    }
}

fn build_mutable_writeback_plan(
    ctype: &CType,
    arg_idx: usize,
    source_ref_slot: Option<usize>,
) -> Result<Option<MutableArgWritebackPlan>, String> {
    match ctype {
        CType::CMutSlice(elem) => {
            let target_slot = source_ref_slot.ok_or_else(|| {
                format!(
                    "native call arg#{arg_idx} for {} requires a mutable reference argument",
                    c_type_label(ctype)
                )
            })?;
            Ok(Some(MutableArgWritebackPlan::Slice {
                arg_index: arg_idx,
                target_slot,
                elem_type: elem.as_ref().clone(),
            }))
        }
        _ => Ok(None),
    }
}

fn apply_mutable_writebacks(
    stack: &mut [ValueWord],
    prepared_args: &[PreparedArg],
    writebacks: &[MutableArgWritebackPlan],
) -> Result<(), String> {
    for plan in writebacks {
        match plan {
            MutableArgWritebackPlan::Slice {
                arg_index,
                target_slot,
                elem_type,
            } => {
                if *target_slot >= stack.len() {
                    return Err(format!(
                        "native call writeback target slot {} out of bounds (stack len {})",
                        target_slot,
                        stack.len()
                    ));
                }
                let prepared = prepared_args.get(*arg_index).ok_or_else(|| {
                    format!(
                        "native call writeback references missing prepared arg at index {}",
                        arg_index
                    )
                })?;
                let PreparedArg::SliceDesc { desc, .. } = prepared else {
                    return Err(format!(
                        "native call writeback expected slice argument at index {}",
                        arg_index
                    ));
                };
                let decoded = decode_slice_elements(
                    *desc,
                    elem_type,
                    &format!("native call arg#{arg_index} writeback"),
                )?;
                stack[*target_slot] = ValueWord::from_array(Arc::new(decoded));
            }
        }
    }
    Ok(())
}

pub fn invoke_linked_function(
    linked: &NativeLinkedFunction,
    args: &[ValueWord],
    raw_invoker: Option<RawCallableInvoker>,
    vm_stack: Option<&mut [ValueWord]>,
) -> Result<ValueWord, String> {
    if linked.signature.params.len() != args.len() {
        return Err(format!(
            "native ABI argument count mismatch: signature expects {}, got {}",
            linked.signature.params.len(),
            args.len()
        ));
    }

    let mut owned_cstrings: Vec<CString> = Vec::new();
    let mut owned_callbacks: Vec<OwnedCallScopedCallback> = Vec::new();
    let mut prepared_args = Vec::with_capacity(linked.signature.params.len());
    let mut pending_writebacks = Vec::new();

    let stack_view = vm_stack.as_ref().map(|stack| &stack[..]);
    for (idx, (ctype, value)) in linked.signature.params.iter().zip(args.iter()).enumerate() {
        let (resolved_value, source_ref_slot) =
            resolve_arg_value_for_native_call(value, idx, stack_view)?;
        if let Some(plan) = build_mutable_writeback_plan(ctype, idx, source_ref_slot)? {
            pending_writebacks.push(plan);
        }
        prepared_args.push(encode_arg(
            &resolved_value,
            ctype,
            idx,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &linked.layouts,
            raw_invoker,
        )?);
    }

    let ffi_args: Vec<Arg> = prepared_args.iter().map(PreparedArg::as_arg).collect();
    let result = match &linked.signature.ret {
        CType::Void => {
            unsafe { linked.cif.call::<()>(linked.code_ptr, &ffi_args) };
            ValueWord::unit()
        }
        CType::I8 => {
            let out = unsafe { linked.cif.call::<i8>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_i8(out)
        }
        CType::U8 => {
            let out = unsafe { linked.cif.call::<u8>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_u8(out)
        }
        CType::I16 => {
            let out = unsafe { linked.cif.call::<i16>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_i16(out)
        }
        CType::U16 => {
            let out = unsafe { linked.cif.call::<u16>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_u16(out)
        }
        CType::I32 => {
            let out = unsafe { linked.cif.call::<i32>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_i32(out)
        }
        CType::I64 => {
            let out = unsafe { linked.cif.call::<i64>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_scalar(NativeScalar::I64(out))
        }
        CType::U32 => {
            let out = unsafe { linked.cif.call::<u32>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_u32(out)
        }
        CType::U64 => {
            let out = unsafe { linked.cif.call::<u64>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_u64(out)
        }
        CType::Isize => {
            let out = unsafe { linked.cif.call::<isize>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_isize(out)
        }
        CType::Usize => {
            let out = unsafe { linked.cif.call::<usize>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_usize(out)
        }
        CType::Ptr | CType::Callback(_) => {
            let out = unsafe { linked.cif.call::<usize>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_ptr(out)
        }
        CType::F32 => {
            let out = unsafe { linked.cif.call::<f32>(linked.code_ptr, &ffi_args) };
            ValueWord::from_native_f32(out)
        }
        CType::F64 => {
            let out = unsafe { linked.cif.call::<f64>(linked.code_ptr, &ffi_args) };
            ValueWord::from_f64(out)
        }
        CType::Bool => {
            let out = unsafe { linked.cif.call::<u8>(linked.code_ptr, &ffi_args) };
            ValueWord::from_bool(out != 0)
        }
        CType::CString => {
            let out = unsafe { linked.cif.call::<*const c_char>(linked.code_ptr, &ffi_args) };
            if out.is_null() {
                return Err("native call returned null cstring pointer".to_string());
            }
            let s = unsafe { CStr::from_ptr(out) }.to_string_lossy().to_string();
            ValueWord::from_string(Arc::new(s))
        }
        CType::NullableCString => {
            let out = unsafe { linked.cif.call::<*const c_char>(linked.code_ptr, &ffi_args) };
            if out.is_null() {
                ValueWord::none()
            } else {
                let s = unsafe { CStr::from_ptr(out) }.to_string_lossy().to_string();
                ValueWord::from_some(ValueWord::from_string(Arc::new(s)))
            }
        }
        CType::CSlice(elem) | CType::CMutSlice(elem) => {
            let out = unsafe { linked.cif.call::<CSliceAbi>(linked.code_ptr, &ffi_args) };
            let values = decode_slice_elements(out, elem, "native call return")?;
            ValueWord::from_array(Arc::new(values))
        }
        CType::CView(layout_name) => {
            let out = unsafe { linked.cif.call::<usize>(linked.code_ptr, &ffi_args) };
            if out == 0 {
                return Err(format!(
                    "native call returned null pointer for cview<{}>",
                    layout_name
                ));
            }
            let layout = linked.layouts.get(layout_name).ok_or_else(|| {
                format!(
                    "missing native layout '{}' required by cview return",
                    layout_name
                )
            })?;
            ValueWord::from_c_view(out, layout.clone())
        }
        CType::CMut(layout_name) => {
            let out = unsafe { linked.cif.call::<usize>(linked.code_ptr, &ffi_args) };
            if out == 0 {
                return Err(format!(
                    "native call returned null pointer for cmut<{}>",
                    layout_name
                ));
            }
            let layout = linked.layouts.get(layout_name).ok_or_else(|| {
                format!(
                    "missing native layout '{}' required by cmut return",
                    layout_name
                )
            })?;
            ValueWord::from_c_mut(out, layout.clone())
        }
    };

    if !pending_writebacks.is_empty() {
        let Some(stack) = vm_stack else {
            return Err(
                "native call expected VM stack context for mutable argument writeback".to_string(),
            );
        };
        apply_mutable_writebacks(stack, &prepared_args, &pending_writebacks)?;
    }

    drop(owned_callbacks);
    drop(owned_cstrings);

    Ok(result)
}

fn parse_signature(signature: &str) -> Result<CSignature, String> {
    let mut src = signature.trim();
    if let Some(rest) = src.strip_prefix("fn") {
        src = rest.trim_start();
    }

    let open = src.find('(').ok_or_else(|| {
        format!(
            "invalid native signature '{}': expected format `fn(<args>) -> <ret>`",
            signature
        )
    })?;
    let close = find_matching_paren(src, open).ok_or_else(|| {
        format!(
            "invalid native signature '{}': expected closing ')' in argument list",
            signature
        )
    })?;

    let params_src = src[open + 1..close].trim();
    let tail = src[close + 1..].trim();
    let ret_src = tail.strip_prefix("->").ok_or_else(|| {
        format!(
            "invalid native signature '{}': expected `-> <ret>` return segment",
            signature
        )
    })?;
    let ret = CType::parse(ret_src.trim())?;

    let params = if params_src.is_empty() || params_src.eq_ignore_ascii_case("void") {
        Vec::new()
    } else {
        split_top_level(params_src, ',')
            .into_iter()
            .map(|token| CType::parse(token.trim()))
            .collect::<Result<Vec<_>, _>>()?
    };

    if params.iter().any(|ty| matches!(ty, CType::Void)) {
        return Err(
            "invalid native signature: `void` is only valid as return type or empty parameter list"
                .to_string(),
        );
    }

    Ok(CSignature { params, ret })
}

fn find_matching_paren(src: &str, open_idx: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if bytes.get(open_idx).copied()? != b'(' {
        return None;
    }
    let mut depth = 0usize;
    for (idx, ch) in src.char_indices().skip(open_idx) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(src: &str, delimiter: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth_paren = 0usize;
    for (idx, ch) in src.char_indices() {
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            _ => {}
        }
        if ch == delimiter && depth_paren == 0 {
            out.push(src[start..idx].trim().to_string());
            start = idx + ch.len_utf8();
        }
    }
    let tail = src[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn value_to_int_i64(value: &ValueWord, label: &str) -> Result<i64, String> {
    if let Some(v) = value.as_i64() {
        return Ok(v);
    }
    if let Some(v) = value.as_bool() {
        return Ok(if v { 1 } else { 0 });
    }
    Err(format!(
        "native call {} expects an exact integer value, got {}",
        label, value
    ))
}

fn value_to_f64(value: &ValueWord, label: &str) -> Result<f64, String> {
    if let Some(v) = value.as_number_strict() {
        return Ok(v);
    }
    // Allow language `int` literals (i48) for float ABI params without opening
    // lossy conversions for native i64/u64 domains.
    if value.is_i64() {
        return Ok(value.as_i64().unwrap_or(0) as f64);
    }
    Err(format!(
        "native call {} expects a floating-point compatible value, got {}",
        label, value
    ))
}

fn value_to_u64(value: &ValueWord, label: &str) -> Result<u64, String> {
    if let Some(scalar) = value.as_native_scalar() {
        return match scalar {
            NativeScalar::U8(v) => Ok(v as u64),
            NativeScalar::U16(v) => Ok(v as u64),
            NativeScalar::U32(v) => Ok(v as u64),
            NativeScalar::U64(v) => Ok(v),
            NativeScalar::Usize(v) => Ok(v as u64),
            NativeScalar::Ptr(v) => Ok(v as u64),
            NativeScalar::I8(v) if v >= 0 => Ok(v as u64),
            NativeScalar::I16(v) if v >= 0 => Ok(v as u64),
            NativeScalar::I32(v) if v >= 0 => Ok(v as u64),
            NativeScalar::I64(v) if v >= 0 => Ok(v as u64),
            NativeScalar::Isize(v) if v >= 0 => Ok(v as u64),
            _ => Err(format!(
                "native call {} expects a non-negative integer value, got {}",
                label, value
            )),
        };
    }

    if let Some(v) = value.as_i64() {
        if v >= 0 {
            return Ok(v as u64);
        }
    }

    if let Some(v) = value.as_bool() {
        return Ok(if v { 1 } else { 0 });
    }

    Err(format!(
        "native call {} expects a non-negative integer value, got {}",
        label, value
    ))
}

fn value_to_usize(value: &ValueWord, label: &str) -> Result<usize, String> {
    let v = value_to_u64(value, label)?;
    usize::try_from(v).map_err(|_| {
        format!(
            "native call {} value {} does not fit in usize on this platform",
            label, v
        )
    })
}

fn is_shape_callable(value: &ValueWord) -> bool {
    value.as_function_id().is_some()
        || value.as_module_function().is_some()
        // cold-path: as_heap_ref retained — multi-variant callable check
        || matches!(
            value.as_heap_ref(), // cold-path
            Some(
                HeapValue::Closure { .. }
                    | HeapValue::HostClosure(_)
                    | HeapValue::FunctionRef { .. }
            )
        )
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CSliceAbi {
    data: *mut c_void,
    len: usize,
}

#[derive(Debug, Clone)]
enum OwnedSliceBuffer {
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    Isize(Vec<isize>),
    Usize(Vec<usize>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Bool(Vec<u8>),
    Ptr(Vec<*mut c_void>),
    CString {
        _strings: Vec<CString>,
        ptrs: Vec<*const c_char>,
    },
    NullableCString {
        _strings: Vec<CString>,
        ptrs: Vec<*const c_char>,
    },
}

impl OwnedSliceBuffer {
    fn len(&self) -> usize {
        match self {
            Self::I8(v) => v.len(),
            Self::U8(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::U16(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::Isize(v) => v.len(),
            Self::Usize(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::F64(v) => v.len(),
            Self::Bool(v) => v.len(),
            Self::Ptr(v) => v.len(),
            Self::CString { ptrs, .. } => ptrs.len(),
            Self::NullableCString { ptrs, .. } => ptrs.len(),
        }
    }

    fn data_ptr(&self) -> *mut c_void {
        if self.len() == 0 {
            return std::ptr::null_mut();
        }
        match self {
            Self::I8(v) => v.as_ptr() as *mut c_void,
            Self::U8(v) => v.as_ptr() as *mut c_void,
            Self::I16(v) => v.as_ptr() as *mut c_void,
            Self::U16(v) => v.as_ptr() as *mut c_void,
            Self::I32(v) => v.as_ptr() as *mut c_void,
            Self::I64(v) => v.as_ptr() as *mut c_void,
            Self::U32(v) => v.as_ptr() as *mut c_void,
            Self::U64(v) => v.as_ptr() as *mut c_void,
            Self::Isize(v) => v.as_ptr() as *mut c_void,
            Self::Usize(v) => v.as_ptr() as *mut c_void,
            Self::F32(v) => v.as_ptr() as *mut c_void,
            Self::F64(v) => v.as_ptr() as *mut c_void,
            Self::Bool(v) => v.as_ptr() as *mut c_void,
            Self::Ptr(v) => v.as_ptr() as *mut c_void,
            Self::CString { ptrs, .. } => ptrs.as_ptr() as *mut c_void,
            Self::NullableCString { ptrs, .. } => ptrs.as_ptr() as *mut c_void,
        }
    }
}

fn c_slice_ffi_type() -> Type {
    Type::structure(vec![Type::pointer(), Type::usize()])
}

fn c_type_to_ffi_type(ctype: &CType) -> Type {
    match ctype {
        CType::I8 => Type::i8(),
        CType::U8 => Type::u8(),
        CType::I16 => Type::i16(),
        CType::U16 => Type::u16(),
        CType::I32 => Type::i32(),
        CType::I64 => Type::i64(),
        CType::U32 => Type::u32(),
        CType::U64 => Type::u64(),
        CType::Isize => Type::isize(),
        CType::Usize => Type::usize(),
        CType::F32 => Type::f32(),
        CType::F64 => Type::f64(),
        CType::Bool => Type::u8(),
        CType::CSlice(_) | CType::CMutSlice(_) => c_slice_ffi_type(),
        CType::CString
        | CType::NullableCString
        | CType::CView(_)
        | CType::CMut(_)
        | CType::Ptr
        | CType::Callback(_) => Type::pointer(),
        CType::Void => Type::void(),
    }
}

#[derive(Debug, Clone)]
enum PreparedArg {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    Isize(isize),
    Usize(usize),
    F32(f32),
    F64(f64),
    Bool(u8),
    Ptr(*mut c_void),
    SliceDesc {
        desc: CSliceAbi,
        _owned: OwnedSliceBuffer,
    },
}

impl PreparedArg {
    fn as_arg(&self) -> Arg {
        match self {
            Self::I8(v) => Arg::new(v),
            Self::U8(v) => Arg::new(v),
            Self::I16(v) => Arg::new(v),
            Self::U16(v) => Arg::new(v),
            Self::I32(v) => Arg::new(v),
            Self::I64(v) => Arg::new(v),
            Self::U32(v) => Arg::new(v),
            Self::U64(v) => Arg::new(v),
            Self::Isize(v) => Arg::new(v),
            Self::Usize(v) => Arg::new(v),
            Self::F32(v) => Arg::new(v),
            Self::F64(v) => Arg::new(v),
            Self::Bool(v) => Arg::new(v),
            Self::Ptr(v) => Arg::new(v),
            Self::SliceDesc { desc, .. } => Arg::new(desc),
        }
    }
}

struct CallbackUserData {
    signature: CallbackSignature,
    callable: ValueWord,
    raw_invoker: Option<RawCallableInvoker>,
}

#[derive(Debug)]
struct OwnedCallScopedCallback {
    closure: Option<Closure<'static>>,
    userdata_ptr: *mut CallbackUserData,
}

impl OwnedCallScopedCallback {
    fn code_ptr_address(&self) -> usize {
        let Some(closure) = self.closure.as_ref() else {
            return 0;
        };
        (*closure.code_ptr()) as *const () as usize
    }
}

impl Drop for OwnedCallScopedCallback {
    fn drop(&mut self) {
        self.closure.take();
        if !self.userdata_ptr.is_null() {
            unsafe { drop(Box::from_raw(self.userdata_ptr)) };
            self.userdata_ptr = std::ptr::null_mut();
        }
    }
}

unsafe fn decode_callback_arg(
    arg_ptr: *const c_void,
    ctype: &CType,
    idx: usize,
) -> Result<ValueWord, String> {
    if arg_ptr.is_null() {
        return Err(format!("callback arg#{idx} pointer is null"));
    }
    match ctype {
        CType::I8 => Ok(ValueWord::from_native_i8(unsafe {
            *(arg_ptr as *const i8)
        })),
        CType::U8 => Ok(ValueWord::from_native_u8(unsafe {
            *(arg_ptr as *const u8)
        })),
        CType::I16 => Ok(ValueWord::from_native_i16(unsafe {
            *(arg_ptr as *const i16)
        })),
        CType::U16 => Ok(ValueWord::from_native_u16(unsafe {
            *(arg_ptr as *const u16)
        })),
        CType::I32 => Ok(ValueWord::from_native_i32(unsafe {
            *(arg_ptr as *const i32)
        })),
        CType::I64 => Ok(ValueWord::from_native_scalar(NativeScalar::I64(unsafe {
            *(arg_ptr as *const i64)
        }))),
        CType::U32 => Ok(ValueWord::from_native_u32(unsafe {
            *(arg_ptr as *const u32)
        })),
        CType::U64 => Ok(ValueWord::from_native_u64(unsafe {
            *(arg_ptr as *const u64)
        })),
        CType::Isize => Ok(ValueWord::from_native_isize(unsafe {
            *(arg_ptr as *const isize)
        })),
        CType::CSlice(elem) | CType::CMutSlice(elem) => {
            let slice = unsafe { *(arg_ptr as *const CSliceAbi) };
            let values = decode_slice_elements(slice, elem, &format!("callback arg#{idx}"))?;
            Ok(ValueWord::from_array(Arc::new(values)))
        }
        CType::Usize | CType::Ptr | CType::Callback(_) | CType::CView(_) | CType::CMut(_) => {
            let raw = unsafe { *(arg_ptr as *const usize) };
            if matches!(
                ctype,
                CType::Ptr | CType::Callback(_) | CType::CView(_) | CType::CMut(_)
            ) {
                Ok(ValueWord::from_native_ptr(raw))
            } else {
                Ok(ValueWord::from_native_usize(raw))
            }
        }
        CType::F32 => Ok(ValueWord::from_native_f32(unsafe {
            *(arg_ptr as *const f32)
        })),
        CType::F64 => Ok(ValueWord::from_f64(unsafe { *(arg_ptr as *const f64) })),
        CType::Bool => Ok(ValueWord::from_bool(
            unsafe { *(arg_ptr as *const u8) } != 0,
        )),
        CType::CString => {
            let s_ptr = unsafe { *(arg_ptr as *const *const c_char) };
            if s_ptr.is_null() {
                return Err(format!(
                    "callback arg#{idx} returned null for non-null cstring"
                ));
            }
            let s = unsafe { CStr::from_ptr(s_ptr) }
                .to_string_lossy()
                .to_string();
            Ok(ValueWord::from_string(Arc::new(s)))
        }
        CType::NullableCString => {
            let s_ptr = unsafe { *(arg_ptr as *const *const c_char) };
            if s_ptr.is_null() {
                Ok(ValueWord::none())
            } else {
                let s = unsafe { CStr::from_ptr(s_ptr) }
                    .to_string_lossy()
                    .to_string();
                Ok(ValueWord::from_some(ValueWord::from_string(Arc::new(s))))
            }
        }
        CType::Void => Ok(ValueWord::unit()),
    }
}

unsafe fn decode_callback_args(
    args: *const *const c_void,
    signature: &CallbackSignature,
) -> Result<Vec<ValueWord>, String> {
    if args.is_null() && !signature.params.is_empty() {
        return Err("callback args pointer is null".to_string());
    }

    let mut out = Vec::with_capacity(signature.params.len());
    for (idx, ctype) in signature.params.iter().enumerate() {
        let value_ptr = unsafe { *args.add(idx) };
        out.push(unsafe { decode_callback_arg(value_ptr, ctype, idx)? });
    }
    Ok(out)
}

fn invoke_callback_userdata(
    userdata: &CallbackUserData,
    args_ptr: *const *const c_void,
) -> Result<ValueWord, String> {
    let decoded = unsafe { decode_callback_args(args_ptr, &userdata.signature)? };

    if let Some(host_callable) = userdata.callable.as_host_closure() {
        return host_callable.call(&decoded);
    }

    let invoker = userdata.raw_invoker.as_ref().ok_or_else(|| {
        "native callback has no callable invoker — callback argument requires VM call context"
            .to_string()
    })?;
    unsafe { invoker.call(&userdata.callable, &decoded) }
}

fn emit_callback_error(message: &str) {
    eprintln!("native callback error: {message}");
}

unsafe extern "C" fn callback_void(
    _cif: &low::ffi_cif,
    _result: &mut (),
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    if let Err(err) = invoke_callback_userdata(userdata, args) {
        emit_callback_error(&err);
    }
}

unsafe extern "C" fn callback_i8(
    _cif: &low::ffi_cif,
    result: &mut i8,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if v < i8::MIN as i64 || v > i8::MAX as i64 {
                Err("callback return out of range for i8".to_string())
            } else {
                Ok(v as i8)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_u8(
    _cif: &low::ffi_cif,
    result: &mut u8,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if !(0..=u8::MAX as i64).contains(&v) {
                Err("callback return out of range for u8".to_string())
            } else {
                Ok(v as u8)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_i16(
    _cif: &low::ffi_cif,
    result: &mut i16,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if v < i16::MIN as i64 || v > i16::MAX as i64 {
                Err("callback return out of range for i16".to_string())
            } else {
                Ok(v as i16)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_u16(
    _cif: &low::ffi_cif,
    result: &mut u16,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if !(0..=u16::MAX as i64).contains(&v) {
                Err("callback return out of range for u16".to_string())
            } else {
                Ok(v as u16)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_i32(
    _cif: &low::ffi_cif,
    result: &mut i32,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if v < i32::MIN as i64 || v > i32::MAX as i64 {
                Err("callback return out of range for i32".to_string())
            } else {
                Ok(v as i32)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_i64(
    _cif: &low::ffi_cif,
    result: &mut i64,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
    {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_u32(
    _cif: &low::ffi_cif,
    result: &mut u32,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if !(0..=u32::MAX as i64).contains(&v) {
                Err("callback return out of range for u32".to_string())
            } else {
                Ok(v as u32)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_u64(
    _cif: &low::ffi_cif,
    result: &mut u64,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args).and_then(|v| value_to_u64(&v, "callback return"))
    {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_isize(
    _cif: &low::ffi_cif,
    result: &mut isize,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_int_i64(&v, "callback return"))
        .and_then(|v| {
            if v < isize::MIN as i64 || v > isize::MAX as i64 {
                Err("callback return out of range for isize".to_string())
            } else {
                Ok(v as isize)
            }
        }) {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_usize(
    _cif: &low::ffi_cif,
    result: &mut usize,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args)
        .and_then(|v| value_to_usize(&v, "callback return"))
    {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_f32(
    _cif: &low::ffi_cif,
    result: &mut f32,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args).and_then(|v| value_to_f64(&v, "callback return"))
    {
        Ok(v) => *result = v as f32,
        Err(err) => {
            *result = 0.0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_f64(
    _cif: &low::ffi_cif,
    result: &mut f64,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args).and_then(|v| value_to_f64(&v, "callback return"))
    {
        Ok(v) => *result = v,
        Err(err) => {
            *result = 0.0;
            emit_callback_error(&err);
        }
    }
}

unsafe extern "C" fn callback_bool(
    _cif: &low::ffi_cif,
    result: &mut u8,
    args: *const *const c_void,
    userdata: &CallbackUserData,
) {
    match invoke_callback_userdata(userdata, args) {
        Ok(v) => {
            let b = v
                .as_bool()
                .unwrap_or_else(|| value_to_int_i64(&v, "callback return").unwrap_or(0) != 0);
            *result = if b { 1 } else { 0 };
        }
        Err(err) => {
            *result = 0;
            emit_callback_error(&err);
        }
    }
}

fn create_callback_handle(
    signature: &CallbackSignature,
    callable: ValueWord,
    raw_invoker: Option<RawCallableInvoker>,
) -> Result<OwnedCallScopedCallback, String> {
    if matches!(
        signature.ret,
        CType::CString | CType::NullableCString | CType::CSlice(_) | CType::CMutSlice(_)
    ) {
        return Err(
            "callback return types `cstring`/`cstring?`/`cslice<_>`/`cmut_slice<_>` are not supported"
                .to_string(),
        );
    }

    let arg_types: Vec<Type> = signature.params.iter().map(c_type_to_ffi_type).collect();
    let cif = Cif::new(arg_types, c_type_to_ffi_type(&signature.ret));

    let userdata_ptr = Box::into_raw(Box::new(CallbackUserData {
        signature: signature.clone(),
        callable,
        raw_invoker,
    }));
    let userdata_ref: &'static CallbackUserData = unsafe { &*userdata_ptr };

    let closure = match &signature.ret {
        CType::Void => Closure::new(cif, callback_void, userdata_ref),
        CType::I8 => Closure::new(cif, callback_i8, userdata_ref),
        CType::U8 => Closure::new(cif, callback_u8, userdata_ref),
        CType::I16 => Closure::new(cif, callback_i16, userdata_ref),
        CType::U16 => Closure::new(cif, callback_u16, userdata_ref),
        CType::I32 => Closure::new(cif, callback_i32, userdata_ref),
        CType::I64 => Closure::new(cif, callback_i64, userdata_ref),
        CType::U32 => Closure::new(cif, callback_u32, userdata_ref),
        CType::U64 => Closure::new(cif, callback_u64, userdata_ref),
        CType::Isize => Closure::new(cif, callback_isize, userdata_ref),
        CType::Usize | CType::Ptr | CType::Callback(_) | CType::CView(_) | CType::CMut(_) => {
            Closure::new(cif, callback_usize, userdata_ref)
        }
        CType::F32 => Closure::new(cif, callback_f32, userdata_ref),
        CType::F64 => Closure::new(cif, callback_f64, userdata_ref),
        CType::Bool => Closure::new(cif, callback_bool, userdata_ref),
        CType::CString | CType::NullableCString | CType::CSlice(_) | CType::CMutSlice(_) => {
            unreachable!("handled above")
        }
    };

    let owned = OwnedCallScopedCallback {
        closure: Some(closure),
        userdata_ptr,
    };
    if owned.code_ptr_address() == 0 {
        return Err("failed to allocate callback pointer".to_string());
    }
    Ok(owned)
}

fn encode_nullable_cstring_arg(
    value: &ValueWord,
    label: &str,
    owned_cstrings: &mut Vec<CString>,
) -> Result<PreparedArg, String> {
    if value.is_none() {
        return Ok(PreparedArg::Ptr(std::ptr::null_mut()));
    }

    let string_ref = if let Some(s) = value.as_str() {
        Some(s.to_string())
    } else {
        value
            .as_some_inner()
            .and_then(|inner| inner.as_str())
            .map(|s| s.to_string())
    }
    .ok_or_else(|| format!("native call {} expects Option<string> for cstring?", label))?;

    let cstring = CString::new(string_ref).map_err(|_| {
        format!(
            "native call {} contains interior NUL byte and cannot be converted to cstring",
            label
        )
    })?;
    let ptr = cstring.as_ptr() as *mut c_void;
    owned_cstrings.push(cstring);
    Ok(PreparedArg::Ptr(ptr))
}

fn encode_native_view_arg(
    value: &ValueWord,
    layout_name: &str,
    require_mutable: bool,
    layouts: &HashMap<String, Arc<NativeTypeLayout>>,
    label: &str,
) -> Result<PreparedArg, String> {
    if !layouts.contains_key(layout_name) {
        return Err(format!(
            "native call {} references unknown `type C` layout '{}'",
            label, layout_name
        ));
    }

    if let Some(view) = value.as_native_view() {
        if view.layout.name != layout_name {
            return Err(format!(
                "native call {} expects {}<{}>, got {}<{}>",
                label,
                if require_mutable { "cmut" } else { "cview" },
                layout_name,
                if view.mutable { "cmut" } else { "cview" },
                view.layout.name
            ));
        }
        if require_mutable && !view.mutable {
            return Err(format!(
                "native call {} expects mutable cmut<{}>, got read-only cview<{}>",
                label, layout_name, layout_name
            ));
        }
        return Ok(PreparedArg::Ptr(view.ptr as *mut c_void));
    }

    let ptr = value_to_usize(value, label)?;
    Ok(PreparedArg::Ptr(ptr as *mut c_void))
}

fn c_type_label(ctype: &CType) -> String {
    match ctype {
        CType::I8 => "i8".to_string(),
        CType::U8 => "u8".to_string(),
        CType::I16 => "i16".to_string(),
        CType::U16 => "u16".to_string(),
        CType::I32 => "i32".to_string(),
        CType::I64 => "i64".to_string(),
        CType::U32 => "u32".to_string(),
        CType::U64 => "u64".to_string(),
        CType::Isize => "isize".to_string(),
        CType::Usize => "usize".to_string(),
        CType::F32 => "f32".to_string(),
        CType::F64 => "f64".to_string(),
        CType::Bool => "bool".to_string(),
        CType::CString => "cstring".to_string(),
        CType::NullableCString => "cstring?".to_string(),
        CType::CSlice(elem) => format!("cslice<{}>", c_type_label(elem)),
        CType::CMutSlice(elem) => format!("cmut_slice<{}>", c_type_label(elem)),
        CType::CView(name) => format!("cview<{name}>"),
        CType::CMut(name) => format!("cmut<{name}>"),
        CType::Ptr => "ptr".to_string(),
        CType::Callback(sig) => {
            let params = sig
                .params
                .iter()
                .map(c_type_label)
                .collect::<Vec<_>>()
                .join(", ");
            format!("callback(fn({params}) -> {})", c_type_label(&sig.ret))
        }
        CType::Void => "void".to_string(),
    }
}

fn encode_slice_arg(value: &ValueWord, elem: &CType, label: &str) -> Result<PreparedArg, String> {
    let array = value
        .as_any_array()
        .ok_or_else(|| {
            format!(
                "native call {label} expects Vec<{}>, got {}",
                c_type_label(elem),
                value
            )
        })?
        .to_generic();
    let values = array.as_ref();
    let make = |buffer: OwnedSliceBuffer| {
        let desc = CSliceAbi {
            data: buffer.data_ptr(),
            len: buffer.len(),
        };
        PreparedArg::SliceDesc {
            desc,
            _owned: buffer,
        }
    };

    let encoded = match elem {
        CType::I8 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if !(i8::MIN as i64..=i8::MAX as i64).contains(&v) {
                    return Err(format!("native call {label}[{i}] out of range for i8"));
                }
                out.push(v as i8);
            }
            make(OwnedSliceBuffer::I8(out))
        }
        CType::U8 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if !(0..=u8::MAX as i64).contains(&v) {
                    return Err(format!("native call {label}[{i}] out of range for u8"));
                }
                out.push(v as u8);
            }
            make(OwnedSliceBuffer::U8(out))
        }
        CType::I16 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if !(i16::MIN as i64..=i16::MAX as i64).contains(&v) {
                    return Err(format!("native call {label}[{i}] out of range for i16"));
                }
                out.push(v as i16);
            }
            make(OwnedSliceBuffer::I16(out))
        }
        CType::U16 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if !(0..=u16::MAX as i64).contains(&v) {
                    return Err(format!("native call {label}[{i}] out of range for u16"));
                }
                out.push(v as u16);
            }
            make(OwnedSliceBuffer::U16(out))
        }
        CType::I32 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                    return Err(format!("native call {label}[{i}] out of range for i32"));
                }
                out.push(v as i32);
            }
            make(OwnedSliceBuffer::I32(out))
        }
        CType::I64 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                out.push(value_to_int_i64(item, &format!("{label}[{i}]"))?);
            }
            make(OwnedSliceBuffer::I64(out))
        }
        CType::U32 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if !(0..=u32::MAX as i64).contains(&v) {
                    return Err(format!("native call {label}[{i}] out of range for u32"));
                }
                out.push(v as u32);
            }
            make(OwnedSliceBuffer::U32(out))
        }
        CType::U64 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                out.push(value_to_u64(item, &format!("{label}[{i}]"))?);
            }
            make(OwnedSliceBuffer::U64(out))
        }
        CType::Isize => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let v = value_to_int_i64(item, &format!("{label}[{i}]"))?;
                if v < isize::MIN as i64 || v > isize::MAX as i64 {
                    return Err(format!("native call {label}[{i}] out of range for isize"));
                }
                out.push(v as isize);
            }
            make(OwnedSliceBuffer::Isize(out))
        }
        CType::Usize => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                out.push(value_to_usize(item, &format!("{label}[{i}]"))?);
            }
            make(OwnedSliceBuffer::Usize(out))
        }
        CType::F32 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                out.push(value_to_f64(item, &format!("{label}[{i}]"))? as f32);
            }
            make(OwnedSliceBuffer::F32(out))
        }
        CType::F64 => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                out.push(value_to_f64(item, &format!("{label}[{i}]"))?);
            }
            make(OwnedSliceBuffer::F64(out))
        }
        CType::Bool => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let b = if let Some(v) = item.as_bool() {
                    v
                } else {
                    value_to_int_i64(item, &format!("{label}[{i}]"))? != 0
                };
                out.push(if b { 1 } else { 0 });
            }
            make(OwnedSliceBuffer::Bool(out))
        }
        CType::Ptr => {
            let mut out = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                out.push(value_to_usize(item, &format!("{label}[{i}]"))? as *mut c_void);
            }
            make(OwnedSliceBuffer::Ptr(out))
        }
        CType::CString => {
            let mut strings = Vec::with_capacity(values.len());
            let mut ptrs = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                let s = item.as_str().ok_or_else(|| {
                    format!("native call {label}[{i}] expects string for cstring element")
                })?;
                let cstring = CString::new(s).map_err(|_| {
                    format!(
                        "native call {label}[{i}] contains interior NUL byte and cannot be converted to cstring"
                    )
                })?;
                ptrs.push(cstring.as_ptr());
                strings.push(cstring);
            }
            make(OwnedSliceBuffer::CString {
                _strings: strings,
                ptrs,
            })
        }
        CType::NullableCString => {
            let mut strings = Vec::with_capacity(values.len());
            let mut ptrs = Vec::with_capacity(values.len());
            for (i, item) in values.iter().enumerate() {
                if item.is_none() {
                    ptrs.push(std::ptr::null());
                    continue;
                }
                let s = if let Some(s) = item.as_str() {
                    Some(s.to_string())
                } else {
                    item.as_some_inner()
                        .and_then(|inner| inner.as_str())
                        .map(|s| s.to_string())
                }
                .ok_or_else(|| {
                    format!("native call {label}[{i}] expects Option<string> for cstring? element")
                })?;
                let cstring = CString::new(s).map_err(|_| {
                    format!(
                        "native call {label}[{i}] contains interior NUL byte and cannot be converted to cstring?"
                    )
                })?;
                ptrs.push(cstring.as_ptr());
                strings.push(cstring);
            }
            make(OwnedSliceBuffer::NullableCString {
                _strings: strings,
                ptrs,
            })
        }
        other => {
            return Err(format!(
                "native call {label} unsupported slice element type '{}'",
                c_type_label(other)
            ));
        }
    };

    Ok(encoded)
}

fn decode_slice_elements(
    slice: CSliceAbi,
    elem: &CType,
    label: &str,
) -> Result<Vec<ValueWord>, String> {
    if slice.len == 0 {
        return Ok(Vec::new());
    }
    if slice.data.is_null() {
        return Err(format!(
            "{label} returned null data pointer for non-empty {}",
            c_type_label(elem)
        ));
    }

    let mut out = Vec::with_capacity(slice.len);
    match elem {
        CType::I8 => {
            let ptr = slice.data as *const i8;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_i8(unsafe { *ptr.add(i) }));
            }
        }
        CType::U8 => {
            let ptr = slice.data as *const u8;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_u8(unsafe { *ptr.add(i) }));
            }
        }
        CType::I16 => {
            let ptr = slice.data as *const i16;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_i16(unsafe { *ptr.add(i) }));
            }
        }
        CType::U16 => {
            let ptr = slice.data as *const u16;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_u16(unsafe { *ptr.add(i) }));
            }
        }
        CType::I32 => {
            let ptr = slice.data as *const i32;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_i32(unsafe { *ptr.add(i) }));
            }
        }
        CType::I64 => {
            let ptr = slice.data as *const i64;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_scalar(NativeScalar::I64(unsafe {
                    *ptr.add(i)
                })));
            }
        }
        CType::U32 => {
            let ptr = slice.data as *const u32;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_u32(unsafe { *ptr.add(i) }));
            }
        }
        CType::U64 => {
            let ptr = slice.data as *const u64;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_u64(unsafe { *ptr.add(i) }));
            }
        }
        CType::Isize => {
            let ptr = slice.data as *const isize;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_isize(unsafe { *ptr.add(i) }));
            }
        }
        CType::Usize => {
            let ptr = slice.data as *const usize;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_usize(unsafe { *ptr.add(i) }));
            }
        }
        CType::F32 => {
            let ptr = slice.data as *const f32;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_f32(unsafe { *ptr.add(i) }));
            }
        }
        CType::F64 => {
            let ptr = slice.data as *const f64;
            for i in 0..slice.len {
                out.push(ValueWord::from_f64(unsafe { *ptr.add(i) }));
            }
        }
        CType::Bool => {
            let ptr = slice.data as *const u8;
            for i in 0..slice.len {
                out.push(ValueWord::from_bool(unsafe { *ptr.add(i) } != 0));
            }
        }
        CType::Ptr => {
            let ptr = slice.data as *const *mut c_void;
            for i in 0..slice.len {
                out.push(ValueWord::from_native_ptr(unsafe { *ptr.add(i) } as usize));
            }
        }
        CType::CString => {
            let ptr = slice.data as *const *const c_char;
            for i in 0..slice.len {
                let item_ptr = unsafe { *ptr.add(i) };
                if item_ptr.is_null() {
                    return Err(format!(
                        "{label} returned null cstring at index {i}; use cstring? for nullable values"
                    ));
                }
                let s = unsafe { CStr::from_ptr(item_ptr) }
                    .to_string_lossy()
                    .to_string();
                out.push(ValueWord::from_string(Arc::new(s)));
            }
        }
        CType::NullableCString => {
            let ptr = slice.data as *const *const c_char;
            for i in 0..slice.len {
                let item_ptr = unsafe { *ptr.add(i) };
                if item_ptr.is_null() {
                    out.push(ValueWord::none());
                } else {
                    let s = unsafe { CStr::from_ptr(item_ptr) }
                        .to_string_lossy()
                        .to_string();
                    out.push(ValueWord::from_some(ValueWord::from_string(Arc::new(s))));
                }
            }
        }
        other => {
            return Err(format!(
                "{label} does not support slice element type '{}'",
                c_type_label(other)
            ));
        }
    }
    Ok(out)
}

fn encode_arg(
    value: &ValueWord,
    ctype: &CType,
    idx: usize,
    owned_cstrings: &mut Vec<CString>,
    owned_callbacks: &mut Vec<OwnedCallScopedCallback>,
    layouts: &HashMap<String, Arc<NativeTypeLayout>>,
    raw_invoker: Option<RawCallableInvoker>,
) -> Result<PreparedArg, String> {
    let label = format!("arg#{idx}");
    match ctype {
        CType::I8 => {
            let v = value_to_int_i64(value, &label)?;
            if !(i8::MIN as i64..=i8::MAX as i64).contains(&v) {
                return Err(format!("native call {} out of range for i8", label));
            }
            Ok(PreparedArg::I8(v as i8))
        }
        CType::U8 => {
            let v = value_to_int_i64(value, &label)?;
            if !(0..=u8::MAX as i64).contains(&v) {
                return Err(format!("native call {} out of range for u8", label));
            }
            Ok(PreparedArg::U8(v as u8))
        }
        CType::I16 => {
            let v = value_to_int_i64(value, &label)?;
            if !(i16::MIN as i64..=i16::MAX as i64).contains(&v) {
                return Err(format!("native call {} out of range for i16", label));
            }
            Ok(PreparedArg::I16(v as i16))
        }
        CType::U16 => {
            let v = value_to_int_i64(value, &label)?;
            if !(0..=u16::MAX as i64).contains(&v) {
                return Err(format!("native call {} out of range for u16", label));
            }
            Ok(PreparedArg::U16(v as u16))
        }
        CType::I32 => {
            let v = value_to_int_i64(value, &label)?;
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                return Err(format!("native call {} out of range for i32", label));
            }
            Ok(PreparedArg::I32(v as i32))
        }
        CType::I64 => Ok(PreparedArg::I64(value_to_int_i64(value, &label)?)),
        CType::U32 => {
            let v = value_to_int_i64(value, &label)?;
            if !(0..=u32::MAX as i64).contains(&v) {
                return Err(format!("native call {} out of range for u32", label));
            }
            Ok(PreparedArg::U32(v as u32))
        }
        CType::U64 => Ok(PreparedArg::U64(value_to_u64(value, &label)?)),
        CType::Isize => Ok(PreparedArg::Isize(value_to_int_i64(value, &label)? as isize)),
        CType::Usize => Ok(PreparedArg::Usize(value_to_usize(value, &label)?)),
        CType::F32 => Ok(PreparedArg::F32(value_to_f64(value, &label)? as f32)),
        CType::F64 => Ok(PreparedArg::F64(value_to_f64(value, &label)?)),
        CType::Bool => {
            let b = if let Some(v) = value.as_bool() {
                v
            } else {
                value_to_int_i64(value, &label)? != 0
            };
            Ok(PreparedArg::Bool(if b { 1 } else { 0 }))
        }
        CType::CString => {
            let s = value
                .as_str()
                .ok_or_else(|| format!("native call {} expects a string for cstring", label))?;
            let cstring = CString::new(s).map_err(|_| {
                format!(
                    "native call {} contains interior NUL byte and cannot be converted to cstring",
                    label
                )
            })?;
            let ptr = cstring.as_ptr() as *mut c_void;
            owned_cstrings.push(cstring);
            Ok(PreparedArg::Ptr(ptr))
        }
        CType::NullableCString => encode_nullable_cstring_arg(value, &label, owned_cstrings),
        CType::CView(layout_name) => {
            encode_native_view_arg(value, layout_name, false, layouts, &label)
        }
        CType::CMut(layout_name) => {
            encode_native_view_arg(value, layout_name, true, layouts, &label)
        }
        CType::CSlice(elem) | CType::CMutSlice(elem) => encode_slice_arg(value, elem, &label),
        CType::Ptr => {
            let ptr = value_to_usize(value, &label)?;
            Ok(PreparedArg::Ptr(ptr as *mut c_void))
        }
        CType::Callback(signature) => {
            if is_shape_callable(value) {
                let handle = create_callback_handle(signature, value.clone(), raw_invoker)?;
                let ptr = handle.code_ptr_address();
                if ptr > i64::MAX as usize {
                    return Err("callback pointer exceeds Shape int range (i64::MAX)".to_string());
                }
                owned_callbacks.push(handle);
                Ok(PreparedArg::Ptr(ptr as *mut c_void))
            } else {
                let ptr = value_to_usize(value, &label)?;
                Ok(PreparedArg::Ptr(ptr as *mut c_void))
            }
        }
        CType::Void => Err("void cannot be used as a function parameter type".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::heap_value::NativeTypeLayout;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn sample_layout(name: &str) -> Arc<NativeTypeLayout> {
        Arc::new(NativeTypeLayout {
            name: name.to_string(),
            abi: "C".to_string(),
            size: 16,
            align: 8,
            fields: Vec::new(),
        })
    }

    #[test]
    fn parse_signature_accepts_basic_form() {
        let sig = parse_signature("fn(i32, f64) -> bool").expect("valid signature");
        assert_eq!(sig.params, vec![CType::I32, CType::F64]);
        assert_eq!(sig.ret, CType::Bool);
    }

    #[test]
    fn parse_signature_supports_nullable_cstring() {
        let sig = parse_signature("fn(cstring?) -> cstring?").expect("valid nullable string sig");
        assert_eq!(sig.params, vec![CType::NullableCString]);
        assert_eq!(sig.ret, CType::NullableCString);
    }

    #[test]
    fn parse_signature_supports_callback_type() {
        let sig = parse_signature("fn(i32, callback(fn(ptr, ptr) -> i32)) -> i32")
            .expect("callback signature should parse");
        assert!(matches!(sig.params[1], CType::Callback(_)));
    }

    #[test]
    fn parse_signature_supports_cview_and_cmut() {
        let sig = parse_signature("fn(cview<QuoteC>, cmut<QuoteC>) -> cview<QuoteC>")
            .expect("native views should parse");
        assert!(matches!(sig.params[0], CType::CView(ref name) if name == "QuoteC"));
        assert!(matches!(sig.params[1], CType::CMut(ref name) if name == "QuoteC"));
        assert!(matches!(sig.ret, CType::CView(ref name) if name == "QuoteC"));
    }

    #[test]
    fn parse_signature_supports_cslice_and_cmut_slice() {
        let sig = parse_signature("fn(cslice<u8>, cmut_slice<cstring?>) -> cslice<u32>")
            .expect("native slices should parse");
        assert!(matches!(sig.params[0], CType::CSlice(_)));
        assert!(matches!(sig.params[1], CType::CMutSlice(_)));
        assert!(matches!(sig.ret, CType::CSlice(_)));
    }

    #[test]
    fn parse_signature_rejects_void_param() {
        let err = parse_signature("fn(void, i32) -> i32").expect_err("void param must fail");
        assert!(err.contains("void"));
    }

    #[test]
    fn validate_layout_references_rejects_unknown_type_c_layout() {
        let sig = parse_signature("fn(cview<QuoteC>) -> void").expect("signature should parse");
        let err = validate_layout_references(&sig, &HashMap::new())
            .expect_err("unknown layout should fail validation");
        assert!(err.contains("unknown `type C` layout 'QuoteC'"));
    }

    #[test]
    fn encode_arg_preserves_u8_width_and_checks_range() {
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();
        let layouts = HashMap::new();
        let arg = encode_arg(
            &ValueWord::from_native_u8(255),
            &CType::U8,
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("u8 value should encode");
        assert!(matches!(arg, PreparedArg::U8(255)));

        let err = encode_arg(
            &ValueWord::from_i64(256),
            &CType::U8,
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect_err("overflow u8 should fail");
        assert!(err.contains("out of range for u8"));
    }

    #[test]
    fn encode_arg_i64_accepts_exact_native_i64() {
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();
        let layouts = HashMap::new();
        let arg = encode_arg(
            &ValueWord::from_native_scalar(NativeScalar::I64(i64::MAX)),
            &CType::I64,
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("i64 value should encode");
        assert!(matches!(arg, PreparedArg::I64(v) if v == i64::MAX));
    }

    #[test]
    fn encode_arg_f64_rejects_native_i64_without_cast() {
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();
        let layouts = HashMap::new();
        let err = encode_arg(
            &ValueWord::from_native_scalar(NativeScalar::I64(123)),
            &CType::F64,
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect_err("native i64 should not auto-coerce to f64");
        assert!(err.contains("floating-point compatible value"));
    }

    #[test]
    fn encode_arg_nullable_cstring_maps_none_and_some_string() {
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();
        let layouts = HashMap::new();

        let null_arg = encode_arg(
            &ValueWord::none(),
            &CType::NullableCString,
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("none should map to null cstring pointer");
        assert!(matches!(null_arg, PreparedArg::Ptr(ptr) if ptr.is_null()));

        let some = ValueWord::from_some(ValueWord::from_string(Arc::new("hello".to_string())));
        let some_arg = encode_arg(
            &some,
            &CType::NullableCString,
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("Option<string> should map to non-null cstring pointer");
        assert!(matches!(some_arg, PreparedArg::Ptr(ptr) if !ptr.is_null()));
    }

    #[test]
    fn encode_arg_cslice_u8_from_vec_byte() {
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();
        let layouts = HashMap::new();
        let values = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_native_u8(1),
            ValueWord::from_native_u8(2),
            ValueWord::from_native_u8(3),
        ]));
        let arg = encode_arg(
            &values,
            &CType::CSlice(Box::new(CType::U8)),
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("Vec<byte> should encode into cslice<u8>");
        assert!(matches!(
            arg,
            PreparedArg::SliceDesc { desc, .. } if desc.len == 3 && !desc.data.is_null()
        ));
    }

    #[test]
    fn encode_arg_cslice_u8_rejects_out_of_range_values() {
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();
        let layouts = HashMap::new();
        let values = ValueWord::from_array(Arc::new(vec![ValueWord::from_i64(256)]));
        let err = encode_arg(
            &values,
            &CType::CSlice(Box::new(CType::U8)),
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect_err("out-of-range u8 slice value should fail");
        assert!(err.contains("out of range for u8"));
    }

    #[test]
    fn resolve_arg_value_dereferences_ref_from_stack() {
        let stack = vec![ValueWord::from_native_u8(42)];
        let value = ValueWord::from_ref(0);
        let (resolved, slot) =
            resolve_arg_value_for_native_call(&value, 0, Some(&stack)).expect("ref should resolve");
        assert_eq!(slot, Some(0));
        assert_eq!(
            resolved
                .as_native_scalar()
                .and_then(|scalar| scalar.as_u64())
                .expect("resolved value should be native u8"),
            42_u64
        );
    }

    #[test]
    fn resolve_arg_value_requires_stack_for_ref() {
        let value = ValueWord::from_ref(0);
        let err = resolve_arg_value_for_native_call(&value, 0, None)
            .expect_err("ref argument without stack context must fail");
        assert!(err.contains("no VM stack context"));
    }

    #[test]
    fn cmut_slice_requires_reference_source_for_writeback() {
        let err = build_mutable_writeback_plan(&CType::CMutSlice(Box::new(CType::U8)), 0, None)
            .expect_err("cmut_slice without ref source should fail");
        assert!(err.contains("requires a mutable reference argument"));
    }

    #[test]
    fn apply_mutable_writebacks_updates_u8_slice_target() {
        let buffer = OwnedSliceBuffer::U8(vec![9, 8, 7]);
        let desc = CSliceAbi {
            data: buffer.data_ptr(),
            len: buffer.len(),
        };
        let prepared_args = vec![PreparedArg::SliceDesc {
            desc,
            _owned: buffer,
        }];
        let plans = vec![MutableArgWritebackPlan::Slice {
            arg_index: 0,
            target_slot: 1,
            elem_type: CType::U8,
        }];

        let mut stack = vec![
            ValueWord::none(),
            ValueWord::from_array(Arc::new(vec![ValueWord::from_native_u8(1)])),
        ];
        apply_mutable_writebacks(&mut stack, &prepared_args, &plans)
            .expect("writeback should succeed");

        let out = stack[1]
            .as_any_array()
            .expect("stack target should be array")
            .to_generic();
        assert_eq!(out.len(), 3);
        assert_eq!(
            out[0]
                .as_native_scalar()
                .and_then(|scalar| scalar.as_u64())
                .expect("array[0] should be native u8"),
            9_u64
        );
        assert_eq!(
            out[1]
                .as_native_scalar()
                .and_then(|scalar| scalar.as_u64())
                .expect("array[1] should be native u8"),
            8_u64
        );
    }

    #[test]
    fn apply_mutable_writebacks_supports_nullable_cstring_slice() {
        let hello = CString::new("hello").expect("cstring");
        let world = CString::new("world").expect("cstring");
        let ptrs: Vec<*const c_char> = vec![hello.as_ptr(), std::ptr::null(), world.as_ptr()];
        let buffer = OwnedSliceBuffer::NullableCString {
            _strings: vec![hello, world],
            ptrs,
        };
        let desc = CSliceAbi {
            data: buffer.data_ptr(),
            len: buffer.len(),
        };
        let prepared_args = vec![PreparedArg::SliceDesc {
            desc,
            _owned: buffer,
        }];
        let plans = vec![MutableArgWritebackPlan::Slice {
            arg_index: 0,
            target_slot: 0,
            elem_type: CType::NullableCString,
        }];

        let mut stack = vec![ValueWord::from_array(Arc::new(vec![]))];
        apply_mutable_writebacks(&mut stack, &prepared_args, &plans)
            .expect("nullable cstring writeback should succeed");

        let out = stack[0]
            .as_any_array()
            .expect("stack target should be array")
            .to_generic();
        assert_eq!(out.len(), 3);
        assert_eq!(
            out[0]
                .as_some_inner()
                .and_then(|inner| inner.as_str())
                .expect("first value should be Some(string)"),
            "hello"
        );
        assert!(out[1].is_none());
        assert_eq!(
            out[2]
                .as_some_inner()
                .and_then(|inner| inner.as_str())
                .expect("third value should be Some(string)"),
            "world"
        );
    }

    #[test]
    fn decode_slice_elements_handles_nullable_cstring() {
        let hello = CString::new("hello").expect("cstring");
        let world = CString::new("world").expect("cstring");
        let ptrs: Vec<*const c_char> = vec![hello.as_ptr(), std::ptr::null(), world.as_ptr()];
        let slice = CSliceAbi {
            data: ptrs.as_ptr() as *mut c_void,
            len: ptrs.len(),
        };
        let decoded = decode_slice_elements(slice, &CType::NullableCString, "native call return")
            .expect("nullable cstring slice should decode");
        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded[0]
                .as_some_inner()
                .and_then(|inner| inner.as_str())
                .expect("first should be Some(string)"),
            "hello"
        );
        assert!(decoded[1].is_none());
        assert_eq!(
            decoded[2]
                .as_some_inner()
                .and_then(|inner| inner.as_str())
                .expect("third should be Some(string)"),
            "world"
        );
    }

    #[test]
    fn encode_arg_native_view_enforces_layout_and_mutability() {
        let layout = sample_layout("QuoteC");
        let mut layouts = HashMap::new();
        layouts.insert(layout.name.clone(), layout.clone());
        let mut owned_cstrings = Vec::new();
        let mut owned_callbacks = Vec::new();

        let view = ValueWord::from_c_view(0x1000, layout.clone());
        let cview_arg = encode_arg(
            &view,
            &CType::CView("QuoteC".to_string()),
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("matching cview should encode");
        assert!(matches!(cview_arg, PreparedArg::Ptr(ptr) if ptr as usize == 0x1000));

        let err = encode_arg(
            &view,
            &CType::CMut("QuoteC".to_string()),
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect_err("cview should not satisfy cmut requirement");
        assert!(err.contains("expects mutable cmut<QuoteC>"));

        let err = encode_arg(
            &view,
            &CType::CView("OtherC".to_string()),
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect_err("layout mismatch should fail");
        assert!(err.contains("unknown `type C` layout 'OtherC'"));

        let ptr_value = ValueWord::from_native_ptr(0x2000);
        let ptr_arg = encode_arg(
            &ptr_value,
            &CType::CView("QuoteC".to_string()),
            0,
            &mut owned_cstrings,
            &mut owned_callbacks,
            &layouts,
            None,
        )
        .expect("raw pointer should be accepted for cview");
        assert!(matches!(ptr_arg, PreparedArg::Ptr(ptr) if ptr as usize == 0x2000));
    }

    #[test]
    fn decode_callback_nullable_cstring_maps_null_to_option_none() {
        let null_ptr: *const c_char = std::ptr::null();
        let arg_ptr = &null_ptr as *const *const c_char as *const c_void;
        let decoded = unsafe { decode_callback_arg(arg_ptr, &CType::NullableCString, 0) }
            .expect("null nullable cstring should decode");
        assert!(decoded.is_none());
    }

    #[test]
    fn decode_callback_nullable_cstring_maps_non_null_to_option_string() {
        let cstring = CString::new("hello").expect("cstring");
        let s_ptr: *const c_char = cstring.as_ptr();
        let arg_ptr = &s_ptr as *const *const c_char as *const c_void;
        let decoded = unsafe { decode_callback_arg(arg_ptr, &CType::NullableCString, 0) }
            .expect("non-null nullable cstring should decode");
        let inner = decoded
            .as_some_inner()
            .expect("should decode to Option::Some");
        assert_eq!(inner.as_str(), Some("hello"));
    }
}
