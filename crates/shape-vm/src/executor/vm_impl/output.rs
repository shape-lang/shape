use super::super::*;

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
    pub(crate) fn write_output(&mut self, text: &str) {
        if let Some(ref mut buf) = self.output_buffer {
            buf.push(text.to_string());
        } else {
            println!("{}", text);
        }
    }

    /// Set a module_binding variable by name using ValueWord directly.
    ///
    /// Phase-1b-vm: the storage-tier parallel-kind track for module
    /// bindings has landed (`module_binding_kinds` companion vec, ADR-
    /// 006 §2.7.8 / Q10) and `module_binding_write_kinded` is the
    /// kinded implementation backbone for any future host-API mutator.
    /// This particular method, however, still has a deleted-carrier
    /// `ValueWord` parameter type — the kind cannot be sourced from a
    /// `ValueWord` (the carrier itself is deleted, CLAUDE.md
    /// "Forbidden Patterns"), so re-lighting requires the host-API
    /// signature flip to a kinded shape (e.g. `(name, KindedSlot)` or
    /// `(name, bits, NativeKind)`). That signature flip is
    /// host-boundary work, scheduled with the Phase-2c host-API
    /// rebuild per ADR-006 §2.7.4.
    pub(crate) fn set_module_binding_by_name_nb(&mut self, _name: &str, _value: ValueWord) {
        todo!(
            "phase-2c — see ADR-006 §2.7.4: set_module_binding_by_name_nb \
             still takes a `ValueWord` (deleted carrier); flip the host- \
             API signature to a kinded shape, then dispatch through \
             `module_binding_write_kinded` (§2.7.8 parallel track is \
             live)"
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
    pub(crate) fn set_last_uncaught_exception(&mut self, value: ValueWord) {
        self.last_uncaught_exception = Some(value);
    }

    /// Clear any previously captured uncaught exception payload.
    pub(crate) fn clear_last_uncaught_exception(&mut self) {
        self.last_uncaught_exception = None;
    }

    /// Take the last uncaught exception payload if present.
    pub fn take_last_uncaught_exception(&mut self) -> Option<ValueWord> {
        self.last_uncaught_exception.take()
    }
}
