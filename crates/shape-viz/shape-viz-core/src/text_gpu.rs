//! GPU-accelerated text rendering using glyphon

use crate::error::{ChartError, Result};
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer as GlyphonRenderer, Viewport as GlyphonViewport,
};
use wgpu::{Device, Queue, TextureFormat};

/// Text anchor positions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextAnchor {
    Start,
    Middle,
    End,
}

/// Text baseline alignment
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextBaseline {
    Top,
    Middle,
    Bottom,
}

/// A text item to be rendered
#[derive(Debug, Clone)]
pub struct TextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub color: crate::theme::Color,
    pub anchor: TextAnchor,
    pub baseline: TextBaseline,
}

/// GPU-accelerated text renderer using glyphon
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: GlyphonViewport,
    atlas: TextAtlas,
    renderer: GlyphonRenderer,
    viewport_width: u32,
    viewport_height: u32,
    text_buffers: Vec<Buffer>,
}

impl TextRenderer {
    /// Create a new text renderer
    pub fn new(
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        // Initialize font system with system fonts
        let font_system = FontSystem::new();

        // Create the glyphon components
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = GlyphonViewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            GlyphonRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        Ok(Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            viewport_width: width,
            viewport_height: height,
            text_buffers: Vec::new(),
        })
    }

    /// Update viewport dimensions
    pub fn resize(&mut self, _device: &Device, queue: &Queue, width: u32, height: u32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.viewport.update(
            queue,
            Resolution {
                width: self.viewport_width,
                height: self.viewport_height,
            },
        );
    }

    /// Prepare text items for rendering
    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        text_items: &[TextItem],
    ) -> Result<()> {
        // Clear previous buffers
        self.text_buffers.clear();

        // Process all text items and create buffers
        for item in text_items {
            // Create a buffer for this text
            let mut buffer = Buffer::new(
                &mut self.font_system,
                Metrics::new(item.font_size, item.font_size * 1.2),
            );

            // Set the buffer size (make it large enough for the text)
            buffer.set_size(
                &mut self.font_system,
                Some(self.viewport_width as f32),
                Some(self.viewport_height as f32),
            );

            // Set the text with attributes
            let attrs = Attrs::new().family(Family::SansSerif);
            buffer.set_text(&mut self.font_system, &item.text, &attrs, Shaping::Advanced);

            // Shape the text
            buffer.shape_until_scroll(&mut self.font_system, false);

            // Store the buffer
            self.text_buffers.push(buffer);
        }

        // Create text areas for rendering
        let mut text_areas = Vec::new();

        for (i, item) in text_items.iter().enumerate() {
            let buffer = &self.text_buffers[i];

            // Measure the text
            let (text_width, text_height) = self.measure_buffer(buffer);

            // Calculate position based on anchor and baseline
            let left = match item.anchor {
                TextAnchor::Start => item.x,
                TextAnchor::Middle => item.x - text_width / 2.0,
                TextAnchor::End => item.x - text_width,
            };

            let top = match item.baseline {
                TextBaseline::Top => item.y,
                TextBaseline::Middle => item.y - text_height / 2.0,
                TextBaseline::Bottom => item.y - text_height,
            };

            // Create text area for rendering
            let text_area = TextArea {
                buffer,
                left,
                top,
                scale: 1.0,
                bounds: TextBounds {
                    left: left as i32,
                    top: top as i32,
                    right: (left + self.viewport_width as f32) as i32,
                    bottom: (top + self.viewport_height as f32) as i32,
                },
                default_color: Color::rgba(
                    (item.color.r * 255.0) as u8,
                    (item.color.g * 255.0) as u8,
                    (item.color.b * 255.0) as u8,
                    (item.color.a * 255.0) as u8,
                ),
                custom_glyphs: &[],
            };

            text_areas.push(text_area);
        }

        // Update viewport resolution
        self.viewport.update(
            queue,
            Resolution {
                width: self.viewport_width,
                height: self.viewport_height,
            },
        );

        // Prepare glyphon for rendering
        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .map_err(|e| ChartError::TextRendering(format!("Failed to prepare text: {:?}", e)))?;

        Ok(())
    }

    /// Render the prepared text
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) -> Result<()> {
        self.renderer
            .render(&self.atlas, &self.viewport, render_pass)
            .map_err(|e| ChartError::TextRendering(format!("Failed to render text: {:?}", e)))?;
        Ok(())
    }

    /// Measure text dimensions for a buffer
    fn measure_buffer(&self, buffer: &Buffer) -> (f32, f32) {
        let mut width = 0.0f32;
        let mut height = 0.0f32;

        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height += run.line_height;
        }

        (width, height)
    }

    /// Clear the cache (call when changing scenes/charts)
    pub fn clear_cache(&mut self) {
        self.atlas.trim();
    }
}
