//! Main GPU renderer implementation

use crate::error::{ChartError, Result};
use crate::layers::LayerManager;
use crate::theme::Color;
use std::cell::RefCell;
use std::sync::Arc;

use super::context::RenderContext;
use super::shaders::{ScreenUniforms, create_render_pipeline};

/// Main GPU renderer
pub struct GpuRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    _surface: Option<wgpu::Surface<'static>>,
    _surface_config: Option<wgpu::SurfaceConfiguration>,
    render_pipeline: wgpu::RenderPipeline,
    _screen_uniform_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    output_buffer: Option<wgpu::Buffer>,
    width: u32,
    height: u32,
    #[cfg(feature = "text-rendering")]
    text_renderer: Option<RefCell<crate::text::TextRenderer>>,
}

impl GpuRenderer {
    /// Create a new GPU renderer for offscreen rendering
    pub async fn new_offscreen(width: u32, height: u32) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await;

        let adapter = match adapter {
            Ok(adapter) => adapter,
            Err(e) => {
                return Err(ChartError::internal(format!(
                    "Failed to request adapter: {}",
                    e
                )));
            }
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                label: Some("Chart Device"),
                memory_hints: wgpu::MemoryHints::default(),
                trace: Default::default(),
            })
            .await?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // Create screen uniform buffer
        let screen_uniforms = ScreenUniforms {
            width: width as f32,
            height: height as f32,
        };
        let screen_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Screen Uniform Buffer"),
            size: std::mem::size_of::<ScreenUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &screen_uniform_buffer,
            0,
            bytemuck::cast_slice(&[screen_uniforms]),
        );

        // Create bind group layout and bind group
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Screen Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Screen Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_uniform_buffer.as_entire_binding(),
            }],
        });

        // Create render pipeline
        let render_pipeline = create_render_pipeline(&device, &bind_group_layout)?;

        // Create offscreen texture
        let (texture, texture_view, output_buffer) =
            Self::create_offscreen_target(&device, width, height)?;

        // Create text renderer if the feature is enabled
        #[cfg(feature = "text-rendering")]
        let text_renderer = {
            match crate::text::TextRenderer::new(
                &device,
                &queue,
                wgpu::TextureFormat::Rgba8Unorm,
                width,
                height,
            ) {
                Ok(renderer) => Some(RefCell::new(renderer)),
                Err(e) => {
                    eprintln!("Warning: Failed to create text renderer: {:?}", e);
                    None
                }
            }
        };

        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            _surface: None,
            _surface_config: None,
            render_pipeline,
            _screen_uniform_buffer: screen_uniform_buffer,
            screen_bind_group,
            texture: Some(texture),
            texture_view: Some(texture_view),
            output_buffer: Some(output_buffer),
            width,
            height,
            #[cfg(feature = "text-rendering")]
            text_renderer,
        })
    }

    /// Create offscreen rendering target
    fn create_offscreen_target(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, wgpu::Buffer)> {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, // Use linear color space
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            label: Some("Chart Texture"),
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create output buffer for reading back texture data
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row * height) as u64;

        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok((texture, texture_view, output_buffer))
    }

    /// Render chart and return RGBA image data
    pub async fn render(
        &self,
        context: &mut RenderContext,
        layer_manager: &LayerManager,
        clear_color: Color,
    ) -> Result<Vec<u8>> {
        // Flush any pending vertices
        context.flush()?;

        let texture_view = self
            .texture_view
            .as_ref()
            .ok_or_else(|| ChartError::internal("No texture view available"))?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Chart Encoder"),
            });

        // First pass: render geometry
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Chart Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear_color.r as f64,
                            g: clear_color.g as f64,
                            b: clear_color.b as f64,
                            a: clear_color.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.screen_bind_group, &[]);

            // Render all layers using the layer manager, which will set scissor rects
            layer_manager.render_all(context, &mut render_pass)?;
        }

        self.queue.submit(Some(encoder.finish()));

        // Second pass: render text if enabled (text is now ordered by stage priority)
        #[cfg(feature = "text-rendering")]
        {
            let all_text = context.text_items();
            if !all_text.is_empty() {
                if let Some(text_renderer) = &self.text_renderer {
                    // Prepare text outside of render pass - items are already in stage order
                    text_renderer
                        .borrow_mut()
                        .prepare(&self.device, &self.queue, &all_text)?;

                    let mut text_encoder =
                        self.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("Text Encoder"),
                            });

                    let renderer = text_renderer.borrow();
                    {
                        let mut render_pass =
                            text_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Text Render Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: texture_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });

                        renderer.render(&mut render_pass)?;
                    }

                    self.queue.submit(Some(text_encoder.finish()));
                }
            }
        }

        // Final encoder for texture readback
        let mut readback_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Readback Encoder"),
                });

        // Copy texture to buffer for readback
        if let Some(output_buffer) = &self.output_buffer {
            let unpadded_bytes_per_row = self.width * 4;
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

            readback_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: self.texture.as_ref().unwrap(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: output_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(self.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.queue.submit(Some(readback_encoder.finish()));

        // Read back the image data
        self.read_texture_data().await
    }

    /// Read texture data back from GPU
    async fn read_texture_data(&self) -> Result<Vec<u8>> {
        let output_buffer = self
            .output_buffer
            .as_ref()
            .ok_or_else(|| ChartError::internal("No output buffer available"))?;

        let buffer_slice = output_buffer.slice(..);
        let (sender, receiver) = futures::channel::oneshot::channel();

        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap_or(());
        });

        let _ = self.device.poll(wgpu::MaintainBase::Wait);

        match receiver.await {
            Ok(Ok(())) => {
                let data = buffer_slice.get_mapped_range();

                // Convert from padded to unpadded RGBA data
                let unpadded_bytes_per_row = self.width * 4;
                let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

                let mut rgba_data = Vec::with_capacity((self.width * self.height * 4) as usize);

                for y in 0..self.height {
                    let start = (y * padded_bytes_per_row) as usize;
                    let end = start + (self.width * 4) as usize;
                    rgba_data.extend_from_slice(&data[start..end]);
                }

                drop(data);
                output_buffer.unmap();

                Ok(rgba_data)
            }
            _ => Err(ChartError::internal("Failed to read texture data from GPU")),
        }
    }

    /// Get renderer dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Get device and queue for external use
    pub fn device_and_queue(&self) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        (self.device.clone(), self.queue.clone())
    }
}
