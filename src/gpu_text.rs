use std::mem;

use bytemuck::{Pod, Zeroable};
use glam::DVec2;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;

use crate::{
    atlas::{CpuAtlas, FIELD_RANGE_PX},
    text::{Document, DocumentGlyph, TextStyle},
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuGlyphInstance {
    rect: [f32; 4],
    uv_rect: [f32; 4],
    fill_top: [f32; 4],
    fill_bottom: [f32; 4],
    outline_color: [f32; 4],
    shadow_color: [f32; 4],
    effect_params: [f32; 4],
}

impl GpuGlyphInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x4
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Globals {
    viewport_atlas: [f32; 4],
    range_padding: [f32; 4],
}

pub struct TextRenderer {
    _atlas_texture: wgpu::Texture,
    _globals_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    shadow_pipeline: wgpu::RenderPipeline,
    main_pipeline: wgpu::RenderPipeline,
    instance_count: u32,
}

impl TextRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        document: &Document,
        atlas: &CpuAtlas,
        viewport: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Self {
        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("MSDF atlas"),
            size: wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.width * 4),
                rows_per_image: Some(atlas.height),
            },
            wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("MSDF sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let globals = make_globals(viewport, atlas);
        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MSDF bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MSDF bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: globals_buffer.as_entire_binding(),
                },
            ],
        });

        let instances = build_initial_instances(document, atlas, viewport, scale_factor);
        let instance_capacity = document.glyphs.len() + document.hud_glyphs.len();
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph instances"),
            size: (instance_capacity.max(1) * mem::size_of::<GpuGlyphInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));

        let shader = device.create_shader_module(wgpu::include_wgsl!("msdf.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("MSDF pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            "MSDF shadow pipeline",
            "vs_shadow",
            "fs_shadow",
        );
        let main_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            "MSDF main pipeline",
            "vs_main",
            "fs_main",
        );

        Self {
            _atlas_texture: atlas_texture,
            _globals_buffer: globals_buffer,
            bind_group,
            instance_buffer,
            shadow_pipeline,
            main_pipeline,
            instance_count: instances.len() as u32,
        }
    }

    pub fn update_initial_view(
        &mut self,
        queue: &wgpu::Queue,
        document: &Document,
        atlas: &CpuAtlas,
        viewport: PhysicalSize<u32>,
        scale_factor: f64,
    ) {
        let globals = make_globals(viewport, atlas);
        queue.write_buffer(&self._globals_buffer, 0, bytemuck::bytes_of(&globals));
        let instances = build_initial_instances(document, atlas, viewport, scale_factor);
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        self.instance_count = instances.len() as u32;
    }

    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.set_pipeline(&self.shadow_pipeline);
        pass.draw(0..6, 0..self.instance_count);
        pass.set_pipeline(&self.main_pipeline);
        pass.draw(0..6, 0..self.instance_count);
    }
}

fn make_globals(viewport: PhysicalSize<u32>, atlas: &CpuAtlas) -> Globals {
    Globals {
        viewport_atlas: [
            viewport.width.max(1) as f32,
            viewport.height.max(1) as f32,
            atlas.width as f32,
            atlas.height as f32,
        ],
        range_padding: [FIELD_RANGE_PX as f32, 0.0, 0.0, 0.0],
    }
}

fn build_initial_instances(
    document: &Document,
    atlas: &CpuAtlas,
    viewport: PhysicalSize<u32>,
    scale_factor: f64,
) -> Vec<GpuGlyphInstance> {
    let page_scale = scale_factor;
    let page_offset = DVec2::new(
        (viewport.width as f64 - document.bounds.size().x * page_scale) * 0.5,
        34.0 * scale_factor,
    );
    let hud_offset = DVec2::new(0.0, viewport.height as f64 / scale_factor - 60.0) * scale_factor;
    let mut instances = Vec::with_capacity(document.glyphs.len() + document.hud_glyphs.len());
    append_instances(
        &mut instances,
        &document.glyphs,
        &document.styles,
        atlas,
        page_scale,
        page_offset,
    );
    append_instances(
        &mut instances,
        &document.hud_glyphs,
        &document.styles,
        atlas,
        scale_factor,
        hud_offset,
    );
    instances
}

fn append_instances(
    output: &mut Vec<GpuGlyphInstance>,
    glyphs: &[DocumentGlyph],
    styles: &[TextStyle],
    atlas: &CpuAtlas,
    scale: f64,
    screen_offset: DVec2,
) {
    for glyph in glyphs.iter().filter(|glyph| !glyph.whitespace) {
        let Some(atlas_glyph) = atlas
            .glyphs
            .get(&glyph.glyph_id)
            .or_else(|| atlas.glyphs.get(&0))
        else {
            continue;
        };
        let style = &styles[glyph.style as usize];
        let plane_min = DVec2::new(
            atlas_glyph.plane_min[0] as f64,
            atlas_glyph.plane_min[1] as f64,
        ) * glyph.font_size;
        let plane_max = DVec2::new(
            atlas_glyph.plane_max[0] as f64,
            atlas_glyph.plane_max[1] as f64,
        ) * glyph.font_size;
        let min = (glyph.origin + plane_min) * scale + screen_offset;
        let max = (glyph.origin + plane_max) * scale + screen_offset;
        let atlas_width = atlas.width as f32;
        let atlas_height = atlas.height as f32;
        let effect_scale = (glyph.font_size * scale) as f32;
        output.push(GpuGlyphInstance {
            rect: [
                min.x as f32,
                min.y as f32,
                (max.x - min.x) as f32,
                (max.y - min.y) as f32,
            ],
            uv_rect: [
                atlas_glyph.pixel_min[0] as f32 / atlas_width,
                atlas_glyph.pixel_min[1] as f32 / atlas_height,
                atlas_glyph.pixel_max[0] as f32 / atlas_width,
                atlas_glyph.pixel_max[1] as f32 / atlas_height,
            ],
            fill_top: style.fill_top,
            fill_bottom: style.fill_bottom,
            outline_color: style.outline_color,
            shadow_color: style.shadow_color,
            effect_params: [
                style.outline_em * effect_scale,
                style.glow_em * effect_scale,
                style.shadow_offset_em[0] * effect_scale,
                style.shadow_offset_em[1] * effect_scale,
            ],
        });
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    label: &'static str,
    vertex_entry: &'static str,
    fragment_entry: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            buffers: &[Some(GpuGlyphInstance::layout())],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msdf_shader_parses() {
        naga::front::wgsl::parse_str(include_str!("msdf.wgsl"))
            .expect("MSDF shader should be valid WGSL");
    }

    #[test]
    fn instance_layout_is_seven_vec4s() {
        assert_eq!(mem::size_of::<GpuGlyphInstance>(), 7 * 16);
        assert_eq!(mem::align_of::<GpuGlyphInstance>(), 4);
    }
}
