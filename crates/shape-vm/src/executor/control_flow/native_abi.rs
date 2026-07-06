//! Native C ABI linking and invocation for `extern C` foreign functions.
//!
//! WF-2A stage 2 rebuild (ADR-006 §2.7.4 / §2.7.5): the foreign-call runtime
//! for `extern C`, rebuilt on the v2 zero-tag runtime. The marshal carrier is
//! [`KindedSlot`] in / out — the SAME carrier the dynamic-language (Python/TS)
//! path and the JIT foreign bridge use. There is NO parallel per-C-call value
//! carrier: the libffi `PreparedArg` / `Arg` encodings are the raw C ABI on
//! the FFI side of the §2.7.5 boundary and never escape
//! [`invoke_linked_function`].
//!
//! Scope of this stage (per the WF-2A stages-1-2 task): scalar marshalling —
//! the integer widths, `bool`, `f32`/`f64`, raw `ptr` (carried as the NON-heap
//! [`NativeKind::UIntSize`] — a foreign address carried as
//! `NativeKind::Ptr(HeapKind::…)` would drive `KindedSlot::Drop` to
//! `Arc::decrement_strong_count` on a C address = UB), `cstring` (Shape string
//! → owned `CString` arg / C-returned string copied into a Shape string), and
//! `void`. DEFERRED to later sub-waves (§4.4 / §3.3 surface-and-stop) and
//! surfaced as a structured `Err(...)` rather than silently defaulted:
//! `cstring?`, `cslice`/`cmut_slice`, `cview`/`cmut` struct by-value, and
//! `callback(...)`.
//!
//! The C-signature parser, `CSignature`, and `c_type_to_ffi_type` are
//! transplanted (ValueWord-free) from the pre-rebuild blob.

use crate::bytecode::{NativeAbiSpec, NativeStructLayoutEntry};
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Library;
use shape_value::{KindedSlot, NativeKind, ValueSlot};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Arc;

// ── C type classification (ValueWord-free) ────────────────────────────────────

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
fn parse_generic_type_arg<'a>(compact: &'a str, type_name: &str) -> Result<&'a str, String> {
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
            return Ok(Self::CSlice(Box::new(elem)));
        }

        if normalized.starts_with("cmut_slice<") && normalized.ends_with('>') {
            let inner = parse_generic_type_arg(&compact, "cmut_slice")?;
            let elem = CType::parse(inner)?;
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

    /// Human label for diagnostics.
    fn label(&self) -> String {
        match self {
            CType::I8 => "i8".into(),
            CType::U8 => "u8".into(),
            CType::I16 => "i16".into(),
            CType::U16 => "u16".into(),
            CType::I32 => "i32".into(),
            CType::I64 => "i64".into(),
            CType::U32 => "u32".into(),
            CType::U64 => "u64".into(),
            CType::Isize => "isize".into(),
            CType::Usize => "usize".into(),
            CType::F32 => "f32".into(),
            CType::F64 => "f64".into(),
            CType::Bool => "bool".into(),
            CType::CString => "cstring".into(),
            CType::NullableCString => "cstring?".into(),
            CType::CSlice(e) => format!("cslice<{}>", e.label()),
            CType::CMutSlice(e) => format!("cmut_slice<{}>", e.label()),
            CType::CView(n) => format!("cview<{}>", n),
            CType::CMut(n) => format!("cmut<{}>", n),
            CType::Ptr => "ptr".into(),
            CType::Callback(_) => "callback(...)".into(),
            CType::Void => "void".into(),
        }
    }
}

#[derive(Debug, Clone)]
struct CSignature {
    params: Vec<CType>,
    ret: CType,
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

// ── The ONE C-type ↔ runtime-kind table (§4.4 / §4.6.2). ──────────────────────
//
// Post-proof per §2.7.5: the kind is the C signature's static classification,
// never fabricated from bits, never Bool-defaulted.

/// C return/value type → the `NativeKind` that labels the resulting slot.
///
/// The load-bearing anti-UB rule: every raw C pointer (`ptr`, `cview`, `cmut`,
/// `callback`, and any pointer-shaped return) is `NativeKind::UIntSize` — a
/// NON-heap scalar. If a foreign pointer were carried as
/// `NativeKind::Ptr(HeapKind::…)`, `KindedSlot::Drop` would call
/// `Arc::decrement_strong_count` on a foreign address = UB.
///
/// Returns `None` for the C types this stage does not yet produce
/// (`cstring?`, `cslice`, `cmut_slice`, `void`); `cstring` maps to the
/// canonical `NativeKind::String` carrier.
fn native_kind_for_ctype(ct: &CType) -> Option<NativeKind> {
    Some(match ct {
        CType::I8 => NativeKind::Int8,
        CType::U8 => NativeKind::UInt8,
        CType::I16 => NativeKind::Int16,
        CType::U16 => NativeKind::UInt16,
        CType::I32 => NativeKind::Int32,
        CType::I64 => NativeKind::Int64,
        CType::U32 => NativeKind::UInt32,
        CType::U64 => NativeKind::UInt64,
        CType::Isize => NativeKind::IntSize,
        CType::Usize => NativeKind::UIntSize,
        CType::F32 => NativeKind::Float32,
        CType::F64 => NativeKind::Float64,
        CType::Bool => NativeKind::Bool,
        CType::CString => NativeKind::String,
        // Every raw C pointer → non-heap UIntSize (anti-UB invariant).
        CType::Ptr | CType::CView(_) | CType::CMut(_) | CType::Callback(_) => NativeKind::UIntSize,
        // Deferred to later sub-waves.
        CType::NullableCString | CType::CSlice(_) | CType::CMutSlice(_) | CType::Void => {
            return None;
        }
    })
}

/// Read the numeric value of a scalar `KindedSlot` as an `i128`, dispatching on
/// its (post-proof, parallel-track) `NativeKind`. Used to marshal a Shape
/// integer value into any C integer/pointer argument slot. The kind is the
/// proof (§2.7.5) — never fabricated from bits, never Bool-defaulted.
fn slot_read_i128(slot: &KindedSlot, label: &str) -> Result<i128, String> {
    let raw = slot.raw();
    Ok(match slot.kind() {
        NativeKind::Int8 => raw as u8 as i8 as i128,
        NativeKind::UInt8 => raw as u8 as i128,
        NativeKind::Int16 => raw as u16 as i16 as i128,
        NativeKind::UInt16 => raw as u16 as i128,
        NativeKind::Int32 => raw as u32 as i32 as i128,
        NativeKind::UInt32 => raw as u32 as i128,
        NativeKind::Int64 | NativeKind::IntSize => raw as i64 as i128,
        NativeKind::UInt64 | NativeKind::UIntSize => raw as i128,
        NativeKind::Bool => {
            if raw != 0 {
                1
            } else {
                0
            }
        }
        other => {
            return Err(format!(
                "native call {}: expected an integer-kinded value, got {:?}",
                label, other
            ));
        }
    })
}

/// Read the numeric value of a scalar `KindedSlot` as `f64`.
fn slot_read_f64(slot: &KindedSlot, label: &str) -> Result<f64, String> {
    match slot.kind() {
        NativeKind::Float64 => Ok(f64::from_bits(slot.raw())),
        NativeKind::Float32 => Ok(f32::from_bits(slot.raw() as u32) as f64),
        // A Shape `int` literal may be passed where the C ABI wants a float.
        NativeKind::Int64 | NativeKind::IntSize => Ok(slot.raw() as i64 as f64),
        other => Err(format!(
            "native call {}: expected a float-compatible value, got {:?}",
            label, other
        )),
    }
}

// ── libffi argument encoding (raw C ABI, FFI side of §2.7.5) ──────────────────

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
        }
    }
}

/// arg-in: one `KindedSlot` → one libffi `PreparedArg`, per the target C type.
/// ONE match over `CType`. `cstring` args go through the invoke loop directly
/// (they need a call-scoped owned `CString`); deferred types surface `Err`.
fn kinded_slot_to_prepared_arg(
    slot: &KindedSlot,
    ct: &CType,
    idx: usize,
) -> Result<PreparedArg, String> {
    let label = format!("arg#{idx} ({})", ct.label());
    let out = match ct {
        CType::I8 => PreparedArg::I8(slot_read_i128(slot, &label)? as i8),
        CType::U8 => PreparedArg::U8(slot_read_i128(slot, &label)? as u8),
        CType::I16 => PreparedArg::I16(slot_read_i128(slot, &label)? as i16),
        CType::U16 => PreparedArg::U16(slot_read_i128(slot, &label)? as u16),
        CType::I32 => PreparedArg::I32(slot_read_i128(slot, &label)? as i32),
        CType::U32 => PreparedArg::U32(slot_read_i128(slot, &label)? as u32),
        CType::I64 => PreparedArg::I64(slot_read_i128(slot, &label)? as i64),
        CType::U64 => PreparedArg::U64(slot_read_i128(slot, &label)? as u64),
        CType::Isize => PreparedArg::Isize(slot_read_i128(slot, &label)? as isize),
        CType::Usize => PreparedArg::Usize(slot_read_i128(slot, &label)? as usize),
        CType::Ptr => PreparedArg::Ptr(slot_read_i128(slot, &label)? as usize as *mut c_void),
        CType::F32 => PreparedArg::F32(slot_read_f64(slot, &label)? as f32),
        CType::F64 => PreparedArg::F64(slot_read_f64(slot, &label)?),
        CType::Bool => {
            let v = slot
                .as_bool()
                .ok_or_else(|| format!("native call {}: expected a bool value", label))?;
            PreparedArg::Bool(if v { 1 } else { 0 })
        }
        // `cstring` is handled in the invoke loop (needs a call-scoped
        // owned CString). Everything below is deferred (§3.3 surface-and-stop).
        CType::CString
        | CType::NullableCString
        | CType::CSlice(_)
        | CType::CMutSlice(_)
        | CType::CView(_)
        | CType::CMut(_)
        | CType::Callback(_)
        | CType::Void => {
            return Err(format!(
                "native call {}: marshalling of C type `{}` is not implemented in this build \
                 (WF-2A stage 2 covers scalar/ptr/cstring; slice/struct/callback/cstring? are a \
                 later sub-wave)",
                label,
                ct.label()
            ));
        }
    };
    Ok(out)
}

/// Encode a `cstring` argument: copy the Shape string into an owned `CString`
/// (kept alive by `owned_cstrings` for the call duration) and pass a pointer to
/// its NUL-terminated bytes. An embedded NUL is a structured error (§4.6.2 —
/// never a silent truncation).
fn encode_cstring_arg(
    slot: &KindedSlot,
    idx: usize,
    owned_cstrings: &mut Vec<CString>,
) -> Result<PreparedArg, String> {
    let label = format!("arg#{idx} (cstring)");
    let s = slot.as_str().ok_or_else(|| {
        format!(
            "native call {}: expected a Shape string for a `cstring` argument, got kind {:?}",
            label,
            slot.kind()
        )
    })?;
    let owned = CString::new(s).map_err(|e| {
        format!(
            "native call {}: cstring argument contains a NUL byte at position {}",
            label,
            e.nul_position()
        )
    })?;
    let ptr = owned.as_ptr() as *mut c_void;
    // The heap buffer behind `owned` is stable across the Vec push (moving the
    // CString handle does not move its heap allocation), so `ptr` stays valid
    // for the call duration.
    owned_cstrings.push(owned);
    Ok(PreparedArg::Ptr(ptr))
}

// ── out-param cell primitives (backing the `NativePtr*` builtins) ─────────────
//
// The compiler-generated `out`-param stub
// (`compiler/functions_foreign.rs::emit_out_param_stub`) allocates one cell per
// `out` param, passes its ADDRESS to the C function as a `ptr` argument, reads
// the value back after the call, and frees the cell. The cell is an 8-byte,
// 8-aligned, zero-initialized heap slot — large enough for any supported scalar
// or pointer out value, zero-filled so a narrower C write (e.g. `int*` into the
// low 4 bytes) reads back cleanly.
//
// The cell ADDRESS is always carried as the NON-heap `NativeKind::UIntSize`
// (see `native_kind_for_ctype`): a raw heap address must never be
// `Ptr(HeapKind::…)` or `KindedSlot::Drop` would `Arc::decrement_strong_count`
// on it = UB. These primitives are `unsafe` at the raw-pointer edge only; the
// kind discipline lives at the `KindedSlot` boundary in the caller.

/// 8-byte, 8-aligned cell layout. `alloc`/`dealloc` MUST use the identical
/// layout, so it is defined once here.
pub(crate) const NATIVE_CELL_LAYOUT: std::alloc::Layout =
    match std::alloc::Layout::from_size_align(8, 8) {
        Ok(l) => l,
        Err(_) => panic!("native cell layout is statically valid"),
    };

/// Allocate a fresh zeroed out-param cell. Returns its address, or `None` on
/// allocation failure.
pub(crate) fn native_cell_new() -> Option<usize> {
    // SAFETY: NATIVE_CELL_LAYOUT is a valid non-zero-sized layout.
    let ptr = unsafe { std::alloc::alloc_zeroed(NATIVE_CELL_LAYOUT) };
    if ptr.is_null() {
        None
    } else {
        Some(ptr as usize)
    }
}

/// Write 8 raw bytes into a live cell.
///
/// # Safety
/// `addr` must be a live cell returned by [`native_cell_new`] and not yet freed.
pub(crate) unsafe fn native_cell_write(addr: usize, bits: u64) {
    unsafe { std::ptr::write(addr as *mut u64, bits) };
}

/// Read the pointer-sized value back out of a live cell.
///
/// # Safety
/// `addr` must be a live cell returned by [`native_cell_new`] and not yet freed.
pub(crate) unsafe fn native_cell_read(addr: usize) -> u64 {
    unsafe { std::ptr::read(addr as *const u64) }
}

/// Release a cell. Pairs with [`native_cell_new`] under the identical layout.
///
/// # Safety
/// `addr` must be a live cell returned by [`native_cell_new`], freed exactly
/// once.
pub(crate) unsafe fn native_cell_free(addr: usize) {
    unsafe { std::alloc::dealloc(addr as *mut u8, NATIVE_CELL_LAYOUT) };
}

// ── Linked handle + link / invoke ────────────────────────────────────────────

/// Linked native function handle used by VM foreign-call dispatch.
pub struct NativeLinkedFunction {
    signature: CSignature,
    cif: Cif,
    code_ptr: CodePtr,
    /// Keep the dynamic library alive for symbol/call lifetime.
    _library: Arc<Library>,
}

// SAFETY: `code_ptr` is a resolved symbol address into a `dlopen`-ed library
// kept alive by `_library`; `Cif` is an immutable libffi call descriptor. The
// handle is only ever read (never mutated) after construction, and the C call
// itself is synchronous. Sharing the immutable handle across threads is sound.
unsafe impl Send for NativeLinkedFunction {}
unsafe impl Sync for NativeLinkedFunction {}

/// Resolve a declared native-library alias to a `dlopen` target
/// (ffi-rebuild §4.2 — pure path computation, no `dlopen`).
///
/// Stage-0 A7 stores the DECLARED alias in `NativeAbiSpec.library` (so content
/// hashes are host-stable), deferring resolution to the executing host at
/// link-now. This resolves the two host-independent cases:
///
/// - **Well-known system aliases** (`c`/`libc`, `m`/`libm`, `pthread`, `dl`) →
///   the platform soname. Mirrors the compile-side
///   `resolve_native_library_alias` table (`functions_foreign.rs:889`).
/// - **A direct path or an explicit soname** (contains `/` or a lib
///   extension) → passthrough; `dlopen` handles it.
///
/// Package-local frontmatter / `shape.toml` `[native-dependencies]` aliases
/// need the compiler's native-resolution context threaded to the executing
/// host — that receiver-side resolution chain (ffi-rebuild §4.11 / integration
/// A7) is a follow-up; such an alias falls through as-is and surfaces a
/// structured "cannot open" error at `dlopen` if unresolved.
pub(crate) fn resolve_library_target(alias: &str) -> String {
    match alias {
        "c" | "libc" => {
            #[cfg(target_os = "linux")]
            return "libc.so.6".to_string();
            #[cfg(target_os = "macos")]
            return "libSystem.B.dylib".to_string();
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return "msvcrt.dll".to_string();
        }
        "m" | "libm" => {
            #[cfg(target_os = "linux")]
            return "libm.so.6".to_string();
            #[cfg(target_os = "macos")]
            return "libSystem.B.dylib".to_string();
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return "msvcrt.dll".to_string();
        }
        other => other.to_string(),
    }
}

pub fn link_native_function(
    spec: &NativeAbiSpec,
    _native_layouts: &[NativeStructLayoutEntry],
    library_cache: &mut HashMap<String, Arc<Library>>,
) -> Result<NativeLinkedFunction, String> {
    if spec.abi != "C" {
        return Err(format!(
            "unsupported native ABI '{}'; only \"C\" is currently supported",
            spec.abi
        ));
    }

    let signature = parse_signature(&spec.signature)?;

    // Resolve the declared alias to a `dlopen` target (§4.2). Cache keyed by
    // the resolved target so two aliases for one library share a `dlopen`.
    let load_target = resolve_library_target(&spec.library);
    let library = if let Some(existing) = library_cache.get(&load_target) {
        existing.clone()
    } else {
        let opened = unsafe { Library::new(&load_target) }.map_err(|e| {
            format!(
                "failed to open native library '{}' (resolved to '{}'): {}",
                spec.library, load_target, e
            )
        })?;
        let shared = Arc::new(opened);
        library_cache.insert(load_target.clone(), shared.clone());
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
        _library: library,
    })
}

pub fn invoke_linked_function(
    linked: &NativeLinkedFunction,
    args: &[KindedSlot],
) -> Result<KindedSlot, String> {
    if linked.signature.params.len() != args.len() {
        return Err(format!(
            "native ABI argument count mismatch: signature expects {}, got {}",
            linked.signature.params.len(),
            args.len()
        ));
    }

    // Owned `CString`s for any `cstring` arguments. Kept alive across the C
    // call, freed when this vector drops after the call returns.
    let mut owned_cstrings: Vec<CString> = Vec::new();

    let mut prepared_args = Vec::with_capacity(linked.signature.params.len());
    for (idx, (ctype, value)) in linked.signature.params.iter().zip(args.iter()).enumerate() {
        let prepared = match ctype {
            CType::CString => encode_cstring_arg(value, idx, &mut owned_cstrings)?,
            _ => kinded_slot_to_prepared_arg(value, ctype, idx)?,
        };
        prepared_args.push(prepared);
    }

    let ffi_args: Vec<Arg> = prepared_args.iter().map(PreparedArg::as_arg).collect();

    // The kind that labels the result slot is the return `CType`'s static
    // classification (§2.7.5). Reject deferred return types before making the
    // call so we never fabricate a placeholder kind.
    let result = match &linked.signature.ret {
        CType::Void => {
            unsafe { linked.cif.call::<()>(linked.code_ptr, &ffi_args) };
            KindedSlot::none()
        }
        CType::I8 => {
            let out = unsafe { linked.cif.call::<i8>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::Int8)
        }
        CType::U8 => {
            let out = unsafe { linked.cif.call::<u8>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::UInt8)
        }
        CType::I16 => {
            let out = unsafe { linked.cif.call::<i16>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::Int16)
        }
        CType::U16 => {
            let out = unsafe { linked.cif.call::<u16>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::UInt16)
        }
        CType::I32 => {
            let out = unsafe { linked.cif.call::<i32>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::Int32)
        }
        CType::U32 => {
            let out = unsafe { linked.cif.call::<u32>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::UInt32)
        }
        CType::I64 => {
            let out = unsafe { linked.cif.call::<i64>(linked.code_ptr, &ffi_args) };
            KindedSlot::from_int(out)
        }
        CType::U64 => {
            let out = unsafe { linked.cif.call::<u64>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out), NativeKind::UInt64)
        }
        CType::Isize => {
            let out = unsafe { linked.cif.call::<isize>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::IntSize)
        }
        CType::Usize => {
            let out = unsafe { linked.cif.call::<usize>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::UIntSize)
        }
        // Every raw C pointer return → non-heap UIntSize (anti-UB invariant).
        CType::Ptr | CType::CView(_) | CType::CMut(_) | CType::Callback(_) => {
            let out = unsafe { linked.cif.call::<usize>(linked.code_ptr, &ffi_args) };
            KindedSlot::new(ValueSlot::from_raw(out as u64), NativeKind::UIntSize)
        }
        CType::F32 => {
            let out = unsafe { linked.cif.call::<f32>(linked.code_ptr, &ffi_args) };
            KindedSlot::from_f32(out)
        }
        CType::F64 => {
            let out = unsafe { linked.cif.call::<f64>(linked.code_ptr, &ffi_args) };
            KindedSlot::from_number(out)
        }
        CType::Bool => {
            let out = unsafe { linked.cif.call::<u8>(linked.code_ptr, &ffi_args) };
            KindedSlot::from_bool(out != 0)
        }
        CType::CString => {
            let out = unsafe { linked.cif.call::<*const c_char>(linked.code_ptr, &ffi_args) };
            if out.is_null() {
                return Err(
                    "native call: `cstring` return was a null pointer (declared non-nullable; \
                     use `cstring?` for a nullable return)"
                        .to_string(),
                );
            }
            // Borrowed-copy semantics: copy the C string into a fresh Shape
            // string carrier; we never free the C-returned pointer
            // (ownership stays with the C library; §4.6.2 / OQ5).
            let s = unsafe { CStr::from_ptr(out) }.to_string_lossy().to_string();
            KindedSlot::from_string(&s)
        }
        deferred @ (CType::NullableCString | CType::CSlice(_) | CType::CMutSlice(_)) => {
            return Err(format!(
                "native call: return type `{}` is not implemented in this build \
                 (WF-2A stage 2 covers scalar/ptr/cstring; cstring?/slice are a later sub-wave)",
                deferred.label()
            ));
        }
    };

    // Sanity: the produced kind must match the static C-type classification.
    debug_assert_eq!(
        native_kind_for_ctype(&linked.signature.ret),
        match linked.signature.ret {
            CType::Void
            | CType::NullableCString
            | CType::CSlice(_)
            | CType::CMutSlice(_) => None,
            _ => Some(result.kind()),
        },
        "native return kind must equal native_kind_for_ctype()"
    );

    // `owned_cstrings` drops here, freeing the arg CStrings now that the C
    // call has completed.
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_signature_scalar_roundtrip() {
        let sig = parse_signature("fn(i64, i64) -> i64").unwrap();
        assert_eq!(sig.params, vec![CType::I64, CType::I64]);
        assert_eq!(sig.ret, CType::I64);

        let sig = parse_signature("fn() -> void").unwrap();
        assert!(sig.params.is_empty());
        assert_eq!(sig.ret, CType::Void);

        let sig = parse_signature("fn(f64) -> f64").unwrap();
        assert_eq!(sig.params, vec![CType::F64]);
        assert_eq!(sig.ret, CType::F64);
    }

    #[test]
    fn native_kind_table_ptr_is_non_heap() {
        // The load-bearing anti-UB assertion: every C pointer kind is a
        // non-heap scalar (UIntSize), never Ptr(HeapKind::…).
        for ct in [
            CType::Ptr,
            CType::CView("Foo".into()),
            CType::CMut("Foo".into()),
        ] {
            assert_eq!(native_kind_for_ctype(&ct), Some(NativeKind::UIntSize));
            assert!(!matches!(
                native_kind_for_ctype(&ct),
                Some(NativeKind::Ptr(_))
            ));
        }
        assert_eq!(native_kind_for_ctype(&CType::I32), Some(NativeKind::Int32));
        assert_eq!(
            native_kind_for_ctype(&CType::F64),
            Some(NativeKind::Float64)
        );
        assert_eq!(native_kind_for_ctype(&CType::Bool), Some(NativeKind::Bool));
        assert_eq!(
            native_kind_for_ctype(&CType::CString),
            Some(NativeKind::String)
        );
        assert_eq!(native_kind_for_ctype(&CType::Void), None);
    }

    #[test]
    fn arg_marshal_int_widths() {
        let v = KindedSlot::from_int(-7);
        assert!(matches!(
            kinded_slot_to_prepared_arg(&v, &CType::I32, 0).unwrap(),
            PreparedArg::I32(-7)
        ));
        assert!(matches!(
            kinded_slot_to_prepared_arg(&v, &CType::I64, 0).unwrap(),
            PreparedArg::I64(-7)
        ));
        let big = KindedSlot::from_int(9_000_000_000);
        assert!(matches!(
            kinded_slot_to_prepared_arg(&big, &CType::I64, 0).unwrap(),
            PreparedArg::I64(9_000_000_000)
        ));
    }

    #[test]
    fn arg_marshal_float_and_bool() {
        let f = KindedSlot::from_number(144.0);
        assert!(matches!(
            kinded_slot_to_prepared_arg(&f, &CType::F64, 0).unwrap(),
            PreparedArg::F64(x) if x == 144.0
        ));
        let i = KindedSlot::from_int(3);
        assert!(matches!(
            kinded_slot_to_prepared_arg(&i, &CType::F64, 0).unwrap(),
            PreparedArg::F64(x) if x == 3.0
        ));
        let b = KindedSlot::from_bool(true);
        assert!(matches!(
            kinded_slot_to_prepared_arg(&b, &CType::Bool, 0).unwrap(),
            PreparedArg::Bool(1)
        ));
    }

    #[test]
    fn arg_marshal_deferred_types_error() {
        let v = KindedSlot::from_int(0);
        assert!(kinded_slot_to_prepared_arg(&v, &CType::CSlice(Box::new(CType::I32)), 0).is_err());
        assert!(kinded_slot_to_prepared_arg(&v, &CType::NullableCString, 0).is_err());
    }

    #[test]
    fn cstring_arg_rejects_embedded_nul() {
        let mut owned = Vec::new();
        let s = KindedSlot::from_string("a\0b");
        assert!(encode_cstring_arg(&s, 0, &mut owned).is_err());
    }

    /// Differential-of-truth against libc: link `abs`/`labs` and invoke.
    #[test]
    fn invoke_libc_abs_labs() {
        #[cfg(target_os = "linux")]
        let libname = "libc.so.6";
        #[cfg(target_os = "macos")]
        let libname = "libSystem.B.dylib";
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let libname = "msvcrt.dll";

        let mut cache: HashMap<String, Arc<Library>> = HashMap::new();

        let abs_spec = NativeAbiSpec {
            abi: "C".into(),
            library: libname.into(),
            symbol: "abs".into(),
            signature: "fn(i32) -> i32".into(),
        };
        let abs = link_native_function(&abs_spec, &[], &mut cache).unwrap();
        let r = invoke_linked_function(&abs, &[KindedSlot::from_int(-7)]).unwrap();
        assert_eq!(r.kind(), NativeKind::Int32);
        assert_eq!(r.raw() as u32 as i32, 7);

        let labs_spec = NativeAbiSpec {
            abi: "C".into(),
            library: libname.into(),
            symbol: "labs".into(),
            signature: "fn(i64) -> i64".into(),
        };
        let labs = link_native_function(&labs_spec, &[], &mut cache).unwrap();
        let r = invoke_linked_function(&labs, &[KindedSlot::from_int(-9_000_000_000)]).unwrap();
        assert_eq!(r.kind(), NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(9_000_000_000));
    }

    /// libm f64 differential: `sqrt(144.0) == 12.0`.
    #[test]
    fn invoke_libm_sqrt() {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            let libname = "libm.so.6";
            #[cfg(target_os = "macos")]
            let libname = "libSystem.B.dylib";
            let mut cache: HashMap<String, Arc<Library>> = HashMap::new();
            let spec = NativeAbiSpec {
                abi: "C".into(),
                library: libname.into(),
                symbol: "sqrt".into(),
                signature: "fn(f64) -> f64".into(),
            };
            let sqrt = link_native_function(&spec, &[], &mut cache).unwrap();
            let r = invoke_linked_function(&sqrt, &[KindedSlot::from_number(144.0)]).unwrap();
            assert_eq!(r.kind(), NativeKind::Float64);
            assert_eq!(r.as_f64(), Some(12.0));
        }
    }

    /// A `ptr`-classified return yields a non-heap UIntSize slot whose Drop is
    /// a no-op — never an `Arc::decrement_strong_count` on a foreign address.
    #[test]
    fn ptr_return_is_non_heap_uintsize() {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            let libname = "libc.so.6";
            #[cfg(target_os = "macos")]
            let libname = "libSystem.B.dylib";
            let mut cache: HashMap<String, Arc<Library>> = HashMap::new();
            let spec = NativeAbiSpec {
                abi: "C".into(),
                library: libname.into(),
                symbol: "labs".into(),
                signature: "fn(ptr) -> ptr".into(),
            };
            let f = link_native_function(&spec, &[], &mut cache).unwrap();
            let arg = KindedSlot::new(ValueSlot::from_raw(0x1000), NativeKind::UIntSize);
            let r = invoke_linked_function(&f, &[arg]).unwrap();
            assert_eq!(r.kind(), NativeKind::UIntSize);
            assert!(!matches!(r.kind(), NativeKind::Ptr(_)));
            drop(r);
        }
    }

    /// End-to-end cstring: libc `strlen("hello") == 5`.
    #[test]
    fn invoke_libc_strlen_cstring_arg() {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            let libname = "libc.so.6";
            #[cfg(target_os = "macos")]
            let libname = "libSystem.B.dylib";
            let mut cache: HashMap<String, Arc<Library>> = HashMap::new();
            let spec = NativeAbiSpec {
                abi: "C".into(),
                library: libname.into(),
                symbol: "strlen".into(),
                signature: "fn(cstring) -> usize".into(),
            };
            let f = link_native_function(&spec, &[], &mut cache).unwrap();
            let r = invoke_linked_function(&f, &[KindedSlot::from_string("hello")]).unwrap();
            assert_eq!(r.kind(), NativeKind::UIntSize);
            assert_eq!(r.raw(), 5);
        }
    }

    // ── out-param cell primitives ────────────────────────────────────────

    /// Cell round-trip: zeroed on alloc, write/read back exact bits, free.
    #[test]
    fn native_cell_roundtrip_and_no_leak() {
        let z = native_cell_new().unwrap();
        assert_eq!(
            unsafe { native_cell_read(z) },
            0,
            "fresh cell must be zeroed"
        );
        unsafe { native_cell_write(z, 0xDEAD_BEEF_CAFE_1234) };
        assert_eq!(unsafe { native_cell_read(z) }, 0xDEAD_BEEF_CAFE_1234);
        unsafe { native_cell_free(z) };

        for i in 0..1000u64 {
            let c = native_cell_new().unwrap();
            assert_eq!(unsafe { native_cell_read(c) }, 0);
            unsafe { native_cell_write(c, i.wrapping_mul(0x9E37_79B9)) };
            assert_eq!(unsafe { native_cell_read(c) }, i.wrapping_mul(0x9E37_79B9));
            unsafe { native_cell_free(c) };
        }
    }

    /// End-to-end out-param marshalling against libm `frexp`:
    /// `double frexp(double x, int* exp)`. Alloc a cell, pass its ADDRESS as
    /// the `ptr` (int*) argument, invoke, then read the exponent back.
    #[test]
    fn out_param_frexp_via_cell() {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            let libname = "libm.so.6";
            #[cfg(target_os = "macos")]
            let libname = "libSystem.B.dylib";
            let mut cache: HashMap<String, Arc<Library>> = HashMap::new();
            let spec = NativeAbiSpec {
                abi: "C".into(),
                library: libname.into(),
                symbol: "frexp".into(),
                signature: "fn(f64, ptr) -> f64".into(),
            };
            let f = link_native_function(&spec, &[], &mut cache).unwrap();

            let cell = native_cell_new().unwrap();
            let cell_slot = KindedSlot::new(ValueSlot::from_raw(cell as u64), NativeKind::UIntSize);
            let r =
                invoke_linked_function(&f, &[KindedSlot::from_number(144.0), cell_slot]).unwrap();

            // 144.0 == 0.5625 * 2^8.
            assert_eq!(r.kind(), NativeKind::Float64);
            assert_eq!(r.as_f64(), Some(0.5625));
            let exp = unsafe { native_cell_read(cell) };
            assert_eq!(exp, 8, "frexp wrote the exponent through the cell pointer");
            unsafe { native_cell_free(cell) };
        }
    }
}
