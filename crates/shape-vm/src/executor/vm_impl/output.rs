use super::super::*;
use shape_value::VMError;

impl VirtualMachine {
    /// Enable output capture for testing
    /// When enabled, print output goes to an internal buffer instead of stdout
    pub fn enable_output_capture(&mut self) {
        self.output_buffer = Some(Vec::new());
    }

    /// Disable output capture — print goes to stdout again.
    pub fn disable_output_capture(&mut self) {
        self.output_buffer = None;
    }

    /// Get captured output (returns empty vec if capture not enabled)
    pub fn get_captured_output(&self) -> Vec<String> {
        self.output_buffer.clone().unwrap_or_default()
    }

    /// Clear captured output
    pub fn clear_captured_output(&mut self) {
        if let Some(ref mut buf) = self.output_buffer {
            buf.clear();
        }
    }

    /// Write to output (either buffer or stdout)
    #[allow(dead_code)]
    pub(crate) fn write_output(&mut self, text: &str) {
        if let Some(ref mut buf) = self.output_buffer {
            buf.push(text.to_string());
        } else {
            println!("{}", text);
        }
    }

    /// Record + log a CONTAINED drop error (WF-1C fix (c), ADR-006 §2.7.30).
    ///
    /// Called from `op_drop_call_impl` when a user `Drop::drop` body aborts
    /// with a `VMError`. The error is not propagated (that would abort the
    /// whole program and skip the remaining scope-exit drops + lose the
    /// return value); instead it is appended to the [`drop_errors`] sink and
    /// echoed to stderr so a shipped run still surfaces the failure without
    /// tearing down the VM.
    ///
    /// [`drop_errors`]: VirtualMachine::drop_errors
    pub(crate) fn record_contained_drop_error(
        &mut self,
        type_name: Option<&str>,
        err: &VMError,
    ) {
        let msg = match type_name {
            Some(tn) => format!("drop() for `{tn}` failed and was contained: {err}"),
            None => format!("drop() failed and was contained: {err}"),
        };
        eprintln!("shape: warning: {msg}");
        self.drop_errors.push(msg);
    }

    /// Contained drop-error diagnostics accumulated so far (WF-1C fix (c)).
    /// Empty unless a user `Drop::drop` body raised a runtime error that the
    /// VM contained. Read-only view; use [`take_drop_errors`] to drain.
    ///
    /// [`take_drop_errors`]: VirtualMachine::take_drop_errors
    pub fn drop_errors(&self) -> &[String] {
        &self.drop_errors
    }

    /// Drain the contained drop-error diagnostics (WF-1C fix (c)).
    pub fn take_drop_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.drop_errors)
    }

    /// Set a module_binding variable by name.
    ///
    /// Phase-1b-vm: the storage-tier parallel-kind track for module
    /// bindings has landed (`module_binding_kinds` companion vec, ADR-
    /// 006 §2.7.8 / Q10) and `module_binding_write_kinded` is the
    /// kinded implementation backbone for any future host-API mutator.
    /// The signature is now kinded per ADR-006 §2.7 / Q7 — the
    /// `KindedSlot` carrier is the boundary shape.
    #[allow(dead_code)]
    pub(crate) fn set_module_binding_by_name_nb(&mut self, _name: &str, _value: KindedSlot) {
        todo!(
            "phase-2c — see ADR-006 §2.7.4: set_module_binding_by_name_nb \
             dispatch through `module_binding_write_kinded` (§2.7.8 \
             parallel track is live); host-API caller wiring lands in \
             the Phase-2c host rebuild"
        );
    }

    /// Get the line number of the last error (for LSP integration)
    pub fn last_error_line(&self) -> Option<u32> {
        self.last_error_line
    }

    /// Get the file path of the last error (for LSP integration)
    pub fn last_error_file(&self) -> Option<&str> {
        self.last_error_file.as_deref()
    }

    /// Capture an uncaught exception payload for host-side rendering.
    ///
    /// Per ADR-006 §2.7 / Q7 the boundary carrier is `KindedSlot`.
    pub(crate) fn set_last_uncaught_exception(&mut self, value: KindedSlot) {
        self.last_uncaught_exception = Some(value);
    }

    /// Clear any previously captured uncaught exception payload.
    pub(crate) fn clear_last_uncaught_exception(&mut self) {
        self.last_uncaught_exception = None;
    }

    /// Take the last uncaught exception payload if present.
    pub fn take_last_uncaught_exception(&mut self) -> Option<KindedSlot> {
        self.last_uncaught_exception.take()
    }
}
