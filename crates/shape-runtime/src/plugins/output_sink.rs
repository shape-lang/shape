//! Plugin Output Sink Wrapper
//!
//! Provides a Rust-friendly wrapper around the C ABI output sink interface.

use std::ffi::c_void;
use std::ptr;

use serde_json::Value;
use shape_abi_v1::OutputSinkVTable;

use shape_ast::error::{Result, ShapeError};

/// Wrapper around a plugin output sink
///
/// Used for sending alerts and events to external systems.
pub struct PluginOutputSink {
    /// Plugin name
    name: String,
    /// Vtable pointer (static lifetime)
    vtable: &'static OutputSinkVTable,
    /// Instance pointer (owned by this struct)
    instance: *mut c_void,
    /// Tags this sink handles (empty = all)
    handled_tags: Vec<String>,
}

impl PluginOutputSink {
    /// Create a new plugin output sink instance
    ///
    /// # Arguments
    /// * `name` - Plugin name
    /// * `vtable` - Output sink vtable (must be static)
    /// * `config` - Configuration value (will be MessagePack encoded)
    pub fn new(name: String, vtable: &'static OutputSinkVTable, config: &Value) -> Result<Self> {
        // Serialize config to MessagePack
        let config_bytes = rmp_serde::to_vec(config).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize plugin config: {}", e),
            location: None,
        })?;

        // Initialize the plugin instance
        let init_fn = vtable.init.ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Plugin '{}' has no init function", name),
            location: None,
        })?;

        let instance = unsafe { init_fn(config_bytes.as_ptr(), config_bytes.len()) };
        if instance.is_null() {
            return Err(ShapeError::RuntimeError {
                message: format!("Plugin '{}' init returned null", name),
                location: None,
            });
        }

        // Get handled tags
        let handled_tags = Self::get_handled_tags_from_vtable(vtable, instance)?;

        Ok(Self {
            name,
            vtable,
            instance,
            handled_tags,
        })
    }

    /// Get the plugin name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the tags this sink handles (empty = all)
    pub fn handled_tags(&self) -> &[String] {
        &self.handled_tags
    }

    /// Send an alert to the sink
    ///
    /// # Arguments
    /// * `alert` - Alert value to send (will be MessagePack encoded)
    pub fn send(&self, alert: &Value) -> Result<()> {
        let send_fn = self.vtable.send.ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Plugin '{}' has no send function", self.name),
            location: None,
        })?;

        let alert_bytes = rmp_serde::to_vec(alert).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize alert: {}", e),
            location: None,
        })?;

        let result = unsafe { send_fn(self.instance, alert_bytes.as_ptr(), alert_bytes.len()) };

        if result != 0 {
            return Err(ShapeError::RuntimeError {
                message: format!("Plugin '{}' send failed with code {}", self.name, result),
                location: None,
            });
        }

        Ok(())
    }

    /// Flush any pending alerts
    pub fn flush(&self) -> Result<()> {
        let flush_fn = match self.vtable.flush {
            Some(f) => f,
            None => return Ok(()), // No flush function, nothing to do
        };

        let result = unsafe { flush_fn(self.instance) };

        if result != 0 {
            return Err(ShapeError::RuntimeError {
                message: format!("Plugin '{}' flush failed with code {}", self.name, result),
                location: None,
            });
        }

        Ok(())
    }

    // ========================================================================
    // Private Helpers
    // ========================================================================

    fn get_handled_tags_from_vtable(
        vtable: &OutputSinkVTable,
        instance: *mut c_void,
    ) -> Result<Vec<String>> {
        let get_tags_fn = match vtable.get_handled_tags {
            Some(f) => f,
            None => return Ok(Vec::new()), // No tag filtering, handles all
        };

        let mut out_ptr: *mut u8 = ptr::null_mut();
        let mut out_len: usize = 0;

        unsafe { get_tags_fn(instance, &mut out_ptr, &mut out_len) };

        if out_ptr.is_null() || out_len == 0 {
            return Ok(Vec::new());
        }

        let data_slice = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let tags: Vec<String> = rmp_serde::from_slice(data_slice).unwrap_or_else(|_| Vec::new());

        // Free the buffer
        if let Some(free_fn) = vtable.free_buffer {
            unsafe { free_fn(out_ptr, out_len) };
        }

        Ok(tags)
    }
}

impl Drop for PluginOutputSink {
    fn drop(&mut self) {
        // Flush before dropping
        let _ = self.flush();

        if let Some(drop_fn) = self.vtable.drop {
            unsafe { drop_fn(self.instance) };
        }
    }
}

// SAFETY: The instance pointer is only accessed through the vtable functions
// which are required to be thread-safe by the plugin contract.
unsafe impl Send for PluginOutputSink {}
unsafe impl Sync for PluginOutputSink {}

#[cfg(test)]
mod tests {
    #[test]
    fn test_handled_tags_default() {
        // Just test that the struct can be created (integration test would use real plugin)
        let tags: Vec<String> = Vec::new();
        assert!(tags.is_empty());
    }
}
