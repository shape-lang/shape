//! ADR-019 §2 / R25 (POLY-ZERO-COPY, issue #199) — the `shared` call path.
//!
//! These drive the REAL VM path (`invoke_foreign_kinded` → `plan_views` →
//! `invoke_with_buffers` → release accounting) against an in-process fake
//! extension, so they assert the HOST's behaviour and fail if the host stops
//! doing it — independently of whether any interpreter is installed. The
//! Python-specific mechanisms (read-only `memoryview`, buffer-protocol export
//! counting) have their own tests in `extensions/python`.
//!
//! The fake is a real consumer, not a stub: it reads through the immutable view
//! and writes through the mutable one, so "zero-copy" here means the bytes the
//! foreign side saw were the caller's own bytes and the caller's array changed
//! underneath it. A fake that ignored the pointer would let a broken view table
//! pass.

use crate::executor::VirtualMachine;
use shape_abi_v1::{
    BUFFER_ELEM_FLOAT64, BUFFER_ELEM_INT64, BUFFER_MODE_SHARED, BUFFER_MODE_SHARED_MUT,
    BufferCapability, EXTENSION_CAPABILITIES_VERSION, ExtensionCapabilities, ForeignBufferView,
};
use shape_value::v2::typed_array::{ELEM_TYPE_F64, TypedArray, stamp_elem_type};
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use std::ffi::c_void;
use std::sync::Mutex;

// ── The fake extension ─────────────────────────────────────────────────────

mod fake {
    use super::*;
    use shape_abi_v1::{ErrorModel, LanguageRuntimeVTable, STATE_MODEL_STATEFUL_OPAQUE};
    use std::ffi::c_char;

    /// What the fake's last `invoke_with_buffers` saw, in order.
    pub static SEEN: Mutex<Vec<(u32, u32, u32, u64)>> = Mutex::new(Vec::new());
    /// Sum of every element read through the immutable views of the last call.
    pub static SUM: Mutex<f64> = Mutex::new(0.0);
    /// The value the fake writes into every element of every mutable view.
    pub static WRITE_VALUE: Mutex<Option<f64>> = Mutex::new(None);
    /// The mask `outstanding_exports` will report — the fake's way of playing a
    /// body that stashed a view.
    pub static RETAIN_MASK: Mutex<u64> = Mutex::new(0);
    /// Whether `invoke_with_buffers` should panic, to exercise the extension
    /// shell's containment and the host's accounting-on-failure path.
    pub static PANIC_IN_BODY: Mutex<bool> = Mutex::new(false);
    /// How many times the plain (copying) `invoke` was used instead.
    pub static PLAIN_INVOKES: Mutex<u32> = Mutex::new(0);

    pub fn reset() {
        SEEN.lock().unwrap().clear();
        *SUM.lock().unwrap() = 0.0;
        *WRITE_VALUE.lock().unwrap() = None;
        *RETAIN_MASK.lock().unwrap() = 0;
        *PANIC_IN_BODY.lock().unwrap() = false;
        *PLAIN_INVOKES.lock().unwrap() = 0;
    }

    unsafe extern "C" fn init(_c: *const u8, _l: usize) -> *mut c_void {
        1usize as *mut c_void
    }
    unsafe extern "C" fn language_id(_i: *mut c_void) -> *const c_char {
        c"python".as_ptr()
    }
    unsafe extern "C" fn drop_instance(_i: *mut c_void) {}
    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn compile(
        _i: *mut c_void,
        _n: *const u8,
        _nl: usize,
        _s: *const u8,
        _sl: usize,
        _pn: *const u8,
        _pnl: usize,
        _pt: *const u8,
        _ptl: usize,
        _rt: *const u8,
        _rtl: usize,
        _a: bool,
        _oe: *mut *mut u8,
        _oel: *mut usize,
    ) -> *mut c_void {
        1usize as *mut c_void
    }

    /// msgpack for the integer 7 — the result every invoke path returns, so a
    /// test that reaches the wrong path is visible in what it did, not in what
    /// it returned.
    fn write_seven(out_ptr: *mut *mut u8, out_len: *mut usize) {
        let mut buf = vec![7u8];
        let len = buf.len();
        let ptr = buf.as_mut_ptr();
        std::mem::forget(buf);
        unsafe {
            *out_ptr = ptr;
            *out_len = len;
        }
    }

    unsafe extern "C" fn invoke(
        _i: *mut c_void,
        _h: *mut c_void,
        _a: *const u8,
        _al: usize,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        *PLAIN_INVOKES.lock().unwrap() += 1;
        write_seven(out_ptr, out_len);
        0
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "C" fn invoke_with_buffers(
        _i: *mut c_void,
        _h: *mut c_void,
        _a: *const u8,
        _al: usize,
        views: *const ForeignBufferView,
        views_len: usize,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        // Every extension entry point contains its own panic — unwinding across
        // the C ABI is undefined behaviour, so containment is the extension's
        // job, exactly as the `language_runtime_plugin!` shells do it for the
        // vtable's own slots.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let views = unsafe { std::slice::from_raw_parts(views, views_len) };
            let mut seen = SEEN.lock().unwrap();
            let mut sum = SUM.lock().unwrap();
            let write = *WRITE_VALUE.lock().unwrap();
            for view in views {
                seen.push((view.arg_index, view.elem_type, view.mode, view.len));
                if view.len == 0 {
                    continue;
                }
                match view.elem_type {
                    BUFFER_ELEM_FLOAT64 => {
                        let s = unsafe {
                            std::slice::from_raw_parts(view.data as *const f64, view.len as usize)
                        };
                        for v in s {
                            *sum += *v;
                        }
                        if view.mode == BUFFER_MODE_SHARED_MUT {
                            if let Some(w) = write {
                                let s = unsafe {
                                    std::slice::from_raw_parts_mut(
                                        view.data as *mut f64,
                                        view.len as usize,
                                    )
                                };
                                for v in s.iter_mut() {
                                    *v = w;
                                }
                            }
                        }
                    }
                    BUFFER_ELEM_INT64 => {
                        let s = unsafe {
                            std::slice::from_raw_parts(view.data as *const i64, view.len as usize)
                        };
                        for v in s {
                            *sum += *v as f64;
                        }
                    }
                    _ => {}
                }
            }
            drop(seen);
            drop(sum);
            if *PANIC_IN_BODY.lock().unwrap() {
                panic!("fake foreign body raised");
            }
        }));

        if result.is_err() {
            return 1;
        }
        write_seven(out_ptr, out_len);
        0
    }

    unsafe extern "C" fn outstanding_exports(_i: *mut c_void) -> u64 {
        *RETAIN_MASK.lock().unwrap()
    }

    unsafe extern "C" fn dispose_function(_i: *mut c_void, _h: *mut c_void) {}
    unsafe extern "C" fn free_buffer(ptr: *mut u8, len: usize) {
        if !ptr.is_null() {
            unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
        }
    }

    static FULL: BufferCapability = BufferCapability {
        struct_size: std::mem::size_of::<BufferCapability>() as u32,
        modes: BUFFER_MODE_SHARED | BUFFER_MODE_SHARED_MUT,
        elem_types: (1 << BUFFER_ELEM_INT64) | (1 << BUFFER_ELEM_FLOAT64),
        _reserved: 0,
        invoke_with_buffers: Some(invoke_with_buffers),
        outstanding_exports: Some(outstanding_exports),
    };
    /// Read-only sharing: the runtime can hand out a view but not a writable
    /// one.
    static READ_ONLY: BufferCapability = BufferCapability {
        struct_size: std::mem::size_of::<BufferCapability>() as u32,
        modes: BUFFER_MODE_SHARED,
        elem_types: (1 << BUFFER_ELEM_INT64) | (1 << BUFFER_ELEM_FLOAT64),
        _reserved: 0,
        invoke_with_buffers: Some(invoke_with_buffers),
        outstanding_exports: Some(outstanding_exports),
    };
    /// Views offered, nothing that can say they were released — ADR-019 §2's
    /// named refusal.
    static UNACCOUNTED: BufferCapability = BufferCapability {
        struct_size: std::mem::size_of::<BufferCapability>() as u32,
        modes: BUFFER_MODE_SHARED | BUFFER_MODE_SHARED_MUT,
        elem_types: (1 << BUFFER_ELEM_INT64) | (1 << BUFFER_ELEM_FLOAT64),
        _reserved: 0,
        invoke_with_buffers: Some(invoke_with_buffers),
        outstanding_exports: None,
    };

    static CAPS_FULL: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &FULL,
    };
    static CAPS_READ_ONLY: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &READ_ONLY,
    };
    static CAPS_UNACCOUNTED: ExtensionCapabilities = ExtensionCapabilities {
        struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
        version: EXTENSION_CAPABILITIES_VERSION,
        buffers: &UNACCOUNTED,
    };

    unsafe extern "C" fn caps_full(_i: *mut c_void) -> *const ExtensionCapabilities {
        &CAPS_FULL
    }
    unsafe extern "C" fn caps_read_only(_i: *mut c_void) -> *const ExtensionCapabilities {
        &CAPS_READ_ONLY
    }
    unsafe extern "C" fn caps_unaccounted(_i: *mut c_void) -> *const ExtensionCapabilities {
        &CAPS_UNACCOUNTED
    }

    const fn vtable(
        capabilities: Option<unsafe extern "C" fn(*mut c_void) -> *const ExtensionCapabilities>,
    ) -> LanguageRuntimeVTable {
        LanguageRuntimeVTable {
            init: Some(init),
            register_types: None,
            compile: Some(compile),
            invoke: Some(invoke),
            dispose_function: Some(dispose_function),
            language_id: Some(language_id),
            get_lsp_config: None,
            free_buffer: Some(free_buffer),
            drop: Some(drop_instance),
            error_model: ErrorModel::Dynamic,
            get_shape_source: None,
            runtime_descriptor: None,
            state_model: STATE_MODEL_STATEFUL_OPAQUE,
            generate_stubs: None,
            instance_concurrency: None,
            dispose_ref: None,
            capabilities,
        }
    }

    pub static SHARING: LanguageRuntimeVTable = vtable(Some(caps_full));
    pub static READ_ONLY_SHARING: LanguageRuntimeVTable = vtable(Some(caps_read_only));
    pub static UNACCOUNTED_SHARING: LanguageRuntimeVTable = vtable(Some(caps_unaccounted));
    /// A runtime that offers no buffer capability at all — the shape the
    /// TypeScript extension ships, and every extension built before #199.
    pub static NO_SHARING: LanguageRuntimeVTable = vtable(None);
}

/// Serialize this module's access to the fake's statics.
static FAKE_LOCK: Mutex<()> = Mutex::new(());

fn vm_with(code: &str, vtable: &'static shape_abi_v1::LanguageRuntimeVTable) -> VirtualMachine {
    use crate::compiler::BytecodeCompiler;
    use crate::executor::VMConfig;
    use shape_runtime::plugins::language_runtime::PluginLanguageRuntime;

    let program = shape_ast::parser::parse_program(code).expect("parse failed");
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("compile failed");

    let runtime =
        PluginLanguageRuntime::new(vtable, &serde_json::Value::Null).expect("fake runtime inits");
    let mut runtimes = std::collections::HashMap::new();
    runtimes.insert("python".to_string(), std::sync::Arc::new(runtime));

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.foreign_fn_handles = vec![None];
    vm.set_language_runtimes(runtimes);
    vm
}

/// A live `Array<number>` argument, owned by the test for the call's duration —
/// which is exactly how a caller's slot owns it in a real program.
struct OwnedF64Array(*mut TypedArray<f64>);

impl OwnedF64Array {
    fn new(values: &[f64]) -> Self {
        let arr = TypedArray::<f64>::from_slice(values);
        unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64) };
        OwnedF64Array(arr)
    }
    /// A slot holding a fresh share, the way a caller's stack slot does.
    ///
    /// `KindedSlot::new` CLAIMS a share rather than borrowing one, and its
    /// `Drop` retires it, so the retain has to be explicit — the same
    /// share-accounting shape as `executor/mod.rs::module_binding_read_owned_kinded`.
    /// Without it the slot's drop would free the array out from under this
    /// owner and the second free would abort the process.
    fn slot(&self) -> KindedSlot {
        unsafe {
            shape_value::v2::refcount::v2_retain(
                self.0 as *const shape_value::v2::heap_header::HeapHeader,
            )
        };
        KindedSlot::new(
            ValueSlot::from_raw(self.0 as usize as u64),
            NativeKind::Ptr(HeapKind::TypedArray),
        )
    }
    fn contents(&self) -> Vec<f64> {
        unsafe { TypedArray::<f64>::as_slice(self.0) }.to_vec()
    }
}

impl Drop for OwnedF64Array {
    fn drop(&mut self) {
        unsafe { shape_value::v2::typed_array::release_v2_typed_array(self.0 as *mut u8) };
    }
}

const SHARED_SUM: &str = r#"
fn python total(shared xs: Array<number>) -> Result<int> {
    return 0
}
"#;

const SHARED_MUT_FILL: &str = r#"
fn python fill(shared mut xs: Array<number>) -> Result<int> {
    return 0
}
"#;

// ── The buffer really crosses ──────────────────────────────────────────────

#[test]
fn a_shared_array_reaches_the_extension_as_a_view_over_the_callers_own_memory() {
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.5, 2.5, 3.0]);
    let mut vm = vm_with(SHARED_SUM, &fake::SHARING);
    vm.invoke_foreign_kinded(0, &[arr.slot()])
        .expect("the shared call runs");

    let seen = fake::SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "exactly one view crossed, got {seen:?}");
    let (arg_index, elem_type, mode, len) = seen[0];
    assert_eq!(arg_index, 0, "the view names the argument it stands in for");
    assert_eq!(elem_type, BUFFER_ELEM_FLOAT64);
    assert_eq!(mode, BUFFER_MODE_SHARED);
    assert_eq!(len, 3, "the view carries the element count, not the bytes");
    assert_eq!(
        *fake::SUM.lock().unwrap(),
        7.0,
        "the extension read the caller's own values through the pointer"
    );
    assert_eq!(
        *fake::PLAIN_INVOKES.lock().unwrap(),
        0,
        "a shared declaration must not fall back to the copying invoke"
    );
}

#[test]
fn a_shared_mut_view_writes_land_in_the_callers_array() {
    // The half a copy could never fake: after the call the CALLER's buffer holds
    // what the foreign side wrote.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();
    *fake::WRITE_VALUE.lock().unwrap() = Some(42.0);

    let arr = OwnedF64Array::new(&[1.0, 2.0, 3.0]);
    let mut vm = vm_with(SHARED_MUT_FILL, &fake::SHARING);
    vm.invoke_foreign_kinded(0, &[arr.slot()])
        .expect("the shared-mut call runs");

    assert_eq!(
        arr.contents(),
        vec![42.0, 42.0, 42.0],
        "writes through the exclusive view are the caller's own memory changing"
    );
}

#[test]
fn an_empty_shared_array_exports_a_zero_length_view_and_no_pointer() {
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[]);
    let mut vm = vm_with(SHARED_SUM, &fake::SHARING);
    vm.invoke_foreign_kinded(0, &[arr.slot()])
        .expect("an empty shared array is not an error");

    let seen = fake::SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].3, 0, "zero elements");
}

// ── Retention: the corruption class, caught ────────────────────────────────

#[test]
fn a_view_the_body_kept_fails_the_call_and_names_the_parameter() {
    // #199 tripwire (2). The fake plays the `numpy.asarray(xs)`-into-a-global
    // body: the call itself succeeds, and the boundary fails it anyway because
    // the alternative is reclaiming memory foreign code still points at.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();
    *fake::RETAIN_MASK.lock().unwrap() = 0b1;

    let arr = OwnedF64Array::new(&[1.0, 2.0]);
    let mut vm = vm_with(SHARED_SUM, &fake::SHARING);
    let err = vm
        .invoke_foreign_kinded(0, &[arr.slot()])
        .expect_err("a retained view fails the call");
    let message = format!("{err:?}");
    assert!(
        message.contains("still held a view"),
        "the failure names what happened, got: {message}"
    );
    assert!(
        message.contains("'xs'"),
        "the failure names the parameter whose view survived, got: {message}"
    );
    assert!(
        message.contains("numpy.array") && message.contains("numpy.asarray"),
        "the failure tells the author which call copies, got: {message}"
    );
}

#[test]
fn retention_outranks_the_bodys_own_failure() {
    // A body that both raised and kept a view has two problems, and only one of
    // them gets worse if the program continues. The accounting is asked even
    // when the invoke returned an error, so the memory-safety verdict is the one
    // reported.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();
    *fake::PANIC_IN_BODY.lock().unwrap() = true;
    *fake::RETAIN_MASK.lock().unwrap() = 0b1;

    let arr = OwnedF64Array::new(&[1.0]);
    let mut vm = vm_with(SHARED_SUM, &fake::SHARING);
    let err = vm
        .invoke_foreign_kinded(0, &[arr.slot()])
        .expect_err("a retained view fails the call even when the body failed too");
    assert!(
        format!("{err:?}").contains("still held a view"),
        "got: {err:?}"
    );
}

#[test]
fn a_panicking_body_leaves_the_pin_balanced() {
    // #199 tripwire (4). The extension shell converts the panic to an error
    // return, the host reports it as an ordinary foreign failure, and the
    // caller's array is intact and still owned — its `Drop` below runs clean
    // under the test binary's allocator checks.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();
    *fake::PANIC_IN_BODY.lock().unwrap() = true;

    let arr = OwnedF64Array::new(&[1.0, 2.0, 3.0]);
    let mut vm = vm_with(SHARED_SUM, &fake::SHARING);
    let outcome = vm.invoke_foreign_kinded(0, &[arr.slot()]);
    assert!(
        outcome.is_ok(),
        "a foreign-side failure is the language's error channel, not a VM error: {outcome:?}"
    );
    assert_eq!(
        arr.contents(),
        vec![1.0, 2.0, 3.0],
        "the caller's buffer survives a failed shared call unchanged"
    );
}

// ── Refusals at the call ───────────────────────────────────────────────────

#[test]
fn a_runtime_that_offers_no_buffers_refuses_the_call_instead_of_copying() {
    // The silent-fallback trap, closed. Deep-copying here would honour the
    // signature and betray the declaration: `shared` says the buffer crossed,
    // and an author reading a green run would believe it.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.0]);
    let mut vm = vm_with(SHARED_SUM, &fake::NO_SHARING);
    let err = vm
        .invoke_foreign_kinded(0, &[arr.slot()])
        .expect_err("a runtime without the capability refuses");
    let message = format!("{err:?}");
    assert!(
        message.contains("does not offer buffer sharing"),
        "got: {message}"
    );
    assert!(
        message.contains("quietly deep-copied"),
        "the refusal says why it is not a fallback, got: {message}"
    );
    assert_eq!(
        *fake::PLAIN_INVOKES.lock().unwrap(),
        0,
        "no call reaches the extension at all"
    );
}

#[test]
fn a_runtime_without_release_accounting_refuses_the_call() {
    // #199 tripwire (2), second half: "a language without release accounting has
    // the mode refused, asserted" — at the call, not just at negotiation.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.0]);
    let mut vm = vm_with(SHARED_SUM, &fake::UNACCOUNTED_SHARING);
    let err = vm
        .invoke_foreign_kinded(0, &[arr.slot()])
        .expect_err("views without accounting are refused");
    assert!(
        format!("{err:?}").contains("release accounting"),
        "got: {err:?}"
    );
    assert_eq!(*fake::PLAIN_INVOKES.lock().unwrap(), 0);
    assert!(
        fake::SEEN.lock().unwrap().is_empty(),
        "no pointer reached the extension"
    );
}

#[test]
fn a_declared_mut_mode_a_runtime_lacks_is_refused_not_downgraded() {
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.0]);
    let mut vm = vm_with(SHARED_MUT_FILL, &fake::READ_ONLY_SHARING);
    let err = vm
        .invoke_foreign_kinded(0, &[arr.slot()])
        .expect_err("a read-only runtime cannot honour `shared mut`");
    let message = format!("{err:?}");
    assert!(message.contains("does not implement"), "got: {message}");
    assert!(
        message.contains("silently discard the body's writes"),
        "the refusal says what a downgrade would cost, got: {message}"
    );
}

#[test]
fn two_shared_views_of_one_array_are_refused_when_either_is_mutable() {
    // ADR-006 exclusivity, checked where it is checkable. Shape's borrow solver
    // does not see through a foreign call's argument list, so the aliasing is
    // caught by address before anything is exported.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.0, 2.0]);
    let mut vm = vm_with(
        r#"
fn python blend(shared a: Array<number>, shared mut b: Array<number>) -> Result<int> {
    return 0
}
"#,
        &fake::SHARING,
    );
    let err = vm
        .invoke_foreign_kinded(0, &[arr.slot(), arr.slot()])
        .expect_err("an aliased mutable view is refused");
    let message = format!("{err:?}");
    assert!(
        message.contains("exclusive borrow"),
        "the refusal cites the rule it enforces, got: {message}"
    );
    assert!(
        fake::SEEN.lock().unwrap().is_empty(),
        "the refusal lands before any pointer is exported"
    );
}

#[test]
fn two_immutable_views_of_one_array_are_allowed() {
    // The other side of the same rule: any number of shared borrows coexist.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[2.0]);
    let mut vm = vm_with(
        r#"
fn python blend(shared a: Array<number>, shared b: Array<number>) -> Result<int> {
    return 0
}
"#,
        &fake::SHARING,
    );
    vm.invoke_foreign_kinded(0, &[arr.slot(), arr.slot()])
        .expect("two immutable views of one buffer is an ordinary shared borrow");
    assert_eq!(fake::SEEN.lock().unwrap().len(), 2);
}

#[test]
fn a_non_array_argument_in_a_shared_position_is_refused() {
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let mut vm = vm_with(SHARED_SUM, &fake::SHARING);
    let int_slot = KindedSlot::new(ValueSlot::from_raw(5), NativeKind::Int64);
    let err = vm
        .invoke_foreign_kinded(0, &[int_slot])
        .expect_err("only a native array has a buffer to export");
    assert!(
        format!("{err:?}").contains("not a native array"),
        "got: {err:?}"
    );
}

// ── Composition with async ─────────────────────────────────────────────────

#[test]
fn shared_on_an_async_declaration_is_refused_at_the_call() {
    // A view is valid for the duration of the CALL, and an offloaded call has no
    // such duration on the interpreter thread: it returns a `Future(id)`
    // immediately, and the caller's last share of the array could be dropped
    // while the worker is still inside the body.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.0]);
    let mut vm = vm_with(
        r#"
async fn python total(shared xs: Array<number>) -> Result<int> {
    return 0
}
"#,
        &fake::SHARING,
    );
    let err = vm
        .invoke_foreign_async_kinded(0, &[arr.slot()])
        .expect_err("shared and async do not compose in this slice");
    let message = format!("{err:?}");
    assert!(message.contains("'xs'"), "got: {message}");
    assert!(
        message.contains("Drop `async` to share"),
        "the refusal offers both ways out, got: {message}"
    );
}

// ── The copied path is untouched ───────────────────────────────────────────

#[test]
fn a_declaration_with_no_shared_parameter_still_takes_the_copying_invoke() {
    // The differential: adding the capability must not reroute the ordinary
    // boundary. A declaration that shares nothing goes through `invoke` exactly
    // as it did before #199, even against a runtime that offers buffers.
    let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fake::reset();

    let arr = OwnedF64Array::new(&[1.0, 2.0]);
    let mut vm = vm_with(
        r#"
fn python total(xs: Array<number>) -> Result<int> {
    return 0
}
"#,
        &fake::SHARING,
    );
    vm.invoke_foreign_kinded(0, &[arr.slot()])
        .expect("the copied path runs");
    assert_eq!(*fake::PLAIN_INVOKES.lock().unwrap(), 1);
    assert!(
        fake::SEEN.lock().unwrap().is_empty(),
        "no view is built for a declaration that shares nothing"
    );
}

// ── The copy tax, structurally ─────────────────────────────────────────────

#[test]
fn a_shared_argument_puts_nothing_on_the_wire() {
    // #199 tripwire (3), as a fact rather than a stopwatch. The copy tax IS the
    // element-wise walk in `typed_array_to_msgpack`: a copied `Array<number>`
    // grows the argument payload by roughly nine bytes per element, and a shared
    // one contributes a single nil however long it is. Asserting the payload
    // rather than the clock keeps this a regression test instead of a flake —
    // if a future change quietly reinstates the walk for shared parameters, the
    // byte count says so on every machine.
    //
    // The wall-clock figures the ticket asks for were measured separately
    // through this same path (release build, 20 iterations): at 1e6 elements the
    // call went from 12.4ms to 0.40ms, a 31x reduction, with the fake still
    // reading every element through the view.
    use crate::executor::control_flow::foreign_marshal;
    use shape_abi_v1::foreign_types::BufferShare;
    use shape_runtime::type_schema::TypeSchemaRegistry;

    let values: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
    let arr = OwnedF64Array::new(&values);
    let schemas = TypeSchemaRegistry::new();
    let types = vec!["Array<number>".to_string()];

    let copied = foreign_marshal::marshal_args_typed(&[arr.slot()], &types, &schemas)
        .expect("the copied path serializes every element");
    let shared = foreign_marshal::marshal_args_with_views(&[arr.slot()], &types, &[0], &schemas)
        .expect("the shared path serializes a placeholder");

    assert!(
        copied.len() > 10_000 * 8,
        "the copied path really does walk the elements onto the wire ({} bytes)",
        copied.len()
    );
    assert!(
        shared.len() < 8,
        "a shared argument contributes one nil, not a payload ({} bytes)",
        shared.len()
    );
    assert_eq!(
        foreign_marshal::plan_views(
            &[arr.slot()],
            &["xs".to_string()],
            &types,
            &[BufferShare::Shared],
            "total",
            "python",
        )
        .expect("plans")
        .len(),
        1,
        "and the elements cross as a view instead"
    );
}
