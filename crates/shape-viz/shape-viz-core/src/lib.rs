//! # Shape Viz Core
//!
//! High-performance GPU-accelerated charting library with modular layer architecture.
//!
//! ## Architecture Overview
//!
//! Shape Viz Core is built around a modular layer system where each visual component
//! (range bars, grid, axes, indicators) is implemented as a separate layer that
//! can be composed together to create complex charts.
//!
//! ### Core Components
//!
//! - **Renderer**: GPU-accelerated rendering engine using WGPU
//! - **Layers**: Modular rendering components (range bars, grid, axes, etc.)
//! - **Viewport**: Coordinate system and transformation management
//! - **Data**: Efficient data structures for generic time series
//! - **Themes**: Professional styling and color schemes
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use shape_viz_core::{Chart, ChartConfig, ChartData, Series};
//! use shape_viz_core::layers::{CandlestickLayer, GridLayer, PriceAxisLayer};
//! use std::any::Any;
//!
//! #[derive(Debug)]
//! struct SimpleSeries {
//!     name: String,
//!     x: Vec<f64>,
//!     y: Vec<f64>,
//! }
//!
//! impl Series for SimpleSeries {
//!     fn name(&self) -> &str { &self.name }
//!     fn len(&self) -> usize { self.x.len() }
//!     fn get_x(&self, index: usize) -> f64 { self.x[index] }
//!     fn get_y(&self, index: usize) -> f64 { self.y[index] }
//!     fn get_x_range(&self) -> (f64, f64) {
//!         (*self.x.first().unwrap(), *self.x.last().unwrap())
//!     }
//!     fn get_y_range(&self, _x_min: f64, _x_max: f64) -> (f64, f64) {
//!         let min = self.y.iter().cloned().fold(f64::INFINITY, f64::min);
//!         let max = self.y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
//!         (min, max)
//!     }
//!     fn find_index(&self, x: f64) -> Option<usize> {
//!         self.x.iter().position(|v| *v == x)
//!     }
//!     fn as_any(&self) -> &dyn Any { self }
//! }
//!
//! let config = ChartConfig::default();
//! let mut chart = pollster::block_on(Chart::new(config))?;
//!
//! // Add layers
//! chart.add_layer(Box::new(GridLayer::new()));
//! chart.add_layer(Box::new(CandlestickLayer::new()));
//! chart.add_layer(Box::new(PriceAxisLayer::new()));
//!
//! // Attach data
//! let series = SimpleSeries {
//!     name: "demo".to_string(),
//!     x: vec![1.0, 2.0, 3.0],
//!     y: vec![10.0, 12.0, 11.0],
//! };
//! chart.set_data(ChartData::new(Box::new(series)))?;
//!
//! // Render frame
//! let image_data = pollster::block_on(chart.render())?;
//! # let _ = image_data;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod chart;
pub mod data;
pub mod error;
pub mod event;
pub mod layers;
pub mod renderer;
pub mod style;
pub mod theme;
pub mod utils;
pub mod viewport;
pub mod wire;

#[cfg(feature = "text-rendering")]
pub mod text_gpu;
#[cfg(feature = "text-rendering")]
pub use text_gpu as text;

// Re-export commonly used types
pub use chart::{Chart, ChartConfig};
pub use data::{ChartData, RangeSeries, Series, TimeRange};
pub use error::{ChartError, Result};
pub use event::{ChartEvent, ChartState};
pub use glam::Vec2;
pub use renderer::{GpuRenderer, RenderContext};
pub use style::{ChartStyle, LayoutStyle};
pub use theme::{ChartTheme, ColorScheme};
pub use viewport::{ChartBounds, Rect, Viewport};

// Re-export layer trait for external layer implementations
pub use layers::Layer;
