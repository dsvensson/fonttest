use std::sync::Arc;

use anyhow::{Context, Result};
use glam::DVec2;
use winit::{dpi::PhysicalSize, window::Window};

use crate::{
    atlas::CpuAtlas, camera::Camera, font::FontSelection, gpu_text::TextRenderer, text::Document,
};

pub struct Renderer {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    background_pipeline: wgpu::RenderPipeline,
    text_renderer: TextRenderer,
    size: PhysicalSize<u32>,
    camera: Camera,
    document: Document,
    atlas: CpuAtlas,
}

#[derive(Debug, Clone, Copy)]
pub enum FrameError {
    Timeout,
    Occluded,
    Outdated,
    Lost,
    Validation,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, font_spec: &str) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .context("creating the wgpu surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .context("requesting a compatible GPU adapter")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("MSDF device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .context("requesting a wgpu device")?;

        let font = FontSelection::resolve(font_spec).context("resolving selected font")?;
        window.set_title(&format!("MSDF Font Explorer — {}", font.resolved_name));
        let document = Document::build(&font).context("shaping the typography document")?;
        let atlas = CpuAtlas::build(
            &font,
            &document.atlas_glyphs,
            device.limits().max_texture_dimension_2d,
        )
        .context("generating the MSDF glyph atlas")?;
        log::info!(
            "loaded {} from {}; shaped {} page and {} HUD glyphs across {} styles on a {:.0}×{:.0} page into a {}×{} atlas ({} unique outlines, {} bytes)",
            font.resolved_name,
            font.source,
            document.glyphs.len(),
            document.hud_glyphs.len(),
            document.styles.len(),
            document.bounds.size().x,
            document.bounds.size().y,
            atlas.width,
            atlas.height,
            atlas.glyphs.len(),
            atlas.pixels.len()
        );

        let width = size.width.max(1);
        let height = size.height.max(1);
        let config = surface
            .get_default_config(&adapter, width, height)
            .context("the surface has no supported configuration")?;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::include_wgsl!("background.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("background pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let background_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let scale_factor = window.scale_factor();
        let camera = Camera::new(size, scale_factor, document.bounds);
        let text_renderer = TextRenderer::new(
            &device,
            &queue,
            config.format,
            &document,
            &atlas,
            size,
            &camera,
        );

        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            background_pipeline,
            text_renderer,
            size,
            camera,
            document,
            atlas,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        self.camera.resize(size);
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.update_text_view();
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.camera.set_scale_factor(scale_factor);
        self.update_text_view();
    }

    pub fn zoom_at(&mut self, cursor: DVec2, wheel_delta: f64) {
        if self.camera.zoom_at(cursor, wheel_delta) {
            self.update_text_view();
        }
    }

    pub fn pan_by(&mut self, screen_delta: DVec2) {
        if self.camera.pan_by(screen_delta) {
            self.update_text_view();
        }
    }

    pub fn reset_view(&mut self) {
        self.camera.reset();
        self.update_text_view();
    }

    fn update_text_view(&mut self) {
        self.text_renderer.update_view(
            &self.queue,
            &self.document,
            &self.atlas,
            self.size,
            &self.camera,
        );
    }

    pub fn reconfigure(&mut self) {
        if self.size.width != 0 && self.size.height != 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn render(&mut self) -> Result<(), FrameError> {
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(());
        }

        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::Timeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::Occluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::Outdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::Validation),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);
            self.text_renderer.draw(&mut pass);
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        if suboptimal {
            self.reconfigure();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn background_shader_parses() {
        naga::front::wgsl::parse_str(include_str!("background.wgsl"))
            .expect("background shader should be valid WGSL");
    }
}
