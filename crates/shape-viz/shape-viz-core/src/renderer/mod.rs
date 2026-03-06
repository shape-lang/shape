//! GPU-accelerated rendering system using WGPU
//!
//! This module provides a modular rendering system with the following components:
//!
//! - **vertex**: Vertex data structures and GPU buffers
//! - **context**: Rendering context with drawing primitives
//! - **shaders**: Shader compilation and uniform structures
//! - **gpu_renderer**: Main GPU renderer implementation

mod context;
mod gpu_renderer;
mod shaders;
mod vertex;

// Re-export all public types for backward compatibility
pub use context::RenderContext;
pub use gpu_renderer::GpuRenderer;
pub use shaders::{ScreenUniforms, create_render_pipeline};
pub use vertex::{Vertex, VertexBuffer};
