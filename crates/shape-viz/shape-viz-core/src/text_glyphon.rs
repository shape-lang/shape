//! Text rendering using glyphon for GPU-accelerated text

use crate::error::{ChartError, Result};
use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics, 
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer as GlyphonRenderer,
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

/// Wrapper around glyphon for text rendering in charts
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: TextAtlas,
    text_renderer: GlyphonRenderer,
    viewport_width: u32,
    viewport_height: u32,
}

impl TextRenderer {
    /// Create a new text renderer
    pub fn new(
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, format);
        let text_renderer = GlyphonRenderer::new(&mut atlas, device, format);

        Ok(Self {
            font_system,
            swash_cache,
            atlas,
            text_renderer,
            viewport_width,
            viewport_height,
        })
    }
    
    /// Update viewport size
    pub fn resize(&mut self, width: u32, height: u32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }
    
    /// Prepare text for rendering
    pub fn prepare_text(
        &mut self,
        device: &Device,
        queue: &Queue,
        texts: &[TextItem],
    ) -> Result<()> {
        let resolution = Resolution {
            width: self.viewport_width,
            height: self.viewport_height,
        };
        
        let mut text_areas = Vec::new();
        
        for item in texts {
            // Create a buffer for this text item
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(item.font_size, item.font_size * 1.2));
            
            // Set the text
            buffer.set_text(
                &mut self.font_system,
                &item.text,
                Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
            );
            
            // Calculate text bounds based on anchor and baseline
            let (width, height) = self.measure_text(&buffer);
            
            let left = match item.anchor {
                TextAnchor::Start => item.x,
                TextAnchor::Middle => item.x - width / 2.0,
                TextAnchor::End => item.x - width,
            };
            
            let top = match item.baseline {
                TextBaseline::Top => item.y,
                TextBaseline::Middle => item.y - height / 2.0,
                TextBaseline::Bottom => item.y - height,
            };
            
            // Create text area for rendering
            let text_area = TextArea {
                buffer: &buffer,
                left,
                top,
                scale: 1.0,
                bounds: TextBounds {
                    left: left as i32,
                    top: top as i32,
                    right: (left + width) as i32,
                    bottom: (top + height) as i32,
                },
                default_color: GlyphonColor::rgb(
                    (item.color[0] * 255.0) as u8,
                    (item.color[1] * 255.0) as u8,
                    (item.color[2] * 255.0) as u8,
                ),
            };
            
            text_areas.push(text_area);
        }
        
        // Prepare glyphs for rendering
        self.text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            resolution,
            text_areas,
            &mut self.swash_cache,
        ).map_err(|e| ChartError::TextRendering(format!("Failed to prepare text: {:?}", e)))?;
        
        Ok(())
    }
    
    /// Render text to the current render pass
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) -> Result<()> {
        self.text_renderer.render(&self.atlas, render_pass)
            .map_err(|e| ChartError::TextRendering(format!("Failed to render text: {:?}", e)))?;
        Ok(())
    }
    
    /// Measure text dimensions
    fn measure_text(&self, buffer: &Buffer) -> (f32, f32) {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                min_x = min_x.min(glyph.x);
                max_x = max_x.max(glyph.x + glyph.w as f32);
                min_y = min_y.min(glyph.y - glyph.font_size);
                max_y = max_y.max(glyph.y);
            }
        }
        
        let width = (max_x - min_x).max(0.0);
        let height = (max_y - min_y).max(0.0);
        
        (width, height)
    }
}

/// Text item to be rendered
pub struct TextItem {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub color: [f32; 4],
    pub anchor: TextAnchor,
    pub baseline: TextBaseline,
}