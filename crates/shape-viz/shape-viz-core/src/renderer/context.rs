//! Rendering context for drawing operations

use crate::error::Result;
use crate::style::ChartStyle;
use crate::theme::{ChartTheme, Color};
use crate::viewport::{Rect, Viewport};
use std::sync::Arc;

use super::vertex::{Vertex, VertexBuffer};

#[cfg(feature = "text-rendering")]
use crate::text::{TextAnchor, TextBaseline, TextItem};

/// Rendering context passed to layers for drawing operations
pub struct RenderContext {
    /// GPU device
    pub device: Arc<wgpu::Device>,
    /// GPU queue
    pub queue: Arc<wgpu::Queue>,
    /// Vertex buffers for batching
    vertex_buffers: Vec<VertexBuffer>,
    /// Current vertices being batched
    current_vertices: Vec<Vertex>,
    /// Maximum vertices per buffer
    max_vertices_per_buffer: u32,
    /// Buffers that contain freshly uploaded geometry awaiting a draw call
    pending_draws: Vec<usize>,
    /// Index of next buffer to use in current frame (prevents reuse before GPU finishes)
    next_buffer_index: usize,
    /// Current viewport
    viewport: Viewport,
    /// Current theme
    theme: ChartTheme,
    /// Current style parameters
    style: ChartStyle,
    /// Text items grouped by stage for proper z-ordering
    #[cfg(feature = "text-rendering")]
    staged_text: std::collections::BTreeMap<i32, Vec<TextItem>>,
    /// Current stage priority for text grouping (derived from LayerStage)
    #[cfg(feature = "text-rendering")]
    current_stage_priority: i32,
}

impl RenderContext {
    /// Create a new render context
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        viewport: Viewport,
        theme: ChartTheme,
        style: ChartStyle,
    ) -> Self {
        Self {
            device,
            queue,
            vertex_buffers: Vec::new(),
            current_vertices: Vec::new(),
            max_vertices_per_buffer: 100_000, // 100K vertices per buffer
            pending_draws: Vec::new(),
            next_buffer_index: 0,
            viewport,
            theme,
            style,
            #[cfg(feature = "text-rendering")]
            staged_text: std::collections::BTreeMap::new(),
            #[cfg(feature = "text-rendering")]
            current_stage_priority: 0,
        }
    }

    /// Set the current stage priority for text grouping
    #[cfg(feature = "text-rendering")]
    pub fn set_stage_priority(&mut self, priority: i32) {
        self.current_stage_priority = priority;
    }

    /// Get the current viewport
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    /// Get the current theme
    pub fn theme(&self) -> &ChartTheme {
        &self.theme
    }

    /// Get the current style
    pub fn style(&self) -> &ChartStyle {
        &self.style
    }

    /// Update viewport and theme
    pub fn update(&mut self, viewport: Viewport, theme: ChartTheme, style: ChartStyle) {
        self.viewport = viewport;
        self.theme = theme;
        self.style = style;
    }

    /// Add vertices to the current batch
    pub fn add_vertices(&mut self, vertices: &[Vertex]) {
        self.current_vertices.extend_from_slice(vertices);
    }

    /// Add a single vertex to the current batch
    pub fn add_vertex(&mut self, vertex: Vertex) {
        self.current_vertices.push(vertex);
    }

    /// Draw a line between two points
    pub fn draw_line(&mut self, start: [f32; 2], end: [f32; 2], color: Color, thickness: f32) {
        let half_thickness = thickness * 0.5;

        // Calculate perpendicular vector for line thickness
        let dx = end[0] - start[0];
        let dy = end[1] - start[1];
        let len = (dx * dx + dy * dy).sqrt();

        if len > 0.0 {
            let nx = -dy / len * half_thickness;
            let ny = dx / len * half_thickness;

            // Create quad for the line
            let vertices = [
                Vertex::new([start[0] + nx, start[1] + ny], color),
                Vertex::new([start[0] - nx, start[1] - ny], color),
                Vertex::new([end[0] + nx, end[1] + ny], color),
                Vertex::new([start[0] - nx, start[1] - ny], color),
                Vertex::new([end[0] - nx, end[1] - ny], color),
                Vertex::new([end[0] + nx, end[1] + ny], color),
            ];

            self.add_vertices(&vertices);
        }
    }

    /// Draw a dashed line between two points
    pub fn draw_dashed_line(
        &mut self,
        start: [f32; 2],
        end: [f32; 2],
        color: Color,
        thickness: f32,
        dash: f32,
        gap: f32,
    ) {
        // Early out for degenerate line
        let dx = end[0] - start[0];
        let dy = end[1] - start[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return;
        }
        // Normalized direction
        let dir_x = dx / len;
        let dir_y = dy / len;
        let mut distance = 0.0;
        let mut current_start = start;
        while distance < len {
            let seg_len = (dash).min(len - distance);
            let current_end = [
                current_start[0] + dir_x * seg_len,
                current_start[1] + dir_y * seg_len,
            ];
            self.draw_line(current_start, current_end, color, thickness);
            // Advance by dash + gap
            distance += dash + gap;
            current_start = [start[0] + dir_x * distance, start[1] + dir_y * distance];
        }
    }

    /// Draw a rectangle
    pub fn draw_rect(&mut self, rect: Rect, color: Color) {
        let x1 = rect.x;
        let y1 = rect.y;
        let x2 = rect.x + rect.width;
        let y2 = rect.y + rect.height;

        let vertices = [
            Vertex::new([x1, y1], color),
            Vertex::new([x2, y1], color),
            Vertex::new([x1, y2], color),
            Vertex::new([x2, y1], color),
            Vertex::new([x2, y2], color),
            Vertex::new([x1, y2], color),
        ];

        self.add_vertices(&vertices);
    }

    /// Draw a filled triangle
    pub fn draw_triangle(&mut self, p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], color: Color) {
        let vertices = [
            Vertex::new(p1, color),
            Vertex::new(p2, color),
            Vertex::new(p3, color),
        ];

        self.add_vertices(&vertices);
    }

    /// Draw a filled circle (approximated with triangles)
    pub fn draw_circle(&mut self, center: [f32; 2], radius: f32, color: Color) {
        const SEGMENTS: u32 = 16;
        let angle_step = std::f32::consts::TAU / SEGMENTS as f32;

        for i in 0..SEGMENTS {
            let angle1 = i as f32 * angle_step;
            let angle2 = (i + 1) as f32 * angle_step;

            let p1 = [
                center[0] + radius * angle1.cos(),
                center[1] + radius * angle1.sin(),
            ];
            let p2 = [
                center[0] + radius * angle2.cos(),
                center[1] + radius * angle2.sin(),
            ];

            self.draw_triangle(center, p1, p2, color);
        }
    }

    /// Draw a filled rounded rectangle
    pub fn draw_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        let r = radius.min(rect.width / 2.0).min(rect.height / 2.0);

        // Center rectangle (full width, minus top/bottom radius)
        self.draw_rect(
            Rect::new(rect.x, rect.y + r, rect.width, rect.height - 2.0 * r),
            color,
        );

        // Top rectangle (between corners)
        self.draw_rect(
            Rect::new(rect.x + r, rect.y, rect.width - 2.0 * r, r),
            color,
        );

        // Bottom rectangle (between corners)
        self.draw_rect(
            Rect::new(
                rect.x + r,
                rect.y + rect.height - r,
                rect.width - 2.0 * r,
                r,
            ),
            color,
        );

        // Four corner quarter-circles
        const SEGMENTS: u32 = 8;
        let angle_step = std::f32::consts::FRAC_PI_2 / SEGMENTS as f32;

        // Top-left corner
        let tl_center = [rect.x + r, rect.y + r];
        for i in 0..SEGMENTS {
            let a1 = std::f32::consts::PI + i as f32 * angle_step;
            let a2 = std::f32::consts::PI + (i + 1) as f32 * angle_step;
            self.draw_triangle(
                tl_center,
                [tl_center[0] + r * a1.cos(), tl_center[1] + r * a1.sin()],
                [tl_center[0] + r * a2.cos(), tl_center[1] + r * a2.sin()],
                color,
            );
        }

        // Top-right corner
        let tr_center = [rect.x + rect.width - r, rect.y + r];
        for i in 0..SEGMENTS {
            let a1 = -std::f32::consts::FRAC_PI_2 + i as f32 * angle_step;
            let a2 = -std::f32::consts::FRAC_PI_2 + (i + 1) as f32 * angle_step;
            self.draw_triangle(
                tr_center,
                [tr_center[0] + r * a1.cos(), tr_center[1] + r * a1.sin()],
                [tr_center[0] + r * a2.cos(), tr_center[1] + r * a2.sin()],
                color,
            );
        }

        // Bottom-right corner
        let br_center = [rect.x + rect.width - r, rect.y + rect.height - r];
        for i in 0..SEGMENTS {
            let a1 = i as f32 * angle_step;
            let a2 = (i + 1) as f32 * angle_step;
            self.draw_triangle(
                br_center,
                [br_center[0] + r * a1.cos(), br_center[1] + r * a1.sin()],
                [br_center[0] + r * a2.cos(), br_center[1] + r * a2.sin()],
                color,
            );
        }

        // Bottom-left corner
        let bl_center = [rect.x + r, rect.y + rect.height - r];
        for i in 0..SEGMENTS {
            let a1 = std::f32::consts::FRAC_PI_2 + i as f32 * angle_step;
            let a2 = std::f32::consts::FRAC_PI_2 + (i + 1) as f32 * angle_step;
            self.draw_triangle(
                bl_center,
                [bl_center[0] + r * a1.cos(), bl_center[1] + r * a1.sin()],
                [bl_center[0] + r * a2.cos(), bl_center[1] + r * a2.sin()],
                color,
            );
        }
    }

    /// Draw a filled rectangle with border
    pub fn draw_rect_with_border(
        &mut self,
        rect: Rect,
        fill_color: Color,
        border_color: Color,
        border_width: f32,
    ) {
        // Draw fill
        self.draw_rect(rect, fill_color);

        // Draw border
        let half_border = border_width * 0.5;

        // Top border
        self.draw_rect(
            Rect::new(
                rect.x - half_border,
                rect.y - half_border,
                rect.width + border_width,
                border_width,
            ),
            border_color,
        );

        // Bottom border
        self.draw_rect(
            Rect::new(
                rect.x - half_border,
                rect.y + rect.height - half_border,
                rect.width + border_width,
                border_width,
            ),
            border_color,
        );

        // Left border
        self.draw_rect(
            Rect::new(
                rect.x - half_border,
                rect.y - half_border,
                border_width,
                rect.height + border_width,
            ),
            border_color,
        );

        // Right border
        self.draw_rect(
            Rect::new(
                rect.x + rect.width - half_border,
                rect.y - half_border,
                border_width,
                rect.height + border_width,
            ),
            border_color,
        );
    }

    /// Flush the current vertex batch to GPU buffers
    pub fn flush(&mut self) -> Result<()> {
        if self.current_vertices.is_empty() {
            return Ok(());
        }

        // Use the next available buffer in sequence to avoid reusing buffers
        // before the GPU has finished reading from them
        let required_capacity = self.current_vertices.len() as u32;

        // Check if we can use the next buffer in sequence
        let buffer_index = if self.next_buffer_index < self.vertex_buffers.len() {
            // Check if existing buffer has enough capacity
            if self.vertex_buffers[self.next_buffer_index].capacity >= required_capacity {
                self.next_buffer_index
            } else {
                // Need a larger buffer, create new one
                let capacity = required_capacity.max(self.max_vertices_per_buffer);
                let buffer = VertexBuffer::new(&self.device, capacity);
                self.vertex_buffers.push(buffer);
                self.vertex_buffers.len() - 1
            }
        } else {
            // Need to create a new buffer
            let capacity = required_capacity.max(self.max_vertices_per_buffer);
            let buffer = VertexBuffer::new(&self.device, capacity);
            self.vertex_buffers.push(buffer);
            self.vertex_buffers.len() - 1
        };

        // Update the buffer and advance to next
        self.vertex_buffers[buffer_index].update(&self.queue, &self.current_vertices)?;
        self.pending_draws.push(buffer_index);
        self.next_buffer_index = buffer_index + 1;

        self.current_vertices.clear();
        Ok(())
    }

    /// Submit any uploaded vertex buffers to the active render pass.
    pub fn commit(&mut self, render_pass: &mut wgpu::RenderPass) -> Result<()> {
        if !self.current_vertices.is_empty() {
            self.flush()?;
        }

        for &buffer_index in &self.pending_draws {
            let buffer = &mut self.vertex_buffers[buffer_index];
            if buffer.vertex_count > 0 {
                render_pass.set_vertex_buffer(0, buffer.slice());
                render_pass.draw(0..buffer.vertex_count, 0..1);
                buffer.vertex_count = 0;
            }
        }

        self.pending_draws.clear();
        Ok(())
    }

    /// Get all vertex buffers for rendering
    pub fn vertex_buffers(&self) -> &[VertexBuffer] {
        &self.vertex_buffers
    }

    /// Clear all batched vertices and reset buffer allocation for new frame
    pub fn clear(&mut self) {
        self.current_vertices.clear();
        self.pending_draws.clear();
        // Reset buffer index for new frame - buffers can be reused once
        // the previous frame's render pass has completed
        self.next_buffer_index = 0;
        for buffer in &mut self.vertex_buffers {
            buffer.vertex_count = 0;
        }
        #[cfg(feature = "text-rendering")]
        {
            self.staged_text.clear();
            self.current_stage_priority = 0;
        }
    }

    /// Get all text items for rendering (flattened from all stages, in order)
    #[cfg(feature = "text-rendering")]
    pub fn text_items(&self) -> Vec<TextItem> {
        self.staged_text
            .values()
            .flat_map(|items| items.iter().cloned())
            .collect()
    }

    /// Get text items for a specific stage priority
    #[cfg(feature = "text-rendering")]
    pub fn text_items_for_stage(&self, stage_priority: i32) -> &[TextItem] {
        self.staged_text
            .get(&stage_priority)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get all stage priorities that have text
    #[cfg(feature = "text-rendering")]
    pub fn text_stage_priorities(&self) -> Vec<i32> {
        self.staged_text.keys().copied().collect()
    }

    /// Take text items for a specific stage (removes them from context)
    #[cfg(feature = "text-rendering")]
    pub fn take_text_for_stage(&mut self, stage_priority: i32) -> Vec<TextItem> {
        self.staged_text.remove(&stage_priority).unwrap_or_default()
    }

    /// Draw text at the specified position
    #[cfg(feature = "text-rendering")]
    pub fn draw_text(&mut self, text: &str, x: f32, y: f32, color: Color, font_size: Option<f32>) {
        self.staged_text
            .entry(self.current_stage_priority)
            .or_default()
            .push(TextItem {
                text: text.to_string(),
                x,
                y,
                font_size: font_size.unwrap_or(12.0),
                color,
                anchor: TextAnchor::Start,
                baseline: TextBaseline::Top,
            });
    }

    /// Draw text with anchor and baseline options
    #[cfg(feature = "text-rendering")]
    pub fn draw_text_anchored(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        color: Color,
        font_size: Option<f32>,
        anchor: TextAnchor,
        baseline: TextBaseline,
    ) {
        self.staged_text
            .entry(self.current_stage_priority)
            .or_default()
            .push(TextItem {
                text: text.to_string(),
                x,
                y,
                font_size: font_size.unwrap_or(12.0),
                color,
                anchor,
                baseline,
            });
    }
}
