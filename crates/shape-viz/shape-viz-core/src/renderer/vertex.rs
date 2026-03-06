//! Vertex data structures for GPU rendering

use crate::error::{ChartError, Result};
use crate::theme::Color;
use bytemuck::{Pod, Zeroable};

/// Vertex data for GPU rendering
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    pub fn new(position: [f32; 2], color: Color) -> Self {
        Self {
            position,
            color: color.to_array(),
        }
    }

    /// Vertex buffer layout description for WGPU
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // Position
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // Color
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// GPU buffer for storing vertex data
#[derive(Debug)]
pub struct VertexBuffer {
    buffer: wgpu::Buffer,
    pub(super) vertex_count: u32,
    pub(super) capacity: u32,
}

impl VertexBuffer {
    /// Create a new vertex buffer with the given capacity
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: (capacity as u64) * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            vertex_count: 0,
            capacity,
        }
    }

    /// Update the buffer with new vertex data
    pub fn update(&mut self, queue: &wgpu::Queue, vertices: &[Vertex]) -> Result<()> {
        if vertices.len() as u32 > self.capacity {
            return Err(ChartError::BufferCreation(format!(
                "Vertex count {} exceeds buffer capacity {}",
                vertices.len(),
                self.capacity
            )));
        }

        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(vertices));
        self.vertex_count = vertices.len() as u32;
        Ok(())
    }

    /// Get the buffer slice for rendering
    pub fn slice(&self) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(..)
    }

    /// Get the number of vertices in the buffer
    pub fn vertex_count(&self) -> u32 {
        self.vertex_count
    }
}
