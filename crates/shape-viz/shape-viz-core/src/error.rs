//! Error types for FChart Core

use thiserror::Error;

/// Main error type for FChart operations
#[derive(Error, Debug)]
pub enum ChartError {
    /// GPU/Rendering related errors
    #[error("GPU error: {0}")]
    Gpu(#[from] wgpu::RequestDeviceError),

    #[error("GPU surface error: {0}")]
    Surface(#[from] wgpu::SurfaceError),

    #[error("GPU device lost")]
    DeviceLost,

    #[error("GPU out of memory")]
    OutOfMemory,

    /// Data related errors
    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Empty dataset")]
    EmptyData,

    #[error("Data range error: {message}")]
    DataRange { message: String },

    /// Rendering errors
    #[error("Shader compilation failed: {0}")]
    ShaderCompilation(String),

    #[error("Texture creation failed: {0}")]
    TextureCreation(String),

    #[error("Buffer creation failed: {0}")]
    BufferCreation(String),

    /// Configuration errors
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),

    /// Layer system errors
    #[error("Layer error: {0}")]
    Layer(String),

    #[error("Layer not found: {0}")]
    LayerNotFound(String),

    /// Text rendering errors
    #[error("Text rendering error: {0}")]
    TextRendering(String),

    #[error("Font loading error: {0}")]
    FontLoading(String),

    /// I/O and format errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Generic errors
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

/// Convenience type alias for Results with ChartError
pub type Result<T> = std::result::Result<T, ChartError>;

impl ChartError {
    /// Create a new data range error
    pub fn data_range(message: impl Into<String>) -> Self {
        Self::DataRange {
            message: message.into(),
        }
    }

    /// Create a new invalid configuration error
    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::InvalidConfig(message.into())
    }

    /// Create a new layer error
    pub fn layer(message: impl Into<String>) -> Self {
        Self::Layer(message.into())
    }

    /// Create a new internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

// GPU-specific error conversions
impl From<wgpu::BufferAsyncError> for ChartError {
    fn from(_: wgpu::BufferAsyncError) -> Self {
        Self::internal("GPU buffer operation failed")
    }
}

// Text rendering error conversion is handled conditionally at compile time
// #[cfg(feature = "text-rendering")]
// impl From<cosmic_text::Error> for ChartError {
//     fn from(err: cosmic_text::Error) -> Self {
//         Self::TextRendering(err.to_string())
//     }
// }
