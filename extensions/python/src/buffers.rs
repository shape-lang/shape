//! Zero-copy buffer sharing for the Python runtime (ADR-019 §2 / #199).
//!
//! A `shared` Shape parameter reaches the body as a `memoryview` over the
//! caller's own array, not as a copied `list`. This module is the Python half
//! of that: the capability block the host negotiates against, the invoke that
//! substitutes views for arguments, and the release accounting that decides
//! whether the host may reclaim the memory afterwards.
//!
//! # The two guarantees, and the CPython facts they rest on
//!
//! **Immutability is prevented, not detected.** A `shared` view is built with
//! `PyBUF_READ`, so CPython itself raises `TypeError: cannot modify read-only
//! memory` on any write through it, before a byte moves. There is no host check
//! to bypass and no window in which a stray write lands and is noticed later.
//! `shared mut` uses `PyBUF_WRITE` and the writes go straight into the caller's
//! array — that is the point of the mode.
//!
//! **Retention is detected through PEP 3118's export count, on an exporter this
//! module owns.** The danger the ticket names is a body that keeps a view:
//! `numpy.asarray(xs)` appended to a module global needs no vtable re-entry, and
//! the host would then reclaim memory foreign code still points at.
//!
//! The accounting hangs off [`ShapeBuffer`], a small object implementing the
//! buffer protocol over the host's pointer. Every `memoryview` the body can
//! reach — the one it was passed, a `cast` of it, a slice of it, or a numpy
//! array over it — ultimately holds ONE buffer export from that object, and
//! CPython calls `bf_releasebuffer` exactly when the last of them goes away. So
//! after the call this module releases the views it created and reads the
//! export count: zero means nothing anywhere still points at the buffer, and
//! anything else names a view the host must not reclaim under.
//!
//! Two designs were tried before this one, and both were wrong in ways worth
//! recording, because each looked right:
//!
//! - **the view's reference count.** Catches a stashed `memoryview` object and
//!   nothing else, and produces a false positive on any body that RAISES —
//!   CPython keeps the argument alive in the traceback, so an ordinary Python
//!   exception would have been reported as a memory-safety failure.
//! - **`memoryview.release()` raising `BufferError`.** Catches a C consumer
//!   holding a buffer, and misses `xs[0:2]`: a slice shares the managed buffer
//!   without registering an export on the view it came from, so the release
//!   SUCCEEDS while the slice keeps the pointer. That is a live hazard reported
//!   as clean.
//!
//! The export count catches all three, because it is the thing CPython itself
//! uses to decide when the memory stops being needed. `release()` is still
//! called on every view — not as the signal, but so that a stashed view is a
//! POISONED view: a later read through it raises `ValueError: operation
//! forbidden on released memoryview` rather than reading reclaimed memory.
//!
//! A false positive here costs a failed call with an explanatory error. A false
//! negative costs memory corruption in an unrelated part of the program, later.
//! Where the two designs above had to be chosen between, that asymmetry decided
//! it.

use shape_abi_v1::{
    BUFFER_ELEM_FLOAT64, BUFFER_ELEM_INT64, BUFFER_MODE_SHARED, BUFFER_MODE_SHARED_MUT,
    BufferCapability, EXTENSION_CAPABILITIES_VERSION, ExtensionCapabilities, ForeignBufferView,
    PluginError,
};
use std::ffi::c_void;

use crate::runtime::PythonRuntime;

/// The buffer capability this runtime implements.
///
/// Both modes, both element types, and — the entry whose absence would disable
/// the capability entirely — real release accounting.
static BUFFERS: BufferCapability = BufferCapability {
    struct_size: std::mem::size_of::<BufferCapability>() as u32,
    modes: BUFFER_MODE_SHARED | BUFFER_MODE_SHARED_MUT,
    elem_types: (1 << BUFFER_ELEM_INT64) | (1 << BUFFER_ELEM_FLOAT64),
    _reserved: 0,
    invoke_with_buffers: Some(python_invoke_with_buffers),
    outstanding_exports: Some(python_outstanding_exports),
};

static CAPABILITIES: ExtensionCapabilities = ExtensionCapabilities {
    struct_size: std::mem::size_of::<ExtensionCapabilities>() as u32,
    version: EXTENSION_CAPABILITIES_VERSION,
    buffers: &BUFFERS,
};

/// This extension's optional-protocol block, for the
/// `language_runtime_plugin!` macro's `capabilities:` slot.
pub fn python_capabilities() -> *const ExtensionCapabilities {
    &CAPABILITIES
}

/// The `struct` module format character CPython casts a raw byte view to.
///
/// `PyMemoryView_FromMemory` always produces an unsigned-byte, itemsize-1 view;
/// casting it gives the body a properly typed and shaped `memoryview`, so
/// `xs[3]` is the fourth number rather than the fourth byte of the first.
pub(crate) fn format_char(elem_type: u32) -> Option<&'static str> {
    match elem_type {
        BUFFER_ELEM_INT64 => Some("q"),
        BUFFER_ELEM_FLOAT64 => Some("d"),
        _ => None,
    }
}

/// A buffer-protocol exporter over one host buffer (ADR-019 §2 / #199).
///
/// The body never sees this object — it sees a `memoryview` over it. It exists
/// so that CPython's own export accounting has somewhere to live: every view
/// reachable from the body, however it was derived, holds one export from here,
/// and `bf_releasebuffer` fires exactly when the last one goes.
///
/// `exports` is an `Arc` rather than a plain field so the count survives being
/// read after the Python object is gone, and so the reader does not have to
/// borrow a `#[pyclass]` while CPython may still be touching it.
#[cfg(feature = "pyo3")]
#[pyo3::pyclass]
pub(crate) struct ShapeBuffer {
    data: usize,
    byte_len: usize,
    readonly: bool,
    exports: std::sync::Arc<std::sync::atomic::AtomicIsize>,
}

#[cfg(feature = "pyo3")]
#[pyo3::pymethods]
impl ShapeBuffer {
    /// # Safety
    ///
    /// CPython calls this with a `Py_buffer` it owns. The pointer this fills in
    /// is the host's, valid for the duration of the Shape call that created
    /// this object — which is the contract `invoke_with_views` documents and
    /// the host upholds by holding the caller's share of the array.
    unsafe fn __getbuffer__(
        slf: pyo3::Bound<'_, Self>,
        view: *mut pyo3::ffi::Py_buffer,
        flags: std::ffi::c_int,
    ) -> pyo3::PyResult<()> {
        let (data, byte_len, readonly, exports) = {
            let me = slf.borrow();
            (
                me.data,
                me.byte_len,
                me.readonly,
                std::sync::Arc::clone(&me.exports),
            )
        };
        // `PyBuffer_FillInfo` is what makes the read-only mode real: asked for a
        // writable buffer over a read-only export, it sets `BufferError` and
        // returns -1, so a `shared` view refuses at the point of export rather
        // than at the first write.
        let rc = unsafe {
            pyo3::ffi::PyBuffer_FillInfo(
                view,
                slf.as_ptr(),
                // A zero-length buffer still needs a non-null, aligned pointer:
                // CPython treats null as an error, and an empty Shape array is
                // not one.
                if byte_len == 0 {
                    std::ptr::NonNull::<u8>::dangling().as_ptr() as *mut std::ffi::c_void
                } else {
                    data as *mut std::ffi::c_void
                },
                byte_len as pyo3::ffi::Py_ssize_t,
                readonly as std::ffi::c_int,
                flags,
            )
        };
        if rc != 0 {
            return Err(pyo3::PyErr::fetch(slf.py()));
        }
        exports.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }

    /// # Safety
    ///
    /// Called by CPython with the `Py_buffer` a matching `__getbuffer__` filled.
    unsafe fn __releasebuffer__(&self, _view: *mut pyo3::ffi::Py_buffer) {
        self.exports
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// One host buffer lent to a foreign body for the duration of one call.
///
/// Owns the exporter, the `memoryview` over it and the typed cast the body
/// actually receives, and knows how to take them all back afterwards.
#[cfg(feature = "pyo3")]
pub(crate) struct LentView<'py> {
    /// The exporter's own export count — the only signal that sees every way a
    /// body can keep the memory.
    exports: std::sync::Arc<std::sync::atomic::AtomicIsize>,
    /// The raw `memoryview`, itemsize 1.
    base: pyo3::Bound<'py, pyo3::PyAny>,
    /// The element-typed view handed to the body.
    cast: pyo3::Bound<'py, pyo3::PyAny>,
}

#[cfg(feature = "pyo3")]
impl<'py> LentView<'py> {
    /// Export `view`'s buffer as a typed `memoryview`.
    ///
    /// # Safety
    ///
    /// `view.data` must address `view.len` correctly-aligned elements of
    /// `view.elem_type` that stay valid until [`Self::reclaim`] has run.
    pub(crate) unsafe fn new(
        py: pyo3::Python<'py>,
        view: &ForeignBufferView,
    ) -> Result<Self, String> {
        use pyo3::prelude::*;

        let Some(format) = format_char(view.elem_type) else {
            return Err(format!(
                "python runtime: element type {} has no memoryview format",
                view.elem_type
            ));
        };
        let elem_size = shape_abi_v1::buffer_elem_size(view.elem_type);
        let byte_len = (view.len as usize).saturating_mul(elem_size);
        let exports = std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(0));

        let exporter = Bound::new(
            py,
            ShapeBuffer {
                data: view.data as usize,
                byte_len,
                // The immutability guarantee, made by CPython rather than
                // checked by us: a read-only export refuses a writable request
                // inside `PyBuffer_FillInfo`, and refuses every write through
                // the resulting view.
                readonly: view.mode != BUFFER_MODE_SHARED_MUT,
                exports: std::sync::Arc::clone(&exports),
            },
        )
        .map_err(|e| format!("python runtime: could not build the buffer exporter: {e}"))?;

        // `PyMemoryView::from` is the export: it calls the exporter's
        // `__getbuffer__`, which is where the count this whole design rests on
        // is incremented.
        let base = pyo3::types::PyMemoryView::from(exporter.as_any())
            .map(|mv| mv.into_any())
            .map_err(|e| {
                format!("python runtime: could not build a memoryview over the shared buffer: {e}")
            })?;

        // itemsize 1 out of `PyBuffer_FillInfo`; the cast is what makes `xs[3]`
        // the fourth NUMBER rather than the fourth byte of the first.
        let cast = base
            .call_method1("cast", (format,))
            .map_err(|e| format!("python runtime: memoryview cast failed: {e}"))?;

        Ok(LentView {
            exports,
            base,
            cast,
        })
    }

    /// The object the body binds at this view's argument position.
    pub(crate) fn for_body(&self) -> pyo3::Py<pyo3::PyAny> {
        self.cast.clone().unbind()
    }

    /// Take the view back, and report whether anything still holds the buffer.
    ///
    /// Releases in derived-first order — the cast holds an export of the base —
    /// so a failure to release means the BODY kept something, not that this
    /// function released in the wrong order. Errors are deliberately swallowed:
    /// they are a symptom, and the export count read afterwards is the verdict.
    ///
    /// Returns `true` when the host must not reclaim the memory.
    pub(crate) fn reclaim(&self) -> bool {
        use pyo3::prelude::*;

        let _ = self.cast.call_method0("release");
        let _ = self.base.call_method0("release");
        self.exports.load(std::sync::atomic::Ordering::Acquire) > 0
    }
}

/// ADR-019 §2 (#199): invoke with `views` substituted for the argument
/// positions they name.
///
/// # Safety
///
/// `instance` must be a live `PythonRuntime`, `handle` one of its compiled
/// functions, and the `views` array must describe `views_len` buffers whose
/// memory stays valid until this returns. The host upholds the last of those by
/// holding the calling Shape frame's own share of each array for the whole call.
pub unsafe extern "C" fn python_invoke_with_buffers(
    instance: *mut c_void,
    handle: *mut c_void,
    args_msgpack: *const u8,
    args_len: usize,
    views: *const ForeignBufferView,
    views_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if instance.is_null() || out_ptr.is_null() || out_len.is_null() {
        return PluginError::InvalidArgument as i32;
    }
    let runtime = unsafe { &*(instance as *const PythonRuntime) };
    let args_slice = if args_msgpack.is_null() || args_len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(args_msgpack, args_len) }
    };
    let views_slice = if views.is_null() || views_len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(views, views_len) }
    };

    // Unwinding across the C ABI is undefined behaviour, so this entry point
    // contains its own panic exactly as the `language_runtime_plugin!` shells do
    // for the vtable's own slots. This one is not generated by the macro — it
    // hangs off the capability block — so the containment is written here.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.invoke_with_views(handle, args_slice, views_slice)
    }));

    match outcome {
        Ok(Ok(mut bytes)) => {
            let len = bytes.len();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            PluginError::Success as i32
        }
        Ok(Err(msg)) => {
            write_error(&msg, out_ptr, out_len);
            PluginError::InternalError as i32
        }
        Err(_) => {
            // A panic leaves the retained mask at whatever the invoke recorded
            // before it, which is the conservative reading: the host will refuse
            // if anything was outstanding.
            write_error(
                "python runtime: the buffer invoke panicked",
                out_ptr,
                out_len,
            );
            PluginError::InternalError as i32
        }
    }
}

/// ADR-019 §2 (#199): which views from the last `invoke_with_buffers` were
/// still exported when the body returned.
///
/// # Safety
///
/// `instance` must be a live `PythonRuntime` or null; a null instance answers
/// "every view retained", which refuses rather than reclaims.
pub unsafe extern "C" fn python_outstanding_exports(instance: *mut c_void) -> u64 {
    if instance.is_null() {
        // The host cannot verify anything about an instance it does not have.
        // Reporting "all views retained" is the only safe answer: it refuses the
        // call rather than letting memory be reclaimed on the strength of a
        // reading that never happened.
        return u64::MAX;
    }
    let runtime = unsafe { &*(instance as *const PythonRuntime) };
    runtime.last_retained_views()
}

fn write_error(msg: &str, out_ptr: *mut *mut u8, out_len: *mut usize) {
    let mut bytes = msg.as_bytes().to_vec();
    let len = bytes.len();
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_declares_release_accounting() {
        // The one slot whose absence would disable the capability entirely
        // (ADR-019 §2). If this ever becomes `None`, the host stops offering
        // buffer sharing for Python and every `shared` call starts failing —
        // which is the correct outcome, and this test is why it would be noticed
        // here rather than there.
        assert!(BUFFERS.outstanding_exports.is_some());
        assert!(BUFFERS.invoke_with_buffers.is_some());
    }

    #[test]
    fn the_capability_declares_both_modes_and_both_element_types() {
        assert_ne!(BUFFERS.modes & BUFFER_MODE_SHARED, 0);
        assert_ne!(BUFFERS.modes & BUFFER_MODE_SHARED_MUT, 0);
        assert_ne!(BUFFERS.elem_types & (1 << BUFFER_ELEM_INT64), 0);
        assert_ne!(BUFFERS.elem_types & (1 << BUFFER_ELEM_FLOAT64), 0);
    }

    #[test]
    fn the_block_reports_its_own_size_so_the_host_can_guard_its_reads() {
        assert_eq!(
            CAPABILITIES.struct_size as usize,
            std::mem::size_of::<ExtensionCapabilities>()
        );
        assert_eq!(
            BUFFERS.struct_size as usize,
            std::mem::size_of::<BufferCapability>()
        );
        assert_eq!(CAPABILITIES.version, EXTENSION_CAPABILITIES_VERSION);
    }

    #[test]
    fn every_declared_element_type_has_a_struct_format() {
        // A mode the block advertises and cannot render is a promise the invoke
        // would break at the worst moment — with the host already expecting a
        // view.
        for elem in [BUFFER_ELEM_INT64, BUFFER_ELEM_FLOAT64] {
            if BUFFERS.elem_types & (1 << elem) != 0 {
                assert!(
                    format_char(elem).is_some(),
                    "element type {elem} is advertised but has no memoryview cast format"
                );
            }
        }
    }

    #[test]
    fn a_null_instance_reports_every_view_retained() {
        // Not a defensive no-op: the answer decides whether the host reclaims
        // memory, so the reading that could not happen must read as "do not".
        assert_eq!(
            unsafe { python_outstanding_exports(std::ptr::null_mut()) },
            u64::MAX
        );
    }
}

#[cfg(all(test, feature = "pyo3"))]
mod python_view_tests {
    //! ADR-019 §2 / R25 (POLY-ZERO-COPY, issue #199) — the Python half, against
    //! a real CPython.
    //!
    //! These are the tests the host-side fakes cannot write: whether a write
    //! through a read-only view actually raises, and whether the buffer
    //! protocol's export count actually notices a stashed view. The host's own
    //! behaviour is asserted in `shape-vm`'s `executor::tests::buffer_views`.
    use super::*;
    use crate::runtime::PythonRuntime;
    use shape_abi_v1::ForeignBufferView;

    /// A buffer the test owns for the call's duration, standing in for the
    /// Shape array a caller's slot keeps alive.
    struct HostBuffer(Vec<f64>);

    impl HostBuffer {
        fn view(&mut self, mode: u32) -> ForeignBufferView {
            ForeignBufferView {
                arg_index: 0,
                elem_type: BUFFER_ELEM_FLOAT64,
                mode,
                _reserved: 0,
                len: self.0.len() as u64,
                data: self.0.as_mut_ptr() as *mut c_void,
            }
        }
    }

    fn runtime_with(body: &str, name: &str) -> (PythonRuntime, *mut c_void) {
        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        let handle = runtime
            .compile(
                name,
                body,
                &["xs".to_string()],
                &["Array<number>".to_string()],
                "Result<number>",
                false,
            )
            .expect("compile succeeds");
        (runtime, handle)
    }

    /// The argument array the host sends: one nil, standing in for the view.
    fn nil_args() -> Vec<u8> {
        rmp_serde::to_vec(&vec![rmpv::Value::Nil]).expect("encodes")
    }

    fn as_f64(bytes: &[u8]) -> f64 {
        let value: rmpv::Value = rmp_serde::from_slice(bytes).expect("decodable result");
        value.as_f64().expect("float result")
    }

    #[test]
    fn a_shared_view_reads_the_hosts_own_memory() {
        let (runtime, handle) = runtime_with("return float(sum(xs))", "total");
        let mut buffer = HostBuffer(vec![1.5, 2.5, 3.0]);
        let out = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect("the shared invoke runs");
        assert_eq!(as_f64(&out), 7.0);
        assert_eq!(
            runtime.last_retained_views(),
            0,
            "a body that only reads leaves nothing exported"
        );
    }

    #[test]
    fn the_view_is_typed_not_a_run_of_bytes() {
        // The cast is why `xs[1]` is the second NUMBER. Without it the body sees
        // an itemsize-1 unsigned-byte view and every index is off by a factor of
        // eight — the kind of bug that produces plausible garbage rather than an
        // error.
        let (runtime, handle) = runtime_with(
            "assert xs.format == 'd' and xs.itemsize == 8\nreturn float(len(xs)) + xs[1]",
            "shape_check",
        );
        let mut buffer = HostBuffer(vec![10.0, 20.0, 30.0]);
        let out = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect("the shared invoke runs");
        assert_eq!(as_f64(&out), 23.0, "three elements, second one 20.0");
    }

    #[test]
    fn a_write_through_a_shared_view_is_prevented_by_python_itself() {
        // #199 tripwire (1). Not detected afterwards — REFUSED, by CPython, at
        // the assignment. `PyBUF_READ` makes the memoryview read-only, so there
        // is no host check to bypass and no window in which the write lands.
        let (runtime, handle) = runtime_with("xs[0] = 99.0\nreturn 0.0", "poke");
        let mut buffer = HostBuffer(vec![1.0, 2.0]);
        let err = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect_err("writing through an immutable view raises");
        assert!(
            err.contains("read-only") || err.contains("TypeError"),
            "the failure is Python's own read-only refusal, got: {err}"
        );
        assert_eq!(
            buffer.0,
            vec![1.0, 2.0],
            "and the host's buffer is untouched"
        );
    }

    #[test]
    fn a_write_through_a_shared_mut_view_lands_in_the_hosts_buffer() {
        let (runtime, handle) = runtime_with(
            "for i in range(len(xs)):\n    xs[i] = xs[i] * 2.0\nreturn 0.0",
            "double",
        );
        let mut buffer = HostBuffer(vec![1.0, 2.0, 3.0]);
        runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED_MUT)])
            .expect("the mutable invoke runs");
        assert_eq!(
            buffer.0,
            vec![2.0, 4.0, 6.0],
            "the body wrote into the host's own memory, with no copy back"
        );
        assert_eq!(runtime.last_retained_views(), 0);
    }

    #[test]
    fn a_body_that_stashes_the_view_object_leaves_it_released_not_dangling() {
        // Keeping the memoryview OBJECT is recoverable: `reclaim` releases it on
        // the way out, which drops its hold on the buffer, so the export count
        // is zero and the host may reclaim. What the body kept is a RELEASED
        // view — the second call below reads through it and gets Python's
        // "operation forbidden on released memoryview" rather than whatever is
        // in the host's memory now.
        //
        // Reporting the stash itself as retained would fail a call that is in
        // fact safe, and would fail every body that merely RAISES too, since
        // CPython keeps the argument alive in the traceback.
        //
        // Both calls go through ONE handle because module state is per-handle
        // (#196), which is exactly what makes the stash survive between them.
        let (runtime, handle) = runtime_with(
            "global KEPT\ntry:\n    KEPT\nexcept NameError:\n    KEPT = xs\n    return 0.0\ntry:\n    KEPT[0]\n    return 1.0\nexcept ValueError:\n    return -1.0",
            "stash",
        );
        let mut buffer = HostBuffer(vec![1.0, 2.0]);
        let first = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect("stashing the object alone is not a boundary failure");
        assert_eq!(as_f64(&first), 0.0, "the first call did the stashing");
        assert_eq!(
            runtime.last_retained_views(),
            0,
            "the view was reclaimed, so nothing points at the buffer"
        );

        let second = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect("the read-back call runs");
        assert_eq!(
            as_f64(&second),
            -1.0,
            "reading through the stashed view raises, rather than reading memory the \
             host has taken back"
        );
    }

    #[test]
    fn a_body_that_leaves_a_derived_view_alive_is_reported_as_retaining_the_buffer() {
        // #199 tripwire (2), and the case that decided this module's design. A
        // memoryview SLICE keeps the buffer without registering an export on the
        // view it came from, so `release()` on that view succeeds and reports
        // nothing while the slice still holds the host's pointer — the same
        // shape as `numpy.asarray(xs)` stashed in a global, and detectable only
        // through the exporter's own count.
        let (runtime, handle) =
            runtime_with("global KEPT\nKEPT = xs[0:2]\nreturn 0.0", "stash_export");
        let mut buffer = HostBuffer(vec![1.0, 2.0, 3.0]);
        let _ = runtime.invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)]);
        assert_eq!(
            runtime.last_retained_views(),
            0b1,
            "a live derived view is a retained buffer, whatever the view object's own \
             accounting says"
        );
    }

    #[test]
    fn a_body_that_hands_the_view_to_a_buffer_consumer_is_reported_too() {
        // The other half of the same class: a consumer holding a BUFFER over the
        // view rather than a derived view. `memoryview(xs)` is the stdlib stand-in
        // for whatever C extension the author actually reached for.
        let (runtime, handle) = runtime_with(
            "global KEPT\nKEPT = memoryview(xs)\nreturn 0.0",
            "stash_consumer",
        );
        let mut buffer = HostBuffer(vec![1.0, 2.0]);
        let _ = runtime.invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)]);
        assert_eq!(runtime.last_retained_views(), 0b1);
    }

    #[test]
    fn an_empty_shared_array_is_an_empty_view_and_not_an_error() {
        let (runtime, handle) = runtime_with("return float(len(xs))", "count");
        let mut buffer = HostBuffer(Vec::new());
        let out = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect("an empty view is not an error");
        assert_eq!(as_f64(&out), 0.0);
        assert_eq!(runtime.last_retained_views(), 0);
    }

    #[test]
    fn a_raising_body_still_produces_a_release_verdict() {
        // #199 tripwire (4): the accounting is not on the success path. A body
        // that raises has still had the view, and the host asks either way.
        let (runtime, handle) = runtime_with("raise ValueError('nope')", "boom");
        let mut buffer = HostBuffer(vec![1.0]);
        let err = runtime
            .invoke_with_views(handle, &nil_args(), &[buffer.view(BUFFER_MODE_SHARED)])
            .expect_err("the body raised");
        assert!(err.contains("nope"), "got: {err}");
        assert_eq!(
            runtime.last_retained_views(),
            0,
            "the view was released despite the exception, so the host may reclaim"
        );
        assert_eq!(buffer.0, vec![1.0], "and the buffer is intact");
    }

    #[test]
    fn the_mask_starts_at_every_view_retained() {
        // A host that asks before any buffer invoke ran gets the answer that
        // refuses, not the answer that reclaims.
        let runtime = PythonRuntime::new(&[]).expect("runtime initializes");
        assert_eq!(runtime.last_retained_views(), u64::MAX);
    }
}
