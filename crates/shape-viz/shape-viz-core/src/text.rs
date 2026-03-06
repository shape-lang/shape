//! Text rendering for charts using cosmic-text

use crate::error::{ChartError, Result};
use crate::theme::Color;
use crate::renderer::Vertex;
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache};
use std::collections::HashMap;

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

/// Cached text buffer
struct CachedText {
    buffer: Buffer,
    width: f32,
    height: f32,
}

/// Text renderer for GPU-based text rendering
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_cache: HashMap<String, CachedText>,
    default_font_size: f32,
}

impl TextRenderer {
    /// Create a new text renderer
    pub fn new() -> Result<Self> {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        
        Ok(Self {
            font_system,
            swash_cache,
            text_cache: HashMap::new(),
            default_font_size: 12.0,
        })
    }
    
    /// Set the default font size
    pub fn set_default_font_size(&mut self, size: f32) {
        self.default_font_size = size;
        self.text_cache.clear(); // Clear cache when font size changes
    }
    
    /// Measure text dimensions without rendering
    pub fn measure_text(&mut self, text: &str, font_size: Option<f32>) -> (f32, f32) {
        let font_size = font_size.unwrap_or(self.default_font_size);
        let cache_key = format!("{}_{}", text, font_size);
        
        if let Some(cached) = self.text_cache.get(&cache_key) {
            return (cached.width, cached.height);
        }
        
        let metrics = Metrics::new(font_size, font_size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        
        buffer.set_size(&mut self.font_system, Some(1000.0), Some(100.0));
        buffer.set_text(&mut self.font_system, text, Attrs::new(), Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, true);
        
        // Calculate actual text bounds
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                min_x = min_x.min(glyph.x_offset);
                max_x = max_x.max(glyph.x_offset + glyph.w as f32);
                min_y = min_y.min(glyph.y_offset);
                // Approximate height based on font size
                max_y = max_y.max(glyph.y_offset + font_size);
            }
        }
        
        let width = (max_x - min_x).max(0.0);
        let height = (max_y - min_y).max(0.0);
        
        // Cache the buffer
        self.text_cache.insert(cache_key, CachedText {
            buffer,
            width,
            height,
        });
        
        (width, height)
    }
    
    /// Render text to vertices for GPU rendering
    pub fn render_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        color: Color,
        font_size: Option<f32>,
        anchor: TextAnchor,
        baseline: TextBaseline,
    ) -> Result<Vec<Vertex>> {
        let font_size = font_size.unwrap_or(self.default_font_size);
        let (width, height) = self.measure_text(text, Some(font_size));
        
        // Calculate anchor offset
        let anchor_x = match anchor {
            TextAnchor::Start => 0.0,
            TextAnchor::Middle => -width / 2.0,
            TextAnchor::End => -width,
        };
        
        // Calculate baseline offset
        let baseline_y = match baseline {
            TextBaseline::Top => 0.0,
            TextBaseline::Middle => -height / 2.0,
            TextBaseline::Bottom => -height,
        };
        
        let cache_key = format!("{}_{}", text, font_size);
        let cached = self.text_cache.get_mut(&cache_key)
            .ok_or_else(|| ChartError::TextRendering("Text not found in cache".to_string()))?;
        
        let mut vertices = Vec::new();
        
        // Render glyphs to vertices
        for run in cached.buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let glyph_x = x + anchor_x + glyph.x_offset;
                let glyph_y = y + baseline_y + glyph.y_offset;
                
                // Create a quad for each glyph
                // For now, we'll create a simple rectangle for each glyph
                // In a real implementation, you'd use a glyph atlas texture
                let x1 = glyph_x;
                let y1 = glyph_y;
                let x2 = glyph_x + glyph.w as f32;
                // Use font size as approximate height
                let y2 = glyph_y + font_size;
                
                // Create two triangles for the glyph quad
                vertices.extend_from_slice(&[
                    Vertex::new([x1, y1], color),
                    Vertex::new([x2, y1], color),
                    Vertex::new([x1, y2], color),
                    
                    Vertex::new([x2, y1], color),
                    Vertex::new([x2, y2], color),
                    Vertex::new([x1, y2], color),
                ]);
            }
        }
        
        Ok(vertices)
    }
    
    /// Clear the text cache
    pub fn clear_cache(&mut self) {
        self.text_cache.clear();
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new().expect("Failed to create text renderer")
    }
}

/// Simple text rendering using rectangles (temporary solution)
/// This is a simplified version that draws text using rectangles
/// until proper glyph atlas support is implemented
pub fn render_text_simple(
    text: &str,
    x: f32,
    y: f32,
    color: Color,
    font_size: f32,
    anchor: TextAnchor,
    baseline: TextBaseline,
) -> Vec<Vertex> {
    // Approximate character size (monospace assumption)
    let char_width = font_size * 0.6;
    let char_height = font_size;
    
    let text_width = text.len() as f32 * char_width;
    let text_height = char_height;
    
    // Calculate anchor offset
    let anchor_x = match anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => -text_width / 2.0,
        TextAnchor::End => -text_width,
    };
    
    // Calculate baseline offset
    let baseline_y = match baseline {
        TextBaseline::Top => 0.0,
        TextBaseline::Middle => -text_height / 2.0,
        TextBaseline::Bottom => -text_height,
    };
    
    let mut vertices = Vec::new();
    
    // For now, just draw a simple rectangle where the text would be
    // This is a placeholder until proper text rendering is implemented
    let x1 = x + anchor_x;
    let y1 = y + baseline_y;
    let x2 = x1 + text_width;
    let _y2 = y1 + text_height;
    
    // For now, draw a filled rectangle to represent the text bounds
    // This is more visible than thin lines
    vertices.extend_from_slice(&[
        // First triangle
        Vertex::new([x1, y1], color),
        Vertex::new([x2, y1], color),
        Vertex::new([x1, y1 + text_height * 0.7], color),
        
        // Second triangle
        Vertex::new([x2, y1], color),
        Vertex::new([x2, y1 + text_height * 0.7], color),
        Vertex::new([x1, y1 + text_height * 0.7], color),
    ]);
    
    vertices
}