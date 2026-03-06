//! Async/Sync bridge for Shape runtime
//!
//! Provides a shared Tokio runtime for async operations in Shape.
//! This simplified version focuses on managing the runtime for market-data
//! provider access so legacy synchronous callers can still execute queries.

use shape_ast::error::{Result, ShapeError};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use tokio::runtime::{Handle, Runtime};

/// Global shared Tokio runtime for Shape
static SHARED_RUNTIME: OnceLock<Arc<Runtime>> = OnceLock::new();

/// Initialize the shared runtime (called once at startup)
pub fn initialize_shared_runtime() -> Result<()> {
    if SHARED_RUNTIME.get().is_some() {
        return Ok(()); // Already initialized
    }

    let runtime = Runtime::new().map_err(|e| ShapeError::RuntimeError {
        message: format!("Failed to create shared Tokio runtime: {}", e),
        location: None,
    })?;

    SHARED_RUNTIME
        .set(Arc::new(runtime))
        .map_err(|_| ShapeError::RuntimeError {
            message: "Failed to set shared runtime (race condition)".to_string(),
            location: None,
        })?;

    Ok(())
}

/// Get the shared runtime handle
pub fn get_runtime_handle() -> Result<Handle> {
    let runtime = SHARED_RUNTIME
        .get()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Shared runtime not initialized. Call initialize_shared_runtime() first."
                .to_string(),
            location: None,
        })?;

    Ok(runtime.handle().clone())
}

/// Block on a future using the shared runtime
///
/// This is safe to call from synchronous contexts as it properly handles
/// nested runtime calls by using the handle instead of entering the runtime.
pub fn block_on_shared<F, T>(future: F) -> Result<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = get_runtime_handle()?;

    // Check if we're already in a Tokio runtime
    let result = if let Ok(_handle_in_runtime) = Handle::try_current() {
        // We're in a runtime, use spawn_blocking to avoid blocking the runtime
        let (tx, rx) = std::sync::mpsc::channel();
        handle.spawn(async move {
            let result = future.await;
            let _ = tx.send(result);
        });
        rx.recv().map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to receive async result: {}", e),
            location: None,
        })?
    } else {
        // Not in a runtime, safe to block directly
        handle.block_on(future)
    };

    Ok(result)
}

/// Deprecated: SyncDataProvider removed
///
/// Use the async data architecture (Phase 6) instead.
/// Legacy code should migrate to ExecutionContext::prefetch_data().
#[derive(Clone)]
pub struct SyncDataProvider {
    _placeholder: std::marker::PhantomData<()>,
}
