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
    pub(crate) fn set_module_binding_by_name_nb(&mut self, name: &str, value: ValueWord) {
        if let Some(idx) = self
            .program
            .module_binding_names
            .iter()
            .position(|n| n == name)
        {
            if idx < self.module_bindings.len() {
                // BARRIER: heap write site — overwrites module binding by name
                self.binding_write_raw(idx, value);
            } else {
                self.module_bindings.resize_with(idx + 1, || Self::NONE_BITS);
                // BARRIER: heap write site — overwrites module binding by name (after resize)
                self.binding_write_raw(idx, value);
            }
        }
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
